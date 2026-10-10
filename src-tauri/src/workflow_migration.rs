//! 予約の資料スキルと限定許可を、未知フィールドを保持して一度だけ移行する。
use crate::{
    config::Config,
    scheduler::{CONFIG_KEY, TOOL_ALLOWLIST_KEY},
};
use serde_json::{Map, Value};

const OLD: &str = "spark_workspace";
const NEW: &str = "genspark_suite";

/// 起動時と復旧待ちの検出処理で、同じ原因の通知を重ねず移行を試す。
pub(crate) fn prepare(app: &tauri::AppHandle, cfg: &Config) -> bool {
    static FAILURE: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
    match migrate(cfg) {
        Ok(changed) => {
            let was_failed = FAILURE.is_failed();
            FAILURE.report(
                app,
                "workflow-migration",
                "予約設定を確認してください",
                Ok(()),
                "予約の移行を再確認できました。スケジュール実行を再開します。",
            );
            if changed && !was_failed {
                crate::notice::emit(app, crate::notice::Notice::info("settings_migration", "資料スキルの予約を移行しました", "旧資料スキルを軽量モードへ移しました。許可範囲と実行状態は保持しています。", true));
            }
            true
        }
        Err(error) => {
            FAILURE.report(
                app,
                "workflow-migration",
                "予約設定を確認してください",
                Err(error.to_string()),
                "予約設定の移行が復旧しました",
            );
            false
        }
    }
}

fn migration_patch(current: &Map<String, Value>) -> anyhow::Result<Map<String, Value>> {
    let mut patch = Map::new();
    if let Some(value) = current
        .get(CONFIG_KEY)
        .filter(|value| value.to_string().contains(OLD))
    {
        let raw = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("予約一覧はJSON文字列で保存してください。"))?;
        let mut tasks: Value = serde_json::from_str(raw)?;
        let mut changed = false;
        for task in tasks
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("予約一覧は配列で指定してください。"))?
        {
            if task.get("tool_name").and_then(Value::as_str) != Some(OLD) {
                continue;
            }
            let task = task
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("資料スキルの予約が不正です。"))?;
            let args = task
                .get_mut("tool_args")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| anyhow::anyhow!("旧資料スキルのtool_argsが不正です。"))?;
            let project = args
                .get("project")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("旧資料スキルのprojectがありません。"))?;
            if project.as_str().is_none_or(|text| text.trim().is_empty())
                || args.contains_key("topic")
                || args.contains_key("mode")
            {
                anyhow::bail!("旧資料スキルの引数が競合または不正です。project/topic/modeを確認してください。");
            }
            args.remove("project");
            args.insert("topic".into(), project);
            args.insert("mode".into(), Value::String("light".into()));
            task.insert("tool_name".into(), Value::String(NEW.into()));
            changed = true;
        }
        if changed {
            patch.insert(
                CONFIG_KEY.into(),
                Value::String(serde_json::to_string(&tasks)?),
            );
        }
    }
    if let Some(value) = current
        .get(TOOL_ALLOWLIST_KEY)
        .filter(|value| !value.is_null())
    {
        let raw = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("予約のスキル許可は文字列で指定してください。"))?;
        let entries = raw
            .split(',')
            .map(|entry| {
                if entry.trim() == OLD {
                    entry.replace(OLD, "genspark_suite:light")
                } else {
                    entry.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        if entries != raw {
            patch.insert(TOOL_ALLOWLIST_KEY.into(), Value::String(entries));
        }
    }
    Ok(patch)
}

pub(crate) fn migrate(cfg: &Config) -> anyhow::Result<bool> {
    migrate_with_effective(cfg, |cfg| {
        Map::from_iter(
            [CONFIG_KEY, TOOL_ALLOWLIST_KEY]
                .map(|key| (key.into(), Value::String(cfg.get(key, "")))),
        )
    })
}

/// 実効値が環境由来になる場合、旧名の値を勝手に保存設定へ移植しない。
fn validate_effective_sources(
    current: &Map<String, Value>,
    effective: &Map<String, Value>,
) -> anyhow::Result<()> {
    for key in [CONFIG_KEY, TOOL_ALLOWLIST_KEY] {
        let saved_source = match current.get(key) {
            Some(Value::String(value)) => !value.is_empty(),
            Some(value) => !value.is_null(),
            None => false,
        };
        if saved_source {
            continue;
        }
        let value = effective
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("{key} の実効値を確認できません。"))?;
        let has_old = if key == TOOL_ALLOWLIST_KEY {
            value.split(',').any(|name| name.trim() == OLD)
        } else {
            if !value.contains(OLD) {
                false
            } else {
                let tasks: Value = serde_json::from_str(value).map_err(|_| {
                    anyhow::anyhow!("環境変数の {key} は不正なJSONです。予約を変更していません。")
                })?;
                tasks
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("環境変数の {key} は配列で指定してください。"))?
                    .iter()
                    .any(|task| task.get("tool_name").and_then(Value::as_str) == Some(OLD))
            }
        };
        if has_old {
            anyhow::bail!("環境変数の {key} にある旧資料スキルを設定で明示的に変更してください。予約・許可・実行状態は変更していません。");
        }
    }
    Ok(())
}

