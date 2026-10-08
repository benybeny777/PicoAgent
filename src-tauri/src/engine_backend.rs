// engine_backend.rs - ローカルLLMの実行バックエンドを実機判定し、安全に準備する。
//
// Windows NVIDIA は CUDA を第一候補にする。ただし約540MBの追加配布物は
// nvidia-smi が実GPUを報告した場合だけ取得し、固定サイズ/SHA-256/実行時デバイス列挙を
// すべて通ったものだけ有効化する。失敗時は呼び出し元が同梱Vulkanへ戻す。

#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(windows)]
use anyhow::{Context, Result};
#[cfg(windows)]
use futures::StreamExt;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use sha2::{Digest, Sha256};
use tauri::AppHandle;

// 以下の定数・型・判定関数はWindows本番コードから使われるが、
// クロスプラットフォームのテスト（tests モジュール、OS問わず実行）からも参照するため
// `#[cfg(windows)]` では隠さず `allow(dead_code)` で非Windowsビルドの警告だけを抑止する。
#[cfg_attr(not(windows), allow(dead_code))]
const RELEASE_TAG: &str = "b11429";
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_CORE_NAME: &str = "llama-b11429-bin-win-cuda-13.4-x64.zip";
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_CORE_SIZE: u64 = 153_089_864;
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_CORE_SHA256: &str = "76ddc6eff2389570789ed608881efc6977751a722015d8e8c94f302224ff1a3a";
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_RUNTIME_NAME: &str = "cudart-llama-bin-win-cuda-13.4-x64.zip";
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_RUNTIME_SIZE: u64 = 423_535_356;
#[cfg_attr(not(windows), allow(dead_code))]
const CUDA_RUNTIME_SHA256: &str =
    "738f8c251ac22b70c3ae6f83a10cf222725df0395246a2cf58f32bdb85fbe668";
#[cfg_attr(not(windows), allow(dead_code))]
const ROCM_NAME: &str = "llama-b11429-bin-win-rocm-10.0-x64.zip";
#[cfg_attr(not(windows), allow(dead_code))]
const ROCM_SIZE: u64 = 256_958_369;
#[cfg_attr(not(windows), allow(dead_code))]
const ROCM_SHA256: &str = "289a8453bb222f0648f91eff3d55133c934119c841eb369b704aafae1d4719d2";
#[cfg_attr(not(windows), allow(dead_code))]
const SYCL_NAME: &str = "llama-b11429-bin-win-sycl-x64.zip";
#[cfg_attr(not(windows), allow(dead_code))]
const SYCL_SIZE: u64 = 147_963_744;
#[cfg_attr(not(windows), allow(dead_code))]
const SYCL_SHA256: &str = "4968340c18e7b34f67b7805b7198e71b24ed043728b4864931be926675af9542";
// 正本: https://rocm.docs.amd.com/en/docs-10.0.0/compatibility/compatibility-matrix.html
// （元データ: https://github.com/ROCm/ROCm/blob/docs/10.0.0/docs/data/os-support.yaml の
// Windows 対応行）。2026-10-08確認。llama.cpp固定版またはPicoAgent配布版の更新時に再確認する。
// ROCm 10.0 では RDNA2（RX 6000 / PRO W6000）と RX 7600 XT・RX 7650 GRE が Windows 対応から外れた。
// 一覧外の AMD GPU は大容量の ROCm 版を取らず Vulkan で動かす。
// Ryzen APU は表示アダプタ名が「AMD Radeon(TM) 8060S Graphics」のように CPU 名と異なるため、
// 下の CPU 名は現状一致しない（TASKS.md 参照）。
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_ROCM_RUNTIME_SKUS: &[&str] = &[
    "radeon ai pro r9700s",
    "radeon ai pro r9700",
    "radeon ai pro r9600d",
    "radeon rx 9070 xt",
    "radeon rx 9070 gre",
    "radeon rx 9070",
    "radeon rx 9060 xt lp",
    "radeon rx 9060 xt",
    "radeon rx 9060",
    "radeon rx 9050",
    "radeon pro w7900 dual slot",
    "radeon pro w7900",
    "radeon pro w7800 48gb",
    "radeon pro w7800",
    "radeon pro w7700",
    "radeon rx 7900 xtx",
    "radeon rx 7900 xt",
    "radeon rx 7900 gre",
    "radeon rx 7800 xt",
    "radeon rx 7700 xt",
    "radeon rx 7700",
    "radeon rx 7600",
    "ryzen ai max+ pro 395",
    "ryzen ai max+ 395",
    "ryzen ai max pro 390",
    "ryzen ai max 390",
    "ryzen ai max pro 385",
    "ryzen ai max 385",
    "ryzen ai max pro 380",
];
#[cfg(windows)]
const MAX_ARCHIVE_SIZE: u64 = 450 * 1024 * 1024;
#[cfg(windows)]
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(windows)]
const MAX_FILES: usize = 512;

