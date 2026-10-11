//! 利用者の質問原文を保持し、引用外の明示的な話者指示だけを送信用に解決する。

pub(super) struct QuestionBinding {
    pub resolved: String,
    pub directive: &'static str,
    pub quotations: Vec<QuoteBinding>,
}

/// 位置は質問原文の文字単位で、開始引用符を含み終了引用符の直後まで。
#[derive(serde::Serialize, Debug, PartialEq)]
pub(super) struct QuoteBinding {
    start: usize,
    end: usize,
    speaker: String,
    recipient: String,
}

fn explicit_quote_relations(question: &str) -> Vec<QuoteBinding> {
    let mut state = crate::text::QuoteState::default();
    let mut opened = None;
    let mut nested = false;
    let mut bindings = Vec::new();
    for (position, (byte, character)) in question.char_indices().enumerate() {
        let depth = state.depth();
        let code = state.in_code();
        state.enter(character);
        if depth == 0 && state.depth() == 1 && !code && !state.in_code() {
            opened = Some((byte, position));
            nested = false;
        } else if state.depth() > 1 {
            nested = true;
        } else if depth == 1 && state.depth() == 0 {
            let Some((start_byte, start)) = opened.take() else {
                continue;
            };
            if nested {
                continue;
            }
            let prefix = question.get(..start_byte).expect("文字境界の引用開始");
            let prefix = prefix
                .rsplit(['。', '！', '？', '\n', '!', '?'])
                .next()
                .unwrap_or("")
                .trim();
            let Some(participants) = prefix.strip_suffix('に') else {
                continue;
            };
            let Some((speaker, recipient)) = participants.split_once('が') else {
                continue;
            };
            // 複雑な節や匿名の人物を、確定した引用関係へ推測で補わない。
            let named = |value: &str| {
                !value.is_empty()
                    && !value.contains(['が', '、', '，', '「', '」', '『', '』', '`', ':', '：'])
                    && !value.chars().any(char::is_whitespace)
                    && !value.chars().skip(1).any(|character| character == 'は')
                    && !["もし", "仮に", "例えば", "たとえば"]
                        .iter()
                        .any(|prefix| value.starts_with(prefix))
                    && !["誰", "誰か", "ある人", "相手", "彼", "彼女", "自分"].contains(&value)
            };
            if !named(speaker) || !named(recipient) {
                continue;
            }
            let suffix = question
                .get(byte + character.len_utf8()..)
                .expect("文字境界の引用終了");
            let said = ["と言いました", "と言った", "と言う"].iter().any(|verb| {
                suffix.strip_prefix(verb).is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with(['。', '！', '!', '\n'])
                })
            });
            if !said {
                continue;
            }
            // 関係の注釈だけを生成し、引用内容の事実や答えはコードで作らない。
            bindings.push(QuoteBinding {
                start,
                end: position + 1,
                speaker: bind(speaker).resolved,
                recipient: bind(recipient).resolved,
            });
        }
    }
    bindings
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
    // 指示語は局所的な参照先だけを示す。所有者から質問全体の回答対象を推測しない。
    let directive = if personal_fact && (user || character_subject) {
        "送信用表記の「利用者」はこの質問の引用外の一人称、「選択中キャラクター」は引用外の呼び掛けの相手を指します。所有格は所有者だけを示し、所有物や第三者が事実の主体である場合はその主体を保ってください。質問全体の回答対象をこの指示語だけで限定しないでください。"
    } else {
        ""
    };
    QuestionBinding {
        resolved: output,
        directive,
        quotations: explicit_quote_relations(question),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 明示した引用関係は人物値と位置だけを注釈し原文を変えない() {
        for (question, speaker, recipient) in [
            (
                "あおいがけんに「あなたは麦茶が好き」と言いました。このあなたは誰？",
                "あおい",
                "けん",
            ),
            (
                "ゆうがむぎに『私は緑茶が好き』と言った。引用の私は誰？",
                "ゆう",
                "むぎ",
            ),
            (
                "私があなたに「私は緑茶が好き」と言いました。",
                "利用者",
                "選択中キャラクター",
            ),
            (
                "田中君が山田君に「あなたの趣味は？」と言った。",
                "田中君",
                "山田君",
            ),
            (
                "友人のあおいがはるかに「あなたの趣味は？」と言った。",
                "友人のあおい",
                "はるか",
            ),
        ] {
            let binding = super::bind(question);
            assert_eq!(binding.quotations.len(), 1, "{question}");
            let quote = &binding.quotations[0];
            assert_eq!(quote.speaker, speaker);
            assert_eq!(quote.recipient, recipient);
            let original: String = question
                .chars()
                .skip(quote.start)
                .take(quote.end - quote.start)
                .collect();
            assert!(original.starts_with(['「', '『']));
            assert!(original.ends_with(['」', '』']));
            assert_eq!(binding.resolved.matches(&original).count(), 1);
            let json = serde_json::to_value(&binding.quotations).unwrap();
            assert_eq!(json[0].as_object().unwrap().len(), 4);
            assert!(json.to_string().find("麦茶").is_none());
        }
    }

    #[test]
    fn 複数独立引用の関係は順序を保ち引用外の質問とも共存する() {
        let question = "あおいがけんに「あなたは麦茶好き」と言った。ゆうがむぎに『私は緑茶好き』と言いました。私の飲み物と比べて";
        let binding = super::bind(question);
        assert_eq!(binding.quotations.len(), 2);
        assert_eq!(binding.quotations[0].speaker, "あおい");
        assert_eq!(binding.quotations[0].recipient, "けん");
        assert_eq!(binding.quotations[1].speaker, "ゆう");
        assert_eq!(binding.quotations[1].recipient, "むぎ");
        assert!(binding.quotations[0].end < binding.quotations[1].start);
        assert!(binding.resolved.ends_with("利用者の飲み物と比べて"));
    }

    #[test]
    fn 曖昧な引用やコードや入れ子や発言確認質問を確定関係へ補わない() {
        for question in [
            "あおいとけんの「あなた」を英訳して",
            "あおいがけんに会って「あなたは麦茶好き」と言った。",
            "誰かがけんに「あなたは麦茶好き」と言った。",
            "あおいが誰かに「あなたは麦茶好き」と言った。",
            "あおいがけんに「あなたは麦茶好き」と言った？",
            "あおいがけんに「あなたは麦茶好き」と言ったらどうなる？",
            "私はあおいがけんに「あなたは麦茶好き」と言った。",
            "もしあおいがけんに「あなたは麦茶好き」と言った。",
            "あおいがけんに「あなたは麦茶好き」と言いました、という文を訳して",
            "あおいがけんに「あなたは麦茶好きと言った。",
            "あおいがけんに「母が『あなたは麦茶好き』と言った」と言いました。",
            "`あおいがけんに「あなたは麦茶好き」と言った。`を訳して",
            "```\nあおいがけんに「あなたは麦茶好き」と言った。\n```を説明",
        ] {
            assert!(super::bind(question).quotations.is_empty(), "{question}");
        }
    }

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
    fn 局所的な指示語説明は一般作業や引用へ追加せず所有者を回答対象へ昇格しない() {
        for question in [
            "私の飲み物",
            "あなたに私の飲み物を教えてほしい",
            "私にあなたの趣味を教えて",
            "私とあなたの好み",
        ] {
            let binding = super::bind(question);
            assert!(!binding.directive.is_empty(), "{question}");
            assert!(binding.directive.contains("所有格は所有者だけ"));
            assert!(!binding.directive.contains("回答対象は"));
        }
        for question in [
            "『私の飲物』という文を英訳",
            "彼は自分の飲物を選ぶ",
            "今日はどう過ごそう？",
            "田中君の好きな飲み物は？",
            "山田君は何が好き？",
            "はるか君の好きな飲み物",
            "まこと君の趣味",
            "`あなたの好きな飲み物は？`を英訳して",
            "```\n私の趣味\n```を説明",
        ] {
            assert!(super::bind(question).directive.is_empty(), "{question}");
        }
    }

    #[test]
    fn 所有格と第三者との比較を原文の関係のまま保ち唯一の回答対象を断定しない() {
        for (question, resolved) in [
            ("私の兄の好みを教えて", "利用者の兄の好みを教えて"),
            (
                "あなたの友人の趣味を教えて",
                "選択中キャラクターの友人の趣味を教えて",
            ),
            (
                "私の好きな作家の出身を教えて",
                "利用者の好きな作家の出身を教えて",
            ),
            ("あおいと私の好みを比べて", "あおいと利用者の好みを比べて"),
            ("私の好きな飲み物を教えて", "利用者の好きな飲み物を教えて"),
            ("私の趣味を教えて", "利用者の趣味を教えて"),
            (
                "私とあなたの好みを教えて",
                "利用者と選択中キャラクターの好みを教えて",
            ),
            (
                "私にあなたの趣味を教えて",
                "利用者に選択中キャラクターの趣味を教えて",
            ),
        ] {
            let binding = super::bind(question);
            assert_eq!(binding.resolved, resolved, "{question}");
            assert!(
                binding.directive.contains("所有格は所有者だけ"),
                "{question}"
            );
            assert!(binding
                .directive
                .contains("質問全体の回答対象をこの指示語だけで限定しない"));
            assert!(!binding.directive.contains("回答対象は"), "{question}");
        }
    }
}
