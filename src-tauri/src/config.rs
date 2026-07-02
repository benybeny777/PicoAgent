// config.rs - 設定の正本（永続ストア）。
//
// 値の優先順位: 永続ファイル（app_config_dir/config.json）> 環境変数 > ハードコード既定値。
// コネクタは get() で都度読むため、設定画面での保存はアプリ再起動なしに反映される。

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value};

pub struct Config {
    path: Mutex<Option<PathBuf>>,
    values: Mutex<Map<String, Value>>,
}

impl Config {
    pub fn new() -> Self {
        Config {
            path: Mutex::new(None),
            values: Mutex::new(Map::new()),
        }
    }

    /// 起動時にファイルパスを設定し、あれば読み込む。
    pub fn load(&self, file: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
                *self.values.lock().unwrap() = map;
            }
            // フォールバック許可: 破損時は空設定で起動（初回起動を成立させるため）。
        }
        *self.path.lock().unwrap() = Some(file);
    }

    /// 文字列設定を取得。空文字は未設定とみなし env → 既定値へフォールバック。
    pub fn get(&self, key: &str, fallback: &str) -> String {
        if let Some(v) = self.values.lock().unwrap().get(key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    // シークレットは暗号化保存されているため、読み取り時に復号する（非暗号値は素通り）。
                    return crate::secret::decrypt(s);
                }
            } else if !v.is_null() {
                return v.to_string();
            }
        }
        if let Ok(env_val) = std::env::var(key) {
            if !env_val.is_empty() {
                return env_val;
            }
        }
        fallback.to_string()
    }

    /// 真偽値設定を取得。
    pub fn get_bool(&self, key: &str, fallback: bool) -> bool {
        if let Some(v) = self.values.lock().unwrap().get(key) {
            if let Some(b) = v.as_bool() {
                return b;
            }
            if let Some(s) = v.as_str() {
                return s == "true" || s == "1";
            }
        }
        if let Ok(env_val) = std::env::var(key) {
            return env_val == "true" || env_val == "1";
        }
        fallback
    }

    /// 永続設定または環境変数に空でない値があるか。
    pub fn has_value(&self, key: &str) -> bool {
        if let Some(v) = self.values.lock().unwrap().get(key) {
            if let Some(s) = v.as_str() {
                return !s.is_empty();
            }
            return !v.is_null();
        }
        std::env::var(key)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    }

    /// 永続化されている生の値一式（設定画面の初期表示用）。
    pub fn get_all(&self) -> Map<String, Value> {
        self.values.lock().unwrap().clone()
    }

    /// 設定を部分更新して永続化する。
    pub fn set_all(&self, patch: Map<String, Value>) -> anyhow::Result<()> {
        {
            let mut values = self.values.lock().unwrap();
            for (k, v) in patch {
                values.insert(k, v);
            }
        }
        let path = self
            .path
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("config.load() が未実行です"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text =
            serde_json::to_string_pretty(&Value::Object(self.values.lock().unwrap().clone()))?;
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// 単一の設定を永続化する。
    pub fn set_string(&self, key: &str, value: String) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert(key.to_string(), Value::String(value));
        self.set_all(patch)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}
