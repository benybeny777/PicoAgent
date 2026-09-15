// generation_backend.rs - 画像・動画・キャラクター生成（ComfyUI／PyTorch）の実行バックエンド判定。
//
// モデル重み（safetensors）とComfyUIのworkflowはCUDA専用品ではない。GPUごとに違うのは
// **実行ランタイム**だけなので、ここでGPUの素性からバックエンドを1つだけ選び、
// 選ばれたバックエンドのPyTorchだけを導入する。CUDAとROCmを予備として両方取得しない。
//
// llama.cppのバックエンド判定（`engine_backend.rs`）とは対応表を共有しない。
// llama.cppのROCmビルドが動くGPUと、PyTorch ROCmが公式対応するGPUは一致しないため、
// 片方の表を流用すると非対応GPUで数十GBの取得を始めてしまう。

use serde::{Deserialize, Serialize};

use crate::gpu::{GpuDevice, GpuMemory, GpuVendor};

/// 実行バックエンド。boolean（`is_cuda`等）で持たず列挙子にする。
/// 将来Intel XPUへ対応する場合は、判定・導入・表示の分岐をここへ足す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenerationBackend {
    Cuda,
    Rocm,
}

impl GenerationBackend {
    /// 利用者へ見せる名前。
    pub fn label(self) -> &'static str {
        match self {
            Self::Cuda => "CUDA",
            Self::Rocm => "ROCm",
        }
    }

    /// 導入済みマニフェストへ記録する識別子。
    pub fn id(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
        }
    }

    /// 固定しているアクセラレータ版。
    pub fn accelerator(self) -> &'static str {
        match self {
            Self::Cuda => CUDA_RELEASE,
            Self::Rocm => ROCM_RELEASE,
        }
    }

    /// 固定しているPyTorch版。
    pub fn torch_version(self) -> &'static str {
        match self {
            Self::Cuda => CUDA_TORCH_VERSION,
            Self::Rocm => ROCM_TORCH_VERSION,
        }
    }

    /// `uv pip sync`へ渡すロックファイル名。共通依存とバックエンド固有依存の境界はここ。
    pub fn lock_file_name(self) -> &'static str {
        match self {
            Self::Cuda => "requirements-comfy.lock",
            Self::Rocm => "requirements-comfy-rocm.lock",
        }
    }
}

/// CUDA側の固定情報（既存経路。変更しない）。
const CUDA_RELEASE: &str = "cuda-12.8";
const CUDA_TORCH_VERSION: &str = "2.11.0+cu128";

/// ROCm側の固定情報。
///
/// 正本: AMD「PyTorch via PIP installation（Windows）」と「Windows support matrices by
/// ROCm version」。ROCm 7.2.1のWindows向けPyTorchはPython 3.12 ABIのwheelで配布され、
/// Radeonソフトウェア26.2.2以降を要求する。実wheelのURLとSHA-256は
/// `character-maker/sidecar/requirements-comfy-rocm.lock` が持つ。
const ROCM_RELEASE: &str = "rocm-7.2.1";
const ROCM_TORCH_VERSION: &str = "2.9.1+rocm7.2.1";
/// 利用者へ案内する必要ドライバー。
pub const ROCM_REQUIRED_DRIVER: &str = "26.2.2";

/// Windowsで公式ROCm版PyTorchが対応するGPU（正規化した小文字名の完全一致）。
///
/// 正本: AMD「Windows support matrices by ROCm version」。
/// `Radeon`の部分一致で全AMD GPUを対応扱いにしない。ここに無いAMD GPUは、
/// 非対応として何も取得しない（公式表を直接照合して追加する。`docs/TASKS.md`参照）。
const WINDOWS_ROCM_PYTORCH_SKUS: &[&str] = &[
    "radeon ai pro r9700",
    "radeon pro w7900 dual slot",
    "radeon pro w7900",
    "radeon rx 9070 xt",
    "radeon rx 9070",
    "radeon rx 9060 xt",
    "radeon rx 7900 xtx",
];

/// lowvram判定の境界。**十進の16GB**で判定する。
pub const LOWVRAM_THRESHOLD_BYTES: u64 = 16_000_000_000;

