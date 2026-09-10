import importlib.util
from pathlib import Path
import unittest


HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GEN = load("media_generate", HERE / "generate.py")
SETUP = load("media_setup", HERE / "setup_models.py")


class WorkflowTests(unittest.TestCase):
    def test_qwen_edit_keeps_input_and_prompt(self):
        workflow = GEN.qwen("帽子を赤くする", 12, 768, 512)
        self.assertEqual(workflow["4"]["inputs"]["image"], "input.png")
        self.assertEqual(workflow["7"]["inputs"]["prompt"], "帽子を赤くする")
        self.assertEqual(workflow["10"]["inputs"]["seed"], 12)

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