#[derive(Clone, Debug)]
pub struct BackendCandidate {
    pub label: &'static str,
    pub server: PathBuf,
    pub ngl: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(not(windows), allow(dead_code))]
struct InstalledManifest {
    release_tag: String,
    backend: String,
    core_name: String,
    core_sha256: String,
    runtime_name: String,
    runtime_sha256: String,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl InstalledManifest {
    fn cuda() -> Self {
        Self {
            release_tag: RELEASE_TAG.to_string(),
            backend: "cuda-13.4".to_string(),
            core_name: CUDA_CORE_NAME.to_string(),
            core_sha256: CUDA_CORE_SHA256.to_string(),
            runtime_name: CUDA_RUNTIME_NAME.to_string(),
            runtime_sha256: CUDA_RUNTIME_SHA256.to_string(),
        }
    }

    fn is_current(&self) -> bool {
        self.release_tag == RELEASE_TAG
            && match self.backend.as_str() {
                "cuda-13.4" => {
                    self.core_name == CUDA_CORE_NAME
                        && self.core_sha256 == CUDA_CORE_SHA256
                        && self.runtime_name == CUDA_RUNTIME_NAME
                        && self.runtime_sha256 == CUDA_RUNTIME_SHA256
                }
                "rocm-10.0" => {
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
            backend: "rocm-10.0".to_string(),
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

// GPU別バックエンドの違いを1か所へ集める。取得・検証・入れ替えの手順は3種とも同じで、
// 以前は `ensure_cuda_engine` / `ensure_rocm_engine` / `ensure_sycl_engine` が同じ処理を
// 3本持っていた（CUDAだけ入れ替えを自前で書き、退避の戻し方が他と違っていた）。
#[cfg_attr(not(windows), allow(dead_code))]
struct GpuBackend {
    /// 候補一覧とログに出す表示名。
    label: &'static str,
    /// 配置フォルダ（`engine-<id>`）と退避・staging フォルダ名に使う識別子。
    id: &'static str,
    /// 取得開始を知らせる文言。
    preparing: &'static str,
    /// 取得する配布物（ファイル名・固定サイズ・SHA-256）。
    archives: &'static [(&'static str, u64, &'static str)],
    /// 導入済み判定に使う固定マニフェスト。
    manifest: fn() -> InstalledManifest,
    /// `--list-devices` の出力に現れるデバイス見出し。どれか1つでも出れば実機で使える。
    device_markers: &'static [&'static str],
    /// 実機で認識できなかったときに利用者へ返す理由。
    device_missing: &'static str,
}

#[cfg_attr(not(windows), allow(dead_code))]
const GPU_BACKENDS: &[GpuBackend] = &[
    GpuBackend {
        label: "CUDA",
        id: "cuda",
        preparing: "NVIDIA GPU向けのCUDA推論エンジンを準備しています",
        archives: &[
            (CUDA_CORE_NAME, CUDA_CORE_SIZE, CUDA_CORE_SHA256),
            (CUDA_RUNTIME_NAME, CUDA_RUNTIME_SIZE, CUDA_RUNTIME_SHA256),
        ],
        manifest: InstalledManifest::cuda,
        device_markers: &["CUDA0:"],
        device_missing: "CUDA版llama-serverがNVIDIA GPUを認識しません",
    },
    GpuBackend {
        label: "ROCm",
        id: "rocm",
        preparing: "AMD GPU向けのROCm推論エンジンを準備しています",
        archives: &[(ROCM_NAME, ROCM_SIZE, ROCM_SHA256)],
        manifest: InstalledManifest::rocm,
        device_markers: &["ROCm0:", "HIP0:"],
        device_missing: "ROCm版llama-serverがAMD GPUを認識しません",
    },
    GpuBackend {
        label: "SYCL",
        id: "sycl",
        preparing: "Intel GPU向けのSYCL推論エンジンを準備しています",
        archives: &[(SYCL_NAME, SYCL_SIZE, SYCL_SHA256)],
        manifest: InstalledManifest::sycl,
        device_markers: &["SYCL0:"],
        device_missing: "SYCL版llama-serverがIntel GPUを認識しません",
    },
];

pub async fn candidates(
    app: &AppHandle,
    bundled_server: PathBuf,
    configured_ngl: String,
) -> Vec<BackendCandidate> {
    // Windows以外ではCUDA/ROCm/SYCL判定を行わないため app を使わない。
    #[cfg(not(windows))]
    let _ = app;
    if configured_ngl.trim() == "0" {
        return vec![BackendCandidate {
            label: "CPU",
            server: bundled_server,
            ngl: "0".to_string(),
        }];
    }

    let mut result = Vec::new();
    #[cfg(windows)]
    {
        // 大容量の追加配布物は、実機で使えると判定できたGPU1種だけを取得する。
        let detected = if has_nvidia_cuda_device().await {
            GPU_BACKENDS.iter().find(|entry| entry.id == "cuda")
        } else if has_supported_amd_rocm_device() {
            GPU_BACKENDS.iter().find(|entry| entry.id == "rocm")
        } else if has_supported_intel_sycl_device() {
            GPU_BACKENDS.iter().find(|entry| entry.id == "sycl")
        } else {
            None
        };
        if let Some(backend) = detected {
            crate::engine::set_status(
                app,
                serde_json::json!({ "phase": "download", "message": backend.preparing }),
            );
            match ensure_gpu_engine(app, backend).await {
                Ok(server) => result.push(BackendCandidate {
                    label: backend.label,
                    server,
                    ngl: configured_ngl.clone(),
                }),
                Err(error) => crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "{}推論エンジンを利用できないためVulkanへ切り替えます: {error}",
                        backend.label
                    ),
                ),
            }
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

#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg_attr(not(windows), allow(dead_code))]
impl GpuBackend {
    /// 取得した `llama-server.exe` が実機GPUを認識するか確かめる。
    #[cfg(windows)]
    async fn probe(&self, server: &Path) -> bool {
        let mut command = crate::tools::process::command(server);
        command.arg("--list-devices");
        let label = format!("{}推論エンジン確認", self.label);
        crate::tools::process::output(command, 30, &label)
            .await
            .is_ok_and(|output| {
                output.status.success()
                    && self
                        .device_markers
                        .iter()
                        .any(|marker| output_contains(&output, marker))
            })
    }
}

// 異常終了で残った staging・退避フォルダかどうかを名前だけで判定する。
// 正常経路は自分で消すが、取得中にプロセスが落ちると数百MBがそのまま残り続ける。
// 実行中の自分のフォルダ（末尾が現在のプロセスID）は対象にしない。
#[cfg_attr(not(windows), allow(dead_code))]
fn is_stale_engine_work_dir(name: &str, current_pid: u32) -> bool {
    if !name.starts_with(".engine-") {
        return false;
    }
    if !name.contains("-stage-") && !name.contains("-backup-") {
        return false;
    }
    !name.ends_with(&format!("-{current_pid}"))
}

/// PicoAgent は同時に1つしか動かないため、他プロセスIDの残骸は回収してよい。
#[cfg(windows)]
fn sweep_stale_engine_work_dirs(app: &AppHandle, parent: &Path, completed_backend: Option<&str>) {
    let current = std::process::id();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let stale = match completed_backend {
            Some(id) => is_completed_engine_backup(name, id, current),
            None => is_stale_engine_work_dir(name, current),
        };
        if !stale {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(entry.path()) {
            crate::log_paths::append_app_log(
                app,
                &format!("前回の推論エンジン取得の残骸を削除できませんでした（{name}）: {error}"),
            );
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_completed_engine_backup(name: &str, id: &str, current_pid: u32) -> bool {
    name.strip_prefix(&format!(".engine-{id}-backup-"))
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some_and(|pid| pid != current_pid)
}

#[cfg(windows)]
async fn ensure_gpu_engine(app: &AppHandle, backend: &GpuBackend) -> Result<PathBuf> {
    let root = crate::log_paths::bundled_dir(app).join(format!("engine-{}", backend.id));
    let server = root.join("llama-server.exe");
    let manifest_path = root.join("picoagent-engine-backend.json");
    if server.is_file()
        && std::fs::read(&manifest_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<InstalledManifest>(&bytes).ok())
            .is_some_and(|manifest| manifest.is_current())
        && backend.probe(&server).await
    {
        // 正常な既存エンジンは取得処理へ進まないため、この経路でも削除失敗した
        // 旧版を回収する。確認済みの方式だけに限定し、別方式の復旧用には触れない。
        if let Some(parent) = root.parent() {
            sweep_stale_engine_work_dirs(app, parent, Some(backend.id));
        }
        return Ok(server);
    }

    let parent = root.parent().context("推論エンジンの配置先が不正です")?;
    std::fs::create_dir_all(parent)?;
    sweep_stale_engine_work_dirs(app, parent, None);
    let work = parent.join(format!(
        ".engine-{}-stage-{}",
        backend.id,
        std::process::id()
    ));
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    std::fs::create_dir_all(&work)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()?;
    let staged_server = work.join("llama-server.exe");
    let install = async {
        for (name, size, sha256) in backend.archives {
            let archive = work.join(format!("{name}.part"));
            download_fixed(&client, asset_url(name), &archive, *size).await?;
            verify_sha256(&archive, sha256)?;
            extract_zip_merge(&archive, &work)?;
            std::fs::remove_file(&archive)?;
        }
        anyhow::ensure!(
            staged_server.is_file(),
            "{}版llama-serverが配布物にありません",
            backend.label
        );
        anyhow::ensure!(
            backend.probe(&staged_server).await,
            "{}",
            backend.device_missing
        );
        crate::store::write_json_atomic(
            &work.join("picoagent-engine-backend.json"),
            &(backend.manifest)(),
        )?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = install {
        if let Err(cleanup) = std::fs::remove_dir_all(&work) {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "取得に失敗した推論エンジンの作業フォルダを削除できませんでした: {cleanup}"
                ),
            );
        }
        return Err(error);
    }
    replace_directory(&work, &root, backend.id)?;
    Ok(server)
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
        // 戻しにも失敗すると、動いていたエンジンが退避先に残ったまま本来の場所から消える。
        // 「入れ替えに失敗した」だけを返すと、利用者は復旧先を知らないまま再取得へ進む。
        if backup.exists() {
            if let Err(restore) = std::fs::rename(&backup, destination) {
                anyhow::bail!(
                    "推論エンジンの入れ替えに失敗し、元の内容も戻せませんでした。退避先: {} ({error} / {restore})",
                    backup.display()
                );
            }
        }
        return Err(error.into());
    }
    // 新版はもう配置済み。旧版を消せないだけで失敗扱いにすると、使えるエンジンを捨てて
    // Vulkan へ落ちるので成功として返す（残りは次回の `sweep_stale_engine_work_dirs` が消す）。
    if backup.exists() {
        if let Err(error) = std::fs::remove_dir_all(&backup) {
            crate::log_paths::log_detached(&format!(
                "入れ替え前の推論エンジンを削除できません（次回の起動時に消します）: {} ({error})",
                backup.display()
            ));
        }
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
        "推論エンジン配布物のファイル数が上限を超えています"
    );
    let mut extracted = 0_u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .context("推論エンジン配布物に危険なパスがあります")?
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
            "推論エンジン配布物の展開サイズが上限を超えています"
        );
        extracted = extracted.saturating_add(copied);
    }
    Ok(())
}

#[cfg(windows)]
fn output_contains(output: &std::process::Output, needle: &str) -> bool {
    std::str::from_utf8(&output.stdout).is_ok_and(|text| text.contains(needle))
        || std::str::from_utf8(&output.stderr).is_ok_and(|text| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 検証済み方式の終了済み退避先だけを回収する() {
        assert!(is_completed_engine_backup(
            ".engine-cuda-backup-123",
            "cuda",
            999
        ));
        for name in [
            ".engine-cuda-backup-999",
            ".engine-rocm-backup-123",
            ".engine-cuda-stage-123",
            ".engine-cuda-backup-invalid",
        ] {
            assert!(!is_completed_engine_backup(name, "cuda", 999));
        }
    }

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
        assert!(is_supported_amd_rocm_name("AMD Radeon RX 9060 XT"));
        assert!(!is_supported_amd_rocm_name("AMD Radeon RX 6800S"));
        // ROCm 10.0 で Windows 対応から外れた RDNA2 と RX 7600 XT は Vulkan へ回す。
        assert!(!is_supported_amd_rocm_name("AMD Radeon RX 6800 XT"));
        assert!(!is_supported_amd_rocm_name("AMD Radeon PRO W6800"));
        assert!(!is_supported_amd_rocm_name("AMD Radeon RX 7600 XT"));
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
    fn 実行中プロセス以外の取得残骸だけを回収対象にする() {
        assert!(is_stale_engine_work_dir(".engine-cuda-stage-1234", 4321));
        assert!(is_stale_engine_work_dir(".engine-rocm-backup-1234", 4321));
        // 実行中の自分の作業フォルダは消さない（取得中の中身が飛ぶ）。
        assert!(!is_stale_engine_work_dir(".engine-sycl-stage-4321", 4321));
        // 完成済みの配置フォルダと無関係なフォルダには触れない。
        assert!(!is_stale_engine_work_dir("engine-cuda", 4321));
        assert!(!is_stale_engine_work_dir(".engine-cuda", 4321));
        assert!(!is_stale_engine_work_dir("models", 4321));
    }

    #[test]
    fn gpuバックエンド表は識別子と配布物を重複なく持つ() {
        for backend in GPU_BACKENDS {
            assert!(!backend.archives.is_empty(), "{}", backend.label);
            assert!(!backend.device_markers.is_empty(), "{}", backend.label);
            // マニフェストは自分のバックエンド用のものであること（取り違えると再取得が毎回走る）。
            assert!((backend.manifest)().is_current(), "{}", backend.label);
        }
        let ids: Vec<&str> = GPU_BACKENDS.iter().map(|entry| entry.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "識別子が重複しています: {ids:?}");
    }

    #[test]
    fn cuda13はturing以降かつdriver580以上に限定する() {
        assert!(supports_cuda_13(b"591.59, 12.0\n"));
        assert!(supports_cuda_13(b"580.00, 7.5\n"));
        assert!(!supports_cuda_13(b"579.99, 8.9\n"));
        assert!(!supports_cuda_13(b"591.59, 7.0\n"));
    }
}
