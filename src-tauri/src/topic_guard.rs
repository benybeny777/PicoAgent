// topic_guard.rs - 会話のブロックリスト（話題ガード）。
//
// ホロライブ等の公式キャラクターを扱ううえで、AIに話させてはいけない話題（中の人・前世の
// 詮索、身元の特定、卒業・引退や炎上の憶測、性的な話題など）を、AIを使わない決定的な照合で
// 止める。小型のローカルAIは指示どおりに話題を避けないことがあるため、モデルの判断には頼らない。
//
// - 一覧はコードへ固定する。設定や外部ファイルから変更・無効化できない（標準機能として常に有効）。
// - 入力は `agent::run` と各送信コマンドの入口で、出力は唯一の公開検査
//   `agent::guard_public_answer()` で照合する。どちらかで一致したら、AIへ送らず・AIの本文を
//   出さず、固定の返答 `BLOCKED_REPLY` に差し替える。
// - 通常の作業相談を巻き込まないよう、一般語（「卒業」「本名」など）は、相手や自分を指す語・
//   ホロライブを指す語の直後に来た場合だけ止める。

/// 話題を止めたときに、AIの返答の代わりに出す固定の一言。
pub(crate) const BLOCKED_REPLY: &str =
    "ごめんね、その話題にはお答えできないんだ。別のお話をしよう！";

/// 照合する向き。主語として扱う語が変わる（入力は相手＝キャラ、出力は自分＝キャラ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Input,
    Output,
}

/// 文中のどこにあっても止める語。単独でホロライブ対策の話題と分かる、一般の作業では
/// まず現れない言い回しだけを置く。
const ALWAYS_BLOCKED: &[(&str, &[&str])] = &[
    (
        "中の人・前世の詮索",
        &[
            "中の人",
            "なかのひと",
            "ナカノヒト",
            "前世バレ",
            "前世特定",
            "前世は誰",
            "前世はだれ",
            "前世の名前",
            "魂の人",
            "声優の正体",
            "正体は誰",
            "正体はだれ",
            "演者は誰",
            "演者はだれ",
        ],
    ),
    (
        "身元の特定",
        &[
            "身バレ",
            "顔バレ",
            "声バレ",
            "特定班",
            "住所特定",
            "本名特定",
            "素顔流出",
        ],
    ),
    (
        "卒業・引退・炎上の憶測",
        &[
            "卒業理由",
            "引退理由",
            "不仲説",
            "裏垢",
            "裏アカ",
            "彼氏バレ",
        ],
    ),
    (
        "性的な話題",
        &[
            "セックス",
            "エッチな",
            "エッチし",
            "エロい",
            "エロ画像",
            "エロ動画",
            "エロ話",
            "ヌード",
            "全裸",
            "裸になって",
            "裸を見せ",
            "下着の色",
            "スリーサイズ",
            "おっぱい",
        ],
    ),
    ("政治・宗教", &["支持政党", "投票先"]),
];

/// 主語の直後に来た場合だけ止める語。単独では通常の作業にも出るため、誰のことかで絞る。
const WITH_SUBJECT: &[(&str, &[&str])] = &[
    ("中の人・前世の詮索", &["前世", "転生前"]),
    (
        "身元の特定",
        &[
            "本名",
            "実名",
            "素顔",
            "顔写真",
            "住んでる",
            "住んでいる",
            "どこに住",
            "最寄り駅",
            "出身校",
            "勤務先",
            "個人情報流出",
        ],
    ),
    (
        "恋愛・私生活の詮索",
        &["彼氏", "恋人", "結婚して", "既婚", "付き合ってる人"],
    ),
    (
        "卒業・引退・炎上の憶測",
        &[
            // 「卒業」「引退」単独は「あなたの卒業を祝う」のような依頼にも出るため、
            // 本人の進退を尋ねる・語る形に限る。
            "卒業する",
            "卒業しない",
            "卒業予定",
            "卒業発表",
            "卒業って",
            "卒業は",
            "引退する",
            "引退しない",
            "引退予定",
            "引退発表",
            "引退って",
            "引退は",
            "辞める",
            "活動休止",
            "炎上",
            "解雇",
            "契約解除",
            "契約解消",
            "匂わせ",
        ],
    ),
    ("収入の詮索", &["年収", "月収", "給料", "収益", "稼ぎ"]),
    ("政治・宗教", &["政党", "宗教", "信仰"]),
];

