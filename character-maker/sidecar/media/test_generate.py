import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch, Mock
from contextlib import nullcontext
import tempfile


HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GEN = load("media_generate", HERE / "generate.py")
SETUP = load("media_setup", HERE / "setup_models.py")


class WorkflowTests(unittest.TestCase):
    def test_real_gpu_lock_blocks_another_process_and_releases(self):
        temporary_root = HERE.parents[2] / "temp"
        temporary_root.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary_root) as directory:
            path = Path(directory) / "characters/temp/completion-gpu.lock"
            script = ("import sys; from pathlib import Path; "
                      "sys.path.insert(0, sys.argv[1]); from generate import gpu_lock; "
                      "lock=gpu_lock(Path(sys.argv[2])); lock.__enter__(); lock.__exit__(None,None,None)")
            def child():
                return GEN.subprocess.run([GEN.os.sys.executable, "-c", script, str(HERE), str(path)],
                    capture_output=True, timeout=10,
                    creationflags=getattr(GEN.subprocess, "CREATE_NO_WINDOW", 0))
            with GEN.gpu_lock(path):
                self.assertNotEqual(child().returncode, 0)
            self.assertEqual(child().returncode, 0)

    def test_qwen_edit_keeps_input_and_prompt(self):
        workflow = GEN.qwen("帽子を赤くする", 12, 768, 512, Path("input.png"))
        self.assertEqual(workflow["4"]["inputs"]["image"], "input.png")
        self.assertEqual(workflow["7"]["inputs"]["image1"], ["4", 0])
        self.assertEqual(workflow["8"]["inputs"]["image1"], ["4", 0])
        self.assertEqual(workflow["7"]["inputs"]["prompt"], "帽子を赤くする")
        self.assertEqual(workflow["10"]["inputs"]["seed"], 12)

    def test_qwen_text_only_generates_without_reference_image(self):
        workflow = GEN.qwen("青空の下の猫", 3, 768, 768, None)
        self.assertNotIn("4", workflow)
        self.assertNotIn("image1", workflow["7"]["inputs"])
        self.assertNotIn("image1", workflow["8"]["inputs"])
        self.assertEqual(workflow["9"]["class_type"], "EmptySD3LatentImage")

    def test_port_zero_reserves_free_local_port(self):
        self.assertGreater(GEN.reserve_port(0), 0)

    def test_wan_text_and_image_modes_share_official_core(self):
        text = GEN.wan("move", 1, 768, 512, 97, None)
        image = GEN.wan("move", 1, 768, 512, 97, Path("input.png"))
        self.assertNotIn("56", text)
        self.assertEqual(image["55"]["inputs"]["start_image"], ["56", 0])
        self.assertEqual(text["37"]["inputs"]["unet_name"], "wan2.2_ti2v_5B_fp16.safetensors")

    def test_ltx_image_mode_replaces_empty_latent(self):
        text = GEN.ltx("move", 2, 768, 512, 97, None)
        image = GEN.ltx("move", 2, 768, 512, 97, Path("input.png"))
        self.assertIn("70", text)
        self.assertNotIn("70", image)
        self.assertEqual(image["72"]["inputs"]["latent_image"], ["77", 2])

    def test_history_accepts_image_or_video_records(self):
        image = {"p": {"outputs": {"1": {"images": [{"filename": "a.png"}]}}}}
        video = {"p": {"outputs": {"1": {"gifs": [{"filename": "a.mp4"}]}}}}
        self.assertEqual(GEN.output_record(image, "p")["filename"], "a.png")
        self.assertEqual(GEN.output_record(video, "p")["filename"], "a.mp4")

    def test_failed_history_reports_original_error(self):
        history = {"p": {"outputs": {}, "status": {"status_str": "error", "messages": [
            ["execution_error", {"exception_message": "CUDA out of memory"}]]}}}
        with self.assertRaisesRegex(RuntimeError, "CUDA out of memory"):
            GEN.output_record(history, "p")
        with self.assertRaisesRegex(RuntimeError, "詳細なし"):
            GEN.output_record({"p": {"status": {"status_str": "error"}}}, "p")

    def test_input_and_publish_failures_clean_only_owned_files(self):
        temporary_root = HERE.parents[2] / "temp"
        temporary_root.mkdir(exist_ok=True)
        for failure in ("input", "publish", "history"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory(dir=temporary_root) as directory:
                root = Path(directory)
                source = root / "source.png"
                source.write_bytes(b"original")
                preserved = root / "finished.png"
                preserved.write_bytes(b"finished")
                output = root / "result.png"
                run = root / ".generating-result"
                incoming = root / "result.png.incoming"
                argv = ["generate.py", "--engine", "qwen-edit", "--prompt", "edit",
                        "--input", str(source), "--output", str(output), "--comfy", str(root),
                        "--models", str(root), "--gpu-lock", str(root / "gpu.lock"),
                        "--seed", "1", "--width", "256", "--height", "256"]
                def copy(src, dst):
                    Path(dst).write_bytes(b"partial")
                    if failure == "input" or Path(dst) == incoming:
                        raise OSError("copy failed")
                history = {"p": {"outputs": {"1": {"images": [{"filename": "image.png"}]}}}}
                if failure == "history":
                    history = {"p": {"status": {"status_str": "error", "messages": [
                        ["execution_error", {"exception_message": "model failed"}]]}}}
                def ready(*args):
                    (run / "output/image.png").write_bytes(b"generated")
                process = Mock()
                process.poll.return_value = None
                with patch.object(GEN.os.sys, "argv", argv), \
                     patch.object(GEN, "gpu_lock", return_value=nullcontext()), \
                     patch.object(GEN.socket, "socket"), \
                     patch.object(GEN.subprocess, "Popen", return_value=process), \
                     patch.object(GEN, "wait_for_server", side_effect=ready), \
                     patch.object(GEN, "request_json", side_effect=[{"prompt_id": "p"}, history]), \
                     patch.object(GEN, "stop_owned") as stop, \
                     patch.object(GEN.shutil, "copy2", side_effect=copy):
                    with self.assertRaisesRegex((OSError, RuntimeError), "copy failed|model failed"):
                        GEN.main()
                    self.assertEqual(stop.call_count, 0 if failure == "input" else 1)
                self.assertFalse(run.exists())
                self.assertFalse(incoming.exists())
                self.assertFalse(output.exists())
                self.assertEqual(source.read_bytes(), b"original")
                self.assertEqual(preserved.read_bytes(), b"finished")

    def test_download_metadata_is_pinned(self):
        self.assertEqual(set(SETUP.MODELS), {"wan-2.2-5b", "ltx-0.9.8-2b-fp8"})
        for items in SETUP.MODELS.values():
            for relative, url, digest, size in items:
                self.assertFalse(Path(relative).is_absolute())
                self.assertIn("/resolve/", url)
                self.assertNotIn("/resolve/main/", url)
                self.assertEqual(len(digest), 64)
                self.assertGreater(size, 0)


if __name__ == "__main__":
    unittest.main()
