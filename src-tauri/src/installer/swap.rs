// 配置の所有者。退避・公開・復元を同じ排他と復旧記録で扱う。
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

static LOCK: Mutex<()> = Mutex::new(());
static NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Serialize, Deserialize)]
struct Item {
    stage: String,
    destination: String,
    backup: String,
    had_old: bool,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    pid: u32,
    committed: bool,
    items: Vec<Item>,
}

fn name(path: &Path) -> Result<String> {
    let value = path
        .file_name()
        .and_then(|x| x.to_str())
        .context("配置名を読めません")?;
    anyhow::ensure!(
        !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':']),
        "配置名が不正です"
    );
    Ok(value.to_owned())
}

fn ordinary(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            super::fs::validate_normal_metadata(path, &meta)?;
            anyhow::ensure!(
                meta.is_dir(),
                "配置対象が通常のディレクトリではありません: {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn has_pending(parent: &Path, id: &str) -> Result<bool> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.starts_with(".swap-journal-") && x.ends_with(".json"))
        {
            let journal: Journal = serde_json::from_slice(&std::fs::read(path)?)?;
            if journal
                .items
                .iter()
                .any(|item| item.backup.starts_with(&format!(".swap-backup-{id}-")))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(crate) fn replace(stage: &Path, destination: &Path, id: &str) -> Result<Option<String>> {
    replace_all(&[(stage, destination, id)])
}
// 検証中は新版を公開するが旧版と未確定記録を保持する。排他はI/O区間だけに置く。
struct Transaction {
    parent: PathBuf,
    journal_path: PathBuf,
    journal: Journal,
    active: bool,
}
impl Transaction {
    fn begin(pairs: &[(&Path, &Path, &str)]) -> Result<Self> {
        let _guard = LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("配置排他が破損しました"))?;
        anyhow::ensure!(!pairs.is_empty(), "配置対象がありません");
        let parent = pairs[0].1.parent().context("配置の親がありません")?;
        let parent = super::fs::checked_directory(parent)?;
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let mut items = Vec::new();
        let mut used = std::collections::HashSet::new();
        for (stage, destination, id) in pairs {
            // キャラクターIDではなく、配置の所有名を検査する。
            let installation_key = id;
            anyhow::ensure!(
                !installation_key.is_empty()
                    && installation_key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "配置識別子が不正です"
            );
            anyhow::ensure!(
                stage
                    .parent()
                    .context("準備先の親がありません")?
                    .canonicalize()?
                    == parent
                    && destination
                        .parent()
                        .context("配置先の親がありません")?
                        .canonicalize()?
                        == parent,
                "準備先と配置先は同じ管理領域が必要です"
            );
            anyhow::ensure!(ordinary(stage)?, "準備済みの配置がありません");
            let stage = name(stage)?;
            let destination_name = name(destination)?;
            anyhow::ensure!(
                used.insert(stage.clone()) && used.insert(destination_name.clone()),
                "配置の組が重複しています"
            );
            let backup = format!(".swap-backup-{id}-{}-{nonce}", std::process::id());
            anyhow::ensure!(!parent.join(&backup).try_exists()?, "退避先が既にあります");
            items.push(Item {
                stage,
                destination: destination_name,
                backup,
                had_old: ordinary(destination)?,
            });
        }
        let journal_path =
            parent.join(format!(".swap-journal-{}-{nonce}.json", std::process::id()));
        let journal = Journal {
            pid: std::process::id(),
            committed: false,
            items,
        };
        crate::store::write_json_atomic(&journal_path, &journal)?;
        let publish = (|| -> Result<()> {
            for item in &journal.items {
                if item.had_old {
                    std::fs::rename(parent.join(&item.destination), parent.join(&item.backup))?;
                }
            }
            for item in &journal.items {
                std::fs::rename(parent.join(&item.stage), parent.join(&item.destination))?;
            }
            Ok(())
        })();
        if let Err(error) = publish {
            restore_uncommitted(&parent, &journal_path, &journal)
                .with_context(|| format!("配置失敗={error}。旧版復元に失敗しました"))?;
            return Err(error);
        }
        Ok(Self {
            parent,
            journal_path,
            journal,
            active: true,
        })
    }

    fn abort(mut self) -> Result<()> {
        let _guard = LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("配置排他が破損しました"))?;
        // 失敗時は記録を保持する。Dropで同じ復元を重ねない。
        self.active = false;
        restore_uncommitted(&self.parent, &self.journal_path, &self.journal)
    }

    fn commit(mut self) -> Result<Option<String>> {
        let _guard = LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("配置排他が破損しました"))?;
        self.journal.committed = true;
        if let Err(error) = crate::store::write_json_atomic(&self.journal_path, &self.journal) {
            self.journal.committed = false;
            return Err(error);
        }
        self.active = false;
        // 検証済み新版は維持する。清掃失敗は確定記録と画面通知へ残す。
        if let Err(error) = clean_committed(&self.parent, &self.journal)
            .and_then(|()| Ok(std::fs::remove_file(&self.journal_path)?))
        {
            return Ok(Some(format!(
                "配置済み旧版の清掃を次回へ残しました: {} ({error})",
                self.journal_path.display()
            )));
        }
        Ok(None)
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let result = (|| -> Result<()> {
            let _guard = LOCK
                .lock()
                .map_err(|_| anyhow::anyhow!("配置排他が破損しました"))?;
            restore_uncommitted(&self.parent, &self.journal_path, &self.journal)
        })();
        if let Err(error) = result {
            crate::log_paths::log_detached(&format!(
                "導入取消の旧版復元に失敗しました。復旧記録を保持します: {} ({error:#})",
                self.journal_path.display()
            ));
        }
    }
}
fn restore_uncommitted(parent: &Path, path: &Path, journal: &Journal) -> Result<()> {
    rollback(parent, journal).with_context(|| format!("復旧記録: {}", path.display()))?;
    std::fs::remove_file(path)
        .with_context(|| format!("旧版復元後の復旧記録を回収できません: {}", path.display()))?;
    Ok(())
}
fn replace_all(pairs: &[(&Path, &Path, &str)]) -> Result<Option<String>> {
    Transaction::begin(pairs)?.commit()
}
pub(crate) async fn replace_with_validation(
    stage: &Path,
    destination: &Path,
    id: &str,
    validate: impl std::future::Future<Output = Result<()>>,
) -> Result<Option<String>> {
    replace_all_validated(&[(stage, destination, id)], validate).await
}
pub(crate) async fn replace_pair_with_validation(
    first: (&Path, &Path, &str),
    second: (&Path, &Path, &str),
    validate: impl std::future::Future<Output = Result<()>>,
) -> Result<Option<String>> {
    replace_all_validated(&[first, second], validate).await
}
async fn replace_all_validated(
    pairs: &[(&Path, &Path, &str)],
    validate: impl std::future::Future<Output = Result<()>>,
) -> Result<Option<String>> {
    let transaction = Transaction::begin(pairs)?;
    if let Err(error) = validate.await {
        transaction
            .abort()
            .with_context(|| format!("配置後検証失敗={error:#}。旧版復元に失敗しました"))?;
        return Err(error);
    }
    transaction.commit()
}

