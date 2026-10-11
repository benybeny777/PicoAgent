"""実モデルを起動せず、LoRA適用の差・欠落・4起動の分離を検査する。"""

import copy
import json
from contextlib import contextmanager
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import probe


def response(delta=0, content="和"):
    tokens = ["和", "私", "あ", "そ", "今"]
    top = [{"id": 300 + i, "token": token, "bytes": list(token.encode("utf-8")), "logprob": -0.4 - i + delta}
           for i, token in enumerate(tokens)]
    return {"choices": [{"message": {"role": "assistant", "content": content},
                         "finish_reason": "length", "logprobs": {"content": [{**top[0], "top_logprobs": top}]}}],
            "__verbose": {"tokens": [300, 900, 901, 902]},
            "usage": {"prompt_tokens": 32, "completion_tokens": 4}}


def result(delta=0, content="和"):
    return {"content": content, "raw_response": response(delta, content),
            "request": {"temperature": 0, "seed": 1, "max_tokens": 4, "logprobs": True, "top_logprobs": 5}}


class 適用検査(unittest.TestCase):
    def test_同じ本文でも共通tokenの非ゼロ差があれば合格する(self):
        verdict = probe.compare(result(), result(0.02))
        self.assertTrue(verdict["passed"])
        self.assertAlmostEqual(verdict["max_abs_logprob_delta"], 0.02)
        self.assertEqual(len(verdict["common_candidates"]), 5)

    def test_本文だけ変わってもlogprobがゼロ差なら不合格にする(self):
        self.assertFalse(probe.compare(result(), result(content="私"))["passed"])
        self.assertFalse(probe.compare(result(), result(1e-7))["passed"])

    def test_無応答やlogprob欠落や非有限値を合格にしない(self):
        for mode in ("空", "欠落", "候補なし", "無限大", "未数値"):
            with self.subTest(条件=mode):
                bad = result()
                first = bad["raw_response"]["choices"][0]
                if mode == "空":
                    bad["content"] = ""
                elif mode == "欠落":
                    first.pop("logprobs")
                elif mode == "候補なし":
                    first["logprobs"]["content"][0]["top_logprobs"] = []
                elif mode == "無限大":
                    first["logprobs"]["content"][0]["logprob"] = float("inf")
                else:
                    first["logprobs"]["content"][0]["logprob"] = None
                with self.assertRaises(ValueError):
                    probe.compare(result(), bad)

    def test_共通候補なしと異なる要求を比較しない(self):
        bad = result()
        for entry in bad["raw_response"]["choices"][0]["logprobs"]["content"]:
            entry["id"] = 500
            for i, candidate in enumerate(entry["top_logprobs"]):
                candidate["id"] = 500 + i
        bad["raw_response"]["__verbose"]["tokens"][0] = 500
        with self.assertRaisesRegex(ValueError, "共通"):
            probe.compare(result(), bad)

    def test_UTF8の先行バイトtokenが同一なら最初の記録位置を比較できる(self):
        base, adapted = result(), result(0.02)
        for value in (base, adapted):
            value["raw_response"]["__verbose"]["tokens"] = [198, 300, 900, 901]
            first = value["raw_response"]["choices"][0]["logprobs"]["content"][0]
            first["top_logprobs"][0]["bytes"] = [140]
            first["top_logprobs"][0]["token"] = ""
        verdict = probe.compare(base, adapted)
        self.assertTrue(verdict["passed"])
        self.assertEqual(verdict["base_position"]["index"], 1)
        self.assertEqual(verdict["base_position"]["prefix_token_ids"], [198])
        adapted["raw_response"]["__verbose"]["tokens"][0] = 197
        with self.assertRaisesRegex(ValueError, "先行token列"):
            probe.compare(base, adapted)

    def test_実token列欠落と曖昧な生成位置を合格にしない(self):
        for tokens in (None, [300, 300, 900], [900, 901, 902], [198, 300, 900]):
            bad = result(0.02)
            bad["raw_response"]["__verbose"]["tokens"] = tokens
            with self.assertRaises(ValueError):
                probe.compare(result(), bad)
        bad = result()
        bad["request"]["seed"] = 2
        with self.assertRaisesRegex(ValueError, "生成要求"):
            probe.compare(result(), bad)

    @contextmanager
    def environment(self, delta=0.02, missing=False, gpu_offload=True):
        directory = Path(__file__).resolve().parents[2] / "temp" / "lora-probe-tests"
        directory.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            root = Path(temporary)
            files = {key: root / (key + suffix) for key, suffix in (("server", ".exe"), ("model", ".gguf"), ("template", ".jinja"), ("adapter", ".gguf"))}
            for key, path in files.items():
                path.write_text(key, encoding="utf-8")
            cases = [{"id": f"case-{split}", "split": split,
                      "target": {"role": "assistant", "content": "正解"},
                      "body": {"messages": [{"role": "user", "content": "私の名前は？"}], "temperature": 0.6}}
                     for split in ("train", "dev", "final")]
            source = root / "input.json"
            source.write_text(json.dumps({"cases": cases}), encoding="utf-8")
            argv = [value for key, path in files.items() for value in ("--" + key, str(path))]
            argv += ["--input", str(source), "--output", str(root / "probe.json")]
            args = probe.parse_args(argv)
            started, stopped, bodies = [], [], []

            @contextmanager
            def server(settings, log):
                index = len(started)
                started.append((settings.gpu_layers, settings.adapter, log))
                self.assertEqual(settings.verbosity, 5)
                count = 24 if settings.gpu_layers and gpu_offload else 0
                log.write_text(f"llama_model_load: offloaded {count}/24 layers to GPU\n", encoding="utf-8")
                client = mock.Mock()

                def answer(endpoint, body, timeout):
                    self.assertEqual(endpoint, "/v1/chat/completions")
                    bodies.append(copy.deepcopy(body))
                    raw = response(delta if settings.adapter is not None else 0)
                    if missing:
                        raw["choices"][0].pop("logprobs")
                    return raw

                client.json.side_effect = answer
                try:
                    yield client, {"pid": 100 + index, "arguments": ["--gpu-layers", str(settings.gpu_layers)]}
                finally:
                    stopped.append(index)

            with mock.patch.object(probe.runner, "local_server", side_effect=server):
                yield args, started, stopped, bodies

    def test_四起動を順番に分離し固定要求とSHAとログと判定を保存する(self):
        with self.environment() as (args, started, stopped, bodies):
            document = probe.run(args)
            self.assertTrue(document["passed"])
            self.assertEqual(document["status"], "completed")
            self.assertEqual([(layers, adapter is not None) for layers, adapter, _ in started], [(0, False), (0, True), (99, False), (99, True)])
            self.assertEqual(stopped, [0, 1, 2, 3])
            self.assertEqual(document["parameters"]["case_ids"], ["case-train"])
            self.assertEqual(len(document["comparisons"]), 2)
            for body in bodies:
                for key, expected in (("temperature", 0), ("seed", 1), ("max_tokens", 4), ("logprobs", True), ("top_logprobs", 5), ("cache_prompt", False), ("verbose", True), ("return_tokens", True), ("post_sampling_probs", False)):
                    self.assertEqual(body[key], expected)
            for record in document["files"].values():
                self.assertEqual(len(record["sha256"]), 64)
            for run in document["runs"]:
                self.assertEqual(len(run["log_file"]["sha256"]), 64)
                self.assertEqual(run["results"][0]["request"]["temperature"], 0)
            saved = json.loads(args.output.read_text(encoding="utf-8"))
            self.assertTrue(saved["passed"])
            self.assertEqual(saved["runs"][1]["results"][0]["raw_response"], response(0.02))

    def test_適用検査の詳細ログ指定は呼出元の設定を書き換えない(self):
        with self.environment() as (args, _, _, _):
            args.verbosity = 0
            document = probe.run(args)
            self.assertTrue(document["passed"])
            self.assertEqual(args.verbosity, 0)

    def test_ゼロ差とlogprob欠落とGPU未使用を結果JSONでも不合格にする(self):
        for options in ({"delta": 0}, {"missing": True}, {"gpu_offload": False}):
            with self.subTest(条件=options), self.environment(**options) as (args, started, stopped, _):
                document = probe.run(args)
                self.assertFalse(document["passed"])
                self.assertEqual(document["status"], "failed")
                self.assertEqual(len(started), 4)
                self.assertEqual(len(stopped), 4)
                self.assertFalse(json.loads(args.output.read_text(encoding="utf-8"))["passed"])

    def test_既存結果を上書きせずサーバを起動しない(self):
        with self.environment() as (args, started, _, _):
            args.output.write_text("保持", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                probe.run(args)
            self.assertEqual(args.output.read_text(encoding="utf-8"), "保持")
            self.assertEqual(started, [])


if __name__ == "__main__":
    unittest.main()
