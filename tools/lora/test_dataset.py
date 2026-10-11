"""教材の分割、根拠、誤答の扱いとツール引数を独立に検査する。"""

import copy
import json
import unittest
from collections import Counter, defaultdict
from pathlib import Path

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
        self.assertEqual({k: v["count"] for k, v in self.data["metadata"]["splits"].items()}, {"train": 1058, "dev": 86, "final": 100})
        self.assertEqual(self.data["metadata"]["splits"]["final"]["categories"]["conversation"], 35)
        self.assertEqual(sum(self.data["metadata"]["splits"]["train"]["categories"][k] for k in dataset.ATTRIBUTION), 300)

    def test_同じ版の最終評価を結果に合わせて変更しない(self):
        self.assertEqual(dataset.VERSION, "1")
        self.assertEqual(self.data["metadata"]["splits"]["final"]["sha256"], "952786f92876e99f2013041f26da56d8f05bed58b0ef73bca5cd9b82f65f7926")

    def test_実行を繰り返しても教材と最終評価は同じになる(self):
        self.assertEqual(dataset.digest(self.data), dataset.digest(dataset.generate_dataset()))

    def additions(self, category=None):
        return [c for c in self.data["cases"] if c["provenance"].get("extension") == 3 and (category is None or c["category"] == category)]

    def test_追加しても既存八百九十六件の正解と入力を変えない(self):
        fixed = {
            "train": (710, "0afbf6b11c4293a9b64fa5892ebe4b4f509d36c5e996f954ba9e2e403ae9d041"),
            "dev": (86, "61f8e3e4372bc2e11e9485283b9707d86b07f72c3701dcf244313780d10ec9ba"),
            "final": (100, "952786f92876e99f2013041f26da56d8f05bed58b0ef73bca5cd9b82f65f7926"),
        }
        for split, (count, digest) in fixed.items():
            cases = [c for c in self.data["cases"] if c["split"] == split and c["provenance"].get("extension") != 3]
            self.assertEqual(len(cases), count)
            self.assertEqual(dataset.digest(cases), digest)
        self.assertEqual(Counter(c["category"] for c in self.additions()), {"multi_attribute": 192, "format_selection": 48, "history_selection": 48, "conversation_distinction": 36, "tool_chain": 24})
        self.assertTrue(all(c["split"] == "train" for c in self.additions()))

    def test_複数属性の中から主体と項目を選び既知と未知を区別する(self):
        for case in self.additions("multi_attribute"):
            ledger = case["provenance"]["facts"]
            self.assertGreaterEqual(len({e["kind"] for e in ledger if e["kind"] in ("飲み物", "食べ物", "趣味")}), 3)
            requests = {tuple(pair) for pair in case["provenance"]["selection"]["requests"]}
            selected = {(e["owner"], e["kind"]) for e in case["expected"]["facts"] + case["expected"]["unknown"]}
            self.assertEqual(requests, selected)
            for expected in case["expected"]["facts"]:
                self.assertIn(expected, ledger)
            for unknown in case["expected"]["unknown"]:
                self.assertFalse(any((e["owner"], e["kind"]) == (unknown["owner"], unknown["kind"]) for e in ledger))
        material = json.loads((Path(dataset.__file__).parent / "fixtures/selection-v3.json").read_text(encoding="utf-8"))
        original = material["worlds"][0]
        changed = copy.deepcopy(original)
        changed["attributes"]["飲み物"] = {"user": original["attributes"]["飲み物"]["character"], "character": original["attributes"]["飲み物"]["user"]}
        before, after = dataset.additional_attribution(original, 0), dataset.additional_attribution(changed, 0)
        self.assertEqual(before[0]["question"], after[0]["question"])
        self.assertIn("玄米茶", before[0]["target"]["content"])
        self.assertIn("ハトムギ茶", after[0]["target"]["content"])
        self.assertIn("玄米茶", after[3]["target"]["content"])

    def test_同じ属性の値ありと欠落と明示的なしで正解が変わる(self):
        pairs = defaultdict(list)
        for case in self.additions("multi_attribute"):
            if key := case["provenance"].get("contrast_id"):
                pairs[key].append(case)
        self.assertEqual(len(pairs), 24)
        for pair in pairs.values():
            self.assertEqual(len(pair), 2)
            left, right = pair
            self.assertEqual(left["question"], right["question"])
            self.assertEqual(left["provenance"]["selection"]["requests"], right["provenance"]["selection"]["requests"])
            self.assertNotEqual(left["target"], right["target"])
            self.assertFalse(left["expected"]["unknown"])
            self.assertTrue(right["expected"]["unknown"] or right["expected"]["facts"][0]["status"] == "explicit_none")

    def test_人格の四書式と三つの一人称を保ち記憶の位置を固定しない(self):
        cases = self.additions("multi_attribute")
        self.assertEqual(Counter(c["provenance"]["profile_format"] for c in cases), {"fields": 48, "prose": 48, "bullets": 48, "long": 48})
        self.assertEqual({c["provenance"]["first_person"] for c in cases}, {"私", "僕", "わたし"})
        positions = set()
        for case in cases:
            first = case["provenance"]["first_person"]
            profile = case["character"]["profile"]
            self.assertTrue(any(value in profile for value in (f"一人称: {first}", f"自分を{first}と呼びます", f"自分を指す言葉は{first}")))
            if case["provenance"]["profile_format"] == "long":
                self.assertGreater(len(profile), 600)
            for item in case["expected"]["facts"] + case["expected"]["unknown"]:
                if item["owner"] == "character":
                    self.assertIn(first + "の", case["target"]["content"])
                if item["owner"] == "user" and item.get("source", "").startswith("/memories/"):
                    positions.add(item["source"])
        self.assertGreaterEqual(len(positions), 5)

    def test_形式の値とキーと項目数と出力順を変える(self):
        cases = self.additions("format_selection")
        self.assertGreaterEqual(len({c["target"]["content"] for c in cases}), 42)
        self.assertEqual(len({next(iter(c["provenance"]["selection"]["mapping"])) for c in cases if c["provenance"]["selection"]["operation"] == "json_one"}), 6)
        self.assertEqual({len(c["target"]["content"].splitlines()) for c in cases if c["provenance"]["selection"]["operation"] == "bullets"}, {2, 3, 4})
        samples = {c["id"]: c for c in cases}
        self.assertEqual(json.loads(samples["train-v3-format_selection-00-01"]["target"]["content"]), {"phase": "仮置き", "at": "07時42分"})
        self.assertEqual(samples["train-v3-format_selection-01-04"]["target"]["content"], "11時08分")
        self.assertEqual(samples["train-v3-format_selection-02-04"]["target"]["content"], "小ホールの端")
        self.assertEqual(samples["train-v3-format_selection-00-05"]["target"]["content"], "厚紙、布テープ、輪ゴム、紙ひも")
        self.assertEqual(samples["train-v3-format_selection-00-07"]["target"]["content"], "「端を差し込む」を先にしてから「紙をそろえる」を行います。")

    def test_履歴の次項目は二番目固定でなく訂正や省略に応じて変わる(self):
        for case in self.additions("history_selection"):
            selection = case["provenance"]["selection"]
            variant = int(case["id"].rsplit("-", 1)[1])
            plan = selection["steps"]
            expected = [1, 2, 1, 2, 2, 0, len(plan) - 1, None][variant]
            self.assertEqual(selection["selected"], [] if expected is None else [expected])
            if expected is None:
                self.assertIn("すべて済んでいます", case["target"]["content"])
            else:
                self.assertIn(plan[expected], case["target"]["content"])
                for i, step in enumerate(plan):
                    if i != expected:
                        self.assertNotIn(step, case["target"]["content"])

    def test_会話の相手と時点と指示対象が変わる十八対を持つ(self):
        pairs = defaultdict(list)
        for case in self.additions("conversation_distinction"):
            pairs[case["provenance"]["contrast_id"]].append(case)
        self.assertEqual(len(pairs), 18)
        for pair in pairs.values():
            self.assertEqual(len(pair), 2)
            self.assertEqual(pair[0]["group_id"], pair[1]["group_id"])
            self.assertNotEqual(pair[0]["question"], pair[1]["question"])
            self.assertNotEqual(pair[0]["target"], pair[1]["target"])
            self.assertNotEqual(pair[0]["provenance"]["selection"], pair[1]["provenance"]["selection"])
        tomorrow, today = pairs["train-v3-conversation-03"]
        self.assertIn("箱詰めは明日", tomorrow["target"]["content"])
        self.assertIn("箱詰めは今日", today["target"]["content"])
        rest, work = pairs["train-v3-conversation-15"]
        self.assertIn("あなたは休み", rest["target"]["content"])
        self.assertIn("あなたは道具を数える", work["target"]["content"])

    def test_ツール連鎖は一つの往復表を段階的に開き未取得情報を渡さない(self):
        groups = defaultdict(list)
        for case in self.additions("tool_chain"):
            groups[case["provenance"]["selection"]["chain_id"]].append(case)
        self.assertEqual(len(groups), 6)
        for cases in groups.values():
            self.assertEqual(len(cases), 4)
            whole = cases[-1]["continuation"]
            self.assertEqual([whole[i]["tool_calls"][0]["name"] for i in (0, 2, 4)], ["read_file", "productivity_tools", "weather"])
            for stage, case in enumerate(cases):
                self.assertEqual(case["continuation"], whole[:stage * 2])
                self.assertEqual(case["question"], cases[0]["question"])
                if stage < 3:
                    target = case["target"]["tool_calls"][0]["function"]
                    prior_call = whole[stage * 2]["tool_calls"][0]
                    self.assertEqual(target["name"], prior_call["name"])
                    self.assertEqual(json.loads(target["arguments"]), prior_call["arguments"])
                    self.assertEqual(case["target"]["content"], "")
            fields = {line.split("\t", 1)[1].split("=", 1)[0]: line.split("=", 1)[1] for line in whole[1]["content"].splitlines() if "\t" in line and "=" in line}
            args = whole[2]["tool_calls"][0]["arguments"]
            self.assertEqual((args["value"], args["total"]), (int(fields["value"]), int(fields["total"])))
            self.assertEqual(whole[4]["tool_calls"][0]["arguments"], {"location": fields["location"]})
            self.assertIn(f"{args['value'] / args['total'] * 100:.2f}%", cases[-1]["target"]["content"])

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

    def test_本番根拠検査前に本番bodyの生成を必須にする(self):
        with self.assertRaisesRegex(ValueError, "Rustでbody"):
            dataset.validate_exported_evidence({"cases": [self.case("user_fact")]})

    def test_ペットの根拠が除外されたら同じ値が別主体にあっても拒否する(self):
        case = copy.deepcopy(next(c for c in self.additions("multi_attribute") if c["id"] == "train-v3-multi_attribute-00-14"))
        source = dataset.pointer(case, case["expected"]["facts"][0]["source"])
        self.assertEqual(source, "私はウサギを飼っています。")
        case["body"] = {"messages": [
            {"role": "system", "content": "キャラクターの好きな動物はウサギです。"},
            {"role": "user", "content": case["question"]},
        ]}
        with self.assertRaisesRegex(ValueError, "根拠が本番bodyから除外"):
            dataset.validate_exported_evidence({"cases": [case]})
        case["body"]["messages"][1]["content"] = source + "\n" + case["question"]
        dataset.validate_exported_evidence({"cases": [case]})

    def test_正解に使わない匿名記憶は本番入力から除外できる(self):
        case = self.case("anonymous")
        source = dataset.pointer(case, case["expected"]["facts"][0]["source"])
        case["body"] = {"messages": [
            {"role": "system", "content": case["character"]["profile"]},
            {"role": "user", "content": source + "\n" + case["question"]},
        ]}
        self.assertNotIn(case["memories"][0], source)
        dataset.validate_exported_evidence({"cases": [case]})

    def test_人格と質問と履歴でも値の一致だけでは根拠を保存したと扱わない(self):
        history = copy.deepcopy(self.additions("history_selection")[0])
        for case in (self.case("character_fact"), self.case("quote_speaker"), history):
            entry = case["expected"]["facts"][0]
            source = dataset.pointer(case, entry["source"])
            case["body"] = {"messages": [{"role": "user", "content": source}]}
            dataset.validate_exported_evidence({"cases": [case]})
            case["body"]["messages"][0]["content"] = entry["value"]
            with self.assertRaisesRegex(ValueError, "根拠が本番bodyから除外"):
                dataset.validate_exported_evidence({"cases": [case]})

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
