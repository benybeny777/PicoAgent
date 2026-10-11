// download - 固定取得の上限・検証・一時ファイル・公開・進捗を一度だけ所有する。
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

static DOWNLOAD_NONCE: AtomicU64 = AtomicU64::new(0);

fn temporary_prefix(destination: &Path) -> Result<String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .context("取得先のファイル名を識別できません")?;
    // 長いカスタム名でも一時ファイルの名前上限を超えない固定長の宛先識別子にする。
    Ok(format!(
        ".pico-download-{}-",
        hex::encode(Sha256::digest(name.as_bytes()))
    ))
}

/// 同じ取得先だけを対象にし、所有PIDの終了を確認してから残物を回収する。
fn recover_download_parts(
    destination: &Path,
    alive: impl Fn(u32) -> Result<bool>,
) -> Result<usize> {
    let prefix = temporary_prefix(destination)?;
    let stem = destination
        .file_stem()
        .and_then(|name| name.to_str())
        .context("旧取得物のファイル名を識別できません")?;
    let legacy = format!("{stem}.part");
    let legacy_prefix = format!("{legacy}-");
    let parent = destination
        .parent()
        .context("取得先の親フォルダがありません")?;
    // 列挙・解析・生存確認が全部成功するまで、回収対象にも触れない。
    let mut candidates = Vec::new();
    let mut owners = std::collections::HashMap::new();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        // 呼出元が.partを正式な保存名に使う場合も、公開済み取得先は回収しない。
        if Some(name.as_os_str()) == destination.file_name() {
            continue;
        }
        let Some(name) = name.to_str() else {
            anyhow::bail!("取得先に識別できないファイル名があり、残物を保全しました");
        };
        let suffix = if let Some(suffix) = name.strip_prefix(&prefix) {
            let fields: Vec<_> = suffix.split('-').collect();
            anyhow::ensure!(
                fields.len() == 3
                    && !fields[2].is_empty()
                    && fields[2].bytes().all(|byte| byte.is_ascii_alphanumeric()),
                "取得途中ファイルの所有情報が不正なため保全しました"
            );
            (fields[0], fields[1])
        } else if let Some(suffix) = name.strip_prefix(&legacy_prefix) {
            let fields: Vec<_> = suffix.split('-').collect();
            anyhow::ensure!(
                fields.len() == 2,
                "旧取得途中ファイルの所有情報が不正なため保全しました"
            );
            (fields[0], fields[1])
        } else if name == legacy {
            anyhow::bail!("旧取得途中ファイルの所有PIDが不明なため保全しました");
        } else {
            continue;
        };
        anyhow::ensure!(
            !suffix.0.is_empty()
                && !suffix.1.is_empty()
                && suffix.0.bytes().all(|byte| byte.is_ascii_digit())
                && suffix.1.bytes().all(|byte| byte.is_ascii_digit()),
            "取得途中ファイルの所有情報が不正なため保全しました"
        );
        let pid: u32 = suffix
            .0
            .parse()
            .context("取得途中ファイルの所有PIDが不正なため保全しました")?;
        let _: u64 = suffix
            .1
            .parse()
            .context("取得途中ファイルの連番が不正なため保全しました")?;
        anyhow::ensure!(pid > 0, "取得途中ファイルの所有PIDが不正なため保全しました");
        let owner_alive = if let Some(&owner_alive) = owners.get(&pid) {
            owner_alive
        } else {
            let owner_alive = alive(pid)?;
            owners.insert(pid, owner_alive);
            owner_alive
        };
        anyhow::ensure!(
            !owner_alive,
            "取得途中ファイルの所有プロセスが実行中のため保全しました"
        );
        anyhow::ensure!(
            entry.file_type()?.is_file(),
            "取得途中ファイルが通常ファイルではないため保全しました"
        );
        super::fs::checked_target(&entry.path())?;
        candidates.push(entry.path());
    }
    let count = candidates.len();
    for path in candidates {
        std::fs::remove_file(path).context("終了済み取得途中ファイルを回収できません")?;
    }
    Ok(count)
}

pub(crate) struct FetchSpec<'a> {
    pub url: &'a str,
    pub destination: &'a Path,
    pub expected_size: Option<u64>,
    pub max_size: u64,
    pub sha256: Option<&'a str>,
}

struct ProgressThrottle {
    last: Instant,
    percent: u64,
}
impl ProgressThrottle {
    fn new() -> Self {
        Self {
            last: Instant::now(),
            percent: 0,
        }
    }
    fn ready(&mut self, received: u64, total: u64, now: Instant) -> bool {
        let percent = if total == 0 {
            0
        } else {
            ((received as u128 * 100) / total as u128) as u64
        };
        if now.duration_since(self.last) >= Duration::from_millis(250) || percent > self.percent {
            self.last = now;
            self.percent = percent;
            true
        } else {
            false
        }
    }
}

