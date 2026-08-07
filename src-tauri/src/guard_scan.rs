// guard_scan.rs - 設計ガードが走査するソース一式を1箇所で決める（テスト専用）。
//
// 各ガードがそれぞれ独自にディレクトリを歩いていたため、走査範囲がバラバラだった。
// 実際に `tools::process::guard` は `skills/` しか見ておらず、同じ規則を破っていた
// バックエンド（`scheduler.rs` / `engine.rs` / `aivis.rs` / `mcp/client.rs` /
// `commands.rs`）を1つも検出できていなかった。**ガードの穴はガードが無いのと同じ**なので、
// 「どこを見るか」はここだけで決め、すべてのガードがこれを使う。
//
// 新しいガードを書くときも自前でディレクトリを歩かないこと。歩いていないかは
// `guard_scan::guard` のメタガードが検出する。

use std::path::{Path, PathBuf};

/// 走査対象のソース1件。
pub struct Source {
    /// 表示用の短い名前（スキル名、またはバックエンドの相対パス）。
    pub name: String,
    /// リポジトリルートからの相対パス。
    pub path: PathBuf,
    pub text: String,
}

impl Source {
    /// スキル（`skills/<名前>/mod.rs`）か。
    pub fn is_skill(&self) -> bool {
        self.path.starts_with("skills")
    }

    /// テストコード（`#[cfg(test)]` 以降）を落とした本体。
    ///
    /// テスト内のバイト列スライスやダミーの外部コマンドまで違反として拾わないため。
    pub fn body(&self) -> &str {
        match self.text.find("#[cfg(test)]") {
            Some(pos) => &self.text[..pos],
            None => &self.text,
        }
    }
}

/// リポジトリルート。ガードがリポジトリ内のファイルを見るときはこれを使うこと。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri の親")
        .to_path_buf()
}

/// スキル、バックエンド、再利用コアのソース全部。ガードはこれを使うこと。
pub fn all_sources() -> Vec<Source> {
    let mut out = skill_sources();
    out.extend(backend_sources());
    out.extend(core_sources());
    out
}

/// `crates/picoagent-core/src/**/*.rs` 一式。
/// コアへ切り出したコードも、アプリ内コードと同じ設計ガードから外さない。
pub fn core_sources() -> Vec<Source> {
    let root = repo_root();
    let mut out = Vec::new();
    collect_rs(&root, &root.join("crates/picoagent-core/src"), &mut out);
    assert!(
        out.len() >= 3,
        "再利用コアの読み込みに失敗している可能性: {}",
        out.len()
    );
    out
}

/// `skills/<名前>/**.rs` 一式（`mod.rs` とスキル専用の補助モジュール）。
///
/// 補助モジュール（`run_command/jobs.rs`・`chitchat/trends.rs` など）も同じ規則で
/// 書かれるので、`mod.rs` だけを見ると外部プロセス起動や `fs::write` の違反を
/// そこへ書いた瞬間に見逃す。`name` はどちらもスキル名（ディレクトリ名）にして、
/// スキル単位で例外を指定するガード（`OWN_CHECK_ALLOWED` など）がそのまま効くようにする。
pub fn skill_sources() -> Vec<Source> {
    let root = repo_root();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("skills"))
        .expect("skills ディレクトリ")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = files
            .flatten()
            .map(|file| file.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
            .collect();
        // mod.rs を先頭にして、既存の表示順（スキル本体が先）を保つ。
        paths.sort_by_key(|path| {
            (
                path.file_name().and_then(|file| file.to_str()) != Some("mod.rs"),
                path.clone(),
            )
        });
        for path in paths {
            if let Ok(text) = std::fs::read_to_string(&path) {
                out.push(Source {
                    name: name.clone(),
                    path: path.strip_prefix(&root).unwrap_or(&path).to_path_buf(),
                    text,
                });
            }
        }
    }
    assert!(
        out.len() > 40,
        "スキルの読み込みに失敗している可能性: {}",
        out.len()
    );
    out
}

/// `src-tauri/src/**/*.rs` 一式。
pub fn backend_sources() -> Vec<Source> {
    let root = repo_root();
    let mut out = Vec::new();
    collect_rs(&root, &root.join("src-tauri/src"), &mut out);
    assert!(
        out.len() > 20,
        "バックエンドの読み込みに失敗している可能性: {}",
        out.len()
    );
    out
}

fn collect_rs(root: &Path, dir: &Path, out: &mut Vec<Source>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push(Source {
                name: path
                    .file_name()
                    .map(|v| v.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                text,
            });
        }
    }
}

/// メタガード: 各ガードが走査範囲を自前で決めていないこと。
mod guard {
    #[test]
    fn 再利用コアへ製品依存を持ち込まない() {
        const FORBIDDEN: &[&str] = &["tauri", "PICOAGENT_", "crate::config"];
        let mut violations = Vec::new();
        for source in super::core_sources() {
            for dependency in FORBIDDEN {
                if source.body().contains(dependency) {
                    violations.push(format!("{}: {dependency}", source.path.display()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "再利用コアへPicoAgent製品固有の依存を持ち込まないでください: {violations:?}"
        );
    }

    #[test]
    fn ガードは走査範囲を自前で決めない() {
        let mut violations = Vec::new();
        for source in super::all_sources() {
            if source.name == "guard_scan.rs" {
                continue; // 走査範囲の定義元
            }
            // `mod guard` を持つファイル＝設計ガード。共通の走査を使っているか。
            if !source.text.contains("mod guard {") {
                continue;
            }
            if !source.text.contains("guard_scan::") {
                violations.push(source.path.display().to_string());
            }
        }
        assert!(
            violations.is_empty(),
            "設計ガードは crate::guard_scan の走査を使ってください\
             （自前でディレクトリを歩くと走査範囲がずれ、実際に規則違反を見逃していました）: {violations:?}"
        );
    }
}