/// どちらの向きでも主語として扱う、ホロライブ・VTuberを指す語。直後に「メンバー」などが
/// 挟まっても主語として扱う。「タレント」「配信者」は事務所や動画の収益の相談にも出るため置かない。
const SHARED_SUBJECTS: &[&str] = &[
    "ホロライブ",
    "hololive",
    "ホロメン",
    "ホロプロ",
    "カバー株式会社",
    "vtuber",
    "ブイチューバー",
];

/// 入力の主語。利用者がキャラクター（相手）について尋ねる語。
const INPUT_SUBJECTS: &[&str] = &[
    "あなた",
    "あんた",
    "きみ",
    "キミ",
    "君",
    "おまえ",
    "お前",
    "てめえ",
];

/// 出力の主語。キャラクターが自分について話す語。
const OUTPUT_SUBJECTS: &[&str] = &[
    "わたし",
    "私",
    "あたし",
    "わたくし",
    "ぼく",
    "僕",
    "おれ",
    "俺",
];

/// 主語から対象語の始まりまでに許す文字数（「あなたの」「私って」「ホロライブの」など）。
/// 広げるほど、主語と無関係な一般語まで巻き込む。
const SUBJECT_GAP: usize = 6;

/// 人を指す主語の直後に来るべき文字（助詞）。「私立」「君たち」のように語の一部として
/// 現れた場合を主語と見なさない。語の直後に対象語が続く「あなた本名は」も認める。
const SUBJECT_PARTICLES: &[char] = &['の', 'は', 'が', 'も', 'っ', 'を', 'に', 'と', 'さ', '、'];

