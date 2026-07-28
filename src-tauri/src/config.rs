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
    /// 設定ファイルを読み込む。破損などで読めなかった場合は警告文字列を返す
    /// （呼び出し側が app.log へ記録する。ファイルが無いだけの初回起動は正常なので None）。
    pub fn load(&self, file: PathBuf) -> Option<String> {
        let mut warning = None;
        match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => *self.values.lock().unwrap() = map,
                Ok(_) | Err(_) => {
                    // フォールバック許可: 破損時は空設定で起動（初回起動を成立させるため）。
                    // ただし「設定が全部消えた」ように見える事象の原因調査ができるよう必ず記録する。
                    // 破損ファイルはその場に残すと次の保存（＝空設定の書き戻し）で消えてしまうので退避する。
                    let saved = crate::store::quarantine_corrupt(&file);
                    warning = Some(match saved {
                        Some(saved) => format!(
                            "設定ファイルが JSON として読めないため空設定で起動します: {}（破損ファイルは {} へ退避しました）",
                            file.display(),
                            saved.display()
                        ),
                        None => format!(
                            "設定ファイルが JSON として読めないため空設定で起動します: {}",
                            file.display()
                        ),
                    });
                }
            },
            Err(error) if file.exists() => {
                warning = Some(format!(
                    "設定ファイルを読み込めないため空設定で起動します: {} ({error})",
                    file.display()
                ));
            }
            Err(_) => {} // ファイルが無いのは初回起動の正常系
        }
        *self.path.lock().unwrap() = Some(file);
        warning
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
        let snapshot = Value::Object(self.values.lock().unwrap().clone());
        // 書き込み中に落ちても設定が全損しないよう、共通層の原子的な書き込みを使う。
        crate::store::write_json_atomic(&path, &snapshot)
    }

    /// 単一の設定を永続化する。
    pub fn set_string(&self, key: &str, value: String) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert(key.to_string(), Value::String(value));
        self.set_all(patch)
    }

    /// テスト専用: 永続化せずインメモリの設定値だけを書き換える
    /// （環境変数と違いテスト間で競合しない）。
    #[cfg(test)]
    pub fn set(&self, key: &str, value: &str) {
        self.values
            .lock()
            .unwrap()
            .insert(key.to_string(), Value::String(value.to_string()));
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-config-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 保存した設定を読み直せる() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");

        let cfg = Config::new();
        assert!(cfg.load(path.clone()).is_none(), "初回起動は警告なし");
        cfg.set_string("PICOAGENT_TEST_KEY", "値".to_string())
            .unwrap();

        let reloaded = Config::new();
        assert!(reloaded.load(path.clone()).is_none());
        assert_eq!(reloaded.get("PICOAGENT_TEST_KEY", ""), "値");

        // 原子的な書き込みなので、保存後に一時ファイルが残らない。
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["config.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 破損した設定は退避されて上書きで失われない() {
        let dir = temp_dir("corrupt");
        let path = dir.join("config.json");
        // 書き込み途中で落ちた状態を模す（末尾が切れた JSON）。
        crate::store::write_atomic(&path, "{\n  \"PICOAGENT_TEST_KEY\": \"値\"".as_bytes())
            .unwrap();

        let cfg = Config::new();
        let warning = cfg.load(path.clone()).expect("破損は警告する");
        assert!(warning.contains("退避"), "退避先を案内する: {warning}");

        // 空設定で起動したあと保存しても、破損ファイルの中身は隣に残っている。
        cfg.set_string("PICOAGENT_OTHER", "x".to_string()).unwrap();
        let saved: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".corrupt-"))
            .collect();
        assert_eq!(saved.len(), 1, "破損ファイルが退避されていない");
        let text = std::fs::read_to_string(dir.join(&saved[0])).unwrap();
        assert!(text.contains("PICOAGENT_TEST_KEY"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
