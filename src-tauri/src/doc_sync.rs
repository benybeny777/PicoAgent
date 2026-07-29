// doc_sync.rs - ドキュメントと実装のズレを検出する設計ガード（テスト専用）。
//
// 「設定キーを足したら SPEC.md へ書く」という運用は AGENTS.md のルールだが、人が
// 覚えているだけでは抜ける（実際に過去のレビューで `PICOAGENT_NAP_MINUTES` など
// 3件の記載漏れが見つかり、そのたびに手作業で突き合わせていた）。突き合わせ自体を
// テストにしておけば、書き忘れたその場で落ちる。

/// 設定キーとして扱わない文字列（マーカー・テスト用ダミー・ドキュメント中の例）。
/// 追加するときは、なぜ設定キーでないのかを理由として書き添えること。
const NOT_CONFIG_KEYS: [(&str, &str); 6] = [
    (
        "PICOAGENT_COMMAND_APPROVAL__",
        "run_command と対話ループの内部プロトコルのマーカー",
    ),
    ("PICOAGENT_TEST_KEY", "config のテストで使うダミー"),
    ("PICOAGENT_TEST_LIST", "store の並列更新テストで使うダミー"),
    ("PICOAGENT_OTHER", "config のテストで使うダミー"),
    (
        "PICOAGENT_XXX",
        "ドキュメント・コメント中の説明用プレースホルダ",
    ),
    (
        "PICOAGENT_OPENAI_API_KEY",
        "secret のテストで使う架空キー（実キーは OPENAI_API_KEY）",
    ),
];

/// ソース中に現れる `PICOAGENT_*` を集める。
fn referenced_keys() -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    for source in crate::guard_scan::all_sources() {
        collect_keys(&source.text, &mut keys);
    }
    // フロント（JS）とセットアップスクリプトも設定キーを参照する。
    for relative in [
        "ui/renderer.js",
        "ui/settings.js",
        "ui/picoagent.js",
        "scripts/fetch-tools.mjs",
        "scripts/picoagent-cli.mjs",
    ] {
        if let Some(text) = read_repo_file(relative) {
            collect_keys(&text, &mut keys);
        }
    }
    for (skip, _) in NOT_CONFIG_KEYS {
        keys.remove(skip);
    }
    keys
}

fn collect_keys(text: &str, out: &mut std::collections::BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while let Some(found) = text[index..].find("PICOAGENT_") {
        let start = index + found;
        let mut end = start + "PICOAGENT_".len();
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        out.insert(text[start..end].to_string());
        index = end;
    }
}

fn read_repo_file(relative: &str) -> Option<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    std::fs::read_to_string(root.join(relative)).ok()
}

#[cfg(test)]
mod guard {
    use super::{read_repo_file, referenced_keys};

    /// 実装が参照する設定キーは、すべて `SPEC.md` に載っていること。
    #[test]
    fn 設定キーはspecに記載する() {
        let spec = read_repo_file("SPEC.md").expect("SPEC.md を読めない");
        let keys = referenced_keys();
        assert!(
            keys.len() > 150,
            "設定キーの収集に失敗しています: {}",
            keys.len()
        );

        let missing: Vec<&String> = keys.iter().filter(|key| !spec.contains(*key)).collect();
        assert!(
            missing.is_empty(),
            "SPEC.md に記載のない設定キーがあります（実装だけ増やしてドキュメントを更新していません）: {missing:?}"
        );
    }

    /// `SPEC.md` に載っているのに実装のどこからも参照されないキーが無いこと（削除漏れ）。
    #[test]
    fn specの設定キーは実装から参照されている() {
        let spec = read_repo_file("SPEC.md").expect("SPEC.md を読めない");
        let referenced = referenced_keys();

        // SPEC の表に現れる `PICOAGENT_*` を拾う。
        let mut in_spec = std::collections::BTreeSet::new();
        super::collect_keys(&spec, &mut in_spec);

        // 旧キー（移行のために記載を残しているもの）は「旧」と同じ行で説明されている。
        let legacy: Vec<&str> = spec
            .lines()
            .filter(|line| line.contains("旧キー") || line.contains("旧座標"))
            .flat_map(|line| line.split(['`', ' ', '|', '/']))
            .filter(|token| token.starts_with("PICOAGENT_"))
            .collect();

        let orphaned: Vec<&String> = in_spec
            .iter()
            .filter(|key| !referenced.contains(*key))
            .filter(|key| !legacy.iter().any(|old| old == &key.as_str()))
            .collect();
        assert!(
            orphaned.is_empty(),
            "実装から参照されない設定キーが SPEC.md に残っています（削除・改名の反映漏れ。旧キーとして残すなら行に「旧キー」と明記してください）: {orphaned:?}"
        );
    }
}
