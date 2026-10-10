// config.rs - 設定の正本（永続ストア）。
//
// 値の優先順位: 永続ファイル（PicoAgent/config.json）> 環境変数 > ハードコード既定値。
// 開発ビルドの PICO_PREVIEW_CHARACTER だけは保存値を書き換えず実描画を選ぶため最優先にする。
// コネクタは get() で都度読むため、設定画面での保存はアプリ再起動なしに反映される。

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[path = "numeric_settings.rs"]
pub mod ints;
pub use ints::{number_settings_schema, validate_int_patch, IntSetting};

const MISSING_REVISION: &str = "missing";

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
    /// 起動処理からも呼ぶ（保存先を決められない場合でも画面は出す）ため公開する。
    pub fn start_read_only(&self, reason: String) -> String {
        *self.read_only_reason.lock().unwrap() = Some(reason.clone());
        reason
    }

    /// PicoAgentが管理する設定ディレクトリを返す。
    ///
    /// 利用者が追加した指示スキルのように、config.jsonと同じ寿命で管理するファイルは
    /// カレントディレクトリへ降格せず、この場所だけへ保存する。
    pub fn config_dir(&self) -> Result<PathBuf, String> {
        let path = self.path.lock().unwrap();
        let Some(file) = path.as_ref() else {
            let reason = self.read_only_reason.lock().unwrap().clone();
            return Err(reason.unwrap_or_else(|| {
                "PicoAgentの設定保存先がまだ初期化されていません。".to_string()
            }));
        };
        file.parent()
            .map(PathBuf::from)
            .ok_or_else(|| "PicoAgentの設定ディレクトリを特定できません。".to_string())
    }

    /// 文字列設定を取得。空文字は未設定とみなし env → 既定値へフォールバック。
    pub fn get(&self, key: &str, fallback: &str) -> String {
        if cfg!(debug_assertions) && key == "PICOAGENT_CHARACTER" {
            if let Ok(preview_character) = std::env::var("PICO_PREVIEW_CHARACTER") {
                if !preview_character.is_empty() {
                    return preview_character;
                }
            }
        }
        if let Some(v) = self.values.lock().unwrap().get(key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    // シークレットはOSの資格情報保護へ預けてあるため、読み取り時に復号する（非保護値は素通り）。
                    let decrypted = crate::secret::decrypt(s);
                    // 復号できないまま保護形式が残るのは、別PC・別OSユーザー・別OSの
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

    /// 数量設定は保存時と同じ宣言で読む。ファイル中の不正値も黙って続行させない。
    pub fn int_or_auto(&self, setting: &IntSetting) -> Result<Option<i64>, String> {
        let stored = self.values.lock().unwrap().get(setting.key).cloned();
        setting.parse(&effective_int_text(stored.as_ref(), setting)?)
    }

    pub fn int(&self, setting: &IntSetting) -> Result<i64, String> {
        self.int_or_auto(setting)?
            .ok_or_else(|| format!("{} は整数が必要な処理では auto を使えません。", setting.key))
    }

    /// 保存候補も永続値・環境変数と同じ優先順で検証する。原本は変更しない。
    pub fn int_or_auto_in_patch(
        &self,
        setting: &IntSetting,
        patch: &Map<String, Value>,
    ) -> Result<Option<i64>, String> {
        let stored = patch
            .get(setting.key)
            .cloned()
            .or_else(|| self.values.lock().unwrap().get(setting.key).cloned());
        setting.parse(&effective_int_text(stored.as_ref(), setting)?)
    }

    pub fn int_in_patch(
        &self,
        setting: &IntSetting,
        patch: &Map<String, Value>,
    ) -> Result<i64, String> {
        self.int_or_auto_in_patch(setting, patch)?
            .ok_or_else(|| format!("{} は整数が必要です。", setting.key))
    }

    pub fn duration_secs(&self, setting: &IntSetting) -> Result<std::time::Duration, String> {
        let value = self.int(setting)?;
        let seconds = u64::try_from(value)
            .map_err(|_| format!("{} は負の待ち時間を指定できません。", setting.key))?;
        Ok(std::time::Duration::from_secs(seconds))
    }

    pub fn duration_key(&self, key: &str) -> Result<std::time::Duration, String> {
        let setting =
            ints::find(key).ok_or_else(|| format!("整数設定の宣言がありません: {key}"))?;
        self.duration_secs(setting)
    }

    pub fn invalid_int_settings(&self) -> Vec<Value> {
        let values = self.get_all();
        ints::INT_SETTINGS
            .iter()
            .filter_map(|setting| {
                effective_int_text(values.get(setting.key), setting)
                    .and_then(|raw| setting.parse(&raw))
                    .err()
                    .map(|message| serde_json::json!({"key":setting.key, "message":message}))
            })
            .collect()
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

    /// 設定画面の楽観的排他に使う、値ごとの不可逆な版。
    /// シークレットの値そのものを画面へ返さず、読込後の変更だけを検出する。
    pub fn value_revisions(&self) -> Map<String, Value> {
        self.values
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value_revision(Some(value)))))
            .collect()
    }

    /// 設定を部分更新して永続化する。
    ///
    /// 内容が1つも変わらない場合はファイルへ書かない。設定は「読み込み → 全体を書き戻す」
    /// 更新なので、ポーリング経路（通知の期限チェックなど）から同じ値で呼ばれるたびに
    /// config.json 全体を書き直してしまい、無意味なディスク書き込みが延々と続く。
    pub fn set_all(&self, patch: Map<String, Value>) -> anyhow::Result<()> {
        self.set_all_with_writer(patch, crate::store::write_json_atomic)
    }

    /// 読込時の版が変わっていない場合だけ、差分を原子的に保存する。
    /// 比較から書込みまで `write` ロック内で行い、検査直後の更新割込みを許さない。
    /// 競合検査に成功した場合だけ、同じ保存ロック内で差分の検証・保護変換を行う。
    /// シークレットを資格情報ストアへ書く処理が競合却下より先に走ることを防ぐ。
    pub fn set_all_checked_with(
        &self,
        mut patch: Map<String, Value>,
        expected_revisions: &Map<String, Value>,
        prepare: impl FnOnce(&Map<String, Value>, &mut Map<String, Value>) -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<String>, Map<String, Value>)> {
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.path.lock().unwrap().clone();
        let Some(path) = path else {
            let reason = self.read_only_reason.lock().unwrap().clone();
            return Err(anyhow::anyhow!(
                reason.unwrap_or_else(|| "config.load() が未実行です".to_string())
            ));
        };
        let current = self.values.lock().unwrap().clone();
        let mut conflicts = expected_revisions
            .iter()
            .filter_map(|(key, expected)| {
                let expected = expected.as_str()?;
                (value_revision(current.get(key)) != expected).then(|| key.clone())
            })
            .collect::<Vec<_>>();
        conflicts.sort();
        if !conflicts.is_empty() {
            return Ok((conflicts, patch));
        }
        prepare(&current, &mut patch)?;
        let mut next = current;
        let mut changed = false;
        for (key, value) in &patch {
            if next.get(key) != Some(value) {
                next.insert(key.clone(), value.clone());
                changed = true;
            }
        }
        if !changed {
            return Ok((Vec::new(), patch));
        }
        crate::store::write_json_atomic(&path, &Value::Object(next.clone()))?;
        *self.values.lock().unwrap() = next;
        Ok((Vec::new(), patch))
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

    /// テスト専用: JSONオブジェクトや配列をインメモリへ設定する。
    #[cfg(test)]
    pub fn set_value(&self, key: &str, value: Value) {
        self.values.lock().unwrap().insert(key.to_string(), value);
    }
}

