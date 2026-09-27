// ui_contract.rs - UI契約（docs/UI-CONTRACT.md）と登録済み command のズレを検出する設計ガード（テスト専用）。
//
// 画面を作り直す人は、旧フロントのコードではなく UI契約の文書だけを読んで作る。文書が実装と
// ずれていると、その前提が崩れて「呼べるはずの command が無い」「知らない command が残る」になる。

#[cfg(test)]
mod guard {
    use std::collections::BTreeSet;

    fn read_repo_file(relative: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("リポジトリ直下を解決できない");
        std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("{relative} を読めない: {error}"))
    }

    /// `invoke_handler` に登録された command 名。
    fn registered_commands() -> BTreeSet<String> {
        let lib = read_repo_file("src-tauri/src/lib.rs");
        let list = lib
            .split("generate_handler![")
            .nth(1)
            .and_then(|rest| rest.split("])").next())
            .expect("generate_handler! が見つからない");
        list.split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.rsplit("::").next().unwrap_or(item).to_string())
            .collect()
    }

    /// UI契約の「command」節の表に載っている command 名。
    fn documented_commands() -> BTreeSet<String> {
        let contract = read_repo_file("docs/UI-CONTRACT.md");
        let section = contract
            .split("## command")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .expect("UI契約に command の節が無い");
        section
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split('`').next())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn 登録済みcommandとui契約は双方向に一致する() {
        let registered = registered_commands();
        let documented = documented_commands();
        assert!(
            registered.len() > 50,
            "command の収集に失敗しています: {}",
            registered.len()
        );
        let undocumented: Vec<_> = registered.difference(&documented).collect();
        let unregistered: Vec<_> = documented.difference(&registered).collect();
        assert!(
            undocumented.is_empty(),
            "docs/UI-CONTRACT.md に載っていない command があります（追加したら表へ書く）: {undocumented:?}"
        );
        assert!(
            unregistered.is_empty(),
            "docs/UI-CONTRACT.md に登録されていない command が残っています（削除・改名を表へ反映する）: {unregistered:?}"
        );
    }
}
