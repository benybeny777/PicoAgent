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
// 取得失敗は停止する。取得前の明示省略は TOOLS_SKIP=1、再取得は TOOLS_FORCE=1。
// 対象を絞る場合は PICOAGENT_SETUP_TOOLS=pandoc,ripgrep のようにカンマ区切りで指定。

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::util::{
    cleanup_dir, cleanup_file, download_to, env_or, exe_name, extract, find_file, flag, log,
    repo_root,
};

struct Tool {
    name: &'static str,
    /// 同梱判定に使う実行ファイル名（拡張子なし）。
    exe: &'static str,
    /// 取得先 URL。`None` の OS は下の同梱表で取得対象外にした組合せ。
    url: Option<String>,
    install_hint: &'static str,
}

/// 同梱するOS・CPUの唯一の判定表。配布先URLはこの判定を通った組合せだけ使う。
type BundlePlatform = (&'static str, &'static str);
type BundleTarget = (&'static str, &'static [BundlePlatform]);
const BUNDLE_TARGETS: &[BundleTarget] = &[
    (
        "office",
        &[
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ],
    ),
    (
        "aivis",
        &[
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ],
    ),
    (
        "whisper",
        &[
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ],
    ),
    (
        "pandoc",
        &[
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ],
    ),
    (
        "ripgrep",
        &[
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ],
    ),
    (
        "ffmpeg",
        &[
            ("windows", "x86_64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ],
    ),
    ("qpdf", &[("windows", "x86_64")]),
    ("poppler", &[("windows", "x86_64")]),
    ("imagemagick", &[("windows", "x86_64")]),
];

fn bundled_target(name: &str, os: &str, arch: &str) -> bool {
    BUNDLE_TARGETS
        .iter()
        .any(|(component, targets)| *component == name && targets.contains(&(os, arch)))
}

pub(crate) fn bundle_requested(name: &str, skip: &str) -> bool {
    if flag(skip) {
        log(
            "setup",
            format!("{name}: {skip}=1 の明示指定により取得しません"),
        );
        return false;
    }
    if !bundled_target(name, std::env::consts::OS, std::env::consts::ARCH) {
        log("setup", format!("{name}: このOS・CPUは同梱表の取得対象外です。公式ポータブル配布を同梱していません。手動導入後、設定の検出ボタンで実行パスを確認して保存してください。"));
        return false;
    }
    true
}

/// 省略は取得前に確定する。取得を始めたあとの失敗は必ず伝播させる。
pub(crate) fn fetch_component(
    name: &str,
    skip: &str,
    operation: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !bundle_requested(name, skip) {
        return Ok(());
    }
    operation()
}

fn tools() -> Vec<Tool> {
    let pandoc = env_or("PANDOC_VERSION", "3.12.1");
    let ripgrep = env_or("RIPGREP_VERSION", "15.2.0");
    let qpdf = env_or("QPDF_VERSION", "12.4.2");
    let poppler = env_or("POPPLER_WIN_VERSION", "26.09.0-0");
    let imagemagick = env_or("IMAGEMAGICK_VERSION", "7.1.2-32");

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
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
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
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
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
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
        },
        Tool {
            name: "qpdf",
            exe: "qpdf",
            // アセット名に `-bin-` は入らない（`qpdf-12.3.2-mingw64.zip`）。以前は入れていて 404 だった。
            url: win.then(|| format!("https://github.com/qpdf/qpdf/releases/download/v{qpdf}/qpdf-{qpdf}-mingw64.zip")),
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
        },
        Tool {
            name: "poppler",
            exe: "pdftotext",
            url: win.then(|| format!("https://github.com/oschwartz10612/poppler-windows/releases/download/v{poppler}/Release-{poppler}.zip")),
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
        },
        Tool {
            name: "imagemagick",
            exe: "magick",
            // 配布は GitHub Releases の **`.7z` のみ**（旧 `imagemagick.org/archive/binaries/` は消滅し、
            // zip 版も無い）。`extract()` は非 zip を `tar` へ回し、Windows 同梱の bsdtar が 7z を展開できる。
            url: win.then(|| format!("https://github.com/ImageMagick/ImageMagick/releases/download/{imagemagick}/ImageMagick-{imagemagick}-portable-Q16-x64.7z")),
            install_hint: "手動導入後、設定の検出ボタンで実行パスを確認して保存してください。",
        },
    ]
}

