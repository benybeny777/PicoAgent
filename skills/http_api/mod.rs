// skills/http_api/mod.rs - 汎用HTTP API/Webhook呼び出しスキル。
//
// 未個別対応のSaaS連携用。許可ドメインと任意のBearer/API keyを設定で制限する。

use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub struct HttpApiTool;

#[async_trait]
impl Tool for HttpApiTool {
    fn name(&self) -> &'static str {
        "http_api"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["api", "webhook", "http"]
    }

    fn label(&self) -> &'static str {
        "汎用HTTP API"
    }

    fn description(&self) -> String {
        "個別スキルがないSaaSやWebhookへ、許可ドメイン制限付きでHTTP GET/POST/PUT/PATCH/DELETEを送る。秘密値は設定から付与する。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "method": { "type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"], "description": "HTTP method。" },
                "url": { "type": "string", "description": "呼び出すURL。許可ドメインのみ。" },
                "json_body": { "type": "object", "description": "JSON body。" },
                "text_body": { "type": "string", "description": "テキスト body。json_body がある場合は無視。" },
                "content_type": { "type": "string", "description": "text_body の Content-Type。省略時 text/plain。" }
            },
            "required": ["method", "url"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_HTTP_ALLOWED_DOMAINS",
                "http_api 許可ドメイン（必須、カンマ区切り）",
                "textarea",
                "api.example.com,hooks.example.com",
            ),
            Field::new(
                "PICOAGENT_HTTP_BEARER_TOKEN",
                "http_api Bearer token（任意）",
                "password",
                "",
            ),
            Field::new(
                "PICOAGENT_HTTP_API_KEY_HEADER",
                "http_api API key ヘッダー名（任意）",
                "text",
                "X-API-Key",
            ),
            Field::new(
                "PICOAGENT_HTTP_API_KEY_VALUE",
                "http_api API key 値（任意）",
                "password",
                "",
            ),
            Field::int(&crate::config::ints::HTTP_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let raw_url = match args.get("url").and_then(Value::as_str).map(str::trim) {
            Some(url) if !url.is_empty() => url,
            _ => return "エラー: url が指定されていません。".to_string(),
        };
        let url = match Url::parse(raw_url) {
            Ok(url) if url.scheme() == "http" || url.scheme() == "https" => url,
            Ok(_) => return "エラー: url は http または https を指定してください。".to_string(),
            Err(error) => return format!("エラー: url の解析に失敗しました: {}", error),
        };
        let timeout = crate::tool_try!(cfg.duration_secs(&crate::config::ints::HTTP_TIMEOUT_SECS));
        let mut request = crate::tools::http::GuardedRequest::get(
            url,
            crate::tools::http::UrlPolicy::domains(&cfg.get("PICOAGENT_HTTP_ALLOWED_DOMAINS", "")),
            timeout,
            MAX_RESPONSE_BYTES,
            "PicoAgent/1.0 (HTTP API tool)",
        );
        let method = args.get("method").and_then(Value::as_str).unwrap_or("GET");
        request.method = match method {
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" => {
                match reqwest::Method::from_bytes(method.as_bytes()) {
                    Ok(method) => method,
                    Err(_) => return "エラー: method が不正です".into(),
                }
            }
            _ => {
                return "エラー: method は GET/POST/PUT/PATCH/DELETE のいずれかを指定してください。"
                    .into()
            }
        };
        let bearer = cfg.get("PICOAGENT_HTTP_BEARER_TOKEN", "");
        if !bearer.is_empty() {
            let value = match reqwest::header::HeaderValue::from_str(&format!("Bearer {bearer}")) {
                Ok(value) => value,
                Err(_) => return "エラー: Bearer token が不正です".into(),
            };
            request
                .headers
                .insert(reqwest::header::AUTHORIZATION, value);
        }
        let header = cfg.get("PICOAGENT_HTTP_API_KEY_HEADER", "");
        let value = cfg.get("PICOAGENT_HTTP_API_KEY_VALUE", "");
        if !header.is_empty() && !value.is_empty() {
            let name = match reqwest::header::HeaderName::from_bytes(header.as_bytes()) {
                Ok(name) => name,
                Err(_) => return "エラー: API key ヘッダー名が不正です".into(),
            };
            let value = match reqwest::header::HeaderValue::from_str(&value) {
                Ok(value) => value,
                Err(_) => return "エラー: API key 値が不正です".into(),
            };
            request.secret_headers.push(name.clone());
            request.headers.insert(name, value);
        }
        let content_type = if let Some(body) = args.get("json_body") {
            request.body = match serde_json::to_vec(body) {
                Ok(body) => Some(body),
                Err(_) => return "エラー: JSON本文を組み立てられません".into(),
            };
            "application/json"
        } else {
            request.body = args
                .get("text_body")
                .and_then(Value::as_str)
                .map(|s| s.as_bytes().to_vec());
            args.get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("text/plain")
        };
        if request
            .body
            .as_ref()
            .is_some_and(|body| body.len() > MAX_BODY_BYTES)
        {
            return "エラー: 本文は0.13 MB以内で指定してください".into();
        }
        let content_type = match reqwest::header::HeaderValue::from_str(content_type) {
            Ok(value) => value,
            Err(_) => return "エラー: Content-Type が不正です".into(),
        };
        request
            .headers
            .insert(reqwest::header::CONTENT_TYPE, content_type);
        let response = match crate::tools::http::fetch_guarded(request).await {
            Ok(response) => response,
            Err(error) => return format!("エラー: {error}"),
        };
        let body = match String::from_utf8(response.body) {
            Ok(body) => body,
            Err(_) => return "エラー: HTTP API の応答がUTF-8ではありません".into(),
        };
        if response.status.is_success() {
            format!("HTTP {}\n{body}", response.status)
        } else {
            format!(
                "エラー: {}",
                crate::tools::http::safe_service_error(cfg, "HTTP API", response.status, &body)
            )
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(HttpApiTool) } }

#[cfg(test)]
mod numeric_tests {
    use crate::config::Config;
    use crate::tools::{
        http_api::HttpApiTool, rss_feed::RssFeedTool, web_fetch::WebFetchTool, Tool,
    };
    use serde_json::json;

    #[tokio::test]
    async fn 不正な整数設定はスキルから理由を返し要求を送らない() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let tools: [(&dyn Tool, &str); 3] = [
            (&HttpApiTool, "PICOAGENT_HTTP_TIMEOUT_SECS"),
            (&RssFeedTool, "PICOAGENT_RSS_TIMEOUT_SECS"),
            (&WebFetchTool, "PICOAGENT_WEB_FETCH_TIMEOUT_SECS"),
        ];
        for (tool, key) in tools {
            for invalid in ["abc", "-1", "0", "61", "１．５", "9223372036854775808"] {
                let cfg = Config::new();
                cfg.set(key, invalid);
                cfg.set("PICOAGENT_HTTP_ALLOWED_DOMAINS", "127.0.0.1");
                let result = tool.run(&cfg, json!({"url":url})).await;
                assert!(result.starts_with("エラー:"), "{}: {result}", tool.name());
                assert!(result.contains(key), "{}: {result}", tool.name());
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
            }
        }
    }
}
