// bundle_scope.rs - 配布物に開発用ファイルが混ざらないことを固定する設計ガード（テスト専用）。
//
// リリースビルドに入るのは (1) `ui/` を写した `ui-dist/`、(2) `tauri.conf.json` の
// `bundle.resources` が指すもの、の2経路だけ。開発用のツール・ドキュメント・撮影用の
// 画像（`tools/` `docs/` `scripts/` `xtask/`）は入らない。
//
// ただし `ui/` は**中身を丸ごと**配信ビルドへコピーするため、ここへ作業用のファイルを
// 置くと黙って製品に同梱される。設定を1行足すだけでも同じことが起きる。目視では気づけ
// ないので、許可した範囲から外れたらテストで落とす。
//
// 範囲を広げるとき（本当に同梱したいものが増えたとき）は、下の allowlist へ理由付きで
// 追加すること。

/// `bundle.resources` に書いてよいもの。追加するときは、なぜ製品へ同梱するのかを書き添える。
const ALLOWED_RESOURCES: [(&str, &str); 1] = [(
    "resources/bundled/**/*",
    "同梱エンジン・読み上げ・Whisper・OfficeCLI・OSS ツールの実行ファイル",
)];

/// `ui/` に置いてよい拡張子。ここは製品へ丸ごと入るため、webview が実行時に使うものだけ。
const ALLOWED_UI_EXTENSIONS: [&str; 12] = [
    "html", "js", "css", "json", "png", "jpg", "jpeg", "svg", "webp", "gif", "ico", "woff2",
];

/// `ui/` の中で走査しないディレクトリ（いずれも Git 管理外の生成物・取得物）。
const UI_GENERATED_DIRS: [(&str, &str); 2] = [
    (
        "assets/characters",
        "cargo xtask sync-characters が characters/ から生成する配信コピー",
    ),
    (
        "vendor",
        "cargo xtask setup live2d が取得する Live2D ランタイム（再配布条件があるため Git 管理外）",
    ),
];

/// 開発用であり、配布経路のどこにも現れてはならないトップレベル。
const DEV_ONLY_DIRS: [&str; 4] = ["tools", "docs", "scripts", "xtask"];

mod guard {
    use super::{ALLOWED_RESOURCES, ALLOWED_UI_EXTENSIONS, DEV_ONLY_DIRS, UI_GENERATED_DIRS};
    use crate::guard_scan;
    use std::path::{Path, PathBuf};

    fn tauri_conf() -> serde_json::Value {
        let path = guard_scan::repo_root().join("src-tauri/tauri.conf.json");
        let text = std::fs::read_to_string(&path).expect("tauri.conf.json");
        serde_json::from_str(&text).expect("tauri.conf.json が JSON として読めない")
    }

    /// `ui/` 配下の実ファイル（生成ディレクトリを除く）を相対パスで集める。
    fn ui_files() -> Vec<PathBuf> {
        let ui = guard_scan::repo_root().join("ui");
        let mut out = Vec::new();
        walk(&ui, &ui, &mut out);
        assert!(
            out.len() >= 5,
            "ui/ の読み込みに失敗している可能性: {}",
            out.len()
        );
        out
    }

    fn walk(ui_root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(ui_root).unwrap_or(&path).to_path_buf();
            if path.is_dir() {
                let is_generated = UI_GENERATED_DIRS
                    .iter()
                    .any(|(name, _)| relative == Path::new(name));
                if !is_generated {
                    walk(ui_root, &path, out);
                }
            } else {
                out.push(relative);
            }
        }
    }

    #[test]
    fn 製品へ同梱するリソースは許可したものだけ() {
        let conf = tauri_conf();
        let resources = conf["bundle"]["resources"]
            .as_array()
            .expect("bundle.resources が配列でない");
        let mut violations = Vec::new();
        for value in resources {
            let pattern = value.as_str().unwrap_or_default();
            if !ALLOWED_RESOURCES
                .iter()
                .any(|(allowed, _)| *allowed == pattern)
            {
                violations.push(pattern.to_string());
            }
        }
        assert!(
            violations.is_empty(),
            "製品へ同梱するリソースが増えています。開発用のツール・ドキュメント・画像を\
             配布物へ入れないでください。本当に必要なら bundle_scope::ALLOWED_RESOURCES へ\
             理由付きで追加してください: {violations:?}"
        );

        // 配信元がすり替わると ui/ 以外が製品へ入りうるので、ここも固定する。
        assert_eq!(
            conf["build"]["frontendDist"].as_str(),
            Some("../ui-dist"),
            "配信元は cargo xtask build-ui が ui/ から作る ui-dist/ だけにしてください"
        );
    }

    #[test]
    fn 開発用フォルダは配布経路に現れない() {
        let conf = tauri_conf();
        let serialized = conf.to_string();
        let mut violations = Vec::new();
        for dir in DEV_ONLY_DIRS {
            // アイコンやリソースの指定に開発用フォルダが紛れ込んでいないか。
            for prefix in ["\"", "/", "../"] {
                if serialized.contains(&format!("{prefix}{dir}/")) {
                    violations.push(format!("tauri.conf.json が {dir}/ を参照しています"));
                }
            }
        }
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn ui配下に開発用ファイルを置かない() {
        let mut violations = Vec::new();
        for relative in ui_files() {
            let extension = relative
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !ALLOWED_UI_EXTENSIONS.contains(&extension.as_str()) {
                violations.push(relative.display().to_string());
            }
        }
        assert!(
            violations.is_empty(),
            "ui/ は中身がそのまま製品へ入ります。作業用のスクリプト・設定・ドキュメントは\
             tools/ など ui/ の外へ置いてください（本当に実行時に必要なら\
             bundle_scope::ALLOWED_UI_EXTENSIONS へ追加）: {violations:?}"
        );
    }
}
