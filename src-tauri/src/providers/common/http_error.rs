use std::fmt;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDateTime, Utc};

/// HTTP失敗の状態を、表示用の短い診断へ変換した後も保持する。
#[derive(Debug)]
pub struct ProviderHttpError {
    pub service: String,
    pub endpoint: String,
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

impl fmt::Display for ProviderHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match reqwest::StatusCode::from_u16(self.status) {
            Ok(status) => formatter.write_str(&crate::tools::http::service_error(
                &self.service,
                &self.endpoint,
                status,
                &self.body,
            )),
            Err(_) => write!(
                formatter,
                "{} が不正なHTTP状態を返しました（{} / HTTP {}）: {}",
                self.service, self.endpoint, self.status, self.body
            ),
        }
    }
}

impl std::error::Error for ProviderHttpError {}

/// 応答時刻を注入できるようにし、HTTP日時の残り時間を一定条件で検証する。
fn retry_after_at(raw: &str, now: SystemTime) -> Result<Duration, String> {
    let raw = raw.trim();
    if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        // 値が極端に長くても、ヘッダーの仕様通り非負秒として上限まで読む。
        let seconds = raw.bytes().fold(0u64, |value, byte| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(byte - b'0'))
                .min(30)
        });
        return Ok(Duration::from_secs(seconds));
    }
    let date = DateTime::parse_from_rfc2822(raw)
        .map(|date| date.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(raw, "%A, %d-%b-%y %H:%M:%S GMT")
                .map(|date| date.and_utc())
        })
        .or_else(|_| {
            NaiveDateTime::parse_from_str(raw, "%a %b %e %H:%M:%S %Y").map(|date| date.and_utc())
        })
        .map_err(|_| "Retry-Afterの値が非負秒またはHTTP日時ではありません".to_string())?;
    let wait = date.signed_duration_since(DateTime::<Utc>::from(now));
    Ok(if wait <= chrono::Duration::zero() {
        Duration::ZERO
    } else {
        wait.to_std()
            .map_err(|_| "Retry-Afterの待ち時間を扱えません".to_string())?
            .min(Duration::from_secs(30))
    })
}

pub async fn response_error(
    service: &str,
    endpoint: &str,
    response: reqwest::Response,
    secret_values: &[&str],
) -> ProviderHttpError {
    response_error_at(
        service,
        endpoint,
        response,
        secret_values,
        SystemTime::now(),
    )
    .await
}