/// 選ばれたバックエンドと、その根拠になったGPU。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedBackend {
    pub backend: GenerationBackend,
    pub device: GpuDevice,
}

/// バックエンド判定の結果。非対応の理由は利用者へそのまま見せる日本語にする。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendSelection {
    Supported(SelectedBackend),
    Unsupported(String),
}

impl BackendSelection {
    pub fn supported(&self) -> Option<&SelectedBackend> {
        match self {
            Self::Supported(value) => Some(value),
            Self::Unsupported(_) => None,
        }
    }
}

/// AMD GPU名が公式ROCm版PyTorchの対応SKUかどうか。
fn is_supported_amd_pytorch_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix("amd ")
        .unwrap_or(&normalized)
        .trim()
        .to_string();
    WINDOWS_ROCM_PYTORCH_SKUS.contains(&normalized.as_str())
}

/// 実GPUから生成バックエンドを1つだけ選ぶ。
///
/// `nvidia_cuda_ready`はnvidia-smiがCUDA対応GPUを報告したかどうか。NVIDIAの既存判定を
/// 変えないため、判定そのものは呼び出し側（`components.rs`）が行った結果を受け取る。
/// 対応OSの判定は呼び出し側が先に行う（`install_character_maker`のWindows x64判定）。
/// ここはGPUの素性だけで決める純関数にして、OSに依らず機械検査できるようにする。
pub fn select(devices: &[GpuDevice], nvidia_cuda_ready: bool) -> BackendSelection {
    if devices.is_empty() {
        return BackendSelection::Unsupported(
            "GPUを確認できません。画像・動画・キャラクター生成には対応GPUが必要です。何も取得していません。".into(),
        );
    }
    // NVIDIAが居る場合は既存のCUDA経路を最優先する（挙動を変えない）。
    if let Some(device) = devices
        .iter()
        .find(|device| device.vendor == GpuVendor::Nvidia)
    {
        if nvidia_cuda_ready {
            return BackendSelection::Supported(SelectedBackend {
                backend: GenerationBackend::Cuda,
                device: device.clone(),
            });
        }
        return BackendSelection::Unsupported(format!(
            "NVIDIA GPU（{}）を確認しましたが、CUDA対応GPUとして応答しません。GeForce Game Ready ドライバーを更新してください。何も取得していません。",
            device.name
        ));
    }
    if let Some(device) = devices
        .iter()
        .find(|device| device.vendor == GpuVendor::Amd)
    {
        if is_supported_amd_pytorch_name(&device.name) {
            return BackendSelection::Supported(SelectedBackend {
                backend: GenerationBackend::Rocm,
                device: device.clone(),
            });
        }
        return BackendSelection::Unsupported(format!(
            "AMD GPU（{}）は、AMD公式のWindows向けROCm版PyTorchの対応GPU一覧にありません。互換レイヤーや非公式ビルドは使いません。何も取得していません。",
            device.name
        ));
    }
    if let Some(device) = devices
        .iter()
        .find(|device| device.vendor == GpuVendor::Intel)
    {
        return BackendSelection::Unsupported(format!(
            "Intel GPU（{}）向けの画像・動画・キャラクター生成は未対応です。ローカルAIのIntel GPU利用はこれまでどおり使えます。何も取得していません。",
            device.name
        ));
    }
    BackendSelection::Unsupported(format!(
        "GPU（{}）の種類を判別できません。対応GPUを確認できないため何も取得していません。",
        devices
            .iter()
            .map(|device| device.name.as_str())
            .collect::<Vec<_>>()
            .join("、")
    ))
}

/// ComfyUIを`--lowvram`で起動するかどうかと、その理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowVramDecision {
    pub lowvram: bool,
    pub reason: String,
}

