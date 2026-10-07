"""モデルを取得せず、記憶索引の更新・破損時の原本保護を検査する。"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2] / "src-tauri/resources/embedding-memory/worker.py"
spec = importlib.util.spec_from_file_location("memory_worker", SOURCE)
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class 記憶索引の検査(unittest.TestCase):
    def setUp(self):
        import torch
        self.torch = torch
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.search = worker.Search.__new__(worker.Search)
        self.search.torch = torch
        self.search.cache_path = Path(self.temp.name) / "vectors.json"
        self.search.cached = {}
        self.search.device = "cpu"
        self.encoded = []

        def encode(texts, prompt):
            if isinstance(texts, str):
                v = torch.zeros(worker.DIMENSIONS)
                v[0] = 1
                return v
            self.encoded.extend(texts)
            v = torch.zeros((len(texts), worker.DIMENSIONS))
            v[:, 0] = 1
            return v
        self.search.encode = encode

    def test_変更分だけ再計算し削除分は索引に残さない(self):
        self.search.search({"documents": ["猫", "犬"], "query": "猫"})
        self.search.search({"documents": ["犬", "猫"], "query": "犬"})
        self.assertEqual(self.encoded, ["猫", "犬"])
        self.search.search({"documents": ["猫", "鳥"], "query": "鳥"})
        self.assertEqual(self.encoded, ["猫", "犬", "鳥"])
        data = json.loads(self.search.cache_path.read_text(encoding="utf-8"))
        self.assertNotIn(worker.fingerprint("犬"), data["vectors"])
        self.assertNotIn("猫", self.search.cache_path.read_text(encoding="utf-8"))

    def test_空記憶は検索も保存もしない(self):
        self.assertEqual(self.search.search({"documents": [], "query": "猫"})["indices"], [])
        self.assertFalse(self.search.cache_path.exists())

    def test_件数と本文と質問の上限を拒否する(self):
        for body in [{"documents": ["猫"] * 201, "query": "猫"}, {"documents": ["猫" * 1001], "query": "猫"}, {"documents": ["猫"], "query": "猫" * 32769}]:
            with self.assertRaises(ValueError):
                self.search.search(body)

    def test_破損索引を退避して記憶原本を変更しない(self):
        original = Path(self.temp.name) / "memory.json"
        original.write_text("大事な記憶", encoding="utf-8")
        self.search.cache_path.write_text("{破損", encoding="utf-8")
        self.assertEqual(self.search.load_cache(), {})
        self.assertTrue(self.search.cache_path.with_suffix(".corrupt").exists())
        self.search.search({"documents": ["猫"], "query": "猫"})
        self.assertFalse(self.search.cache_path.with_suffix(".corrupt").exists())
        self.assertEqual(original.read_text(encoding="utf-8"), "大事な記憶")

    def test_非有限値と未正規化と次元違いを拒否する(self):
        for vector in [[0.0]*768, [float("nan")]*768, [1.0]*5]:
            with self.assertRaises(ValueError):
                self.search.validate(vector)

    def test_revision変更時は既存ベクトルを再利用しない(self):
        self.search.cache_path.write_text(json.dumps({"revision": "旧版", "vectors": {}}), encoding="utf-8")
        self.assertEqual(self.search.load_cache(), {})


if __name__ == "__main__":
    unittest.main()