async fn response_error_at(
    service: &str,
    endpoint: &str,
    response: reqwest::Response,
    secret_values: &[&str],
    now: SystemTime,
) -> ProviderHttpError {
    let status = response.status().as_u16();
    let parsed = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .map(|header| {
            header
                .to_str()
                .map_err(|_| "Retry-Afterの値を文字列として読めません".to_string())
                .and_then(|raw| retry_after_at(raw, now))
        });
    let (retry_after, header_error) = match parsed {
        Some(Ok(wait)) => (Some(wait), None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    let mut diagnostic = match crate::tools::http::response_text(response).await {
        Ok(body) => body,
        Err(error) => format!("エラー応答本文の取得に失敗しました: {error}"),
    };
    if let Some(error) = header_error {
        // 本文が長くてもヘッダーを読めなかった理由は診断へ残す。
        diagnostic = format!("{error}; {diagnostic}");
    }
    ProviderHttpError {
        service: service.into(),
        endpoint: endpoint.into(),
        status,
        retry_after,
        body: crate::text::preview(
            &crate::tools::http::redact_secret_values(&diagnostic, secret_values),
            200,
        ),
    }
}

pub fn send_error(error: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(error.without_url())
}

pub async fn checked_response(
    service: &str,
    endpoint: &str,
    response: reqwest::Response,
    secret_values: &[&str],
) -> anyhow::Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(anyhow::Error::new(
        response_error(service, endpoint, response, secret_values).await,
    ))
}

pub async fn provider_response(
    cfg: &crate::config::Config,
    service: &str,
    endpoint: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> anyhow::Result<reqwest::Response> {
    let response = response.map_err(send_error)?;
    let secrets = crate::tools::http::configured_secret_values(cfg);
    let values: Vec<&str> = secrets.iter().map(String::as_str).collect();
    checked_response(service, endpoint, response, &values).await
}

pub async fn provider_probe(
    cfg: &crate::config::Config,
    service: &str,
    endpoint: &str,
    request: reqwest::RequestBuilder,
) -> Result<bool, String> {
    probe_response(cfg, service, endpoint, request.send().await).await
}

/// 管理サービスは接続拒否なら停止中、503なら初期化中として起動監視を続ける。
pub async fn managed_health_probe(
    cfg: &crate::config::Config,
    service: &str,
    endpoint: &str,
    request: reqwest::RequestBuilder,
) -> Result<bool, String> {
    let response = request.send().await;
    match &response {
        Err(error) if connection_refused(error) => return Ok(false),
        Ok(response) if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE => {
            return Ok(false);
        }
        _ => {}
    }
    probe_response(cfg, service, endpoint, response)
        .await
        .map_err(|error| format!("{service}の疎通確認に失敗しました: {error}"))
}

fn connection_refused(error: &reqwest::Error) -> bool {
    use std::error::Error;
    let mut cause = error.source();
    while let Some(error) = cause {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
        {
            return true;
        }
        cause = error.source();
    }
    false
}

async fn probe_response(
    cfg: &crate::config::Config,
    service: &str,
    endpoint: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<bool, String> {
    provider_response(cfg, service, endpoint, response)
        .await
        .map(|_| true)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::compatible::test_server::Server;

    #[test]
    fn 再試行待ちは秒と日時を同じ上限で検査する() {
        let now = DateTime::parse_from_rfc3339("2026-10-11T00:00:00Z").unwrap();
        let clock: SystemTime = now.with_timezone(&Utc).into();
        for (header, seconds) in [
            ("0", 0),
            ("8", 8),
            ("99999999999999999999999999999999999999", 30),
            ("Sun, 11 Oct 2026 00:00:12 GMT", 12),
            ("Sun, 11 Oct 2026 00:01:00 GMT", 30),
            ("Sat, 10 Oct 2026 23:59:59 GMT", 0),
            ("Sunday, 11-Oct-26 00:00:12 GMT", 12),
            ("Sun Oct 11 00:00:12 2026", 12),
        ] {
            assert_eq!(
                retry_after_at(header, clock).unwrap().as_secs(),
                seconds,
                "{header}"
            );
        }
        for header in ["", "-1", "1.5", "未対応"] {
            assert!(retry_after_at(header, clock)
                .unwrap_err()
                .contains("Retry-After"));
        }
    }

    #[test]
    fn 型付きエラーは文脈を付けても状態を保持する() {
        let error = anyhow::Error::new(ProviderHttpError {
            service: "テストAI".into(),
            endpoint: "/chat".into(),
            status: 503,
            retry_after: Some(Duration::from_secs(8)),
            body: "一時停止".into(),
        })
        .context("利用者への応答を生成できません");
        let typed = error.downcast_ref::<ProviderHttpError>().unwrap();
        assert_eq!(typed.status, 503);
        assert_eq!(typed.retry_after, Some(Duration::from_secs(8)));
        assert!(typed.to_string().contains("HTTP 503"));
    }

    #[tokio::test]
    async fn エラー本文は秘密値を伏せてから短くし状態を保持する() {
        let secret = "秘密の\"token";
        let quoted = serde_json::to_string(secret).unwrap();
        let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
        let body = format!(
            "{secret}; {}; {encoded}; {}",
            &quoted[1..quoted.len() - 1],
            "長".repeat(300)
        );
        let server = Server::raw(format!("HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
        let response = crate::tools::http::client()
            .unwrap()
            .get(&server.url)
            .send()
            .await
            .unwrap();
        let error = response_error("検査AI", "/chat", response, &[secret]).await;
        assert_eq!(error.status, 429);
        for forbidden in [secret, &encoded] {
            assert!(!error.body.contains(forbidden));
        }
        assert!(!error.body.contains("token"));
        assert!(error.body.contains("秘密値を省略"));
        assert!(error.body.chars().count() <= 201);
        assert!(error.to_string().contains("HTTP 429"));
        assert_eq!(server.finish().len(), 1);
    }

    #[tokio::test]
    async fn 本文読込や待機ヘッダーの失敗でも状態型を失わない() {
        for (header, incomplete, expected_wait) in [
            (b"8".as_slice(), true, Some(Duration::from_secs(8))),
            (b"bad".as_slice(), false, None),
            (b"\xff".as_slice(), false, None),
        ] {
            let mut reply =
                b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nRetry-After: ".to_vec();
            reply.extend_from_slice(header);
            reply.extend_from_slice(if incomplete {
                b"\r\nContent-Length: 100\r\n\r\nshort"
            } else {
                b"\r\nContent-Length: 5\r\n\r\nshort"
            });
            let server = Server::raw(reply);
            let response = crate::tools::http::client()
                .unwrap()
                .get(&server.url)
                .send()
                .await
                .unwrap();
            let error = checked_response("検査AI", "/chat", response, &[])
                .await
                .unwrap_err()
                .context("生成失敗");
            let typed = error.downcast_ref::<ProviderHttpError>().unwrap();
            assert_eq!(typed.status, 503);
            assert_eq!(typed.retry_after, expected_wait);
            assert!(typed.body.contains(if incomplete {
                "本文の取得に失敗"
            } else {
                "Retry-After"
            }));
            assert_eq!(server.finish().len(), 1);
        }
    }

    #[tokio::test]
    async fn 管理サービスの応答待ち期限は停止中へ読み替えない() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let cfg = crate::config::Config::new();
        let result = managed_health_probe(
            &cfg,
            "検査エンジン",
            "/health",
            crate::tools::http::client()
                .unwrap()
                .get(format!("http://{address}/health"))
                .timeout(Duration::from_millis(100)),
        )
        .await;
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        let error = result.unwrap_err();
        assert!(error.contains("検査エンジンの疎通確認に失敗"));
        assert!(!error.contains(&address.to_string()));
    }

    #[tokio::test]
    async fn 疎通確認は非成功と接続失敗を理由で返し正常だけ成功する() {
        let cfg = crate::config::Config::new();
        for status in [200, 401, 500, 503] {
            let server = Server::json(status, &serde_json::json!({"error":"確認不可"}));
            let result = provider_probe(
                &cfg,
                "検査AI",
                "/models",
                crate::tools::http::client().unwrap().get(&server.url),
            )
            .await;
            if status == 200 {
                assert!(result.unwrap());
            } else {
                assert!(result.unwrap_err().contains(&format!("HTTP {status}")));
            }
            assert_eq!(server.finish().len(), 1);
        }
        let server = Server::raw(Vec::new());
        let result = provider_probe(
            &cfg,
            "検査AI",
            "/models",
            crate::tools::http::client().unwrap().get(&server.url),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(server.finish().len(), 1);
    }

    #[tokio::test]
    async fn 主要プロバイダの全要求が状態型を保持する() {
        use crate::providers::{
            anthropic::Anthropic, local::Local, ollama::Ollama, openai::OpenAI, Provider,
        };
        for (provider, url_key, secret_key) in [
            (
                &OpenAI as &dyn Provider,
                "OPENAI_BASE_URL",
                "OPENAI_COMPAT_API_KEY",
            ),
            (
                &Anthropic as &dyn Provider,
                "PICOAGENT_ANTHROPIC_BASE_URL",
                "PICOAGENT_ANTHROPIC_API_KEY",
            ),
            (&Ollama as &dyn Provider, "OLLAMA_HOST", ""),
            (&Local as &dyn Provider, "", ""),
        ] {
            let server = Server::json(503, &serde_json::json!({"error":"一時停止"}));
            let cfg = crate::config::Config::new();
            if !url_key.is_empty() {
                cfg.set(url_key, &server.url);
            }
            if !secret_key.is_empty() {
                cfg.set(secret_key, "テスト用秘密値");
            }
            if provider.id() == "local" {
                let url = reqwest::Url::parse(&server.url).unwrap();
                cfg.set("PICOAGENT_LOCAL_PORT", &url.port().unwrap().to_string());
            }
            let messages = [crate::message::Message::new("user", "検査")];
            let listener = std::sync::Arc::new(|_: String| panic!("失敗応答の本文を公開しない"));
            let results = [
                provider.list_models(&cfg).await.map(|_| ()),
                provider
                    .chat(&cfg, "model", &messages, &[])
                    .await
                    .map(|_| ()),
                provider
                    .chat_with_options(&cfg, "model", &messages, &[], true)
                    .await
                    .map(|_| ()),
                provider
                    .chat_stream(&cfg, "model", &messages, &[], listener)
                    .await
                    .map(|_| ()),
            ];
            for result in results {
                let error = result.unwrap_err();
                assert_eq!(
                    error.downcast_ref::<ProviderHttpError>().unwrap().status,
                    503,
                    "{}: {error:#}",
                    provider.id()
                );
            }
            assert_eq!(
                server.finish().len(),
                4,
                "失敗時に{}を別要求へ替えない",
                provider.id()
            );
        }
    }
}
