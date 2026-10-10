// http.rs - 外向きHTTPの共有クライアントと設定値解決。

use std::sync::OnceLock;
use std::time::Duration;
use std::{net::IpAddr, net::SocketAddr};
use url::Url;

use crate::config::Config;

/// URL取得の許可方針。どちらも非公開IPへの接続を禁止する。
#[derive(Clone)]
pub enum UrlPolicy {
    PublicWeb,
    AllowedDomains(Vec<String>),
}

impl UrlPolicy {
    pub fn domains(text: &str) -> Self {
        Self::AllowedDomains(
            text.split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
        )
    }

    fn check(&self, url: &Url) -> Result<(), String> {
        validate_http_url(url)?;
        if let Self::AllowedDomains(domains) = self {
            let host = url.host_str().ok_or("URLにホスト名がありません")?;
            if !domains.iter().any(|domain| {
                host.eq_ignore_ascii_case(domain)
                    || host.to_ascii_lowercase().ends_with(&format!(".{domain}"))
            }) {
                return Err("許可されていないドメインです".into());
            }
        }
        Ok(())
    }
}

pub struct GuardedRequest {
    pub method: reqwest::Method,
    pub url: Url,
    pub headers: reqwest::header::HeaderMap,
    /// 任意名のAPIキーヘッダも、接続元が変われば除去する。
    pub secret_headers: Vec<reqwest::header::HeaderName>,
    pub body: Option<Vec<u8>>,
    pub policy: UrlPolicy,
    pub timeout: Duration,
    pub max_redirects: usize,
    pub max_bytes: usize,
    /// web_fetchの明示した先頭取得だけが有効にする。
    pub allow_truncation: bool,
    pub user_agent: &'static str,
}

impl GuardedRequest {
    pub fn get(
        url: Url,
        policy: UrlPolicy,
        timeout: Duration,
        max_bytes: usize,
        user_agent: &'static str,
    ) -> Self {
        Self {
            method: reqwest::Method::GET,
            url,
            headers: Default::default(),
            secret_headers: Vec::new(),
            body: None,
            policy,
            timeout,
            max_redirects: 5,
            max_bytes,
            allow_truncation: false,
            user_agent,
        }
    }
}

pub struct GuardedResponse {
    pub status: reqwest::StatusCode,
    pub url: Url,
    pub body: Vec<u8>,
    pub truncated: bool,
}

/// 全ホップで許可・公開IPを検査し、検査したDNS応答だけへ接続する。
pub async fn fetch_guarded(request: GuardedRequest) -> Result<GuardedResponse, String> {
    fetch_with_resolver(
        request,
        |url| async move { public_http_addresses(&url).await },
    )
    .await
}

