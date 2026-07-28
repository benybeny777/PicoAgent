// store.rs - アプリ状態ファイル（config.json / memory.json / usage.json / 会話履歴 …）の
// 永続化の共通層。
//
// 状態ファイルはどれも「読み込み → 全体を書き戻す」形で更新する。ここで
// `std::fs::write` を使うと、書き込み中にアプリが落ちる・電源が切れると**中身が
// 途中まで**のファイルが残る。各ストアは破損時に「空で継続」するフォールバックを
// 持っているため、次の保存でその空の状態がそのまま書き戻され、**設定や記憶が
// 恒久的に消える**。1回の中断が全損につながる作りだった。
//
// そこで書き込みは必ず「同じディレクトリの一時ファイルへ書く → fsync → rename」で
// 行う。rename は同一ファイルシステム上で原子的なので、どの瞬間に落ちても
// 「更新前の完全なファイル」か「更新後の完全なファイル」のどちらかしか残らない。
//
// 新しい状態ファイルを足すときも必ずここを通すこと。直接 `fs::write` していないかは
// `store::guard` のテストが走査して検出する。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;

/// 状態ファイルを原子的に書き換える。
///
/// 途中で落ちても更新前の内容が壊れない。書き込みに失敗した場合は一時ファイルを
/// 残さずに片付ける。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("保存先の親ディレクトリを特定できません"))?;
    std::fs::create_dir_all(dir)?;

    let temp = temp_path(path);
    let result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        // rename の前に中身をディスクへ確定させる。ここを省くと、rename だけが先に
        // 永続化されて「名前はあるが中身が空」のファイルが残りうる。
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// 状態を JSON（整形済み）として原子的に書き出す。
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    write_atomic(path, text.as_bytes())
}

/// 設定に JSON 文字列で保存された一覧を読む。**壊れていても空一覧にしない。**
///
/// 目覚まし・リマインダー・スケジュールなどは「読み込む → 要素を足す → 全体を書き戻す」
/// 形で更新する。読めなかったときに空一覧から始めると、**次の保存で登録済みの内容が
/// すべて消える**。空一覧と「読めなかった」を型で区別し、書き戻す側がエラーを
/// そのままユーザーへ返せるようにする。
pub fn load_list<T: serde::de::DeserializeOwned>(
    cfg: &Config,
    key: &str,
) -> Result<Vec<T>, String> {
    let raw = cfg.get(key, "");
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "保存済みの設定「{key}」を読み取れません（{error}）。\
             上書きで失わないよう操作を中止しました。設定画面から内容を直すか、\
             作り直す場合は設定画面で該当項目を空にしてください。"
        )
    })
}

/// 読み込めなかった状態ファイルを退避し、退避先を返す。
///
/// 破損したファイルをその場に残すと、次の保存（＝空の状態の書き戻し）で上書きされて
/// 原因調査もユーザーによる復旧もできなくなる。拡張子を変えて隣に残す。
pub fn quarantine_corrupt(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path.file_name()?.to_string_lossy().to_string();
    let target = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    std::fs::rename(path, &target).ok()?;
    Some(target)
}

fn temp_path(path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "state".to_string());
    // 同じディレクトリに置く（rename が原子的なのは同一ファイルシステム内だけ）。
    path.with_file_name(format!(".{name}.tmp-{}-{nanos}", std::process::id()))
}

/// 状態ファイルの書き込みが共通層を迂回していないかを走査するガード。
#[cfg(test)]
mod guard {
    /// `std::fs::write` の直接使用を許すファイルと、その理由。
    /// アプリ状態ファイルはここに足さず、`store::write_atomic` を使うこと。
    const RAW_WRITE_ALLOWED: [(&str, &str); 4] = [
        ("store.rs", "共通層の実装そのもの"),
        (
            "permissions.rs",
            "ユーザーが指定したファイルへの書き込みファサード（状態ファイルではない）",
        ),
        (
            "commands.rs",
            "一時 WAV とユーザー向けエクスポート（失われても再実行できる）",
        ),
        ("client.rs", "テスト用スタブスクリプトの生成"),
    ];

    use crate::guard_scan;

    #[test]
    fn 状態ファイルの書き込みは共通層を通す() {
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources() {
            if RAW_WRITE_ALLOWED
                .iter()
                .any(|(allowed, _)| *allowed == source.name)
            {
                continue;
            }
            // スキルが扱うのはユーザー指定ファイル。状態ストアの規則ではなく
            // permissions::fs のファサード規則（permissions::guard）の管轄。
            if source.is_skill() {
                continue;
            }
            if source.body().contains("fs::write(") {
                offenders.push(source.path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "状態ファイルは crate::store::write_atomic / write_json_atomic を使ってください: {offenders:?}"
        );
    }

    /// 「壊れていたら空で続ける」フォールバックは、そのまま書き戻すとデータが全損する。
    ///
    /// 破損を空一覧・空ストアへ落とす箇所は、必ず (1) 破損ファイルを
    /// `store::quarantine_corrupt()` で退避するか、(2) `store::load_list()` で
    /// エラーとして返して書き戻しを止めるか、のどちらかを通すこと。実際に、目覚まし・
    /// リマインダー・スケジュールは空一覧から始めてそのまま保存し、登録済みの内容を
    /// すべて消す作りだった。
    #[test]
    fn 破損を空で置き換える箇所は退避かエラー返却を伴う() {
        let mut offenders = Vec::new();
        for source in guard_scan::all_sources() {
            if source.name == "store.rs" {
                continue; // 退避・読み取りの定義元
            }
            let body = source.body();
            let silently_empty = body.contains("from_str(&text).unwrap_or_default()")
                || body.contains("from_str(&raw).unwrap_or_default()");
            if !silently_empty {
                continue;
            }
            if body.contains("quarantine_corrupt") || body.contains("store::load_list") {
                continue;
            }
            offenders.push(source.path.display().to_string());
        }
        assert!(
            offenders.is_empty(),
            "破損時に空で続ける箇所は store::quarantine_corrupt での退避か store::load_list でのエラー返却を伴わせてください\
             （そのまま書き戻すとユーザーのデータが恒久的に消えます）: {offenders:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{quarantine_corrupt, write_atomic, write_json_atomic};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("picoagent-store-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 上書きしても一時ファイルを残さない() {
        let dir = temp_dir("overwrite");
        let path = dir.join("state.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            left,
            vec!["state.json".to_string()],
            "一時ファイルが残っている"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 親ディレクトリが無くても作成して書ける() {
        let dir = temp_dir("mkdir");
        let path = dir.join("nested").join("deep").join("state.json");
        write_json_atomic(&path, &serde_json::json!({ "a": 1 })).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"a\""));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 破損ファイルは退避して次の保存で消えない() {
        let dir = temp_dir("quarantine");
        let path = dir.join("config.json");
        std::fs::write(&path, "{ 壊れた").unwrap();
        let saved = quarantine_corrupt(&path).expect("退避されるはず");
        assert!(saved.exists());
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "{ 壊れた");
        // 存在しないファイルには何もしない。
        assert!(quarantine_corrupt(&path).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
