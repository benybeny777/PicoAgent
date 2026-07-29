// xtask/src/assets.rs - キャラ同期・UI 配信ビルド・残存プロセス停止。
//
// 旧 `scripts/sync-characters.mjs` / `build-ui-dist.mjs` / `stop-stale.mjs` の置き換え。

use std::fs;
use std::process::Command;

use anyhow::{bail, Result};

use crate::util::{copy_dir, list_files, log, repo_root};

/// ルートの `characters/`（正本）を webview 配信先 `ui/assets/characters/` へ同期する。
///
/// キャラは扱いやすいようプロジェクト直下を正本にしているが、webview は
/// `frontendDist`（`ui/`）配下しか配信できないため、dev / build 前にコピーして配る。
/// 生成物は `.gitignore` 対象。
pub fn sync_characters() -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    if !src.exists() {
        bail!("キャラクターの正本が見つかりません: {}", src.display());
    }
    fs::remove_dir_all(&dest).ok();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    // live2d-src/（Cubism Editor 用の制作素材 PSD）は実行時に使わないため配信へ含めない。
    copy_dir(&src, &dest, &|path| {
        path.to_string_lossy().contains("live2d-src")
    })?;
    log(
        "sync-characters",
        format!("{} -> {} を同期しました", src.display(), dest.display()),
    );
    Ok(())
}

/// `ui/` を Tauri 配信用 `ui-dist/` へコピーする。
///
/// release では依存追加なしで HTML/CSS/JS の余分な空白とコメントを落とし、
/// 配布物から読みやすいソースを減らす（解析を不可能にするものではない）。
pub fn build_ui_dist(release: bool) -> Result<()> {
    let root = repo_root();
    let src = root.join("ui");
    let dest = root.join("ui-dist");
    if !src.exists() {
        bail!("ui が見つかりません: {}", src.display());
    }
    fs::remove_dir_all(&dest).ok();
    copy_dir(&src, &dest, &|_| false)?;

    if release {
        for file in list_files(&dest) {
            let ext = file
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            let minified = match ext.as_str() {
                "html" => minify_html(&text),
                "css" => minify_css(&text),
                "js" => minify_js(&text),
                _ => continue,
            };
            fs::write(&file, minified)?;
        }
    }

    log(
        "build-ui-dist",
        format!(
            "ui -> ui-dist ({})",
            if release { "release" } else { "dev" }
        ),
    );
    Ok(())
}

fn minify_html(text: &str) -> String {
    let without_comments = strip_between(text, "<!--", "-->");
    let mut out = String::with_capacity(without_comments.len());
    let chars = without_comments.chars();
    let mut pending_space = false;
    for ch in chars {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            // タグ間の空白は落とし、それ以外は1つの空白へまとめる。
            if !(out.is_empty() || ch == '<' && out.ends_with('>')) {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn minify_css(text: &str) -> String {
    let without_comments = strip_between(text, "/*", "*/");
    let mut collapsed = String::with_capacity(without_comments.len());
    let mut in_space = false;
    for ch in without_comments.chars() {
        if ch.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        in_space = false;
        collapsed.push(ch);
    }
    // 記号の前後の空白を落とす。
    let mut out = String::with_capacity(collapsed.len());
    let symbols = ['{', '}', ':', ';', ',', '>', '~', '+'];
    let chars: Vec<char> = collapsed.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == ' ' {
            let prev = out.chars().last();
            let next = chars.get(index + 1).copied();
            let touching_symbol = prev.is_some_and(|c| symbols.contains(&c))
                || next.is_some_and(|c| symbols.contains(&c));
            if touching_symbol {
                index += 1;
                continue;
            }
        }
        out.push(ch);
        index += 1;
    }
    out.replace(";}", "}").trim().to_string()
}

fn minify_js(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `open` と `close` に囲まれた範囲を落とす（コメント除去用の素朴な実装）。
fn strip_between(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) => rest = &after[end + close.len()..],
            // 閉じが無い場合は残り全部がコメント扱い。
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// 前回起動の残存プロセスを停止する。
///
/// 残っていると同梱 DLL を掴んだままになり、ビルドが `os error 32` で失敗したり、
/// 新しいインスタンスの WebView 生成と競合して起動直後に落ちたりする。
pub fn stop_stale() -> Result<()> {
    // PicoAgent 管理下のものだけを対象にする（同名の無関係プロセスを巻き込まない）。
    let targets = ["picoagent", "llama-server", "whisper-cli"];
    let mut stopped = Vec::new();

    for name in targets {
        let killed = if cfg!(windows) {
            Command::new("taskkill")
                .args(["/IM", &format!("{name}.exe"), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        } else {
            // 完全一致だけを対象にする（-x）。
            Command::new("pkill")
                .args(["-x", name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        if killed {
            stopped.push(name);
        }
    }

    // Aivis は PicoAgent 管理下のパスから起動したものだけを止める。
    stop_managed_aivis(&mut stopped);

    if stopped.is_empty() {
        log("stop-stale", "停止対象の残存プロセスはありません");
    } else {
        log(
            "stop-stale",
            format!("停止しました: {}", stopped.join(", ")),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    // 実行パスに PicoAgent 管理下の bundled/aivis を含むものだけを止める。
    let ok = Command::new("pkill")
        .args(["-f", "PicoAgent.*bundled.*aivis.*run"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}

#[cfg(not(unix))]
fn stop_managed_aivis(stopped: &mut Vec<&'static str>) {
    let ok = Command::new("taskkill")
        .args(["/IM", "run.exe", "/T", "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if ok {
        stopped.push("aivis");
    }
}
