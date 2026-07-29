// xtask - PicoAgent の開発・ビルドタスクランナー（Node 不要）。
//
// 旧 `npm run <script>` の置き換え。同梱物の取得・UI 配信ビルド・キャラ同期・
// 残存プロセス停止・ローカル API CLI をここに集約する。
//
// 使い方: `cargo xtask <サブコマンド>`（エイリアスは リポジトリ直下の .cargo/config.toml）

mod assets;
mod bundled;
mod cli;
mod office;
mod tools;
mod util;

use std::process::Command;

use anyhow::{bail, Context, Result};

use util::{log, repo_root};

const USAGE: &str = "\
使い方: cargo xtask <サブコマンド>

  dev                 開発起動（同梱物の未取得分を取得してから tauri dev）
  build               製品ビルド（同上 + tauri build）
  setup [対象…]        同梱物の取得。対象は engine / aivis / whisper / live2d / office / tools
                      （省略時はすべて）
  sync-characters     characters/ を ui/assets/characters/ へ同期
  build-ui [--release] ui/ を ui-dist/ へ配信ビルド（--release で最小化）
  stop                残存プロセスの停止
  cli <コマンド…>      ローカル API を叩く CLI（status / models / chat / ask / repl）

例:
  cargo xtask dev
  cargo xtask setup tools
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
        "dev" => tauri("dev", rest),
        "build" => tauri("build", rest),
        "setup" => setup(rest),
        "sync-characters" => assets::sync_characters(),
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

/// 同梱物を取得する。対象を省略するとすべて。
///
/// 個々の失敗は各取得側の方針（ベストエフォートか必須か）に従う。
fn setup(targets: &[String]) -> Result<()> {
    let all = ["live2d", "engine", "aivis", "whisper", "office", "tools"];
    let wanted: Vec<&str> = if targets.is_empty() {
        all.to_vec()
    } else {
        targets.iter().map(String::as_str).collect()
    };

    for target in wanted {
        match target {
            "engine" => bundled::fetch_engine()?,
            "whisper" => bundled::fetch_whisper()?,
            "live2d" => bundled::fetch_live2d()?,
            "aivis" => office::fetch_aivis()?,
            "office" => office::fetch_office()?,
            "tools" => tools::fetch_all()?,
            other => bail!(
                "未知の取得対象です: {other}（engine / aivis / whisper / live2d / office / tools）"
            ),
        }
    }
    Ok(())
}

/// `tauri dev` / `tauri build` を前処理つきで実行する。
///
/// 前処理は旧 `npm run dev` / `npm run build` と同じ順序:
/// 残存プロセス停止 → Live2D 取得 → キャラ同期 → UI 配信ビルド → 同梱物取得。
fn tauri(mode: &str, extra: &[String]) -> Result<()> {
    let release = mode == "build";

    assets::stop_stale()?;
    bundled::fetch_live2d()?;
    assets::sync_characters()?;
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
