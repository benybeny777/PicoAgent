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
    /// 「スナップショットを取る → ファイルへ書く」を直列化するためのロック。
    /// これが無いと、2つの保存が同時に走ったとき **あとから取ったスナップショットが
    /// 先に書かれ、古い方が後から上書きする**ことがあり、メモリ上には残っているのに
    /// ファイルからは片方の変更が消える（再起動で設定が戻る）。
    write: Mutex<()>,
}

impl Config {
    pub fn new() -> Self {
        Config {
            path: Mutex::new(None),
            values: Mutex::new(Map::new()),
            write: Mutex::new(()),
        }
    }

    /// 起動時にファイルパスを設定し、あれば読み込む。
    /// 設定ファイルを読み込む。破損などで読めなかった場合は警告文字列を返す
    /// （呼び出し側が app.log へ記録する。ファイルが無いだけの初回起動は正常なので None）。
    pub fn load(&self, file: PathBuf) -> Result<Option<String>, String> {
        let mut warning = None;
        match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => *self.values.lock().unwrap() = map,
                Ok(_) | Err(_) => {
                    // フォールバック許可: 破損時は空設定で起動（初回起動を成立させるため）。
                    // ただし「設定が全部消えた」ように見える事象の原因調査ができるよう必ず記録する。
                    // 破損ファイルはその場に残すと次の保存（＝空設定の書き戻し）で消えてしまうので退避する。
                    let saved = crate::store::quarantine_corrupt(&file).ok_or_else(|| {
                        format!(
                            "壊れた設定ファイルを退避できません。上書きを中止しました: {}",
                            file.display()
                        )
                    })?;
                    warning = Some(format!(
                        "設定ファイルが JSON として読めないため空設定で起動します: {}（破損ファイルは {} へ退避しました）",
                        file.display(),
                        saved.display()
                    ));
                }
            },
            Err(error) if file.exists() => {
                return Err(format!(
                    "設定ファイルを読み込めません。上書きを中止しました: {} ({error})",
                    file.display()
                ));
            }
            Err(_) => {} // ファイルが無いのは初回起動の正常系
        }
        *self.path.lock().unwrap() = Some(file);
        Ok(warning)
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
    ///
    /// 内容が1つも変わらない場合はファイルへ書かない。設定は「読み込み → 全体を書き戻す」
    /// 更新なので、ポーリング経路（通知の期限チェックなど）から同じ値で呼ばれるたびに
    /// config.json 全体を書き直してしまい、無意味なディスク書き込みが延々と続く。
    pub fn set_all(&self, patch: Map<String, Value>) -> anyhow::Result<()> {
        {
            let mut values = self.values.lock().unwrap();
            let mut changed = false;
            for (k, v) in patch {
                if values.get(&k) != Some(&v) {
                    values.insert(k, v);
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
        }
        // スナップショットの採取から書き込みまでを直列化する（並び替わりを防ぐ）。
        // フォールバック許可: 毒された場合も保存自体は続ける（中身は () で不変）。
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        assert!(
            cfg.load(path.clone()).unwrap().is_none(),
            "初回起動は警告なし"
        );
        cfg.set_string("PICOAGENT_TEST_KEY", "値".to_string())
            .unwrap();

        let reloaded = Config::new();
        assert!(reloaded.load(path.clone()).unwrap().is_none());
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

    /// 設定は毎回「全体を書き戻す」ので、同じ値での保存を弾かないと
    /// ポーリング経路から呼ばれるたびに無意味なディスク書き込みが続く。
    #[test]
    fn 内容が変わらない保存はファイルへ書かない() {
        let dir = temp_dir("nochange");
        let path = dir.join("config.json");

        let cfg = Config::new();
        cfg.load(path.clone()).unwrap();
        cfg.set_string("PICOAGENT_TEST_KEY", "値".to_string())
            .unwrap();
        assert!(path.exists());

        // ファイルを消してから同じ値で保存する。書き込みが起きれば再生成される。
        std::fs::remove_file(&path).unwrap();
        cfg.set_string("PICOAGENT_TEST_KEY", "値".to_string())
            .unwrap();
        assert!(!path.exists(), "同じ値なのに書き込んでいる");

        // 値が変われば当然書く。
        cfg.set_string("PICOAGENT_TEST_KEY", "別の値".to_string())
            .unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 保存が同時に走っても、ファイルには全部の変更が入った状態が残ること。
    ///
    /// スナップショットの採取と書き込みが直列化されていないと、あとから取った
    /// スナップショットが先に書かれ、古い方が後から上書きしてしまう。
    #[test]
    fn 同時保存でも変更が消えない() {
        use std::sync::Arc;
        let dir = temp_dir("concurrent");
        let path = dir.join("config.json");
        let cfg = Arc::new(Config::new());
        cfg.load(path.clone()).unwrap();

        let mut handles = Vec::new();
        for index in 0..8 {
            let cfg = Arc::clone(&cfg);
            handles.push(std::thread::spawn(move || {
                cfg.set_string(&format!("PICOAGENT_TEST_KEY_{index}"), index.to_string())
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        // ファイルから読み直して、8件すべてが残っていること。
        let reloaded = Config::new();
        reloaded.load(path.clone()).unwrap();
        for index in 0..8 {
            assert_eq!(
                reloaded.get(&format!("PICOAGENT_TEST_KEY_{index}"), ""),
                index.to_string(),
                "同時保存で {index} 番目の変更がファイルから消えている"
            );
        }
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
        let warning = cfg
            .load(path.clone())
            .expect("破損時も退避できる")
            .expect("破損は警告する");
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
