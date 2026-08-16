// xtask/src/tools.rs - 精度向上用の OSS CLI ツール群を同梱ディレクトリへ取得する。
//
// 旧 `scripts/fetch-tools.mjs` の置き換え。PicoAgent 管理下の
// `src-tauri/resources/bundled/tools/<ツール名>/` へ配置する。
//
//   pandoc      … 文書変換スキルの Markdown/EPUB/reST 変換を高精度化（win/mac/linux）
//   ripgrep     … ローカル文書検索スキルの走査を高速化（win/mac/linux）
//   ffmpeg      … 音声文字起こしスキルの mp3/m4a 等 → 16kHz WAV 前処理（win/mac/linux）
//   qpdf        … PDF 編集スキルのページ抽出・結合・回転（win のみ配布。mac/linux は brew/apt）
//   poppler     … PDF 編集スキルのテキスト抽出 pdftotext（win のみ配布）
//   imagemagick … 画像変換スキルの PSD/XCF/HEIC/TIFF/WebP 変換（win のみポータブル配布）
//
// 失敗は既定で警告のみ（各スキルは PATH のツールへフォールバックして続行）。
// 必須にする場合は TOOLS_REQUIRED=1。再取得は TOOLS_FORCE=1。
// 対象を絞る場合は PICOAGENT_SETUP_TOOLS=pandoc,ripgrep のようにカンマ区切りで指定。

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::util::{
    cleanup_dir, cleanup_file, download_to, env_or, exe_name, extract, find_file, flag, log,
    repo_root, warn,
};

struct Tool {
    name: &'static str,
    /// 同梱判定に使う実行ファイル名（拡張子なし）。
    exe: &'static str,
    /// 取得先 URL。`None` の OS は公式ポータブル配布が無いので PATH へ委ねる。
    url: Option<String>,
    fallback: &'static str,
}

fn tools() -> Vec<Tool> {
    let pandoc = env_or("PANDOC_VERSION", "3.10.2");
    let ripgrep = env_or("RIPGREP_VERSION", "15.2.0");
    let qpdf = env_or("QPDF_VERSION", "12.4.0");
    let poppler = env_or("POPPLER_WIN_VERSION", "26.02.0-0");
    let imagemagick = env_or("IMAGEMAGICK_VERSION", "7.1.2-29");

    let arm = cfg!(target_arch = "aarch64");
    let win = cfg!(windows);
    let mac = cfg!(target_os = "macos");

    vec![
        Tool {
            name: "pandoc",
            exe: "pandoc",
            url: Some(if win {
                format!("https://github.com/jgm/pandoc/releases/download/{pandoc}/pandoc-{pandoc}-windows-x86_64.zip")
            } else if mac {
                format!(
                    "https://github.com/jgm/pandoc/releases/download/{pandoc}/pandoc-{pandoc}-{}-macOS.zip",
                    if arm { "arm64" } else { "x86_64" }
                )
            } else {
                format!(
                    "https://github.com/jgm/pandoc/releases/download/{pandoc}/pandoc-{pandoc}-linux-{}.tar.gz",
                    if arm { "arm64" } else { "amd64" }
                )
            }),
            fallback: "apt install pandoc / brew install pandoc で導入すると PATH から使われます。",
        },
        Tool {
            name: "ripgrep",
            exe: "rg",
            url: Some(if win {
                format!("https://github.com/BurntSushi/ripgrep/releases/download/{ripgrep}/ripgrep-{ripgrep}-x86_64-pc-windows-msvc.zip")
            } else if mac {
                format!(
                    "https://github.com/BurntSushi/ripgrep/releases/download/{ripgrep}/ripgrep-{ripgrep}-{}-apple-darwin.tar.gz",
                    if arm { "aarch64" } else { "x86_64" }
                )
            } else {
                format!(
                    "https://github.com/BurntSushi/ripgrep/releases/download/{ripgrep}/ripgrep-{ripgrep}-{}.tar.gz",
                    if arm { "aarch64-unknown-linux-gnu" } else { "x86_64-unknown-linux-musl" }
                )
            }),
            fallback: "apt install ripgrep / brew install ripgrep で導入すると PATH から使われます。",
        },
        Tool {
            name: "ffmpeg",
            exe: "ffmpeg",
            url: Some(if win {
                // BtbN の latest タグは恒久 URL（自動更新）。100MB 前後と大きい。
                "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip".to_string()
            } else if mac {
                format!(
                    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/{}/release/ffmpeg.zip",
                    if arm { "arm64" } else { "amd64" }
                )
            } else {
                format!(
                    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-{}-gpl.tar.xz",
                    if arm { "linuxarm64" } else { "linux64" }
                )
            }),
            fallback: "apt install ffmpeg / brew install ffmpeg で導入すると PATH から使われます。",
        },
        Tool {
            name: "qpdf",
            exe: "qpdf",
            // アセット名に `-bin-` は入らない（`qpdf-12.3.2-mingw64.zip`）。以前は入れていて 404 だった。
            url: win.then(|| format!("https://github.com/qpdf/qpdf/releases/download/v{qpdf}/qpdf-{qpdf}-mingw64.zip")),
            fallback: "apt install qpdf / brew install qpdf で導入すると PATH から使われます。",
        },
        Tool {
            name: "poppler",
            exe: "pdftotext",
            url: win.then(|| format!("https://github.com/oschwartz10612/poppler-windows/releases/download/v{poppler}/Release-{poppler}.zip")),
            fallback: "apt install poppler-utils / brew install poppler で導入すると PATH から使われます。",
        },
        Tool {
            name: "imagemagick",
            exe: "magick",
            // 配布は GitHub Releases の **`.7z` のみ**（旧 `imagemagick.org/archive/binaries/` は消滅し、
            // zip 版も無い）。`extract()` は非 zip を `tar` へ回し、Windows 同梱の bsdtar が 7z を展開できる。
            url: win.then(|| format!("https://github.com/ImageMagick/ImageMagick/releases/download/{imagemagick}/ImageMagick-{imagemagick}-portable-Q16-x64.7z")),
            fallback: "apt install imagemagick / brew install imagemagick で導入すると PATH（magick / convert）から使われます。",
        },
    ]
}

