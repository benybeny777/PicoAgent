// キャラクターごとの明示的な描画方式を検査し、対象IDだけを原子的に更新する。
use serde_json::{Map, Value};

use crate::config::Config;

pub(crate) const CONFIG_KEY: &str = "PICOAGENT_RENDERER_OVERRIDES";

pub(crate) struct SavedRenderer {
    pub overrides: Map<String, Value>,
    pub notification_error: Option<String>,
}

pub(crate) fn checked(value: &Value) -> Result<Map<String, Value>, String> {
    let map = value
        .as_object()
        .ok_or_else(|| "キャラクターの描画設定はオブジェクトで指定してください。".to_string())?;
    for (id, renderer) in map {
        if !crate::character_pack::is_valid_id(id) {
            return Err("キャラクターの描画設定に不正なIDがあります。".to_string());
        }
        if renderer.as_str() != Some("image") {
            return Err("キャラクターの描画設定は image のみ指定できます。既定へ戻す場合は項目を削除してください。".to_string());
        }
    }
    Ok(map.clone())
}

pub(crate) fn load(values: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    match values.get(CONFIG_KEY) {
        Some(value) => checked(value),
        None => Ok(Map::new()),
    }
}

pub(crate) fn validate_patch(patch: &Map<String, Value>) -> Result<(), String> {
    if let Some(value) = patch.get(CONFIG_KEY) {
        checked(value)?;
    }
    Ok(())
}

