// build.rs - Tauri ビルド準備 ＋ コネクタの自動 mod 生成。
//
// src/providers/*.rs と ルートの skills/*.rs を走査し、各ファイルの
// `pub mod <名前>;` 宣言を OUT_DIR へ生成する。providers/mod.rs・tools/mod.rs が
// これを include! することで、「.rs を1枚置くだけ」でコネクタが追加できる
// （中央レジストリの編集不要）思想を Rust でも維持する。スキル（ツール）の連結
// ファイルは扱いやすいようプロジェクト直下の skills/ に置く（トレイトは tools/mod.rs）。

use std::{env, fs, path::Path};

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
            // include! 経由だと mod 解決位置が OUT_DIR 基準になるため、実ファイルへの
            // 絶対パスを #[path] で明示する（Windows でも forward slash で可）。
            let abs = Path::new(&manifest_dir)
                .join(dir)
                .join(format!("{}.rs", n))
                .to_string_lossy()
                .replace('\\', "/");
            decls.push_str(&format!("#[path = \"{}\"]\npub mod {};\n", abs, n));
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