async fn fetch_with_resolver<F, Fut>(
    request: GuardedRequest,
    resolve: F,
) -> Result<GuardedResponse, String>
where
    F: Fn(Url) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<SocketAddr>, String>>,
{
    let timeout = request.timeout;
    tokio::time::timeout(timeout, async {
        let mut request = request;
        for redirects in 0..=request.max_redirects {
            request.policy.check(&request.url)?;
            let addresses = resolve(request.url.clone()).await?;
            if addresses.is_empty() {
                return Err("接続先のIPアドレスが見つかりません".into());
            }
            let host = request.url.host_str().ok_or("URLにホスト名がありません")?;
            // プロキシが再名前解決する経路を使わず、検査済みの宛先へ直接接続する。
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(timeout)
                .user_agent(request.user_agent)
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| "HTTP取得の準備に失敗しました".to_string())?;
            let mut builder = client
                .request(request.method.clone(), request.url.clone())
                .headers(request.headers.clone());
            if let Some(body) = &request.body {
                builder = builder.body(body.clone());
            }
            let response = builder
                .send()
                .await
                .map_err(|error| format!("HTTP取得に失敗しました: {}", error.without_url()))?;
            let status = response.status();
            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects == request.max_redirects {
                    return Err(format!(
                        "リダイレクトが{}回を超えました",
                        request.max_redirects
                    ));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or("リダイレクト先がありません")?
                    .to_str()
                    .map_err(|_| "リダイレクト先が不正です")?;
                let next = request
                    .url
                    .join(location)
                    .map_err(|_| "リダイレクト先を解析できません")?;
                if request.url.scheme() == "https" && next.scheme() == "http" {
                    return Err("HTTPSからHTTPへのリダイレクトは使えません".into());
                }
                if request.url.origin() != next.origin() {
                    for name in [
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::COOKIE,
                        reqwest::header::PROXY_AUTHORIZATION,
                    ] {
                        request.headers.remove(name);
                    }
                    for name in &request.secret_headers {
                        request.headers.remove(name);
                    }
                    request.headers.remove("x-api-key");
                }
                if (status.as_u16() == 303 && request.method != reqwest::Method::HEAD)
                    || (matches!(status.as_u16(), 301 | 302)
                        && request.method == reqwest::Method::POST)
                {
                    request.method = reqwest::Method::GET;
                    request.body = None;
                    request.headers.remove(reqwest::header::CONTENT_TYPE);
                    request.headers.remove(reqwest::header::CONTENT_LENGTH);
                }
                request.url = next;
                continue;
            }
            let (body, truncated) =
                response_bytes(response, request.max_bytes, request.allow_truncation).await?;
            return Ok(GuardedResponse {
                status,
                url: request.url,
                body,
                truncated,
            });
        }
        Err("リダイレクトを完了できません".into())
    })
    .await
    .map_err(|_| "HTTP取得の待ち時間を超えました".to_string())?
}

fn validate_http_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("http または https のURLだけを指定できます".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("ユーザー情報を埋め込んだURLは使えません".into());
    }
    let host = url.host_str().ok_or("URLにホスト名がありません")?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err("ローカルホストへは接続できません".into());
    }
    if host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|ip| !is_public_ip(ip))
    {
        return Err("非公開IPへは接続できません".into());
    }
    Ok(())
}

async fn response_bytes(
    mut response: reqwest::Response,
    max_bytes: usize,
    allow_truncation: bool,
) -> Result<(Vec<u8>, bool), String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "外部サービスの応答本文を読み取れません: {}",
            error.without_url()
        )
    })? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            if allow_truncation {
                bytes.extend_from_slice(&chunk[..max_bytes.saturating_sub(bytes.len())]);
                return Ok((bytes, true));
            }
            return Err(format!(
                "外部サービスの応答本文が上限（{:.2} MB）を超えました",
                max_bytes as f64 / 1_000_000.0
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, false))
}

/// 応答本文へ反映された設定秘密値も、切り詰める前に伏せる。
pub fn configured_secret_values(cfg: &Config) -> Vec<String> {
    let mut keys: Vec<String> = cfg.get_all().keys().cloned().collect();
    keys.extend(std::env::vars_os().filter_map(|(key, _)| key.into_string().ok()));
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|key| crate::secret::is_secret_key(key))
        .map(|key| cfg.get(&key, ""))
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn redact_secret_values(body: &str, secrets: &[&str]) -> String {
    let mut safe = body.to_string();
    let mut ordered: Vec<&str> = secrets.to_vec();
    ordered.sort_by_key(|value| std::cmp::Reverse(value.len()));
    ordered.dedup();
    for value in ordered {
        if value.is_empty() {
            continue;
        }
        safe = safe.replace(value, "[秘密値を省略]");
        if let Ok(encoded) = serde_json::to_string(&value) {
            safe = safe.replace(&encoded[1..encoded.len() - 1], "[秘密値を省略]");
        }
        let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        safe = safe.replace(&encoded, "[秘密値を省略]");
    }
    safe
}