fn validate_spec(spec: &FetchSpec<'_>) -> Result<()> {
    let url =
        reqwest::Url::parse(spec.url).map_err(|_| anyhow::anyhow!("取得URLの形式が不正です"))?;
    anyhow::ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "取得URLは認証情報を含まないHTTPSで指定してください"
    );
    anyhow::ensure!(spec.max_size > 0, "取得可能な空き容量がありません");
    if let Some(size) = spec.expected_size {
        anyhow::ensure!(
            size > 0 && size <= spec.max_size,
            "取得物の期待サイズが取得上限を超えています"
        );
    }
    if let Some(hash) = spec.sha256 {
        anyhow::ensure!(
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "取得物のSHA-256形式が不正です"
        );
    }
    Ok(())
}

pub(crate) async fn fetch_fixed(
    spec: FetchSpec<'_>,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<()> {
    validate_spec(&spec)?;
    super::fs::checked_target(spec.destination)?;
    let response = super::http::client()
        .map_err(anyhow::Error::msg)?
        .get(spec.url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("取得要求に失敗: {}", error.without_url()))?;
    receive_fixed(spec, response, progress).await
}

async fn receive_fixed(
    spec: FetchSpec<'_>,
    mut response: reqwest::Response,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<()> {
    anyhow::ensure!(
        response.status().is_success(),
        "取得に失敗: HTTP {}",
        response.status().as_u16()
    );
    let declared = response.content_length();
    if let Some(size) = declared {
        anyhow::ensure!(
            size > 0 && size <= spec.max_size,
            "取得応答のサイズが取得上限を超えています"
        );
        if let Some(expected) = spec.expected_size {
            anyhow::ensure!(size == expected, "取得応答のサイズが期待値と一致しません");
        }
    }
    let total = spec.expected_size.or(declared).unwrap_or(0);
    let destination = super::fs::checked_target(spec.destination)?;
    let parent = destination
        .parent()
        .context("取得先の親フォルダがありません")?;
    // 作成と公開は同期完了し遅延作成を防ぐ。受信書込は従来通り非同期で処理する。
    // 固有名を所有するNamedTempFileは、取得future破棄時も作成済み一時ファイルを回収する。
    let prefix = format!(
        "{}{}-{}-",
        temporary_prefix(&destination)?,
        std::process::id(),
        DOWNLOAD_NONCE.fetch_add(1, Ordering::Relaxed)
    );
    let temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .context("取得途中ファイルを作れません")?;
    let result: Result<u64> = async {
        let mut file = tokio::fs::File::from_std(temporary.as_file().try_clone()?);
        let mut received = 0_u64;
        let mut hash = Sha256::new();
        let mut throttle = ProgressThrottle::new();
        progress(0, total);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| anyhow::anyhow!("取得本文を読めません: {}", error.without_url()))?
        {
            received = received
                .checked_add(chunk.len() as u64)
                .context("取得量が表現上限を超えました")?;
            anyhow::ensure!(
                received <= spec.max_size,
                "実際の取得量が取得上限を超えました"
            );
            if let Some(expected) = spec.expected_size.or(declared) {
                anyhow::ensure!(received <= expected, "実際の取得量が期待サイズを超えました");
            }
            file.write_all(&chunk)
                .await
                .context("取得途中ファイルへ書けません")?;
            hash.update(&chunk);
            if throttle.ready(received, total, Instant::now()) {
                progress(received, total);
            }
        }
        anyhow::ensure!(received > 0, "取得本文が空です");
        if let Some(expected) = spec.expected_size {
            anyhow::ensure!(
                received == expected,
                "実際の取得量が期待サイズと一致しません"
            );
        }
        if let Some(declared) = declared {
            anyhow::ensure!(
                received == declared,
                "実際の取得量が応答のサイズと一致しません"
            );
        }
        if let Some(expected) = spec.sha256 {
            anyhow::ensure!(
                hex::encode(hash.finalize()).eq_ignore_ascii_case(expected),
                "取得物のSHA-256が一致しません"
            );
        }
        file.flush()
            .await
            .context("取得途中ファイルを確定できません")?;
        drop(file);
        temporary
            .as_file()
            .sync_all()
            .context("取得途中ファイルを同期できません")?;
        let checked = super::fs::checked_target(spec.destination)?;
        anyhow::ensure!(checked == destination, "取得中に保存先が変更されました");
        Ok(received)
    }
    .await;
    let received = match result {
        Ok(received) => received,
        Err(original) => {
            if let Err(cleanup) = temporary.close() {
                anyhow::bail!(
                    "取得に失敗し一時ファイルも回収できません: {original}; 回収: {cleanup}"
                );
            }
            return Err(original);
        }
    };
    if let Err(error) = temporary.persist(&destination) {
        let original = error.error;
        if let Err(cleanup) = error.file.close() {
            anyhow::bail!("検証済み取得物を配置できず一時ファイルも回収できません: {original}; 回収: {cleanup}");
        }
        anyhow::bail!("検証済み取得物を配置できません: {original}");
    }
    progress(received, total);
    recover_download_parts(&destination, super::swap::process_alive)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::compatible::test_server::Server;

    #[tokio::test]
    async fn 検証失敗と本文中断では旧モデルだけを残し正常時だけ公開する() {
        let hash = hex::encode(Sha256::digest(b"new"));
        for (name, raw, expected, maximum, digest, succeeds) in [
            (
                "正常",
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nnew",
                Some(3),
                4,
                Some(hash.as_str()),
                true,
            ),
            (
                "サイズ違い",
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nnew",
                Some(4),
                4,
                None,
                false,
            ),
            (
                "ハッシュ違い",
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nnew",
                Some(3),
                4,
                Some("0000000000000000000000000000000000000000000000000000000000000000"),
                false,
            ),
            (
                "本文中断",
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nnew",
                Some(5),
                5,
                None,
                false,
            ),
            (
                "上限超過",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nnew\r\n0\r\n\r\n",
                None,
                2,
                None,
                false,
            ),
            (
                "未知サイズ",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nnew\r\n0\r\n\r\n",
                None,
                4,
                None,
                true,
            ),
            (
                "空",
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
                None,
                4,
                None,
                false,
            ),
            (
                "非成功",
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 6\r\n\r\nsecret",
                None,
                8,
                None,
                false,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("uv.zip.part");
            std::fs::write(&destination, b"old").unwrap();
            let abandoned = directory.path().join(format!(
                "{}2147483647-1-crash",
                temporary_prefix(&destination).unwrap()
            ));
            std::fs::write(&abandoned, b"resume").unwrap();
            let server = Server::raw(raw.as_bytes().to_vec());
            let response = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap()
                .get(&server.url)
                .send()
                .await
                .unwrap();
            let mut updates = Vec::new();
            let result = receive_fixed(
                FetchSpec {
                    url: "https://fixture.invalid",
                    destination: &destination,
                    expected_size: expected,
                    max_size: maximum,
                    sha256: digest,
                },
                response,
                &mut |received, total| updates.push((received, total)),
            )
            .await;
            assert_eq!(result.is_ok(), succeeds, "{name}: {result:?}");
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                if succeeds { b"new" } else { b"old" },
                "{name}"
            );
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                if succeeds { 1 } else { 2 },
                "{name}: 一時ファイル残存"
            );
            assert_eq!(
                abandoned.exists(),
                !succeeds,
                "{name}: 失敗時の旧取得資産保持"
            );
            if succeeds {
                assert_eq!(updates.last().unwrap().0, 3);
            }
            assert_eq!(server.finish().len(), 1);
        }
    }

    #[test]
    fn 不正な取得仕様は送信前に秘密を含まない理由で拒否する() {
        for (url, size, maximum, hash) in [
            ("http://fixture.invalid", Some(3), 4, None),
            ("https://user:secret@fixture.invalid", Some(3), 4, None),
            ("https://fixture.invalid", Some(5), 4, None),
            ("https://fixture.invalid", None, 0, None),
            ("https://fixture.invalid", None, 4, Some("secret")),
        ] {
            let error = validate_spec(&FetchSpec {
                url,
                destination: Path::new("model.gguf"),
                expected_size: size,
                max_size: maximum,
                sha256: hash,
            })
            .unwrap_err()
            .to_string();
            assert!(!error.contains("secret") && !error.contains("fixture.invalid"));
        }
    }

    #[test]
    fn 進捗は一割合または二百五十ミリ秒で通知する() {
        let mut throttle = ProgressThrottle::new();
        let start = throttle.last;
        let count = (1..=1000)
            .filter(|n| throttle.ready(n * 1000, 1_000_000, start))
            .count();
        assert_eq!(count, 100);
        let mut unknown = ProgressThrottle::new();
        let start = unknown.last;
        assert!(!unknown.ready(1000, 0, start + Duration::from_millis(249)));
        assert!(unknown.ready(2000, 0, start + Duration::from_millis(250)));
    }

    #[test]
    fn 終了済みの同宛先だけ回収し生存と未知所有と確認失敗を保全する() {
        for (suffix, alive, succeeds) in [
            ("2147483647-1-crash", Ok(false), true),
            ("123-1-live", Ok(true), false),
            ("unknown-1-crash", Ok(false), false),
            ("123-1-crash", Err("所有プロセスを確認できません"), false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("model.gguf");
            let owned = directory.path().join(format!(
                "{}{suffix}",
                temporary_prefix(&destination).unwrap()
            ));
            let unrelated = directory.path().join(".tmpunknown");
            let other = directory.path().join(format!(
                "{}2147483647-1-crash",
                temporary_prefix(Path::new("other.gguf")).unwrap()
            ));
            for path in [&owned, &unrelated, &other] {
                std::fs::write(path, b"retain").unwrap();
            }
            let result =
                recover_download_parts(&destination, |_| alive.map_err(anyhow::Error::msg));
            assert_eq!(result.is_ok(), succeeds, "{suffix}: {result:?}");
            assert_eq!(owned.exists(), !succeeds);
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"retain");
            assert_eq!(std::fs::read(&other).unwrap(), b"retain");
        }
        for name in [
            "model.part-123-1",
            "model.part-123-1-more",
            "model.part",
            "model.part-other",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("model.gguf");
            let part = directory.path().join(name);
            std::fs::write(&part, b"retain").unwrap();
            let result = recover_download_parts(&destination, |_| Ok(false));
            assert_eq!(result.is_ok(), name == "model.part-123-1", "{name}");
            assert_eq!(part.exists(), name != "model.part-123-1");
        }
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("model.gguf");
        let prefix = temporary_prefix(&destination).unwrap();
        let dead = directory.path().join(format!("{prefix}2147483647-1-crash"));
        let live = directory
            .path()
            .join(format!("{prefix}{}-2-live", std::process::id()));
        for path in [&dead, &live] {
            std::fs::write(path, b"retain").unwrap();
        }
        assert!(recover_download_parts(&destination, super::super::swap::process_alive).is_err());
        assert!(
            dead.exists() && live.exists(),
            "生存所有者の確認失敗時は列挙済み対象も保持する"
        );
        assert!(
            recover_download_parts(&directory.path().join("欠損/model.gguf"), |_| Ok(false))
                .is_err()
        );
    }

    #[test]
    fn 取得先がpart名でも公開済み本体を残して終了済み残物だけ回収する() {
        for filename in ["model.gguf", "model.part", "uv.zip.part", "uv.tar.gz.part"] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join(filename);
            std::fs::write(&destination, b"verified").unwrap();
            let abandoned = directory.path().join(format!(
                "{}123-1-crash",
                temporary_prefix(&destination).unwrap()
            ));
            std::fs::write(&abandoned, b"incomplete").unwrap();
            assert_eq!(
                recover_download_parts(&destination, |_| Ok(false)).unwrap(),
                1,
                "{filename}"
            );
            assert_eq!(std::fs::read(&destination).unwrap(), b"verified");
            assert!(!abandoned.exists());
        }
    }

    #[test]
    fn 同じ所有者の新旧二ファイルは生存確認を一回だけ行う() {
        for fails in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let destination = directory.path().join("model.gguf");
            let fresh = directory.path().join(format!(
                "{}123-1-crash",
                temporary_prefix(&destination).unwrap()
            ));
            let legacy = directory.path().join("model.part-123-2");
            for path in [&fresh, &legacy] {
                std::fs::write(path, b"retain").unwrap();
            }
            let calls = std::cell::Cell::new(0);
            let result = recover_download_parts(&destination, |pid| {
                assert_eq!(pid, 123);
                calls.set(calls.get() + 1);
                if fails {
                    anyhow::bail!("所有者の確認に失敗しました");
                }
                Ok(false)
            });
            assert_eq!(calls.get(), 1);
            if fails {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("所有者の確認に失敗"));
                assert!(fresh.exists() && legacy.exists());
            } else {
                assert_eq!(result.unwrap(), 2);
                assert!(!fresh.exists() && !legacy.exists());
            }
        }
    }

    #[tokio::test]
    async fn 取得future破棄は書込後の固有一時ファイルを回収し旧モデルを保持する() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("model.gguf");
        std::fs::write(&destination, b"old").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nn")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let target = destination.clone();
        let download = tokio::spawn(async move {
            receive_fixed(
                FetchSpec {
                    url: "https://fixture.invalid",
                    destination: &target,
                    expected_size: Some(100),
                    max_size: 100,
                    sha256: None,
                },
                response,
                &mut |received, _| {
                    if received > 0 {
                        signal.notify_one();
                    }
                },
            )
            .await
        });
        started.notified().await;
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
        download.abort();
        assert!(download.await.unwrap_err().is_cancelled());
        server.abort();
        let _ = server.await;
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
