// doc_sync.rs - ドキュメントと実装のズレを検出する設計ガード（テスト専用）。
//
// 「設定キーを足したら docs/SETTINGS.md へ書く」という運用は AGENTS.md のルールだが、人が
// 覚えているだけでは抜ける（実際に過去のレビューで `PICOAGENT_NAP_MINUTES` など
// 3件の記載漏れが見つかり、そのたびに手作業で突き合わせていた）。突き合わせ自体を
// テストにしておけば、書き忘れたその場で落ちる。

/// 設定キーとして扱わない文字列（マーカー・テスト用ダミー・ドキュメント中の例）。
/// 追加するときは、なぜ設定キーでないのかを理由として書き添えること。
const NOT_CONFIG_KEYS: [(&str, &str); 14] = [
    (
        "PICOAGENT_COMMAND_APPROVAL__",
        "run_command と対話ループの内部プロトコルのマーカー",
    ),
    (
        "PICOAGENT_USER_QUESTION__",
        "ask_user と対話ループの内部プロトコルのマーカー",
    ),
    (
        "PICOAGENT_VIEW_IMAGE__",
        "view_image とエージェントループの内部プロトコルのマーカー",
    ),
    (
        "PICOAGENT_APP_QUIT_REQUESTED",
        "app_control と終了処理の内部プロトコルのマーカー",
    ),
    ("PICOAGENT_TEST_KEY", "config のテストで使うダミー"),
    (
        "PICOAGENT_TEST_SECRET",
        "config の復号失敗テストで使うダミー",
    ),
    ("PICOAGENT_TEST_LIST", "store の並列更新テストで使うダミー"),
    (
        "PICOAGENT_TEST_KEY_",
        "config の同時保存テストで使うダミー（連番）",
    ),
    ("PICOAGENT_OTHER", "config のテストで使うダミー"),
    (
        "PICOAGENT_XXX",
        "ドキュメント・コメント中の説明用プレースホルダ",
    ),
    (
        "PICOAGENT_OPENAI_API_KEY",
        "secret のテストで使う架空キー（実キーは OPENAI_API_KEY）",
    ),
    (
        "PICOAGENT_QA_LONG_AUDIO",
        "長時間文字起こしの明示実機テストだけで使う入力パス",
    ),
    (
        "PICOAGENT_STOP_MANAGED_PROCESSES",
        "NSISインストーラの共通マクロ名（設定キーではない）",
    ),
    (
        "PICOAGENT_STOP_BUNDLED_PROCESS",
        "NSISインストーラの共通マクロ名（設定キーではない）",
    ),
];

