// OpenAI 互換4プロバイダの通信・設定・正規形変換の持ち主。
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use super::http_error::{provider_probe, provider_response, send_error};
use super::openai::{from_wire, to_wire};
use super::Provider;
use crate::config::Config;
use crate::message::{Field, Message};
use crate::tools::http;

pub(crate) struct CompatSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub api_key_key: &'static str,
    pub base_url_key: &'static str,
    pub model_key: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub static_models: &'static [&'static str],
    pub key_placeholder: &'static str,
    pub key_label: &'static str,
    pub base_label: &'static str,
    pub extra_body: Option<fn(&Config, &str, &mut Value)>,
    pub extra_fields: Option<fn() -> Vec<Field>>,
    pub model_prefix: Option<&'static str>,
}

pub(crate) struct CompatProvider(pub &'static CompatSpec);

impl CompatProvider {
    fn url(&self, cfg: &Config, endpoint: &str) -> String {
        format!(
            "{}{}",
            cfg.get(self.0.base_url_key, self.0.default_base_url)
                .trim_end_matches('/'),
            endpoint
        )
    }

    fn body(&self, cfg: &Config, model: &str, messages: &[Message], tools: &[Value]) -> Value {
        let mut body = json!({"model": model, "messages": to_wire(messages), "temperature": 0.6});
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(extra) = self.0.extra_body {
            extra(cfg, model, &mut body);
        }
        body
    }

    fn models(&self, data: &Value) -> anyhow::Result<Vec<String>> {
        let entries = data.get("data").and_then(Value::as_array).ok_or_else(|| {
            anyhow::anyhow!("{} 応答の data がモデル一覧ではありません", self.0.label)
        })?;
        entries
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{} 応答の data[{}].id がモデル名ではありません",
                            self.0.label,
                            index
                        )
                    })?;
                let id = match self.0.model_prefix {
                    Some(prefix) => id.strip_prefix(prefix).unwrap_or(id),
                    None => id,
                };
                if id.trim().is_empty() {
                    anyhow::bail!("{} 応答の data[{}].id が空です", self.0.label, index);
                }
                Ok(id.to_owned())
            })
            .collect()
    }
}

