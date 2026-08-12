// skills/desktop_control/window.rs - ウィンドウの一覧取得と前面化。
//
// 座標だけで操作させると、モデルは「どのウィンドウを触っているか」を見失いやすい。
// 先に対象ウィンドウをOSハンドル由来の識別子で選ばせてから座標を扱う方が、誤爆がはっきり減る
// （`browser_control` で要素へ番号を振ったのと同じ考え方）。
//
// Windows は `EnumWindows` 系で COM を使わずに実装できる。macOS は Accessibility の
// 許可と AX API が必要で、この環境では動作確認ができないため未対応として明示する。

/// 画面上のウィンドウ1つ分。
pub struct WindowInfo {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// OS 内部のハンドル。モデルには文字列の不透明な識別子としてだけ見せる。
    pub handle: isize,
}

/// HWNDは破棄後に再利用され得るため、ハンドルだけでなく一覧取得時のタイトルと矩形も
/// 識別子へ含める。別ウィンドウへ再利用された場合は同じIDにならない。
pub fn identifier(window: &WindowInfo) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    window.title.hash(&mut hasher);
    window.x.hash(&mut hasher);
    window.y.hash(&mut hasher);
    window.width.hash(&mut hasher);
    window.height.hash(&mut hasher);
    format!("{}-{:016x}", window.handle, hasher.finish())
}

/// 表示中のトップレベルウィンドウを、画面に見えている順で返す。
#[cfg(windows)]
pub fn list() -> Result<Vec<WindowInfo>, String> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    };

    unsafe extern "system" fn collect(handle: HWND, param: LPARAM) -> BOOL {
        // 安全性: param には呼び出し側で確保した Vec<WindowInfo> の生ポインタだけを渡す。
        let windows = unsafe { &mut *(param as *mut Vec<WindowInfo>) };
        if unsafe { IsWindowVisible(handle) } == 0 {
            return TRUE;
        }
        let length = unsafe { GetWindowTextLengthW(handle) };
        if length <= 0 {
            return TRUE; // タイトルの無いウィンドウは対象にしない（作業用の不可視窓が多い）
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = unsafe { GetWindowTextW(handle, buffer.as_mut_ptr(), buffer.len() as i32) };
        if written <= 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&buffer[..written as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            return TRUE;
        }
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if unsafe { GetWindowRect(handle, &mut rect) } == 0 {
            return TRUE;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return TRUE; // 最小化などで実体の無いものは省く
        }
        windows.push(WindowInfo {
            title,
            x: rect.left,
            y: rect.top,
            width,
            height,
            handle: handle as isize,
        });
        TRUE
    }

    let mut windows: Vec<WindowInfo> = Vec::new();
    // 安全性: コールバックは同じスレッドで同期に呼ばれ、ポインタは EnumWindows の間だけ有効。
    let ok = unsafe {
        EnumWindows(
            Some(collect),
            &mut windows as *mut Vec<WindowInfo> as isize,
        )
    };
    if ok == 0 && windows.is_empty() {
        return Err("エラー: ウィンドウ一覧を取得できませんでした。".to_string());
    }
    Ok(windows)
}

/// 指定したウィンドウを前面に出す。
#[cfg(windows)]
pub fn focus(handle: isize) -> Result<(), String> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let handle = handle as HWND;
    // 安全性: list() が返した生存中のハンドルだけを受け取る。
    unsafe {
        if IsIconic(handle) != 0 {
            ShowWindow(handle, SW_RESTORE);
        }
        if SetForegroundWindow(handle) == 0 {
            return Err(
                "エラー: ウィンドウを前面にできませんでした（別のアプリが前面を保持している可能性があります）。"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// macOS は Accessibility の許可と AX API が必要で、まだ実装していない。
/// できないことを黙って成功させず、はっきり伝える。
#[cfg(not(windows))]
pub fn list() -> Result<Vec<WindowInfo>, String> {
    Err("エラー: ウィンドウ一覧は現在 Windows でのみ利用できます。".to_string())
}

#[cfg(not(windows))]
pub fn focus(_handle: isize) -> Result<(), String> {
    Err("エラー: ウィンドウの前面化は現在 Windows でのみ利用できます。".to_string())
}

/// 一覧を安定識別子付きの短い行へ整形する。
pub fn format(windows: &[WindowInfo]) -> String {
    if windows.is_empty() {
        return "表示中のウィンドウは見つかりませんでした。".to_string();
    }
    let mut out = String::from("表示中のウィンドウ:\n");
    for window in windows {
        out.push_str(&format!(
            "  id={} {} （{},{} {}x{}）\n",
            identifier(window),
            window.title,
            window.x,
            window.y,
            window.width,
            window.height
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{format, identifier, WindowInfo};

    fn sample(handle: isize, title: &str) -> WindowInfo {
        WindowInfo { title: title.to_string(), x: 10, y: 20, width: 800, height: 600, handle }
    }

    #[test]
    fn 一覧は識別子と位置を並べる() {
        let text = format(&[sample(101, "メモ帳"), sample(202, "ブラウザ")]);
        assert!(
            text.contains(&format!("id={} メモ帳 （10,20 800x600）", identifier(&sample(101, "メモ帳")))),
            "text: {text}"
        );
        assert!(
            text.contains(&format!("id={} ブラウザ", identifier(&sample(202, "ブラウザ")))),
            "text: {text}"
        );
    }

    #[test]
    fn 空なら見つからないと伝える() {
        assert!(format(&[]).contains("見つかりませんでした"));
    }
}
