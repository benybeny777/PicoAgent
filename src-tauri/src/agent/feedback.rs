//! 計画の事前生成と、最終回答が明らかに不十分な場合の再生成を担当する。

use crate::config::Config;
use crate::message::Message;
use crate::providers::Provider;

use super::{agent_depth, super_agent_enabled};

fn extract_json_object(content: &str) -> anyhow::Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(content.trim())
        .map_err(|error| anyhow::anyhow!("計画の応答をJSONとして読めません: {error}"))?;
    anyhow::ensure!(
        value.is_object(),
        "計画の応答がJSONオブジェクトではありません"
    );
    Ok(value)
}

fn is_complex_query(text: &str) -> bool {
    let count = text.chars().count();
    if count < 60 {
        return false;
    }
    let questions = text.matches('?').count() + text.matches('？').count();
    count > 120
        || questions >= 2
        || text.contains("手順")
        || text.contains("ステップ")
        || text.contains("step")
}

pub(super) async fn planning_step(
    provider: &dyn Provider,
    cfg: &Config,
    model: &str,
    user_text: &str,
) -> anyhow::Result<Option<String>> {
    if !super_agent_enabled(cfg) {
        return Ok(None);
    }
    if agent_depth() > 1 || !is_complex_query(user_text) {
        return Ok(None);
    }
    let plan_messages = vec![
        Message::new(
            "system",
            "あなたはタスク分解AIです。ユーザーの依頼を最大4つのサブタスクに分解して、必ず次のJSON形式だけで返してください。{\"steps\":[\"手順1\",\"手順2\",...]}",
        ),
        Message::new("user", user_text),
    ];
    let result = provider
        .chat_with_options(cfg, model, &plan_messages, &[], true)
        .await
        .map_err(|error| error.context("実行計画の生成に失敗しました"))?;
    let plan = extract_json_object(&result.content)?;
    let steps = plan
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("計画の応答にsteps配列がありません"))?;
    anyhow::ensure!(
        !steps.is_empty() && steps.len() <= 4,
        "計画のstepsは1〜4件で指定する必要があります"
    );
    let steps = steps
        .iter()
        .map(|step| {
            step.as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("計画の手順が空文字または文字列以外です"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if steps.len() <= 1 {
        return Ok(None);
    }
    let plan_lines: Vec<String> = steps
        .iter()
        .enumerate()
        .map(|(index, step)| format!("{}. {}", index + 1, step))
        .collect();
    // 計画は進捗表示としてそのまま画面へ出る。人格を渡さない専用文脈で作らせているが、
    // 利用者の発話に人格設定が引用されている場合などに復唱しうるので、公開経路と同じ検査を通す。
    // 落ちたら計画そのものを出さない（部分削除した残骸は見せない）。
    super::guard_public_answer(format!("【実行計画】\n{}", plan_lines.join("\n")), "")
        .map(Some)
        .map_err(|reason| anyhow::anyhow!("実行計画を公開できません: {reason}"))
}

pub(super) async fn self_improve(
    provider: &dyn Provider,
    cfg: &Config,
    model: &str,
    messages: &[Message],
    answer: String,
) -> anyhow::Result<String> {
    if !cfg.get_bool("PICOAGENT_SELF_EVAL_ENABLED", false) && !super_agent_enabled(cfg) {
        return Ok(answer);
    }
    if agent_depth() > 1 {
        return Ok(answer);
    }
    let too_short = answer.trim().chars().count() < 8;
    let refusal = (answer.contains("申し訳") && answer.contains("できません"))
        || answer.contains("お答えできません");
    if !too_short && !refusal {
        return Ok(answer);
    }
    let clean = super::context::request_view(messages)?;
    // 再生成の失敗はターン全体へ伝播する。元の不十分な回答も公開・保存しない。
    let message = provider
        // 非公開利用: 再生成した本文もagent::runの公開検査を通るまでは利用者へ渡さない。
        .chat(cfg, model, &clean, &[])
        .await
        .map_err(|error| error.context("自己評価の再生成に失敗しました"))?;
    anyhow::ensure!(
        !message.content.trim().is_empty(),
        "自己評価の再生成が空の回答を返しました"
    );
    Ok(message.content)
}

#[cfg(test)]
mod tests {
    use super::{extract_json_object, is_complex_query};

    #[test]
    fn 計画json以外の本文を成功扱いしない() {
        assert!(
            extract_json_object("計画: ```json\n{\"steps\":[\"調査\",\"実装\"]}\n```").is_err()
        );
        let value = extract_json_object("{\"steps\":[\"調査\",\"実装\"]}").unwrap();
        assert_eq!(value["steps"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn 短い単一質問は追加の計画推論を起動しない() {
        assert!(!is_complex_query("今日の天気は？"));
        assert!(is_complex_query(&format!(
            "{}？{}？",
            "手順を説明して".repeat(10),
            "検証方法も教えて"
        )));
    }
    struct 再生成検査 {
        calls: std::sync::atomic::AtomicUsize,
        result: Option<&'static str>,
        first_answer: &'static str,
        plans: std::sync::atomic::AtomicUsize,
        plan_result: Option<&'static str>,
        main_requests: std::sync::Mutex<Vec<Vec<crate::message::Message>>>,
        stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }

    #[async_trait::async_trait]
    impl crate::providers::Provider for 再生成検査 {
        fn id(&self) -> &'static str {
            "self-eval-test"
        }
        fn label(&self) -> &'static str {
            "自己評価検査"
        }
        fn default_model(&self, _: &crate::config::Config) -> String {
            "test".into()
        }
        async fn list_models(&self, _: &crate::config::Config) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        async fn is_up(&self, _: &crate::config::Config) -> Result<bool, String> {
            Ok(true)
        }
        async fn chat(
            &self,
            _: &crate::config::Config,
            _: &str,
            messages: &[crate::message::Message],
            _: &[serde_json::Value],
        ) -> anyhow::Result<crate::message::Message> {
            self.main_requests.lock().unwrap().push(messages.to_vec());
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return Ok(crate::message::Message::new("assistant", self.first_answer));
            }
            match self.result {
                Some(content) => Ok(crate::message::Message::new("assistant", content)),
                None => anyhow::bail!("再生成の接続が切れました"),
            }
        }
        async fn chat_with_options(
            &self,
            _: &crate::config::Config,
            _: &str,
            messages: &[crate::message::Message],
            tools: &[serde_json::Value],
            json_mode: bool,
        ) -> anyhow::Result<crate::message::Message> {
            assert!(json_mode);
            assert!(tools.is_empty());
            assert_eq!(messages.len(), 2);
            assert!(!messages[0].content.contains("通常会話だけの人格"));
            self.plans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(stop) = &self.stop {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            match self.plan_result {
                Some(content) => Ok(crate::message::Message::new("assistant", content)),
                None => Err(crate::providers::ProviderHttpError {
                    service: "計画検査".into(),
                    endpoint: "/chat".into(),
                    status: 503,
                    retry_after: None,
                    body: "計画を取得できません".into(),
                }
                .into()),
            }
        }
    }

    const 正常回答: &str = "工程は前提の確認、実施、結果の検証の順です。";
    const 正常計画: &str = r#"{"steps":["計画だけの準備","計画だけの検証"]}"#;

    fn 検査プロバイダ(
        first_answer: &'static str,
        result: Option<&'static str>,
    ) -> 再生成検査 {
        再生成検査 {
            calls: 0.into(),
            result,
            first_answer,
            plans: 0.into(),
            plan_result: Some(正常計画),
            main_requests: Default::default(),
            stop: None,
        }
    }

    fn 計画設定(super_agent: bool) -> crate::config::Config {
        let cfg = crate::config::Config::new();
        cfg.set(
            "PICOAGENT_SUPER_AGENT",
            if super_agent { "true" } else { "false" },
        );
        cfg.set("PICOAGENT_SUPER_AGENT_MAX_STEPS", "20");
        cfg.set("PICOAGENT_SELF_EVAL_ENABLED", "false");
        cfg
    }

    fn 複雑な依頼() -> String {
        "工程の順序と検査の観点を詳しく説明してください。".repeat(6)
    }

    #[tokio::test]
    async fn 通常送信と自己評価と公開再生成は同じ私有資料を参照し公開履歴には残さない() {
        use super::super::context::{ProtocolHistory, TurnContext, TurnContextSession};
        use std::sync::{Arc, Mutex};
        const MATERIAL: &str = "月の図書館で司書を務める設定";
        for self_eval in [true, false] {
            let cfg = 計画設定(false);
            cfg.set(
                "PICOAGENT_SELF_EVAL_ENABLED",
                if self_eval { "true" } else { "false" },
            );
            let provider =
                検査プロバイダ(if self_eval { "はい" } else { MATERIAL }, Some(正常回答));
            let mut messages = vec![crate::message::Message::new("user", "工程を説明して")];
            let original = messages[0].content.clone();
            let mut context = TurnContext::default();
            context.push("長期メモリ", MATERIAL);
            let session = TurnContextSession::new(
                ProtocolHistory::default(),
                &messages,
                context,
                6000,
                Arc::new(Mutex::new(crate::tools::ToolSchemaSet::default())),
                1,
            );
            let answer = super::super::with_turn_context(
                session.clone(),
                super::super::run(&cfg, &provider, "test", &mut messages, 2, None, None),
            )
            .await
            .unwrap();
            assert_eq!(answer, 正常回答);
            let requests = provider.main_requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            for request in requests.iter() {
                assert!(request
                    .iter()
                    .any(|message| message.role == "user" && message.content.contains(MATERIAL)));
            }
            assert_eq!(messages[0].content, original);
            assert!(messages
                .iter()
                .all(|message| !message.content.contains(MATERIAL)));
            session.finish(&messages, &messages).unwrap();
        }
    }

    #[tokio::test]
    async fn 中断ターンは安全な公開履歴があってもprotocolを確定しない() {
        use super::super::context::{ProtocolHistory, TurnContext, TurnContextSession};
        use std::sync::{Arc, Mutex};
        let cfg = 計画設定(false);
        let provider = 検査プロバイダ(正常回答, None);
        let mut messages = vec![crate::message::Message::new("user", "工程を説明して")];
        let session = TurnContextSession::new(
            ProtocolHistory::default(),
            &messages,
            TurnContext::default(),
            6000,
            Arc::new(Mutex::new(crate::tools::ToolSchemaSet::default())),
            1,
        );
        let cancel = std::sync::atomic::AtomicBool::new(true);
        super::super::with_turn_context(
            session.clone(),
            super::super::run(
                &cfg,
                &provider,
                "test",
                &mut messages,
                2,
                None,
                Some(&cancel),
            ),
        )
        .await
        .unwrap();
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(session.finish(&messages, &messages).is_err());
    }

    #[tokio::test]
    async fn 旧計画設定だけでは追加推論せず単純質問も計画しない() {
        for (super_agent, text) in [(false, 複雑な依頼()), (true, "工程を説明して".into())]
        {
            let cfg = 計画設定(super_agent);
            let old_key = ["PICOAGENT", "PLANNING", "ENABLED"].join("_");
            cfg.set(&old_key, "true");
            let provider = 検査プロバイダ(正常回答, None);
            let mut messages = vec![crate::message::Message::new("user", &text)];
            assert_eq!(
                super::super::run(&cfg, &provider, "test", &mut messages, 2, None, None)
                    .await
                    .unwrap(),
                正常回答
            );
            assert_eq!(provider.plans.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn 計画の通信失敗と不正応答では通常推論と回答保存へ進まない() {
        for (response, reason) in [
            (None, "実行計画の生成に失敗"),
            (Some(""), "JSON"),
            (Some("not JSON"), "JSON"),
            (Some("{}"), "steps配列"),
            (Some(r#"{"steps":"準備"}"#), "steps配列"),
            (Some(r#"{"steps":[]}"#), "1〜4件"),
            (Some(r#"{"steps":[" ","検証"]}"#), "空文字"),
            (Some(r#"{"steps":[null,"検証"]}"#), "文字列以外"),
            (
                Some(r#"{"steps":["ユーザーの依頼を分類して返答方針を決める","検証"]}"#),
                "公開できません",
            ),
        ] {
            let cfg = 計画設定(true);
            let mut provider = 検査プロバイダ(正常回答, None);
            provider.plan_result = response;
            let mut messages = vec![crate::message::Message::new("user", 複雑な依頼())];
            let error = super::super::run(&cfg, &provider, "test", &mut messages, 2, None, None)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(reason), "{response:?}: {error}");
            assert_eq!(provider.plans.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert!(!messages.iter().any(|message| message.role == "assistant"));
        }
    }

    #[tokio::test]
    async fn 正常計画は進捗だけで元の会話本文を変更しない() {
        let cfg = 計画設定(true);
        let provider = 検査プロバイダ(正常回答, None);
        let original = vec![
            crate::message::Message::new("system", "通常会話だけの人格"),
            crate::message::Message::new("user", 複雑な依頼()),
        ];
        let mut messages = original.clone();
        let answer = super::super::run(&cfg, &provider, "test", &mut messages, 2, None, None)
            .await
            .unwrap();
        assert_eq!(answer, 正常回答);
        assert_eq!(provider.plans.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let requests = provider.main_requests.lock().unwrap();
        assert_eq!(
            serde_json::to_value(&requests[0]).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&messages[..original.len()]).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("計画だけの")));
    }

    #[tokio::test]
    async fn 計画中の停止要求は計画失敗より優先し通常推論しない() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        for already_stopped in [false, true] {
            let cfg = 計画設定(true);
            let stop = Arc::new(AtomicBool::new(already_stopped));
            let mut provider = 検査プロバイダ(正常回答, None);
            provider.plan_result = None;
            provider.stop = Some(stop.clone());
            let mut messages = vec![crate::message::Message::new("user", 複雑な依頼())];
            assert_eq!(
                super::super::run(&cfg, &provider, "test", &mut messages, 2, None, Some(&stop))
                    .await
                    .unwrap(),
                super::super::cancel_message()
            );
            assert_eq!(
                provider.plans.load(Ordering::SeqCst),
                usize::from(!already_stopped)
            );
            assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
            assert_eq!(
                messages.last().unwrap().content,
                super::super::cancel_message()
            );
        }
    }

    #[tokio::test]
    async fn 自己評価の失敗と空回答は最初の回答を会話へ確定しない() {
        use crate::{config::Config, message::Message};
        for (result, reason) in [
            (None, "再生成に失敗"),
            (Some(" \n\t "), "空の回答"),
            (Some("再生成した十分な説明です。"), ""),
        ] {
            let cfg = Config::new();
            cfg.set("PICOAGENT_SELF_EVAL_ENABLED", "true");
            cfg.set("PICOAGENT_SUPER_AGENT", "false");
            let provider = 検査プロバイダ("短文", result);
            let mut messages = vec![Message::new("user", "仕組みの説明をしてください")];
            let response =
                super::super::run(&cfg, &provider, "test", &mut messages, 2, None, None).await;
            assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            if reason.is_empty() {
                assert_eq!(response.unwrap(), result.unwrap());
                assert_eq!(messages.last().unwrap().content, result.unwrap());
            } else {
                assert!(response.unwrap_err().to_string().contains(reason));
                assert!(!messages.iter().any(|message| message.role == "assistant"));
            }
        }
    }
}
