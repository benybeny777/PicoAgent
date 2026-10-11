// config.rs - 設定の正本（永続ストア）。
//
// 値の優先順位: 永続ファイル（PicoAgent/config.json）> 環境変数 > ハードコード既定値。
// 開発ビルドの PICO_PREVIEW_CHARACTER だけは保存値を書き換えず実描画を選ぶため最優先にする。
// 通常の保存はすぐ反映し、再初期化が必要な値だけは承諾されるまで実行中の値を保つ。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[path = "numeric_settings.rs"]
pub mod ints;
pub use ints::{number_settings_schema, validate_int_patch, IntSetting};

const MISSING_REVISION: &str = "missing";

#[derive(Default)]
struct ConfigValues {
    saved: Map<String, Value>,
    /// 再初期化を待つ項目の実行値。未保存だった項目も None として固定する。
    deferred: HashMap<String, Option<Value>>,
}

enum RuntimeUpdate<'a> {
    Preserve,
    Defer(&'a [&'a str]),
    Activate,
}

pub struct Config {
    path: Mutex<Option<PathBuf>>,
    /// 原本を読めず・退避できず、保存先を持てないまま起動した理由。
    /// 保存要求が来たときに「なぜ保存できないのか」を利用者へそのまま返すため覚えておく。
    read_only_reason: Mutex<Option<String>>,
    /// 保存値と実行値を同じロックで公開し、保存直後の先行反映を防ぐ。
    values: Mutex<ConfigValues>,
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
            values: Mutex::new(ConfigValues::default()),
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
                Ok(Value::Object(map)) => self.values.lock().unwrap().saved = map,
                Ok(_) | Err(_) => {
                    // フォールバック許可: 破損時は空設定で起動（初回起動を成立させるため）。（根拠: PR #463 採用案。退避・通知と原本保護）
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

    /// 実行中の生の値。画面表示・保存検証には永続値を返す get_all() を使う。
    pub fn get_runtime_value(&self, key: &str) -> Option<Value> {
        let values = self.values.lock().unwrap();
        values
            .deferred
            .get(key)
            .cloned()
            .unwrap_or_else(|| values.saved.get(key).cloned())
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
        Self::resolve_string(key, self.get_runtime_value(key).as_ref(), fallback)
    }

    /// 同じ保存スナップショットの値を、通常取得と同じ優先順位で文字列へ解決する。
    /// 開発中の描画上書きは保存値ではないため、get() だけで先に適用する。
    pub(crate) fn resolve_string(key: &str, raw: Option<&Value>, fallback: &str) -> String {
        if let Some(v) = raw {
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
        let stored = self.get_runtime_value(setting.key);
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
            .or_else(|| self.values.lock().unwrap().saved.get(setting.key).cloned());
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
        if let Some(v) = self.get_runtime_value(key) {
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

    /// 実行中の設定または環境変数に空でない値があるか。
    pub fn has_value(&self, key: &str) -> bool {
        if let Some(v) = self.get_runtime_value(key) {
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
        self.values.lock().unwrap().saved.clone()
    }

    /// 設定画面の楽観的排他に使う、値ごとの不可逆な版。
    /// シークレットの値そのものを画面へ返さず、読込後の変更だけを検出する。
    pub fn value_revisions(&self) -> Map<String, Value> {
        Self::revisions_for(&self.values.lock().unwrap().saved)
    }

    /// 読み込んだ値または保存が確定した差分と、必ず同じ時点の版を返す。
    pub fn revisions_for(values: &Map<String, Value>) -> Map<String, Value> {
        values
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

    /// 明示された操作の対象だけを、保存が成功した時点で実行側へ反映する。
    /// 保存済みと同じ値を選び直した場合も、対象の保留だけを解除する。
    pub fn set_all_and_activate(&self, patch: Map<String, Value>) -> anyhow::Result<()> {
        self.save_patch(
            patch,
            &Map::new(),
            RuntimeUpdate::Activate,
            |_, _| Ok(()),
            crate::store::write_json_atomic,
        )
        .map(|_| ())
    }

    /// 削除などの明示操作が参照できなくする値だけを、保存側と実行側で置き換える。
    /// 操作に失敗した場合は設定を戻す。operation 内では設定の保存・保留解除を行わない。
    pub fn replace_value_for_operation<T>(
        &self,
        key: &str,
        removed: Option<&Value>,
        replacement: &Value,
        operation: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        self.replace_for_operation(key, replacement, |raw| raw == removed, operation)
    }

    /// 環境変数から選ばれた値も含め、対象文字列へ解決される選択だけを置き換える。
    pub fn replace_string_for_operation<T>(
        &self,
        key: &str,
        removed: &str,
        replacement: &str,
        operation: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        self.replace_for_operation(
            key,
            &Value::String(replacement.to_string()),
            |raw| Self::resolve_string(key, raw, "") == removed,
            operation,
        )
    }

    fn replace_for_operation<T>(
        &self,
        key: &str,
        replacement: &Value,
        matches: impl Fn(Option<&Value>) -> bool,
        operation: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (current, mut runtime) = {
            let values = self.values.lock().unwrap();
            (values.saved.clone(), values.deferred.get(key).cloned())
        };
        let mut next = current.clone();
        let saved_changed = matches(current.get(key)) && current.get(key) != Some(replacement);
        if saved_changed {
            next.insert(key.to_string(), replacement.clone());
        }
        let runtime_value = match &runtime {
            Some(value) => value.as_ref(),
            None => current.get(key),
        };
        if matches(runtime_value) {
            runtime = Some(Some(replacement.clone()));
        }
        let path = saved_changed.then(|| self.writable_path()).transpose()?;
        let snapshot = Value::Object(next);
        if let Some(path) = &path {
            crate::store::write_json_atomic(path, &snapshot)?;
        }
        let result = match operation() {
            Ok(result) => result,
            Err(error) => {
                if let Some(path) = &path {
                    if let Err(restore) =
                        crate::store::write_json_atomic(path, &Value::Object(current))
                    {
                        return Err(anyhow::anyhow!(
                            "操作に失敗しました: {error:#}。設定の復元にも失敗しました: {restore:#}"
                        ));
                    }
                }
                return Err(error);
            }
        };
        let Value::Object(next) = snapshot else {
            unreachable!("設定スナップショットは必ずJSONオブジェクト")
        };
        let mut values = self.values.lock().unwrap();
        if let Some(runtime) = runtime {
            if runtime.as_ref() == next.get(key) {
                values.deferred.remove(key);
            } else {
                values.deferred.insert(key.to_string(), runtime);
            }
        }
        values.saved = next;
        Ok(result)
    }

    /// 読込時の版が変わっていない場合だけ、差分を原子的に保存する。
    /// 比較から書込みまで `write` ロック内で行い、検査直後の更新割込みを許さない。
    /// 競合検査に成功した場合だけ、同じ保存ロック内で差分の検証・保護変換を行う。
    /// シークレットを資格情報ストアへ書く処理が競合却下より先に走ることを防ぐ。
    pub fn set_all_checked_with(
        &self,
        patch: Map<String, Value>,
        expected_revisions: &Map<String, Value>,
        prepare: impl FnOnce(&Map<String, Value>, &mut Map<String, Value>) -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<String>, Map<String, Value>)> {
        self.set_all_checked_deferred(patch, expected_revisions, &[], prepare)
    }

    /// 再初期化を待つキーは保存だけを先に済ませ、現在の実行値を維持する。
    /// 連続保存しても最初の実行値を保ち、保存失敗・競合時は保留を作らない。
    pub fn set_all_checked_deferred(
        &self,
        patch: Map<String, Value>,
        expected_revisions: &Map<String, Value>,
        deferred_keys: &[&str],
        prepare: impl FnOnce(&Map<String, Value>, &mut Map<String, Value>) -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<String>, Map<String, Value>)> {
        self.save_patch(
            patch,
            expected_revisions,
            RuntimeUpdate::Defer(deferred_keys),
            prepare,
            crate::store::write_json_atomic,
        )
    }

    fn save_patch(
        &self,
        mut patch: Map<String, Value>,
        expected_revisions: &Map<String, Value>,
        runtime_update: RuntimeUpdate<'_>,
        prepare: impl FnOnce(&Map<String, Value>, &mut Map<String, Value>) -> anyhow::Result<()>,
        write_snapshot: impl FnOnce(&std::path::Path, &Value) -> anyhow::Result<()>,
    ) -> anyhow::Result<(Vec<String>, Map<String, Value>)> {
        // スナップショットの採取から公開まで、全保存経路を同じ順序で直列化する。
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.writable_path()?;
        let current = self.values.lock().unwrap().saved.clone();
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
        let mut deferred = Vec::new();
        for (key, value) in &patch {
            if next.get(key) != Some(value) {
                if let RuntimeUpdate::Defer(keys) = &runtime_update {
                    if keys.contains(&key.as_str()) {
                        deferred.push((key.clone(), next.get(key).cloned()));
                    }
                }
                next.insert(key.clone(), value.clone());
                changed = true;
            }
        }
        let snapshot = Value::Object(next);
        // ディスクI/O中は values ロックを外し、実行側の読取を止めない。
        if changed {
            write_snapshot(&path, &snapshot)?;
        }
        let Value::Object(next) = snapshot else {
            unreachable!("設定スナップショットは必ずJSONオブジェクト")
        };
        let mut values = self.values.lock().unwrap();
        for (key, previous) in deferred {
            values.deferred.entry(key).or_insert(previous);
        }
        if matches!(runtime_update, RuntimeUpdate::Activate) {
            for key in patch.keys() {
                values.deferred.remove(key);
            }
        }
        values.saved = next;
        Ok((Vec::new(), patch))
    }

    fn writable_path(&self) -> anyhow::Result<PathBuf> {
        self.path.lock().unwrap().clone().ok_or_else(|| {
            anyhow::anyhow!(self
                .read_only_reason
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "config.load() が未実行です".to_string()))
        })
    }

    /// 利用者が再初期化を承諾した時点で、保存済みの値を実行側へ公開する。
    pub fn activate_saved_values(&self) {
        // 保存と同じ write → values 順に取り、保存途中の解除・保留復活を防ぐ。
        let _write = self
            .write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.values.lock().unwrap().deferred.clear();
    }

    fn set_all_with_writer(
        &self,
        patch: Map<String, Value>,
        write_snapshot: impl FnOnce(&std::path::Path, &Value) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.save_patch(
            patch,
            &Map::new(),
            RuntimeUpdate::Preserve,
            |_, _| Ok(()),
            write_snapshot,
        )
        .map(|_| ())
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
            .saved
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
            .saved
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    /// テスト専用: JSONオブジェクトや配列をインメモリへ設定する。
    #[cfg(test)]
    pub fn set_value(&self, key: &str, value: Value) {
        self.values
            .lock()
            .unwrap()
            .saved
            .insert(key.to_string(), value);
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
        cfg.set_value(setting.key, serde_json::Value::Null);
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
    fn 保存した差分の版は後続更新や未編集の最新値を混ぜない() {
        let directory = tempfile::tempdir().unwrap();
        let cfg = Config::new();
        cfg.load(directory.path().join("config.json")).unwrap();
        cfg.set_all(serde_json::Map::from_iter([
            ("A".into(), serde_json::json!("読込時")),
            ("B".into(), serde_json::json!("変更前")),
        ]))
        .unwrap();
        let snapshot = cfg.get_all();
        let loaded_revisions = Config::revisions_for(&snapshot);
        cfg.set_string("A", "別操作の更新".into()).unwrap();
        let (_, saved) = cfg
            .set_all_checked_with(
                serde_json::Map::from_iter([("B".into(), serde_json::json!("変更後"))]),
                &serde_json::Map::from_iter([("B".into(), loaded_revisions["B"].clone())]),
                |_, _| Ok(()),
            )
            .unwrap();
        // 応答までの別更新を読み直さず、今回保存した値に対応する版を使う。
        cfg.set_string("B", "保存後の別更新".into()).unwrap();
        let saved_revisions = Config::revisions_for(&saved);
        assert_eq!(saved_revisions.len(), 1);
        assert_eq!(
            saved_revisions["B"],
            super::value_revision(Some(&serde_json::json!("変更後")))
        );
        assert_ne!(saved_revisions["B"], cfg.value_revisions()["B"]);
        assert_eq!(
            loaded_revisions["A"],
            super::value_revision(snapshot.get("A"))
        );
        assert_ne!(loaded_revisions["A"], cfg.value_revisions()["A"]);
    }

    fn saved_config(initial: serde_json::Value) -> (tempfile::TempDir, std::path::PathBuf, Config) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        crate::store::write_json_atomic(&path, &initial).unwrap();
        let cfg = Config::new();
        cfg.load(path.clone()).unwrap();
        (directory, path, cfg)
    }

    fn deferred_config(
        active: &str,
        saved: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, Config) {
        let (directory, path, cfg) = saved_config(serde_json::json!({
            "対象": active,
            "別項目": "別実行中"
        }));
        cfg.set_all_checked_deferred(
            serde_json::json!({"対象": saved, "別項目": "別保存後"})
                .as_object()
                .unwrap()
                .clone(),
            &cfg.value_revisions(),
            &["対象", "別項目"],
            |_, _| Ok(()),
        )
        .unwrap();
        (directory, path, cfg)
    }

    #[test]
    fn 明示反映は保存済みと同じ値でも対象だけ保留解除する() {
        for selected in ["保存後", "新しい選択"] {
            let (_directory, path, cfg) = deferred_config("実行中", "保存後");
            let before = std::fs::read(&path).unwrap();
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            cfg.set_all_and_activate(
                serde_json::json!({"対象": selected})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
            assert_eq!(cfg.get("対象", ""), selected);
            assert_eq!(cfg.get("別項目", ""), "別実行中");
            assert_eq!(cfg.get_all()["別項目"], "別保存後");
            let reloaded = Config::new();
            reloaded.load(path.clone()).unwrap();
            assert_eq!(reloaded.get("対象", ""), selected);
            if selected == "保存後" {
                assert_eq!(std::fs::read(&path).unwrap(), before);
                assert_eq!(
                    std::fs::metadata(&path).unwrap().modified().unwrap(),
                    modified
                );
            }
        }
    }

    #[test]
    fn 明示反映の実保存に失敗したら対象と別項目の保留を保つ() {
        let (_directory, path, cfg) = deferred_config("実行中", "保存後");
        let before = cfg.get_all();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(cfg
            .set_all_and_activate(
                serde_json::json!({"対象": "新しい選択"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .is_err());
        assert_eq!(cfg.get_all(), before);
        assert_eq!(cfg.get("対象", ""), "実行中");
        assert_eq!(cfg.get("別項目", ""), "別実行中");
        std::fs::remove_dir(&path).unwrap();
        cfg.set_string("対象", "後続の通常保存".into()).unwrap();
        assert_eq!(cfg.get("対象", ""), "実行中");
    }

    #[test]
    fn 削除操作は保存側と実行側の対象値だけを置き換える() {
        for (active, saved, removed, next_active, next_saved) in [
            ("A", "B", "B", "A", "既定"),
            ("A", "B", "A", "既定", "B"),
            ("A", "B", "C", "A", "B"),
            ("A", "A", "A", "既定", "既定"),
        ] {
            let (directory, path, cfg) = deferred_config(active, saved);
            let asset = directory.path().join("character.json");
            std::fs::write(&asset, removed).unwrap();
            let result = cfg
                .replace_value_for_operation(
                    "対象",
                    Some(&serde_json::json!(removed)),
                    &serde_json::json!("既定"),
                    || {
                        let persisted = Config::new();
                        persisted.load(path.clone()).unwrap();
                        assert_eq!(persisted.get("対象", ""), next_saved);
                        assert_eq!(cfg.get("対象", ""), active);
                        assert_eq!(cfg.get_all()["対象"], saved);
                        std::fs::remove_file(&asset)?;
                        Ok("削除完了")
                    },
                )
                .unwrap();
            assert_eq!(result, "削除完了");
            assert!(!asset.exists());
            assert_eq!(cfg.get("対象", ""), next_active);
            assert_eq!(cfg.get_all()["対象"], next_saved);
            assert_eq!(cfg.get("別項目", ""), "別実行中");
            assert_eq!(cfg.get_all()["別項目"], "別保存後");
            cfg.activate_saved_values();
            assert_eq!(cfg.get("対象", ""), next_saved);
        }
    }

    #[test]
    fn 削除失敗は設定ファイルを戻して保存値と実行値を維持する() {
        for removed in ["A", "B"] {
            let (directory, path, cfg) = deferred_config("A", "B");
            let before = std::fs::read(&path).unwrap();
            let values = cfg.get_all();
            let asset = directory.path().join("削除できないディレクトリ");
            std::fs::create_dir(&asset).unwrap();
            std::fs::write(asset.join("残っているファイル"), "保持").unwrap();
            assert!(cfg
                .replace_value_for_operation(
                    "対象",
                    Some(&serde_json::json!(removed)),
                    &serde_json::json!("既定"),
                    || {
                        std::fs::remove_dir(&asset)?;
                        Ok(())
                    },
                )
                .is_err());
            assert_eq!(std::fs::read(path).unwrap(), before);
            assert_eq!(cfg.get_all(), values);
            assert_eq!(cfg.get("対象", ""), "A");
            assert_eq!(cfg.get("別項目", ""), "別実行中");
            assert!(asset.join("残っているファイル").exists());
        }
    }

    #[test]
    fn 削除前の設定保存に失敗したら削除操作を実行しない() {
        let (_directory, path, cfg) = deferred_config("A", "B");
        let values = cfg.get_all();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let executed = std::cell::Cell::new(false);
        assert!(cfg
            .replace_value_for_operation(
                "対象",
                Some(&serde_json::json!("B")),
                &serde_json::json!("既定"),
                || {
                    executed.set(true);
                    Ok(())
                },
            )
            .is_err());
        assert!(!executed.get());
        assert_eq!(cfg.get_all(), values);
        assert_eq!(cfg.get("対象", ""), "A");
        assert_eq!(cfg.get("別項目", ""), "別実行中");
    }

    #[test]
    fn 欠損実行値の置換は別の保存値を保ち既存値を欠損と誤認しない() {
        for saved in [None, Some("次回の値")] {
            let (_directory, path, cfg) = saved_config(serde_json::json!({}));
            if let Some(saved) = saved {
                cfg.set_all_checked_deferred(
                    serde_json::json!({"対象": saved})
                        .as_object()
                        .unwrap()
                        .clone(),
                    &cfg.value_revisions(),
                    &["対象"],
                    |_, _| Ok(()),
                )
                .unwrap();
            }
            assert_eq!(cfg.get_runtime_value("対象"), None);
            cfg.replace_value_for_operation("対象", None, &serde_json::json!("既定"), || Ok(()))
                .unwrap();
            assert_eq!(cfg.get("対象", ""), "既定");
            assert_eq!(cfg.get_all()["対象"], saved.unwrap_or("既定"));
            let reloaded = Config::new();
            reloaded.load(path).unwrap();
            assert_eq!(reloaded.get("対象", ""), saved.unwrap_or("既定"));
        }
        let (_directory, _path, cfg) = deferred_config("既存", "既存");
        cfg.replace_value_for_operation("対象", None, &serde_json::json!("既定"), || Ok(()))
            .unwrap();
        assert_eq!(cfg.get("対象", ""), "既存");
        assert_eq!(cfg.get_all()["対象"], "既存");
    }

    #[test]
    fn 環境値を使う文字列置換は保存側と実行側を個別に解決する() {
        let key = "TEST_CONFIG_RUNTIME_9005";
        let _restore = RestoreEnvironment::new(key);
        std::env::set_var(key, "A");
        let empty = Some(serde_json::json!(""));
        let null = Some(serde_json::Value::Null);
        let other = Some(serde_json::json!("B"));
        for (initial, saved, expected_runtime, expected_saved) in [
            (None, None, "既定", "既定"),
            (empty.clone(), empty.clone(), "既定", "既定"),
            (null.clone(), null.clone(), "既定", "既定"),
            (None, other.clone(), "既定", "B"),
            (empty.clone(), other.clone(), "既定", "B"),
            (null.clone(), other.clone(), "既定", "B"),
            (other.clone(), empty, "B", "既定"),
            (other, null, "B", "既定"),
        ] {
            let initial_values = initial
                .clone()
                .map(|value| (key.to_string(), value))
                .into_iter()
                .collect();
            let (_directory, path, cfg) = saved_config(serde_json::Value::Object(initial_values));
            if initial != saved {
                cfg.set_all_checked_deferred(
                    serde_json::Map::from_iter([(
                        key.to_string(),
                        saved.clone().expect("このケースの保存先には値がある"),
                    )]),
                    &cfg.value_revisions(),
                    &[key],
                    |_, _| Ok(()),
                )
                .unwrap();
            }
            let before = cfg.get_all();
            assert_eq!(
                cfg.get(key, "未設定"),
                Config::resolve_string(key, initial.as_ref(), "未設定")
            );
            assert_eq!(
                Config::resolve_string(key, before.get(key), "未設定"),
                Config::resolve_string(key, saved.as_ref(), "未設定")
            );
            cfg.replace_string_for_operation(key, "A", "既定", || Ok(()))
                .unwrap();
            assert_eq!(cfg.get(key, "未設定"), expected_runtime);
            let after = cfg.get_all();
            assert_eq!(
                Config::resolve_string(key, after.get(key), "未設定"),
                expected_saved
            );
            let reloaded = Config::new();
            reloaded.load(path).unwrap();
            assert_eq!(reloaded.get(key, "未設定"), expected_saved);
        }
    }

    #[test]
    fn 再初期化待ちの設定は全取得形式で実行値を保ち保存値を別に返す() {
        let setting = super::IntSetting {
            key: "TEST_CONFIG_RUNTIME_9003",
            label: "保留する数量",
            default: Some(5),
            min: 0,
            max: 60,
            unit: "件",
        };
        let missing = "TEST_CONFIG_RUNTIME_9004";
        let _restore = RestoreEnvironment::new(missing);
        std::env::remove_var(missing);
        let initial = serde_json::json!({
            "文字列": "実行中",
            "真偽値": true,
            setting.key: "10",
            "空文字": "",
            "空値": null,
            "即時反映": "保存前"
        });
        let (_directory, path, cfg) = saved_config(initial);
        let before = cfg.value_revisions();
        let patch = serde_json::json!({
            "文字列": "保存後",
            "真偽値": false,
            setting.key: "20",
            missing: "新規値",
            "空文字": "入力済み",
            "空値": "入力済み",
            "即時反映": "保存後"
        })
        .as_object()
        .unwrap()
        .clone();
        let (conflicts, saved) = cfg
            .set_all_checked_deferred(
                patch.clone(),
                &before,
                &["文字列", "真偽値", setting.key, missing, "空文字", "空値"],
                |_, _| Ok(()),
            )
            .unwrap();
        assert!(conflicts.is_empty());
        assert_eq!(saved, patch);
        assert_eq!(cfg.get("文字列", ""), "実行中");
        assert!(cfg.get_bool("真偽値", false));
        assert_eq!(cfg.int(&setting).unwrap(), 10);
        assert_eq!(
            cfg.int_in_patch(&setting, &serde_json::Map::new()).unwrap(),
            20
        );
        assert_eq!(cfg.get_runtime_value(missing), None);
        assert_eq!(cfg.get(missing, "未設定"), "未設定");
        assert!(!cfg.has_value(missing));
        assert_eq!(cfg.get_runtime_value("空文字"), Some(serde_json::json!("")));
        assert_eq!(cfg.get_runtime_value("空値"), Some(serde_json::Value::Null));
        assert!(!cfg.has_value("空文字"));
        assert!(!cfg.has_value("空値"));
        assert_eq!(cfg.get("即時反映", ""), "保存後");
        assert_eq!(cfg.get_all(), patch);
        assert_ne!(cfg.value_revisions()["文字列"], before["文字列"]);

        let reloaded = Config::new();
        reloaded.load(path.clone()).unwrap();
        assert_eq!(reloaded.get_all(), cfg.get_all());
        assert_eq!(reloaded.value_revisions(), cfg.value_revisions());
        assert_eq!(reloaded.get("文字列", ""), "保存後");
        assert!(!reloaded.get_bool("真偽値", true));
        assert_eq!(reloaded.int(&setting).unwrap(), 20);
        assert!(reloaded.has_value(missing));
        let persisted = std::fs::read(&path).unwrap();
        cfg.activate_saved_values();
        for key in patch.keys() {
            assert_eq!(cfg.get_runtime_value(key), reloaded.get_runtime_value(key));
        }
        assert_eq!(std::fs::read(path).unwrap(), persisted);
    }

    #[test]
    fn 保留中の連続保存は最初の実行値を保ち承諾時だけ最新版を反映する() {
        let (_directory, _path, cfg) = saved_config(serde_json::json!({
            "対象": "起動時",
            "変更なし": "起動時"
        }));
        for value in ["保存一回目", "保存二回目"] {
            let expected = cfg.value_revisions();
            assert!(cfg
                .set_all_checked_deferred(
                    serde_json::json!({"対象": value, "変更なし": "起動時"})
                        .as_object()
                        .unwrap()
                        .clone(),
                    &expected,
                    &["対象", "変更なし"],
                    |_, _| Ok(()),
                )
                .unwrap()
                .0
                .is_empty());
            assert_eq!(cfg.get("対象", ""), "起動時");
            assert_eq!(cfg.get_all()["対象"], value);
        }
        // 通常経路の追記でも保留を破らず、変更のない項目には保留を作らない。
        cfg.set_string("対象", "別経路の保存".into()).unwrap();
        cfg.set_string("変更なし", "即時反映".into()).unwrap();
        assert_eq!(cfg.get("対象", ""), "起動時");
        assert_eq!(cfg.get("変更なし", ""), "即時反映");
        cfg.activate_saved_values();
        assert_eq!(cfg.get("対象", ""), "別経路の保存");
        cfg.set_all_checked_deferred(
            serde_json::json!({"対象": "次の保存"})
                .as_object()
                .unwrap()
                .clone(),
            &cfg.value_revisions(),
            &["対象"],
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(cfg.get("対象", ""), "別経路の保存");
        cfg.activate_saved_values();
        assert_eq!(cfg.get("対象", ""), "次の保存");
    }

    #[test]
    fn 競合と検証失敗と実保存失敗では実行値の保留を追加しない() {
        for failure in ["競合", "検証失敗", "実保存失敗"] {
            let initial = serde_json::json!({"対象": "保存前"});
            let (_directory, path, cfg) = saved_config(initial.clone());
            let mut expected = cfg.value_revisions();
            if failure == "競合" {
                expected.insert("対象".into(), serde_json::json!("古い版"));
            }
            if failure == "実保存失敗" {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            let result = cfg.set_all_checked_deferred(
                serde_json::json!({"対象": "保存後"})
                    .as_object()
                    .unwrap()
                    .clone(),
                &expected,
                &["対象"],
                |_, _| {
                    if failure == "検証失敗" {
                        anyhow::bail!("保存候補を拒否");
                    }
                    Ok(())
                },
            );
            if failure == "競合" {
                assert_eq!(result.unwrap().0, ["対象"]);
            } else {
                assert!(result.is_err(), "{failure}を検知していない");
            }
            assert_eq!(cfg.get_all(), *initial.as_object().unwrap());
            assert_eq!(cfg.get("対象", ""), "保存前");
            if failure == "実保存失敗" {
                std::fs::remove_dir(&path).unwrap();
            }
            // 失敗した保存が保留を残すと、この通常保存が実行側へ反映されない。
            cfg.set_string("対象", "正常な保存".into()).unwrap();
            assert_eq!(cfg.get("対象", ""), "正常な保存", "{failure}");
            let reloaded = Config::new();
            reloaded.load(path).unwrap();
            assert_eq!(reloaded.get("対象", ""), "正常な保存");
        }
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