fn tools_dir() -> PathBuf {
    repo_root().join("src-tauri/resources/bundled/tools")
}

pub fn fetch_all() -> Result<()> {
    let dir = tools_dir();
    fs::create_dir_all(&dir)?;

    let only = env_or("PICOAGENT_SETUP_TOOLS", "");
    let only: Vec<String> = only
        .split(',')
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();

    let mut failures = Vec::new();
    for tool in tools() {
        if !only.is_empty() && !only.contains(&tool.name.to_string()) {
            continue;
        }
        if let Err(error) = fetch_one(&tool) {
            failures.push(tool.name);
            warn("setup:tools", format!("{}: {error}", tool.name));
            warn("setup:tools", format!("  {}", tool.fallback));
        }
    }

    if !failures.is_empty() {
        if flag("TOOLS_REQUIRED") {
            bail!("同梱に失敗したツール: {}", failures.join(", "));
        }
        warn(
            "setup:tools",
            format!(
                "一部ツールは同梱できませんでした（{}）。該当スキルは PATH のツールを使います。",
                failures.join(", ")
            ),
        );
    }
    Ok(())
}

fn fetch_one(tool: &Tool) -> Result<()> {
    let dir = tools_dir().join(tool.name);
    fs::create_dir_all(&dir)?;

    let wanted = exe_name(tool.exe);
    if flag("TOOLS_FORCE") {
        cleanup_dir(&dir, "旧ツール配置")?;
        fs::create_dir_all(&dir)?;
    } else if let Some(existing) = find_file(&dir, &wanted) {
        log(
            "setup:tools",
            format!("{} ready: {}", tool.name, existing.display()),
        );
        return Ok(());
    }

    let Some(url) = tool.url.as_ref() else {
        warn(
            "setup:tools",
            format!(
                "{}: このOS向けのポータブル配布が無いため同梱をスキップします。{}",
                tool.name, tool.fallback
            ),
        );
        return Ok(());
    };

    // 取得と展開は隣の作業ディレクトリで行い、実行ファイルを確認できてから本設置する。
    // 直接 dir へ展開すると、中断時に半端に展開されたツリーが残り、次回の
    // 「実行ファイルがあるか」判定を通ってしまう（DLL 欠けのまま「取得済み」になる）。
    let work = dir.with_extension("incoming");
    cleanup_dir(&work, "前回のツール導入途中フォルダ")?;
    fs::create_dir_all(&work)?;

    let install = (|| -> Result<()> {
        let file_name = url
            .rsplit('/')
            .next()
            .and_then(|name| name.split('?').next())
            .unwrap_or("archive");
        let archive = work.join(file_name);
        download_to(url, &archive)?;
        extract(&archive, &work)?;
        cleanup_file(&archive, "展開済みツールアーカイブ")?;

        if find_file(&work, &wanted).is_none() {
            bail!("展開後に {wanted} が見つかりません。");
        }
        cleanup_dir(&dir, "旧ツール配置")?;
        fs::rename(&work, &dir)?;
        Ok(())
    })();

    if let Err(error) = install {
        cleanup_dir(&work, "失敗したツール導入途中フォルダ")
            .with_context(|| format!("元のツール導入エラー: {error:#}"))?;
        return Err(error);
    }

    let found = find_file(&dir, &wanted)
        .ok_or_else(|| anyhow::anyhow!("設置後に {wanted} が見つかりません。"))?;
    crate::util::make_executable(&found);
    log(
        "setup:tools",
        format!("{} fetched: {}", tool.name, found.display()),
    );
    Ok(())
}
