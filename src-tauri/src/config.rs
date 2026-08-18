// config.rs - 設定の正本（永続ストア）。
//
// 値の優先順位: 永続ファイル（app_config_dir/config.json）> 環境変数 > ハードコード既定値。
// コネクタは get() で都度読むため、設定画面での保存はアプリ再起動なしに反映される。

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value};

pub struct Config {
    path: Mutex<Option<PathBuf>>,
    /// 原本を読めず・退避できず、保存先を持てないまま起動した理由。
    /// 保存要求が来たときに「なぜ保存できないのか」を利用者へそのまま返すため覚えておく。
    read_only_reason: Mutex<Option<String>>,
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
            read_only_reason: Mutex::new(None),
            values: Mutex::new(Map::new()),
            write: Mutex::new(()),
        }
    }

    /// 起動時にファイルパスを設定し、あれば読み込む。
    /// 設定ファイルを読み込む。破損などで読めなかった場合は警告文字列を返す
    /// （呼び出し側が app.log へ記録する。ファイルが無いだけの初回起動は正常なので None）。
    /// 原本を安全に退避できない場合は保存先を設定せず、読み取り専用の既定値で起動する。
    pub fn load(&self, file: PathBuf) -> Result<Option<String>, String> {
        let mut warning = None;
        match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(map)) => *self.values.lock().unwrap() = map,
                Ok(_) | Err(_) => {
                    // フォールバック許可: 破損時は空設定で起動（初回起動を成立させるため）。
                    // ただし「設定が全部消えた」ように見える事象の原因調査ができるよう必ず記録する。
                    // 破損ファイルはその場に残すと次の保存（＝空設定の書き戻し）で消えてしまうので退避する。
                    if let Some(saved) = crate::store::quarantine_corrupt(&file) {
                        warning = Some(format!(
                            "設定ファイルが JSON として読めないため空設定で起動します: {}（破損ファイルは {} へ退避しました）",
                            file.display(),
                            saved.display()
                        ));
                    } else {
                        return Ok(Some(self.start_read_only(format!(
                            "壊れた設定ファイルを退避できないため、設定を保存しない読み取り専用状態で起動します: {}",
                            file.display()
                        ))));
                    }
                }
            },
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Ok(Some(self.start_read_only(format!(
                    "設定ファイルを読み込めないため、設定を保存しない読み取り専用状態で起動します: {} ({error})",
                    file.display()
                ))));
            }
            Err(_) => {} // ファイルが無いのは初回起動の正常系
        }
        *self.path.lock().unwrap() = Some(file);
        Ok(warning)
    }

    /// 保存先を持たない読み取り専用状態に入り、その理由を覚えて同じ文言を返す。
    /// 覚えておかないと、設定画面の保存が「config.load() が未実行です」という
    /// 実装都合の文言になり、利用者には原因も対象ファイルも分からない。
    fn start_read_only(&self, reason: String) -> String {
        *self.read_only_reason.lock().unwrap() = Some(reason.clone());
        reason
    }

    /// 文字列設定を取得。空文字は未設定とみなし env → 既定値へフォールバック。
    pub fn get(&self, key: &str, fallback: &str) -> String {
        if let Some(v) = self.values.lock().unwrap().get(key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    // シークレットは暗号化保存されているため、読み取り時に復号する（非暗号値は素通り）。
                    let decrypted = crate::secret::decrypt(s);
                    // 復号できないまま暗号文が残るのは、別PC・別Windowsユーザーの
                    // `config.json` を持ち込んだ場合。以前はその暗号文をそのまま
                    // APIキーとして外部サービスへ送っており、返ってくるのは 401 で、
                    // 利用者には原因が分からなかった。**この端末では使えない値**なので
                    // 未設定として扱い、各プロバイダの「設定画面で入力してください」
                    // という案内へ乗せる。保存値自体は上書きしない。
                    if crate::secret::is_encrypted(&decrypted) {
                        return fallback.to_string();
                    }
                    return decrypted;
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
        self.set_all_with_writer(patch, crate::store::write_json_atomic)
    }

    fn set_all_with_writer(
        &self,
        patch: Map<String, Value>,
        write_snapshot: impl FnOnce(&std::path::Path, &Value) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        // スナップショットの採取から書き込みまでを直列化する（並び替わりを防ぐ）。
        // フォールバック許可: 毒された場合も保存自体は続ける（中身は () で不変）。
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.path.lock().unwrap().clone();
        let Some(path) = path else {
            // 読み取り専用で起動した場合は、その理由（対象ファイルを含む）をそのまま返す。
            let reason = self.read_only_reason.lock().unwrap().clone();
            return Err(anyhow::anyhow!(
                reason.unwrap_or_else(|| "config.load() が未実行です".to_string())
            ));
        };
        let next = {
            let values = self.values.lock().unwrap();
            let mut next = values.clone();
            let mut changed = false;
            for (k, v) in patch {
                if next.get(&k) != Some(&v) {
                    next.insert(k, v);
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
            next
        };
        let snapshot = Value::Object(next);
        // 書き込み中に落ちても設定が全損しないよう、共通層の原子的な書き込みを使う。
        // ディスクI/O中は values ロックを外し、プロバイダ等の get() を止めない。
        write_snapshot(&path, &snapshot)?;
        let Value::Object(next) = snapshot else {
            unreachable!("設定スナップショットは必ずJSONオブジェクト")
        };
        *self.values.lock().unwrap() = next;
        Ok(())
    }

    /// 単一の設定を永続化する。
    pub fn set_string(&self, key: &str, value: String) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert(key.to_string(), Value::String(value));
        self.set_all(patch)
    }

    /// 削除済みキャラクターを選択していた旧設定だけを安全な代替へ移す。
    pub fn migrate_removed_character(&self, removed: &str, fallback: &str) -> anyhow::Result<bool> {
        let selected = self
            .values
            .lock()
            .unwrap()
            .get("PICOAGENT_CHARACTER")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if selected.as_deref() != Some(removed) {
            return Ok(false);
        }
        self.set_string("PICOAGENT_CHARACTER", fallback.to_string())?;
        Ok(true)
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
    /// 別PC・別Windowsユーザーの config.json を持ち込むと復号できない。
    /// その暗号文をAPIキーとして外部へ送ると 401 が返るだけで原因が分からないので、
    /// 未設定として扱い各プロバイダの設定案内へ乗せる。
    #[test]
    fn 復号できない暗号文は未設定として扱う() {
        let cfg = super::Config::new();
        cfg.set("PICOAGENT_TEST_SECRET", "enc:dpapi:v1:こわれたデータ");
        assert_eq!(cfg.get("PICOAGENT_TEST_SECRET", ""), "");
        assert_eq!(cfg.get("PICOAGENT_TEST_SECRET", "既定値"), "既定値");
        // 平文はそのまま読める。
        cfg.set("PICOAGENT_TEST_SECRET", "sk-plain");
        assert_eq!(cfg.get("PICOAGENT_TEST_SECRET", ""), "sk-plain");
    }

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

    #[test]
    fn 削除済みキャラの保存選択だけを既定キャラへ移す() {
        let dir = temp_dir("removed-character");
        let path = dir.join("config.json");
        let cfg = Config::new();
        cfg.load(path.clone()).unwrap();
        cfg.set_string("PICOAGENT_CHARACTER", "mugi-live2d".to_string())
            .unwrap();

        assert!(cfg
            .migrate_removed_character("mugi-live2d", "mugi")
            .unwrap());
        assert_eq!(cfg.get("PICOAGENT_CHARACTER", ""), "mugi");
        assert!(!cfg
            .migrate_removed_character("mugi-live2d", "mugi")
            .unwrap());

        let reloaded = Config::new();
        reloaded.load(path.clone()).unwrap();
        assert_eq!(reloaded.get("PICOAGENT_CHARACTER", ""), "mugi");
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
    fn 保存のディスク待ち中も現在値を読める() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;

        let dir = temp_dir("read-during-write");
        let path = dir.join("config.json");
        let cfg = Arc::new(Config::new());
        cfg.load(path.clone()).unwrap();
        cfg.set_string("PICOAGENT_TEST_KEY", "保存前".to_string())
            .unwrap();

        let (write_started_tx, write_started_rx) = mpsc::channel();
        let (release_write_tx, release_write_rx) = mpsc::channel();
        let writer_cfg = Arc::clone(&cfg);
        let writer = std::thread::spawn(move || {
            let mut patch = serde_json::Map::new();
            patch.insert(
                "PICOAGENT_TEST_KEY".to_string(),
                serde_json::Value::String("保存後".to_string()),
            );
            writer_cfg
                .set_all_with_writer(patch, move |path, snapshot| {
                    write_started_tx.send(()).unwrap();
                    release_write_rx.recv().unwrap();
                    crate::store::write_json_atomic(path, snapshot)
                })
                .unwrap();
        });
        write_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("ディスク書き込み待ちへ入る");

        let (read_tx, read_rx) = mpsc::channel();
        let reader_cfg = Arc::clone(&cfg);
        let reader = std::thread::spawn(move || {
            read_tx
                .send(reader_cfg.get("PICOAGENT_TEST_KEY", ""))
                .unwrap();
        });
        let observed = read_rx.recv_timeout(Duration::from_secs(1));
        release_write_tx.send(()).unwrap();
        writer.join().unwrap();
        reader.join().unwrap();

        assert_eq!(
            observed.expect("保存中のget()がvaluesロックで止まっている"),
            "保存前"
        );
        assert_eq!(cfg.get("PICOAGENT_TEST_KEY", ""), "保存後");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 設定ファイルの保存失敗時は実行中の値を変えない() {
        let dir = temp_dir("write-failure");
        let path = dir.join("config.json");
        let cfg = Config::new();
        cfg.load(path).unwrap();
        cfg.set_string("PICOAGENT_TEST_KEY", "保存前".to_string())
            .unwrap();

        let mut patch = serde_json::Map::new();
        patch.insert(
            "PICOAGENT_TEST_KEY".to_string(),
            serde_json::Value::String("保存後".to_string()),
        );
        let error = cfg
            .set_all_with_writer(patch, |_, _| anyhow::bail!("書き込み失敗"))
            .expect_err("保存失敗を返す");
        assert!(error.to_string().contains("書き込み失敗"));
        assert_eq!(cfg.get("PICOAGENT_TEST_KEY", ""), "保存前");
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

    #[test]
    fn 読み込めない設定でも保存を止めて起動を続ける() {
        let dir = temp_dir("unreadable");
        // ファイルの代わりにディレクトリを渡し、読み取り不能をOS共通で再現する。
        let cfg = Config::new();
        let warning = cfg
            .load(dir.clone())
            .expect("読み取り失敗だけで起動を止めない")
            .expect("読み取り専用状態を警告する");
        assert!(warning.contains("読み取り専用"), "warning: {warning}");
        let error = cfg
            .set_string("PICOAGENT_OTHER", "x".to_string())
            .expect_err("原本を読めない状態では保存しない");
        // 保存できない理由は、実装都合ではなく起動時と同じ文言（対象パス付き）で返す。
        assert!(error.to_string().contains("読み取り専用"), "error: {error}");
        assert!(
            error.to_string().contains(&dir.display().to_string()),
            "error: {error}"
        );
        assert!(dir.is_dir(), "元の対象を上書きしていない");
        std::fs::remove_dir_all(&dir).ok();
    }
}
