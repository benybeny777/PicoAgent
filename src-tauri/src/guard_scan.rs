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
use std::sync::OnceLock;

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
pub fn all_sources() -> Vec<&'static Source> {
    let mut out: Vec<_> = skill_sources().iter().collect();
    out.extend(backend_sources());
    out.extend(core_sources());
    out
}

/// `crates/picoagent-core/src/**/*.rs` 一式。
/// コアへ切り出したコードも、アプリ内コードと同じ設計ガードから外さない。
pub fn core_sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(load_core_sources)
}

fn load_core_sources() -> Vec<Source> {
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
pub fn skill_sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(load_skill_sources)
}

fn load_skill_sources() -> Vec<Source> {
    let root = repo_root();
    let mut out = Vec::new();
    for entry in sorted_entries(&root.join("skills")) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry
            .file_type()
            .expect("スキルの種類を取得できません")
            .is_dir()
        {
            continue;
        }
        let mut sources = Vec::new();
        collect_rs(&root, &entry.path(), &mut sources);
        // mod.rs を先頭にして、既存の表示順（スキル本体が先）を保つ。
        let primary = Path::new("skills").join(&name).join("mod.rs");
        sources.sort_by(|a, b| (a.path != primary, &a.path).cmp(&(b.path != primary, &b.path)));
        for mut source in sources {
            source.name = name.clone();
            out.push(source);
        }
    }
    assert!(
        out.len() > 40,
        "スキルの読み込みに失敗している可能性: {}",
        out.len()
    );
    out
}

/// `ui/**/*.js` 一式（webview フロントの自前ソース）。
///
/// 名前を1つずつ書き並べた一覧で検査していたため、`layout-editor.js` /
/// `top-menu.js` / `mesh-rig.js` / `character-preview.js` が誰にも見られていなかった。
/// **ガードの穴はガードが無いのと同じ**なので、フロントの走査範囲もここで決める。
/// `ui/vendor/` は取得した第三者ランタイム（自前のコードではない）なので、
/// 除外する。自前の入れ子モジュールも同じ走査で検査する。
pub fn ui_sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(load_ui_sources)
}

fn load_ui_sources() -> Vec<Source> {
    let root = repo_root();
    let dir = root.join("ui");
    let mut out = Vec::new();
    collect_matching(&root, &dir, &mut out, "js", &[dir.join("vendor")]);
    assert!(
        out.len() > 8,
        "UIソースの読み込みに失敗している可能性: {}",
        out.len()
    );
    out
}

/// `src-tauri/src/**/*.rs` 一式。
pub fn backend_sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(load_backend_sources)
}

fn load_backend_sources() -> Vec<Source> {
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
    collect_matching(root, dir, out, "rs", &[]);
}

/// ビルド・配布CLIも必要なガードから同じ正本を参照する。
pub fn xtask_sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        let root = repo_root();
        let mut out = Vec::new();
        collect_rs(&root, &root.join("xtask/src"), &mut out);
        assert!(out.len() > 5, "xtaskのソース走査が不完全です");
        out
    })
}

fn collect_matching(
    root: &Path,
    dir: &Path,
    out: &mut Vec<Source>,
    extension: &str,
    excluded: &[PathBuf],
) {
    for entry in sorted_entries(dir) {
        let path = entry.path();
        if excluded.contains(&path) {
            continue;
        }
        let kind = entry.file_type().unwrap_or_else(|error| {
            panic!(
                "検査対象の種類を取得できません: {} ({error})",
                path.display()
            )
        });
        if kind.is_dir() {
            collect_matching(root, &path, out, extension, excluded);
        } else if path.extension().is_some_and(|e| e == extension) {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("検査対象を読み込めません: {} ({error})", path.display())
            });
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

// 読めない項目を除外すると検査が成功してしまうため、対象と理由を示して失敗する。
fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries = std::fs::read_dir(dir)
        .and_then(|entries| entries.collect::<std::io::Result<Vec<_>>>())
        .unwrap_or_else(|error| panic!("検査対象を列挙できません: {} ({error})", dir.display()));
    entries.sort_by_key(|entry| entry.file_name());
    entries
}

