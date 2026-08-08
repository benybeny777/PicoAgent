// xtask - PicoAgent の開発・ビルドタスクランナー（Node 不要）。
//
// 旧 `npm run <script>` の置き換え。同梱物の取得・UI 配信ビルド・キャラ同期・
// 残存プロセス停止・ローカル API CLI をここに集約する。
//
// 使い方: `cargo xtask <サブコマンド>`（エイリアスは リポジトリ直下の .cargo/config.toml）

mod assets;
mod bundled;
mod character_pack;
mod cli;
mod office;
mod release;
mod tools;
mod util;

use std::process::Command;

use anyhow::{bail, Context, Result};

use util::{log, repo_root};

const USAGE: &str = "\
使い方: cargo xtask <サブコマンド>

  dev                 開発起動（同梱物の未取得分を取得してから tauri dev）
  build               開発用の製品ビルド（同上 + tauri build）
  build-distribution  販売条件を検査し、販売対象キャラだけで製品ビルド
  preview-distribution 販売対象キャラだけのUIを生成（法務ゲート前の確認用）
  release-check       販売前の未完了項目を一覧表示
  setup [対象…]        取得対象は engine / aivis / whisper / live2d / live2d-samples / tha3 / office / tools
                      （省略時は製品へ同梱する全対象）
  sync-characters     characters/ を ui/assets/characters/ へ同期
  character-pack      2Dキャラクターパックを検証・導入
  build-ui [--release] ui/ を ui-dist/ へ配信ビルド（--release で最小化）
  stop                残存プロセスの停止
  cli <コマンド…>      ローカル API を叩く CLI（status / models / chat / ask / repl）

例:
  cargo xtask dev
  cargo xtask setup tools
  cargo xtask setup tha3
  cargo xtask setup live2d-samples
  TOOLS_FORCE=1 cargo xtask setup tools
  cargo xtask cli chat 今日の天気は？";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run(&args) {
        eprintln!("エラー: {error:#}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];

    match command {
        "dev" => tauri("dev", rest, false),
        "build" => tauri("build", rest, false),
        "build-distribution" => tauri("build", rest, true),
        "preview-distribution" => preview_distribution(),
        "release-check" => {
            let _ = release::check(false)?;
            Ok(())
        }
        "setup" => setup(rest),
        "sync-characters" => assets::sync_characters(),
        "character-pack" => character_pack::run(rest),
        "build-ui" => assets::build_ui_dist(rest.iter().any(|a| a == "--release")),
        "stop" => assets::stop_stale(),
        "cli" => cli::run(rest),
        "" | "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => {
            println!("{USAGE}");
            bail!("未知のサブコマンドです: {other}")
        }
    }
}

fn preview_distribution() -> Result<()> {
    let characters = release::included_characters(&release::manifest()?)?;
    assets::sync_distribution_characters(&characters)?;
    assets::build_ui_dist(false)
}

/// 同梱物を取得する。対象を省略するとすべて。
///
/// 個々の失敗は各取得側の方針（ベストエフォートか必須か）に従う。
fn setup(targets: &[String]) -> Result<()> {
    // THA3も利用者が設定画面からすぐ制作できる製品同梱物なので一括取得へ含める。
    // 通常動作中は読み込まず、生成開始時だけPythonとモデルを起動する。
    let all = [
        "live2d", "engine", "aivis", "whisper", "tha3", "office", "tools",
    ];
    let wanted: Vec<&str> = if targets.is_empty() {
        all.to_vec()
    } else {
        targets.iter().map(String::as_str).collect()
    };

    for target in wanted {
        match target {
            "engine" => bundled::fetch_engine()?,
            "whisper" => bundled::fetch_whisper()?,
            "live2d" => bundled::fetch_live2d_runtime()?,
            "live2d-samples" => bundled::fetch_live2d_samples()?,
            "tha3" => bundled::fetch_tha3()?,
            "aivis" => office::fetch_aivis()?,
            "office" => office::fetch_office()?,
            "tools" => tools::fetch_all()?,
            other => bail!(
                "未知の取得対象です: {other}（engine / aivis / whisper / live2d / live2d-samples / tha3 / office / tools）"
            ),
        }
    }
    Ok(())
}

/// `tauri dev` / `tauri build` を前処理つきで実行する。
///
/// 前処理は旧 `npm run dev` / `npm run build` と同じ順序:
/// 残存プロセス停止 → Live2Dランタイム取得 → キャラ同期 → UI 配信ビルド → 同梱物取得。
fn tauri(mode: &str, extra: &[String], distribution: bool) -> Result<()> {
    let release = mode == "build";
    if distribution {
        let _ = release::check(true)?;
    }
    // 製品ビルドは通常版も販売版も配布許可リストだけを使う。開発サンプルを含む
    // 全キャラ同期は `tauri dev` に限定し、手元に取得済みのモデルが偶然混ざるのを防ぐ。
    let product_characters = product_characters_for_build(release)?;

    assets::stop_stale()?;
    bundled::fetch_live2d_runtime()?;
    if let Some(characters) = product_characters.as_ref() {
        assets::sync_distribution_characters(characters)?;
    } else {
        assets::sync_characters()?;
    }
    assets::build_ui_dist(release)?;
    bundled::fetch_engine()?;
    office::fetch_aivis()?;
    // 文字起こしはベストエフォート（公式バイナリ配布が無い OS がある）。
    if let Err(error) = bundled::fetch_whisper() {
        util::warn("setup:whisper", error.to_string());
        util::warn(
            "setup:whisper",
            "文字起こしなしでの起動を続行します。必須にする場合は WHISPER_REQUIRED=1 を指定してください。",
        );
        if util::flag("WHISPER_REQUIRED") {
            bail!("同梱 Whisper を取得できませんでした");
        }
    }
    bundled::fetch_tha3()?;
    office::fetch_office()?;
    tools::fetch_all()?;

    log("xtask", format!("cargo tauri {mode} を開始します"));
    let status = Command::new("cargo")
        .arg("tauri")
        .arg(mode)
        .args(extra)
        .current_dir(repo_root())
        .status()
        .context(
            "cargo tauri を起動できません。未インストールなら `cargo install tauri-cli --version '^2'` を実行してください。",
        )?;
    if !status.success() {
        bail!(
            "cargo tauri {mode} が失敗しました（終了コード {:?}）",
            status.code()
        );
    }
    Ok(())
}

fn product_characters_for_build(release_build: bool) -> Result<Option<Vec<String>>> {
    if release_build {
        Ok(Some(release::included_characters(&release::manifest()?)?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn 製品ビルドだけ配布許可キャラへ限定する() {
        assert!(super::product_characters_for_build(false)
            .unwrap()
            .is_none());
        assert_eq!(
            super::product_characters_for_build(true).unwrap().unwrap(),
            ["mugi", "rin", "mai"]
        );
    }

    #[test]
    fn 変更履歴の年別リンクが存在する() {
        let root = super::repo_root();
        let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap();
        let mut checked = 0;
        for segment in changelog.split("](").skip(1) {
            let Some(target) = segment.split(')').next() else {
                continue;
            };
            if target.starts_with("docs/changelog/") {
                checked += 1;
                assert!(
                    root.join(target).is_file(),
                    "変更履歴リンクが切れています: {target}"
                );
            }
        }
        assert!(checked > 0, "年別変更履歴へのリンクがありません");
    }
}
