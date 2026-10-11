//! SNSの本文・検索語と公開範囲の入力検査。認証とAPIは各スキルが所有する。
use serde_json::Value;

pub fn text_arg<'a>(args: &'a Value, key: &str, max_chars: usize) -> Result<&'a str, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} が指定されていません。"))?;
    if text.chars().count() > max_chars {
        return Err(format!("{key} は {max_chars} 文字以内で指定してください。"));
    }
    Ok(text)
}

/// 省略時だけ明示した既定を使い、型不正や他サービスの公開範囲へ降格しない。
pub fn visibility<'a>(
    args: &'a Value,
    allowed: &[&str],
    default: &'a str,
) -> Result<&'a str, String> {
    let value = match args.get("visibility") {
        None => default,
        Some(value) => value
            .as_str()
            .ok_or("visibility は文字列で指定してください。")?,
    };
    if !allowed.contains(&value) {
        return Err(format!(
            "visibility は {} のいずれかを指定してください。",
            allowed.join("/")
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 投稿六サービスは共通入力検査を使い文字数検査を写さない() {
        for skill in [
            "mastodon",
            "misskey",
            "bluesky",
            "x_twitter",
            "linkedin",
            "reddit",
        ] {
            let source = crate::guard_scan::skill_sources()
                .iter()
                .find(|source| source.name == skill)
                .unwrap();
            let body = source.body();
            assert!(
                body.contains("crate::tools::social_post::text_arg"),
                "{skill}が共通本文検査を使っていません"
            );
            assert!(
                !body.contains(".chars().count()"),
                "{skill}へ文字数判定が戻っています"
            );
            if matches!(skill, "mastodon" | "misskey") {
                assert!(
                    body.contains("crate::tools::social_post::visibility("),
                    "{skill}が共通公開範囲検査を使っていません"
                );
            }
        }
    }

    #[test]
    fn 四サービスの本文上限と検索語を同じ文字数規則で検査する() {
        for (key, limit) in [
            ("status", 5000),
            ("text", 3000),
            ("text", 300),
            ("text", 280),
            ("query", 512),
            ("query", 200),
        ] {
            let exact = "あ".repeat(limit);
            let args = json!({key:format!(" \n{exact}\t")});
            assert_eq!(text_arg(&args, key, limit).unwrap(), exact);
            assert!(text_arg(&json!({key:format!("{exact}い")}), key, limit).is_err());
            for invalid in [Value::Null, json!(true), json!(1), json!(" \n\t")] {
                assert!(text_arg(&json!({key:invalid}), key, limit).is_err());
            }
            assert!(text_arg(&json!({}), key, limit).is_err());
        }
    }

    #[test]
    fn 公開範囲は各サービスの表だけを認め不正値を診断へ混ぜない() {
        for allowed in [
            &["public", "unlisted", "private", "direct"][..],
            &["public", "home", "followers", "specified"][..],
        ] {
            assert_eq!(visibility(&json!({}), allowed, "public").unwrap(), "public");
            for value in allowed {
                assert_eq!(
                    visibility(&json!({"visibility":value}), allowed, "public").unwrap(),
                    *value
                );
            }
            for value in [json!(null), json!(true), json!(""), json!("secret-invalid")] {
                let args = json!({"visibility":value});
                let result = visibility(&args, allowed, "public");
                assert!(result.is_err());
                assert!(!result.unwrap_err().contains("secret-invalid"));
            }
        }
    }

    #[tokio::test]
    async fn 実投稿入口は不正公開範囲を送信せず正常投稿の認証と本文を保持する() {
        use crate::providers::compatible::test_server::Server;
        use crate::tools::{mastodon::MastodonTool, misskey::MisskeyTool, Tool};
        type SocialCase = (
            Box<dyn Tool>,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
        );
        let cases: [SocialCase; 2] = [
            (
                Box::new(MastodonTool),
                "PICOAGENT_MASTODON_ACCESS_TOKEN",
                "PICOAGENT_MASTODON_BASE_URL",
                "post",
                "status",
            ),
            (
                Box::new(MisskeyTool),
                "PICOAGENT_MISSKEY_TOKEN",
                "PICOAGENT_MISSKEY_BASE_URL",
                "create_note",
                "text",
            ),
        ];
        for (tool, token_key, base_key, action, text_key) in cases {
            let server = Server::json(200, &json!({"ok":true}));
            let cfg = crate::config::Config::new();
            cfg.set(token_key, "test-token");
            cfg.set(base_key, &server.url);
            let invalid = tool
                .run(
                    &cfg,
                    json!({"action":action,text_key:" 本文🌿 ","visibility":"未知の範囲"}),
                )
                .await;
            assert!(invalid.starts_with("エラー:"));
            let normal = tool
                .run(
                    &cfg,
                    json!({"action":action,text_key:" 本文🌿 ","visibility":"public"}),
                )
                .await;
            assert!(!normal.starts_with("エラー:"), "{normal}");
            let requests = server.finish();
            assert_eq!(requests.len(), 1, "不正値がAPIへ送信されました");
            let (headers, body) = requests[0].split_once("\r\n\r\n").unwrap();
            if action == "post" {
                assert!(headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer"));
                let form: std::collections::HashMap<_, _> =
                    url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                assert_eq!(form.get("status").unwrap(), "本文🌿");
                assert_eq!(form.get("visibility").unwrap(), "public");
            } else {
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["i"], "test-token");
                assert_eq!(body["text"], "本文🌿");
                assert_eq!(body["visibility"], "public");
            }
        }
    }
}
