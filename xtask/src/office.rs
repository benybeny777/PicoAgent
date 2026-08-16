// xtask/src/office.rs - OfficeCLI（LibreOffice）と AivisSpeech Engine の取得。
//
// 旧 `scripts/fetch-office.mjs` / `fetch-aivis.mjs` の置き換え。
// どちらもベストエフォート同梱で、失敗しても既定では起動を続行する。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::util::{
    cleanup_dir, cleanup_file, command_exists, download_to, env_or, exe_name, extract, find_file,
    flag, get_json, log, make_executable, mb, repo_root, warn,
};

// ── OfficeCLI（LibreOffice） ───────────────────────────────────────────

/// LibreOffice を `resources/bundled/office/` へ配置する。
///
///   Windows … 公式 MSI を administrative install（`msiexec /a`）で展開（管理者権限不要）
///   macOS   … 公式 DMG をマウントして `LibreOffice.app` をコピー
///   Linux   … 公式のポータブル配布が無いため警告のみ（`apt` 等で導入し PATH の soffice を使う）
///
/// 失敗は既定で警告のみ（Office 変換なしで起動を続行）。必須なら `OFFICE_REQUIRED=1`。
/// 再取得は `OFFICE_FORCE=1`、版は `LIBREOFFICE_VERSION` で上書きできる。
pub fn fetch_office() -> Result<()> {
    let result = fetch_office_inner();
    match result {
        Ok(()) => Ok(()),
        Err(error) if !flag("OFFICE_REQUIRED") => {
            // フォールバック許可: ベストエフォート同梱（未取得でも Office 変換以外は動く）。
            warn("setup:office", error.to_string());
            warn(
                "setup:office",
                "Office ファイル変換なしでの起動を続行します。必須にする場合は OFFICE_REQUIRED=1 を付けるか、PICOAGENT_LIBREOFFICE_PATH で既存の LibreOffice を指定してください。",
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn fetch_office_inner() -> Result<()> {
    let dir = repo_root().join("src-tauri/resources/bundled/office");
    fs::create_dir_all(&dir)?;
    let wanted = exe_name("soffice");

    if flag("OFFICE_FORCE") {
        cleanup_dir(&dir, "旧Office配置")?;
        fs::create_dir_all(&dir)?;
    } else if let Some(existing) = find_file(&dir, &wanted) {
        log(
            "setup:office",
            format!("office cli ready: {}", existing.display()),
        );
        return Ok(());
    }

    // 公式が慎重な利用者向けに案内する成熟ブランチを固定し、監査時に最新版へ更新する。
    // **バージョンは4桁**（`24.8.7.2`）。`old/` 配下は4桁のディレクトリしか無く、
    // 3桁（`24.8.4`）を指定すると 404 になる（実際にこれで取得できていなかった）。
    let version = env_or("LIBREOFFICE_VERSION", "25.8.7.2");
    let base = format!("https://downloadarchive.documentfoundation.org/libreoffice/old/{version}");

    if cfg!(windows) {
        fetch_office_windows(&dir, &base, &version)?;
    } else if cfg!(target_os = "macos") {
        fetch_office_macos(&dir, &base, &version)?;
    } else {
        bail!("Linux 向けの公式ポータブル配布が無いため同梱できません。apt 等で libreoffice を導入し、PATH の soffice か PICOAGENT_LIBREOFFICE_PATH 設定を使ってください。");
    }

    let found =
        find_file(&dir, &wanted).ok_or_else(|| anyhow!("展開後に {wanted} が見つかりません。"))?;
    let size = fs::metadata(&found).map(|m| m.len()).unwrap_or(0);
    log(
        "setup:office",
        format!("office cli fetched: {} ({})", found.display(), mb(size)),
    );
    Ok(())
}

/// `msiexec` へ渡すパスを Windows の区切り文字へ正規化する。
///
/// `repo_root().join("src-tauri/resources/bundled/office")` は区切りが混在した
/// `C:\...\src-tauri/resources/bundled/office` を作る。tar や PowerShell はこれを許容するが、
/// **`msiexec` は混在パスのパッケージを開けず 1619（ERROR_INSTALL_PACKAGE_OPEN_FAILED）で
/// 失敗する**（実際にこれで LibreOffice の同梱取得がずっと失敗していた。ダウンロード自体は
/// 成功しているうえ警告止まりなので気づきにくい）。
fn msiexec_path(path: &Path) -> String {
    if cfg!(windows) {
        path.to_string_lossy().replace('/', "\\")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn fetch_office_windows(dir: &Path, base: &str, version: &str) -> Result<()> {
    let msi = dir.join(format!("LibreOffice_{version}.msi"));
    download_to(
        &format!("{base}/win/x86_64/LibreOffice_{version}_Win_x86-64.msi"),
        &msi,
    )?;
    // administrative install はファイル展開のみで、管理者権限やレジストリ変更を伴わない。
    let target = dir.join("extract");
    let status = Command::new("msiexec")
        .arg("/a")
        .arg(msiexec_path(&msi))
        .arg("/qn")
        .arg(format!("TARGETDIR={}", msiexec_path(&target)))
        .status()
        .context("msiexec を起動できません")?;
    cleanup_file(&msi, "展開済みLibreOffice MSI")?;
    if !status.success() {
        bail!(
            "msiexec /a に失敗しました（終了コード {:?}）",
            status.code()
        );
    }
    Ok(())
}

fn fetch_office_macos(dir: &Path, base: &str, version: &str) -> Result<()> {
    let suffix = if cfg!(target_arch = "aarch64") {
        format!("aarch64/LibreOffice_{version}_MacOS_aarch64.dmg")
    } else {
        format!("x86_64/LibreOffice_{version}_MacOS_x86-64.dmg")
    };
    let dmg = dir.join("LibreOffice.dmg");
    download_to(&format!("{base}/mac/{suffix}"), &dmg)?;

    let mount = dir.join("mnt");
    let attach = Command::new("hdiutil")
        .arg("attach")
        .arg(&dmg)
        .args(["-nobrowse", "-mountpoint"])
        .arg(&mount)
        .status()
        .context("hdiutil を起動できません")?;
    if !attach.success() {
        bail!("hdiutil attach に失敗しました");
    }
    let copied = Command::new("cp")
        .arg("-R")
        .arg(mount.join("LibreOffice.app"))
        .arg(dir)
        .status();
    // マウント解除は失敗しても続行する（コピー結果で判定する）。
    let _ = Command::new("hdiutil").arg("detach").arg(&mount).status();
    cleanup_file(&dmg, "展開済みLibreOffice DMG")?;

    if !copied.map(|status| status.success()).unwrap_or(false) {
        bail!("LibreOffice.app のコピーに失敗しました");
    }
    Ok(())
}

// ── AivisSpeech Engine（読み上げ） ─────────────────────────────────────

/// AivisSpeech Engine を `resources/bundled/aivis/` へ配置する。
///
/// 更新時は `AIVIS_RELEASE=latest` またはタグを指定、再取得は `AIVIS_FORCE=1`。
/// `.7z.001` の展開に `7z` / `7za` が要るため、無い環境では警告のみで続行する
/// （必須にする場合は `AIVIS_REQUIRED=1`）。
pub fn fetch_aivis() -> Result<()> {
    match fetch_aivis_inner() {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            let missing_7z = message.contains("7z") || message.contains("p7zip");
            if !cfg!(windows) && !flag("AIVIS_REQUIRED") && missing_7z {
                // フォールバック許可: 展開ツールが無い環境では音声なしで起動を続ける。
                warn("setup:aivis", message);
                warn(
                    "setup:aivis",
                    "音声を使わない起動は続行します。Aivis を必須にする場合は AIVIS_REQUIRED=1 を指定してください。",
                );
                return Ok(());
            }
            Err(error)
        }
    }
}

fn fetch_aivis_inner() -> Result<()> {
    let dir = repo_root().join("src-tauri/resources/bundled/aivis");
    let legacy = repo_root().join("src-tauri/resources/aivis");
    let run = exe_name("run");

    if !flag("AIVIS_FORCE") {
        if let Some(existing) = find_file(&dir, &run) {
            make_executable(&existing);
            log("setup:aivis", format!("aivis ready: {}", dir.display()));
            return Ok(());
        }
        if let Some(legacy_run) = find_file(&legacy, &run) {
            copy_engine_tree(&legacy_run, &dir)?;
            log(
                "setup:aivis",
                format!("旧 resources/aivis から移行しました: {}", dir.display()),
            );
            return Ok(());
        }
    }

    let default_tag = aivis_target_version()?;
    let tag = env_or("AIVIS_RELEASE", &default_tag);
    let api = if tag == "latest" {
        "https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases/latest".to_string()
    } else {
        format!("https://api.github.com/repos/Aivis-Project/AivisSpeech-Engine/releases/tags/{tag}")
    };
    let release = get_json(&api)?;
    let (name, url) = pick_aivis_asset(&release)?;

    let work = std::env::temp_dir().join("picoagent-aivis");
    cleanup_dir(&work, "前回のAivis導入途中フォルダ")?;
    fs::create_dir_all(&work)?;
    let archive = work.join(&name);
    let bytes = download_to(&url, &archive)?;
    log("setup:aivis", format!("fetched {name} ({})", mb(bytes)));

    let extracted = work.join("x");
    fs::create_dir_all(&extracted)?;
    extract_aivis(&archive, &extracted)?;

    let run_path = find_file(&extracted, &run)
        .ok_or_else(|| anyhow!("AivisSpeech Engine の run 実行ファイルが見つかりません"))?;
    copy_engine_tree(&run_path, &dir)?;
    cleanup_dir(&work, "完了したAivis導入途中フォルダ")?;
    log("setup:aivis", format!("aivis ready: {}", dir.display()));
    Ok(())
}

fn aivis_target_version() -> Result<String> {
    let path = repo_root().join("src-tauri/resources/component-manifest.json");
    let text =
        fs::read_to_string(&path).with_context(|| format!("{} を読めません", path.display()))?;
    let manifest: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} が不正なJSONです", path.display()))?;
    manifest
        .pointer("/components/aivis/targetVersion")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty() && !version.contains(['/', '\\']))
        .map(ToString::to_string)
        .context("Aivis の配布対象版が不正です")
}

fn pick_aivis_asset(release: &Value) -> Result<(String, String)> {
    let arm = cfg!(target_arch = "aarch64");
    let want = if cfg!(windows) {
        "AivisSpeech-Engine-Windows-x64-"
    } else if cfg!(target_os = "macos") {
        if arm {
            "AivisSpeech-Engine-macOS-arm64-"
        } else {
            "AivisSpeech-Engine-macOS-x64-"
        }
    } else if arm {
        "AivisSpeech-Engine-Linux-arm64-"
    } else {
        "AivisSpeech-Engine-Linux-x64-"
    };

    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("リリース情報に assets がありません"))?;
    for asset in assets {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
        if name.starts_with(want) && name.ends_with(".7z.001") {
            let url = asset
                .get("browser_download_url")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("アセットの URL がありません: {name}"))?;
            return Ok((name.to_string(), url.to_string()));
        }
    }
    bail!("一致するアセットが見つかりません: {want}*.7z.001")
}

fn extract_aivis(archive: &Path, out: &Path) -> Result<()> {
    let seven = if command_exists("7z") {
        Some("7z")
    } else if command_exists("7za") {
        Some("7za")
    } else {
        None
    };
    if let Some(seven) = seven {
        let status = Command::new(seven)
            .arg("x")
            .arg(archive)
            .arg(format!("-o{}", out.display()))
            .arg("-y")
            .status()
            .with_context(|| format!("{seven} を起動できません"))?;
        if !status.success() {
            bail!("{seven} での展開に失敗しました");
        }
        return Ok(());
    }
    if !cfg!(windows) && archive.to_string_lossy().ends_with(".7z.001") {
        bail!("Aivis の .7z.001 展開には 7z/7za が必要です。macOS は `brew install p7zip` を実行してください。");
    }
    extract(archive, out)
        .context("展開に失敗しました。7-Zip をインストールするか、tar が 7z 展開に対応する環境で実行してください。")
}

/// `run` があるディレクトリ一式を配置先へコピーする。
fn copy_engine_tree(run_path: &Path, dest: &Path) -> Result<()> {
    let root = run_path
        .parent()
        .ok_or_else(|| anyhow!("run の親ディレクトリを特定できません"))?;
    cleanup_dir(dest, "旧Aivis配置")?;
    crate::util::copy_dir(root, dest, &|_| false)?;
    fs::write(dest.join(".gitkeep"), "\n")?;

    let dest_run: PathBuf = dest.join(
        run_path
            .file_name()
            .ok_or_else(|| anyhow!("run のファイル名を特定できません"))?,
    );
    if !dest_run.is_file() {
        bail!("run 実行ファイルの配置に失敗しました");
    }
    make_executable(&dest_run);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aivisの開発取得も配布対象版を正本にする() {
        let version = aivis_target_version().unwrap();
        assert!(
            version.split('.').all(|part| part.parse::<u64>().is_ok()),
            "配布対象版は数値の版にすること: {version}"
        );
    }

    /// Aivis のリリース資産名からプラットフォーム用を選べること。
    #[test]
    fn aivisのアセットを選ぶ() {
        let release = serde_json::json!({ "assets": [
            { "name": "AivisSpeech-Engine-Windows-x64-1.2.0.7z.001", "browser_download_url": "u1" },
            { "name": "AivisSpeech-Engine-macOS-arm64-1.2.0.7z.001", "browser_download_url": "u2" },
            { "name": "AivisSpeech-Engine-macOS-x64-1.2.0.7z.001", "browser_download_url": "u3" },
            { "name": "AivisSpeech-Engine-Linux-x64-1.2.0.7z.001", "browser_download_url": "u4" },
            { "name": "AivisSpeech-Engine-Linux-arm64-1.2.0.7z.001", "browser_download_url": "u5" },
            { "name": "AivisSpeech-Engine-Linux-x64-1.2.0.7z.002", "browser_download_url": "u6" }
        ]});
        let (name, _url) = pick_aivis_asset(&release).unwrap();
        assert!(name.ends_with(".7z.001"), "分割の先頭を選ぶこと: {name}");
        assert!(name.contains(if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        }));
    }

    #[test]
    fn 該当アセットが無ければエラーにする() {
        let release = serde_json::json!({ "assets": [
            { "name": "SOURCE.tar.gz", "browser_download_url": "u" }
        ]});
        assert!(pick_aivis_asset(&release).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn msiexecへ渡すパスに区切りを混在させない() {
        // `repo_root().join("src-tauri/resources/...")` が作る混在パスを msiexec に渡すと
        // 1619 で失敗する。正規化を外すとここで気づけるようにしておく。
        let mixed = std::path::PathBuf::from(r"C:\repo\src-tauri/resources/bundled/office\x.msi");
        let normalized = super::msiexec_path(&mixed);
        assert!(
            !normalized.contains('/'),
            "msiexec へ渡すパスに / が残っています: {normalized}"
        );
        assert_eq!(
            normalized,
            r"C:\repo\src-tauri\resources\bundled\office\x.msi"
        );
    }
}
