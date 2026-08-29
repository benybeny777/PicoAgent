// xtask - PicoAgent の開発・ビルドタスクランナー（Node 不要）。
//
// 旧 `npm run <script>` の置き換え。同梱物の取得・UI 配信ビルド・キャラ同期・
// 残存プロセス停止・ローカル API CLI をここに集約する。
//
// 使い方: `cargo xtask <サブコマンド>`（エイリアスは リポジトリ直下の .cargo/config.toml）

mod agent_harness;
mod assets;
mod bundled;
mod character_pack;
mod cli;
mod license;
mod office;
mod release;
mod tools;
mod util;
mod verify;

use std::process::Command;

use anyhow::{bail, Context, Result};

use util::{log, repo_root};

const USAGE: &str = "\
使い方: cargo xtask <サブコマンド>

  dev                 最小構成で開発起動（追加機能は setup で明示取得）
  build               任意の大容量追加機能を含めない開発用製品ビルド
  build-full          全追加機能を明示取得して作る開発用フルビルド
  build-beta          販売ゲート前の軽量アルファ版インストーラを生成
  build-distribution  販売条件を検査し、販売対象キャラだけで製品ビルド
  preview-distribution 販売対象キャラだけのUIを生成（法務ゲート前の確認用）
  release-check       販売前の未完了項目を一覧表示
  bump-version <種別> 公開中の版から major / minor / patch を自動採番して全設定へ反映
  update-feed <配布物> [説明] 公開Git用latest.jsonへWindows(.exe)／macOS(.dmg)の更新情報を書き出す
  license keygen       販売者用の署名鍵を生成（秘密鍵はリポジトリ外）
  license issue ...    BOOTH等の購入者へ渡すProライセンスを発行
  setup <対象…>        取得対象は all / engine / aivis / whisper / pixi / live2d / live2d-samples / tha3 / office / tools
                      （不要な大容量取得を防ぐため対象指定は必須）
  sync-characters [--force] characters/ を ui/assets/characters/ へ差分同期（--force で全再構築）
  character-pack      2Dキャラクターパックを検証・導入
  build-ui [--release] [--beta] [--force] ui/ を ui-dist/ へ差分ビルド（--force で全再構築）
  verify              本体・xtask・JavaScript・Git差分の検証を最後まで実行
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
        "dev" => tauri("dev", rest, Build::default()),
        "build" => tauri(
            "build",
            rest,
            Build {
                lightweight_bundle: true,
                ..Build::default()
            },
        ),
        "build-full" => tauri("build", rest, Build::default()),
        "build-beta" => tauri(
            "build",
            rest,
            Build {
                lightweight_bundle: true,
                beta: true,
                ..Build::default()
            },
        ),
        "build-distribution" => tauri(
            "build",
            rest,
            Build {
                lightweight_bundle: true,
                require_release_check: true,
                ..Build::default()
            },
        ),
        "preview-distribution" => preview_distribution(),
        "release-check" => {
            let _ = release::check(false)?;
            Ok(())
        }
        "bump-version" => release::bump_version(rest),
        "update-feed" => release::write_update_feed(rest),
        "license" => license::run(rest),
        "setup" => setup(rest),
        "sync-characters" => {
            reject_unknown_flags(rest, &["--force"])?;
            assets::sync_characters_with_force(rest.iter().any(|a| a == "--force"))
        }
        "character-pack" => character_pack::run(rest),
        "build-ui" => {
            reject_unknown_flags(rest, &["--release", "--beta", "--force"])?;
            assets::build_ui_dist(
                rest.iter().any(|a| a == "--release"),
                rest.iter().any(|a| a == "--beta"),
                rest.iter().any(|a| a == "--force"),
            )
        }
        "verify" => verify::run(),
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

fn reject_unknown_flags(args: &[String], allowed: &[&str]) -> Result<()> {
    if let Some(unknown) = args.iter().find(|arg| !allowed.contains(&arg.as_str())) {
        bail!("未対応のオプションです: {unknown}");
    }
    Ok(())
}

fn preview_distribution() -> Result<()> {
    let characters = release::included_characters(&release::manifest()?)?;
    assets::sync_distribution_characters(&characters)?;
    assets::build_ui_dist(false, false, false)
}

/// ビルドの種類。真偽値を並べて渡すと呼び出し側で取り違えるため、名前を付けて渡す。
#[derive(Clone, Copy, Default)]
struct Build {
    /// 販売用の軽量バンドル設定（開発用の追加ツールを同梱しない）を使う。
    lightweight_bundle: bool,
    /// 販売前ゲート（`release-check`）を必須にする。
    require_release_check: bool,
    /// 開発中アルファ版として作る（全機能試用の `beta-full` と画面の ALPHA 表示）。
    beta: bool,
}