#[async_trait]
impl Provider for CompatProvider {
    fn id(&self) -> &'static str {
        self.0.id
    }
    fn label(&self) -> &'static str {
        self.0.label
    }
    fn config_schema(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::new(
                self.0.api_key_key,
                self.0.key_label,
                "password",
                self.0.key_placeholder,
            ),
            Field::new(self.0.model_key, "既定モデル", "text", self.0.default_model),
        ];
        if let Some(extra) = self.0.extra_fields {
            fields.extend(extra());
        }
        fields.push(Field::new(
            self.0.base_url_key,
            self.0.base_label,
            "text",
            self.0.default_base_url,
        ));
        fields
    }
    fn default_model(&self, cfg: &Config) -> String {
        cfg.get(self.0.model_key, self.0.default_model)
    }

    async fn list_models(&self, cfg: &Config) -> anyhow::Result<Vec<String>> {
        let key = cfg.get(self.0.api_key_key, "");
        if key.is_empty() {
            // フォールバック許可: 未設定時だけ通信せず選択候補を示す。（根拠: PR #467 の利用者決定）
            return Ok(self
                .0
                .static_models
                .iter()
                .map(|id| (*id).to_owned())
                .collect());
        }
        let response = http::client()
            .map_err(anyhow::Error::msg)?
            .get(self.url(cfg, "/models"))
            .bearer_auth(key)
            .timeout(http::metadata_timeout(cfg).map_err(anyhow::Error::msg)?)
            .send()
            .await;
        let response = provider_response(cfg, self.0.label, "/models", response).await?;
        let data: Value = response.json().await.map_err(send_error)?;
        self.models(&data)
    }

    async fn is_up(&self, cfg: &Config) -> Result<bool, String> {
        let key = cfg.get(self.0.api_key_key, "");
        if key.is_empty() {
            return Ok(false);
        }
        let request = http::client()?
            .get(self.url(cfg, "/models"))
            .bearer_auth(key)
            .timeout(Duration::from_secs(3));
        provider_probe(cfg, self.0.label, "/models", request).await
    }

    async fn chat(
        &self,
        cfg: &Config,
        model: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> anyhow::Result<Message> {
        let key = cfg.get(self.0.api_key_key, "");
        if key.is_empty() {
            anyhow::bail!(
                "{} API キーが未設定です（設定画面の {} で入力してください）",
                self.0.label,
                self.0.label
            );
        }
        let response = http::client()
            .map_err(anyhow::Error::msg)?
            .post(self.url(cfg, "/chat/completions"))
            .bearer_auth(key)
            .json(&self.body(cfg, model, messages, tools))
            .send()
            .await;
        let response = provider_response(cfg, self.0.label, "/chat/completions", response).await?;
        let data: Value = response.json().await.map_err(send_error)?;
        let message = data
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("message"))
            .filter(|message| message.is_object())
            .ok_or_else(|| {
                anyhow::anyhow!("{} 応答に choices[0].message がありません", self.0.label)
            })?;
        Ok(from_wire(message))
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::thread::JoinHandle;
    use std::time::Duration;

    /// 同じ応答を各要求へ返し、完了時に要求をすべて回収する共通HTTP検査器。
    pub(crate) struct Server {
        pub url: String,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<Vec<String>>>,
    }

    impl Server {
        pub fn json(status: u16, body: &serde_json::Value) -> Self {
            let body = body.to_string();
            Self::raw(format!("HTTP/1.1 {status} 試験\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes())
        }

        pub fn raw(response: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let worker = std::thread::spawn(move || {
                let mut requests = Vec::new();
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // Windowsでlistenerの非ブロック属性を継承した接続も通常の期限付き読取へ戻す。
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut buffer = [0; 4096];
                            loop {
                                let count = stream.read(&mut buffer).unwrap();
                                assert!(count > 0, "要求が途中で切断されました");
                                request.extend_from_slice(&buffer[..count]);
                                if let Some(end) =
                                    request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                                {
                                    let headers = String::from_utf8_lossy(&request[..end]);
                                    let length = headers
                                        .lines()
                                        .find_map(|line| {
                                            let (name, value) = line.split_once(':')?;
                                            name.eq_ignore_ascii_case("content-length")
                                                .then(|| value.trim().parse::<usize>().unwrap())
                                        })
                                        .unwrap_or(0);
                                    if request.len() >= end + 4 + length {
                                        break;
                                    }
                                }
                            }
                            requests.push(String::from_utf8(request).unwrap());
                            stream.write_all(&response).unwrap();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("試験サーバの受付失敗: {error}"),
                    }
                }
                requests
            });
            Self {
                url,
                stop,
                worker: Some(worker),
            }
        }

        pub fn finish(mut self) -> Vec<String> {
            self.stop.store(true, Ordering::Release);
            self.worker.take().unwrap().join().unwrap()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::Server;
    use super::*;
    use crate::message::ToolCall;
    use crate::providers::{deepseek, gemini, mistral, xai};
    use std::sync::{Arc, Mutex};

    fn specs() -> [&'static CompatSpec; 4] {
        [&mistral::SPEC, &deepseek::SPEC, &xai::SPEC, &gemini::SPEC]
    }

    #[test]
    fn 四社とも共通protocolと固定ツールを先頭一致のまま送る() {
        let (before, after, _) = crate::agent::context::protocol_fixture();
        let tools = vec![
            json!({"type":"function","function":{"name":"doc_search","parameters":{"type":"object"}}}),
        ];
        let cfg = Config::new();
        for spec in specs() {
            let provider = CompatProvider(spec);
            let earlier = provider.body(&cfg, spec.default_model, &before, &tools);
            let later = provider.body(&cfg, spec.default_model, &after, &tools);
            assert_eq!(earlier["tools"], later["tools"], "{}", spec.id);
            assert_eq!(later["messages"], json!(to_wire(&after)), "{}", spec.id);
            let prefix = &later["messages"].as_array().unwrap()[..before.len()];
            assert_eq!(earlier["messages"], json!(prefix), "{}", spec.id);
        }
    }
    fn config(spec: &CompatSpec, server: &Server, key: &str) -> Config {
        let cfg = Config::new();
        cfg.set(spec.api_key_key, key);
        cfg.set(spec.base_url_key, &server.url);
        cfg
    }

    #[tokio::test]
    async fn 四社のツール有無と画像履歴を共通正規形のまま送受信する() {
        let mut call = Message::new("assistant", "検索します");
        call.tool_calls = Some(vec![ToolCall {
            id: "呼出1".into(),
            name: "lookup".into(),
            arguments: json!({"q":"猫"}),
        }]);
        let mut result = Message::new("tool", "見つかりました");
        result.tool_call_id = Some("呼出1".into());
        let messages = vec![
            Message::new("system", "日本語で応答"),
            Message::user_with_images("画像", vec!["data:image/png;base64,AAAA".into()]),
            call,
            result,
        ];
        let wire = json!({"content":"回答", "tool_calls":[{"id":"呼出2","function":{"name":"lookup","arguments":"{\"q\":\"犬\"}"}}]});
        for spec in specs() {
            for tools in [
                vec![],
                vec![json!({"type":"function", "function":{"name":"lookup"}})],
            ] {
                let server = Server::json(200, &json!({"choices":[{"message":wire}]}));
                let cfg = config(spec, &server, "検査キー");
                let message = CompatProvider(spec)
                    .chat(&cfg, spec.default_model, &messages, &tools)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(message).unwrap(),
                    serde_json::to_value(from_wire(&wire)).unwrap()
                );
                let requests = server.finish();
                assert_eq!(requests.len(), 1, "{}", spec.id);
                let body: Value =
                    serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
                assert_eq!(body["messages"], json!(to_wire(&messages)), "{}", spec.id);
                assert_eq!(body["model"], spec.default_model);
                assert_eq!(body["temperature"], 0.6);
                assert_eq!(
                    body.get("tools"),
                    (!tools.is_empty()).then(|| json!(tools)).as_ref()
                );
                assert_eq!(
                    body.get("thinking"),
                    (spec.id == "deepseek")
                        .then(|| json!({"type":"enabled"}))
                        .as_ref()
                );
                assert!(body.get("stream").is_none());
                assert!(body.get("response_format").is_none());
            }
        }
    }

    #[test]
    fn deepseekのthinkingは対応モデルだけに既存設定で付く() {
        let cfg = Config::new();
        let provider = CompatProvider(&deepseek::SPEC);
        for enabled in [true, false] {
            cfg.set(
                "PICOAGENT_DEEPSEEK_THINKING",
                if enabled { "true" } else { "false" },
            );
            for model in [
                "deepseek-v4-flash",
                "deepseek-v4-pro",
                "deepseek-chat",
                "deepseek-reasoner",
            ] {
                let body = provider.body(&cfg, model, &[], &[]);
                let expected = model
                    .starts_with("deepseek-v4-")
                    .then(|| json!({"type":if enabled {"enabled"} else {"disabled"}}));
                assert_eq!(body.get("thinking"), expected.as_ref());
            }
        }
    }

    #[tokio::test]
    async fn キー未設定だけ静的候補を返し通信も利用確認もしない() {
        for spec in specs() {
            let server = Server::json(500, &json!({}));
            let cfg = config(spec, &server, "");
            let provider = CompatProvider(spec);
            assert_eq!(
                provider.list_models(&cfg).await.unwrap(),
                spec.static_models
            );
            assert!(!provider.is_up(&cfg).await.unwrap());
            assert!(provider
                .chat(&cfg, spec.default_model, &[], &[])
                .await
                .is_err());
            assert!(server.finish().is_empty());
        }
    }

    #[tokio::test]
    async fn 設定済み一覧の失敗と壊れたschemaは候補へ降格しない() {
        for spec in specs() {
            for (status, body) in [
                (401, json!({"error":"拒否"})),
                (500, json!({"error":"停止"})),
                (200, json!({})),
                (200, json!({"data":[{"id":3}]})),
                (200, json!({"data":[{"id":""}]})),
            ] {
                let server = Server::json(status, &body);
                let cfg = config(spec, &server, "検査キー");
                assert!(
                    CompatProvider(spec).list_models(&cfg).await.is_err(),
                    "{} {status} {body}",
                    spec.id
                );
                assert_eq!(server.finish().len(), 1);
            }
            for data in [json!([]), json!([{"id":"models/remote-only"}])] {
                let server = Server::json(200, &json!({"data":data}));
                let cfg = config(spec, &server, "検査キー");
                let actual = CompatProvider(spec).list_models(&cfg).await.unwrap();
                let expected = if data.as_array().unwrap().is_empty() {
                    vec![]
                } else {
                    vec![if spec.id == "gemini" {
                        "remote-only"
                    } else {
                        "models/remote-only"
                    }
                    .to_owned()]
                };
                assert_eq!(actual, expected);
                assert_eq!(server.finish().len(), 1);
            }
        }
    }

    #[tokio::test]
    async fn geminiの拒否はツールを外して再送しない() {
        for status in [400, 429, 500] {
            let server = Server::json(status, &json!({"error":"拒否"}));
            let cfg = config(&gemini::SPEC, &server, "検査キー");
            let tools = vec![json!({"type":"function","function":{"name":"lookup"}})];
            assert!(CompatProvider(&gemini::SPEC)
                .chat(&cfg, "gemini-3.6-flash", &[], &tools)
                .await
                .is_err());
            let requests = server.finish();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].contains("\"tools\""));
        }
    }

    #[tokio::test]
    async fn 四社の非sse通知は全文一回でツールのみなら通知しない() {
        for spec in specs() {
            for content in ["全文", ""] {
                let server = Server::json(
                    200,
                    &json!({"choices":[{"message":{"content":content,"tool_calls":[{"id":"1","function":{"name":"lookup","arguments":"{}"}}]}}]}),
                );
                let cfg = config(spec, &server, "検査キー");
                let chunks = Arc::new(Mutex::new(Vec::new()));
                let sink = Arc::clone(&chunks);
                CompatProvider(spec)
                    .chat_stream(
                        &cfg,
                        spec.default_model,
                        &[],
                        &[],
                        Arc::new(move |chunk| sink.lock().unwrap().push(chunk)),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    *chunks.lock().unwrap(),
                    if content.is_empty() {
                        vec![]
                    } else {
                        vec![content.to_owned()]
                    }
                );
                assert_eq!(server.finish().len(), 1);
            }
        }
    }
}
