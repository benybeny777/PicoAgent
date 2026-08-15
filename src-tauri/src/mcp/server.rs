// mcp/server.rs - PicoAgent のスキルを MCP ツールとして公開するサーバ側実装。
//
// ローカル API（api_server.rs。既定無効・127.0.0.1 のみ・Bearer 認証対応）の
// POST /mcp に JSON-RPC 2.0 メッセージを受け、スキルレジストリ（tools::all）を
// tools/list / tools/call として提供する。Claude Code などの MCP クライアントから
//   claude mcp add --transport http picoagent http://127.0.0.1:11500/mcp
// のように登録して使える。

use serde_json::{json, Value};

use super::{rpc_error, rpc_result, PROTOCOL_VERSION};
use crate::config::Config;

/// JSON-RPC メッセージを1件処理する。応答不要（通知）の場合は None。
pub async fn handle_rpc(cfg: &Config, message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    // 通知（id なし）は応答しない。未知の通知も黙って無視する（MCP の慣例）。
    let is_notification = message.get("id").is_none();

    match method {
        "initialize" => Some(rpc_result(
            &id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "PicoAgent", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "ping" => Some(rpc_result(&id, json!({}))),
        "tools/list" => {
            let tools: Vec<Value> = crate::tools::all()
                .iter()
                .filter(|tool| tool.agent_callable())
                .map(|tool| {
                    json!({
                        "name": tool.name(),
                        "description": tool.description(),
                        "inputSchema": tool.parameters(),
                    })
                })
                .collect();
            Some(rpc_result(&id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or_default();
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(&id, -32602, "params.name が必要です"));
            };
            // 一覧に出していないツール（アプリが内部で使うもの）は外から呼ばせない。
            // 公開している面と実行できる面をずらさないため、`tools/list` と同じ条件で絞る。
            let published = crate::tools::all()
                .iter()
                .any(|tool| tool.name() == name && tool.agent_callable());
            if !published {
                return Some(rpc_error(
                    &id,
                    -32601,
                    &format!("公開していない、または未登録のツールです: {name}"),
                ));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = crate::tools::dispatch(cfg, name, args).await;
            // スキルは失敗も文字列で返す設計のため、「エラー:」始まりを isError に写像する。
            let is_error = result.starts_with("エラー");
            Some(rpc_result(
                &id,
                json!({
                    "content": [{ "type": "text", "text": result }],
                    "isError": is_error
                }),
            ))
        }
        _ if is_notification => None,
        other => Some(rpc_error(
            &id,
            -32601,
            &format!("未対応のメソッドです: {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::handle_rpc;
    use serde_json::json;

    /// `tools/list` に出していないツールは `tools/call` でも実行できないこと。
    ///
    /// 公開している面と実行できる面がずれると、外部の MCP クライアントから
    /// 「無いはずのツール」を動かせてしまう。
    #[test]
    fn 一覧に無いツールは実行できない() {
        let cfg = crate::config::Config::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // chitchat はアプリが内部で使うスキルで、agent_callable=false。
            let hidden = crate::tools::all()
                .iter()
                .find(|tool| !tool.agent_callable())
                .map(|tool| tool.name().to_string())
                .expect("非公開スキルが1つは存在する前提");

            let listed = handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            )
            .await
            .unwrap();
            let names: Vec<String> = listed["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap_or("").to_string())
                .collect();
            assert!(!names.contains(&hidden), "一覧に出ていない前提が崩れている");

            let called = handle_rpc(
                &cfg,
                &json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": hidden, "arguments": {} }
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                called["error"]["code"], -32601,
                "実行を拒否していない: {called}"
            );
        });
    }

    #[test]
    fn 初期化とツール一覧と実行ができる() {
        let cfg = crate::config::Config::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // initialize
            let response = handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            )
            .await
            .unwrap();
            assert_eq!(response["result"]["serverInfo"]["name"], "PicoAgent");
            assert_eq!(
                response["result"]["protocolVersion"],
                super::PROTOCOL_VERSION
            );

            // 通知には応答しない
            assert!(handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
            )
            .await
            .is_none());

            // tools/list にスキルが載る
            let response = handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            )
            .await
            .unwrap();
            let tools = response["result"]["tools"].as_array().unwrap();
            assert!(
                tools.iter().any(|tool| tool["name"] == "get_time"),
                "tools: {}",
                tools.len()
            );
            assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));

            // tools/call で設定不要スキル（時刻）を実行できる
            let response = handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "get_time", "arguments": {} } }),
            )
            .await
            .unwrap();
            let text = response["result"]["content"][0]["text"].as_str().unwrap();
            assert!(!text.is_empty());
            assert_eq!(response["result"]["isError"], false);

            // 未知メソッドはエラー応答
            let response = handle_rpc(
                &cfg,
                &json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }),
            )
            .await
            .unwrap();
            assert_eq!(response["error"]["code"], -32601);
        });
    }
}