/// 生成に使うGPUから`--lowvram`の要否を決める。
///
/// 専用VRAM16GB未満・共有メモリ・取得失敗はすべてlowvram（安全側）。
pub fn lowvram_decision(device: Option<&GpuDevice>) -> LowVramDecision {
    match device.map(|device| device.memory) {
        Some(GpuMemory::Dedicated(bytes)) if bytes >= LOWVRAM_THRESHOLD_BYTES => LowVramDecision {
            lowvram: false,
            reason: format!(
                "専用VRAM{}GBのため通常起動します。",
                decimal_gb_label(bytes)
            ),
        },
        Some(GpuMemory::Dedicated(bytes)) => LowVramDecision {
            lowvram: true,
            reason: format!(
                "専用VRAMが{}GB（16GB未満）のため、省メモリ設定（lowvram）で起動します。",
                decimal_gb_label(bytes)
            ),
        },
        Some(GpuMemory::Shared) => LowVramDecision {
            lowvram: true,
            reason: "専用VRAMを持たずシステムメモリを共有するGPUのため、省メモリ設定（lowvram）で起動します。".into(),
        },
        _ => LowVramDecision {
            lowvram: true,
            reason: "専用VRAM量を確認できないため、安全側の省メモリ設定（lowvram）で起動します。"
                .into(),
        },
    }
}

/// 動画モデルの取得に必要な専用VRAM（Wan 2.2 TI2V 5B／LTX 2Bの8GB級を想定）。
const VIDEO_MODEL_MIN_VRAM_BYTES: u64 = 7_500_000_000;
/// 共有メモリのGPUで動画モデルを取得してよい搭載RAMの下限。
///
/// 共有メモリ機はVRAMをシステムRAMから切り出すため、専用VRAM量では判定できない。
/// 8GB級のVRAMを確保しても本体側が足りる構成に限る。
const VIDEO_MODEL_MIN_SHARED_RAM_BYTES: u64 = 32_000_000_000;

/// 動画モデルを取得してよいメモリがあるか。足りない場合は日本語の理由を返す。
///
/// **共有メモリを専用VRAMへ加算しない。** 判定できない場合は取得しない。
pub fn video_memory_shortage(device: &GpuDevice, total_ram_bytes: Option<u64>) -> Option<String> {
    match device.memory {
        GpuMemory::Dedicated(bytes) if bytes >= VIDEO_MODEL_MIN_VRAM_BYTES => None,
        GpuMemory::Dedicated(bytes) => Some(format!(
            "動画モデルには専用VRAM 8GB以上が必要です（このGPUは{}GB）。動画モデルは取得していません。",
            decimal_gb_label(bytes)
        )),
        GpuMemory::Shared => match total_ram_bytes {
            Some(ram) if ram >= VIDEO_MODEL_MIN_SHARED_RAM_BYTES => None,
            Some(ram) => Some(format!(
                "システムメモリを共有するGPUで動画モデルを使うには搭載メモリ32GB以上が必要です（このPCは{}GB）。動画モデルは取得していません。",
                decimal_gb_label(ram)
            )),
            None => Some(
                "システムメモリを共有するGPUですが、搭載メモリ量を確認できません。動画モデルは取得していません。".into(),
            ),
        },
        GpuMemory::Unknown => Some(
            "GPUの専用VRAM量を確認できないため、動画モデルは取得していません。".into(),
        ),
    }
}

/// 十進GBの表示（小数第1位まで）。
fn decimal_gb_label(bytes: u64) -> String {
    let value = ((bytes as f64 / 1_000_000_000.0) * 10.0).round() / 10.0;
    format!("{value}")
}

/// そのバックエンドの依存ロックファイル。無ければ日本語の理由を返し、取得を始めさせない。
///
/// ROCm版wheelの固定情報（公式URLとSHA-256）はロックファイルが持つ。推測のハッシュを
/// 置かないため、ロックが無い版ではAMD経路を止める。
pub fn lock_path(
    assets: &std::path::Path,
    backend: GenerationBackend,
) -> Result<std::path::PathBuf, String> {
    let path = assets.join("sidecar").join(backend.lock_file_name());
    if path.is_file() {
        return Ok(path);
    }
    Err(format!(
        "{}版PyTorchの固定情報（{}）がこの版に含まれていません。何も取得していません。",
        backend.label(),
        backend.lock_file_name()
    ))
}

/// サイドカーがGPUメモリ不足で終了したときの終了コード。
///
/// 再試行の可否はRust側だけで判断する。文言でOOMを推測すると、別の失敗まで
/// 再試行してしまう。サイドカーは「OOMだった」ことだけをこのコードで返す。
pub const GPU_OOM_EXIT_CODE: i32 = 3;

