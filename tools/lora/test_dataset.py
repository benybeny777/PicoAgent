"""教材の分割、根拠、誤答の扱いとツール引数を独立に検査する。"""

import copy
import json
import unittest

import dataset


class 教材検査(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = dataset.generate_dataset()

    def case(self, category, *, split="train", suffix="0"):
        return copy.deepcopy(next(c for c in self.data["cases"] if c["category"] == category and c["split"] == split and c["id"].endswith("-" + suffix)))

    def rejects(self, case, message):
        # 改変した一行だけでも対検査ができるよう、元の集合へ戻す。
        data = copy.deepcopy(self.data)
        index = next(i for i, value in enumerate(data["cases"]) if value["id"] == case["id"])
        data["cases"][index] = case
        with self.assertRaisesRegex(ValueError, message):
            dataset.validate_dataset(data)

    def test_学習と開発と未使用最終の件数を固定する(self):
        self.assertEqual({k: v["count"] for k, v in self.data["metadata"]["splits"].items()}, {"train": 710, "dev": 86, "final": 100})
        self.assertEqual(self.data["metadata"]["splits"]["final"]["categories"]["conversation"], 35)
        self.assertEqual(sum(self.data["metadata"]["splits"]["train"]["categories"][k] for k in dataset.ATTRIBUTION), 300)

    def test_同じ版の最終評価を結果に合わせて変更しない(self):
        self.assertEqual(dataset.VERSION, "1")
        self.assertEqual(self.data["metadata"]["splits"]["final"]["sha256"], "952786f92876e99f2013041f26da56d8f05bed58b0ef73bca5cd9b82f65f7926")

    def test_実行を繰り返しても教材と最終評価は同じになる(self):
        self.assertEqual(dataset.digest(self.data), dataset.digest(dataset.generate_dataset()))

    def test_値交換の対は主体の正解も交換する(self):
        a = self.case("user_fact")
        b = self.case("user_fact", suffix="1")
        self.assertEqual(a["question"], b["question"])
        self.assertEqual(a["group_id"], b["group_id"])
        self.assertIn("ジャスミンティー", a["target"]["content"])
        self.assertIn("ほうじ茶", b["target"]["content"])
        self.assertEqual(a["expected"]["facts"][0]["owner"], "user")
        self.assertEqual(b["expected"]["facts"][0]["owner"], "user")

    def test_派生元が分割をまたいだら拒否する(self):
        case = self.case("user_fact", split="final")
        case["group_id"] = self.case("user_fact")["group_id"]
        self.rejects(case, "派生元")

    def test_名前と事実の値は各分割と既存四十問から隔離する(self):
        summaries = self.data["metadata"]["splits"]
        for field in ("entities", "values"):
            for left, right in (("train", "dev"), ("train", "final"), ("dev", "final")):
                self.assertFalse(set(summaries[left][field]) & set(summaries[right][field]))
        names = {x for info in summaries.values() for x in info["entities"]}
        values = {x for info in summaries.values() for x in info["values"]}
        self.assertFalse(names & set(dataset.RESERVED_NAMES))
        self.assertFalse(values & set(dataset.RESERVED_VALUES))
        case = self.case("user_fact")
        case["question"] = "ゆうさんの好みを教えて。"
        self.rejects(case, "既存開発評価")

    def test_未知と明示的な不定と既知の回数を区別する(self):
        unknown = self.case("unknown_frequency")
        known = self.case("known_frequency")
        explicit = self.case("explicit_none")
        self.assertIn("分かりません", unknown["target"]["content"])
        self.assertNotIn("決めていません", unknown["target"]["content"])
        self.assertEqual(unknown["expected"]["unknown"], [{"owner": "character", "kind": "頻度"}])
        self.assertIn("月に3回", known["target"]["content"])
        self.assertFalse(known["expected"]["unknown"])
        self.assertIn("回数は決めていません", explicit["target"]["content"])
        self.assertEqual(explicit["expected"]["facts"][0]["status"], "explicit_none")
        known["expected"]["unknown"] = [{"owner": "character", "kind": "頻度"}]
        self.rejects(known, "既知情報を不明")

    def test_根拠のない正解と資料の所有主体の逆転を拒否する(self):
        case = self.case("user_fact")
        case["memories"][1] = "この記録には好みがありません。"
        self.rejects(case, "正解の根拠")
        case = self.case("user_fact")
        case["provenance"]["facts"][0]["owner"] = "character"
        self.rejects(case, "資料と所有主体")

    def test_知らない回数を決めていないと言い換えた教材を拒否する(self):
        case = self.case("unknown_user")
        case["target"]["content"] = "あなたの好みは決めていません。"
        self.rejects(case, "未知を不存在")

    def test_動詞と名乗りに名詞用の確認語尾を誤結合しない(self):
        for case in self.data["cases"]:
            if case["split"] != "train":
                continue
            content = case["target"]["content"]
            self.assertNotRegex(content, r"楽しむ(?:です|だよ|だね)")
            if case["category"] == "identity":
                self.assertNotIn(f"は{case['character']['name']}だね", content)
        self.assertEqual(self.case("known_frequency")["target"]["content"], "ほうじ茶は月に3回楽しんでいます。")

    def test_趣味の回答は単なる好みから推測せず明示資料を使う(self):
        case = next(c for c in self.data["cases"] if c["id"] == "train-02-user_fact-0")
        self.assertIn("私の趣味は木工です。", case["memories"])
        self.assertIn("趣味は木工", case["target"]["content"])
        self.assertNotIn("どんな趣味が好き", case["question"])

    def test_ファイル指定の確認は読込ツールがある環境だけで教える(self):
        for case in self.data["cases"]:
            if case["category"] == "clarification" and case["id"].endswith("-0"):
                self.assertEqual(case["tool_names"], ["read_file"])
                self.assertNotIn("tool_calls", case["target"])

    def test_過去の誤答は履歴であり正解ではない(self):
        case = self.case("correction")
        self.assertEqual(case["provenance"]["masked_history_indices"], [1])
        self.assertIn("ほうじ茶", case["history"][1]["content"])
        self.assertNotIn("ほうじ茶", case["target"]["content"])
        self.assertIn("ジャスミンティー", case["target"]["content"])
        case["target"]["content"] = case["history"][1]["content"]
        self.rejects(case, "正解と根拠")

    def test_引用は話者と相手を分け不明な相手を補わない(self):
        speaker = self.case("quote_speaker")
        recipient = self.case("quote_recipient")
        unknown = self.case("quote_unknown")
        self.assertEqual(speaker["expected"]["facts"][0]["owner"], "speaker")
        self.assertEqual(recipient["expected"]["facts"][0]["owner"], "recipient")
        self.assertEqual(recipient["expected"]["facts"][0]["status"], "reported")
        self.assertIn("述べています", recipient["target"]["content"])
        self.assertFalse(unknown["expected"]["facts"])
        self.assertIn("分かりません", unknown["target"]["content"])

    def test_日常支援にも形式指定と不足確認と継続と失敗報告を含む(self):
        for split in dataset.SPLITS:
            cases = [c for c in self.data["cases"] if c["split"] == split]
            categories = {c["category"] for c in cases}
            self.assertTrue({"format", "clarification", "continuity", "conversation", "tool_call", "tool_result", "tool_error", "tool_unavailable"}.issubset(categories))
            for case in cases:
                self.assertTrue(case["expected"]["manual_rubric"])
        case = self.case("format", suffix="1")
        self.assertEqual(json.loads(case["target"]["content"]), {"status": "準備中"})
        self.assertEqual(self.case("clarification", suffix="3")["target"]["content"], "開始は15時20分です。")
        self.assertIn("分かりません", self.case("clarification", suffix="5")["target"]["content"])

    def test_ツールの途中に余計な利用者ターンを入れない(self):
        case = self.case("tool_result")
        self.assertEqual([m["role"] for m in case["continuation"]], ["assistant", "tool"])
        call = case["continuation"][0]["tool_calls"][0]
        self.assertIsInstance(call["arguments"], dict)
        self.assertEqual(call["id"], case["continuation"][1]["tool_call_id"])
        target = self.case("tool_call")["target"]["tool_calls"][0]
        self.assertIsInstance(target["function"]["arguments"], str)
        self.assertEqual(json.loads(target["function"]["arguments"]), {"location": "金沢市"})
        case["continuation"][1]["tool_call_id"] = "別の呼出"
        self.rejects(case, "対応しない")

    def test_計算結果と失敗結果を独立に照合する(self):
        case = self.case("tool_result", suffix="2")
        self.assertEqual(case["target"]["content"], "4.94%です。")
        self.assertIn("13 / 263 = 4.94%", case["continuation"][-1]["content"])
        error = self.case("tool_error", suffix="1")
        self.assertIn("許可ルート外", error["continuation"][-1]["content"])
        self.assertIn("読めませんでした", error["target"]["content"])
        self.assertNotIn("未着手", error["target"]["content"])

    def test_正解の架空ツールと不正JSON引数を拒否する(self):
        case = self.case("tool_call")
        case["target"]["tool_calls"][0]["function"]["name"] = "架空ツール"
        self.rejects(case, "正解ツール呼出")
        case = self.case("tool_call")
        case["target"]["tool_calls"][0]["function"]["arguments"] = "壊れたJSON"
        self.rejects(case, "Expecting value")

    def test_実schema検査前に本番bodyの生成を必須にする(self):
        with self.assertRaisesRegex(ValueError, "Rustでbody"):
            dataset.validate_tool_arguments({"cases": [self.case("tool_call")]})

    def test_渡された実schemaで必須と余分なキーと型を検査する(self):
        # この小さなschemaは検査器の境界テスト用。製品schemaの代用にはしない。
        case = self.case("tool_call")
        case["tool_names"] = ["weather"]
        case["body"] = {"tools": [{"type": "function", "function": {"name": "weather", "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]}}}]}
        dataset.validate_tool_arguments({"cases": [case]})
        for args, error in (({}, "必須"), ({"location": 12}, "型"), ({"location": "金沢市", "invented": True}, "schemaにない")):
            bad = copy.deepcopy(case)
            bad["target"]["tool_calls"][0]["function"]["arguments"] = json.dumps(args)
            with self.assertRaisesRegex(ValueError, error):
                dataset.validate_tool_arguments({"cases": [bad]})


if __name__ == "__main__":
    unittest.main()
