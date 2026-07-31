// project_workspace/mod.rs - プロジェクトごとの許可ルート・AI・キャラクター設定を切り替える。

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const WORKSPACES_KEY: &str = "PICOAGENT_WORKSPACES";
const ACTIVE_KEY: &str = "PICOAGENT_ACTIVE_WORKSPACE";

pub struct ProjectWorkspaceTool;

fn load(cfg: &Config) -> Result<Vec<Value>, String> {
    crate::store::load_list(cfg, WORKSPACES_KEY)
}

fn save(cfg: &Config, items: &[Value]) -> Result<(), String> {
    cfg.set_string(
        WORKSPACES_KEY,
        serde_json::to_string(items).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("ワークスペースを保存できません: {error}"))
}

fn slug(value: &str) -> String {
    let out: String = value
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    out.split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[async_trait]
impl Tool for ProjectWorkspaceTool {
    fn name(&self) -> &'static str { "project_workspace" }
    fn label(&self) -> &'static str { "プロジェクト切り替え" }
    fn description(&self) -> String {
        "仕事やプロジェクトごとに、許可ルート・追加許可パス・AI・モデル・キャラクターを保存して切り替える。".to_string()
    }
    fn examples(&self) -> Vec<&'static str> {
        vec!["このフォルダを仕事用ワークスペースとして登録して", "動画制作ワークスペースへ切り替えて"]
    }
    fn parameters(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "action":{"type":"string","enum":["list","add","switch","remove"]},
                "id":{"type":"string","description":"識別子。省略時は名前から生成。"},
                "name":{"type":"string"},
                "allowed_root":{"type":"string"},
                "allowed_paths":{"type":"string","description":"追加許可パス。改行または ; 区切り。"},
                "provider":{"type":"string"},
                "model":{"type":"string"},
                "character":{"type":"string"},
                "confirm":{"type":"boolean","description":"remove の確認。"}
            },
            "required":["action"]
        })
    }
    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(ACTIVE_KEY, "現在のプロジェクトワークスペース", "text", "")]
    }
    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        if action == "list" {
            let items = match load(cfg) {
                Ok(items) => items,
                Err(error) => return format!("エラー: {error}"),
            };
            let active = cfg.get(ACTIVE_KEY, "");
            return if items.is_empty() { "登録済みワークスペースはありません。".to_string() }
            else { match serde_json::to_string_pretty(&items) {
                Ok(json) => format!("現在: {active}\n{json}"),
                Err(error) => format!("エラー: 一覧を整形できません: {error}"),
            }};
        }
        let name = args.get("name").and_then(Value::as_str).unwrap_or("").trim();
        let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
        let id = if id.is_empty() { slug(name) } else { slug(id) };
        if id.is_empty() { return "エラー: id または name を指定してください。".to_string(); }
        match action {
            "add" => {
                let _guard = crate::store::lock_state(WORKSPACES_KEY);
                let mut items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                if items.iter().any(|item| item.get("id").and_then(Value::as_str) == Some(&id)) {
                    return format!("エラー: 同じIDのワークスペースがあります: {id}");
                }
                items.push(json!({
                    "id":id,"name":if name.is_empty(){&id}else{name},
                    "allowed_root":args.get("allowed_root").and_then(Value::as_str).unwrap_or(""),
                    "allowed_paths":args.get("allowed_paths").and_then(Value::as_str).unwrap_or(""),
                    "provider":args.get("provider").and_then(Value::as_str).unwrap_or(""),
                    "model":args.get("model").and_then(Value::as_str).unwrap_or(""),
                    "character":args.get("character").and_then(Value::as_str).unwrap_or("")
                }));
                match save(cfg, &items) { Ok(()) => format!("ワークスペースを登録しました: {id}"), Err(error) => format!("エラー: {error}") }
            }
            "switch" => {
                let _guard = crate::store::lock_state(WORKSPACES_KEY);
                let items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                let Some(item) = items.iter().find(|item| item.get("id").and_then(Value::as_str) == Some(&id)) else {
                    return format!("エラー: ワークスペースが見つかりません: {id}");
                };
                let mut patch = Map::new();
                patch.insert(ACTIVE_KEY.to_string(), Value::String(id.clone()));
                for (source, target) in [("allowed_root","PICOAGENT_ALLOWED_ROOT"),("allowed_paths","PICOAGENT_ALLOWED_PATHS"),("provider","PICOAGENT_PROVIDER"),("model","PICOAGENT_MODEL"),("character","PICOAGENT_CHARACTER")] {
                    if let Some(value) = item.get(source).and_then(Value::as_str).filter(|value| !value.is_empty()) {
                        patch.insert(target.to_string(), Value::String(value.to_string()));
                    }
                }
                match cfg.set_all(patch) { Ok(()) => format!("ワークスペースを切り替えました: {id}\n許可ルートは次のスキル実行から反映されます。AI・モデル・キャラクターの変更は安全に読み直すため、PicoAgentを再起動すると反映されます。"), Err(error) => format!("エラー: 切り替えを保存できません: {error}") }
            }
            "remove" => {
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!("プレビュー: ワークスペース {id} を登録解除します。ファイルは削除しません。実行には confirm=true が必要です。");
                }
                let _guard = crate::store::lock_state(WORKSPACES_KEY);
                let mut items = match load(cfg) {
                    Ok(items) => items,
                    Err(error) => return format!("エラー: {error}"),
                };
                let before = items.len();
                items.retain(|item| item.get("id").and_then(Value::as_str) != Some(&id));
                if items.len() == before { return format!("エラー: ワークスペースが見つかりません: {id}"); }
                if cfg.get(ACTIVE_KEY, "") == id {
                    let mut patch = Map::new();
                    let encoded = match serde_json::to_string(&items) {
                        Ok(encoded) => encoded,
                        Err(error) => return format!("エラー: ワークスペースを保存できません: {error}"),
                    };
                    patch.insert(WORKSPACES_KEY.to_string(), Value::String(encoded));
                    patch.insert(ACTIVE_KEY.to_string(), Value::String(String::new()));
                    match cfg.set_all(patch) { Ok(()) => format!("使用中のワークスペース登録を解除しました: {id}"), Err(error) => format!("エラー: ワークスペースを保存できません: {error}") }
                } else {
                    match save(cfg, &items) { Ok(()) => format!("ワークスペースの登録を解除しました: {id}"), Err(error) => format!("エラー: {error}") }
                }
            }
            _ => "エラー: action は list/add/switch/remove のいずれかを指定してください。".to_string()
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ProjectWorkspaceTool) } }
