// build.rs - Tauri ビルド準備 ＋ コネクタの自動 mod 生成。
//
// src/providers/*.rs と ルートの skills/*.rs を走査し、各ファイルの
// `pub mod <名前>;` 宣言を OUT_DIR へ生成する。providers/mod.rs・tools/mod.rs が
// これを include! することで、「.rs を1枚置くだけ」でコネクタが追加できる
// （中央レジストリの編集不要）思想を Rust でも維持する。スキル（ツール）の連結
// ファイルは扱いやすいようプロジェクト直下の skills/ に置く（トレイトは tools/mod.rs）。

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

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
            let source_path = Path::new(&manifest_dir)
                .join(dir)
                .join(format!("{}.rs", n));
            let rel = relative_path(Path::new(&out_dir), &source_path)
                .to_string_lossy()
                .replace('\\', "/");
            decls.push_str(&format!("#[path = \"{}\"]\npub mod {};\n", rel, n));
        }
    }
    fs::write(Path::new(&out_dir).join(out_name), decls).unwrap();
    println!("cargo:rerun-if-changed={}", dir);
}

fn main() {
    generate_mods("src/providers", "provider_mods.rs");
    generate_mods("../skills", "tool_mods.rs");
    tauri_build::build();
}
