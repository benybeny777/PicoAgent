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

fn allowed_domain(cfg: &Config, url: &Url) -> bool {
    let allowed = cfg.get("PICOAGENT_HTTP_ALLOWED_DOMAINS", "");
    if allowed.trim().is_empty() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    allowed
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| host == entry || host.ends_with(&format!(".{}", entry)))
}

fn trim_response(text: &str) -> String {
    if text.len() <= MAX_RESPONSE_BYTES {
        return text.to_string();
    }
    let mut end = MAX_RESPONSE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（{} バイトで打ち切り）", &text[..end], MAX_RESPONSE_BYTES)
}

#[async_trait]
impl Tool for HttpApiTool {
    fn name(&self) -> &'static str {
        "http_api"
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
            Field::new("PICOAGENT_HTTP_ALLOWED_DOMAINS", "http_api 許可ドメイン（必須、カンマ区切り）", "textarea", "api.example.com,hooks.example.com"),
            Field::new("PICOAGENT_HTTP_BEARER_TOKEN", "http_api Bearer token（任意）", "password", ""),
            Field::new("PICOAGENT_HTTP_API_KEY_HEADER", "http_api API key ヘッダー名（任意）", "text", "X-API-Key"),
            Field::new("PICOAGENT_HTTP_API_KEY_VALUE", "http_api API key 値（任意）", "password", ""),
            Field::new("PICOAGENT_HTTP_TIMEOUT_SECS", "http_api 待ち時間（秒、3〜60）", "text", "20"),
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
        if !allowed_domain(cfg, &url) {
            return "エラー: PICOAGENT_HTTP_ALLOWED_DOMAINS で許可されていないドメインです。".to_string();
        }
        let timeout = crate::tools::http::timeout(
            cfg,
            "PICOAGENT_HTTP_TIMEOUT_SECS",
            20,
            3,
            60,
        );
        let client = match reqwest::Client::builder()
            .timeout(timeout)
            .user_agent("PicoAgent/1.0 (HTTP API tool)")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: HTTP API の準備に失敗しました: {}", error),
        };
        let method = args.get("method").and_then(Value::as_str).unwrap_or("GET");
        let mut request = match method {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            _ => return "エラー: method は GET/POST/PUT/PATCH/DELETE のいずれかを指定してください。".to_string(),
        };
        let bearer = cfg.get("PICOAGENT_HTTP_BEARER_TOKEN", "");
        if !bearer.is_empty() {
            request = request.bearer_auth(bearer);
        }
        let api_key_header = cfg.get("PICOAGENT_HTTP_API_KEY_HEADER", "");
        let api_key_value = cfg.get("PICOAGENT_HTTP_API_KEY_VALUE", "");
        if !api_key_header.is_empty() && !api_key_value.is_empty() {
            request = request.header(api_key_header, api_key_value);
        }
        if let Some(body) = args.get("json_body") {
            request = request.json(body);
        } else if let Some(text) = args.get("text_body").and_then(Value::as_str) {
            if text.len() > MAX_BODY_BYTES {
                return format!("エラー: text_body は {} バイト以内で指定してください。", MAX_BODY_BYTES);
            }
            request = request
                .header("Content-Type", args.get("content_type").and_then(Value::as_str).unwrap_or("text/plain"))
                .body(text.to_string());
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => return format!("エラー: HTTP API への接続に失敗しました: {}", error),
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            format!("HTTP {}\n{}", status, trim_response(&body))
        } else {
            format!("エラー: HTTP API が HTTP {} を返しました。", status)
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(HttpApiTool) } }
