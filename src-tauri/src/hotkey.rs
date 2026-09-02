// hotkey.rs - グローバルホットキー（他のアプリを使っている最中でも PicoAgent を呼び出す）。
//
// デスクトップペットは常に手前に居るが、全画面のエディタやブラウザの裏へ回ると
// クリックで呼び出せない。キーボードから直接「呼び出して入力欄を開く」「隠す／戻す」
// を行えるようにして、作業を中断せずに話しかけられるようにする。

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::commands::AppState;

/// 呼び出し（表示・前面化・入力欄を開く）の既定キー。
pub const SUMMON_DEFAULT: &str = "CommandOrControl+Alt+P";
/// 隠す／戻すの既定キー。
pub const HIDE_DEFAULT: &str = "CommandOrControl+Alt+H";
/// そのホットキーだけを無効にする指定値。空欄は他の設定欄と同じく「既定値を使う」意味に
/// なるため（`config.get` が既定へフォールバックする）、無効化は明示の値で表す。
pub const DISABLED: &str = "off";

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegistrationSettings {
    enabled: bool,
    summon: String,
    hide: String,
}

static APPLIED_SETTINGS: Mutex<Option<RegistrationSettings>> = Mutex::new(None);

fn needs_registration_refresh(
    applied: Option<&RegistrationSettings>,
    desired: &RegistrationSettings,
) -> bool {
    applied != Some(desired)
}

/// ホットキーで行う動作。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// 表示して前面へ出し、入力欄を開く。
    Summon,
    /// 表示中なら隠し、隠れていれば呼び出しと同じ扱いで戻す。
    ToggleHide,
}

/// 設定を読み直してホットキーを登録し直す（起動時と設定保存時に呼ぶ）。
///
/// 登録できなかったキーは `Err` にまとめて返す。呼び出し側がログや設定画面へ
/// 出すことで、「設定したのに効かない」を黙って放置しない。
pub fn apply(app: &AppHandle) -> Result<(), String> {
    // 設定値は先に取り出す（`State` の借用をホットキー登録の間まで持ち越さない）。
    let desired = {
        let state = app.state::<AppState>();
        RegistrationSettings {
            enabled: state.config.get_bool("PICOAGENT_HOTKEY_ENABLED", true),
            summon: state
                .config
                .get("PICOAGENT_HOTKEY_SUMMON", SUMMON_DEFAULT)
                .trim()
                .to_string(),
            hide: state
                .config
                .get("PICOAGENT_HOTKEY_HIDE", HIDE_DEFAULT)
                .trim()
                .to_string(),
        }
    };

    // 設定画面はホットキー以外の保存でも apply を呼ぶ。同じキーを一度解除して直後に
    // 張り直すと、Windows側の解除反映前に「HotKey already registered」になることがある。
    // 実際の指定が変わらない限り、正常に保持中の登録を触らない。
    let mut applied = APPLIED_SETTINGS
        .lock()
        .map_err(|_| "ホットキー登録状態のロックが壊れています".to_string())?;
    if !needs_registration_refresh(applied.as_ref(), &desired) {
        return Ok(());
    }

    let manager = app.global_shortcut();
    // 前回の登録は毎回全部外してから張り直す（キーを変えたときに旧キーが残らない）。
    manager
        .unregister_all()
        .map_err(|error| format!("以前のホットキー解除に失敗しました: {error}"))?;

    let mut failures: Vec<String> = Vec::new();
    let mut summon_ready = false;

    if desired.enabled {
        for (combo, action) in [
            (&desired.summon, Action::Summon),
            (&desired.hide, Action::ToggleHide),
        ] {
            let combo = combo.as_str();
            // 空欄は他の設定欄と同じく「既定値を使う」意味（`config.get` が既定を返す）。
            // 片方だけ使いたいときのために、`off` を無効化の明示指定として受け付ける。
            if combo.is_empty() || combo.eq_ignore_ascii_case(DISABLED) {
                continue;
            }
            // 解除がOSへ反映される前に同じキーを張り直すと「already registered」で失敗する
            // ことがある。明示解除を挟んで1度だけやり直し、単発の取りこぼしを
            // 「設定したのに効かない」で終わらせない。
            let registered = match manager.on_shortcut(combo, shortcut_handler(action)) {
                Ok(()) => Ok(()),
                Err(first) => {
                    let _ = manager.unregister(combo);
                    manager
                        .on_shortcut(combo, shortcut_handler(action))
                        .map_err(|retry| format!("{first}（解除して再試行しても失敗: {retry}）"))
                }
            };
            match registered {
                Ok(()) => {
                    if action == Action::Summon {
                        summon_ready = true;
                    }
                }
                Err(error) => failures.push(format!(
                    "{combo}（他のアプリが同じキーを使っている可能性があります）: {error}"
                )),
            }
        }
    }

    // 呼び出しキーが無いまま隠れたままにすると、右クリックメニューにも手が届かず
    // アプリを操作できなくなる。呼び戻す手段が無いときは必ず表示へ戻す。
    if !summon_ready {
        if let Some(window) = app.get_webview_window("main") {
            if !window.is_visible().unwrap_or(true) {
                window
                    .show()
                    .map_err(|error| format!("ウィンドウの再表示に失敗しました: {error}"))?;
                window
                    .set_focus()
                    .map_err(|error| format!("ウィンドウの前面化に失敗しました: {error}"))?;
            }
        }
    }

    if failures.is_empty() {
        *applied = Some(desired);
        Ok(())
    } else {
        *applied = None;
        Err(format!(
            "ホットキーを登録できませんでした: {}",
            failures.join(" / ")
        ))
    }
}

