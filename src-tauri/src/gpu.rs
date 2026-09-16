// gpu.rs - GPU情報（ベンダー・型番・専用VRAM・共有メモリ・ユニファイドメモリ）の正本。
//
// 表示アダプタ名だけを見る判定は、Intel GPUをAMD扱いにしたり、共有メモリのAPUを
// 専用VRAM搭載機と誤認したりする。取得開始前に必要な事実をここで1度だけ集め、
// llama.cpp（`engine_backend.rs`）とPyTorch／ComfyUI（`generation_backend.rs`）が
// **それぞれの対応表**で判断する。対応表は共有しない。同じGPUでも、llama.cppの
// ROCmビルドが動く範囲とPyTorch ROCmが公式対応する範囲は一致しないためである。

/// GPUのベンダー。PCIベンダーIDで判定し、表示名の文字列一致には頼らない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    /// Apple Silicon の統合GPU（ユニファイドメモリ）。
    Apple,
    /// 判別できないベンダー（仮想ディスプレイ、リモート表示など）。
    Other,
}

impl GpuVendor {
    /// PCIベンダーID（`MatchingDeviceId`の`ven_XXXX`）から判定する。
    #[cfg_attr(not(windows), allow(dead_code))]
    fn from_pci_id(id: u16) -> Self {
        match id {
            0x10de => Self::Nvidia,
            // 0x1002はAMD（旧ATI）、0x1022はAMDのAPU側で使われる。
            0x1002 | 0x1022 => Self::Amd,
            0x8086 => Self::Intel,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nvidia => "NVIDIA",
            Self::Amd => "AMD",
            Self::Intel => "Intel",
            Self::Apple => "Apple",
            Self::Other => "不明なベンダー",
        }
    }
}

/// GPUが使うメモリの素性。**共有メモリとユニファイドメモリを専用VRAMへ加算しない。**
///
/// 生成対象はWindowsとApple Silicon Macだけなので、他OSのビルドでは値を作る側が無い。
/// 判定ロジックはOSに依らずテストするため、型は隠さず警告だけ止める。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuMemory {
    /// 専用VRAM（バイト）。
    Dedicated(u64),
    /// APUなど、システムRAMを共有するGPU（搭載量は別途判定する）。
    Shared,
    /// Apple Siliconのユニファイドメモリ（バイト）。CPUとGPUで同じ実体を使う。
    ///
    /// **専用VRAMの判定を流用しない。** 16GBの専用VRAMと16GBのユニファイドメモリは
    /// 同じ意味を持たない（後者はOSとアプリの作業領域を含む総量である）。
    Unified(u64),
    /// 正確に取得できなかった。推測で埋めない。
    Unknown,
}

/// 1枚のGPU。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuDevice {
    pub vendor: GpuVendor,
    /// 表示アダプタ名（例: `AMD Radeon RX 7900 XTX`）。
    pub name: String,
    pub memory: GpuMemory,
}

/// 専用VRAMが実質存在しない機種の上限。これ以下は共有メモリのAPUとして扱う。
///
/// APUはUEFIが確保する小さなフレームバッファ（64MB〜512MB程度）を専用VRAMとして
/// 報告する。これを専用VRAMとして信じると、実際には共有メモリで動く機種を
/// 「専用VRAMが極端に少ないGPU」と誤認する。どちらでも安全側（lowvram）になるが、
/// 利用者へ出す理由が変わるため区別する。
#[cfg_attr(not(windows), allow(dead_code))]
const SHARED_MEMORY_MAX_BYTES: u64 = 1_000_000_000;

/// 表示アダプタ名の一覧。llama.cppのバックエンド判定（`engine_backend.rs`）が使う。
///
/// 既存のローカルLLM経路の判定材料を変えないため、取得元（`EnumDisplayDevices`）は
/// 変更しない。詳細情報が要る場合は`devices()`を使う。
#[cfg(windows)]
pub fn adapter_names() -> Vec<String> {
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};

    let mut names = Vec::new();
    for index in 0..16u32 {
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // 安全性: cbを正しく設定した固定長バッファだけをOS APIへ渡す。
        let ok = unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) };
        if ok == 0 {
            break;
        }
        let end = device
            .DeviceString
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(device.DeviceString.len());
        let name = String::from_utf16_lossy(&device.DeviceString[..end])
            .trim()
            .to_string();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// いま搭載されているGPUの一覧。取得できない環境では空を返す。