fn tools_dir() -> PathBuf {
    repo_root().join("src-tauri/resources/bundled/tools")
}

pub fn fetch_all() -> Result<()> {
    if flag("TOOLS_SKIP") {
        log("setup:tools", "TOOLS_SKIP=1 の明示指定により取得しません");
        return Ok(());
    }
    let dir = tools_dir();
    fs::create_dir_all(&dir)?;

    let only = env_or("PICOAGENT_SETUP_TOOLS", "");
    let only: Vec<String> = only
        .split(',')
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();

    for tool in tools() {
        if !only.is_empty() && !only.contains(&tool.name.to_string()) {
            continue;
        }
        fetch_component(tool.name, "TOOLS_SKIP", || fetch_one(&tool))
            .with_context(|| format!("{}の取得に失敗しました。{}", tool.name, tool.install_hint))?;
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
        bail!("同梱表で取得対象の{}に配布URLがありません", tool.name);
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
    crate::util::make_executable(&found)?;
    log(
        "setup:tools",
        format!("{} fetched: {}", tool.name, found.display()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同梱表は未対応の組合せと別cpuへの誤配布を除外する() {
        assert!(bundled_target("office", "windows", "x86_64"));
        assert!(!bundled_target("office", "linux", "x86_64"));
        for name in [
            "office",
            "aivis",
            "pandoc",
            "ripgrep",
            "ffmpeg",
            "qpdf",
            "poppler",
            "imagemagick",
        ] {
            assert!(!bundled_target(name, "windows", "aarch64"), "{name}");
        }
        for name in ["qpdf", "poppler", "imagemagick"] {
            assert!(!bundled_target(name, "macos", "aarch64"));
            assert!(!bundled_target(name, "linux", "x86_64"));
        }
        assert!(!bundled_target("whisper", "freebsd", "x86_64"));
    }

    #[test]
    fn 明示省略は取得ゼロで既定取得失敗は停止する() {
        const KEY: &str = "XTASK_TEST_COMPONENT_SKIP";
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var(KEY, value),
                    None => std::env::remove_var(KEY),
                }
            }
        }
        let _restore = Restore(std::env::var_os(KEY));
        let component = BUNDLE_TARGETS
            .iter()
            .find(|(name, _)| bundled_target(name, std::env::consts::OS, std::env::consts::ARCH))
            .unwrap()
            .0;
        std::env::set_var(KEY, "1");
        fetch_component(component, KEY, || panic!("明示省略で取得してはいけない")).unwrap();
        for value in ["0", "true", ""] {
            std::env::set_var(KEY, value);
            let mut calls = 0;
            let error = fetch_component(component, KEY, || {
                calls += 1;
                bail!("取得故障注入")
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            assert!(error.to_string().contains("取得故障注入"));
        }
        fetch_component("未対応部品", KEY, || {
            panic!("未同梱組合せで取得してはいけない")
        })
        .unwrap();
    }

    #[test]
    fn 取得経路は必須反転と失敗後の継続を再導入しない() {
        for source in [
            include_str!("main.rs"),
            include_str!("office.rs"),
            include_str!("tools.rs"),
            include_str!("bundled.rs"),
        ] {
            let production = source.split("#[cfg(test)]").next().unwrap();
            for key in ["OFFICE", "AIVIS", "WHISPER", "TOOLS"] {
                assert!(!production.contains(&format!("{key}_{}", "REQUIRED")));
            }
        }
        assert!(include_str!("main.rs").contains("bundled::fetch_whisper()?;"));
    }
}