/// 照合用に正規化する。空白と区切り記号を除き、全角英数を半角へ寄せて小文字にする。
/// 「中 の 人」「中・の・人」「ＶＴｕｂｅｒ」のような書き方の差で素通りさせない。
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '・' | '･'
                        | '.'
                        | '．'
                        | '。'
                        | '-'
                        | '_'
                        | '＿'
                        | '*'
                        | '＊'
                        | '/'
                        | '／'
                        | '|'
                        | '｜'
                        | '"'
                        | '\''
                        | '「'
                        | '」'
                        | '『'
                        | '』'
                )
        })
        .map(|c| match c {
            // 読点は主語の区切り（「僕、引退する」）として残し、1種類へ揃える。
            ',' | '，' => '、',
            '！'..='～' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// 語も同じ正規化をかけてから比べる（全角英数や大文字で書いた語も揃える）。
fn normalized_terms(terms: &[&str]) -> Vec<String> {
    terms.iter().map(|term| normalize(term)).collect()
}

/// `subject` の出現位置の直後 `SUBJECT_GAP` 文字以内に `term` が始まるか。
/// `needs_particle` の主語は、直後が助詞か対象語のときだけ主語と見なす。
fn follows_subject(text: &[char], subject: &[char], term: &[char], needs_particle: bool) -> bool {
    if subject.is_empty() || term.is_empty() {
        return false;
    }
    (0..text.len().saturating_sub(subject.len() - 1))
        .filter(|&start| text[start..].starts_with(subject))
        .any(|start| {
            let after = start + subject.len();
            if needs_particle
                && !text[after..].starts_with(term)
                && !text
                    .get(after)
                    .is_some_and(|c| SUBJECT_PARTICLES.contains(c))
            {
                return false;
            }
            (after..=(after + SUBJECT_GAP).min(text.len()))
                .any(|begin| text[begin..].starts_with(term))
        })
}

/// 止める話題に当たれば、その分類名を返す（ログ用。利用者の本文は記録しない）。
pub(crate) fn blocked_category(text: &str, direction: Direction) -> Option<&'static str> {
    let normalized = normalize(text);
    // 常に止める語は、読点を挟んだ書き方（「中、の、人」）でも見つける。
    let joined = normalized.replace('、', "");
    if normalized.is_empty() {
        return None;
    }
    for (category, terms) in ALWAYS_BLOCKED {
        if normalized_terms(terms)
            .iter()
            .any(|term| !term.is_empty() && joined.contains(term.as_str()))
        {
            return Some(category);
        }
    }
    let own_subjects = match direction {
        Direction::Input => INPUT_SUBJECTS,
        Direction::Output => OUTPUT_SUBJECTS,
    };
    let chars: Vec<char> = normalized.chars().collect();
    let subjects: Vec<(Vec<char>, bool)> = own_subjects
        .iter()
        .map(|subject| (normalize(subject).chars().collect(), true))
        .chain(
            SHARED_SUBJECTS
                .iter()
                .map(|subject| (normalize(subject).chars().collect(), false)),
        )
        .collect();
    for (category, terms) in WITH_SUBJECT {
        for term in normalized_terms(terms) {
            let term: Vec<char> = term.chars().collect();
            if subjects.iter().any(|(subject, needs_particle)| {
                follows_subject(&chars, subject, &term, *needs_particle)
            }) {
                return Some(category);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{blocked_category, Direction, BLOCKED_REPLY};

    #[test]
    fn 中の人や前世の詮索は書き方を変えても止める() {
        for text in [
            "中の人って誰なの？",
            "中 の 人 教えて",
            "中・の・人",
            "中、の、人",
            "ナカノヒトは？",
            "前世の名前を知ってる？",
            "身バレしたって本当？",
        ] {
            assert!(
                blocked_category(text, Direction::Input).is_some(),
                "{text} を止めていません"
            );
        }
    }

    #[test]
    fn 一般語は相手やホロライブを指すときだけ止める() {
        for text in [
            "あなたの本名は？",
            "君って彼氏いるの？",
            "お前はいつ卒業するの",
            "ホロライブの卒業って何があったの",
            "ホロライブメンバーの本名一覧",
            "ＶＴｕｂｅｒの年収ってどのくらい？",
            "あなたの支持する宗教は？",
            "あなたの前世って何？",
            "ホロライブの契約解除の噂",
        ] {
            assert!(
                blocked_category(text, Direction::Input).is_some(),
                "{text} を止めていません"
            );
        }
    }

    #[test]
    fn 通常の作業相談は止めない() {
        for text in [
            "卒業式の案内文を作って",
            "本名と住所の欄があるフォームを作りたい",
            "私の年収から所得税を計算して",
            "彼女に送る誕生日メッセージを考えて",
            "炎上したときの広報対応の手順をまとめて",
            "今日の予定を教えて",
            "そろそろ帰ろうかな",
            "スケッチのコツは？",
            "タレント事務所の収益を集計して",
            "あなたの卒業を祝うメッセージを書いて",
            "前世代のGPUと比べて",
            "前世紀の建築様式",
            "契約解除通知書を作って",
            "個人情報流出時の対応手順",
            "女バレの練習メニュー",
            "エッチング工程の説明",
            "エラーログを確認して",
            "PCを休止状態にして",
            "匂わせ投稿の企画案",
            "あなた、その処理をやめて",
            "エラーの原因を調べて",
        ] {
            assert_eq!(
                blocked_category(text, Direction::Input),
                None,
                "{text} を止めています"
            );
        }
    }

    #[test]
    fn 出力はキャラクター自身についての話を止める() {
        assert!(blocked_category("私の本名はね、", Direction::Output).is_some());
        assert!(blocked_category("僕、来月で引退するんだ", Direction::Output).is_some());
        // 利用者について話すのは止めない（入力の主語は出力では主語にしない）。
        assert_eq!(
            blocked_category("あなたの年収なら税額は約20万円です。", Direction::Output),
            None
        );
        // 語の一部として現れた「私」は主語にしない。
        assert_eq!(
            blocked_category("私立大学を卒業した場合の手続きです。", Direction::Output),
            None
        );
        // 主語から離れた一般語は巻き込まない。
        assert_eq!(
            blocked_category(
                "私がまとめた手順です。炎上時はまず事実確認を行います。",
                Direction::Output
            ),
            None
        );
    }

    #[test]
    fn 固定の返答自体は止めない() {
        assert_eq!(blocked_category(BLOCKED_REPLY, Direction::Output), None);
    }
}
