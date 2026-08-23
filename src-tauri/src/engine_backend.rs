// engine_backend.rs - ローカルLLMの実行バックエンドを実機判定し、安全に準備する。
//
// Windows NVIDIA は CUDA を第一候補にする。ただし約540MBの追加配布物は
// nvidia-smi が実GPUを報告した場合だけ取得し、固定サイズ/SHA-256/実行時デバイス列挙を
// すべて通ったものだけ有効化する。失敗時は呼び出し元が同梱Vulkanへ戻す。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

const RELEASE_TAG: &str = "b10566";
const CUDA_CORE_NAME: &str = "llama-b10566-bin-win-cuda-13.3-x64.zip";
const CUDA_CORE_SIZE: u64 = 146_890_631;
const CUDA_CORE_SHA256: &str = "c3e2336c1427e8bd7b5beb3c8618d2f7a268bc5fb6ec3f28c1e06cdb78d2e80a";
const CUDA_RUNTIME_NAME: &str = "cudart-llama-bin-win-cuda-13.3-x64.zip";
const CUDA_RUNTIME_SIZE: u64 = 390_970_417;
const CUDA_RUNTIME_SHA256: &str =
    "1462a050eb4c684921ba51dcc4cc488a036674c3e73e9945ee705b854808d03e";
const ROCM_NAME: &str = "llama-b10566-bin-win-rocm-7.14-x64.zip";
const ROCM_SIZE: u64 = 196_945_718;
const ROCM_SHA256: &str = "699027a1017a0d1309abe3d6528639611de18ec24e006ece7d6400a4630e8ba6";
const SYCL_NAME: &str = "llama-b10566-bin-win-sycl-x64.zip";
const SYCL_SIZE: u64 = 119_838_336;
const SYCL_SHA256: &str = "118ae5d788ef29c2cb79a0ed0230e2f26adc725d82f7c188960ef62bd36f58a5";
// 正本: https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/shared/hipsdk/reference/system-requirements.html
// 2026-08-24確認。llama.cpp固定版またはPicoAgent配布版の更新時に再確認する。
const WINDOWS_ROCM_RUNTIME_SKUS: &[&str] = &[
    "radeon ai pro r9700",
    "radeon pro w7900 dual slot",
    "radeon pro w7900",
    "radeon pro w7800",
    "radeon pro w7700",
    "radeon pro w6800",
    "radeon pro w6600",
    "radeon rx 9070 xt",
    "radeon rx 9070",
    "rx 9070 gre",
    "radeon rx 9060 xt",
    "radeon rx 9060",
    "radeon rx 7900 xtx",
    "radeon rx 7900 xt",
    "radeon rx 7800 xt",
    "radeon rx 7700 xt",
    "radeon rx 7650 gre",
    "radeon rx 7600 xt",
    "radeon rx 7600",
    "radeon rx 6950 xt",
    "radeon rx 6900 xt",
    "radeon rx 6800 xt",
    "radeon rx 6800",
    "radeon rx 6750 xt",
    "radeon rx 6700 xt",
    "radeon rx 6700",
    "radeon rx 6650 xt",
    "radeon rx 6600 xt",
    "radeon rx 6600",
    "ryzen ai max+ pro 395",
    "ryzen ai max+ 395",
    "ryzen ai max pro 390",
    "ryzen ai max 390",
    "ryzen ai max pro 385",
    "ryzen ai max 385",
    "ryzen ai max pro 380",
];
const MAX_ARCHIVE_SIZE: u64 = 450 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 512;

#[derive(Clone, Debug)]
pub struct BackendCandidate {
    pub label: &'static str,
    pub server: PathBuf,
    pub ngl: String,
}

#[derive(Serialize, Deserialize)]
struct InstalledManifest {
    release_tag: String,
    backend: String,
    core_name: String,
    core_sha256: String,
    runtime_name: String,
    runtime_sha256: String,
}

impl InstalledManifest {
    fn cuda() -> Self {
        Self {
            release_tag: RELEASE_TAG.to_string(),
            backend: "cuda-13.3".to_string(),
            core_name: CUDA_CORE_NAME.to_string(),
            core_sha256: CUDA_CORE_SHA256.to_string(),
            runtime_name: CUDA_RUNTIME_NAME.to_string(),
            runtime_sha256: CUDA_RUNTIME_SHA256.to_string(),
        }
    }

