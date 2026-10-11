"""全入力の照合と、不完全な証拠を合格にしないことを検査する。"""

import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import parity
from runner import file_record
from training import prepare_case


class Tokenizer:
    eos_token_id = ord("終")

    def apply_chat_template(self, messages, *, add_generation_prompt, **kwargs):
        rendered = "".join(message["role"] + ":" + message["content"] + "終" for message in messages)
        return rendered + "assistant:" if add_generation_prompt else rendered

    def encode(self, text, **kwargs):
        return [ord(character) for character in text]


class BosTokenizer(Tokenizer):
    bos_token = "<BOS>"
    bos_token_id = 1

    def apply_chat_template(self, messages, **kwargs):
        return self.bos_token + super().apply_chat_template(messages, **kwargs)

    def encode(self, text, **kwargs):
        return [ord(character) for character in text.replace(self.bos_token, "\x01")]


class 照合検査(unittest.TestCase):
    def setUp(self):
        self.cases = [{"id": "最終入力", "split": "final", "body": {
            "messages": [{"role": "user", "content": "私の好きな飲み物は？"}], "tools": []},
            "target": {"role": "assistant", "content": "この正解はサーバへ送信しません。"}}]
        self.template = "本番テンプレート"
        self.files = {"input": {"sha256": "入力SHA"}, "template": {"sha256": "テンプレートSHA"}}
        prepared = prepare_case(self.cases[0], Tokenizer(), self.template, 4096)
        prompt = prepared["prompt"]
        self.inspection = {"mode": "inspect", "status": "completed", "files": self.files,
                           "expected_results": 1, "results": [{
                               "case_id": "最終入力", "status": "completed", "request": self.cases[0]["body"],
                               "prompt": prompt, "tokens": prepared["input_ids"][:prepared["prefix_length"]],
                               "tokenize_request": {"content": prompt, "add_special": True, "parse_special": True},
                           }]}

    def compare(self, inspection=None):
        inspected = parity.inspection_rows(inspection or self.inspection, self.cases, self.files)
        return parity.compare(self.cases, inspected, Tokenizer(), self.template, 4096)[0]

    def test_最終評価も正解を送信せず入力だけ全token照合する(self):
        before = copy.deepcopy(self.cases)
        result = self.compare()
        self.assertEqual(result["status"], "matched")
        self.assertEqual(result["hf"], result["llama"])
        self.assertNotIn("この正解", json.dumps(result, ensure_ascii=False))
        self.assertEqual(self.cases, before)

    def test_全文差分とtoken差分の最初の位置を独立して記録する(self):
        for target in ("prompt", "tokens"):
            with self.subTest(対象=target):
                changed = copy.deepcopy(self.inspection)
                row = changed["results"][0]
                if target == "prompt":
                    row["prompt"] += "余分"
                    row["tokenize_request"]["content"] = row["prompt"]
                else:
                    row["tokens"][-1] += 1
                result = self.compare(changed)
                self.assertEqual(result["status"], "mismatched")
                key = "first_prompt_difference" if target == "prompt" else "first_token_difference"
                self.assertIsNotNone(result[key])
                other_key = "first_token_difference" if target == "prompt" else "first_prompt_difference"
                self.assertIsNone(result[other_key])

    def test_同数の重複やIDの差し替えも拒否する(self):
        for bad in ("missing", "duplicate", "unknown", "incomplete", "evaluate"):
            with self.subTest(条件=bad):
                changed = copy.deepcopy(self.inspection)
                if bad == "missing":
                    changed["results"] = []
                elif bad == "duplicate":
                    self.cases.append({**copy.deepcopy(self.cases[0]), "id": "別入力"})
                    changed["expected_results"] = 2
                    changed["results"].append(copy.deepcopy(changed["results"][0]))
                elif bad == "unknown":
                    changed["results"][0]["case_id"] = "別ID"
                elif bad == "incomplete":
                    changed["status"] = "failed"
                else:
                    changed["mode"] = "evaluate"
                with self.assertRaises(ValueError):
                    parity.inspection_rows(changed, self.cases, self.files)
                self.cases = self.cases[:1]

    def test_来歴と本番bodyと特殊token設定の差し替えを拒否する(self):
        for bad in ("input", "template", "request", "special", "token_type"):
            with self.subTest(条件=bad):
                changed = copy.deepcopy(self.inspection)
                row = changed["results"][0]
                if bad in ("input", "template"):
                    changed["files"][bad]["sha256"] = "別SHA"
                elif bad == "request":
                    row["request"]["tools"] = [{"type": "function"}]
                elif bad == "special":
                    row["tokenize_request"]["add_special"] = False
                else:
                    row["tokens"][0] = True
                with self.assertRaises(ValueError):
                    parity.inspection_rows(changed, self.cases, self.files)

    def test_学習側が拒否したケースを一致扱いしない(self):
        self.cases[0]["target"]["content"] = ""
        result = self.compare()
        self.assertEqual(result["status"], "failed")
        self.assertIn("空の回答", result["error"])

    def bos_inspection(self, tokenizer):
        prepared = prepare_case(self.cases[0], tokenizer, self.template, 4096)
        inspection = copy.deepcopy(self.inspection)
        row = inspection["results"][0]
        row["prompt"] = prepared["prompt"][len(tokenizer.bos_token):]
        row["tokens"] = prepared["input_ids"][:prepared["prefix_length"]]
        row["tokenize_request"]["content"] = row["prompt"]
        return inspection

    def compare_bos(self, inspection, tokenizer):
        inspected = parity.inspection_rows(inspection, self.cases, self.files)
        return parity.compare(self.cases, inspected, tokenizer, self.template, 4096)[0]

    def test_単一bosのtoken追加を確認した場合だけ本文を復元し原文の証拠は残す(self):
        tokenizer = BosTokenizer()
        inspection = self.bos_inspection(tokenizer)
        before = copy.deepcopy(inspection)
        result = self.compare_bos(inspection, tokenizer)
        self.assertEqual(result["status"], "matched")
        self.assertTrue(result["bos_adjustment"]["applied"])
        self.assertEqual(result["bos_adjustment"]["bos_token_id"], 1)
        self.assertFalse(result["raw_prompt_match"])
        self.assertIsNotNone(result["first_raw_prompt_difference"])
        self.assertIsNone(result["first_prompt_difference"])
        self.assertEqual(result["hf"], result["llama_effective"])
        self.assertNotEqual(result["hf"]["prompt_sha256"], result["llama"]["prompt_sha256"])
        self.assertEqual(result["llama"]["prompt_sha256"], parity.value_digest(before["results"][0]["prompt"]))
        self.assertEqual(inspection, before)

    def test_bos以外の実内容差と余分bosと欠損tokenを一致にしない(self):
        tokenizer = BosTokenizer()
        for change in ("本文差", "余分BOS", "BOS欠損", "末尾token欠損", "別BOSのID", "原文とtokenの不整合"):
            with self.subTest(条件=change):
                inspection = self.bos_inspection(tokenizer)
                row = inspection["results"][0]
                if change == "本文差":
                    row["prompt"] += "余分な本文"
                    row["tokens"] = [tokenizer.bos_token_id] + tokenizer.encode(row["prompt"])
                elif change == "余分BOS":
                    row["prompt"] = tokenizer.bos_token + row["prompt"]
                    row["tokens"].insert(0, tokenizer.bos_token_id)
                elif change == "BOS欠損":
                    row["tokens"].pop(0)
                elif change == "末尾token欠損":
                    row["tokens"].pop()
                elif change == "別BOSのID":
                    row["tokens"][0] = 999
                else:
                    row["prompt"] += "token列へ反映されていない差分"
                row["tokenize_request"]["content"] = row["prompt"]
                result = self.compare_bos(inspection, tokenizer)
                self.assertEqual(result["status"], "mismatched")
                self.assertFalse(result["bos_adjustment"]["applied"])
                if change == "原文とtokenの不整合":
                    self.assertTrue(result["tokens_match"])
                    self.assertFalse(result["prompt_match"])
                else:
                    self.assertFalse(result["tokens_match"])

    def test_同じtokenへ正規化される本文差もbos補完後に拒否する(self):
        tokenizer = BosTokenizer()
        inspection = self.bos_inspection(tokenizer)
        row = inspection["results"][0]
        original = row["prompt"]
        row["prompt"] = original + " "
        row["tokenize_request"]["content"] = row["prompt"]
        original_encode = tokenizer.encode
        # token列だけでは検出できない正規化をするトークナイザも、全文で区別する。
        def normalized_encode(text, **kwargs):
            return original_encode(original if text == row["prompt"] else text, **kwargs)
        with mock.patch.object(tokenizer, "encode", side_effect=normalized_encode):
            result = self.compare_bos(inspection, tokenizer)
        self.assertTrue(result["bos_adjustment"]["applied"])
        self.assertTrue(result["tokens_match"])
        self.assertFalse(result["prompt_match"])
        self.assertEqual(result["status"], "mismatched")
        self.assertEqual(result["first_prompt_difference"]["llama"], " ")

    def test_bos文字列とidの不一致を推測で補わない(self):
        tokenizer = BosTokenizer()
        inspection = self.bos_inspection(tokenizer)
        with mock.patch.object(tokenizer, "bos_token_id", 2):
            result = self.compare_bos(inspection, tokenizer)
        self.assertEqual(result["status"], "mismatched")
        self.assertFalse(result["bos_adjustment"]["applied"])
        self.assertEqual(result["bos_adjustment"]["reason"], "BOS文字列とtoken IDの対応が一致しない")

    def test_実行結果を新規保存し既存結果を保持する(self):
        directory = Path(__file__).resolve().parents[2] / "temp" / "lora-parity-tests"
        directory.mkdir(parents=True, exist_ok=True)
        try:
            with tempfile.TemporaryDirectory(dir=directory) as temporary:
                root = Path(temporary)
                args = SimpleNamespace(input=root / "cases.json", inspection=root / "inspect.json",
                                       base=root / "base", template=root / "template.jinja",
                                       output=root / "parity.json", max_length=4096)
                args.base.mkdir()
                (args.base / "tokenizer.json").write_text("{}", encoding="utf-8")
                args.input.write_text(json.dumps({"cases": self.cases}), encoding="utf-8")
                args.template.write_text(self.template, encoding="utf-8")
                self.inspection["files"] = {name: file_record(getattr(args, name)) for name in ("input", "template")}
                args.inspection.write_text(json.dumps(self.inspection), encoding="utf-8")
                with mock.patch.object(parity, "load_tokenizer", return_value=Tokenizer()):
                    result = parity.run(args)
                    self.assertEqual(result["status"], "matched")
                    self.assertEqual(result["matched"], 1)
                    self.assertEqual(json.loads(args.output.read_text(encoding="utf-8"))["cases"], 1)
                    original = args.output.read_bytes()
                    with self.assertRaises(FileExistsError):
                        parity.run(args)
                    self.assertEqual(args.output.read_bytes(), original)
                    args.output = root / "mismatch.json"
                    self.inspection["results"][0]["tokens"].append(999)
                    args.inspection.write_text(json.dumps(self.inspection), encoding="utf-8")
                    argv = [part for key, value in vars(args).items()
                            for part in ("--" + key.replace("_", "-"), str(value))]
                    self.assertEqual(parity.main(argv), 1)
                    saved = json.loads(args.output.read_text(encoding="utf-8"))
                    self.assertEqual(saved["mismatched"], 1)
                    self.assertIsNotNone(saved["results"][0]["first_token_difference"])
                    args.output = root / "failed.json"
                    self.inspection["status"] = "failed"
                    args.inspection.write_text(json.dumps(self.inspection), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        parity.run(args)
                    self.assertEqual(json.loads(args.output.read_text(encoding="utf-8"))["status"], "failed")
        finally:
            directory.rmdir()


if __name__ == "__main__":
    unittest.main()