pub fn safe_service_error(
    cfg: &Config,
    label: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> String {
    let secrets = configured_secret_values(cfg);
    let values: Vec<&str> = secrets.iter().map(String::as_str).collect();
    service_error(label, "API", status, &redact_secret_values(body, &values))
}

pub async fn service_reply(
    cfg: &Config,
    response: Result<reqwest::Response, reqwest::Error>,
    label: &str,
) -> String {
    match service_body(cfg, label, response).await {
        Ok(body) => body,
        Err(error) => format!("エラー: {error}"),
    }
}

/// 接続・状態・エラー本文の検証を、JSONと音声の応答で共通にする。
pub async fn service_response(
    cfg: &Config,
    label: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<reqwest::Response, String> {
    let response = response
        .map_err(|error| format!("{label} への接続に失敗しました: {}", error.without_url()))?;
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        let body = response_text(response).await?;
        Err(safe_service_error(cfg, label, status, &body))
    }
}

pub async fn service_body(
    cfg: &Config,
    label: &str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<String, String> {
    response_text(service_response(cfg, label, response).await?).await
}

type SharedClient = OnceLock<Result<reqwest::Client, String>>;
static CLIENT: SharedClient = OnceLock::new();

/// 接続確立の上限。応答本文には掛からないので、モデルDLや生成のストリーミングは妨げない。
/// 到達しないホストでTCP接続がOS既定（分単位）まで滞留するのを防ぐ。
const CONNECT_TIMEOUT_SECS: u64 = 10;
/// 接続後に1バイトも届かない状態の上限。長時間ストリーム全体の時間は制限しない。
const READ_TIMEOUT_SECS: u64 = 120;
const MAX_SERVICE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// 専用のUser-Agent、DNS固定、リダイレクト制御等が不要な経路で接続プールを共有する標準クライアント。
/// 個別の待ち時間はRequestBuilder::timeoutで指定する。
pub fn init() -> Result<(), String> {
    initialize_client(&CLIENT, || {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(READ_TIMEOUT_SECS))
            .build()
            .map_err(|error| {
                format!(
                    "標準HTTPクライアントを初期化できません: {}",
                    error.without_url()
                )
            })
    })
}

fn initialize_client(
    state: &SharedClient,
    build: impl FnOnce() -> Result<reqwest::Client, String>,
) -> Result<(), String> {
    state
        .get_or_init(build)
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

fn initialized_client(state: &SharedClient) -> Result<&reqwest::Client, String> {
    state
        .get()
        .ok_or_else(|| "標準HTTPクライアントが未初期化です。起動時の初期化が必要です".to_string())?
        .as_ref()
        .map_err(Clone::clone)
}

/// 起動時に確定した接続プールだけを返す。初期化失敗後に別クライアントを作らない。
pub fn client() -> Result<&'static reqwest::Client, String> {
    // 単体検査はTauri setupを実行しないため、同じ起動時初期化を検査入口で呼ぶ。
    // 未初期化・初期化失敗の境界は独立したSharedClientで同じ関数を検査する。
    if cfg!(test) {
        init()?;
    }
    initialized_client(&CLIENT)
}

/// URL取得スキルがローカル端末や社内ネットワークをSSRFで読まないための宛先検査。
/// ホスト名は接続前に名前解決し、1件でも非公開IPへ向く場合は拒否する。
pub async fn require_public_http_url(url: &Url) -> Result<(), String> {
    public_http_addresses(url).await.map(|_| ())
}

/// 検査済みの接続先を返す。呼び出し側がHTTPクライアントへこの結果を固定すれば、
/// 検査後の再名前解決でプライベートIPへ差し替わるDNS rebindingも防げる。
pub async fn public_http_addresses(url: &Url) -> Result<Vec<SocketAddr>, String> {
    validate_http_url(url)?;
    let host = url.host_str().ok_or("URLにホスト名がありません")?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URLのポートを判定できません".to_string())?;
    let addresses: Vec<SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        _ => tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| "接続先を名前解決できません".to_string())?
            .collect(),
    };
    if addresses.is_empty() {
        return Err("接続先のIPアドレスが見つかりません".to_string());
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err("ローカル・プライベート・予約済みネットワークへは接続できません".to_string());
    }
    Ok(addresses)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let s = ip.segments();
            // IANAの公開例外を保持し、文書・試験・未割当の特殊宛先を拒否する。
            // https://www.iana.org/assignments/iana-ipv6-special-registry
            if s.starts_with(&[0x64, 0xff9b, 0, 0, 0, 0]) {
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::from(
                    (u32::from(s[6]) << 16) | u32::from(s[7]),
                )));
            }
            if s[0] == 0x2002 {
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::from(
                    (u32::from(s[1]) << 16) | u32::from(s[2]),
                )));
            }
            if s[0] == 0x2001 && s[1] < 0x200 {
                let anycast = s.starts_with(&[0x2001, 1, 0, 0, 0, 0, 0]) && (1..=3).contains(&s[7]);
                return anycast
                    || s[1] == 3
                    || (s[1] == 4 && s[2] == 0x112)
                    || (s[1] & 0xfff0 == 0x20)
                    || (s[1] & 0xfff0 == 0x30);
            }
            (0x2000..0x4000).contains(&s[0])
                && !(s[0] == 0x2001 && s[1] == 0xdb8)
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