    fn is_current(&self) -> bool {
        self.release_tag == RELEASE_TAG
            && match self.backend.as_str() {
                "cuda-13.3" => {
                    self.core_name == CUDA_CORE_NAME
                        && self.core_sha256 == CUDA_CORE_SHA256
                        && self.runtime_name == CUDA_RUNTIME_NAME
                        && self.runtime_sha256 == CUDA_RUNTIME_SHA256
                }
                "rocm-7.14" => {
                    self.core_name == ROCM_NAME
                        && self.core_sha256 == ROCM_SHA256
                        && self.runtime_name.is_empty()
                        && self.runtime_sha256.is_empty()
                }
                "sycl" => {
                    self.core_name == SYCL_NAME
                        && self.core_sha256 == SYCL_SHA256
                        && self.runtime_name.is_empty()
                        && self.runtime_sha256.is_empty()
                }
                _ => false,
            }
    }

    fn rocm() -> Self {
        Self {
            release_tag: RELEASE_TAG.to_string(),
            backend: "rocm-7.14".to_string(),
            core_name: ROCM_NAME.to_string(),
            core_sha256: ROCM_SHA256.to_string(),
            runtime_name: String::new(),
            runtime_sha256: String::new(),
        }
    }

    fn sycl() -> Self {
        Self {
            release_tag: RELEASE_TAG.to_string(),
            backend: "sycl".to_string(),
            core_name: SYCL_NAME.to_string(),
            core_sha256: SYCL_SHA256.to_string(),
            runtime_name: String::new(),
            runtime_sha256: String::new(),
        }
    }
}

pub async fn candidates(
    app: &AppHandle,
    bundled_server: PathBuf,
    configured_ngl: String,
) -> Vec<BackendCandidate> {
    if configured_ngl.trim() == "0" {
        return vec![BackendCandidate {
            label: "CPU",
            server: bundled_server,
            ngl: "0".to_string(),
        }];
    }

    let mut result = Vec::new();
    #[cfg(windows)]
    if has_nvidia_cuda_device().await {
        crate::engine::set_status(
            app,
            serde_json::json!({
                "phase": "download",
                "message": "NVIDIA GPU向けのCUDA推論エンジンを準備しています"
            }),
        );
        match ensure_cuda_engine(app).await {
            Ok(server) => result.push(BackendCandidate {
                label: "CUDA",
                server,
                ngl: configured_ngl.clone(),
            }),
            Err(error) => crate::log_paths::append_app_log(
                app,
                &format!("CUDA推論エンジンを利用できないためVulkanへ切り替えます: {error}"),
            ),
        }
    } else if has_supported_amd_rocm_device() {
        crate::engine::set_status(
            app,
            serde_json::json!({
                "phase": "download",
                "message": "AMD GPU向けのROCm推論エンジンを準備しています"
            }),
        );
        match ensure_rocm_engine(app).await {
            Ok(server) => result.push(BackendCandidate {
                label: "ROCm",
                server,
                ngl: configured_ngl.clone(),
            }),
            Err(error) => crate::log_paths::append_app_log(
                app,
                &format!("ROCm推論エンジンを利用できないためVulkanへ切り替えます: {error}"),
            ),
        }
    } else if has_supported_intel_sycl_device() {
        crate::engine::set_status(
            app,
            serde_json::json!({
                "phase": "download",
                "message": "Intel GPU向けのSYCL推論エンジンを準備しています"
            }),
        );
        match ensure_sycl_engine(app).await {
            Ok(server) => result.push(BackendCandidate {
                label: "SYCL",
                server,
                ngl: configured_ngl.clone(),
            }),
            Err(error) => crate::log_paths::append_app_log(
                app,
                &format!("SYCL推論エンジンを利用できないためVulkanへ切り替えます: {error}"),
            ),
        }
    }

    result.push(BackendCandidate {
        label: if cfg!(any(windows, target_os = "linux")) {
            "Vulkan"
        } else if cfg!(target_os = "macos") {
            "Metal"
        } else {
            "CPU"
        },
        server: bundled_server.clone(),
        ngl: configured_ngl,
    });
    result.push(BackendCandidate {
        label: "CPU",
        server: bundled_server,
        ngl: "0".to_string(),
    });
    result
}

#[cfg(windows)]
async fn has_nvidia_cuda_device() -> bool {
    let mut command = crate::tools::process::command("nvidia-smi.exe");
    command.args([
        "--query-gpu=driver_version,compute_cap",
        "--format=csv,noheader,nounits",
    ]);
    crate::tools::process::output(command, 10, "NVIDIA GPU確認")
        .await
        .is_ok_and(|output| output.status.success() && supports_cuda_13(&output.stdout))
}