/// `--lowvram`を付けて1回だけやり直すかどうか。
///
/// 通常起動のOOMだけが対象。lowvram中のOOM、OOM以外の失敗、終了コードを取得できない
/// 場合は再試行しない（無限再試行を作らない）。
pub fn should_retry_with_lowvram(exit_code: Option<i32>, lowvram_used: bool) -> bool {
    !lowvram_used && exit_code == Some(GPU_OOM_EXIT_CODE)
}

/// 生成に使うGPU。導入済みランタイムのベンダーに一致するものを選ぶ。
///
/// 生成のたびにnvidia-smiを起動しないため、記録済みベンダーとの一致だけで選ぶ。
/// 記録が無い場合は先頭のGPUを使い、GPUが1枚も見つからなければ`None`。
pub fn generation_device(
    devices: &[GpuDevice],
    installed: Option<&InstalledRuntime>,
) -> Option<GpuDevice> {
    if let Some(installed) = installed {
        if let Some(device) = devices
            .iter()
            .find(|device| device.vendor.label() == installed.gpu_vendor)
        {
            return Some(device.clone());
        }
    }
    devices.first().cloned()
}

/// lowvram起動・OOM後の再試行を、利用者に分かる日本語で進捗欄へ出す。
pub fn notify_progress(title: &str, body: &str) {
    use tauri::Emitter;

    let Some(app) = crate::durable_jobs::app_handle() else {
        return;
    };
    let payload = serde_json::json!({
        "kind": "generation",
        "ok": true,
        "title": title,
        "body": body,
        "at": chrono::Local::now().to_rfc3339(),
    });
    if let Err(error) = app.emit("pet:notice", payload) {
        crate::log_paths::append_app_log(
            app,
            &format!("生成の進捗を画面へ通知できません: {error}"),
        );
    }
}

/// 導入済みランタイムの素性。GPU交換・ドライバー変更後に古いバックエンドを再利用しないための記録。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    pub gpu_vendor: String,
    pub gpu_name: String,
    pub backend: GenerationBackend,
    pub torch: String,
    pub accelerator: String,
}

impl InstalledRuntime {
    pub fn of(selected: &SelectedBackend) -> Self {
        Self {
            gpu_vendor: selected.device.vendor.label().to_string(),
            gpu_name: selected.device.name.clone(),
            backend: selected.backend,
            torch: selected.backend.torch_version().to_string(),
            accelerator: selected.backend.accelerator().to_string(),
        }
    }

    /// 記録したGPUベンダーがいまも存在するか。
    ///
    /// 導入時のGPUが取り外されている場合、そのバックエンドのPyTorchはもう動かない。
    /// 実行のたびにnvidia-smiを起動せずに済ませるため、判定はベンダーの一致だけで行う。
    pub fn matches_present_gpu(&self, devices: &[GpuDevice]) -> bool {
        devices
            .iter()
            .any(|device| device.vendor.label() == self.gpu_vendor)
    }

    /// いまのGPUでそのまま使い続けてよい導入かどうか。
    ///
    /// GPUの機種名までは一致を求めない（同じベンダーの同じバックエンドなら再導入は不要）。
    /// バックエンド・ベンダー・固定版のいずれかが食い違えば再導入させる。
    pub fn is_reusable_for(&self, selected: &SelectedBackend) -> bool {
        let expected = Self::of(selected);
        self.backend == expected.backend
            && self.gpu_vendor == expected.gpu_vendor
            && self.torch == expected.torch
            && self.accelerator == expected.accelerator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vendor: GpuVendor, name: &str, memory: GpuMemory) -> GpuDevice {
        GpuDevice {
            vendor,
            name: name.to_string(),
            memory,
        }
    }

    fn nvidia() -> GpuDevice {
        device(
            GpuVendor::Nvidia,
            "NVIDIA GeForce RTX 5060 Laptop GPU",
            GpuMemory::Dedicated(8_000_000_000),
        )
    }