/// モデル一覧のような短い問い合わせの待ち時間。設定画面のプロバイダ選択が固まらないようにする。
pub fn metadata_timeout(cfg: &Config) -> Result<Duration, String> {
    cfg.duration_secs(&crate::config::ints::HTTP_METADATA_TIMEOUT_SECS)
}

/// 外部サービスの応答本文を、読取失敗を空本文へ落とさず取得する。
pub async fn response_text(response: reqwest::Response) -> Result<String, String> {
    let (bytes, _) = response_bytes(response, MAX_SERVICE_RESPONSE_BYTES, false).await?;
    String::from_utf8(bytes)
        .map_err(|error| format!("外部サービスの応答本文がUTF-8ではありません: {error}"))
}

/// 外部サービスがエラー応答を返したときの、利用者へ出す共通メッセージ。
///
/// この文字列は**キャラクターのフキダシにそのまま出る**。以前は各プロバイダが
/// `"Ollama /api/chat failed: {status} {body}"` のように英語の定型文と応答本文を
/// 無制限に連結しており、日本語の利用者に英語＋JSONの塊が見えていた。
/// 本文は原因の手がかりとして残すが、長さを抑える。
pub fn service_error(
    service: &str,
    endpoint: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> String {
    let detail = crate::text::preview(body.trim(), 200);
    if detail.is_empty() {
        format!("{service} がエラーを返しました（{endpoint} / HTTP {status}）")
    } else {
        format!("{service} がエラーを返しました（{endpoint} / HTTP {status}）: {detail}")
    }
}

/// 外部サービスのJSONを、解析失敗を空オブジェクトへ落とさず取得する。
pub fn response_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text)
        .map_err(|error| format!("外部サービスの応答JSONを解析できません: {error}"))
}

/// `Tool::run()` の `String` 戻り値で、本文読取失敗をその場でエラー文字列へ変換する。
#[macro_export]
macro_rules! tool_response_text {
    ($response:expr) => {
        match $crate::tools::http::response_text($response).await {
            Ok(body) => body,
            Err(error) => return format!("エラー: {error}"),
        }
    };
}

/// `Tool::run()` の `String` 戻り値で、不正JSONをその場でエラー文字列へ変換する。
#[macro_export]
macro_rules! tool_response_json {
    ($text:expr) => {
        match $crate::tools::http::response_json($text) {
            Ok(value) => value,
            Err(error) => return format!("エラー: {error}"),
        }
    };
}

#[cfg(test)]
mod guarded_tests {
    use super::*;

