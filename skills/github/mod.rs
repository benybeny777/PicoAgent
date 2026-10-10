// skills/github/mod.rs - GitHub Issue の一覧取得・作成スキル。

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const GITHUB_API_BASE: &str = "https://api.github.com";
const MAX_TITLE_CHARS: usize = 300;
const MAX_BODY_CHARS: usize = 20_000;

pub struct GitHubTool;

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &'static str {
        "github_issue"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["github", "issue", "イシュー"]
    }

    fn label(&self) -> &'static str {
        "GitHub Issue"
    }

    fn description(&self) -> String {
        "GitHub API で Issue を一覧取得または作成する。private repository や作成には token が必要。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "create"], "description": "list はIssue一覧、create はIssue作成。" },
                "owner": { "type": "string", "description": "リポジトリ owner。省略時は設定値。" },
                "repo": { "type": "string", "description": "リポジトリ名。省略時は設定値。" },
                "state": { "type": "string", "enum": ["open", "closed", "all"], "description": "list の状態。省略時 open。" },
                "title": { "type": "string", "description": "create のIssue件名。" },
                "body": { "type": "string", "description": "create のIssue本文。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_GITHUB_TOKEN",
                "GitHub のアクセストークン",
                "password",
                "github_pat_...",
            ),
            Field::new(
                "PICOAGENT_GITHUB_OWNER",
                "GitHub 既定 owner",
                "text",
                "owner",
            ),
            Field::new(
                "PICOAGENT_GITHUB_REPO",
                "GitHub 既定 repository",
                "text",
                "repo",
            ),
            Field::int(&crate::config::ints::GITHUB_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_GITHUB_OWNER", ""));
        let repo = args
            .get("repo")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| cfg.get("PICOAGENT_GITHUB_REPO", ""));
        if owner.is_empty() || repo.is_empty() {
            return super::config_guide(
                "GitHub Issue",
                &["PICOAGENT_GITHUB_OWNER", "PICOAGENT_GITHUB_REPO"],
                "依頼で owner/repo を渡すか、既定のリポジトリを設定してください。",
            );
        }
        let timeout_secs =
            crate::tool_try!(cfg.int(&crate::config::ints::GITHUB_TIMEOUT_SECS)) as u64;
        let token = cfg.get("PICOAGENT_GITHUB_TOKEN", "");
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .user_agent("PicoAgent/1.0")
            .build()
        {
            Ok(client) => client,
            Err(error) => return format!("エラー: GitHub API の準備に失敗しました: {}", error),
        };
        match action {
            "list" => {
                let state = args.get("state").and_then(Value::as_str).unwrap_or("open");
                let mut request = client
                    .get(format!(
                        "{}/repos/{}/{}/issues",
                        GITHUB_API_BASE, owner, repo
                    ))
                    .query(&[("state", state), ("per_page", "10")]);
                if !token.is_empty() {
                    request = request.bearer_auth(token);
                }
                let response = match request.send().await {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: GitHub API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                let body = crate::tool_try!(
                    crate::tools::http::service_body(cfg, "GitHub API", Ok(response)).await
                );
                let parsed: Value = crate::tool_response_json!(&body);
                let mut out = String::from("Issue一覧:\n");
                for issue in parsed.as_array().into_iter().flatten() {
                    if issue.get("pull_request").is_some() {
                        continue;
                    }
                    out.push_str(&format!(
                        "- #{} {} ({})\n",
                        issue.get("number").and_then(Value::as_u64).unwrap_or(0),
                        issue
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("（無題）"),
                        issue.get("html_url").and_then(Value::as_str).unwrap_or("")
                    ));
                }
                out
            }
            "create" => {
                if token.is_empty() {
                    return super::config_guide(
                        "GitHub Issue",
                        &["PICOAGENT_GITHUB_TOKEN"],
                        "Issue 作成や private repo 操作には GitHub token が必要です。",
                    );
                }
                let title = match args.get("title").and_then(Value::as_str).map(str::trim) {
                    Some(title) if !title.is_empty() => title,
                    _ => return "エラー: title が指定されていません。".to_string(),
                };
                if title.chars().count() > MAX_TITLE_CHARS {
                    return format!(
                        "エラー: title は {} 文字以内で指定してください。",
                        MAX_TITLE_CHARS
                    );
                }
                let body = args.get("body").and_then(Value::as_str).unwrap_or("");
                if body.chars().count() > MAX_BODY_CHARS {
                    return format!(
                        "エラー: body は {} 文字以内で指定してください。",
                        MAX_BODY_CHARS
                    );
                }
                let response = match client
                    .post(format!(
                        "{}/repos/{}/{}/issues",
                        GITHUB_API_BASE, owner, repo
                    ))
                    .bearer_auth(token)
                    .json(&json!({ "title": title, "body": body }))
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return format!(
                            "エラー: GitHub API への接続に失敗しました: {}",
                            error.without_url()
                        )
                    }
                };
                let body = crate::tool_try!(
                    crate::tools::http::service_body(cfg, "GitHub API", Ok(response)).await
                );
                let parsed: Value = crate::tool_response_json!(&body);
                format!(
                    "OK: Issueを作成しました。{}",
                    parsed.get("html_url").and_then(Value::as_str).unwrap_or("")
                )
            }
            _ => "エラー: action は list または create を指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(GitHubTool) } }