/// 同梱物を取得する。全対象の取得も `all` の明示を必須にする。
///
/// 個々の失敗は各取得側の方針（ベストエフォートか必須か）に従う。
fn setup(targets: &[String]) -> Result<()> {
    // THA3も開発環境で制作機能をすぐ確認できるよう一括取得へ含める。
    // 販売用軽量版には再配布せず、利用者が作者配布物を用意して設定する。
    let all = [
        "pixi", "live2d", "engine", "aivis", "whisper", "tha3", "office", "tools",
    ];
    anyhow::ensure!(
        !targets.is_empty(),
        "不要な大容量取得を防ぐため対象を指定してください（例: cargo xtask setup engine、全対象は cargo xtask setup all）"
    );
    let wanted: Vec<&str> = if targets.iter().any(|target| target == "all") {
        anyhow::ensure!(targets.len() == 1, "all と個別対象は同時に指定できません");
        all.to_vec()
    } else {
        targets.iter().map(String::as_str).collect()
    };

    for target in wanted {
        match target {
            "engine" => bundled::fetch_engine()?,
            "whisper" => bundled::fetch_whisper()?,
            "pixi" => bundled::fetch_pixi_runtime()?,
            "live2d" => bundled::fetch_live2d_runtime()?,
            "live2d-samples" => bundled::fetch_live2d_samples()?,
            "tha3" => bundled::fetch_tha3()?,
            "aivis" => office::fetch_aivis()?,
            "office" => office::fetch_office()?,
            "tools" => tools::fetch_all()?,
            other => bail!(
                "未知の取得対象です: {other}（all / engine / aivis / whisper / pixi / live2d / live2d-samples / tha3 / office / tools）"
            ),
        }
    }
    Ok(())
}

/// `tauri dev` / `tauri build` を前処理つきで実行する。
///
/// 開発起動はネットワークや未使用の追加機能に阻まれない最小構成とする。
/// 製品ビルドだけは配布に必要なランタイムと同梱物を事前に揃える。
fn tauri(mode: &str, extra: &[String], build: Build) -> Result<()> {
    let release = mode == "build";
    if build.require_release_check {
        let _ = release::check(true)?;
    }
    // 製品ビルドは通常版も販売版も配布許可リストだけを使う。開発サンプルを含む
    // 全キャラ同期は `tauri dev` に限定し、手元に取得済みのモデルが偶然混ざるのを防ぐ。
    let product_characters = product_characters_for_build(release)?;

    assets::stop_stale()?;
    if release {
        bundled::fetch_live2d_runtime()?;
    }
    if let Some(characters) = product_characters.as_ref() {
        assets::sync_distribution_characters(characters)?;
    } else {
        assets::sync_characters()?;
    }
    assets::build_ui_dist(release, build.beta, false)?;
    if release {
        bundled::fetch_engine()?;
    }
    if release && !build.lightweight_bundle {
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
    }

    log("xtask", format!("cargo tauri {mode} を開始します"));
    let mut command = Command::new("cargo");
    command.arg("tauri").arg(mode).args(extra);
    if build.lightweight_bundle {
        command.args(["--config", "src-tauri/tauri.distribution.conf.json"]);
    }
    if build.beta {
        command.args(["--features", "beta-full"]);
    }
    let status = command
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
            [
                "mugi",
                "rin",
                "mai",
                "koharu",
                "sena",
                "noa",
                "photoreal-prototype"
            ]
        );
    }

    #[test]
    fn 販売資料のスキル数が実装と一致する() {
        let root = super::repo_root();
        let skill_count = std::fs::read_dir(root.join("skills"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("mod.rs").is_file())
            .count();
        let sales = std::fs::read_to_string(root.join("docs/SALES.md")).unwrap();
        assert!(
            sales.contains(&format!("実務スキル {skill_count} 種")),
            "docs/SALES.md のスキル数が実装（{skill_count}種）と一致しません"
        );
        // 公開資料も同じ数を載せている。実装が増減したまま文言だけ古くならないようにする。
        for (path, expected) in [
            (
                "distribution/site/index.html",
                format!("{skill_count} SKILLS"),
            ),
            (
                "distribution/site/index.html",
                format!("{skill_count}種のスキル"),
            ),
            (
                "distribution/site/README.md",
                format!("実務スキル{skill_count}種"),
            ),
            (
                "distribution/site/README.md",
                format!("スキル{skill_count}種）"),
            ),
        ] {
            let text = std::fs::read_to_string(root.join(path)).unwrap();
            assert!(
                text.contains(&expected),
                "{path} に「{expected}」がありません（実装は{skill_count}種）"
            );
        }
    }

    /// 販売資料が挙げる自前キャラが、実際の同梱対象と食い違わないこと。
    ///
    /// スキル数と同じ形のずれが実際に起きた（3体と書いたまま6体へ増えていた）。
    /// キャラは商品の中身そのものなので、増減したら販売資料も必ず直す。
    #[test]
    fn 販売資料の自前キャラが同梱対象と一致する() {
        let root = super::repo_root();
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("distribution/release.json")).unwrap(),
        )
        .unwrap();
        let included: Vec<String> = manifest["includedCharacters"]
            .as_array()
            .expect("includedCharacters が配列でない")
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect();
        let sales = std::fs::read_to_string(root.join("docs/SALES.md")).unwrap();
        let missing: Vec<&String> = included
            .iter()
            .filter(|id| !sales.contains(id.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "docs/SALES.md が同梱キャラを挙げていません: {missing:?}"
        );
    }
}