fn supports_cuda_13(output: &[u8]) -> bool {
    let Ok(output) = std::str::from_utf8(output) else {
        return false;
    };
    output.lines().any(|line| {
        let Some((driver, compute)) = line.split_once(',') else {
            return false;
        };
        let driver_major = driver
            .trim()
            .split('.')
            .next()
            .and_then(|value| value.parse::<u32>().ok());
        let compute = compute.trim().parse::<f32>().ok();
        driver_major.is_some_and(|value| value >= 580) && compute.is_some_and(|value| value >= 7.5)
    })
}

#[cfg(windows)]
fn has_supported_amd_rocm_device() -> bool {
    crate::gpu::adapter_names()
        .iter()
        .any(|name| is_supported_amd_rocm_name(name))
}

fn is_supported_amd_rocm_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    let normalized = normalized.strip_prefix("amd ").unwrap_or(&normalized);
    WINDOWS_ROCM_RUNTIME_SKUS.contains(&normalized)
}

#[cfg(windows)]
fn has_supported_intel_sycl_device() -> bool {
    crate::gpu::adapter_names()
        .iter()
        .any(|name| is_supported_intel_sycl_name(name))
}

fn is_supported_intel_sycl_name(name: &str) -> bool {
    // 正本: https://www.intel.com/content/www/us/en/developer/articles/release-notes/oneapi-dpcpp/2026.html
    // Intel oneAPI 2026のGPU要件に明記された系統。旧世代にも同名が多いUHD/HDは
    // 大容量取得せずVulkanへ回す。Core Ultra等の新しい汎用名は正規化後の完全一致だけ許可する。
    let name = name
        .trim()
        .to_ascii_lowercase()
        .replace("(r)", "")
        .replace("(tm)", "");
    name.trim() == "intel graphics"
        || name.contains("intel arc")
        || name.contains("iris xe")
        || name.contains("data center gpu flex")
        || name.contains("data center gpu max")
        || name.contains("intel server gpu")
}

#[cfg(windows)]
async fn ensure_cuda_engine(app: &AppHandle) -> Result<PathBuf> {
    let root = crate::log_paths::bundled_dir(app).join("engine-cuda");
    let server = root.join("llama-server.exe");
    let manifest_path = root.join("picoagent-engine-backend.json");
    if server.is_file()
        && root.join("ggml-cuda.dll").is_file()
        && root.join("cudart64_13.dll").is_file()
        && std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstalledManifest>(&bytes).ok())
            .is_some_and(|manifest| manifest.is_current())
        && cuda_probe(&server).await
    {
        return Ok(server);
    }

    let parent = root.parent().context("CUDAエンジンの配置先が不正です")?;
    std::fs::create_dir_all(parent)?;
    let work = parent.join(format!(".engine-cuda-stage-{}", std::process::id()));
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    std::fs::create_dir_all(&work)?;
    let result = install_cuda_engine(&work).await;
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&work);
        return Err(error);
    }

    let staged_server = work.join("llama-server.exe");
    anyhow::ensure!(
        staged_server.is_file(),
        "CUDA版llama-serverが配布物にありません"
    );
    anyhow::ensure!(
        cuda_probe(&staged_server).await,
        "CUDA版llama-serverがNVIDIA GPUを認識しません"
    );
    crate::store::write_json_atomic(
        &work.join("picoagent-engine-backend.json"),
        &InstalledManifest::cuda(),
    )?;

    let backup = parent.join(format!(".engine-cuda-backup-{}", std::process::id()));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    if root.exists() {
        std::fs::rename(&root, &backup)?;
    }
    if let Err(error) = std::fs::rename(&work, &root) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &root);
        }
        return Err(error.into());
    }
    if backup.exists() {
        std::fs::remove_dir_all(backup)?;
    }
    Ok(root.join("llama-server.exe"))
}

#[cfg(windows)]
async fn install_cuda_engine(work: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    for (name, size, sha256) in [
        (CUDA_CORE_NAME, CUDA_CORE_SIZE, CUDA_CORE_SHA256),
        (CUDA_RUNTIME_NAME, CUDA_RUNTIME_SIZE, CUDA_RUNTIME_SHA256),
    ] {
        let archive = work.join(format!("{name}.part"));
        download_fixed(&client, asset_url(name), &archive, size).await?;
        verify_sha256(&archive, sha256)?;
        extract_zip_merge(&archive, work)?;
        std::fs::remove_file(archive)?;
    }
    Ok(())
}