// 実環境を書き換えずに由来を検査できる注入口。通常経路も同じ所有者と保存境界を使う。
fn migrate_with_effective(
    cfg: &Config,
    effective: impl Fn(&Config) -> Map<String, Value>,
) -> anyhow::Result<bool> {
    let _guard = crate::store::lock_state(CONFIG_KEY);
    let current = cfg.get_all();
    validate_effective_sources(&current, &effective(cfg))?;
    if migration_patch(&current)?.is_empty() {
        return Ok(false);
    }
    // 設定の保存ロック内で最新値から作り直し、永続化成功後だけ公開する。
    let (_, patch) = cfg.set_all_checked_with(Map::new(), &Map::new(), |current, patch| {
        validate_effective_sources(current, &effective(cfg))?;
        *patch = migration_patch(current)?;
        Ok(())
    })?;
    Ok(!patch.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> Map<String, Value> {
        Map::from_iter([
            (CONFIG_KEY.into(), Value::String(json!([{"id":"固定ID","enabled":false,"tool_name":OLD,"tool_args":{"project":"資料","goal":"目的","未使用":42},"未来属性":{"x":1},"last_run_minute":"過去"}]).to_string())),
            (TOOL_ALLOWLIST_KEY.into(), json!("weather, spark_workspace ,genspark_suite")),
            (crate::scheduler::STATE_KEY.into(), json!("元の実行状態")),
        ])
    }

    #[test]
    fn 予約の未知属性と実行状態を保持し軽量権限だけ移す() {
        let original = settings();
        let patch = migration_patch(&original).unwrap();
        let mut next = original.clone();
        next.extend(patch.clone());
        assert_eq!(
            next[crate::scheduler::STATE_KEY],
            original[crate::scheduler::STATE_KEY]
        );
        let before: Value = serde_json::from_str(original[CONFIG_KEY].as_str().unwrap()).unwrap();
        let after: Value = serde_json::from_str(next[CONFIG_KEY].as_str().unwrap()).unwrap();
        let mut expected = before;
        expected[0]["tool_name"] = json!(NEW);
        expected[0]["tool_args"]
            .as_object_mut()
            .unwrap()
            .remove("project");
        expected[0]["tool_args"]["topic"] = json!("資料");
        expected[0]["tool_args"]["mode"] = json!("light");
        assert_eq!(after, expected);
        assert_eq!(
            next[TOOL_ALLOWLIST_KEY],
            json!("weather, genspark_suite:light ,genspark_suite")
        );
        assert!(migration_patch(&next).unwrap().is_empty());
    }

    #[test]
    fn 引数競合時は許可も予約も移行しない() {
        for key in ["topic", "mode"] {
            let mut original = settings();
            let mut tasks: Value =
                serde_json::from_str(original[CONFIG_KEY].as_str().unwrap()).unwrap();
            tasks[0]["tool_args"][key] = json!("既存値");
            original.insert(CONFIG_KEY.into(), json!(tasks.to_string()));
            assert!(migration_patch(&original).is_err());
        }
    }

    #[test]
    fn 保存失敗で設定と実行状態を公開しない() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        std::fs::write(&file, json!(settings()).to_string()).unwrap();
        let cfg = Config::new();
        cfg.load(file.clone()).unwrap();
        let before = cfg.get_all();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(migrate(&cfg).is_err());
        assert_eq!(cfg.get_all(), before);
    }

    #[test]
    fn 環境由来の旧予約と許可は保存不在空nullの全条件で永続化も状態更新もしない() {
        for key in [CONFIG_KEY, TOOL_ALLOWLIST_KEY] {
            for saved in [None, Some(json!("")), Some(Value::Null)] {
                let dir = tempfile::tempdir().unwrap();
                let file = dir.path().join("config.json");
                let mut original = settings();
                match saved {
                    Some(value) => {
                        original.insert(key.into(), value);
                    }
                    None => {
                        original.remove(key);
                    }
                }
                let bytes = json!(original).to_string();
                std::fs::write(&file, &bytes).unwrap();
                let cfg = Config::new();
                cfg.load(file.clone()).unwrap();
                let before = cfg.get_all();
                let effective = Map::from_iter([
                    (
                        CONFIG_KEY.into(),
                        json!(json!([{"tool_name":OLD,"tool_args":{"project":"資料"}}]).to_string()),
                    ),
                    (TOOL_ALLOWLIST_KEY.into(), json!(OLD)),
                ]);
                let error = migrate_with_effective(&cfg, |_| effective.clone())
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(key));
                assert_eq!(cfg.get_all(), before);
                assert_eq!(std::fs::read(&file).unwrap(), bytes.as_bytes());
            }
        }
    }

    #[test]
    fn 保存非空値を環境より優先しnull許可は未指定のまま移行を保存する() {
        let effective = Map::from_iter([
            (CONFIG_KEY.into(), json!("")),
            (TOOL_ALLOWLIST_KEY.into(), json!("")),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        let mut original = settings();
        original.insert(TOOL_ALLOWLIST_KEY.into(), Value::Null);
        std::fs::write(&file, json!(original).to_string()).unwrap();
        let cfg = Config::new();
        cfg.load(file.clone()).unwrap();
        assert!(migrate_with_effective(&cfg, |_| effective.clone()).unwrap());
        let next = cfg.get_all();
        assert_eq!(next[TOOL_ALLOWLIST_KEY], Value::Null);
        assert_eq!(
            next[crate::scheduler::STATE_KEY],
            original[crate::scheduler::STATE_KEY]
        );
        let tasks: Value = serde_json::from_str(next[CONFIG_KEY].as_str().unwrap()).unwrap();
        assert_eq!(tasks[0]["tool_name"], NEW);
        assert!(!migrate_with_effective(&cfg, |_| effective.clone()).unwrap());
        let env_old = Map::from_iter([
            (
                CONFIG_KEY.into(),
                json!("旧名を含む不正JSON spark_workspace"),
            ),
            (TOOL_ALLOWLIST_KEY.into(), json!(OLD)),
        ]);
        // 保存済み非空設定は、注入した環境候補より優先する。
        assert!(validate_effective_sources(&settings(), &env_old).is_ok());
        let mut explicit_empty = Map::new();
        explicit_empty.insert(TOOL_ALLOWLIST_KEY.into(), Value::Null);
        assert!(migration_patch(&explicit_empty).unwrap().is_empty());
        let mentions = Map::from_iter([
            (
                CONFIG_KEY.into(),
                json!(json!([{"tool_name":"weather","future":OLD}]).to_string()),
            ),
            (TOOL_ALLOWLIST_KEY.into(), json!("weather")),
        ]);
        assert!(validate_effective_sources(&Map::new(), &mentions).is_ok());
    }
}