/// 終了時にホットキーの登録を解除する（OS へ確保したキーを明示的に返す）。
pub fn release(app: &AppHandle) {
    if let Err(error) = app.global_shortcut().unregister_all() {
        crate::log_paths::append_app_log(
            app,
            &format!("終了時のホットキー解除に失敗しました: {error}"),
        );
    }
    if let Ok(mut applied) = APPLIED_SETTINGS.lock() {
        *applied = None;
    }
}

/// ホットキーの処理を作る（登録の再試行で同じ処理をもう一度渡すため関数にする）。
fn shortcut_handler(
    action: Action,
) -> impl Fn(&AppHandle, &Shortcut, ShortcutEvent) + Send + Sync + 'static {
    move |app, _shortcut, event| {
        // 押した瞬間だけ反応する（離したときにもイベントが来る）。
        if event.state == ShortcutState::Pressed {
            run(app, action);
        }
    }
}

/// ホットキーが押されたときの実処理。
fn run(app: &AppHandle, action: Action) {
    let Some(window) = app.get_webview_window("main") else {
        crate::log_paths::append_app_log(app, "ホットキー: メインウィンドウが見つかりません");
        return;
    };
    let visible = window.is_visible().unwrap_or(true);

    if action == Action::ToggleHide && visible {
        if let Err(error) = window.hide() {
            crate::log_paths::append_app_log(
                app,
                &format!("ホットキーでの非表示に失敗しました: {error}"),
            );
        }
        return;
    }

    // 表示→最小化解除→前面化は、多重起動時の前面化と同じ手順を使う。
    crate::window_surface::surface_main_window(app);
    // 居眠りを解除して入力欄を開くのはフロント側の担当。
    if let Err(error) = window.emit("pet:hotkey", "summon") {
        crate::log_paths::append_app_log(app, &format!("ホットキーで入力欄を開けません: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        needs_registration_refresh, RegistrationSettings, DISABLED, HIDE_DEFAULT, SUMMON_DEFAULT,
    };
    use tauri_plugin_global_shortcut::Shortcut;

    /// 既定のキー文字列が実際に解釈できること（打ち間違いのまま配布しない）。
    #[test]
    fn 既定のホットキーは解釈できる() {
        for combo in [SUMMON_DEFAULT, HIDE_DEFAULT] {
            assert!(
                combo.parse::<Shortcut>().is_ok(),
                "既定のホットキー {combo} を解釈できません"
            );
        }
    }

    /// 呼び出しと非表示に同じキーを割り当てていないこと（後勝ちで片方が死ぬ）。
    #[test]
    fn 既定のホットキーは重複しない() {
        assert_ne!(SUMMON_DEFAULT, HIDE_DEFAULT);
    }

    /// 無効化の指定値が実在のキー指定と衝突しないこと（衝突すると、そのキーを
    /// 割り当てたつもりのユーザーが黙って無効化される）。
    #[test]
    fn 無効化の指定値はキーとして解釈できない() {
        assert!(DISABLED.parse::<Shortcut>().is_err());
    }

    #[test]
    fn 同じ設定の保存では登録をやり直さない() {
        let settings = RegistrationSettings {
            enabled: true,
            summon: SUMMON_DEFAULT.to_string(),
            hide: HIDE_DEFAULT.to_string(),
        };
        assert!(!needs_registration_refresh(Some(&settings), &settings));

        let changed = RegistrationSettings {
            hide: DISABLED.to_string(),
            ..settings.clone()
        };
        assert!(needs_registration_refresh(Some(&settings), &changed));
        assert!(needs_registration_refresh(None, &settings));
    }
}