pub(crate) fn set_character(
    config: &Config,
    character_id: &str,
    renderer: Option<&str>,
    publish: impl FnOnce(&Map<String, Value>) -> Result<(), String>,
) -> Result<SavedRenderer, String> {
    if !crate::character_pack::is_valid_id(character_id) {
        return Err("キャラクターIDが不正です。".to_string());
    }
    if renderer.is_some_and(|renderer| renderer != "image") {
        return Err("描画方式は image または既定への復帰だけを指定できます。".to_string());
    }
    let _guard = crate::store::lock_state(CONFIG_KEY);
    let (conflicts, prepared) = config
        .set_all_checked_with(Map::new(), &Map::new(), |latest, patch| {
            let mut map = load(latest).map_err(anyhow::Error::msg)?;
            match renderer {
                Some(renderer) => {
                    map.insert(
                        character_id.to_string(),
                        Value::String(renderer.to_string()),
                    );
                }
                None => {
                    map.remove(character_id);
                }
            }
            patch.insert(CONFIG_KEY.to_string(), Value::Object(map));
            validate_patch(patch).map_err(anyhow::Error::msg)
        })
        .map_err(|error| format!("キャラクターの描画設定を保存できません: {error}"))?;
    if !conflicts.is_empty() {
        return Err("キャラクターの描画設定が別の操作で更新されました。".to_string());
    }
    let map = load(&prepared)?;
    // 保存と通知の順序も揃え、並行した別ID更新の古いmapが後から届くことを防ぐ。
    let notification_error = publish(&map).err();
    Ok(SavedRenderer {
        overrides: map,
        notification_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Barrier};

    fn save(
        config: &Config,
        id: &str,
        renderer: Option<&str>,
    ) -> Result<Map<String, Value>, String> {
        set_character(config, id, renderer, |_| Ok(())).map(|saved| saved.overrides)
    }

    fn setup() -> (Config, std::path::PathBuf) {
        let root = crate::guard_scan::repo_root().join("temp").join(format!(
            "renderer-setting-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = Config::new();
        config.load(root.join("config.json")).unwrap();
        (config, root)
    }

    #[test]
    fn 描画設定の型と方式と識別子を検査する() {
        for value in [
            json!(null),
            json!([]),
            json!({"../x":"image"}),
            json!({"mugi":null}),
            json!({"mugi":"mmd"}),
        ] {
            assert!(checked(&value).is_err(), "不正値を拒否する: {value}");
        }
        assert_eq!(checked(&json!({})).unwrap(), Map::new());
    }

    #[test]
    fn 通知失敗でも保存済みの描画設定と理由を返す() {
        let (config, root) = setup();
        let saved = set_character(&config, "mugi", Some("image"), |_| {
            Err("通知先を利用できません".to_string())
        })
        .unwrap();
        assert_eq!(
            saved.notification_error.as_deref(),
            Some("通知先を利用できません")
        );
        assert_eq!(
            Value::Object(saved.overrides.clone()),
            json!({"mugi":"image"})
        );
        assert_eq!(load(&config.get_all()).unwrap(), saved.overrides);
        let reloaded = Config::new();
        reloaded.load(root.join("config.json")).unwrap();
        assert_eq!(load(&reloaded.get_all()).unwrap(), saved.overrides);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 一つの識別子を戻しても別キャラの描画設定を保持する() {
        let (config, root) = setup();
        save(&config, "mugi", Some("image")).unwrap();
        save(&config, "hiyori", Some("image")).unwrap();
        assert_eq!(
            Value::Object(save(&config, "mugi", None).unwrap()),
            json!({"hiyori":"image"})
        );
        let reloaded = Config::new();
        reloaded.load(root.join("config.json")).unwrap();
        assert_eq!(
            Value::Object(load(&reloaded.get_all()).unwrap()),
            json!({"hiyori":"image"})
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 二つの識別子の同時保存で他方を消さない() {
        let (config, root) = setup();
        let config = Arc::new(config);
        let barrier = Arc::new(Barrier::new(2));
        let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
        let threads = ["mugi", "hiyori"].map(|id| {
            let config = config.clone();
            let barrier = barrier.clone();
            let notifications = notifications.clone();
            std::thread::spawn(move || {
                barrier.wait();
                set_character(&config, id, Some("image"), |map| {
                    notifications.lock().unwrap().push(map.len());
                    Ok(())
                })
                .unwrap();
            })
        });
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(*notifications.lock().unwrap(), vec![1, 2]);
        assert_eq!(
            Value::Object(load(&config.get_all()).unwrap()),
            json!({"mugi":"image", "hiyori":"image"})
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 保存失敗と壊れた既存設定ではメモリも変えない() {
        let (config, root) = setup();
        save(&config, "mugi", Some("image")).unwrap();
        let before = config.get_all();
        std::fs::remove_file(root.join("config.json")).unwrap();
        std::fs::create_dir(root.join("config.json")).unwrap();
        let notified = std::cell::Cell::new(false);
        assert!(set_character(&config, "hiyori", Some("image"), |_| {
            notified.set(true);
            Ok(())
        })
        .is_err());
        assert!(!notified.get(), "保存失敗後は通知しない");
        assert_eq!(config.get_all(), before);
        config.set_value(CONFIG_KEY, json!({"mugi":"unknown"}));
        let broken = config.get_all();
        assert!(save(&config, "hiyori", Some("image")).is_err());
        assert_eq!(config.get_all(), broken);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 古い設定画面の保存で単一キャラの更新を上書きしない() {
        let (config, root) = setup();
        save(&config, "mugi", Some("image")).unwrap();
        let revisions = config.value_revisions();
        save(&config, "hiyori", Some("image")).unwrap();
        let patch = Map::from_iter([(CONFIG_KEY.to_string(), json!({}))]);
        let (conflicts, _) = config
            .set_all_checked_with(patch, &revisions, |_, patch| {
                validate_patch(patch).map_err(anyhow::Error::msg)
            })
            .unwrap();
        assert_eq!(conflicts, vec![CONFIG_KEY]);
        assert_eq!(
            Value::Object(load(&config.get_all()).unwrap()),
            json!({"mugi":"image", "hiyori":"image"})
        );
        let patch = Map::from_iter([(CONFIG_KEY.to_string(), json!({}))]);
        let (conflicts, _) = config
            .set_all_checked_with(patch, &config.value_revisions(), |_, patch| {
                validate_patch(patch).map_err(anyhow::Error::msg)
            })
            .unwrap();
        assert!(conflicts.is_empty());
        assert!(
            load(&config.get_all()).unwrap().is_empty(),
            "最新の設定画面から既定へ戻せる"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
