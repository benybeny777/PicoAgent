// xtask/src/cli.rs - PicoAgent のローカル API を叩く最小 CLI（Ollama の CLI 風）。
//
// 旧 `scripts/picoagent-cli.mjs` の置き換え。
//
// 前提: PicoAgent が起動していて、設定でローカル API が有効なこと
//       （設定画面「基本」タブ →「ローカルAPI」→ 有効化して再起動）。
//
// 接続先の変更: PICOAGENT_API_PORT（既定 11500）/ PICOAGENT_API_KEY（設定時は必須）

use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::util::env_or;

pub const USAGE: &str = "\
使い方: cargo xtask cli <コマンド> [引数…]
  status               API の死活とバージョン
  models               モデル一覧
  chat <メッセージ…>    1問1答（スキルも使える）
  ask <プロンプト…>     生成（chat と同等）
  repl                 対話モード（会話文脈を保持）
環境変数: PICOAGENT_API_PORT（既定 11500）/ PICOAGENT_API_KEY";

fn base_url() -> String {
    format!("http://127.0.0.1:{}", env_or("PICOAGENT_API_PORT", "11500"))
}

fn request(method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    let base = base_url();
    let url = format!("{base}{path}");
    // 応答は待つが（ローカルモデルの生成は分単位になりうる）、到達しないホストでの
    // 接続滞留だけは切る。
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?;
    let mut builder = match method {
        "GET" => client.get(&url),
        _ => client.post(&url),
    }
    .header("content-type", "application/json");

    let key = env_or("PICOAGENT_API_KEY", "");
    if !key.is_empty() {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    if let Some(body) = body {
        builder = builder.json(&body);
    }

    let response = builder.send().map_err(|_| {
        anyhow::anyhow!(
            "接続できません: {base}\n\
             PicoAgent を起動し、設定の「ローカルAPI」を有効にして再起動してください。\
             ポートが違う場合は PICOAGENT_API_PORT を設定します。"
        )
    })?;

    let status = response.status();
    let text = response.text().context("API応答本文を読めません")?;
    parse_response(status.is_success(), status.as_u16(), &text)
}

fn parse_response(success: bool, status: u16, text: &str) -> Result<Value> {
    let preview: String = text.chars().take(200).collect();
    let json: Value = serde_json::from_str(text)
        .with_context(|| format!("HTTP {status}: API応答がJSONではありません: {preview}"))?;
    if !success {
        let message = json.get("error").and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or(error.as_str())
        });
        bail!("HTTP {status}: {}", message.unwrap_or(&preview));
    }
    Ok(json)
}

pub fn run(args: &[String]) -> Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let text = args[args.len().min(1)..].join(" ").trim().to_string();

    match command {
        "status" | "version" => status(),
        "models" | "list" => models(),
        "chat" => {
            if text.is_empty() {
                bail!("使い方: cargo xtask cli chat こんにちは");
            }
            let result = request(
                "POST",
                "/api/chat",
                Some(json!({
                    "messages": [{ "role": "user", "content": text }]
                })),
            )?;
            println!("{}", message_content(&result));
            Ok(())
        }
        "ask" | "generate" => {
            if text.is_empty() {
                bail!("使い方: cargo xtask cli ask 今日の天気は？");
            }
            let result = request("POST", "/api/generate", Some(json!({ "prompt": text })))?;
            println!(
                "{}",
                result.get("response").and_then(Value::as_str).unwrap_or("")
            );
            Ok(())
        }
        "repl" | "run" => repl(),
        _ => {
            println!("{USAGE}");
            if command.is_empty() {
                Ok(())
            } else {
                bail!("未知のコマンドです: {command}")
            }
        }
    }
}

fn message_content(result: &Value) -> String {
    result
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn status() -> Result<()> {
    let version = request("GET", "/api/version", None)?;
    let tags = request("GET", "/api/tags", None)?;
    let model = tags
        .get("models")
        .and_then(Value::as_array)
        .and_then(|m| m.first());
    println!(
        "PicoAgent v{} ({})",
        version
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?"),
        base_url()
    );
    println!(
        "モデル: {} / プロバイダ: {}",
        model
            .and_then(|m| m.get("name").and_then(Value::as_str))
            .unwrap_or("(未選択)"),
        model
            .and_then(|m| m
                .get("details")
                .and_then(|d| d.get("provider"))
                .and_then(Value::as_str))
            .unwrap_or("?")
    );
    Ok(())
}

fn models() -> Result<()> {
    let tags = request("GET", "/api/tags", None)?;
    for model in tags
        .get("models")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
    {
        println!(
            "{}\t({})",
            model.get("name").and_then(Value::as_str).unwrap_or("?"),
            model
                .get("details")
                .and_then(|d| d.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or("?")
        );
    }
    Ok(())
}

fn repl() -> Result<()> {
    status()?;
    println!("対話モード（exit / Ctrl+C で終了）");
    let stdin = std::io::stdin();
    let mut messages: Vec<Value> = Vec::new();

    loop {
        print!("> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin
            .lock()
            .read_line(&mut line)
            .context("入力を読めません")?
            == 0
        {
            break; // Ctrl+D
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text == "exit" || text == "quit" {
            break;
        }
        messages.push(json!({ "role": "user", "content": text }));
        // 応答待ちの間は次の入力を読まない（連投で会話が入れ違うのを防ぐ）。
        match request("POST", "/api/chat", Some(json!({ "messages": messages }))) {
            Ok(result) => {
                let reply = message_content(&result);
                messages.push(json!({ "role": "assistant", "content": reply }));
                println!("{reply}");
            }
            Err(error) => {
                eprintln!("{error}");
                messages.pop();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 非jsonのhttp失敗も本文理由と状態を保持する() {
        for (status, body) in [
            (500, "サーバー故障"),
            (401, "資格情報がありません"),
            (200, "壊れた成功応答"),
        ] {
            let error = format!(
                "{:#}",
                parse_response(status == 200, status, body).unwrap_err()
            );
            assert!(error.contains(&status.to_string()) && error.contains(body));
        }
        let error = parse_response(
            false,
            422,
            r#"{"error":{"message":"入力を確認してください"}}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("入力を確認してください"));
        assert_eq!(
            parse_response(true, 200, r#"{"ok":true}"#).unwrap()["ok"],
            true
        );
    }
    #[test]
    fn http応答のjson読込失敗を空値へ降格しない() {
        let production = include_str!("cli.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            ".json().unwrap_or",
            ".json().ok()",
            ".json().unwrap_or_else",
        ] {
            assert!(!production.contains(forbidden));
        }
    }
}