/// 数量値は秘密設定の復号・未設定降格を経由せず、元の型も文字列として検査する。
fn effective_int_text(stored: Option<&Value>, setting: &IntSetting) -> Result<String, String> {
    if let Some(value) = stored {
        match value {
            Value::Null => {}
            Value::String(raw) if raw.is_empty() => {}
            Value::String(raw) => return Ok(raw.clone()),
            value => return Ok(value.to_string()),
        }
    }
    match std::env::var(setting.key) {
        Ok(raw) if !raw.is_empty() => Ok(raw),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(setting.default_text()),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!(
            "{} の環境変数が文字列として読めません。",
            setting.key
        )),
    }
}

fn value_revision(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return MISSING_REVISION.to_string();
    };
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
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

    struct RestoreEnvironment {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl RestoreEnvironment {
        fn new(key: &'static str) -> Self {
            Self {
                key,
                original: std::env::var_os(key),
            }
        }
    }

    impl Drop for RestoreEnvironment {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn 数量の保存値と環境値と既定を順に検証する() {
        // 既存の並行保存試験が使う0〜7とは異なる環境キーを専有する。
        let setting = super::IntSetting {
            key: "PICOAGENT_TEST_KEY_9001",
            label: "数量検査",
            default: Some(5),
            min: 0,
            max: 10,
            unit: "件",
        };
        let _restore = RestoreEnvironment::new(setting.key);
        std::env::remove_var(setting.key);
        let cfg = Config::new();
        assert_eq!(cfg.int(&setting).unwrap(), 5);
        for raw in ["", " ", "　"] {
            cfg.set(setting.key, raw);
            assert_eq!(cfg.int(&setting).unwrap(), 5);
        }
        for (raw, expected) in [("0", 0), ("10", 10), ("７", 7), (" ８ ", 8)] {
            cfg.set(setting.key, raw);
            assert_eq!(cfg.int(&setting).unwrap(), expected);
        }
        for raw in [
            "1.5",
            "１．５",
            "-1",
            "11",
            "bad",
            "auto",
            "9223372036854775808",
        ] {
            cfg.set(setting.key, raw);
            assert!(cfg.int(&setting).unwrap_err().contains(setting.key));
        }
        std::env::set_var(setting.key, "９");
        cfg.set(setting.key, "");
        assert_eq!(cfg.int(&setting).unwrap(), 9);
        cfg.values
            .lock()
            .unwrap()
            .insert(setting.key.to_string(), serde_json::Value::Null);
        assert_eq!(cfg.int(&setting).unwrap(), 9);
        cfg.set(setting.key, "2");
        assert_eq!(cfg.int(&setting).unwrap(), 2);
        std::env::set_var(setting.key, "bad");
        assert_eq!(cfg.int(&setting).unwrap(), 2);
        for value in [serde_json::json!(""), serde_json::Value::Null] {
            let patch = serde_json::Map::from_iter([(setting.key.to_string(), value)]);
            assert!(cfg
                .int_or_auto_in_patch(&setting, &patch)
                .unwrap_err()
                .contains(setting.key));
            assert!(cfg
                .int_in_patch(&setting, &patch)
                .unwrap_err()
                .contains(setting.key));
            assert_eq!(
                cfg.int(&setting).unwrap(),
                2,
                "検査だけで保存値を変更しない"
            );
        }
        cfg.set(setting.key, "");
        assert!(cfg.int(&setting).unwrap_err().contains(setting.key));
        std::env::set_var(setting.key, "");
        assert_eq!(cfg.int(&setting).unwrap(), 5);
        for value in [serde_json::json!(""), serde_json::Value::Null] {
            let patch = serde_json::Map::from_iter([(setting.key.to_string(), value)]);
            assert_eq!(cfg.int_or_auto_in_patch(&setting, &patch).unwrap(), Some(5));
            assert_eq!(cfg.int_in_patch(&setting, &patch).unwrap(), 5);
        }
        let automatic = super::IntSetting {
            default: None,
            ..setting
        };
        let patch =
            serde_json::Map::from_iter([(setting.key.to_string(), serde_json::json!("auto"))]);
        assert_eq!(cfg.int_or_auto_in_patch(&automatic, &patch).unwrap(), None);
        assert!(cfg
            .int_in_patch(&automatic, &patch)
            .unwrap_err()
            .contains(setting.key));
        std::env::remove_var(setting.key);
        let signed = super::IntSetting { min: -3, ..setting };
        cfg.set(signed.key, "-3");
        assert_eq!(cfg.int(&signed).unwrap(), -3);
        assert!(cfg.duration_secs(&signed).unwrap_err().contains(signed.key));
    }

    #[test]
    fn 数量が複数不正なら全理由を返し保存済み内容を変更しない() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let initial = serde_json::json!({"PICOAGENT_TEST_KEY":"保存前"});
        crate::store::write_json_atomic(&path, &initial).unwrap();
        let before = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let cfg = Config::new();
        cfg.load(path.clone()).unwrap();
        let patch = serde_json::Map::from_iter([
            (
                "PICOAGENT_MEMORY_MAX_ITEMS".to_string(),
                serde_json::json!("9"),
            ),
            (
                "PICOAGENT_HTTP_METADATA_TIMEOUT_SECS".to_string(),
                serde_json::json!("1.5"),
            ),
            ("PICOAGENT_LOCAL_CTX".to_string(), serde_json::json!("8192")),
            (
                "PICOAGENT_TEST_KEY".to_string(),
                serde_json::json!("保存後"),
            ),
        ]);
        let mut invalid_keys = Vec::new();
        let result = cfg.set_all_checked_with(patch, &serde_json::Map::new(), |_, patch| {
            let (normalized, invalid) = super::validate_int_patch(patch.clone());
            invalid_keys = invalid
                .iter()
                .map(|item| item["key"].as_str().unwrap().to_string())
                .collect();
            if !invalid.is_empty() {
                anyhow::bail!("{}", serde_json::Value::Array(invalid));
            }
            *patch = normalized;
            Ok(())
        });
        assert!(result.is_err());
        invalid_keys.sort();
        assert_eq!(
            invalid_keys,
            [
                "PICOAGENT_HTTP_METADATA_TIMEOUT_SECS",
                "PICOAGENT_LOCAL_CTX",
                "PICOAGENT_MEMORY_MAX_ITEMS"
            ]
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(cfg.get_all(), initial.as_object().unwrap().clone());
    }

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
    fn 読込後に変わった設定は差分保存で上書きしない() {
        use std::cell::Cell;

        let dir = temp_dir("checked-update");
        let path = dir.join("config.json");
        let cfg = Config::new();
        cfg.load(path.clone()).unwrap();
        cfg.set_string("A", "読込時".to_string()).unwrap();
        let revisions = cfg.value_revisions();

        cfg.set_string("A", "別経路の更新".to_string()).unwrap();
        let mut patch = serde_json::Map::new();
        patch.insert("A".into(), serde_json::json!("画面の更新"));
        let prepared = Cell::new(false);
        let (conflicts, _) = cfg
            .set_all_checked_with(patch, &revisions, |_, _| {
                prepared.set(true);
                Ok(())
            })
            .unwrap();
        assert_eq!(conflicts, vec!["A"]);
        assert!(!prepared.get(), "競合時に保存前処理を実行してはいけない");
        assert_eq!(cfg.get("A", ""), "別経路の更新");

        let mut patch = serde_json::Map::new();
        patch.insert("B".into(), serde_json::json!("追加"));
        let mut missing = serde_json::Map::new();
        missing.insert("B".into(), serde_json::json!("missing"));
        assert!(cfg
            .set_all_checked_with(patch, &missing, |_, _| Ok(()))
            .unwrap()
            .0
            .is_empty());
        assert_eq!(cfg.get("B", ""), "追加");
        std::fs::remove_dir_all(dir).ok();
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

    /// 読み取り専用起動は「保存できない」状態であって「起動できない」状態ではない。
    /// 起動処理側がここで受け取る `Err` を握って進めるよう、理由付きで返し続けること。
    #[test]
    fn 読み取り専用起動でも移行の失敗を理由付きで返す() {
        let cfg = Config::new();
        let reason = cfg.start_read_only("設定の保存先を取得できません".to_string());
        assert!(reason.contains("設定の保存先を取得できません"));
        cfg.set("PICOAGENT_CHARACTER", "mugi-live2d");

        let error = cfg
            .migrate_removed_character("mugi-live2d", "mugi")
            .unwrap_err()
            .to_string();
        assert!(error.contains("設定の保存先を取得できません"), "{error}");
        // 保存できないだけで、実行中の値は起動に使える状態のまま残す。
        assert_eq!(cfg.get("PICOAGENT_CHARACTER", ""), "mugi-live2d");
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
    #[test]
    fn 数量読取は保存値を優先し不正型を既定値へ戻さない() {
        use super::ints;
        let cfg = Config::new();
        cfg.set(ints::HTTP_TIMEOUT_SECS.key, "３０");
        assert_eq!(cfg.int(&ints::HTTP_TIMEOUT_SECS).unwrap(), 30);
        assert_eq!(
            cfg.duration_key(ints::HTTP_TIMEOUT_SECS.key).unwrap(),
            std::time::Duration::from_secs(30)
        );
        for bad in [
            serde_json::json!(false),
            serde_json::json!({}),
            serde_json::json!(1.5),
            serde_json::json!("enc:dpapi:v1:壊れた値"),
        ] {
            cfg.set_value(ints::HTTP_TIMEOUT_SECS.key, bad.clone());
            assert!(cfg.int(&ints::HTTP_TIMEOUT_SECS).is_err());
            assert_eq!(cfg.get_all()[ints::HTTP_TIMEOUT_SECS.key], bad);
        }
        cfg.set(ints::BROWSER_MAX_CHARS.key, "auto");
        assert_eq!(cfg.int_or_auto(&ints::BROWSER_MAX_CHARS).unwrap(), None);
        assert!(cfg.int(&ints::BROWSER_MAX_CHARS).is_err());
        assert!(cfg.duration_key("未宣言のキー").is_err());
    }

    #[test]
    fn 不正な保存済み数量は保持し正常項目だけ読める() {
        use super::ints;
        let cfg = Config::new();
        cfg.set(ints::HTTP_TIMEOUT_SECS.key, "abc");
        cfg.set(ints::RSS_TIMEOUT_SECS.key, "999");
        cfg.set(ints::CLIPBOARD_MAX_CHARS.key, "3000");
        let before = cfg.get_all();
        let invalid = cfg.invalid_int_settings();
        for key in [ints::HTTP_TIMEOUT_SECS.key, ints::RSS_TIMEOUT_SECS.key] {
            assert!(invalid.iter().any(|entry| entry["key"] == key));
        }
        assert_eq!(cfg.int(&ints::CLIPBOARD_MAX_CHARS).unwrap(), 3000);
        assert_eq!(cfg.get_all(), before);
    }
}
