"""教材の分割、根拠、誤答の扱いとツール引数を独立に検査する。"""

import copy
import hashlib
import json
import re
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


class 製品判定用追加教材検査(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.base = dataset.generate_dataset()
        cls.extended = dataset.generate_dataset("product-gate")
        cls.cases = [c for c in cls.extended["cases"] if c["provenance"].get("extension") == "product-gate"]
        cls.failures = [c for c in cls.cases if c["category"] == "gate_tool_failure"]

    def test_追加は明示指定だけで既存全ケースと固定材料を変えない(self):
        self.assertEqual(dataset.digest(self.base), "1b45ee2ed76d6c930a4406f1224264d66bc5c506e18b6f28846b338214faa71c")
        self.assertEqual(self.extended["cases"][:1244], self.base["cases"])
        self.assertEqual({k: v["count"] for k, v in self.extended["metadata"]["splits"].items()}, {"train": 1410, "dev": 86, "final": 100})
        original = json.dumps(self.cases[:304], ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
        self.assertEqual(hashlib.sha256(original).hexdigest(), "d98dad4b4b9db088c8b800ae0187ec268f50151d1e21b2129f33ae0bda977614")
        for name, expected in (("selection-v3.json", "26f1d5cc408df3dce09ef688acceacfb42f333d537786be22011cf95ccc899ed"), ("casual-challenge.json", "3a6fa14e6a5d413cacc2aed5e30ae9cc725ac9e4973c504c50b75f7d4d451a33")):
            path = Path(dataset.__file__).parent / "fixtures" / name
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)
        with self.assertRaisesRegex(ValueError, "未対応"):
            dataset.generate_dataset("unknown")

    def test_三百五十二件は学習専用で新人物と三つの一人称を持つ(self):
        self.assertEqual(Counter(c["category"] for c in self.cases), {"gate_ownership": 48, "gate_quote": 48, "gate_knowledge": 64, "gate_correction": 32, "gate_conversation": 48, "gate_tool_branch": 64, "gate_tool_failure": 48})
        self.assertTrue(all(c["split"] == "train" for c in self.cases))
        old_names = {name for c in self.base["cases"] for name in c["provenance"]["entities"].values()}
        names = {name for c in self.cases for name in c["provenance"]["entities"].values()}
        self.assertFalse(names & old_names)
        self.assertEqual({c["provenance"]["first_person"] for c in self.cases}, {"私", "僕", "わたし"})
        self.assertEqual({c["provenance"]["profile_format"] for c in self.cases}, {"fields", "prose", "bullets", "long"})
        for case in self.cases:
            text = dataset.canonical({key: case[key] for key in ("question", "target", "history")})
            self.assertNotRegex(text, r"\{(?:first|user|character|speaker|recipient|[a-z_]+)\}")

    def test_既知と未記載と未定は同じ質問で根拠だけを変える(self):
        groups = defaultdict(list)
        for case in self.cases:
            key = case["provenance"].get("contrast_id", "")
            if key.endswith(("start_status", "frequency_status")):
                groups[key].append(case)
        self.assertEqual(len(groups), 16)
        for cases in groups.values():
            self.assertEqual(len(cases), 3)
            self.assertEqual(len({c["question"] for c in cases}), 1)
            self.assertEqual(len({c["target"]["content"] for c in cases}), 3)
            known, missing, undecided = cases
            self.assertEqual(known["expected"]["facts"][0]["status"], "known")
            self.assertFalse(known["expected"]["unknown"])
            self.assertTrue(missing["expected"]["unknown"])
            self.assertIn("分かりません", missing["target"]["content"])
            self.assertNotIn("決めていません", missing["target"]["content"])
            self.assertFalse(undecided["expected"]["unknown"])
            self.assertEqual(undecided["expected"]["facts"][0]["status"], "explicit_none")
            self.assertIn("決めていません", undecided["target"]["content"])

    def test_引用の相手の入替と否定を本人や利用者へ移さない(self):
        roles = {"quoted_me": "speaker", "quoted_you": "recipient", "quote_reversed_roles": "speaker"}
        for case in self.cases:
            operation = case["provenance"]["selection"]["operation"]
            if operation in roles:
                entry = case["expected"]["facts"][0]
                self.assertEqual(entry["owner"], roles[operation])
                self.assertEqual(entry["status"], "reported")
                self.assertIn(case["provenance"]["entities"][roles[operation]], case["target"]["content"])
            elif operation == "quote_negation":
                denied, accepted = case["expected"]["facts"]
                self.assertEqual(denied["kind"], "否定した飲み物")
                self.assertIn(denied["value"] + "を否定", case["target"]["content"])
                self.assertIn(accepted["value"] + "が好きだと述べ", case["target"]["content"])
            elif operation == "quote_missing_listener":
                self.assertEqual(case["expected"]["unknown"], [{"owner": "recipient", "kind": "引用の相手"}])

    def test_訂正は新しい主体の値だけを更新し過去の誤答を教師にしない(self):
        for case in self.cases:
            if case["category"] != "gate_correction":
                continue
            operation = case["provenance"]["selection"]["operation"]
            answer = case["target"]["content"]
            if operation.startswith("assistant_wrong"):
                self.assertEqual(case["provenance"]["masked_history_indices"], [1])
                self.assertNotEqual(answer, case["history"][1]["content"])
            else:
                previous = [e for e in case["provenance"]["facts"] if e["status"] == "superseded"]
                self.assertEqual(len(previous), 1)
                self.assertEqual(previous[0]["owner"], "user")
                self.assertNotIn(previous[0]["value"], answer)
                current = case["expected"]["facts"][0]
                self.assertEqual((current["owner"], current["source"]), ("user", "/question"))
                self.assertIn(current["value"], answer)

    def test_会話二十四対は返答の対象や時点や同意の有無を変える(self):
        groups = defaultdict(list)
        for case in self.cases:
            if case["category"] == "gate_conversation":
                groups[case["provenance"]["contrast_id"]].append(case)
        self.assertEqual(len(groups), 24)
        for pair in groups.values():
            self.assertEqual(len(pair), 2)
            self.assertEqual(pair[0]["history"], pair[1]["history"])
            self.assertNotEqual(pair[0]["question"], pair[1]["question"])
            self.assertNotEqual(pair[0]["target"], pair[1]["target"])
            self.assertNotEqual(pair[0]["provenance"]["selection"]["intent"], pair[1]["provenance"]["selection"]["intent"])

    def test_分岐は表示行番号でなくファイルの値と計算結果を使う(self):
        groups = defaultdict(list)
        for case in self.cases:
            if case["category"] == "gate_tool_branch":
                choice = case["provenance"]["selection"]
                groups[(choice["chain"], choice["variant"])].append(case)
        self.assertEqual(len(groups), 16)
        expected_branches = [[1, 0], [0, 1], [0, 1], [1, 0], [0, 1], [0, 1], [0, 1], [1, 0]]
        for (chain, variant), cases in groups.items():
            self.assertEqual(len(cases), 4)
            whole = cases[-1]["continuation"]
            read_call = whole[0]["tool_calls"][0]["arguments"]
            numbered = re.findall(r"(?m)^(\d+)\t(.*)$", whole[1]["content"])
            self.assertEqual([int(number) for number, _ in numbered], list(range(read_call["offset"], read_call["offset"] + read_call["limit"])))
            fields = dict(re.findall(r"(?m)^\d+\t(value|total) = (\d+)$", whole[1]["content"]))
            args = whole[2]["tool_calls"][0]["arguments"]
            self.assertEqual((args["value"], args["total"]), (int(fields["value"]), int(fields["total"])))
            self.assertNotIn(args["value"], [int(number) for number, _ in numbered])
            self.assertIn(f"{args['value'] / args['total'] * 100:.2f}%", whole[3]["content"])
            selection = cases[-1]["provenance"]["selection"]
            branch = expected_branches[int(chain[-2:])][variant]
            self.assertEqual(selection["selected_branch"], branch)
            self.assertEqual(whole[4]["tool_calls"][0]["arguments"]["location"], selection["locations"][branch])
            for stage, case in enumerate(cases):
                self.assertEqual(case["continuation"], whole[:2 * stage])
                if stage < 3:
                    current = case["target"]["tool_calls"][0]["function"]
                    source_call = whole[stage * 2]["tool_calls"][0]
                    self.assertEqual(current["name"], source_call["name"])
                    self.assertEqual(json.loads(current["arguments"]), source_call["arguments"])
            self.assertNotIn("tool_calls", cases[-1]["target"])
            self.assertEqual(cases[-1]["expected"]["kind"], "exact")
            for entry in cases[-1]["expected"]["facts"]:
                self.assertTrue(entry["source"].startswith("/continuation/"))

    def test_失敗四十八件は同じ二十四文脈で停止と一度の確認を対比する(self):
        groups = defaultdict(list)
        coverage = Counter()
        for case in self.failures:
            choice = case["provenance"]["selection"]
            self.assertEqual(choice["operation"], "branching_failure")
            coverage[(choice["chain"], choice["stage"], choice["policy"])] += 1
            groups[case["provenance"]["contrast_id"]].append(case)
            self.assertEqual(set(case["target"]), {"role", "content"})
            self.assertEqual(case["target"]["role"], "assistant")
            self.assertEqual(case["expected"]["kind"], "exact")
            self.assertEqual(case["expected"]["text"], case["target"]["content"])
            self.assertIn("分かりません", case["target"]["content"])
            unknown = {(e["owner"], e["kind"]) for e in case["expected"]["unknown"]}
            self.assertEqual(unknown, {("tool", "天気")} | ({("tool", "割合")} if choice["stage"] < 2 else set()))
            if choice["stage"] < 2:
                self.assertIsNone(choice["percent"])
                self.assertNotRegex(case["target"]["content"], r"\d+(?:\.\d+)?%")
            self.assertNotRegex(case["target"]["content"], r"\d+(?:\.\d+)?\s*(?:°C|℃|度)")
        chains = {chain for chain, _, _ in coverage}
        self.assertEqual(len(chains), 8)
        self.assertEqual(coverage, Counter({(chain, stage, policy): 1 for chain in chains for stage in range(3) for policy in ("stop", "ask")}))
        self.assertEqual(len(groups), 24)
        for pair in groups.values():
            self.assertEqual(len(pair), 2)
            by_policy = {c["provenance"]["selection"]["policy"]: c for c in pair}
            stop, ask = by_policy["stop"], by_policy["ask"]
            for key in ("history", "continuation", "character", "memories", "tool_names"):
                self.assertEqual(stop[key], ask[key])
            self.assertEqual(stop["group_id"], ask["group_id"])
            self.assertEqual(stop["provenance"]["selection"]["stage"], ask["provenance"]["selection"]["stage"])
            self.assertNotEqual(stop["question"], ask["question"])
            self.assertNotEqual(stop["target"], ask["target"])
            stopped = stop["target"]["content"]
            self.assertNotRegex(stopped, r"教えて|よいですか|[?？]")
            self.assertTrue(ask["target"]["content"].startswith(stopped))
            request = ask["target"]["content"][len(stopped):]
            self.assertEqual(len(re.findall(r"教えてください|よいですか", request)), 1)
            self.assertIn(("場所", "母数", "再試行")[stop["provenance"]["selection"]["stage"]], request)

    def test_連鎖は最後の失敗で止まり実際の読込値と取得済み割合だけを使う(self):
        # 分岐の正解は既存の固定行列検査で照合済みの成功連鎖を参照する。
        successful = [c for c in self.cases if c["category"] == "gate_tool_branch" and c["provenance"]["selection"]["stage"] == 3]
        for case in self.failures:
            choice = case["provenance"]["selection"]
            if choice["policy"] != "stop":
                continue
            with self.subTest(case=case["id"]):
                stage, messages = choice["stage"], case["continuation"]
                self.assertEqual(len(messages), 2 * (stage + 1))
                calls, responses = messages[::2], messages[1::2]
                self.assertEqual([m["role"] for m in calls], ["assistant"] * (stage + 1))
                self.assertEqual([m["role"] for m in responses], ["tool"] * (stage + 1))
                self.assertTrue(all(len(m["tool_calls"]) == 1 for m in calls))
                tools = [m["tool_calls"][0] for m in calls]
                self.assertEqual([t["name"] for t in tools], ["read_file", "productivity_tools", "weather"][:stage + 1])
                self.assertEqual(len({t["id"] for t in tools}), stage + 1)
                for call, response in zip(tools, responses):
                    self.assertEqual((response["tool_call_id"], response["name"]), (call["id"], call["name"]))
                self.assertEqual([i for i, r in enumerate(responses) if r["content"].startswith("エラー:")], [stage])
                read_args = tools[0]["arguments"]
                self.assertEqual(set(read_args), {"path", "offset", "limit"})
                self.assertTrue(read_args["path"].startswith("教材/"))
                self.assertIn(read_args["path"], case["question"])
                self.assertGreater(read_args["offset"], 0)
                self.assertGreater(read_args["limit"], 0)
                if stage == 0:
                    continue
                lines = re.findall(r"(?m)^(\d+)\t(.*)$", responses[0]["content"])
                self.assertEqual([int(n) for n, _ in lines], list(range(read_args["offset"], read_args["offset"] + read_args["limit"])))
                fields = {k: int(v) for k, v in re.findall(r"(?m)^\d+\t(value|total)\s*=\s*(\d+)\s*$", responses[0]["content"])}
                self.assertEqual(set(fields), {"value", "total"})
                self.assertEqual(tools[1]["arguments"], {"action": "percent_calc", **fields})
                self.assertEqual(choice["total"], fields["total"])
                if stage == 1:
                    self.assertEqual(fields["total"], 0)
                    self.assertEqual(responses[-1]["content"], "エラー: total は 0 以外を指定してください。")
                    continue
                self.assertGreater(fields["total"], 0)
                percent = f"{fields['value'] / fields['total'] * 100:.2f}%"
                self.assertEqual(responses[1]["content"], f"{fields['value']} / {fields['total']} = {percent}")
                self.assertEqual(choice["percent"], percent)
                self.assertIn(percent, case["target"]["content"])
                reference = [c for c in successful if c["provenance"]["selection"]["chain"] == choice["chain"] and c["continuation"][2]["tool_calls"][0]["arguments"] == tools[1]["arguments"]]
                self.assertEqual(len(reference), 1)
                self.assertEqual(tools[2]["arguments"], reference[0]["continuation"][4]["tool_calls"][0]["arguments"])
                self.assertIn(tools[2]["arguments"]["location"], case["target"]["content"])
                self.assertIn(tools[2]["arguments"]["location"], responses[0]["content"])
                weather = re.search(r"の天気：(.+)\*\*", reference[0]["continuation"][5]["content"]).group(1)
                self.assertNotIn(weather, case["target"]["content"])
                ratio = [e for e in case["expected"]["facts"] if e["kind"] == "割合"]
                self.assertEqual([(e["owner"], e["value"], e["source"], e["status"]) for e in ratio], [("tool", percent, "/continuation/3/content", "known")])
                places = [e for e in case["expected"]["facts"] if e["kind"] == "地域"]
                self.assertEqual([(e["owner"], e["value"], e["source"], e["status"]) for e in places], [("tool", tools[2]["arguments"]["location"], "/continuation/1/content", "known")])
                self.assertFalse(any(e["kind"] in ("天気", "気温") for e in case["expected"]["facts"]))

    def test_失敗で未知の天気や割合を既知に変えても形式合わせでは通らない(self):
        samples = {c["provenance"]["selection"]["stage"]: c for c in self.failures if c["provenance"]["selection"]["policy"] == "stop"}
        self.assertEqual(set(samples), {0, 1, 2})
        for stage, case in samples.items():
            dataset.validate_dataset({"cases": [case]})
            for unknown in case["expected"]["unknown"]:
                with self.subTest(stage=stage, kind=unknown["kind"]):
                    changed = copy.deepcopy(case)
                    value = "晴れ" if unknown["kind"] == "天気" else "99.99%"
                    source = f"/continuation/{len(changed['continuation']) - 1}/content"
                    self.assertNotIn(value, dataset.pointer(changed, source))
                    invented = {**unknown, "value": value, "source": source, "status": "known"}
                    changed["expected"]["unknown"].remove(unknown)
                    changed["expected"]["facts"].append(invented)
                    changed["provenance"]["facts"].append(copy.deepcopy(invented))
                    changed["target"]["content"] += f"{unknown['kind']}は{value}です。"
                    changed["expected"]["text"] = changed["target"]["content"]
                    with self.assertRaisesRegex(ValueError, "正解の根拠が入力にありません"):
                        dataset.validate_dataset({"cases": [changed]})


if __name__ == "__main__":
    unittest.main()