/// ソース中に現れる `PICOAGENT_*` を集める。
fn referenced_keys() -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    for source in crate::guard_scan::all_sources() {
        collect_keys(&source.text, &mut keys);
    }
    // フロント（JS）とタスクランナー（xtask）も設定キーを参照する。
    for relative in [
        "ui/app/main.js",
        "ui/app/ambient.js",
        "ui/character/image.js",
        "ui/quick-actions.js",
        "ui/settings.js",
        "ui/settings-schedules.js",
        "ui/picoagent.js",
        "xtask/src/tools.rs",
        "xtask/src/cli.rs",
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

    #[test]
    fn 実行レイヤーはmanifestのwebpと追加パックのpngを両立する() {
        let manifest = read_repo_file("ui/character/manifest.js").expect("manifest.js を読めない");
        let image = read_repo_file("ui/character/image.js").expect("image.js を読めない");
        assert!(manifest.contains("runtimeLayers"));
        assert!(image.contains("this.pack.runtimeLayers || {}"));
        assert!(image.contains("layers/${name}.png"));
    }

    /// 実装が参照する設定キーは、すべて `docs/SETTINGS.md` に載っていること。
    #[test]
    fn 設定キーは設定リファレンスに記載する() {
        let settings = read_repo_file("docs/SETTINGS.md").expect("docs/SETTINGS.md を読めない");
        let keys = referenced_keys();
        assert!(
            keys.len() > 150,
            "設定キーの収集に失敗しています: {}",
            keys.len()
        );

        let missing: Vec<&String> = keys.iter().filter(|key| !settings.contains(*key)).collect();
        assert!(
            missing.is_empty(),
            "docs/SETTINGS.md に記載のない設定キーがあります（実装だけ増やしてドキュメントを更新していません）: {missing:?}"
        );
    }

    /// `docs/SETTINGS.md` に載っているのに実装のどこからも参照されないキーが無いこと（削除漏れ）。
    #[test]
    fn 設定リファレンスのキーは実装から参照されている() {
        let settings = read_repo_file("docs/SETTINGS.md").expect("docs/SETTINGS.md を読めない");
        let referenced = referenced_keys();

        // 設定リファレンスの表に現れる `PICOAGENT_*` を拾う。
        let mut documented = std::collections::BTreeSet::new();
        super::collect_keys(&settings, &mut documented);
        // 設定キーでない `PICOAGENT_*`（内部マーカー、インストーラのマクロ名など）は
        // 実装側の収集と同じ基準で外す。片側だけ外すと、`NOT_CONFIG_KEYS` へ登録した
        // 文字列を設定リファレンスで説明できなくなる。
        for (skip, _) in super::NOT_CONFIG_KEYS {
            documented.remove(skip);
        }

        // 旧キー（移行のために記載を残しているもの）は「旧」と同じ行で説明されている。
        let legacy: Vec<&str> = settings
            .lines()
            .filter(|line| line.contains("旧キー") || line.contains("旧座標"))
            .flat_map(|line| line.split(['`', ' ', '|', '/']))
            .filter(|token| token.starts_with("PICOAGENT_"))
            .collect();

        let orphaned: Vec<&String> = documented
            .iter()
            .filter(|key| !referenced.contains(*key))
            .filter(|key| !legacy.iter().any(|old| old == &key.as_str()))
            .collect();
        assert!(
            orphaned.is_empty(),
            "実装から参照されない設定キーが docs/SETTINGS.md に残っています（削除・改名の反映漏れ。旧キーとして残すなら行に「旧キー」と明記してください）: {orphaned:?}"
        );
    }

    /// `SPEC.md` のスキル分類表と `skills/` が双方向で一致すること。
    ///
    /// 分類表は「そのスキルが読むだけか、送信・削除まで行うか」を示す設計上の一覧なので、
    /// スキルを足したのに載せ忘れると、副作用の分からないスキルが黙って増える。
    #[test]
    fn スキルはspecの分類表に載せる() {
        let spec = read_repo_file("SPEC.md").expect("SPEC.md を読めない");
        let section = spec
            .split("### スキルの分類")
            .nth(1)
            .and_then(|rest| rest.split("### RAG との関係").next())
            .expect("SPEC.md にスキルの分類表が見つかりません");

        // 表の「スキル」列（各行の5番目のセル）から `名前` を拾う。
        let mut listed = std::collections::BTreeSet::new();
        for line in section.lines().filter(|line| line.starts_with("| **")) {
            let Some(cell) = line.split('|').nth(4) else {
                continue;
            };
            for token in cell.split('`').skip(1).step_by(2) {
                listed.insert(token.to_string());
            }
        }

        let actual: std::collections::BTreeSet<String> = crate::guard_scan::skill_sources()
            .iter()
            .map(|source| source.name.clone())
            .collect();

        let missing: Vec<&String> = actual.difference(&listed).collect();
        assert!(
            missing.is_empty(),
            "SPEC.md のスキル分類表に載っていないスキルがあります（役割と副作用を決めて追記してください）: {missing:?}"
        );

        let orphaned: Vec<&String> = listed.difference(&actual).collect();
        assert!(
            orphaned.is_empty(),
            "存在しないスキルが SPEC.md の分類表に残っています（削除・改名の反映漏れ）: {orphaned:?}"
        );
    }

    /// 破壊的な右クリック操作は、実装と利用者向け文書の双方で同じ完全ラベルを使う。
    #[test]
    fn 会話と記憶の消去メニュー名は文書と一致する() {
        const LABELS: [&str; 3] = [
            "表示と会話をリセット",
            "会話履歴と覚えたことを消す",
            "消す（元に戻せません）",
        ];
        // 旧名を移行別名として残さない理由: 失われる長期メモリと不可逆性を隠すため。
        const FORBIDDEN: [&str; 1] = ["会話履歴を消してリセット"];
        let menu = read_repo_file("src-tauri/src/menu.rs").expect("menu.rs を読めない");
        for label in LABELS {
            assert!(
                menu.contains(label),
                "右クリックメニューにラベルがありません: {label}"
            );
            for document in ["README.md", "MANUAL.md", "SPEC.md"] {
                let text =
                    read_repo_file(document).unwrap_or_else(|| panic!("{document} を読めない"));
                assert!(
                    text.contains(label),
                    "{document} にメニューラベルがありません: {label}"
                );
            }
        }
        for old in FORBIDDEN {
            for file in ["src-tauri/src/menu.rs", "README.md", "MANUAL.md", "SPEC.md"] {
                let text = read_repo_file(file).unwrap_or_else(|| panic!("{file} を読めない"));
                assert!(
                    !text.contains(old),
                    "{file} に誤解を招く旧ラベルが残っています: {old}"
                );
            }
        }
    }

    #[test]
    fn スヌーズ再発火は多重実行と失敗を解放する() {
        let source = read_repo_file("ui/app/ambient.js").expect("ambient.js を読めない");
        let start = source
            .find("async function checkAlarms()")
            .expect("checkAlarms がない");
        let body = &source[start
            ..source
                .find("window.addEventListener(\"beforeunload\"")
                .expect("目覚まし確認の末尾がない")];
        let guard = body
            .find("if (alarmCheckInFlight) return")
            .expect("多重実行ガードがない");
        let advance = body
            .find("snooze.dueAt =")
            .expect("次回期限の先行更新がない");
        let fire = body
            .find("await fireAlarm(state.alarm, state)")
            .expect("スヌーズ発火がない");
        let release = body.find("finally").expect("失敗時の解放がない");
        assert!(guard < advance && advance < fire && fire < release);
        assert!(body.contains("alarmCheckInFlight = false"));
        assert!(source.contains("snooze === snoozeState"));
    }
}