///
/// Windowsでは次の2つを突き合わせる。
///
/// - `EnumDisplayDevices`: いま存在する表示アダプタ名。
/// - 表示アダプタのクラスキー: ベンダーIDと64bitの専用VRAM量。
///
/// レジストリ側には取り外し済みGPUの項目が残るため、**いま存在する名前と一致した
/// 項目だけ**を採用する。`Win32_VideoController.AdapterRAM`はuint32で4GB超が飽和
/// するため使わない。
#[cfg(windows)]
pub fn devices() -> Vec<GpuDevice> {
    let present = adapter_names();
    let mut devices = Vec::new();
    for entry in display_class_entries() {
        if !present.iter().any(|name| name == &entry.name) {
            continue;
        }
        if devices
            .iter()
            .any(|device: &GpuDevice| device.name == entry.name)
        {
            continue;
        }
        devices.push(GpuDevice {
            vendor: entry.vendor,
            name: entry.name,
            memory: entry.memory,
        });
    }
    // レジストリから素性を取れなかったアダプタも、存在自体は隠さずUnknownで残す。
    for name in present {
        if !devices.iter().any(|device| device.name == name) {
            devices.push(GpuDevice {
                vendor: GpuVendor::Other,
                name,
                memory: GpuMemory::Unknown,
            });
        }
    }
    devices
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn adapter_names() -> Vec<String> {
    Vec::new()
}

/// Apple Silicon Macの統合GPU。ユニファイドメモリの総量をそのまま持つ。
///
/// Intel Macは対象外なので、`aarch64`のときだけGPUを名乗る。GPU名はチップ名
/// （`machdep.cpu.brand_string`。例: `Apple M3 Pro`）を使い、取得できない場合も
/// Apple Siliconであること自体は確定しているため総称で返す。
#[cfg(target_os = "macos")]
pub fn devices() -> Vec<GpuDevice> {
    if !cfg!(target_arch = "aarch64") {
        // Intel Macは生成機能の対象外。推測でGPUを名乗らない。
        return Vec::new();
    }
    let memory = crate::engine::total_memory_bytes()
        .map(GpuMemory::Unified)
        .unwrap_or(GpuMemory::Unknown);
    vec![GpuDevice {
        vendor: GpuVendor::Apple,
        name: apple_chip_name().unwrap_or_else(|| "Apple Silicon".to_string()),
        memory,
    }]
}

#[cfg(target_os = "macos")]
fn apple_chip_name() -> Option<String> {
    let mut size = 0usize;
    let name = c"machdep.cpu.brand_string";
    // 安全性: 1回目は長さ取得だけを行い、2回目はその長さのバッファへ書かせる。
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || size == 0
        || size > 256
    {
        return None;
    }
    let mut buffer = vec![0u8; size];
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(buffer.len());
    let text = String::from_utf8_lossy(&buffer[..end]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn devices() -> Vec<GpuDevice> {
    // Windows・Apple Silicon Mac以外は生成機能の対象外。推測でGPUを名乗らない。
    Vec::new()
}

#[cfg(windows)]
struct DisplayClassEntry {
    vendor: GpuVendor,
    name: String,
    memory: GpuMemory,
}

/// 表示アダプタのクラスキーを順に読む。
///
/// `HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\NNNN`
/// の各項目から`DriverDesc`（名前）、`MatchingDeviceId`（ベンダーID）、
/// `HardwareInformation.qwMemorySize`（専用VRAM）を読む。
#[cfg(windows)]
fn display_class_entries() -> Vec<DisplayClassEntry> {
    const DISPLAY_CLASS: &str =
        r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let mut entries = Vec::new();
    // 表示アダプタの項目は0000から連番で並ぶ。上限は現実的な搭載枚数に収める。
    for index in 0..32u32 {
        let key = format!("{DISPLAY_CLASS}\\{index:04}");
        let Some(name) = registry_string(&key, "DriverDesc") else {
            continue;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let vendor = registry_string(&key, "MatchingDeviceId")
            .and_then(|id| pci_vendor_id(&id))
            .map(GpuVendor::from_pci_id)
            .unwrap_or(GpuVendor::Other);
        let memory = match registry_qword(&key, "HardwareInformation.qwMemorySize") {
            Some(bytes) if bytes > SHARED_MEMORY_MAX_BYTES => GpuMemory::Dedicated(bytes),
            // 専用VRAMを持たない、または極小のフレームバッファしか持たない機種。
            Some(_) => GpuMemory::Shared,
            None => GpuMemory::Unknown,
        };
        entries.push(DisplayClassEntry {
            vendor,
            name,
            memory,
        });
    }
    entries
}

/// `pci\ven_1002&dev_744c&...` からベンダーIDを取り出す。
#[cfg_attr(not(windows), allow(dead_code))]
fn pci_vendor_id(matching_device_id: &str) -> Option<u16> {
    let lowered = matching_device_id.to_ascii_lowercase();
    let start = lowered.find("ven_")? + "ven_".len();
    let digits: String = lowered[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    u16::from_str_radix(&digits, 16).ok()
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `HKEY_LOCAL_MACHINE`配下の文字列値を読む。読めなければ`None`。
#[cfg(windows)]
fn registry_string(subkey: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let subkey = wide(subkey);
    let value = wide(value);
    let mut bytes: u32 = 0;
    // 安全性: 終端付きUTF-16と、長さを渡した固定長バッファだけをOS APIへ渡す。
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS || bytes == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// `HKEY_LOCAL_MACHINE`配下の64bit数値を読む。REG_QWORDとREG_BINARY(8バイト)に対応する。
#[cfg(windows)]
fn registry_qword(subkey: &str, value: &str) -> Option<u64> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RRF_RT_REG_QWORD,
    };

    let subkey = wide(subkey);
    let value = wide(value);
    let mut buffer = [0u8; 8];
    let mut size = buffer.len() as u32;
    // 安全性: 8バイト固定のバッファと、その長さだけをOS APIへ渡す。
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_QWORD | RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size as usize != buffer.len() {
        return None;
    }
    Some(u64::from_le_bytes(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pciベンダーidでベンダーを判定する() {
        assert_eq!(GpuVendor::from_pci_id(0x10de), GpuVendor::Nvidia);
        assert_eq!(GpuVendor::from_pci_id(0x1002), GpuVendor::Amd);
        assert_eq!(GpuVendor::from_pci_id(0x1022), GpuVendor::Amd);
        assert_eq!(GpuVendor::from_pci_id(0x8086), GpuVendor::Intel);
        assert_eq!(GpuVendor::from_pci_id(0x1414), GpuVendor::Other);
    }

    #[test]
    fn matching_device_idからベンダーidを取り出す() {
        assert_eq!(pci_vendor_id(r"pci\ven_1002&dev_744c"), Some(0x1002));
        assert_eq!(
            pci_vendor_id(r"PCI\VEN_10DE&DEV_2504&SUBSYS_0"),
            Some(0x10de)
        );
        assert_eq!(pci_vendor_id(r"root\basicdisplay"), None);
    }

    #[test]
    fn 共有メモリと不明を専用vram量として持たない() {
        // 共有メモリ・不明はバイト数を持たない。専用VRAMへ足し込む経路を作らない。
        assert!(
            matches!(GpuMemory::Dedicated(16_000_000_000), GpuMemory::Dedicated(bytes) if bytes == 16_000_000_000)
        );
        assert!(!matches!(GpuMemory::Shared, GpuMemory::Dedicated(_)));
        assert!(!matches!(GpuMemory::Unknown, GpuMemory::Dedicated(_)));
    }
}
