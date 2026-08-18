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
const ALLOWED_RESOURCES: [(&str, &str); 2] = [
    (
        "resources/bundled/**/*",
        "通常ビルド用の同梱エンジン・追加機能実行ファイル",
    ),
    (
        "resources/component-manifest.json",
        "軽量配布版が第三者追加機能の公式取得元と導入方式を表示・検証するためのマニフェスト",
    ),
];

/// 販売版（`tauri.distribution.conf.json`）の `bundle.resources` に書いてよいもの。
///
/// 販売版は**第三者ソフトを再配布しない**方針なので、通常ビルドより狭い。ここを広げると
/// LibreOffice・AivisSpeech・Whisper・THA3 のような取得物がそのまま販売物へ入る。
/// 通常ビルド側（`ALLOWED_RESOURCES`）は `resources/bundled/**/*` を許しているため、
/// 販売用の上書き設定を見ていないと、この違いが誰にも検出されないまま消える。
const ALLOWED_DISTRIBUTION_RESOURCES: [(&str, &str); 2] = [
    (
        "resources/bundled/engine/**/*",
        "販売版に入れるのはローカルAIエンジンだけ（自前配布が許される同梱物）",
    ),
    (
        "resources/component-manifest.json",
        "第三者追加機能の公式取得元と導入方式を表示・検証するためのマニフェスト",
    ),
];

/// `ui/` に置いてよい拡張子。ここは製品へ丸ごと入るため、webview が実行時に使うものだけ。
const ALLOWED_UI_EXTENSIONS: [&str; 12] = [
    "html", "js", "css", "json", "png", "jpg", "jpeg", "svg", "webp", "gif", "ico", "woff2",
];

/// `ui/` の中で走査しない Git 管理外の生成物・取得物。
const UI_GENERATED_DIRS: [(&str, &str); 2] = [
    (
        "assets/characters",
        "cargo xtask sync-characters が characters/ から生成する配信コピー",
    ),
    (
        "vendor/live2d",
        "cargo xtask setup live2d が取得する Live2D ランタイム（再配布条件があるため Git 管理外）",
    ),
];

/// 開発用であり、配布経路のどこにも現れてはならないトップレベル。
const DEV_ONLY_DIRS: [&str; 4] = ["tools", "docs", "scripts", "xtask"];

mod guard {
    use super::{
        ALLOWED_DISTRIBUTION_RESOURCES, ALLOWED_RESOURCES, ALLOWED_UI_EXTENSIONS, DEV_ONLY_DIRS,
        UI_GENERATED_DIRS,
    };
    use crate::guard_scan;
    use std::path::{Path, PathBuf};

