// gpu.rs - OS APIで表示アダプタ名を取得する共通層。

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