#[cfg(windows)]
async fn ensure_rocm_engine(app: &AppHandle) -> Result<PathBuf> {
    let root = crate::log_paths::bundled_dir(app).join("engine-rocm");
    let server = root.join("llama-server.exe");
    let manifest_path = root.join("picoagent-engine-backend.json");
    if server.is_file()
        && std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstalledManifest>(&bytes).ok())
            .is_some_and(|manifest| manifest.is_current())
        && rocm_probe(&server).await
    {
        return Ok(server);
    }

    let parent = root.parent().context("ROCmエンジンの配置先が不正です")?;
    std::fs::create_dir_all(parent)?;
    let work = parent.join(format!(".engine-rocm-stage-{}", std::process::id()));
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    std::fs::create_dir_all(&work)?;
    let archive = work.join(format!("{ROCM_NAME}.part"));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    let install = async {
        download_fixed(&client, asset_url(ROCM_NAME), &archive, ROCM_SIZE).await?;
        verify_sha256(&archive, ROCM_SHA256)?;
        extract_zip_merge(&archive, &work)?;
        std::fs::remove_file(&archive)?;
        anyhow::ensure!(
            rocm_probe(&work.join("llama-server.exe")).await,
            "ROCm版llama-serverがAMD GPUを認識しません"
        );
        crate::store::write_json_atomic(
            &work.join("picoagent-engine-backend.json"),
            &InstalledManifest::rocm(),
        )?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = install {
        let _ = std::fs::remove_dir_all(&work);
        return Err(error);
    }
    replace_directory(&work, &root, "rocm")?;
    Ok(root.join("llama-server.exe"))
}

#[cfg(windows)]
async fn ensure_sycl_engine(app: &AppHandle) -> Result<PathBuf> {
    let root = crate::log_paths::bundled_dir(app).join("engine-sycl");
    let server = root.join("llama-server.exe");
    let manifest_path = root.join("picoagent-engine-backend.json");
    if server.is_file()
        && std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstalledManifest>(&bytes).ok())
            .is_some_and(|manifest| manifest.is_current())
        && sycl_probe(&server).await
    {
        return Ok(server);
    }

    let parent = root.parent().context("SYCLエンジンの配置先が不正です")?;
    std::fs::create_dir_all(parent)?;
    let work = parent.join(format!(".engine-sycl-stage-{}", std::process::id()));
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    std::fs::create_dir_all(&work)?;
    let archive = work.join(format!("{SYCL_NAME}.part"));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    let install = async {
        download_fixed(&client, asset_url(SYCL_NAME), &archive, SYCL_SIZE).await?;
        verify_sha256(&archive, SYCL_SHA256)?;
        extract_zip_merge(&archive, &work)?;
        std::fs::remove_file(&archive)?;
        anyhow::ensure!(
            sycl_probe(&work.join("llama-server.exe")).await,
            "SYCL版llama-serverがIntel GPUを認識しません"
        );
        crate::store::write_json_atomic(
            &work.join("picoagent-engine-backend.json"),
            &InstalledManifest::sycl(),
        )?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = install {
        let _ = std::fs::remove_dir_all(&work);
        return Err(error);
    }
    replace_directory(&work, &root, "sycl")?;
    Ok(root.join("llama-server.exe"))
}

#[cfg(windows)]
fn replace_directory(stage: &Path, destination: &Path, id: &str) -> Result<()> {
    let parent = destination
        .parent()
        .context("推論エンジンの配置先が不正です")?;
    let backup = parent.join(format!(".engine-{id}-backup-{}", std::process::id()));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    if destination.exists() {
        std::fs::rename(destination, &backup)?;
    }
    if let Err(error) = std::fs::rename(stage, destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(error.into());
    }
    if backup.exists() {
        std::fs::remove_dir_all(backup)?;
    }
    Ok(())
}

#[cfg(windows)]
fn asset_url(name: &str) -> String {
    format!("https://github.com/ggml-org/llama.cpp/releases/download/{RELEASE_TAG}/{name}")
}

#[cfg(windows)]
async fn download_fixed(
    client: &reqwest::Client,
    url: String,
    path: &Path,
    expected: u64,
) -> Result<()> {
    anyhow::ensure!(
        expected <= MAX_ARCHIVE_SIZE,
        "推論エンジン配布物がサイズ上限を超えています"
    );
    let response = client.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length == expected,
            "推論エンジン配布物のサイズが固定値と一致しません"
        );
    }
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(path).await?;
    let mut total = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total = total.saturating_add(chunk.len() as u64);
        anyhow::ensure!(
            total <= expected,
            "推論エンジン配布物が予定サイズを超えました"
        );
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    anyhow::ensure!(
        total == expected,
        "推論エンジン配布物のサイズが固定値と一致しません"
    );
    Ok(())
}

