//! 利用者の質問原文を保持し、引用外の明示的な話者指示だけを送信用に解決する。

pub(super) struct QuestionBinding {
    pub resolved: String,
    pub directive: &'static str,
}

pub(super) fn bind(question: &str) -> QuestionBinding {
    let mut output = String::with_capacity(question.len());
    let mut remaining = question;
    let mut quotes = crate::text::QuoteState::default();
    let mut user = false;
    let mut character_subject = false;
    let mut personal_fact = false;
    while let Some(character) = remaining.chars().next() {
        if !quotes.enter(character) {
            // 文の翻訳や見出し説明まで人物の事実質問へ読み替えない。
            personal_fact |= [
                "好き",
                "好み",
                "趣味",
                "名前",
                "飲み物",
                "飲物",
                "食べ物",
                "果物",
                "覚えて",
                "普段",
                "毎日",
                "何杯",
                "得意",
                "苦手",
                "嫌い",
                "住んで",
                "職業",
            ]
            .iter()
            .any(|word| remaining.starts_with(word));
            if ["利用者の", "利用者は", "ユーザーの", "ユーザーは"]
                .iter()
                .any(|prefix| remaining.starts_with(prefix))
            {
                user = true;
            }
            if ["キャラクターの", "キャラの"]
                .iter()
                .any(|prefix| remaining.starts_with(prefix))
            {
                character_subject = true;
            }
            let replacement = [
                ("わたし", "利用者"),
                ("ワタシ", "利用者"),
                ("ぼく", "利用者"),
                ("僕", "利用者"),
                ("俺", "利用者"),
                ("私", "利用者"),
                ("あなた", "選択中キャラクター"),
                ("きみ", "選択中キャラクター"),
                ("君", "選択中キャラクター"),
            ]
            .into_iter()
            .find(|(pronoun, _)| {
                // 敬称や単語の一部を二人称へ読み替えない。
                let boundary = output.chars().next_back().is_none_or(|previous| {
                    previous.is_whitespace()
                        || "、，。？！?!:：;；()（）[]【】「」『』\"'".contains(previous)
                }) || ["そして", "それから", "しかし", "では", "また", "それでは"]
                    .iter()
                    .any(|prefix| output.ends_with(prefix))
                    || if *pronoun == "君" {
                        [
                            "利用者と",
                            "利用者は",
                            "利用者に",
                            "利用者の",
                            "選択中キャラクターと",
                        ]
                        .iter()
                        .any(|prefix| output.ends_with(prefix))
                    } else {
                        output
                            .chars()
                            .next_back()
                            .is_some_and(|previous| "はがのとにをもでへやか".contains(previous))
                    };
                boundary
                    && remaining.strip_prefix(pronoun).is_some_and(|rest| {
                        rest.is_empty()
                            || rest.starts_with([
                                'の', 'は', 'が', 'に', 'を', 'と', 'も', 'だ', 'っ', '、', '，',
                                '。', '？', '?',
                            ])
                    })
            });
            if let Some((pronoun, subject)) = replacement {
                let rest = remaining.strip_prefix(pronoun).expect("一致済みの指示語");
                if rest.starts_with(['の', 'は', 'が', 'と']) {
                    if subject == "利用者" {
                        user = true;
                    } else {
                        character_subject = true;
                    }
                }
                output.push_str(subject);
                remaining = rest;
                continue;
            }
        }
        output.push(character);
        remaining = remaining
            .strip_prefix(character)
            .expect("確認済みの先頭文字");
    }
    let directive = match (personal_fact, user, character_subject) {
        (true, true, false) => "回答対象は選択中キャラクターではなく、利用者です。",
        (true, false, true) => "回答対象は利用者ではなく、選択中キャラクターです。",
        (true, true, true) => "回答対象は利用者と選択中キャラクターの両方です。それぞれの事実を区別して答えてください。",
        _ => "",
    };
    QuestionBinding {
        resolved: output,
        directive,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 明示した話者を解決し引用と近い単語と曖昧な自分は変えない() {
        for (original, expected) in [
            ("私の好きな飲み物は？", "利用者の好きな飲み物は？"),
            (
                "あなたの好きな飲み物は？",
                "選択中キャラクターの好きな飲み物は？",
            ),
            (
                "僕と君の趣味を教えて",
                "利用者と選択中キャラクターの趣味を教えて",
            ),
            ("わたしは何が好き？", "利用者は何が好き？"),
            (
                "俺の好みとあなたの好み",
                "利用者の好みと選択中キャラクターの好み",
            ),
            (
                "彼の『私はコーヒーが好き』と私の好みを比較",
                "彼の『私はコーヒーが好き』と利用者の好みを比較",
            ),
            (
                "「あなたの趣味は？」という文を訳して",
                "「あなたの趣味は？」という文を訳して",
            ),
            ("私立大学と君主と自分の趣味", "私立大学と君主と自分の趣味"),
            ("彼は自分の趣味を語った", "彼は自分の趣味を語った"),
            ("むぎの好きな食べ物", "むぎの好きな食べ物"),
            ("田中君の好きな飲み物は？", "田中君の好きな飲み物は？"),
            ("山田君は何が好き？", "山田君は何が好き？"),
            ("はるか君の好きな飲み物", "はるか君の好きな飲み物"),
            ("まこと君の趣味", "まこと君の趣味"),
            ("そしてあなたの趣味", "そして選択中キャラクターの趣味"),
            ("それから私の好み", "それから利用者の好み"),
            ("しかしあなたの名前", "しかし選択中キャラクターの名前"),
            (
                "`あなたの好きな飲み物は？`を英訳して",
                "`あなたの好きな飲み物は？`を英訳して",
            ),
            ("```\n私の趣味\n```を説明", "```\n私の趣味\n```を説明"),
            ("あおいと私の趣味", "あおいと利用者の趣味"),
        ] {
            assert_eq!(super::bind(original).resolved, expected, "{original}");
        }
    }

    #[test]
    fn 回答対象は引用や受け手と混ぜず双方質問と未知質問を区別する() {
        for (question, directive) in [
            ("私の飲み物", "回答対象は選択中キャラクターではなく、利用者です。"),
            ("あなたに私の飲み物を教えてほしい", "回答対象は選択中キャラクターではなく、利用者です。"),
            ("私にあなたの趣味を教えて", "回答対象は利用者ではなく、選択中キャラクターです。"),
            ("私とあなたの好み", "回答対象は利用者と選択中キャラクターの両方です。それぞれの事実を区別して答えてください。"),
            ("『私の飲物』という文を英訳", ""),
            ("彼は自分の飲物を選ぶ", ""),
            ("今日はどう過ごそう？", ""),
            ("田中君の好きな飲み物は？", ""),
            ("山田君は何が好き？", ""),
            ("はるか君の好きな飲み物", ""),
            ("まこと君の趣味", ""),
            ("`あなたの好きな飲み物は？`を英訳して", ""),
            ("```\n私の趣味\n```を説明", ""),
        ] {
            assert_eq!(super::bind(question).directive, directive, "{question}");
        }
    }
}