    fn read_conf(name: &str) -> serde_json::Value {
        let path = guard_scan::repo_root().join("src-tauri").join(name);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name} を読めない"));
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} が JSON として読めない"))
    }

    fn tauri_conf() -> serde_json::Value {
        read_conf("tauri.conf.json")
    }

    fn distribution_conf() -> serde_json::Value {
        read_conf("tauri.distribution.conf.json")
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

    /// 販売版の上書き設定（`--config tauri.distribution.conf.json`）だけを見て決まる範囲。
    ///
    /// 通常ビルドは `resources/bundled/**/*` を丸ごと同梱してよいが、販売版は
    /// 第三者ソフトを再配布しないためエンジンだけに絞っている。実際に利用者へ届くのは
    /// こちらなので、通常設定と同じ強さで固定する。
    #[test]
    fn 販売版へ同梱するリソースは許可したものだけ() {
        let conf = distribution_conf();
        let resources = conf["bundle"]["resources"]
            .as_array()
            .expect("販売版の bundle.resources が配列でない");
        let violations: Vec<String> = resources
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_string())
            .filter(|pattern| {
                !ALLOWED_DISTRIBUTION_RESOURCES
                    .iter()
                    .any(|(allowed, _)| allowed == pattern)
            })
            .collect();
        assert!(
            violations.is_empty(),
            "販売版へ同梱するリソースが増えています。第三者ソフトを自前で再配布しない方針です。\
             本当に必要なら bundle_scope::ALLOWED_DISTRIBUTION_RESOURCES へ理由付きで追加して\
             ください: {violations:?}"
        );
        let missing: Vec<&str> = ALLOWED_DISTRIBUTION_RESOURCES
            .iter()
            .map(|(allowed, _)| *allowed)
            .filter(|allowed| {
                !resources
                    .iter()
                    .any(|value| value.as_str() == Some(*allowed))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "販売版に必要なリソース指定が欠けています: {missing:?}"
        );

        // 販売版が通常設定より広くなっていないか（上書き設定なので、書き忘れると
        // 通常設定の `resources/bundled/**/*` がそのまま効いてしまう）。
        assert!(
            resources.len() <= ALLOWED_RESOURCES.len(),
            "販売版のリソース指定が通常ビルドより多くなっています"
        );
    }

    #[test]
    fn 開発用フォルダは配布経路に現れない() {
        let mut violations = Vec::new();
        for (label, conf) in [
            ("tauri.conf.json", tauri_conf()),
            ("tauri.distribution.conf.json", distribution_conf()),
        ] {
            let serialized = conf.to_string();
            for dir in DEV_ONLY_DIRS {
                // アイコンやリソースの指定に開発用フォルダが紛れ込んでいないか。
                for prefix in ["\"", "/", "../"] {
                    if serialized.contains(&format!("{prefix}{dir}/")) {
                        violations.push(format!("{label} が {dir}/ を参照しています"));
                    }
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

    /// 同梱PIXI 6.5.10はWebGL同期関数を`new Function`で生成するため、CSPの
    /// `unsafe-eval`を現時点では外せない。必要性を第三者ランタイムだけへ閉じ込め、
    /// PicoAgent自身のコードが同じ緩和へ依存し始めるのを防ぐ。
    #[test]
    fn 自前フロントは動的コード実行へ依存しない() {
        let ui = guard_scan::repo_root().join("ui");
        let mut violations = Vec::new();
        for relative in ui_files() {
            if relative
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("js")
            {
                continue;
            }
            let path = ui.join(&relative);
            let source = std::fs::read_to_string(&path).expect("UIのJavaScriptを読めない");
            let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
            let has_call = |needle: &str| {
                normalized.match_indices(needle).any(|(index, _)| {
                    normalized[..index]
                        .chars()
                        .next_back()
                        .is_none_or(|before| {
                            !before.is_ascii_alphanumeric() && before != '_' && before != '$'
                        })
                })
            };
            if has_call("eval(")
                || has_call("eval (")
                || has_call("Function(")
                || has_call("Function (")
            {
                violations.push(relative.display().to_string());
            }
        }
        assert!(
            violations.is_empty(),
            "PicoAgent自身のJavaScriptでeval／Functionコンストラクタを使わないでください。\
             CSPのunsafe-evalはGit管理外のLive2D用PIXIだけに限定します: {violations:?}"
        );
    }

    #[test]
    fn 現行live2d描画に必要なcspだけを維持する() {
        let conf = tauri_conf();
        let csp = conf["app"]["security"]["csp"]
            .as_str()
            .expect("app.security.csp が文字列でない");
        assert!(
            csp.contains("script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'"),
            "PIXI 6.5.10の動的WebGL同期とCubism CoreのWASMに必要な許可です。\
             外す場合はLive2D実描画とSPECの理由を同じ変更で更新してください"
        );
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "インラインJavaScriptは許可しないでください"
        );
    }

    #[test]
    fn フロントのinvoke失敗は診断ログへ残す() {
        let path = guard_scan::repo_root().join("ui/picoagent.js");
        let source = std::fs::read_to_string(path).expect("ui/picoagent.js");
        assert!(
            source.contains("async function call(name, args)")
                && source.contains("logInvokeError(name, error)"),
            "共通invoke入口は失敗をapp.logへ記録してから再スローしてください"
        );
        assert_eq!(
            source.matches("invoke(").count(),
            3,
            "invokeを直接追加せず、失敗ログを保証するcall()を使ってください"
        );
    }

    /// 人格プロンプトの混入は Rust の `remove_prompt_leak` が最終回答から消すが、
    /// ストリーム読み上げは最終回答が整う前の途中テキストを声にする。フロント側の
    /// 除去を外すと、表示には出ないのに設定文だけを喋る状態へ戻る。
    #[test]
    fn 読み上げ経路は人格プロンプトの混入を落とす() {
        let path = guard_scan::repo_root().join("ui/renderer.js");
        let source = std::fs::read_to_string(path).expect("ui/renderer.js");
        assert!(
            source.contains("function stripPersonaLeak(text)")
                && source.contains("setPersonaSpeechBlocklist(profile)"),
            "人格プロンプトの除去と、選択中キャラへの照合表の更新を残してください"
        );
        // 挨拶・自発雑談・目覚ましは sanitizeSpeechText を通さず prepareSpeech へ直接来る。
        // 全経路が必ず通るのは prepareSpeech だけなので、除去はそこにあることを確かめる。
        let prepare = source
            .split_once("async function prepareSpeech(text)")
            .expect("prepareSpeech がありません")
            .1;
        let body = prepare.split_once("\n}").expect("関数の終端がありません").0;
        assert!(
            body.contains("stripPersonaLeak("),
            "音声合成の唯一の入口 prepareSpeech で人格プロンプトの混入を落としてください\
             （挨拶・自発雑談・目覚ましは sanitizeSpeechText を通らない）"
        );
    }

    #[test]
    fn フロントのpromise失敗を空のcatchで捨てない() {
        let root = guard_scan::repo_root();
        let mut offenders = Vec::new();
        for name in ["renderer.js", "settings.js", "common.js", "picoagent.js"] {
            let path = root.join("ui").join(name);
            let source = std::fs::read_to_string(&path).expect("UIソースを読めない");
            if source.contains(".catch(() => {})") {
                offenders.push(path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "Promise失敗は診断ログへ残してください: {offenders:?}"
        );
    }
}