static CLEANUP_FAILURE: crate::notice::FailureLatch = crate::notice::FailureLatch::new();
pub(crate) fn complete(app: &tauri::AppHandle, outcome: Result<Option<String>>) -> Result<()> {
    let warning = outcome?;
    CLEANUP_FAILURE.report(
        app,
        "installer-cleanup",
        "導入済み旧版の回収",
        warning.map_or(Ok(()), Err),
        "導入済み旧版の回収が復旧しました",
    );
    Ok(())
}

fn rollback(parent: &Path, journal: &Journal) -> Result<()> {
    let mut failures = Vec::new();
    for item in journal.items.iter().rev() {
        let stage = parent.join(&item.stage);
        let destination = parent.join(&item.destination);
        let backup = parent.join(&item.backup);
        let result = (|| -> Result<()> {
            let backup_exists = ordinary(&backup)?;
            if (backup_exists || !item.had_old) && ordinary(&destination)? && !ordinary(&stage)? {
                std::fs::rename(&destination, &stage)?;
            }
            if backup_exists {
                anyhow::ensure!(!ordinary(&destination)?, "旧版の復元先が占有されています");
                std::fs::rename(&backup, &destination)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            failures.push(format!("{}: {error}", backup.display()));
        }
    }
    anyhow::ensure!(failures.is_empty(), "{}", failures.join(" / "));
    Ok(())
}

fn clean_committed(parent: &Path, journal: &Journal) -> Result<()> {
    for item in &journal.items {
        anyhow::ensure!(
            ordinary(&parent.join(&item.destination))?,
            "公開済み配置がありません。退避先を保持します"
        );
    }
    for item in &journal.items {
        for path in [&item.backup, &item.stage] {
            let path = parent.join(path);
            if ordinary(&path)? {
                std::fs::remove_dir_all(path)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn recover(parent: &Path) -> Result<()> {
    let _guard = LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("配置排他が破損しました"))?;
    if !parent.try_exists()? {
        return Ok(());
    }
    let parent = super::fs::checked_directory(parent)?;
    let parent = parent.as_path();
    let entries = std::fs::read_dir(parent)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let filename = name(&path)?;
        if !filename.starts_with(".swap-journal-") || !filename.ends_with(".json") {
            continue;
        }
        anyhow::ensure!(
            entry.file_type()?.is_file(),
            "復旧記録が通常のファイルではありません"
        );
        let journal: Journal = serde_json::from_slice(&std::fs::read(&path)?)?;
        anyhow::ensure!(
            (1..=2).contains(&journal.items.len()),
            "復旧記録の配置数が不正です"
        );
        anyhow::ensure!(
            filename.starts_with(&format!(".swap-journal-{}-", journal.pid)),
            "復旧記録のプロセスIDが一致しません"
        );
        let mut unique = std::collections::HashSet::new();
        for item in &journal.items {
            for value in [&item.stage, &item.destination, &item.backup] {
                anyhow::ensure!(
                    name(Path::new(value))? == *value && unique.insert(value),
                    "復旧記録に管理領域外または重複パスがあります"
                );
            }
            anyhow::ensure!(
                [
                    "python",
                    "character-maker",
                    "whisper",
                    "video-wan",
                    "video-ltx",
                    "image-krea2",
                    "video-ltx25",
                    "media-runtime",
                    "aivis",
                    "engine-cuda",
                    "engine-rocm",
                    "engine-sycl"
                ]
                .contains(&item.destination.as_str()),
                "復旧記録の配置先が管理対象ではありません"
            );
            let id = item
                .destination
                .strip_prefix("engine-")
                .unwrap_or(&item.destination);
            let nonce = filename
                .strip_prefix(&format!(".swap-journal-{}-", journal.pid))
                .and_then(|x| x.strip_suffix(".json"))
                .context("復旧記録の識別子がありません")?;
            anyhow::ensure!(
                !nonce.is_empty() && nonce.bytes().all(|x| x.is_ascii_digit()),
                "復旧記録の識別子が不正です"
            );
            let stage_prefix = if item.destination.starts_with("engine-") {
                format!(".engine-{id}-stage-{}", journal.pid)
            } else {
                format!(".installing-{id}-{}", journal.pid)
            };
            let stage_valid = item.stage == stage_prefix
                || (id == "aivis"
                    && item
                        .stage
                        .strip_prefix(&(stage_prefix + "-"))
                        .is_some_and(|suffix| {
                            !suffix.is_empty() && suffix.bytes().all(|x| x.is_ascii_digit())
                        }));
            anyhow::ensure!(
                item.backup == format!(".swap-backup-{id}-{}-{nonce}", journal.pid) && stage_valid,
                "復旧記録の準備名または退避名が不正です"
            );
        }
        if process_alive(journal.pid)? {
            continue;
        }
        if journal.committed {
            clean_committed(parent, &journal)?;
        } else {
            rollback(parent, &journal)?;
            for item in &journal.items {
                let stage = parent.join(&item.stage);
                if ordinary(&stage)? {
                    std::fs::remove_dir_all(stage)?;
                }
            }
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) use crate::tools::process::process_alive;

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "picoagent-swap-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }
    fn tree(path: &Path, text: &str) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("value"), text).unwrap();
    }
    #[test]
    fn 単独と組の公開後は旧版と復旧記録が残らない() {
        let parent = setup();
        for id in ["one", "two"] {
            tree(&parent.join(format!("stage-{id}")), "new");
            tree(&parent.join(id), "old");
        }
        replace_all(&[
            (&parent.join("stage-one"), &parent.join("one"), "one"),
            (&parent.join("stage-two"), &parent.join("two"), "two"),
        ])
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(parent.join("one/value")).unwrap(),
            "new"
        );
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 2);
        tree(&parent.join("stage-one"), "next");
        replace(&parent.join("stage-one"), &parent.join("one"), "one").unwrap();
        assert_eq!(
            std::fs::read_to_string(parent.join("one/value")).unwrap(),
            "next"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
    fn python_pair(parent: &Path) -> [(PathBuf, PathBuf, &'static str); 2] {
        ["python", "character-maker"].map(|id| {
            let stage = parent.join(format!(".installing-{id}-{}", std::process::id()));
            let destination = parent.join(id);
            tree(&stage, "new");
            tree(&destination, "old");
            (stage, destination, id)
        })
    }
    fn assert_restored(parent: &Path, pairs: &[(PathBuf, PathBuf, &str)]) {
        for (stage, destination, id) in pairs {
            assert_eq!(
                std::fs::read_to_string(destination.join("value")).unwrap(),
                "old"
            );
            assert_eq!(std::fs::read_to_string(stage.join("value")).unwrap(), "new");
            assert!(!has_pending(parent, id).unwrap());
        }
    }
    #[tokio::test]
    async fn 配置後検証の失敗は単独と組の旧版を保持する() {
        for paired in [false, true] {
            let parent = setup();
            let pairs = python_pair(&parent);
            let refs: Vec<_> = pairs
                .iter()
                .take(if paired { 2 } else { 1 })
                .map(|(stage, destination, id)| (stage.as_path(), destination.as_path(), *id))
                .collect();
            let validation = async {
                // 待機中に他の復旧処理が排他へ入れても、生存中記録へ触れない。
                recover(&parent)?;
                for (_, destination, id) in &refs {
                    assert_eq!(std::fs::read_to_string(destination.join("value"))?, "new");
                    assert!(has_pending(&parent, id)?);
                }
                anyhow::bail!("配置先Python検査失敗")
            };
            let result = replace_all_validated(&refs, validation).await;
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("配置先Python検査失敗"));
            assert_restored(&parent, &pairs[..refs.len()]);
            std::fs::remove_dir_all(parent).unwrap();
        }
    }
    #[tokio::test]
    async fn 配置後検証の取消は両旧版を復元して未確定記録を回収する() {
        let parent = setup();
        let pairs = python_pair(&parent);
        let owned = pairs.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            replace_pair_with_validation(
                (&owned[0].0, &owned[0].1, owned[0].2),
                (&owned[1].0, &owned[1].1, owned[1].2),
                async move {
                    started.send(()).unwrap();
                    std::future::pending::<Result<()>>().await
                },
            )
            .await
        });
        ready.await.unwrap();
        recover(&parent).unwrap();
        for (_, destination, id) in &pairs {
            assert_eq!(
                std::fs::read_to_string(destination.join("value")).unwrap(),
                "new"
            );
            assert!(has_pending(&parent, id).unwrap());
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_restored(&parent, &pairs);
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[tokio::test]
    async fn 配置後検証の成功だけが旧版を回収する() {
        let parent = setup();
        let pairs = python_pair(&parent);
        replace_pair_with_validation(
            (&pairs[0].0, &pairs[0].1, pairs[0].2),
            (&pairs[1].0, &pairs[1].1, pairs[1].2),
            async {
                recover(&parent)?;
                Ok(())
            },
        )
        .await
        .unwrap();
        for (_, destination, id) in &pairs {
            assert_eq!(
                std::fs::read_to_string(destination.join("value")).unwrap(),
                "new"
            );
            assert!(!has_pending(&parent, id).unwrap());
        }
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 2);
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn 片側公開後の中断は両旧版を復元する() {
        let parent = setup();
        tree(&parent.join("one"), "new");
        tree(&parent.join("stage-two"), "new");
        tree(&parent.join(".swap-backup-one"), "old1");
        tree(&parent.join(".swap-backup-two"), "old2");
        let journal = Journal {
            pid: std::process::id(),
            committed: false,
            items: vec![
                Item {
                    stage: "stage-one".into(),
                    destination: "one".into(),
                    backup: ".swap-backup-one".into(),
                    had_old: true,
                },
                Item {
                    stage: "stage-two".into(),
                    destination: "two".into(),
                    backup: ".swap-backup-two".into(),
                    had_old: true,
                },
            ],
        };
        rollback(&parent, &journal).unwrap();
        assert_eq!(
            std::fs::read_to_string(parent.join("one/value")).unwrap(),
            "old1"
        );
        assert_eq!(
            std::fs::read_to_string(parent.join("two/value")).unwrap(),
            "old2"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn 終了済みの公開完了記録だけを清掃する() {
        let parent = setup();
        let pid = 2_147_483_647;
        tree(&parent.join("whisper"), "new");
        tree(&parent.join(format!(".swap-backup-whisper-{pid}-1")), "old");
        let journal = Journal {
            pid,
            committed: true,
            items: vec![Item {
                stage: format!(".installing-whisper-{pid}"),
                destination: "whisper".into(),
                backup: format!(".swap-backup-whisper-{pid}-1"),
                had_old: true,
            }],
        };
        crate::store::write_json_atomic(
            &parent.join(format!(".swap-journal-{pid}-1.json")),
            &journal,
        )
        .unwrap();
        recover(&parent).unwrap();
        assert_eq!(
            std::fs::read_to_string(parent.join("whisper/value")).unwrap(),
            "new"
        );
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
        std::fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn 壊れた記録や管理外の配置は消さずに停止する() {
        let parent = setup();
        let pid = 2_147_483_647;
        tree(&parent.join("private"), "protected");
        let path = parent.join(format!(".swap-journal-{pid}-1.json"));
        std::fs::write(&path, b"{").unwrap();
        assert!(recover(&parent).is_err());
        let journal = Journal {
            pid,
            committed: true,
            items: vec![Item {
                stage: ".installing-private-1".into(),
                destination: "private".into(),
                backup: ".swap-backup-private-1-1".into(),
                had_old: true,
            }],
        };
        crate::store::write_json_atomic(&path, &journal).unwrap();
        assert!(recover(&parent).is_err());
        assert_eq!(
            std::fs::read_to_string(parent.join("private/value")).unwrap(),
            "protected"
        );
        assert!(path.exists());
        std::fs::remove_dir_all(parent).unwrap();
    }
}
