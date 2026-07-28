// mcp/mod.rs - MCP（Model Context Protocol）層。
//
// PicoAgent のアーキテクチャは次の3層に分離されている:
//   - agent  … エージェントループ（chat → tool_calls 実行 → 再 chat）。src/agent.rs
//   - skills … 内蔵ツール（プロジェクト直下 skills/<名前>/mod.rs、inventory 自動登録）
//   - mcp    … 外部との相互運用層（このモジュール）
//       * client: 設定した外部 MCP サーバ（stdio）を起動し、そのツールを
//         `mcp__<サーバ名>__<ツール名>` としてエージェントから使えるようにする
//       * server: PicoAgent のスキルを MCP ツールとしてローカル API の /mcp で公開し、
//         Claude Code などの MCP クライアントから呼べるようにする
//
// プロトコルは JSON-RPC 2.0。stdio トランスポートは改行区切り JSON、
// HTTP トランスポートは POST /mcp への単一 JSON-RPC メッセージで応答する。

pub mod client;
pub mod server;

/// 対応する MCP プロトコルバージョン。
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC 2.0 の成功応答を組み立てる。
pub fn rpc_result(id: &serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// JSON-RPC 2.0 のエラー応答を組み立てる。
pub fn rpc_error(id: &serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// MCP のツール名・サーバ名として安全な文字だけに正規化する（英数と _ - のみ）。
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "tool".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::{rpc_error, rpc_result, sanitize_name};
    use serde_json::json;

    #[test]
    fn ツール名を安全に正規化する() {
        assert_eq!(sanitize_name("read_file"), "read_file");
        assert_eq!(sanitize_name("メモ帳/検索"), "______");
        assert_eq!(sanitize_name(""), "tool");
    }

    #[test]
    fn jsonrpc応答を組み立てる() {
        let ok = rpc_result(&json!(1), json!({ "x": 1 }));
        assert_eq!(ok["jsonrpc"], "2.0");
        assert_eq!(ok["result"]["x"], 1);
        let err = rpc_error(&json!("a"), -32601, "method not found");
        assert_eq!(err["error"]["code"], -32601);
        assert_eq!(err["id"], "a");
    }
}