    fn supported_amd() -> GpuDevice {
        device(
            GpuVendor::Amd,
            "AMD Radeon RX 9070 XT",
            GpuMemory::Dedicated(16_000_000_000),
        )
    }

    #[test]
    fn 対応nvidia_gpuではcudaだけを選ぶ() {
        let selection = select(&[nvidia()], true);
        let selected = selection.supported().expect("CUDAを選ぶ");
        assert_eq!(selected.backend, GenerationBackend::Cuda);
        assert_eq!(selected.backend.lock_file_name(), "requirements-comfy.lock");
    }

    #[test]
    fn 対応amd_gpuではrocmだけを選ぶ() {
        let selection = select(&[supported_amd()], false);
        let selected = selection.supported().expect("ROCmを選ぶ");
        assert_eq!(selected.backend, GenerationBackend::Rocm);
        assert_eq!(
            selected.backend.lock_file_name(),
            "requirements-comfy-rocm.lock"
        );
    }

    #[test]
    fn 非対応amd_gpuでは何も取得しない() {
        let selection = select(
            &[device(
                GpuVendor::Amd,
                "AMD Radeon RX 6600",
                GpuMemory::Dedicated(8_000_000_000),
            )],
            false,
        );
        let BackendSelection::Unsupported(reason) = selection else {
            panic!("非対応AMD GPUで取得を始めてはいけない");
        };
        assert!(reason.contains("何も取得していません"), "{reason}");
    }

    #[test]
    fn intel_gpuでは生成用アセットを取得しない() {
        let selection = select(
            &[device(
                GpuVendor::Intel,
                "Intel Arc A770 Graphics",
                GpuMemory::Dedicated(16_000_000_000),
            )],
            false,
        );
        let BackendSelection::Unsupported(reason) = selection else {
            panic!("Intel GPUで生成用アセットを取得してはいけない");
        };
        assert!(reason.contains("Intel"), "{reason}");
        assert!(reason.contains("何も取得していません"), "{reason}");
    }

    #[test]
    fn ローカルllmとpytorchで対応表を共有しない() {
        // llama.cppのROCmビルドが動くGPUと、PyTorch ROCmが公式対応するGPUは一致しない。
        // 片方の表を流用すると、非対応GPUで数十GBの取得を始めてしまう。
        let sources = crate::guard_scan::backend_sources();
        let text = |name: &str| {
            sources
                .iter()
                .find(|source| source.path.ends_with(name))
                .unwrap_or_else(|| panic!("{name}を読み込める"))
                .text
                .clone()
        };
        let engine = text("src-tauri/src/engine_backend.rs");
        let generation = text("src-tauri/src/generation_backend.rs");
        // このテスト自身も走査対象に入るため、定数名は分割して組み立てる。
        let runtime_table = format!("WINDOWS_ROCM_{}", "RUNTIME_SKUS");
        let pytorch_table = format!("WINDOWS_ROCM_{}", "PYTORCH_SKUS");
        assert!(!generation.contains(&runtime_table));
        assert!(!engine.contains(&pytorch_table));
        assert!(!engine.contains("generation_backend::"));
        // llama.cppのROCm対応SKUをPyTorch側が自動で対応扱いにしない。
        assert!(engine.contains("\"radeon rx 6600\""));
        assert!(!is_supported_amd_pytorch_name("AMD Radeon RX 6600"));
    }

    #[test]
    fn cuda側の固定版を変えない() {
        // 既存のCUDA経路（対応GPU・モデル・画質・手順）を、AMD対応で動かさない。
        assert_eq!(CUDA_TORCH_VERSION, "2.11.0+cu128");
        assert_eq!(CUDA_RELEASE, "cuda-12.8");
        let lock = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("リポジトリルート")
            .join("character-maker/sidecar/requirements-comfy.lock");
        let text = std::fs::read_to_string(lock).expect("CUDA固定情報を読み込める");
        for pinned in [
            "torch==2.11.0+cu128",
            "torchvision==0.26.0+cu128",
            "torchaudio==2.11.0+cu128",
        ] {
            assert!(text.contains(pinned), "{pinned}");
        }
    }

    #[test]
    fn gpuなしでは生成用アセットを取得しない() {
        assert!(matches!(
            select(&[], false),
            BackendSelection::Unsupported(_)
        ));
    }

