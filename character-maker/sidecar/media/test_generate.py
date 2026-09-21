import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch, Mock
from contextlib import nullcontext
import tempfile
import io
import hashlib


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

    def run_main(self, root, *, lowvram, history):
        """ComfyUIとGPUだけを差し替えてmain()を通し、起動コマンドと後片付けを確かめる。"""
        source = root / "source.png"
        source.write_bytes(b"original")
        output = root / "result.png"
        run = root / ".generating-result"
        argv = ["generate.py", "--engine", "qwen-edit", "--prompt", "edit",
                "--input", str(source), "--output", str(output), "--comfy", str(root),
                "--models", str(root), "--gpu-lock", str(root / "gpu.lock"),
                "--seed", "1", "--width", "256", "--height", "256"]
        if lowvram:
            argv.append("--lowvram")
        def ready(*args):
            (run / "output/image.png").write_bytes(b"generated")
        process = Mock()
        process.poll.return_value = None
        with patch.object(GEN.os.sys, "argv", argv), \
             patch.object(GEN, "gpu_lock", return_value=nullcontext()), \
             patch.object(GEN, "reserve_port", return_value=18188), \
             patch.object(GEN.subprocess, "Popen", return_value=process) as popen, \
             patch.object(GEN, "wait_for_server", side_effect=ready), \
             patch.object(GEN, "request_json", side_effect=[{"prompt_id": "p"}, history]), \
             patch.object(GEN, "stop_owned") as stop:
            error = None
            try:
                GEN.main()
            except BaseException as raised:  # noqa: BLE001 - 呼び出し側で種類を確かめる
                error = raised
        return {"command": popen.call_args[0][0], "stop": stop.call_count,
                "error": error, "run": run, "output": output}

    def test_lowvram_flag_only_changes_comfy_startup(self):
        temporary_root = HERE.parents[2] / "temp"
        temporary_root.mkdir(exist_ok=True)
        history = {"p": {"outputs": {"1": {"images": [{"filename": "image.png"}]}}}}
        commands = {}
        for lowvram in (False, True):
            with self.subTest(lowvram=lowvram), tempfile.TemporaryDirectory(dir=temporary_root) as directory:
                result = self.run_main(Path(directory), lowvram=lowvram, history=history)
                self.assertIsNone(result["error"])
                # 作業ディレクトリは実行ごとに変わるため、比較の前だけ差し替える。
                commands[lowvram] = [str(part).replace(directory, "ROOT") for part in result["command"]]
                self.assertEqual("--lowvram" in result["command"], lowvram)
        # 変わるのは--lowvramの有無だけ。seed・モデル・入力・出力先は同じにする。
        self.assertEqual(commands[True], commands[False] + ["--lowvram"])

    def test_gpu_out_of_memory_uses_dedicated_exit_code_and_leaves_nothing(self):
        temporary_root = HERE.parents[2] / "temp"
        temporary_root.mkdir(exist_ok=True)
        for message in ("CUDA out of memory. Tried to allocate 2.00 GiB",
                        "HIP error: out of memory",
                        "Allocation on device failed"):
            with self.subTest(message=message), tempfile.TemporaryDirectory(dir=temporary_root) as directory:
                history = {"p": {"outputs": {}, "status": {"status_str": "error", "messages": [
                    ["execution_error", {"exception_message": message}]]}}}
                result = self.run_main(Path(directory), lowvram=False, history=history)
                self.assertIsInstance(result["error"], GEN.GpuOutOfMemory)
                # 再試行の前にComfyUIと子孫プロセスを回収し、途中成果物を残さない。
                self.assertEqual(result["stop"], 1)
                self.assertFalse(result["run"].exists())
                self.assertFalse(result["output"].exists())
        self.assertEqual(GEN.OOM_EXIT_CODE, 3)

    def test_non_memory_failures_are_not_reported_as_out_of_memory(self):
        self.assertFalse(GEN.is_out_of_memory("ComfyUIの生成に失敗しました: model failed"))
        self.assertFalse(GEN.is_out_of_memory("生成が時間内に完了しませんでした"))
        self.assertTrue(GEN.is_out_of_memory("torch.OutOfMemoryError"))

    def test_download_metadata_is_pinned(self):
        self.assertEqual(set(SETUP.MODELS), {"wan-2.2-5b", "ltx-0.9.8-2b-fp8", "krea-2-turbo-nvfp4", "ltx-2.5-nvfp4"})
        for items in SETUP.MODELS.values():
            for relative, url, digest, size in items:
                self.assertFalse(Path(relative).is_absolute())
                self.assertTrue("/resolve/" in url or "raw.githubusercontent.com/Lightricks/LTX-2/a95ab856" in url)
                self.assertNotIn("/resolve/main/", url)
                self.assertEqual(len(digest), 64)
                self.assertGreater(size, 0)

    def test_Kreaは参照画像を拒否し低メモリで同じ生成寸法を保つ(self):
        with self.assertRaisesRegex(ValueError, "文章から"):
            GEN.modern_workflow("krea-2-turbo-nvfp4", "猫", 1, 512, 512, 9, Path("x.png"), True)
        workflow = GEN.modern_workflow("krea-2-turbo-nvfp4", "猫", 1, 512, 512, 9, None, True)
        self.assertEqual(workflow["7"]["inputs"]["steps"], 8)
        self.assertEqual(workflow["6"]["inputs"]["width"], 512)
        self.assertEqual(workflow["2"]["inputs"]["device"], "default")
        self.assertEqual(workflow["8"]["class_type"], "VAEDecodeTiled")

    def test_LTX25は音声付き一段階生成でリンクが全て存在する(self):
        for reference in (None, Path("x.png")):
            workflow = GEN.modern_workflow("ltx-2.5-nvfp4", "猫", 1, 512, 320, 49, reference, True)
            self.assertEqual(workflow["2"]["inputs"]["device"], "cpu")
            self.assertEqual(workflow["22"]["inputs"]["audio"], ["21", 0])
            self.assertEqual(workflow["23"]["inputs"]["format"], "mp4")
            self.assertEqual(workflow["23"]["inputs"]["format.codec"], "h264")
            self.assertNotIn("LatentUpscale", str(workflow))
            for node in workflow.values():
                for value in node["inputs"].values():
                    if isinstance(value, list):
                        self.assertIn(value[0], workflow)

    def test_未同意と非対応GPUは通信より前に停止する(self):
        with patch.object(SETUP, "remote") as network, patch.object(SETUP, "check_hardware") as hardware:
            with self.assertRaisesRegex(ValueError, "同意"):
                SETUP.preflight("krea-2-turbo-nvfp4", None, HERE, HERE)
            hardware.assert_not_called()
            network.assert_not_called()
            hardware.side_effect = RuntimeError("非対応GPU")
            with self.assertRaisesRegex(RuntimeError, "非対応GPU"):
                SETUP.preflight("krea-2-turbo-nvfp4", SETUP.LICENSE_REVISION, HERE, HERE)
            network.assert_not_called()

    def test_認証不足で取得前確認が停止する(self):
        with patch.object(SETUP, "check_hardware"), patch.object(SETUP.shutil, "disk_usage", return_value=Mock(free=100_000_000_000)), patch.object(SETUP, "remote", side_effect=RuntimeError("認証が必要")):
            with self.assertRaisesRegex(RuntimeError, "認証が必要"):
                SETUP.preflight("ltx-2.5-nvfp4", SETUP.LICENSE_REVISION, HERE, HERE)

    def test_認証ヘッダーを外部配信先へ渡さない(self):
        request = SETUP.urllib.request.Request("https://huggingface.co/model", headers={"Authorization": "Bearer dummy"})
        redirected = SETUP.PrivateRedirect().redirect_request(request, None, 302, "", {}, "https://cdn.example/model")
        self.assertFalse(redirected.has_header("Authorization"))

    def test_管理領域外への保存を拒否する(self):
        with self.assertRaisesRegex(ValueError, "管理ディレクトリ外"):
            SETUP.safe_path(HERE, "../../escape")

    def test_進捗表示の一時ロックでは本体取得を失敗させない(self):
        with tempfile.TemporaryDirectory(dir=HERE.parents[2] / "temp") as directory:
            path = Path(directory) / "progress.json"
            with patch.object(Path, "replace", side_effect=PermissionError("表示競合")), patch.object(SETUP.time, "sleep"):
                SETUP.progress(path, "取得中", 1, 2)
            self.assertFalse(path.with_suffix(".tmp").exists())

    def test_途中転送を正しい位置から再開し検証後だけ公開する(self):
        with tempfile.TemporaryDirectory(dir=HERE.parents[2] / "temp") as directory:
            root = Path(directory)
            first = io.BytesIO(b"abc")
            first.status = 200
            first.headers = {}
            second = io.BytesIO(b"def")
            second.status = 206
            second.headers = {"Content-Range": "bytes 3-5/6"}
            item = ("model.bin", "https://example.com/model", hashlib.sha256(b"abcdef").hexdigest(), 6)
            with patch.object(SETUP, "remote", side_effect=[first, second]) as network, patch.object(SETUP.time, "sleep"):
                SETUP.download(root / "models", root / "cache", item, 1, 1, root / "progress.json", 6, 0)
            self.assertEqual(network.call_args_list[1].kwargs["headers"], {"Range": "bytes=3-"})
            self.assertEqual((root / "models/model.bin").read_bytes(), b"abcdef")
            self.assertFalse((root / "cache/model.bin.part").exists())

    def test_再開位置が違う応答を既存途中データへ結合しない(self):
        with tempfile.TemporaryDirectory(dir=HERE.parents[2] / "temp") as directory:
            root = Path(directory)
            pending = SETUP.safe_path(root / "cache", "model.bin.part")
            pending.write_bytes(b"abc")
            response = io.BytesIO(b"abcdef")
            response.status = 206
            response.headers = {"Content-Range": "bytes 0-5/6"}
            item = ("model.bin", "https://example.com/model", hashlib.sha256(b"abcdef").hexdigest(), 6)
            with patch.object(SETUP, "remote", return_value=response):
                with self.assertRaisesRegex(ValueError, "再開位置"):
                    SETUP.download(root / "models", root / "cache", item, 1, 1, root / "progress.json", 6, 0)
            self.assertEqual(pending.read_bytes(), b"abc")
            self.assertFalse((root / "models/model.bin").exists())


if __name__ == "__main__":
    unittest.main()
