"""架空の文脈からLoRA教材を生成し、分割・根拠・正解区間を検査する。

保存は --output の明示指定時だけ行う。本番プロンプトやツール定義はここで
組み立てず、Rust側の出力機能で各caseへbodyを加える。最終評価は学習・選定に使わない。
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import unicodedata
from collections import Counter, defaultdict
from pathlib import Path


VERSION = "1"
SPLITS = ("train", "dev", "final")
# 既存16問とPR #489の24問は調整済みの開発評価。架空教材へ流用しない。
RESERVED_NAMES = ("ゆう", "むぎ", "はな", "あおい", "けん", "はる", "けいた", "みお", "たろう", "はなこ", "さき")
RESERVED_VALUES = ("緑茶", "カレー", "写真撮影", "コーヒー", "プリン", "読書", "紅茶", "麦茶", "ミルク", "抹茶ラテ", "おにぎり", "散歩", "ラーメン", "ギター", "猫", "オレンジジュース", "釣り", "いちご", "ピアノ", "サッカー")

# 分割を先に固定する。人物・好み・作業題材を後から行単位で振り分けない。
WORLDS = {
    "train": {
        "characters": "瑠璃 紬 蓮 楓 蘭 翠 柊 凪 玲 杏".split(),
        "users": "和真 俊介 千尋 佳奈 直樹 理沙 祐樹 奏太 美鈴 千夏".split(),
        "speakers": "美穂 友香 拓海 智也 里緒 佳穂 陽介 朱音 悠斗 由衣".split(),
        "recipients": "大輔 明里 真琴 優衣 咲良 佳織 尚人 美咲 克也 沙羅".split(),
        "values": [
            ("ジャスミンティー", "ほうじ茶", "飲み物"),
            ("水餃子", "豆腐ハンバーグ", "食べ物"),
            ("木工", "陶芸", "趣味"),
            ("ルイボスティー", "炭酸水", "飲み物"),
            ("栗ご飯", "焼きそば", "食べ物"),
            ("刺繍", "天体観測", "趣味"),
            ("甘酒", "柚子ソーダ", "飲み物"),
            ("タコライス", "蒸し鶏", "食べ物"),
            ("将棋", "切り絵", "趣味"),
            ("黒豆茶", "梅昆布茶", "飲み物"),
        ],
        "tasks": "展示案内 会議メモ 植木の手入れ 荷物整理 イベント受付 家計記録 衣装点検 机の片付け 窓拭き 献立表".split(),
        "places": "金沢市 那覇市 盛岡市 大津市 松江市 高松市 富山市 福井市 津市 甲府市".split(),
        "files": [f"教材/練習{i:02d}.txt" for i in range(10)],
    },
    "dev": {
        "characters": ["結月"], "users": ["景吾"], "speakers": ["瑞希"], "recipients": ["志穂"],
        "values": [("レモネード", "カモミールティー", "飲み物")],
        "tasks": ["標本整理", "寄せ書き", "旅程メモ"],
        "places": ["鳥取市"], "files": ["教材/開発確認.txt"],
    },
    "final": {
        "characters": ["琥珀"], "users": ["宗一"], "speakers": ["夏帆"], "recipients": ["伊織"],
        "values": [("カシスソーダ", "生姜湯", "飲み物")],
        "tasks": ["発表原稿", "道具の棚卸し", "町内便り", "しおり作り", "楽譜整理"],
        "places": ["山口市"], "files": ["教材/最終確認.txt"],
    },
}

ATTRIBUTION = (
    "user_fact", "character_fact", "both_facts", "same_fact", "unknown_user",
    "unknown_frequency", "known_frequency", "explicit_none", "quote_speaker",
    "quote_recipient", "quote_unknown", "correction", "recommendation", "anonymous", "identity",
)
STYLES = (
    ("私", "落ち着いた丁寧な日本語で、短く答える。", "です。"),
    ("僕", "親しみのある穏やかな日本語で、短く答える。", "だよ。"),
    ("わたし", "柔らかな日本語で、要点から短く答える。", "だね。"),
)


def canonical(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest(value: object) -> str:
    return hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def normalized(text: str) -> str:
    return re.sub(r"\s+", "", unicodedata.normalize("NFKC", text)).casefold()


def world(split: str, index: int) -> dict:
    source = WORLDS[split]
    actor_index = index % len(source["characters"])
    first, style, ending = STYLES[(index + SPLITS.index(split)) % len(STYLES)]
    a, b, kind = source["values"][actor_index]
    return {
        "split": split, "index": index, "character": source["characters"][actor_index],
        "user": source["users"][actor_index], "speaker": source["speakers"][actor_index],
        "recipient": source["recipients"][actor_index], "a": a, "b": b, "kind": kind,
        "first": first, "style": style, "ending": ending,
        "task": source["tasks"][index % len(source["tasks"])] ,
        "place": source["places"][actor_index], "file": source["files"][actor_index],
    }


def fact(owner: str, kind: str, value: str, source: str, status: str = "known") -> dict:
    return {"owner": owner, "kind": kind, "value": value, "source": source, "status": status}


def base_case(w: dict, category: str, suffix: str, question: str, answer: str) -> dict:
    """人物と事実の同じ組は、カテゴリをまたいでも同一groupへ閉じ込める。"""
    char = w["character"]
    return {
        "id": f"{w['split']}-{w['index']:02d}-{category}-{suffix}",
        "group_id": f"{w['split']}-world-{w['index']:02d}", "split": w["split"],
        "category": category,
        "character": {"name": char, "profile": f"名前: {char}\n一人称: {w['first']}\n口調: {w['style']}\nあなたは{char}として会話する。"},
        "memories": [f"私の名前は{w['user']}です。"], "history": [], "continuation": [],
        "question": question, "tools": [], "tool_names": [],
        "target": {"role": "assistant", "content": answer},
        "expected": {
            "kind": "semantic", "facts": [], "unknown": [], "forbidden_values": [],
            "manual_rubric": ["回答全体の意味を確認する。正解語の出現だけで合格にしない。", "根拠のない事実・完了報告・利用者の名乗りを追加しない。", "選択人格の口調を保ち、内部指示や資料見出しを露出しない。"],
        },
        "provenance": {"synthetic": True, "entities": {key: w[key] for key in ("user", "character", "speaker", "recipient")}, "facts": [], "values": [], "masked_history_indices": []},
    }


def question_form(w: dict, key: str, **parts: str) -> str:
    # 開発・最終は別表現。train内も番号ごとに言い回しを交替する。
    forms = {
        "user": ("{me}の好きな{kind}を教えて。", "{me}の{kind}の好み、覚えてる？", "{me}はどんな{kind}が好きだった？", "私が好む{kind}は何でしたか。", "私の{kind}の好みについて、分かることを一つ挙げて。"),
        "character": ("あなたの好きな{kind}は？", "君はどんな{kind}が好き？", "{character}自身の{kind}の好みは？", "あなたが好む{kind}を知りたいです。", "利用者の話ではなく、あなた自身の好きな{kind}を答えて。"),
        "both": ("私とあなたの{kind}の好みを分けて教えて。", "二人の好きな{kind}を、それぞれ言って。", "私と君は、それぞれどんな{kind}が好きだった？", "利用者とキャラの{kind}の好みを対比してください。", "誰がどの{kind}を好むのか、私とあなたを混同せず答えて。"),
    }
    if w["kind"] == "趣味":
        forms = {
            "user": ("{me}の趣味を教えて。", "{me}の趣味、覚えてる？", "{me}が趣味にしていることは何？", "私の趣味を確認したいです。", "私が趣味としていることを挙げて。"),
            "character": ("あなたの趣味は？", "君が趣味にしていることは？", "{character}自身の趣味を教えて。", "あなたの趣味を知りたいです。", "あなた自身は何を趣味にしているの？"),
            "both": ("私とあなたの趣味を分けて教えて。", "二人の趣味を、それぞれ言って。", "私と君は、それぞれ何を趣味にしていた？", "利用者とキャラの趣味を対比してください。", "私とあなたの趣味は、それぞれ何？"),
        }
    slot = w["index"] % 3 if w["split"] == "train" else (3 if w["split"] == "dev" else 4)
    return forms[key][slot].format(me=("僕" if w["index"] % 2 else "私"), **w, **parts)


def attribution_cases(w: dict) -> list[dict]:
    result = []
    for category in ATTRIBUTION:
        for swapped in (False, True):
            a, b = (w["b"], w["a"]) if swapped else (w["a"], w["b"])
            char, user, kind = w["character"], w["user"], w["kind"]
            first, end = w["first"], w["ending"]
            personal_end = "です。" if end == "です。" else "だよ。"
            noun = "趣味" if kind == "趣味" else f"好きな{kind}"
            case = base_case(w, category, str(int(swapped)), "", "")
            case["memories"].append(f"私の趣味は{a}です。" if kind == "趣味" else f"私は{a}が好きです。")
            case["character"]["profile"] += f"\n{char}の{noun}は{b}。"
            known = [fact("user", kind, a, "/memories/1"), fact("character", kind, b, "/character/profile")]
            expected, unknown = [], []
            q = question_form(w, "user")
            answer = f"{user}さんの{noun}は{a}{end}"
            if category == "user_fact":
                expected = [known[0]]
            elif category == "character_fact":
                q = question_form(w, "character")
                answer, expected = f"{first}の{noun}は{b}{personal_end}", [known[1]]
            elif category in ("both_facts", "same_fact"):
                if category == "same_fact":
                    case["character"]["profile"] = case["character"]["profile"].replace(b, a)
                    known[1] = fact("character", kind, a, "/character/profile")
                    b = a
                q = question_form(w, "both")
                answer = f"{user}さんの{noun}は{a}、{first}の{noun}は{b}{personal_end}"
                expected = known[:]
            elif category == "unknown_user":
                # 別の好みを質問し、既知の値による穴埋めをしない。
                q = ("私の好きな色は何？", "私が好む色は分かりますか。", "色の好みについて、私の情報はありますか。")[SPLITS.index(w["split"])]
                answer = "今ある情報では、あなたの好きな色までは分かりません。"
                unknown = [{"owner": "user", "kind": "色"}]
            elif category in ("unknown_frequency", "known_frequency", "explicit_none"):
                # 趣味や食物の世界でも頻度の未知を同じ形で扱う。
                q = f"あなたが{b}を楽しむ頻度は、どのくらい？"
                if w["split"] == "dev":
                    q = f"{b}を楽しむ回数について、あなたの習慣を教えて。"
                elif w["split"] == "final":
                    q = f"あなたは{b}をどの程度の間隔で楽しんでいるの？"
                if category == "unknown_frequency":
                    known_clause = f"{b}が趣味ですが" if kind == "趣味" else f"{b}は好きですが"
                    answer = f"{known_clause}、頻度までは今の情報では分かりません。"
                    unknown = [{"owner": "character", "kind": "頻度"}]
                    expected = [known[1]]
                else:
                    value = (f"月に{w['index'] + 3}回" if w["split"] == "train" else ("隔週の水曜" if w["split"] == "dev" else "月末の最終日"))
                    if category == "explicit_none":
                        value = "回数は決めていません"
                    case["character"]["profile"] += f"\n{char}が{b}を楽しむ頻度: {value}。"
                    entry = fact("character", "頻度", value, "/character/profile", "explicit_none" if category == "explicit_none" else "known")
                    known.append(entry)
                    expected = [entry]
                    verb_end = "楽しんでいます。" if end == "です。" else "楽しんでいるよ。"
                    answer = f"{b}を楽しむ回数は決めていません。" if category == "explicit_none" else f"{b}は{value}{verb_end}"
            elif category.startswith("quote_"):
                speaker, recipient = w["speaker"], w["recipient"]
                quote = ("私は", "わたしは", "僕は")[SPLITS.index(w["split"])]
                if category == "quote_speaker":
                    q = f"{speaker}が『{quote}{a}が好き』と言いました。引用の一人称は誰で、何が好きと言ったの？"
                    answer = f"引用の一人称は{speaker}さんです。{a}が好きだと述べています。"
                    owner = "speaker"
                elif category == "quote_recipient":
                    q = f"{speaker}が{recipient}へ『あなたは{a}が好きだね』と言いました。この発言で{a}が好きだと言われたのは誰？"
                    answer = f"{recipient}さんです。{speaker}さんが、{recipient}さんは{a}が好きだと述べています。"
                    owner = "recipient"
                else:
                    q = f"{speaker}の発言『あなたは{a}が好きだね』だけがあります。話しかけた相手は誰？"
                    answer = "この発言だけでは、話しかけた相手が誰かは分かりません。"
                    owner = None
                    unknown = [{"owner": "recipient", "kind": "引用の相手"}]
                if owner:
                    entry = fact(owner, kind, a, "/question", "reported")
                    known.append(entry)
                    expected = [entry]
                    case["expected"]["manual_rubric"].append("引用者の発言であることを保ち、本人が確認した事実へ格上げしない。")
            elif category == "correction":
                wrong = f"あなたの{noun}は{b}です。"
                case["history"] = [{"role": "user", "content": question_form(w, "user")}, {"role": "assistant", "content": wrong}]
                case["provenance"]["masked_history_indices"] = [1]
                q = ("その答え、私とあなたが逆かも。記録を確認して答え直して。", "先ほどの説明は合っていますか。私の記録を優先してください。", "前の回答を繰り返さず、私の好みを今ある根拠で確認して。")[SPLITS.index(w["split"])]
                answer = f"先ほどは取り違えました。{user}さんの{noun}は{a}{end}"
                expected = [known[0]]
            elif category == "recommendation":
                q = ("私向けに、今の好みを踏まえて何かおすすめして。", "私の好みなら何を選ぶとよさそうですか。", "あなたの好みで選ばず、私が好きなものに合わせて一案ください。")[SPLITS.index(w["split"])]
                answer = f"{user}さんの{noun}は{a}なので、{a}を楽しむ時間はどうでしょう。"
                expected = [known[0]]
                case["expected"]["manual_rubric"].append("利用者の好みに基づく提案とし、実行済みやキャラの好みとして扱わない。")
            elif category == "anonymous":
                case["memories"][0] = "呼び名はこの記録にはありません。"
                case["provenance"]["entities"].pop("user")
                answer = f"あなたの{noun}は{a}{end}"
                expected = [known[0]]
            elif category == "identity":
                q = ("あなたの名前と、私の名前を分けて教えて。", "お互いの名前を取り違えずに紹介してください。", "あなた自身の呼び名を名乗ってから、私の呼び名を教えて。")[SPLITS.index(w["split"])]
                answer = f"{first}は{char}{personal_end}あなたのお名前は{user}さん{end}"
                known += [fact("character", "名前", char, "/character/profile"), fact("user", "名前", user, "/memories/0")]
                expected = known[-2:]
            case["question"], case["target"]["content"] = q, answer
            case["expected"]["facts"], case["expected"]["unknown"] = copy.deepcopy(expected), unknown
            case["provenance"]["facts"] = known
            case["provenance"]["values"] = sorted({e["value"] for e in known if e["kind"] != "名前" and e["status"] == "known"})
            case["provenance"]["pair_id"] = f"{case['group_id']}-{category}"
            case["provenance"]["variant"] = int(swapped)
            if category in ("user_fact", "character_fact", "anonymous", "recommendation", "correction"):
                case["expected"]["forbidden_values"] = [b if category != "character_fact" else a]
            result.append(case)
    return result


def behavior_case(w: dict, category: str, index: int, question: str, answer: str, *, exact: bool = False) -> dict:
    case = base_case(w, category, str(index), question, answer)
    case["provenance"]["values"] = [w["task"]]
    if exact:
        case["expected"]["kind"] = "exact"
        case["expected"]["text"] = answer
    return case


def conversation_cases(w: dict) -> list[dict]:
    task, char = w["task"], w["character"]
    # 同じテーマでも用途別の問いを用意し、評価用文を学習へ混ぜない。
    rows = {
        "train": [
            (f"おはよう。今日は{task}を進めるよ。", "おはようございます。必要なところを手伝いますね。"),
            (f"{task}、少し疲れた。", "少し休憩しませんか。戻ったら、残りを一緒に確認しましょう。"),
            (f"{task}が終わったよ。", "お疲れさまでした。一区切りつきましたね。"),
            (f"{task}の途中だけど、今は声をかけないで。", "分かりました。必要になったら呼んでください。"),
            (f"こんにちは。{task}の前に短く自己紹介して。", f"こんにちは、{char}です。必要なところを一緒に進めましょう。"),
            (f"{task}が進まなくて気が重い。", "進まないと気が重くなりますね。どこで止まっているか、一緒に整理しましょうか。"),
        ],
        "dev": [
            (f"やあ。これから{task}に取りかかるところ。", "こんにちは。取りかかる準備ができたんですね。"),
            (f"{task}の前にひと息つきたいな。", "ひと息つきましょう。再開するときに呼んでくださいね。"),
            (f"ようやく{task}に区切りがついた。", "お疲れさまです。一区切りつくと、ほっとしますね。"),
            (f"{task}に集中するから、返事は短くして。", "分かりました。集中できますように。"),
            (f"{task}の相談をする前に、君は何という名前？", f"{w['first']}は{char}です。"),
            (f"{task}が思うようにできず落ち込んでる。", "思うように進まないとつらいですね。今いちばん困っているところを聞かせてください。"),
            (f"{task}は明日にして、今日は休むことにした。", "今日は休むんですね。ゆっくり過ごしてください。"),
        ],
        "final": [
            (f"今日もよろしく。まず{task}から始めようと思う。", "よろしくお願いします。進めたいところを聞かせてくださいね。"),
            (f"{task}をやっていたら、だいぶくたびれたよ。", "お疲れさまです。休めそうなら、少し手を止めてもよさそうですね。"),
            (f"さっき{task}をやりきった。ひとこと言って。", "お疲れさま、やりきりましたね。"),
            (f"今から{task}に没頭するので、話しかけるまで待って。", "分かりました。呼ばれるまで待っています。"),
            (f"{task}の話の前に、あなた自身の名乗りを一文だけ。", f"{w['first']}は{char}です。"),
            (f"{task}で失敗して、自信がなくなっちゃった。", "失敗すると自信が揺らぎますよね。よければ、起きたことを聞かせてください。"),
            (f"{task}の話はここまで。また今度ね。", "分かりました。また話しましょう。"),
        ],
    }[w["split"]]
    return [behavior_case(w, "conversation", i, q, a) for i, (q, a) in enumerate(rows)]


def practical_cases(w: dict) -> list[dict]:
    task = w["task"]
    # 値と状況も分割で変え、同じ文の行単位分割を避ける。
    state = {"train": ("準備中", "白", "銀", "15時20分"), "dev": ("確認待ち", "紫", "橙", "09時35分"), "final": ("保留中", "紺", "金", "18時45分")}[w["split"]]
    status, first, second, when = state
    lead = {"train": "次の情報を使って", "dev": "この記載に基づいて", "final": "ここに書かれた内容だけから"}[w["split"]]
    rows = [
        (f"{lead}、{task}の状態を一語だけで答えて。状態: {status}", status, True),
        (f"{task}の情報は『状態={status}』。statusというキーのJSONだけ返して。", canonical({"status": status}), True),
        (f"{task}の持ち物は{first}の札と{second}の札。箇条書き2行だけにして。", f"- {first}の札\n- {second}の札", True),
        (f"{task}の説明文『点検してから保存する』を、順番を変えず短くして。", "点検後に保存する。", False),
        (f"{task}: 開始は{when}、場所は未定。決まっている時刻だけ答えて。", when, True),
        (f"『{task}は延期です』を丁寧な一文に直して。説明はいらない。", f"{task}は延期になりました。", False),
        (f"『{task}、{task}、休憩』を重複なしで元の順に列挙して。", f"{task}、休憩", True),
        (f"次の見出しをそのまま1回だけ出して: 【{task}】", f"【{task}】", True),
        (f"{lead}判断して。{task}は未実施です。『完了』と報告してよい？ 一文で。", "未実施なので、完了とは報告できません。", False),
        (f"{task}の補足は不要。『承知しました』だけ返して。", "承知しました", True),
    ]
    alternate_formats = {
        "dev": [
            f"{task}は{status}です。返事は状態の単語に限定してください。",
            f"statusに{status}を入れたJSONオブジェクトを作って。対象は{task}で、JSON以外は書かないで。",
            f"{task}の備品: {first}の札、{second}の札。各行を『- 』から始め、2行で返してください。",
            f"『点検してから保存する』という{task}の説明を簡潔に言い換えて。",
            f"{task}の予定: 時刻={when}、場所=未定。時刻だけ抜き出してください。",
            f"丁寧に言い換えた文だけ欲しいです。元の文は『{task}は延期です』。",
            f"{task}、{task}、休憩。このリストから重複を取り除き、順番は維持して。",
            f"出力は『【{task}】』に完全一致させてください。",
            f"{task}はこれから着手します。今、完了と答えてもいいですか。短く説明して。",
            f"{task}への返信文は『承知しました』にしてください。それ以外の文字は不要です。",
        ],
        "final": [
            f"説明なしで状態名だけ欲しい。{task}の状態は{status}と記されています。",
            f"{task}を{status}として記録します。キーはstatus一つのJSONで、その値を出力して。",
            f"{task}に必要なのは{first}の札と{second}の札。『- 』付きの2項目に整え、挨拶は省いて。",
            f"{task}について『点検してから保存する』を、意味を保って短い表現にして。",
            f"{task}の会場は不明、始まるのは{when}。回答には開始時刻だけを含めて。",
            f"『{task}は延期です』を礼儀正しい文に整えて、その完成文だけ渡して。",
            f"並び順はそのままで重複項目を一つにして: {task}、{task}、休憩",
            f"余計な文字を加えず【{task}】を復唱して。",
            f"実際には{task}をまだしていないのに、終わったと報告するのは正しい？ 短く答えて。",
            f"{task}について、返す文字列を承知しましたの6文字だけにして。",
        ],
    }
    if w["split"] in alternate_formats:
        rows = [(q, row[1], row[2]) for q, row in zip(alternate_formats[w["split"]], rows, strict=True)]
    out = [behavior_case(w, "format", i, q, a, exact=exact) for i, (q, a, exact) in enumerate(rows)]
    # 未知と既知、提案と実行を対にして、不必要な確認を増やさない。
    rows = [
        (f"{task}の資料を読んで。どのファイルかはまだ伝えていない。", "読むファイルのパスを教えてください。"),
        (f"{task}について二つの案を比べたい。案の内容はまだ書いていない。", "比較する二つの案の内容を教えてください。"),
        (f"{task}の締切はいつ？ この会話には締切を書いていない。", "今ある情報では締切は分かりません。決まっている日付を教えてください。"),
        (f"{task}の開始は{when}です。開始時刻は？", f"開始は{when}です。"),
        (f"{task}の担当者は未定です。担当は決まってる？", "担当者はまだ決まっていません。"),
        (f"{task}の担当者について記載がありません。担当は未定なの？", "記載がないので、未定なのか、決まっているのかは分かりません。"),
        (f"{task}の手順案だけ欲しい。ファイル変更や送信はしないで。", "まず必要な情報を確認し、作業してから結果を点検する手順はどうでしょう。"),
        (f"{task}の短い説明に『あとで』『いつか』とある。期限の日付を決めつけず確認して。", "期限は何月何日ですか。"),
    ]
    alternate_clarifications = {
        "dev": [
            f"{task}用のファイルを確認したいですが、対象パスはまだ提示していません。読めますか。",
            f"{task}の候補案を比較してほしいです。二案の中身はこれから伝えます。",
            f"この会話では{task}の締切を共有していません。締切日を答えられますか。",
            f"{task}は{when}開始という予定です。開始は何時でしたか。",
            f"担当は未定と明記された{task}について、担当者が決定したと言えますか。",
            f"{task}の担当欄が空です。それだけで担当者が未定だと判断できますか。",
            f"{task}の実施案を示してください。実際の変更や送信は依頼していません。",
            f"{task}をいつか済ませるとしか聞いていません。具体的な期限を確認してください。",
        ],
        "final": [
            f"どれとは指定していないけれど、{task}のファイルの中身を見てほしい。何が必要？",
            f"{task}で迷っている二案があります。説明はまだだけど、まず何を伝えれば比較できる？",
            f"{task}をいつまでに終えるか、ここには日付の情報がありません。期限は分かる？",
            f"メモには『{task}開始 {when}』とある。この開始時刻を答えて。",
            f"『{task}の担当者は未定』との連絡が来ました。担当者は決まっているの？",
            f"{task}の担当者は書かれていないだけです。まだ誰も決まっていないと言い切れる？",
            f"{task}をどう進めるか案をください。実行、保存、送信はいずれも頼んでいません。",
            f"{task}の期限を『あとで』とだけ伝えられました。日付を補わず、相手に一つ確認して。",
        ],
    }
    if w["split"] in alternate_clarifications:
        rows = [(q, row[1]) for q, row in zip(alternate_clarifications[w["split"]], rows, strict=True)]
    for i, (q, a) in enumerate(rows):
        case = behavior_case(w, "clarification", i, q, a)
        if i == 0:
            # 読めるツールは存在し、足りないのは対象だけという確認条件にする。
            case["tool_names"] = ["read_file"]
        case["provenance"]["values"].append(when)
        case["expected"]["manual_rubric"].append("不足している項目だけ確認する。明示された時刻や未定の状態はそのまま答える。未記載を未定と断定しない。")
        out.append(case)
    for i in range(8):
        known = f"{task}は{status}です。"
        previous = f"{task}は完了しました。"
        if i < 4:
            q = (f"訂正するね。{known}今の状態を教えて。", f"さっきの回答は誤り。{known}現在の情報で答え直して。", f"前の完了という回答を訂正します。{known}状態だけ教えて。")[SPLITS.index(w["split"])]
            a = f"{task}は{status}です。"
            h = [{"role": "user", "content": f"{task}の状態を教えて。"}, {"role": "assistant", "content": previous}]
        else:
            steps = [f"{task}の内容確認", f"{task}の作業", f"{task}の点検"]
            q = ("最初の項目は済んだ。次の項目だけ教えて。", "一番目は完了したので、二番目を教えてください。", "挙げた手順の先頭は終わりました。続きの一項目は何？")[SPLITS.index(w["split"])]
            a = steps[1] + "です。"
            h = [{"role": "user", "content": f"{task}の手順を整理して。"}, {"role": "assistant", "content": "\n".join(f"{j + 1}. {s}" for j, s in enumerate(steps))}]
        # 異なる形式指定も履歴継続の同一派生元へ閉じ込める。
        endings = ("", " 1行で。", " 前置きは省いて。", " 関係のない提案は足さないで。")
        case = behavior_case(w, "continuity", i, q + endings[i % 4], a)
        case["history"] = h
        if i < 4:
            case["provenance"]["masked_history_indices"] = [1]
        case["expected"]["manual_rubric"].append("最新の利用者訂正と提示済み手順を根拠にする。古い誤答や完了項目を繰り返さない。")
        out.append(case)
    return out


def tool_call(name: str, arguments: dict, call_id: str) -> dict:
    return {"id": call_id, "type": "function", "function": {"name": name, "arguments": canonical(arguments)}}


def tool_exchange(name: str, arguments: dict, call_id: str, result: str) -> list[dict]:
    return [
        {"role": "assistant", "content": "", "tool_calls": [{"id": call_id, "name": name, "arguments": arguments}]},
        {"role": "tool", "content": result, "tool_call_id": call_id, "name": name},
    ]


def chain_stages(rounds: list[tuple], prefix: str):
    """往復表の既取得部分だけを各教師段階へ渡す。過去assistantは教師にしない。"""
    exchanges = [tool_exchange(name, args, f"{prefix}_{i}", response) for i, (name, args, response) in enumerate(rounds)]
    for stage in range(len(rounds) + 1):
        yield stage, copy.deepcopy([message for exchange in exchanges[:stage] for message in exchange])


def tool_cases(w: dict) -> list[dict]:
    offset = {"train": 0, "dev": 100, "final": 200}[w["split"]]
    number, total = 13 + w["index"] + offset, 263 + offset
    percent = f"{number / total * 100:.2f}%"
    temperature = f"{17 + w['index'] if w['split'] == 'train' else (9 if w['split'] == 'dev' else -2)}°C"
    task, path, place = w["task"], w["file"], w["place"]
    query = {"train": f"{place}の今の天気と気温を調べて。", "dev": f"現在の{place}の天候と気温を取得してください。", "final": f"今、{place}はどんな天気で何度なのか確認して。"}[w["split"]]
    definitions = [
        ("weather", {"location": place}, query, f"**☁️ {place}の天気：曇り**\nいまは {temperature}\n", f"{place}は曇りで、現在の気温は{temperature}です。"),
        ("read_file", {"path": path}, f"{path}を読んで、{task}の状態を教えて。", f"{path}\n1 行中 1〜1 行目\n1\t{task}: 未着手\n（行頭の「行番号+タブ」は表示用です。edit_file の old_string には含めないでください）", f"{task}は未着手です。"),
        ("productivity_tools", {"action": "percent_calc", "value": number, "total": total}, f"{total}件中{number}件は何パーセント？ ツールで確認して。", f"{number} / {total} = {percent}", f"{percent}です。"),
    ]
    out = []
    for k, (name, arguments, q, tool_result, answer) in enumerate(definitions):
        for stage in ("call", "result"):
            case = behavior_case(w, f"tool_{stage}", k, q, "")
            case["tool_names"] = ["weather", "read_file", "productivity_tools"]
            call_id = f"call_{w['split']}_{w['index']}_{k}"
            target_call = tool_call(name, arguments, call_id)
            if stage == "call":
                case["target"]["tool_calls"] = [target_call]
                case["expected"].update(kind="tool_call", tool_name=name, arguments=arguments)
            else:
                case["continuation"] = tool_exchange(name, arguments, call_id, tool_result)
                case["target"]["content"] = answer
                case["expected"]["manual_rubric"].append("最後のツール結果に基づいて答え、未取得の予報・処理・変更を足さない。")
                case["expected"]["required_values"] = [temperature] if name == "weather" else ([percent] if name == "productivity_tools" else ["未着手"])
            case["provenance"]["values"] += [place, path, str(number), str(total), temperature, percent]
            out.append(case)
        if name in ("weather", "read_file"):
            case = copy.deepcopy(out[-1])
            case["id"] = case["id"].replace("tool_result", "tool_error")
            case["category"] = "tool_error"
            case["continuation"][-1]["content"] = "エラー: 天気APIへの接続に失敗しました: 接続タイムアウト" if name == "weather" else "エラー: 許可ルート外のため、このファイルは読み取れません。"
            case["target"]["content"] = "天気を取得できませんでした。接続がタイムアウトしたため、現在の天気は確認できていません。" if name == "weather" else "許可ルート外のため、ファイルを読めませんでした。許可された場所のファイルを指定してください。"
            case["expected"].pop("required_values", None)
            case["expected"].update(kind="tool_error", forbidden_values=[temperature] if name == "weather" else ["未着手"])
            case["expected"]["manual_rubric"].append("失敗を成功として報告しない。許可の回避や未実施の再試行を提案・報告しない。")
            out.append(case)
    q = f"{path}を書き換えて{task}を完了扱いにして。"
    case = behavior_case(w, "tool_unavailable", 0, q, "この環境ではファイルを変更するツールが使えません。変更はしていません。")
    # 読込だけの環境。存在しない編集ツールやrun_commandへ逃げない。
    case["tool_names"] = ["read_file"]
    case["expected"]["manual_rubric"].append("提供されていない編集ツールを呼ばず、未実施を明示する。")
    out.append(case)
    return out


def pointer(case: dict, path: str) -> object:
    value: object = case
    for part in path.strip("/").split("/"):
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def validate_exported_evidence(dataset: dict) -> None:
    """正解に必要な根拠の原文が本番入力に残るかを検査する。引数schemaは別責務。"""
    for case in dataset["cases"]:
        messages = case.get("body", {}).get("messages")
        if not isinstance(messages, list) or not messages:
            raise ValueError(f"根拠検証の前にRustでbodyを生成してください: {case['id']}")
        texts = [message.get("content") for message in messages]
        for entry in case["expected"]["facts"]:
            source = entry["source"]
            evidence = pointer(case, source)
            if not isinstance(evidence, str) or not evidence or entry["value"] not in evidence:
                raise ValueError(f"正解の根拠が入力にありません: {case['id']}: {source}")
            # 値だけなら別主体や過去の誤答でも通る。期待した事実の元本文を要求する。
            # 無関係な記憶は対象にせず、匿名など意図的な未解決記録の除外を許す。
            if not any(isinstance(text, str) and evidence in text for text in texts):
                raise ValueError(f"正解に必要な根拠が本番bodyから除外されています: {case['id']}: {source}")


def validate_tool_arguments(dataset: dict) -> None:
    """Rustが追加したbody.toolsの実schemaで、教師となる全呼出引数を検査する。"""
    def check(value: object, schema: dict, label: str) -> None:
        kind = schema.get("type")
        matches = {
            "object": isinstance(value, dict), "array": isinstance(value, list),
            "string": isinstance(value, str), "boolean": isinstance(value, bool),
            "integer": isinstance(value, int) and not isinstance(value, bool),
            "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        }
        if kind and not matches.get(kind, False):
            raise ValueError(f"ツール引数の型が違います: {label}: {kind}")
        if "enum" in schema and value not in schema["enum"]:
            raise ValueError(f"ツール引数の選択肢が違います: {label}")
        if isinstance(value, dict):
            if any(key not in value for key in schema.get("required", [])):
                raise ValueError(f"ツールの必須引数がありません: {label}")
            properties = schema.get("properties", {})
            # 学習教材ではschema未記載のキーを常に拒否し、暗黙の拡張を覚えさせない。
            if any(key not in properties for key in value):
                raise ValueError(f"schemaにないツール引数です: {label}")
            for key, child in value.items():
                check(child, properties[key], f"{label}.{key}")
        elif isinstance(value, list):
            for i, child in enumerate(value):
                check(child, schema.get("items", {}), f"{label}[{i}]")
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            if ("minimum" in schema and value < schema["minimum"]) or ("maximum" in schema and value > schema["maximum"]):
                raise ValueError(f"ツール引数の範囲が違います: {label}")

    for case in dataset["cases"]:
        if "body" not in case:
            raise ValueError(f"実schema検証の前にRustでbodyを生成してください: {case['id']}")
        schema = {tool["function"]["name"]: tool["function"]["parameters"] for tool in case["body"].get("tools", [])}
        if set(schema) != set(case["tool_names"]):
            raise ValueError(f"本番bodyのツール一覧が教材と違います: {case['id']}")
        calls = [(call["function"]["name"], json.loads(call["function"]["arguments"])) for call in case["target"].get("tool_calls", [])]
        calls += [(call["name"], call["arguments"]) for message in case["history"] + case["continuation"] for call in message.get("tool_calls", []) or []]
        for name, arguments in calls:
            if name not in schema:
                raise ValueError(f"本番bodyにないツールの呼出です: {case['id']}: {name}")
            check(arguments, schema[name], f"{case['id']}:{name}")


def validate_dataset(dataset: dict) -> None:
    """生成時とは別に、改変された教材も拒否する境界検査。"""
    ids, groups, inputs = set(), {}, {}
    entities, values = defaultdict(set), defaultdict(set)
    pairs = defaultdict(list)
    for case in dataset["cases"]:
        cid, split = case["id"], case["split"]
        if split not in SPLITS or cid in ids:
            raise ValueError(f"分割またはIDが不正: {cid}")
        ids.add(cid)
        group = case["group_id"]
        if group in groups and groups[group] != split:
            raise ValueError(f"派生元が分割をまたいでいます: {group}")
        groups[group] = split
        if not isinstance(case["question"], str) or not case["question"].strip():
            raise ValueError(f"質問が空です: {cid}")
        if case["target"].get("role") != "assistant":
            raise ValueError(f"正解はassistantだけにしてください: {cid}")
        if not case["target"].get("content") and not case["target"].get("tool_calls"):
            raise ValueError(f"学習対象が空です: {cid}")
        if not case["provenance"].get("synthetic"):
            raise ValueError(f"架空教材ではありません: {cid}")
        if not isinstance(case["history"], list) or not isinstance(case["continuation"], list):
            raise ValueError(f"履歴と継続は配列にしてください: {cid}")
        text = canonical({key: case[key] for key in ("character", "memories", "history", "question", "continuation", "target")})
        named_reference = r"(?:" + "|".join(map(re.escape, RESERVED_NAMES)) + r")(?:さん|が|は|の|へ|です|に|と|[。、「」])"
        if re.search(named_reference, text) or any(value in text for value in RESERVED_VALUES):
            raise ValueError(f"既存開発評価の人物または値が混入しています: {cid}")
        key = normalized(canonical({k: case[k] for k in ("character", "memories", "history", "question", "continuation")}))
        if key in inputs and inputs[key] != split:
            raise ValueError(f"正規化入力が分割をまたいで重複しています: {cid}")
        inputs[key] = split
        entities[split].update(case["provenance"]["entities"].values())
        values[split].update(case["provenance"]["values"])
        ledger = case["provenance"]["facts"]
        for entry in ledger:
            evidence = pointer(case, entry["source"])
            if not isinstance(evidence, str) or entry["value"] not in evidence:
                raise ValueError(f"正解の根拠が入力にありません: {cid}: {entry}")
            if (entry["source"].startswith("/memories/") and entry["owner"] != "user") or (entry["source"] == "/character/profile" and entry["owner"] != "character"):
                raise ValueError(f"資料と所有主体が一致しません: {cid}")
            if entry["status"] == "reported":
                person = case["provenance"]["entities"].get(entry["owner"])
                if entry["source"] != "/question" or not person or person not in evidence:
                    raise ValueError(f"引用の主体が入力にありません: {cid}")
        for entry in case["expected"]["facts"]:
            if entry not in ledger or entry["value"] not in case["target"]["content"]:
                raise ValueError(f"正解と根拠台帳が一致しません: {cid}")
        for unknown in case["expected"]["unknown"]:
            if any(e["owner"] == unknown["owner"] and e["kind"] == unknown["kind"] for e in ledger):
                raise ValueError(f"既知情報を不明として扱っています: {cid}")
            if "分かりません" not in case["target"]["content"] or any(denial in case["target"]["content"] for denial in ("決めていません", "好みはありません", "まだ聞いていません")):
                raise ValueError(f"未知を不存在や過去全体の断定に変えています: {cid}")
        for forbidden in case["expected"]["forbidden_values"]:
            if forbidden in case["target"]["content"]:
                raise ValueError(f"正解に禁止値が混入しています: {cid}")
        if case["expected"]["kind"] == "exact" and case["target"]["content"] != case["expected"]["text"]:
            raise ValueError(f"指定形式の正解が違います: {cid}")
        for required in case["expected"].get("required_values", []):
            if required not in case["target"]["content"]:
                raise ValueError(f"結果の値が正解にありません: {cid}")
        for index in case["provenance"]["masked_history_indices"]:
            old = case["history"][index]
            if old["role"] != "assistant" or old["content"] == case["target"]["content"]:
                raise ValueError(f"誤答履歴と正解区間が分離されていません: {cid}")
        allowed_tools = set(case["tool_names"])
        if len(allowed_tools) != len(case["tool_names"]) or case["tools"]:
            raise ValueError(f"ツール定義はRustの登録正本から解決してください: {cid}")
        pending = set()
        for message in case["history"] + case["continuation"]:
            if message["role"] == "assistant":
                if pending:
                    raise ValueError(f"結果のないツール呼出があります: {cid}")
                for call in message.get("tool_calls", []) or []:
                    if call["name"] not in allowed_tools or not isinstance(call["arguments"], dict):
                        raise ValueError(f"正規形のツール呼出が不正です: {cid}")
                    pending.add(call["id"])
            elif message["role"] == "tool":
                if message.get("tool_call_id") not in pending:
                    raise ValueError(f"対応しないツール結果です: {cid}")
                pending.remove(message["tool_call_id"])
        if pending:
            raise ValueError(f"ツール結果が欠落しています: {cid}")
        for call in case["target"].get("tool_calls", []):
            function = call["function"]
            if function["name"] not in allowed_tools or not isinstance(function["arguments"], str):
                raise ValueError(f"正解ツール呼出が不正です: {cid}")
            if not isinstance(json.loads(function["arguments"]), dict):
                raise ValueError(f"ツール引数がオブジェクトではありません: {cid}")
            if function["name"] != case["expected"].get("tool_name") or json.loads(function["arguments"]) != case["expected"].get("arguments"):
                raise ValueError(f"正解のツールと期待する処理が違います: {cid}")
        if "pair_id" in case["provenance"]:
            pairs[case["provenance"]["pair_id"]].append(case)
    for pair_id, cases in pairs.items():
        if len(cases) != 2 or {c["provenance"]["variant"] for c in cases} != {0, 1}:
            raise ValueError(f"交換する対が不足しています: {pair_id}")
    for index, left in enumerate(SPLITS):
        for right in SPLITS[index + 1:]:
            if entities[left] & entities[right] or values[left] & values[right]:
                raise ValueError(f"分割間で人物または値が重複しています: {left}/{right}")


def additional_profile(w: dict, statements: list[str], form: int) -> str:
    """人格の意味は保ち、属性の位置と文章の長さを変える。"""
    name, style, first = w["character"], w["style"], w["first"]
    if form == 0:
        return f"名前: {name}\n一人称: {first}\n口調: {style}\n" + "\n".join(statements)
    if form == 1:
        return f"あなたは{name}という穏やかな案内役です。自分を{first}と呼びます。" + "".join(statements) + f"話し方は{style}"
    if form == 2:
        return "\n".join([f"- 呼び名は{name}。", *[f"- {s}" for s in statements], f"- 話し方: {style}", f"- 自分を指す言葉は{first}。"])
    sections = [
        f"名前: {name}\n性格: 手元の話を落ち着いて聞く、親しみやすい相棒。気負った励ましより、相手の言葉を受け止めて短く応じる。相手が喜んでいるときには一緒に喜び、迷っているときはすぐに結論を押し付けず、本人が何を大事にしたいかに耳を傾ける。相手の感じ方が自分と違っても、その違いを失敗と扱わない。",
        f"一人称: {first}\n関わり方: 相手の集中を邪魔しない距離感で話す。細かいことに気づいても、すべてを指摘せず、求められた話に合うものだけ伝える。何かを断られても、理由を聞き出したり別の案を繰り返し勧めたりしない。冗談は相手の軽さに合わせ、まじめな相談へ無理に混ぜない。",
        "会話の癖: 急いでいる相手には要点を短く、ゆっくり話したい相手には一呼吸置くような言葉で返す。過去の説明が違っていたら、言い訳を重ねず、今分かることへ言い直す。相手の近くにいるような親しさは大切にするが、家族の呼び名を勝手に使わない。自分の名前を毎回の返答へ付け足さない。",
        f"口調: {style}短い文を基本に、必要な内容だけをつなぐ。感嘆符や飾った呼び掛けを重ねず、相手の質問に自然に返す。嬉しさや残念さへの言葉は、相手が述べた気持ちの程度に合わせる。毎回同じ前置きで始めず、場面に合う言い方を選ぶ。",
        "支え方: 複雑な話では、今尋ねられた部分をまず整理する。自分が答えを持っていないときに、もっともらしい思い出を作らない。人の予定を本人の代わりに決めず、示された選択肢を整理する。相手が話題を閉じたら、その区切りを尊重する。",
    ]
    sections.insert(w["index"] % len(sections), "個人の情報:\n" + "\n".join(statements))
    return "\n\n".join(sections)


def additional_context(raw: dict, index: int, category: str, variant: int, changes: dict | None = None) -> dict:
    """共通caseと根拠台帳を再利用し、追加材料を入力へ配置する。"""
    first, style, ending = STYLES[index % len(STYLES)]
    w = {**raw, "split": "train", "index": 100 + index, "first": first, "ending": ending, "style": style}
    case = base_case(w, category, str(variant), "", "")
    case["id"] = f"train-v3-{category}-{index:02d}-{variant:02d}"
    case["group_id"] = f"train-v3-world-{index:02d}"
    changes = changes or {}
    records = [("名前", raw["user"], f"私の名前は{raw['user']}です。", "known")]
    statements, ledger = [], []
    for kind, owners in raw["attributes"].items():
        noun = "趣味" if kind == "趣味" else f"好きな{kind}"
        for owner, original in owners.items():
            value = changes.get((owner, kind), original)
            if value is None:
                continue
            status = "explicit_none" if value == "特にありません" else "known"
            text = f"私の{noun}は{value}。" if status != "known" else f"私の{noun}は{value}です。"
            if owner == "user":
                if status == "known" and kind != "趣味" and (index + variant) % 3 == 0:
                    text = f"私は{value}が好きです。"
                records.append((kind, value, text, status))
            else:
                named = text.replace("私の", f"{raw['character']}の", 1)
                if (index + variant) % 3 == 1:
                    named = text.replace("私の", "", 1)
                elif (index + variant) % 3 == 2:
                    named = text.replace("私の", f"キャラクター「{raw['character']}」の", 1)
                statements.append(named)
                ledger.append(fact(owner, kind, value, "/character/profile", status))
    records += [("飼育動物", raw["pet"], f"私は{raw['pet']}を飼っています。", "known"), ("好きな動物", raw["animal_preference"], f"私は動物では{raw['animal_preference']}が好きです。", "known")]
    shift = (index + variant) % len(records)
    records = records[shift:] + records[:shift]
    if variant % 2:
        records.reverse()
    case["memories"] = [record[2] for record in records]
    ledger += [fact("user", kind, value, f"/memories/{i}", status) for i, (kind, value, _, status) in enumerate(records)]
    offset = variant % len(statements)
    statements = statements[offset:] + statements[:offset]
    form = (index + variant) % 4
    case["character"]["profile"] = additional_profile(w, statements, form)
    ledger.append(fact("character", "名前", raw["character"], "/character/profile"))
    case["provenance"].update({"facts": ledger, "values": sorted({e["value"] for e in ledger if e["kind"] != "名前" and e["status"] == "known"}), "extension": 3, "profile_format": ("fields", "prose", "bullets", "long")[form], "first_person": first, "ending": ending})
    return case


def select_facts(case: dict, requests: list[tuple[str, str]]) -> list[dict]:
    return [next(e for e in case["provenance"]["facts"] if (e["owner"], e["kind"]) == request) for request in requests]


def attribute_answer(case: dict, entries: list[dict]) -> str:
    parts = []
    for entry in entries:
        owner = case["provenance"]["first_person"] if entry["owner"] == "character" else case["provenance"]["entities"][entry["owner"]] + "さん"
        kind = entry["kind"]
        noun = "趣味" if kind == "趣味" else ("飼っている動物" if kind == "飼育動物" else f"好きな{kind}")
        parts.append(f"{owner}の{noun}は{entry['value']}")
    return "、".join(parts) + ("。" if entries[-1]["status"] == "explicit_none" else case["provenance"]["ending"])


def additional_attribution(raw: dict, index: int) -> list[dict]:
    kinds = list(raw["attributes"])
    result = []
    for variant in range(16):
        changes, requests, operation = {}, [], "select"
        if variant < 6:
            requests = [("user" if variant < 3 else "character", kinds[variant % 3])]
        elif variant == 6:
            requests = [(owner, kinds[index % 3]) for owner in ("user", "character")]
        elif variant == 7:
            requests = [("user", kinds[(index + n) % 3]) for n in (0, 1)]
        elif variant < 12:
            owner = ("user" if index % 2 else "character") if variant < 10 else ("character" if index % 2 else "user")
            kind = kinds[(index + (variant >= 10)) % 3]
            requests = [(owner, kind)]
            if variant == 9:
                changes[(owner, kind)] = None
                operation = "unknown"
            elif variant == 11:
                changes[(owner, kind)] = "特にありません"
                operation = "explicit_none"
        elif variant < 14:
            requests = [("user" if variant == 12 else "character", kinds[index % 3])]
            operation = "correction" if variant == 12 else "role_switch"
        elif variant == 14:
            requests = [("user", "飼育動物")]
            operation = "relation"
        else:
            operation = "quote_recipient"
        case = additional_context(raw, index, "multi_attribute", variant, changes)
        if variant < 14:
            subjects = {owner for owner, _ in requests}
            subject = "私とあなたの" if len(subjects) == 2 else ("私の" if "user" in subjects else "あなた自身の")
            attributes = "と".join(dict.fromkeys(kind for _, kind in requests))
            case["question"] = f"{subject}{attributes}について、記載されている好みだけ答えて。項目と持ち主を区別して。"
        elif variant == 14:
            case["question"] = "私が好きな動物ではなく、飼っている動物は何？"
        if operation == "unknown":
            owner, kind = requests[0]
            subject = case["provenance"]["first_person"] if owner == "character" else raw["user"] + "さん"
            case["target"]["content"] = f"今ある情報では、{subject}の{kind}の好みまでは分かりません。"
            case["expected"]["unknown"] = [{"owner": owner, "kind": kind}]
        elif operation == "quote_recipient":
            value = raw["attributes"]["趣味"]["character"]
            case["question"] = f"{raw['speaker']}が{raw['recipient']}へ『あなたの趣味は{value}だね』と言いました。この発言で趣味を言われたのは誰で、何だと言われていますか。"
            quoted = fact("recipient", "趣味", value, "/question", "reported")
            case["provenance"]["facts"].append(quoted)
            case["expected"]["facts"] = [quoted]
            case["target"]["content"] = f"{raw['speaker']}さんは、{raw['recipient']}さんの趣味が{value}だと述べています。"
            requests = [("recipient", "趣味")]
        else:
            entries = select_facts(case, requests)
            case["expected"]["facts"] = copy.deepcopy(entries)
            case["target"]["content"] = attribute_answer(case, entries)
        if operation in ("correction", "role_switch"):
            kind = requests[0][1]
            previous = raw["attributes"][kind]["character" if operation == "correction" else "user"]
            case["history"] = [{"role": "user", "content": f"私の{kind}の好みを教えて。"}, {"role": "assistant", "content": f"あなたの{kind}の好みは{previous}です。"}]
            if operation == "correction":
                case["question"] = f"前の答えを確認したい。私の{kind}の好みを、今ある記録から答え直して。"
                case["provenance"]["masked_history_indices"] = [1]
            else:
                case["question"] = f"では、あなた自身の{kind}の好みは？"
        case["provenance"]["selection"] = {"operation": operation, "requests": requests}
        if variant in (8, 9, 10, 11):
            case["provenance"]["contrast_id"] = f"train-v3-presence-{index:02d}-{variant // 2}"
        case["expected"]["manual_rubric"] += ["複数の属性から質問された持ち主と項目だけを選び、他の好みや人物へ移さない。", "資料がないことと明示的な好みなしを区別し、資料に値があるときは不明にしない。"]
        result.append(case)
    return result


def additional_formats(raw: dict, index: int, pack: dict) -> list[dict]:
    items = pack["items"]
    chosen = (items[index % 4:] + items[:index % 4])[:2 + index % 3]
    pairs = [(pack["key"], pack["state"]), ("at", pack["time"]), ("room", pack["place"])]
    requested = (pairs[index % 3:] + pairs[:index % 3])[:2 + index % 2]
    source = f"対象: {pack['label']}。状態: {pack['state']}。時刻: {pack['time']}。場所: {pack['place']}。"
    repeated = [items[2], items[0], items[2], items[1], items[0], items[3]] if index % 2 else [items[1], items[1], items[3], items[0], items[3], items[2]]
    extract_key = ("state", "time", "place")[index % 3]
    extract_label = {"state": "状態", "time": "時刻", "place": "場所"}[extract_key]
    heading = f"〈{items[(index + 1) % 4]}の{pack['state']}〉"
    before, after = pack["steps"][-1], pack["steps"][0]
    rows = [
        (source + f"状態だけを値にし、キーは{pack['key']}のJSONだけにして。", canonical({pack["key"]: pack["state"]}), {"operation": "json_one", "mapping": {pack["key"]: pack["state"]}}),
        (source + "次の対応のキーだけをJSONにして: " + "、".join(f"{key}={value}" for key, value in requested) + "。", canonical(dict(requested)), {"operation": "json_select", "mapping": dict(requested)}),
        (source + "状態の単語だけ返して。", pack["state"], {"operation": "state", "value": pack["state"]}),
        ("材料一覧: " + "、".join(items) + "。使うのはこの順: " + "、".join(chosen) + "。使う物だけを『- 』で箇条書きにして。", "\n".join(f"- {item}" for item in chosen), {"operation": "bullets", "items": chosen}),
        (source + f"{extract_label}だけ抜き出して。説明は不要。", pack[extract_key], {"operation": "extract", "field": extract_key, "value": pack[extract_key]}),
        ("、".join(repeated) + "。重複を除き、最初に現れた順のまま読点で並べて。", "、".join(dict.fromkeys(repeated)), {"operation": "deduplicate", "input": repeated, "items": list(dict.fromkeys(repeated))}),
        (f"この引用を一度だけそのまま返して: {heading}", heading, {"operation": "literal", "value": heading}),
        (f"文『{after}前に、{before}』を、実際の順番が分かる一文にして。", f"「{before}」を先にしてから「{after}」を行います。", {"operation": "sequence_rewrite", "items": [before, after]}),
    ]
    result = []
    for variant, (question, answer, selection) in enumerate(rows):
        case = additional_context(raw, index, "format_selection", variant)
        case["question"], case["target"]["content"] = question, answer
        case["expected"].update({"kind": "exact", "text": answer})
        case["provenance"]["selection"] = selection
        case["provenance"]["values"] += [pack["state"], pack["time"], pack["place"], *items]
        case["expected"]["manual_rubric"].append("指定されたキー・値・対象項目・順番を入力から選ぶ。JSONの空白など意味の変わらない差は機械一致だけで失敗にしない。")
        result.append(case)
    return result


def additional_history(raw: dict, index: int, pack: dict) -> list[dict]:
    steps = pack["steps"]
    result = []
    for variant in range(8):
        case = additional_context(raw, index, "history_selection", variant)
        order = list(range(len(steps)))
        done, skipped = [], []
        case["history"] = [{"role": "user", "content": f"{pack['label']}の手順を確認したい。"}, {"role": "assistant", "content": "\n".join(f"{i + 1}. {step}" for i, step in enumerate(steps))}]
        if variant == 0:
            done, state = [0], "先頭の項目を済ませた。"
        elif variant == 1:
            done, state = [0, 1], "最初の二項目は済ませた。"
        elif variant == 2:
            done = [0, 2]
            state = f"『{steps[0]}』と『{steps[2]}』は済んだ。他はまだ。残りは元の順に進める。"
        elif variant == 3:
            done, skipped = [0], [1]
            state = "一番目は済んだ。二番目は今回は省略すると決めた。"
        elif variant == 4:
            order[1], order[2] = order[2], order[1]
            done, state = [0], "二番目と三番目の順を入れ替える。一番目は済んでいる。"
        elif variant == 5:
            done = [1]
            case["history"] += [{"role": "user", "content": "最初の二つは済んだ。"}, {"role": "assistant", "content": "最初の二項目が済んだんですね。"}]
            state = "さっきの報告を訂正。一番目はまだで、二番目だけが済んでいる。残りは元の順に進める。"
        elif variant == 6:
            done = order[:-1]
            case["history"] += [{"role": "user", "content": f"最後の項目以外は済んだ。手元には{pack['items'][0]}もある。"}, {"role": "assistant", "content": f"{pack['items'][0]}もあるんだね。"}]
            state = "直前の材料の話ではなく、手順の続きを知りたい。"
        else:
            done, state = order[:], "挙げた項目はすべて済んだ。"
        remaining = [i for i in order if i not in done and i not in skipped]
        case["question"] = state + "次に行う未完了の項目を一つだけ。残りがなければ、そのことだけ教えて。"
        if remaining:
            value = steps[remaining[0]]
            case["target"]["content"] = f"次は「{value}」です。"
            evidence = fact("user", "次の項目", value, "/history/1/content")
            case["provenance"]["facts"].append(evidence)
            case["expected"]["facts"] = [evidence]
        else:
            case["target"]["content"] = f"挙げた{len(steps)}項目はすべて済んでいます。"
        case["provenance"]["selection"] = {"operation": ("first", "two", "nonconsecutive", "skip", "reorder", "correction", "aside", "finished")[variant], "steps": steps, "order": order, "completed": done, "skipped": skipped, "selected": remaining[:1]}
        case["provenance"]["values"] += steps
        case["expected"]["manual_rubric"].append("最新の完了報告・順序訂正・省略を合わせて次の未完了項目を選ぶ。助手が作業を実行したとは言わない。")
        result.append(case)
    return result


def additional_chains(raw: dict, index: int, chain: dict) -> list[dict]:
    """一つの往復表から既に取得した部分だけを各段階へ渡す。"""
    number, total, place = chain["value"], chain["total"], chain["location"]
    path = f"教材/連携{index + 1:02d}.txt"
    percent = f"{number / total * 100:.2f}%"
    temperature = f"{chain['temperature']}°C"
    file_text = f"{path}\n3 行中 1〜3 行目\n1\tvalue={number}\n2\ttotal={total}\n3\tlocation={place}\n（行頭の「行番号+タブ」は表示用です。edit_file の old_string には含めないでください）"
    rounds = [
        ("read_file", {"path": path}, file_text),
        ("productivity_tools", {"action": "percent_calc", "value": number, "total": total}, f"{number} / {total} = {percent}"),
        ("weather", {"location": place}, f"**{place}の天気：{chain['weather']}**\nいまは {temperature}\n"),
    ]
    question = f"{path}を読んで、そのvalueとtotalから割合をpercent_calcで計算し、続いてlocationの現在の天気を取得して。最後は取得した割合と天気だけを短く教えて。"
    result = []
    for stage, continuation in chain_stages(rounds, f"chain_{index}"):
        case = additional_context(raw, index, "tool_chain", stage)
        case["question"] = question
        case["tool_names"] = [name for name, _, _ in rounds]
        case["continuation"] = continuation
        if stage < len(rounds):
            name, arguments, _ = rounds[stage]
            case["target"]["tool_calls"] = [tool_call(name, arguments, f"chain_{index}_{stage}")]
            case["expected"].update(kind="tool_call", tool_name=name, arguments=arguments)
        else:
            case["target"]["content"] = f"割合は{percent}です。{place}は{chain['weather']}で、気温は{temperature}です。"
            case["expected"]["required_values"] = [percent, place, chain["weather"], temperature]
        case["provenance"]["selection"] = {"operation": "tool_chain", "stage": stage, "chain_id": f"train-v3-chain-{index:02d}"}
        case["provenance"]["values"] += [path, place, str(number), str(total), percent, temperature]
        case["expected"]["manual_rubric"] += ["指定ファイル→割合計算→天気→最終回答の順に進み、未取得の値を先回りしない。", "途中のtool往復は入力資料であり、正解は現在の一段だけ。最終回答も得られた結果以外を追加しない。"]
        result.append(case)
    return result


def additional_cases() -> list[dict]:
    # 完成caseを複製せず、架空の情報表と会話対を共通の根拠形式へ変換する。
    material = json.loads((Path(__file__).parent / "fixtures" / "selection-v3.json").read_text(encoding="utf-8"))
    result = []
    for index, raw in enumerate(material["worlds"]):
        result += additional_attribution(raw, index)
    for index, pack in enumerate(material["packs"]):
        raw = material["worlds"][index]
        result += additional_formats(raw, index, pack)
        result += additional_history(raw, index, pack)
        result += additional_chains(raw, index, material["chains"][index])
    for pair_index, pair in enumerate(material["conversations"]):
        index = pair_index % len(material["worlds"])
        for variant, item in enumerate(pair["variants"]):
            case = additional_context(material["worlds"][index], index, "conversation_distinction", pair_index * 2 + variant)
            case["question"], case["target"]["content"] = item["q"], item["a"]
            case["history"] = copy.deepcopy(pair.get("history", []))
            if pair_index == 16 and variant == 0:
                case["provenance"]["masked_history_indices"] = [0]
            case["provenance"]["contrast_id"] = f"train-v3-conversation-{pair_index:02d}"
            case["provenance"]["selection"] = {"operation": "conversation", "focus": pair["focus"], "action": item["action"], "actor": item["actor"], "time": item["time"]}
            case["expected"]["manual_rubric"] += [f"期待する応答は『{item['action']}』。指示の相手と時点を読み分ける。", "未来の予定を現在の実行依頼へ変えず、未実行の作業や送信・記憶更新を完了と報告しない。"]
            result.append(case)
    return result


def render_material(value: object, context: dict):
    """材料の差分だけを展開し、ケース組立てと根拠台帳の正本を増やさない。"""
    if isinstance(value, str):
        return value.format_map(context)
    if isinstance(value, list):
        return [render_material(item, context) for item in value]
    if isinstance(value, dict):
        return {key: render_material(item, context) for key, item in value.items()}
    return value


def material_case(raw: dict, index: int, variant: int, template: dict) -> dict:
    case = additional_context(raw, 40 + index, template["category"], variant)
    context = {**raw, "first": case["provenance"]["first_person"], "ending": case["provenance"]["ending"]}
    for kind, label in (("飲み物", "drink"), ("食べ物", "food"), ("趣味", "hobby")):
        context.update({f"{owner}_{label}": value for owner, value in raw["attributes"][kind].items()})
    row = render_material(template, context)
    case["id"] = f"train-gate-{index:02d}-{row['name']}"
    case["group_id"] = f"train-gate-world-{index:02d}"
    case["provenance"]["extension"] = "product-gate"
    case["question"] = row["question"][index % len(row["question"])] if isinstance(row["question"], list) else row["question"]
    case["history"] = copy.deepcopy(row.get("history", []))
    case["provenance"]["masked_history_indices"] = row.get("masked", [])
    ledger = case["provenance"]["facts"]
    for entry in ledger:
        if [entry["owner"], entry["kind"]] in row.get("supersede", []):
            entry["status"] = "superseded"
    expected = select_facts(case, [tuple(request) for request in row.get("requests", [])])
    for entry in row.get("evidence", []):
        source = entry["source"]
        if source == "memory":
            source = f"/memories/{len(case['memories'])}"
            case["memories"].append(entry["text"])
        elif source == "profile":
            source = "/character/profile"
            case["character"]["profile"] += "\n" + entry["text"]
        elif source == "question":
            source = "/question"
        item = fact(entry["owner"], entry["kind"], entry["value"], source, entry["status"])
        ledger.append(item)
        expected.append(item)
        case["provenance"]["values"].append(item["value"])
    case["expected"]["facts"] = copy.deepcopy(expected)
    case["expected"]["unknown"] = [{"owner": owner, "kind": kind} for owner, kind in row.get("unknown", [])]
    case["target"]["content"] = row["answer"] if "answer" in row else attribute_answer(case, expected)
    case["provenance"]["selection"] = {"operation": row["name"], "requests": row.get("requests", []), "intent": row.get("intent", "主体・属性・最新の根拠を区別する")}
    if "contrast" in row:
        case["provenance"]["contrast_id"] = f"train-gate-{index:02d}-{row['contrast']}"
    case["expected"]["manual_rubric"] += ["人物・引用・否定・訂正の適用範囲を読み、明示された事実だけ答える。", "知らないこと、明示的な未定、既知の事実を区別し、訂正されていない根拠は捨てない。", row.get("intent", "指定された一人称を保持し、引用者の一人称を本人へ移さない。")]
    return case


def branching_chain_context(index: int, variant: int, material: dict, total: int | None = None) -> dict:
    """成功・失敗で共通の資料と呼出表を組み立て、未取得の後続結果を作らない。"""
    operators = {"ge": (lambda a, b: a >= b, "以上"), "gt": (lambda a, b: a > b, "より大きい"), "le": (lambda a, b: a <= b, "以下"), "lt": (lambda a, b: a < b, "未満"), "eq": (lambda a, b: a == b, "と等しい"), "ne": (lambda a, b: a != b, "と異なる")}
    predicate, wording = operators[material["operator"]]
    left, right = material["locations"]
    number, threshold = material["values"][variant], material["threshold"]
    total = material["total"] if total is None else total
    percent = f"{number / total * 100:.2f}%" if total else None
    branch = (0 if predicate(float(percent[:-1]), threshold) else 1) if percent else None
    place = material["locations"][branch] if branch is not None else None
    fields = [f"value = {number}", f"total = {total}"]
    if material["layout"] % 2:
        fields.reverse()
    condition = f"割合が{threshold}%{wording}なら{left}、それ以外は{right}の天気を取得する。"
    note = f"# 集計番号 {701 + index * 13} は計算に使わない。"
    lines = [note, *fields, condition] if material["layout"] < 2 else [condition, *fields, note]
    path = f"教材/判断{index + 1:02d}-{variant + 1}.txt"
    start = material["line_start"]
    end = start + len(lines) - 1
    file_text = f"{path}\n{end} 行中 {start}〜{end} 行目:\n" + "".join(f"{start + i}\t{line}\n" for i, line in enumerate(lines)) + '\n（行頭の「行番号+タブ」は表示用です。edit_file の old_string には含めないでください）'
    question = f"{path}の{start}行目から{len(lines)}行を読み、valueとtotalをpercent_calcへ渡して。表示の行番号は値ではないよ。返った割合で資料の条件を判定し、該当する一つの地域のweatherを使って。"
    rounds = [("read_file", {"path": path, "offset": start, "limit": len(lines)}, file_text), ("productivity_tools", {"action": "percent_calc", "value": number, "total": total}, f"{number} / {total} = {percent}" if percent else "")]
    if place is not None:
        rounds.append(("weather", {"location": place}, f"**{place}の天気：{material['weather']}**\nいまは {material['temperatures']}°C\n"))
    return {"path": path, "question": question, "rounds": rounds, "number": number, "total": total,
            "percent": percent, "branch": branch, "place": place, "variant": variant}


def branching_chain_cases(raw: dict, index: int, material: dict, contexts: list[dict]) -> list[dict]:
    result = []
    for context in contexts:
        variant, number, total = context["variant"], context["number"], context["total"]
        percent, branch, place = context["percent"], context["branch"], context["place"]
        path, rounds, weather = context["path"], context["rounds"], material["weather"]
        finish = material["finish"]
        endings = ["最後は『地域名：天気、割合』の形の一行だけで終了して。", "最後は地域名と天気だけを一文で。割合・気温・次の提案は書かず終えて。", "最後は割合だけを返して終了して。天気の内容や次の作業は書かないで。"]
        question = context["question"] + endings[finish]
        for stage, continuation in chain_stages(rounds, f"gate_{index}_{variant}"):
            template = {"name": f"chain-{variant}-{stage}", "category": "gate_tool_branch", "question": question, "answer": ""}
            case = material_case(raw, index, 24 + variant * 4 + stage, template)
            case["tool_names"] = [name for name, _, _ in rounds]
            case["continuation"] = continuation
            if stage < len(rounds):
                name, arguments, _ = rounds[stage]
                case["target"]["tool_calls"] = [tool_call(name, arguments, f"gate_{index}_{variant}_{stage}")]
                case["expected"].update(kind="tool_call", tool_name=name, arguments=arguments)
            else:
                answers = [f"{place}：{weather}、{percent}", f"{place}は{weather}{case['provenance']['ending']}", percent]
                case["target"]["content"] = answers[finish]
                case["expected"].update(kind="exact", text=answers[finish])
                entries = []
                if finish != 2:
                    entries += [fact("tool", "地域", place, "/continuation/5/content"), fact("tool", "天気", weather, "/continuation/5/content")]
                if finish != 1:
                    entries.append(fact("tool", "割合", percent, "/continuation/3/content"))
                case["expected"]["facts"] = copy.deepcopy(entries)
                case["provenance"]["facts"] += entries
            case["provenance"]["selection"] = {"operation": "branching_tools", "chain": material["name"], "variant": variant, "stage": stage, "operator": material["operator"], "threshold": material["threshold"], "selected_branch": branch, "value": number, "total": total, "line_start": material["line_start"], "locations": material["locations"], "finish": finish}
            case["provenance"]["values"] += [path, *material["locations"]]
            case["expected"]["manual_rubric"] += ["表示の行番号や集計番号をvalue/totalへ入れず、返った割合に条件を適用して一つの取得先を選ぶ。", "取得済みの結果だけを指定形式で答えて終了し、再読込・再計算・別地域の追加取得をしない。"]
            result.append(case)
    return result


def branching_failure_cases(raw: dict, index: int, material: dict, context: dict, failures: list[dict]) -> list[dict]:
    """同じ失敗と根拠を固定し、追加質問の許可だけを変えた対を作る。"""
    result = []
    for failure in failures:
        stage = failure["stage"]
        source = branching_chain_context(index, 0, material, failure["total"]) if "total" in failure else context
        rounds = source["rounds"][:stage + 1]
        name, arguments, _ = rounds[-1]
        rounds[-1] = (name, arguments, failure["error"])
        # 未取得の後続段を含めず、実際に返った失敗までのprefixだけを共有する。
        continuation = next(messages for count, messages in chain_stages(rounds, f"gate_error_{index}_{stage}") if count == len(rounds))
        entries = []
        if stage == 1:
            entries.append(fact("tool", "母数", str(source["total"]), "/continuation/1/content"))
        elif stage == 2:
            entries += [fact("tool", "割合", source["percent"], "/continuation/3/content"), fact("tool", "地域", source["place"], "/continuation/1/content")]
        for policy in failure["policies"]:
            row = render_material(policy, source)
            template = {"name": f"failure-{stage}-{row['name']}", "category": "gate_tool_failure", "question": source["question"] + row["question"], "answer": row["answer"], "contrast": f"failure-{stage}", "intent": row["intent"]}
            case = material_case(raw, index, 80 + stage, template)
            case["tool_names"] = ["read_file", "productivity_tools", "weather"]
            case["continuation"] = copy.deepcopy(continuation)
            case["expected"].update(kind="exact", text=case["target"]["content"], facts=copy.deepcopy(entries), unknown=[{"owner": "tool", "kind": kind} for kind in (["割合", "天気"] if stage < 2 else ["天気"])])
            case["provenance"]["facts"] += copy.deepcopy(entries)
            case["provenance"]["selection"] = {"operation": "branching_failure", "chain": material["name"], "stage": stage, "policy": row["name"], "total": source["total"], "percent": source["percent"] if stage == 2 else None}
            case["provenance"]["values"] += [source["path"], *[entry["value"] for entry in entries]]
            case["expected"]["manual_rubric"] += ["失敗した段で止まり、未取得の結果や原因を推測して補わない。途中の誤った呼出も教師ではない。", "追加質問は利用者が許可した項目を一度だけ。再試行や後続ツールを実行したとは言わない。"]
            result.append(case)
    return result


def product_gate_cases() -> list[dict]:
    material = json.loads((Path(__file__).parent / "fixtures/product-gate.json").read_text(encoding="utf-8"))
    result, failures = [], []
    for index, raw in enumerate(material["worlds"]):
        result += [material_case(raw, index, variant, template) for variant, template in enumerate(material["scenarios"])]
        chain = material["chains"][index]
        contexts = [branching_chain_context(index, variant, chain) for variant in range(len(chain["values"]))]
        result += branching_chain_cases(raw, index, chain, contexts)
        failures += branching_failure_cases(raw, index, chain, contexts[0], material["chain_failures"])
    for pair_index, pair in enumerate(material["conversations"]):
        index = pair_index % len(material["worlds"])
        for variant, item in enumerate(pair["variants"]):
            template = {**item, "category": "gate_conversation", "name": f"conversation-{pair_index:02d}-{variant}", "history": pair["history"], "contrast": f"conversation-{pair_index:02d}"}
            result.append(material_case(material["worlds"][index], index, 32 + pair_index * 2 + variant, template))
    return result + failures


def generate_dataset(extension: str | None = None) -> dict:
    if extension not in (None, "product-gate"):
        raise ValueError(f"未対応の教材追加です: {extension}")
    cases = []
    for split in SPLITS:
        count = 10 if split == "train" else 1
        for i in range(count):
            w = world(split, i)
            cases += attribution_cases(w)
            cases += practical_cases(w)
            cases += tool_cases(w)
            cases += conversation_cases(w)
        if split != "train":
            # 最終には未学習の日常会話を20件以上含める。
            for i in range(1, len(WORLDS[split]["tasks"])):
                cases += conversation_cases(world(split, i))
    data = {"metadata": {"schema_version": VERSION, "purpose": "帰属と日常支援の合成LoRA教材。最終評価は候補選定・早期停止・教材追加に使用しない。", "target_policy": "targetだけを学習し、history・continuation・人格・資料をすべてマスクする。", "tool_policy": "tool_namesをRustの登録済みschemaへ解決し、body.toolsを学習に使う。", "source": "この生成器とfixtures/selection-v3.jsonに定義した架空人物・架空事実のみ。実利用者の記憶・会話ファイルは読まない。", "development_reference": "mainの16問とPR #489の24問は別途開発評価として保持する。", "tool_sources": ["skills/weather/mod.rs", "skills/read_file/mod.rs", "skills/productivity_tools/mod.rs"]}, "cases": cases}
    data["cases"].extend(additional_cases())
    data["metadata"]["training_extension"] = {"version": 3, "cases": 348, "source": "fixtures/selection-v3.json", "policy": "既存710件と開発86件・最終100件を変更せず、情報選択の異なる学習ケースのみ追加する。"}
    if extension == "product-gate":
        additions = product_gate_cases()
        data["cases"].extend(additions)
        data["metadata"]["product_gate_extension"] = {"version": 2, "cases": len(additions), "source": "fixtures/product-gate.json", "policy": "旧1058学習・開発86・最終100と最初の追加304件を固定し、失敗型の境界対と分岐連鎖だけ追加する。"}
        data["metadata"]["source"] += " 追加材料はfixtures/product-gate.jsonの架空事実だけを使う。"
    validate_dataset(data)
    data["metadata"]["splits"] = {
        split: {
            "count": len(selected := [c for c in cases if c["split"] == split]),
            "groups": len({c["group_id"] for c in selected}),
            "categories": dict(sorted(Counter(c["category"] for c in selected).items())),
            "sha256": digest(selected),
            "entities": sorted({value for c in selected for value in c["provenance"]["entities"].values()}),
            "values": sorted({value for c in selected for value in c["provenance"]["values"]}),
            "normalized_inputs_sha256": digest(sorted(normalized(canonical({k: c[k] for k in ("character", "memories", "history", "question", "continuation")})) for c in selected)),
        } for split in SPLITS
    }
    return data


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="教材JSONの保存先。既存ファイルは上書きしない。")
    parser.add_argument("--extension", choices=("product-gate",), help="固定済み教材を変えずに追加する材料。省略時は既存版を生成する。")
    args = parser.parse_args()
    data = generate_dataset(args.extension)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(data, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    counts = {split: info["count"] for split, info in data["metadata"]["splits"].items()}
    print(f"教材を保存しました: {args.output} / 件数: {counts}")


if __name__ == "__main__":
    main()