#[cfg(windows)]
fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    anyhow::ensure!(
        hex::encode(digest.finalize()) == expected,
        "推論エンジン配布物のSHA-256が一致しません"
    );
    Ok(())
}

#[cfg(windows)]
fn extract_zip_merge(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    anyhow::ensure!(
        zip.len() <= MAX_FILES,
        "CUDA配布物のファイル数が上限を超えています"
    );
    let mut extracted = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .context("CUDA配布物に危険なパスがあります")?
            .to_path_buf();
        let output = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let remaining = MAX_EXTRACTED_BYTES.saturating_sub(extracted);
        let mut target = std::fs::File::create(output)?;
        let copied = std::io::copy(
            &mut std::io::Read::take(&mut entry, remaining.saturating_add(1)),
            &mut target,
        )?;
        anyhow::ensure!(
            copied <= remaining,
            "CUDA配布物の展開サイズが上限を超えています"
        );
        extracted = extracted.saturating_add(copied);
    }
    Ok(())
}

#[cfg(windows)]
async fn cuda_probe(server: &Path) -> bool {
    let mut command = crate::tools::process::command(server);
    command.arg("--list-devices");
    crate::tools::process::output(command, 30, "CUDA推論エンジン確認")
        .await
        .is_ok_and(|output| output.status.success() && output_contains(&output, "CUDA0:"))
}

#[cfg(windows)]
async fn rocm_probe(server: &Path) -> bool {
    let mut command = crate::tools::process::command(server);
    command.arg("--list-devices");
    crate::tools::process::output(command, 30, "ROCm推論エンジン確認")
        .await
        .is_ok_and(|output| {
            output.status.success()
                && (output_contains(&output, "ROCm0:") || output_contains(&output, "HIP0:"))
        })
}

#[cfg(windows)]
async fn sycl_probe(server: &Path) -> bool {
    let mut command = crate::tools::process::command(server);
    command.arg("--list-devices");
    crate::tools::process::output(command, 30, "SYCL推論エンジン確認")
        .await
        .is_ok_and(|output| output.status.success() && output_contains(&output, "SYCL0:"))
}

fn output_contains(output: &std::process::Output, needle: &str) -> bool {
    std::str::from_utf8(&output.stdout).is_ok_and(|text| text.contains(needle))
        || std::str::from_utf8(&output.stderr).is_ok_and(|text| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 固定バックエンドマニフェストだけを現行として扱う() {
        assert!(InstalledManifest::cuda().is_current());
        let mut stale = InstalledManifest::cuda();
        stale.core_sha256 = "0".repeat(64);
        assert!(!stale.is_current());
        assert!(InstalledManifest::rocm().is_current());
        assert!(InstalledManifest::sycl().is_current());
    }

    #[test]
    fn amd公式対応skuだけをrocm候補にする() {
        assert!(is_supported_amd_rocm_name("AMD Radeon RX 7900 XTX"));
        assert!(is_supported_amd_rocm_name("AMD Ryzen AI Max+ 395"));
        assert!(!is_supported_amd_rocm_name("AMD Radeon RX 6800S"));
        assert!(!is_supported_amd_rocm_name("AMD Radeon Graphics"));
    }

    #[test]
    fn intel公式対応系統だけをsycl候補にする() {
        assert!(is_supported_intel_sycl_name("Intel(R) Arc(TM) Graphics"));
        assert!(is_supported_intel_sycl_name("Intel(R) Iris(R) Xe Graphics"));
        assert!(is_supported_intel_sycl_name("Intel(R) Graphics"));
        assert!(!is_supported_intel_sycl_name("Intel(R) UHD Graphics 630"));
        assert!(!is_supported_intel_sycl_name("Intel(R) HD Graphics 630"));
    }

    #[test]
    fn cuda13はturing以降かつdriver580以上に限定する() {
        assert!(supports_cuda_13(b"591.59, 12.0\n"));
        assert!(supports_cuda_13(b"580.00, 7.5\n"));
        assert!(!supports_cuda_13(b"579.99, 8.9\n"));
        assert!(!supports_cuda_13(b"591.59, 7.0\n"));
    }
}
