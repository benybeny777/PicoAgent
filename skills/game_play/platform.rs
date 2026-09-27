// OS依存の対象確認・撮影・単一入力。前面化や任意キー送信は公開しない。
use std::sync::atomic::AtomicBool;
#[cfg(windows)]
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Target {
    id: u32,
    pid: u32,
    title: String,
    rect: (i32, i32, u32, u32),
}

#[cfg(windows)]
fn identity(window: &xcap::Window) -> Result<Target, String> {
    Ok(Target {
        id: window.id().map_err(|e| e.to_string())?,
        pid: window.pid().map_err(|e| e.to_string())?,
        title: window.title().map_err(|e| e.to_string())?,
        rect: (
            window.x().map_err(|e| e.to_string())?,
            window.y().map_err(|e| e.to_string())?,
            window.width().map_err(|e| e.to_string())?,
            window.height().map_err(|e| e.to_string())?,
        ),
    })
}

#[cfg(windows)]
fn checked(target: &Target) -> Result<xcap::Window, String> {
    let candidates =
        xcap::Window::all().map_err(|e| format!("ウィンドウ一覧を取得できません: {e}"))?;
    for window in candidates {
        if window.id().map_err(|e| e.to_string())? == target.id {
            if identity(&window)? != *target
                || !window.is_focused().map_err(|e| e.to_string())?
                || window.is_minimized().map_err(|e| e.to_string())?
            {
                return Err("対象ゲームが前面にないか、ウィンドウの位置・大きさ・実体が変わったため停止しました。".into());
            }
            return Ok(window);
        }
    }
    Err("対象ゲームのウィンドウが閉じられたため停止しました。".into())
}

#[cfg(windows)]
pub(super) fn windows() -> Result<String, String> {
    super::super::desktop_control::window::list()
        .map(|list| super::super::desktop_control::window::format(&list))
}

#[cfg(windows)]
pub(super) fn select(title: &str) -> Result<Target, String> {
    let mut found = Vec::new();
    for window in xcap::Window::all().map_err(|e| e.to_string())? {
        if window.title().map_err(|e| e.to_string())? == title {
            found.push(identity(&window)?);
        }
    }
    if found.len() != 1 {
        return Err("設定と完全一致するゲームウィンドウが1つだけ必要です。ゲームを起動し、ウィンドウ名を確認してください。".into());
    }
    let target = found.remove(0);
    checked(&target)?;
    Ok(target)
}

#[cfg(windows)]
pub(super) fn capture(target: &Target) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    let window = checked(target)?;
    if u64::from(target.rect.2) * u64::from(target.rect.3) > 8_500_000 {
        return Err("撮影対象が大きすぎます。ゲームの表示解像度を下げてください。".into());
    }
    let image = window
        .capture_image()
        .map_err(|e| format!("ゲーム画面を撮影できません: {e}"))?;
    checked(target)?;
    if image.width() < 64
        || image.height() < 64
        || image.pixels().all(|p| p[0] < 4 && p[1] < 4 && p[2] < 4)
    {
        return Err(
            "ゲーム画面が空または黒いため停止しました。ウィンドウ表示を確認してください。".into(),
        );
    }
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, xcap::image::ImageFormat::Png)
        .map_err(|e| format!("画像を保存形式へ変換できません: {e}"))?;
    Ok(bytes.into_inner())
}

#[cfg(windows)]
pub(super) fn press(target: &Target, key: &str, cancel: &AtomicBool) -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }
    // 修飾キーや操作キーを利用者が押している間は入力を混ぜない。
    let held_keys = [
        0x10, 0x11, 0x12, 0x5B, 0x5C, 0x0D, 0x20, 0x25, 0x26, 0x27, 0x28, 0x41, 0x43, 0x44, 0x53,
        0x46, 0x57, 0x58, 0x5A,
    ];
    if held_keys
        .iter()
        .any(|vk| unsafe { GetAsyncKeyState(*vk) } < 0)
    {
        return Err("キーが押されているためゲーム操作を停止しました。キーボードから手を離して再度依頼してください。".into());
    }
    let key = match key {
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "enter" => Key::Return,
        "space" => Key::Space,
        "z" => Key::Unicode('z'),
        "x" => Key::Unicode('x'),
        "c" => Key::Unicode('c'),
        "a" => Key::Unicode('a'),
        "s" => Key::Unicode('s'),
        "d" => Key::Unicode('d'),
        "w" => Key::Unicode('w'),
        "f" => Key::Unicode('f'),
        _ => return Err("許可されていないキーです。".into()),
    };
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("入力を準備できません: {e}"))?;
    checked(target)?;
    if cancel.load(Ordering::SeqCst) {
        return Err("入力前に停止しました。".into());
    }
    // 同期区間内で必ず押下と解放を試す。入力途中にFutureを破棄されても押しっぱなしにしない。
    let pressed = enigo.key(key, Direction::Press);
    // ゲームのフレーム単位の入力読取りに届く短時間だけ保持する。解放までawaitを挟まない。
    std::thread::sleep(std::time::Duration::from_millis(40));
    let released = enigo.key(key, Direction::Release);
    pressed.map_err(|e| format!("キー入力の成否が不明です。再送せず停止します: {e}"))?;
    released.map_err(|e| format!("キーを解放できません。ゲームを停止して確認してください: {e}"))?;
    checked(target)?;
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn windows() -> Result<String, String> {
    Err("ゲームプレイは現在Windows専用です。".into())
}
#[cfg(not(windows))]
pub(super) fn select(_title: &str) -> Result<Target, String> {
    Err("ゲームプレイは現在Windows専用です。".into())
}
#[cfg(not(windows))]
pub(super) fn capture(_target: &Target) -> Result<Vec<u8>, String> {
    Err("ゲームプレイは現在Windows専用です。".into())
}
#[cfg(not(windows))]
pub(super) fn press(_target: &Target, _key: &str, _cancel: &AtomicBool) -> Result<(), String> {
    Err("ゲームプレイは現在Windows専用です。".into())
}