    #[test]
    fn cuda対応として応答しないnvidiaではamd経路へ落ちない() {
        let selection = select(&[nvidia()], false);
        let BackendSelection::Unsupported(reason) = selection else {
            panic!("CUDAを確認できないNVIDIAで取得を始めてはいけない");
        };
        assert!(reason.contains("NVIDIA"), "{reason}");
    }

    #[test]
    fn cudaとrocmを同時に選ばない() {
        // 1回の判定で返るバックエンドは常に1つだけ。両対応を混在させない。
        for (devices, ready) in [
            (vec![nvidia(), supported_amd()], true),
            (vec![supported_amd(), nvidia()], true),
        ] {
            let selected = select(&devices, ready).supported().cloned().expect("選択");
            assert_eq!(selected.backend, GenerationBackend::Cuda);
        }
        let selected = select(&[supported_amd()], false)
            .supported()
            .cloned()
            .expect("選択");
        assert_eq!(selected.backend, GenerationBackend::Rocm);
    }

    #[test]
    fn gpu交換後に不一致バックエンドを再利用しない() {
        let cuda = SelectedBackend {
            backend: GenerationBackend::Cuda,
            device: nvidia(),
        };
        let rocm = SelectedBackend {
            backend: GenerationBackend::Rocm,
            device: supported_amd(),
        };
        let installed = InstalledRuntime::of(&cuda);
        assert!(installed.is_reusable_for(&cuda));
        assert!(!installed.is_reusable_for(&rocm));

        // 同じNVIDIAでも別機種へ載せ替えた場合は再導入不要（バックエンドが同じ）。
        let another_nvidia = SelectedBackend {
            backend: GenerationBackend::Cuda,
            device: device(
                GpuVendor::Nvidia,
                "NVIDIA GeForce RTX 5090",
                GpuMemory::Dedicated(32_000_000_000),
            ),
        };
        assert!(installed.is_reusable_for(&another_nvidia));

        // 固定版が変わった記録（旧版のまま）は再利用しない。
        let mut stale = InstalledRuntime::of(&cuda);
        stale.torch = "2.8.0+cu126".into();
        assert!(!stale.is_reusable_for(&cuda));
    }

    #[test]
    fn 通常起動のoomだけlowvramで1回再試行する() {
        assert!(should_retry_with_lowvram(Some(GPU_OOM_EXIT_CODE), false));
        // lowvram中のOOMでは再試行しない。
        assert!(!should_retry_with_lowvram(Some(GPU_OOM_EXIT_CODE), true));
        // OOM以外の失敗では再試行しない。
        assert!(!should_retry_with_lowvram(Some(1), false));
        assert!(!should_retry_with_lowvram(Some(0), false));
        // 終了コードを取得できない場合も再試行しない。
        assert!(!should_retry_with_lowvram(None, false));
    }

    #[test]
    fn 生成に使うgpuは導入済みバックエンドのベンダーに合わせる() {
        let devices = vec![
            device(
                GpuVendor::Intel,
                "Intel Iris Xe Graphics",
                GpuMemory::Shared,
            ),
            supported_amd(),
        ];
        let installed = InstalledRuntime::of(&SelectedBackend {
            backend: GenerationBackend::Rocm,
            device: supported_amd(),
        });
        assert_eq!(
            generation_device(&devices, Some(&installed)).map(|device| device.name),
            Some("AMD Radeon RX 9070 XT".to_string())
        );
        assert_eq!(generation_device(&[], Some(&installed)), None);
    }

    #[test]
    fn 専用vramが16gb未満ならlowvramにする() {
        let decision = lowvram_decision(Some(&device(
            GpuVendor::Nvidia,
            "NVIDIA GeForce RTX 5060 Laptop GPU",
            GpuMemory::Dedicated(15_999_999_999),
        )));
        assert!(decision.lowvram);
        assert!(decision.reason.contains("lowvram"), "{}", decision.reason);
    }

    #[test]
    fn 専用vramが16gb以上なら通常起動にする() {
        let decision = lowvram_decision(Some(&device(
            GpuVendor::Amd,
            "AMD Radeon RX 9070 XT",
            GpuMemory::Dedicated(LOWVRAM_THRESHOLD_BYTES),
        )));
        assert!(!decision.lowvram);
    }