    #[test]
    fn 未初期化と初期化失敗は通信を始めず再生成もしない() {
        let state = SharedClient::new();
        assert!(initialized_client(&state).unwrap_err().contains("未初期化"));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..2 {
            let error = initialize_client(&state, || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("TLS初期化の試験失敗".into())
            })
            .unwrap_err();
            assert_eq!(error, "TLS初期化の試験失敗");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            initialized_client(&state).unwrap_err(),
            "TLS初期化の試験失敗"
        );
    }

    #[test]
    fn 起動時に標準接続プールを一度だけ確定する() {
        init().unwrap();
        let first = client().unwrap();
        init().unwrap();
        assert!(std::ptr::eq(first, client().unwrap()));
        let sources = crate::guard_scan::all_sources();
        let source = sources
            .iter()
            .find(|source| {
                source.path.to_string_lossy().replace('\\', "/") == "src-tauri/src/lib.rs"
            })
            .unwrap();
        let setup = source.body().split(".setup(|app| {").nth(1).unwrap();
        assert!(
            setup.find("tools::http::init()").unwrap() < setup.find("components::init(").unwrap()
        );
    }
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(
        reply: String,
    ) -> (
        SocketAddr,
        Arc<Mutex<Vec<String>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let output = seen.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let size = stream.read(&mut buffer).await.unwrap();
                    if size == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..size]);
                    if let Some(header_end) =
                        bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    {
                        let header = String::from_utf8_lossy(&bytes[..header_end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|s| s.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                }
                output
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(bytes).unwrap());
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (address, seen, task)
    }

    fn request(address: SocketAddr) -> GuardedRequest {
        GuardedRequest::get(
            Url::parse(&format!("http://allowed.test:{}/", address.port())).unwrap(),
            UrlPolicy::domains("allowed.test"),
            Duration::from_secs(2),
            128,
            "PicoAgent test",
        )
    }

    #[tokio::test]
    async fn 禁止リダイレクト先には要求が一度も届かない() {
        let (denied, hits, target_task) =
            server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into())
                .await;
        for code in [302, 307, 308] {
            for host in ["denied.test", "127.0.0.1"] {
                let (allowed, source_hits, source_task) = server(format!("HTTP/1.1 {code} Redirect\r\nLocation: http://{host}:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", denied.port())).await;
                let mut req = request(allowed);
                req.method = reqwest::Method::POST;
                req.body = Some(b"private payload".to_vec());
                let result = fetch_with_resolver(req, |url| async move {
                    Ok(vec![SocketAddr::from((
                        [127, 0, 0, 1],
                        url.port().unwrap(),
                    ))])
                })
                .await;
                assert!(result.is_err());
                assert_eq!(source_hits.lock().unwrap().len(), 1);
                assert!(hits.lock().unwrap().is_empty());
                source_task.abort();
                let _ = source_task.await;
            }
        }
        // 実スキルの入口も、明示許可ドメインだけではプライベートIPを許さない。
        let cfg = Config::new();
        cfg.set("PICOAGENT_HTTP_ALLOWED_DOMAINS", "127.0.0.1");
        let output = crate::tools::http_api::HttpApiTool
            .run(
                &cfg,
                serde_json::json!({"method":"GET","url":format!("http://{denied}/")}),
            )
            .await;
        assert!(output.contains("非公開IP"));
        assert!(hits.lock().unwrap().is_empty());
        target_task.abort();
        let _ = target_task.await;
    }

    #[tokio::test]
    async fn dns固定と秘密ヘッダ除去と本文の規定を検査する() {
        for (code, host) in [
            (302, "other.test"),
            (303, "other.test"),
            (307, "other.test"),
            (308, "other.test"),
            (307, "allowed.test"),
        ] {
            let (target, hits, target_task) = server(
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into(),
            )
            .await;
            let (source, _, source_task) = server(format!("HTTP/1.1 {code} Redirect\r\nLocation: http://{host}:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.port())).await;
            let mut req = request(source);
            req.policy = UrlPolicy::domains("allowed.test,other.test");
            req.method = reqwest::Method::POST;
            req.body = Some(b"payload".to_vec());
            for name in ["authorization", "cookie", "x-custom-token"] {
                req.headers.insert(
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    "secret".parse().unwrap(),
                );
            }
            req.secret_headers.push("x-custom-token".parse().unwrap());
            let resolved = Arc::new(Mutex::new(Vec::new()));
            let calls = resolved.clone();
            let output = fetch_with_resolver(req, move |url| {
                calls
                    .lock()
                    .unwrap()
                    .push(url.host_str().unwrap().to_string());
                async move {
                    Ok(vec![SocketAddr::from((
                        [127, 0, 0, 1],
                        url.port().unwrap(),
                    ))])
                }
            })
            .await
            .unwrap();
            assert_eq!(output.body, b"ok");
            assert_eq!(*resolved.lock().unwrap(), ["allowed.test", host]);
            {
                let hits = hits.lock().unwrap();
                let sent = &hits[0];
                assert!(!sent.to_ascii_lowercase().contains("secret"));
                if code < 307 {
                    assert!(sent.starts_with("GET "));
                    assert!(!sent.ends_with("payload"));
                } else {
                    assert!(sent.starts_with("POST "));
                    assert!(sent.ends_with("payload"));
                }
            }
            source_task.abort();
            target_task.abort();
            let _ = source_task.await;
            let _ = target_task.await;
        }
    }

    #[tokio::test]
    async fn 全体時間と容量とリダイレクト上限を守る() {
        let (address, _, task) =
            server("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlong".into())
                .await;
        let mut req = request(address);
        req.max_bytes = 2;
        assert!(
            fetch_with_resolver(req, |_| async move { Ok(vec![address]) })
                .await
                .err()
                .unwrap()
                .contains("上限")
        );
        let mut prefix = request(address);
        prefix.max_bytes = 2;
        prefix.allow_truncation = true;
        let response = fetch_with_resolver(prefix, |_| async move { Ok(vec![address]) })
            .await
            .unwrap();
        assert_eq!(response.body, b"lo");
        assert!(response.truncated);
        let mut req = request(address);
        req.timeout = Duration::from_millis(20);
        assert!(fetch_with_resolver(req, |_| async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(vec![address])
        })
        .await
        .err()
        .unwrap()
        .contains("待ち時間"));
        task.abort();
        let _ = task.await;
        let (address, hits, task) = server("HTTP/1.1 302 Redirect\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()).await;
        let mut req = request(address);
        req.max_redirects = 1;
        assert!(
            fetch_with_resolver(req, |_| async move { Ok(vec![address]) })
                .await
                .err()
                .unwrap()
                .contains("リダイレクト")
        );
        assert_eq!(hits.lock().unwrap().len(), 2);
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn エラー本文を読み秘密値を切り詰める前に除去する() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_HTTP_BEARER_TOKEN", "unique-secret-123");
        let boundary = safe_service_error(
            &cfg,
            "API",
            reqwest::StatusCode::UNAUTHORIZED,
            &format!("{}unique-secret-123", "a".repeat(190)),
        );
        assert!(!boundary.contains("unique-"));
        cfg.set("PICOAGENT_HTTP_API_KEY_VALUE", "quoted\"secret\n");
        let secret = cfg.get("PICOAGENT_HTTP_API_KEY_VALUE", "");
        let escaped = serde_json::to_string(&secret).unwrap();
        let encoded: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
        let redacted = safe_service_error(
            &cfg,
            "API",
            reqwest::StatusCode::BAD_REQUEST,
            &format!("{escaped} {encoded}"),
        );
        assert!(!redacted.contains("quoted"));
        let body = "権限不足 unique-secret-123";
        let reply = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (address, _, task) = server(reply).await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let text = service_reply(
            &cfg,
            client
                .get(format!("http://{address}/?token=unique-secret-123"))
                .send()
                .await,
            "API",
        )
        .await;
        assert!(text.contains("権限不足"));
        assert!(text.contains("401"));
        assert!(!text.contains("unique-secret-123"));
        task.abort();
        let _ = task.await;
        // 接続不能なURLのqueryにも秘密値を入れ、reqwestのURL表示が除去されることを確かめる。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let unavailable = listener.local_addr().unwrap();
        drop(listener);
        let error = service_reply(
            &cfg,
            client
                .get(format!("http://{unavailable}/?token=unique-secret-123"))
                .send()
                .await,
            "API",
        )
        .await;
        assert!(error.contains("エラー"));
        assert!(!error.contains("unique-secret-123"));
        assert!(!error.contains("?token="));
    }
    use crate::tools::Tool;
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    #[test]
    fn 待ち時間の不正値を理由付きで返す() {
        let cfg = crate::config::Config::new();
        for value in ["999", "broken", "2", "-1", "3.5"] {
            cfg.set("PICOAGENT_HTTP_METADATA_TIMEOUT_SECS", value);
            let error = super::metadata_timeout(&cfg).unwrap_err();
            assert!(error.contains("PICOAGENT_HTTP_METADATA_TIMEOUT_SECS"));
        }
        cfg.set("PICOAGENT_HTTP_METADATA_TIMEOUT_SECS", "3");
        assert_eq!(super::metadata_timeout(&cfg).unwrap().as_secs(), 3);
        assert!(cfg.duration_key("TEST_TIMEOUT").is_err());
    }

    #[test]
    fn 不正な応答jsonを空オブジェクトへ落とさない() {
        let error = super::response_json("not-json").expect_err("不正JSONはエラーにする");
        assert!(error.contains("応答JSONを解析できません"));
    }

    #[test]
    fn ssrf向けに非公開ipを拒否する() {
        assert!(!super::is_public_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!super::is_public_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!super::is_public_ip(IpAddr::V4(Ipv4Addr::new(
            100, 64, 0, 1
        ))));
        assert!(!super::is_public_ip(IpAddr::V4(Ipv4Addr::new(
            198, 51, 100, 1
        ))));
        assert!(!super::is_public_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!super::is_public_ip(IpAddr::V6(
            Ipv4Addr::new(127, 0, 0, 1).to_ipv6_mapped()
        )));
        assert!(super::is_public_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        for (address, public) in [
            ("2001:2::1", false),
            ("3fff:fff::1", false),
            ("2001::1", false),
            ("2002:7f00:1::1", false),
            ("64:ff9b::7f00:1", false),
            ("2002:808:808::1", true),
            ("64:ff9b::808:808", true),
            ("2001:1::1", true),
            ("2001:3::1", true),
            ("2001:4:112::1", true),
            ("2001:20::1", true),
            ("2001:30::1", true),
            ("2606:4700::1111", true),
        ] {
            assert_eq!(
                super::is_public_ip(address.parse().unwrap()),
                public,
                "{address}"
            );
        }
    }

    #[tokio::test]
    async fn ipv6リテラルは角括弧を名前解決へ渡さない() {
        let url = url::Url::parse("https://[2606:4700::1111]/").unwrap();
        let addresses = super::public_http_addresses(&url).await.unwrap();
        assert_eq!(addresses, vec!["[2606:4700::1111]:443".parse().unwrap()]);
    }
}

/// 設計ガードは `guard` モジュールへ置く（`::guard::` で一覧できないと、
/// ガードを数えるときに見落とす。実際にこのガードだけ `tests` に紛れていた）。
#[cfg(test)]
mod guard {
    #[test]
    fn 安全取得とapiエラー応答は共有層で管理する() {
        for body in [
            include_str!("../../../skills/web_fetch/mod.rs"),
            include_str!("../../../skills/rss_feed/mod.rs"),
            include_str!("../../../skills/http_api/mod.rs"),
        ] {
            assert!(body.contains("http::fetch_guarded("));
            assert!(!body.contains(".send()"));
            assert!(!body.contains("public_http_addresses("));
        }
        let duplicates: Vec<_> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| source.body().contains("fn handle_text_response("))
            .map(|source| source.path.clone())
            .collect();
        assert!(
            duplicates.is_empty(),
            "API応答はservice_reply/service_bodyを使ってください: {duplicates:?}"
        );
    }

    fn client_builder_timeout_violations(text: &str) -> Vec<(usize, String)> {
        const BUILDER: &str = "Client::builder()";
        let mut violations = Vec::new();
        let mut search_start = 0;

        while let Some(relative_start) = text[search_start..].find(BUILDER) {
            let start = search_start + relative_start;
            let after_builder = &text[start + BUILDER.len()..];
            let line = text[..start].bytes().filter(|byte| *byte == b'\n').count() + 1;
            let Some(relative_end) = after_builder.find(".build(") else {
                violations.push((line, "対応する.build()が見つかりません".to_string()));
                break;
            };
            let chain = &after_builder[..relative_end];
            let has_total_timeout = chain.contains(".timeout(");
            let has_stall_timeouts =
                chain.contains(".connect_timeout(") && chain.contains(".read_timeout(");
            if !has_total_timeout && !has_stall_timeouts {
                violations.push((
                    line,
                    "要求全体timeout、または接続＋無通信timeoutがありません".to_string(),
                ));
            }
            search_start = start + BUILDER.len() + relative_end + ".build(".len();
        }

        violations
    }

    #[test]
    fn httpクライアントは共有または有限待ち時間つきで構築する() {
        // `reqwest::get()` は既定クライアントを毎回作り直すうえ待ち時間も付かないため、
        // `Client::new()` と同じ違反として検出する（この穴を通って ollama の
        // list_models が待ち時間なしのまま残っていた）。
        const BANNED: [&str; 2] = ["reqwest::Client::new()", "reqwest::get("];
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| !source.path.ends_with("tools/http.rs"))
            .flat_map(|source| {
                BANNED
                    .iter()
                    .filter(|banned| source.text.contains(**banned))
                    .map(|banned| format!("{} ({banned})", source.path.display()))
                    .collect::<Vec<String>>()
            })
            .collect();
        assert!(
            violations.is_empty(),
            "標準HTTPはtools::http::client()を共有してください:\n{}",
            violations.join("\n")
        );

        let builder_violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .flat_map(|source| {
                // このファイルのガード用fixture文字列は実装ではない。共有クライアント本体は
                // 最初のcfg(test)より前にあるため、そこまでは他ファイルと同じ規則で検査する。
                let implementation = if source.path.ends_with("tools/http.rs") {
                    source.text.split("#[cfg(test)]").next().unwrap_or("")
                } else {
                    source.text.as_str()
                };
                client_builder_timeout_violations(implementation)
                    .into_iter()
                    .map(|(line, reason)| format!("{}:{line} ({reason})", source.path.display()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            builder_violations.is_empty(),
            "個別HTTPクライアントには要求全体timeout、または長時間処理向けの接続＋無通信timeoutを設定してください:\n{}",
            builder_violations.join("\n")
        );
    }

    #[test]
    fn 個別builderの上限検査は抜けを検出する() {
        assert!(
            client_builder_timeout_violations("Client::builder().timeout(duration).build()")
                .is_empty()
        );
        assert!(client_builder_timeout_violations(
            "reqwest::Client::builder().connect_timeout(a).read_timeout(b).build()"
        )
        .is_empty());
        let violations = client_builder_timeout_violations(
            "reqwest::Client::builder().user_agent(\"test\").build()",
        );
        assert_eq!(violations.len(), 1);
        assert!(violations[0].1.contains("timeout"));
    }

    #[test]
    fn 応答本文とjsonの失敗を空へ落とさない() {
        let banned = [
            ".text().await.unwrap_or_default()",
            ".json().await.unwrap_or(",
            "serde_json::from_str(&body).unwrap_or_else(|_| json!({}))",
            "serde_json::from_str(&body).unwrap_or_else(|_| json!([]))",
            "serde_json::from_str(&text).unwrap_or_else(|_| json!({}))",
        ];
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .chain(crate::guard_scan::xtask_sources())
            .filter(|source| !source.path.ends_with("tools/http.rs"))
            .flat_map(|source| {
                banned
                    .iter()
                    .filter(|pattern| source.text.contains(**pattern))
                    .map(|pattern| format!("{} ({pattern})", source.path.display()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            violations.is_empty(),
            "外部応答の失敗を空本文・空JSONへ落とさないでください:\n{}",
            violations.join("\n")
        );
    }
}
