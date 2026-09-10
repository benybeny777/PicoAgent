"""部位の所有範囲と原画照合を回帰検査する。"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


builder = load("build-mesh-rig")
importer = load("import-face-landmarks")


class FaceLandmarkTests(unittest.TestCase):
    def test_開口差分の目領域は完全に透明(self):
        anchors = {"mouth": {"x0": 80, "y0": 140, "x1": 120, "y1": 160}}
        region = builder.measured_mouth_region(anchors, (200, 240))
        result = builder.feathered_region(Image.new("RGBA", (200, 240), "red"), region["box"], region["feather"])
        self.assertEqual(result.getchannel("A").crop((30, 50, 170, 110)).getbbox(), None)
        self.assertEqual(result.getpixel((100,150))[3],255)

    def test_異なる原画の解析を拒否し既存設定を保持(self):
        with tempfile.TemporaryDirectory(dir=ROOT/"temp") as folder:
            root = Path(folder)
            (root/"source/source").mkdir(parents=True)
            (root/"target/layers").mkdir(parents=True)
            (root/"target/mesh-rig/source").mkdir(parents=True)
            (root/"source/source/input.png").write_bytes(b"source")
            (root/"target/layers/mouth_closed.png").write_bytes(b"different")
            existing = root/"target/mesh-rig/source/face-landmarks.json"
            existing.write_text('{"preserved":true}',encoding="utf-8")
            with self.assertRaisesRegex(ValueError,"正本が一致"):
                importer.import_landmarks(root/"source",root/"target")
            self.assertEqual(json.loads(existing.read_text()),{"preserved":True})


if __name__ == "__main__":
    unittest.main()