    #[test]
    fn 共有メモリならlowvramにする() {
        let decision = lowvram_decision(Some(&device(
            GpuVendor::Amd,
            "AMD Radeon 8060S Graphics",
            GpuMemory::Shared,
        )));
        assert!(decision.lowvram);
    }

    #[test]
    fn vram取得失敗ならlowvramにする() {
        assert!(
            lowvram_decision(Some(&device(
                GpuVendor::Amd,
                "AMD Radeon RX 9070 XT",
                GpuMemory::Unknown
            )))
            .lowvram
        );
        assert!(lowvram_decision(None).lowvram);
    }

    #[test]
    fn 境界の16gbを十進で判定する() {
        // 16GiB（17_179_869_184）ではなく十進16GBを境界にする。
        assert_eq!(LOWVRAM_THRESHOLD_BYTES, 16_000_000_000);
        assert!(
            !lowvram_decision(Some(&device(
                GpuVendor::Nvidia,
                "NVIDIA GeForce RTX 5080",
                GpuMemory::Dedicated(16_000_000_000)
            )))
            .lowvram
        );
    }

    #[test]
    fn 固定情報の無いバックエンドでは取得を開始しない() {
        let temporary = std::env::temp_dir().join(format!(
            "picoagent-backend-lock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let sidecar = temporary.join("sidecar");
        std::fs::create_dir_all(&sidecar).expect("検査用ディレクトリ");
        std::fs::write(sidecar.join("requirements-comfy.lock"), b"").expect("CUDA固定情報");
        assert!(lock_path(&temporary, GenerationBackend::Cuda).is_ok());
        let error = lock_path(&temporary, GenerationBackend::Rocm).expect_err("ROCm固定情報なし");
        assert!(error.contains("何も取得していません"), "{error}");
        std::fs::remove_dir_all(&temporary).expect("検査用ディレクトリの回収");
    }

    #[test]
    fn 実物のcuda固定情報を同梱している() {
        // 既存のCUDA経路が固定情報を失っていないことを実ファイルで確かめる。
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("リポジトリルート")
            .join("character-maker");
        assert!(lock_path(&assets, GenerationBackend::Cuda).is_ok());
        assert!(assets.join("sidecar/verify_backend.py").is_file());
        assert!(assets.join("sidecar/requirements-comfy-rocm.in").is_file());
    }

    #[test]
    fn cuda専用インデックスをcuda経路の中だけで渡す() {
        // ROCm経路へPyTorchのCUDAインデックスを渡すと、PyPIのCPU版torchで
        // ROCm版が上書きされる。分岐の中にあることを機械検査で固定する。
        let source = &crate::guard_scan::backend_sources()
            .iter()
            .find(|source| source.path.ends_with("src-tauri/src/components.rs"))
            .expect("components.rsを読み込める")
            .text;
        let index = "https://download.pytorch.org/whl/cu128";
        let position = source.find(index).expect("CUDAインデックスの指定");
        let before = &source[position.saturating_sub(600)..position];
        assert!(
            before.contains("GenerationBackend::Cuda"),
            "CUDAインデックスはCUDA経路の分岐内だけで渡す"
        );
        // ロックファイルはバックエンドから選ぶ（どちらか片方だけを導入する）。
        assert!(source.contains("crate::generation_backend::lock_path"));
        assert!(!source.contains("requirements-comfy.lock"));
        assert!(!source.contains("requirements-comfy-rocm.lock"));
    }

    #[test]
    fn amd対応表を文字列の部分一致で広げない() {
        assert!(is_supported_amd_pytorch_name("AMD Radeon RX 9070 XT"));
        assert!(is_supported_amd_pytorch_name("Radeon RX 7900 XTX"));
        assert!(!is_supported_amd_pytorch_name(
            "AMD Radeon RX 7900 XTX Ultra"
        ));
        assert!(!is_supported_amd_pytorch_name("AMD Radeon Graphics"));
        assert!(!is_supported_amd_pytorch_name("Radeon"));
    }
}
