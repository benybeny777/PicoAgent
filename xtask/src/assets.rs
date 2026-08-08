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

/// 販売設定で明示したキャラだけを配信先へ同期する。
///
/// 開発用の公式サンプルや制作途中モデルを誤って販売物へ含めないため、通常同期とは
/// 別入口にする。元のcharacters/は変更せず、生成先のindex.jsonだけを絞り込む。
pub fn sync_distribution_characters(included: &[String]) -> Result<()> {
    let root = repo_root();
    let src = root.join("characters");
    let dest = root.join("ui/assets/characters");
    fs::remove_dir_all(&dest).ok();
    fs::create_dir_all(&dest)?;

    let index_path = src.join("index.json");
    let mut index: serde_json::Value = serde_json::from_str(&fs::read_to_string(&index_path)?)?;
    let characters = index
        .get_mut("characters")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("characters/index.json にcharacters配列がありません"))?;
    characters.retain(|character| {
        character
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| included.iter().any(|allowed| allowed == id))
    });
    if characters.len() != included.len() {
        bail!(
            "販売設定のキャラ件数とindex.jsonの一致件数が異なります: 設定{}件 / 一致{}件",
            included.len(),
            characters.len()
        );
    }
    let default = index
        .get("default")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !included.iter().any(|id| id == default) {
        index["default"] = serde_json::Value::String(included[0].clone());
    }
    fs::write(
        dest.join("index.json"),
        format!("{}\n", serde_json::to_string_pretty(&index)?),
    )?;

    for id in included {
        copy_dir(&src.join(id), &dest.join(id), &|path| {
            path.to_string_lossy().contains("live2d-src")
        })?;
    }
    let placeholder = src.join("_preview-placeholder.png");
    if placeholder.is_file() {
        fs::copy(placeholder, dest.join("_preview-placeholder.png"))?;
    }
    log(
        "sync-characters",
        format!(
            "販売対象{}体だけを同期しました: {}",
            included.len(),
            included.join(", ")
        ),
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
    // READMEで実表示を検証した8形式の画像を、設定画面の選択カードにも使う。
    // 正本を二重管理せず、配信用生成物へだけまとめてコピーする。
    let preview_src = root.join("docs/images");
    let preview_dest = dest.join("assets/layout-previews");
    fs::create_dir_all(&preview_dest)?;
    for id in [
        "pet",
        "mascot",
        "chat",
        "talk",
        "bar",
        "workspace",
        "stream",
        "widget",
    ] {
        let source = preview_src.join(format!("layout-shot-{id}.webp"));
        if !source.exists() {
            bail!(
                "表示形式のプレビュー画像が見つかりません: {}",
                source.display()
            );
        }
        fs::copy(source, preview_dest.join(format!("{id}.webp")))?;
    }

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

    // 配信する中身が確定してから版を打つ（release の最小化後の内容でハッシュを取る）。
    stamp_asset_versions(&dest)?;

    log(
        "build-ui-dist",
        format!(
            "ui -> ui-dist ({})",
            if release { "release" } else { "dev" }
        ),
    );
    Ok(())
}

/// HTML が読み込むローカルの CSS / JS へ、内容から決まる `?v=…` を付ける。
///
/// 手書きの版番号は付け忘れ・書き間違いが起きる（実際に `styles.css` と `renderer.js` で
/// 版が食い違っていた）。内容ハッシュにしておけば、変更したファイルだけが確実に読み直され、
/// webview のキャッシュに古い CSS が残らない。
fn stamp_asset_versions(dest: &std::path::Path) -> Result<()> {
    for file in list_files(dest) {
        if file.extension().map(|e| e != "html").unwrap_or(true) {
            continue;
        }
        let Some(dir) = file.parent().map(|dir| dir.to_path_buf()) else {
            continue;
        };
        let text = fs::read_to_string(&file)?;
        let stamped = rewrite_asset_refs(&text, &|relative| {
            let target = dir.join(relative);
            fs::read(&target).ok().map(|bytes| content_version(&bytes))
        });
        if stamped != text {
            fs::write(&file, stamped)?;
        }
    }
    Ok(())
}

/// 内容から決まる短い版文字列（FNV-1a。外部クレートを増やさない）。
fn content_version(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

/// HTML 内の `href="…"` / `src="…"` のうち、同梱の CSS / JS を指すものへ版を付け直す。
fn rewrite_asset_refs(html: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    const ATTRIBUTES: [&str; 2] = ["href=\"", "src=\""];
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    loop {
        let Some((index, attribute)) = ATTRIBUTES
            .iter()
            .filter_map(|attribute| rest.find(attribute).map(|index| (index, *attribute)))
            .min_by_key(|(index, _)| *index)
        else {
            out.push_str(rest);
            return out;
        };
        let value_start = index + attribute.len();
        out.push_str(&rest[..value_start]);
        let after = &rest[value_start..];
        let Some(end) = after.find('"') else {
            out.push_str(after);
            return out;
        };
        out.push_str(&stamped_reference(&after[..end], version_of));
        out.push('"');
        rest = &after[end + 1..];
    }
}

/// 参照1つ分の書き換え。外部 URL・絶対パス・CSS/JS 以外はそのまま返す。
fn stamped_reference(value: &str, version_of: &dyn Fn(&str) -> Option<String>) -> String {
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let lower = path.to_lowercase();
    if !(lower.ends_with(".css") || lower.ends_with(".js")) {
        return value.to_string();
    }
    if path.starts_with('/') || path.contains("://") || lower.starts_with("data:") {
        return value.to_string();
    }
    match version_of(path) {
        Some(version) => format!("{path}?v={version}"),
        None => value.to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::{content_version, rewrite_asset_refs};

    #[test]
    fn 同梱のcssとjsへ内容版を付ける() {
        let html =
            r#"<link rel="stylesheet" href="styles.css" /><script src="renderer.js"></script>"#;
        let stamped = rewrite_asset_refs(html, &|relative| match relative {
            "styles.css" => Some("aaaaaaaa".to_string()),
            "renderer.js" => Some("bbbbbbbb".to_string()),
            _ => None,
        });
        assert_eq!(
            stamped,
            r#"<link rel="stylesheet" href="styles.css?v=aaaaaaaa" /><script src="renderer.js?v=bbbbbbbb"></script>"#
        );
    }

    #[test]
    fn 手書きの版と外部参照は壊さない() {
        // 手書きの `?v=…` は内容版へ置き換える（付け忘れ・食い違いを残さない）。
        let stamped =
            rewrite_asset_refs(r#"<script src="renderer.js?v=20260807"></script>"#, &|_| {
                Some("cccccccc".to_string())
            });
        assert_eq!(stamped, r#"<script src="renderer.js?v=cccccccc"></script>"#);

        // 外部 URL・存在しないファイル・CSS/JS 以外は素通し。
        let untouched =
            r#"<a href="https://example.com/app.js"><img src="icon.png"><link href="missing.css">"#;
        assert_eq!(
            rewrite_asset_refs(untouched, &|relative| (relative == "styles.css")
                .then(|| "dddddddd".to_string())),
            untouched
        );
    }

    #[test]
    fn 内容が変われば版も変わる() {
        assert_eq!(content_version(b"body{}"), content_version(b"body{}"));
        assert_ne!(content_version(b"body{}"), content_version(b"body{ }"));
        assert_eq!(content_version(b"body{}").len(), 8);
    }
}