/// メタガード: 各ガードが走査範囲を自前で決めていないこと。
#[cfg(test)]
mod guard {
    fn ungrounded_comments(text: &str) -> Vec<usize> {
        let policy: serde_json::Value =
            serde_json::from_str(include_str!("../../tools/contracts/fallback-policy.json"))
                .unwrap();
        let marker = policy["marker"].as_str().unwrap();
        let opening = policy["basisOpen"].as_str().unwrap();
        let closing = policy["basisClose"].as_str().unwrap();
        let prefixes = policy["authorityPrefixes"].as_array().unwrap();
        let has_basis = |line: &str| {
            line.split_once(opening)
                .and_then(|(_, tail)| tail.split_once(closing))
                .is_some_and(|(basis, _)| {
                    prefixes.iter().any(|prefix| {
                        let prefix = prefix.as_str().unwrap();
                        let Some(tail) = basis.strip_prefix(prefix) else {
                            return false;
                        };
                        prefix != policy["numberedAuthorityPrefix"].as_str().unwrap()
                            || tail
                                .chars()
                                .next()
                                .is_some_and(|character| character.is_ascii_digit())
                    })
                })
        };
        text.lines()
            .enumerate()
            .filter_map(|(index, line)| {
                (line
                    .split_once("//")
                    .is_some_and(|(_, comment)| comment.contains(marker) && !has_basis(comment)))
                .then_some(index + 1)
            })
            .collect()
    }

    #[test]
    fn フォールバックの許可には共通方針の根拠を必須にする() {
        let sources = super::all_sources();
        let errors: Vec<_> = sources
            .into_iter()
            .chain(super::xtask_sources())
            .chain(super::ui_sources())
            .flat_map(|source| {
                ungrounded_comments(source.body())
                    .into_iter()
                    .map(|line| format!("{}:{line}", source.path.display()))
            })
            .collect();
        assert!(errors.is_empty(), "根拠のない許可コメント: {errors:?}");
        assert_eq!(
            ungrounded_comments("// フォールバック許可: 便利なので続ける"),
            [1]
        );
        assert!(ungrounded_comments(
            "// フォールバック許可: 利用者が採用した候補（根拠: PR #467）"
        )
        .is_empty());
    }

    #[test]
    fn 共通走査は画面の入れ子と開発ツールを含み第三者資産を除く() {
        assert!(super::ui_sources()
            .iter()
            .any(|source| source.path.ends_with("ui/app/main.js")));
        assert!(super::ui_sources()
            .iter()
            .all(|source| !source.path.starts_with("ui/vendor")));
        assert!(super::xtask_sources()
            .iter()
            .any(|source| source.path.ends_with("xtask/src/cli.rs")));
    }

    #[test]
    fn 同じ実行内の走査はソース本文を共有する() {
        let first = super::all_sources();
        let second = super::all_sources();
        assert_eq!(first.len(), second.len());
        assert!(first.iter().zip(&second).all(|(a, b)| std::ptr::eq(*a, *b)));
    }

    #[test]
    fn 入れ子のソースを読み不正なutf8を見逃さない() {
        let root = super::repo_root().join("temp").join(format!(
            "guard-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("nested/deeper")).unwrap();
        std::fs::write(root.join("nested/deeper/source.rs"), "fn example() {}").unwrap();
        let mut sources = Vec::new();
        super::collect_rs(&root, &root, &mut sources);
        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources[0].path,
            std::path::Path::new("nested/deeper/source.rs")
        );
        std::fs::write(root.join("invalid.rs"), [0xff]).unwrap();
        let invalid = std::panic::catch_unwind(|| {
            super::collect_rs(&root, &root, &mut Vec::new());
        });
        std::fs::remove_dir_all(&root).unwrap();
        assert!(invalid.is_err(), "読み取れないソースを除外して成功させない");
        assert!(std::panic::catch_unwind(|| super::sorted_entries(&root)).is_err());
    }

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
