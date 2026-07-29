// build.rs - Tauri ビルド準備 ＋ コネクタの自動 mod 生成。
//
// src/providers/*.rs と ルートの skills/<スキル名>/mod.rs を走査し、各ファイルの
// `pub mod <名前>;` 宣言を OUT_DIR へ生成する。providers/mod.rs・tools/mod.rs が
// これを include! することで、「決まった場所にファイルを置くだけ」でコネクタが追加できる
// （中央レジストリの編集不要）思想を Rust でも維持する。スキル（ツール）は
// プロジェクト直下の skills/<スキル名>/mod.rs に置く（トレイトは tools/mod.rs）。

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

/// Rust の識別子として使える名前か（コネクタ名はそのまま `mod` 名になる）。
fn is_valid_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn relative_path(from_dir: &Path, to_file: &Path) -> PathBuf {
    let from_components: Vec<Component<'_>> = from_dir.components().collect();
    let to_components: Vec<Component<'_>> = to_file.components().collect();
    let mut common_len = 0;

    while common_len < from_components.len()
        && common_len < to_components.len()
        && from_components[common_len] == to_components[common_len]
    {
        common_len += 1;
    }

    if common_len == 0 {
        return to_file.to_path_buf();
    }

    let mut rel = PathBuf::new();
    for _ in common_len..from_components.len() {
        rel.push("..");
    }
    for component in &to_components[common_len..] {
        rel.push(component.as_os_str());
    }
    rel
}

fn generate_mods(dir: &str, out_name: &str) {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut decls = String::new();
    if let Ok(entries) = fs::read_dir(dir) {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let stem = p.file_stem()?.to_str()?.to_string();
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "rs" && stem != "mod" {
                    Some(stem)
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        for n in &names {
            // include! 経由だと mod 解決位置が OUT_DIR 基準になるため、生成先から
            // 実ファイルへの相対パスを #[path] で明示する。
            let source_path = Path::new(&manifest_dir).join(dir).join(format!("{}.rs", n));
            let rel = relative_path(Path::new(&out_dir), &source_path)
                .to_string_lossy()
                .replace('\\', "/");
            decls.push_str(&format!("#[path = \"{}\"]\npub mod {};\n", rel, n));
        }
    }
    fs::write(Path::new(&out_dir).join(out_name), decls).unwrap();
    println!("cargo:rerun-if-changed={}", dir);
}

fn generate_skill_mods(dir: &str, out_name: &str) {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut decls = String::new();
    if let Ok(entries) = fs::read_dir(dir) {
        let mut modules: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if !p.is_dir() {
                    return None;
                }
                let name = p.file_name()?.to_str()?.to_string();
                let mod_path = p.join("mod.rs");
                if mod_path.is_file() {
                    Some((name, mod_path))
                } else {
                    None
                }
            })
            .collect();
        modules.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, path) in &modules {
            // フォルダ名はそのまま `pub mod <名前>;` になる。ハイフン等が入っていると
            // 生成コードが構文エラーになり、原因の分からないビルド失敗になるため、
            // ここで何が悪いのかを明示して止める。
            assert!(
                is_valid_module_name(name),
                "スキルのフォルダ名 \"{name}\" は Rust のモジュール名にできません。\
                 英小文字・数字・アンダースコアだけを使ってください（例: my_skill）。"
            );
            let source_path = Path::new(&manifest_dir).join(path);
            let rel = relative_path(Path::new(&out_dir), &source_path)
                .to_string_lossy()
                .replace('\\', "/");
            decls.push_str(&format!("#[path = \"{}\"]\npub mod {};\n", rel, name));
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    fs::write(Path::new(&out_dir).join(out_name), decls).unwrap();
    println!("cargo:rerun-if-changed={}", dir);
}

fn rerun_if_changed_recursive(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            rerun_if_changed_recursive(&path);
        }
    }
}

fn main() {
    generate_mods("src/providers", "provider_mods.rs");
    generate_skill_mods("../skills", "tool_mods.rs");
    // スキル固有の補助モジュール・スクリプト・テンプレートも各スキル配下へ置くため、
    // mod.rs 以外の追加や変更でもビルドが追従するよう再帰的に監視する。
    rerun_if_changed_recursive(Path::new("../skills"));
    tauri_build::build();
}
