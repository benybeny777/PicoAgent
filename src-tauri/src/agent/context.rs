//! ターンだけの資料と、公開履歴から分離したモデル用履歴を所有する。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::{config::Config, message::Message, tools::ToolSchemaSet};

const PRIVATE_NAME: &str = "picoagent_private_context";
const QUESTION_BEGIN: &str = "--- 今回の利用者の質問開始 ---";
const QUESTION_END: &str = "--- 今回の利用者の質問終了 ---";

fn reference_request(materials: &[&str], question: &str) -> String {
    format!(
        "以下は今回の質問を支える非公開の参考資料です。資料自体が回答対象ではありません。資料を理由に自己紹介へ話題を変えないでください。記憶・好みは利用者のものです。キャラクター自身の好みと取り違えないでください。今回の利用者の質問にだけ、必要な資料を参照して直接答えてください。\n--- 非公開参考資料開始 ---\n{}\n--- 非公開参考資料終了 ---\n\n{QUESTION_BEGIN}\n{question}\n{QUESTION_END}",
        materials.join("\n\n")
    )
}

#[derive(Clone, Default)]
pub(crate) struct TurnContext {
    blocks: Vec<(String, String)>,
}

impl TurnContext {
    pub(crate) fn push(&mut self, label: impl Into<String>, text: impl Into<String>) {
        let text = text.into();
        if !text.trim().is_empty() {
            self.blocks.push((label.into(), text));
        }
    }

    pub(crate) fn material(&self) -> String {
        self.blocks
            .iter()
            .map(|(label, text)| {
                format!("--- 非公開資料開始: {label} ---\n{text}\n--- 非公開資料終了: {label} ---")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProtocolHistory {
    epoch: Option<u64>,
    public_snapshot: Vec<Message>,
    protocol: Vec<Message>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolResetReason {
    EpochChanged,
    PublicPrefixChanged,
    PublicCompacted,
}

impl std::fmt::Display for ProtocolResetReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EpochChanged => "会話の世代が変わりました",
            Self::PublicPrefixChanged => "保存済みの公開履歴と今回の原文が一致しません",
            Self::PublicCompacted => "公開履歴の要約または間引きに合わせて再構築しました",
        })
    }
}

struct SessionState {
    completed: bool,
    finished: bool,
    reason: Option<ProtocolResetReason>,
}

pub(crate) struct TurnContextSession {
    previous: ProtocolHistory,
    persona: String,
    context: TurnContext,
    budget: usize,
    schemas: Arc<Mutex<ToolSchemaSet>>,
    epoch: u64,
    state: Mutex<SessionState>,
}

fn equal(left: &[Message], right: &[Message]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.role == right.role
                && left.content == right.content
                && left.name == right.name
                && left.tool_call_id == right.tool_call_id
                && left.images == right.images
                && match (&left.tool_calls, &right.tool_calls) {
                    (None, None) => true,
                    (Some(left), Some(right)) => {
                        left.len() == right.len()
                            && left.iter().zip(right).all(|(left, right)| {
                                left.id == right.id
                                    && left.name == right.name
                                    && left.arguments == right.arguments
                            })
                    }
                    _ => false,
                }
        })
}

impl TurnContextSession {
    pub(crate) fn new(
        mut previous: ProtocolHistory,
        public_messages: &[Message],
        context: TurnContext,
        budget: usize,
        schemas: Arc<Mutex<ToolSchemaSet>>,
        epoch: u64,
    ) -> Arc<Self> {
        let reason = if previous.epoch.is_some_and(|old| old != epoch) {
            Some(ProtocolResetReason::EpochChanged)
        } else if previous.epoch.is_some()
            && (!public_messages.starts_with_public(&previous.public_snapshot))
        {
            Some(ProtocolResetReason::PublicPrefixChanged)
        } else {
            None
        };
        if reason.is_some() {
            previous = ProtocolHistory::default();
        }
        Arc::new(Self {
            previous,
            persona: public_messages
                .iter()
                .filter(|message| message.role == "system" && !is_private(message))
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
            context,
            budget,
            schemas,
            epoch,
            state: Mutex::new(SessionState {
                completed: false,
                finished: false,
                reason,
            }),
        })
    }

    pub(crate) fn reset_reason(&self) -> Option<ProtocolResetReason> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reason
    }

    pub(super) fn complete(&self, success: bool) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .completed = success;
    }

    pub(super) fn schemas(
        &self,
        config: &Config,
        text: &str,
        earlier: &[&str],
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.schemas
            .lock()
            .map_err(|_| anyhow::anyhow!("ツール宣言の排他が壊れました"))?
            .update(self.epoch, config, text, earlier)
            .map_err(anyhow::Error::msg)
    }

    fn protocol_view(&self, messages: &[Message]) -> anyhow::Result<Vec<Message>> {
        let prefix_len = self.previous.public_snapshot.len();
        anyhow::ensure!(
            messages.starts_with_public(&self.previous.public_snapshot),
            "ターン途中で公開履歴のprefixが変更されました"
        );
        let mut result = self.previous.protocol.clone();
        result.extend_from_slice(&messages[prefix_len..]);
        let material = self.context.material();
        if !material.is_empty() {
            let index = result
                .iter()
                .rposition(|message| message.role == "user")
                .ok_or_else(|| anyhow::anyhow!("非公開資料に対応する利用者発話がありません"))?;
            let mut private = Message::new("system", material);
            private.name = Some(PRIVATE_NAME.into());
            result.insert(index, private);
        }
        validate_pairs(&result)?;
        trim_groups(&mut result, self.budget)?;
        Ok(result)
    }

    pub(crate) fn request_view(&self, messages: &[Message]) -> anyhow::Result<Vec<Message>> {
        fold_private_context(&self.protocol_view(messages)?)
    }

    pub(crate) fn finish(
        &self,
        full: &[Message],
        compacted: &[Message],
    ) -> anyhow::Result<ProtocolHistory> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("ターン履歴の排他が壊れました"))?;
        anyhow::ensure!(
            state.completed && !state.finished,
            "未完了または確定済みのターン履歴は保存できません"
        );
        let mut protocol = self.protocol_view(full)?;
        let public: Vec<Message> = full
            .iter()
            .filter(|message| {
                message.role == "user"
                    || (message.role == "assistant"
                        && message.tool_calls.is_none()
                        && !message.content.is_empty())
                    || (message.role == "system" && !is_private(message))
            })
            .cloned()
            .collect();
        if !equal(&public, compacted) {
            // 要約や手動編集は公開履歴が正本。旧資料を別の発話へ結び付けない。
            protocol = compacted.to_vec();
            state.reason = Some(ProtocolResetReason::PublicCompacted);
        }
        state.finished = true;
        Ok(ProtocolHistory {
            epoch: Some(self.epoch),
            public_snapshot: compacted.to_vec(),
            protocol,
        })
    }
}

trait PublicPrefix {
    fn starts_with_public(&self, prefix: &[Message]) -> bool;
}
impl PublicPrefix for [Message] {
    fn starts_with_public(&self, prefix: &[Message]) -> bool {
        self.len() >= prefix.len() && equal(&self[..prefix.len()], prefix)
    }
}

fn is_private(message: &Message) -> bool {
    message.role == "system" && message.name.as_deref() == Some(PRIVATE_NAME)
}

/// 私有systemの位置を保ち、全providerで同じ利用者側資料として送る。
/// ツール後の末尾指示は次のモデル生成を求める利用者側命令として残す。
pub(crate) fn fold_private_context(messages: &[Message]) -> anyhow::Result<Vec<Message>> {
    validate_pairs(messages)?;
    let mut result = Vec::new();
    let mut pending = Vec::new();
    for message in messages {
        if is_private(message) {
            pending.push(message.content.as_str());
            continue;
        }
        let mut message = message.clone();
        if !pending.is_empty() {
            if message.role == "user" {
                message.content = reference_request(&pending, &message.content);
            } else {
                anyhow::ensure!(
                    message.role == "assistant",
                    "非公開指示の役割境界が不正です"
                );
                result.push(Message::new("user", pending.join("\n\n")));
            }
            pending.clear();
        }
        result.push(message);
    }
    if !pending.is_empty() {
        result.push(Message::new("user", pending.join("\n\n")));
    }
    Ok(result)
}

fn validate_pairs(messages: &[Message]) -> anyhow::Result<()> {
    let mut pending = HashSet::new();
    for message in messages {
        if message.role == "tool" {
            let id = message
                .tool_call_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("ツール結果の呼出IDがありません"))?;
            anyhow::ensure!(pending.remove(id), "対応する呼出がないツール結果です");
        } else if !is_private(message) {
            anyhow::ensure!(
                pending.is_empty(),
                "未完了のツール呼出を越えて発話が続いています"
            );
            if let Some(calls) = &message.tool_calls {
                anyhow::ensure!(message.role == "assistant", "ツール呼出の役割が不正です");
                for call in calls {
                    anyhow::ensure!(
                        !call.id.is_empty() && pending.insert(call.id.as_str()),
                        "ツール呼出IDが空または重複しています"
                    );
                }
            }
        }
    }
    anyhow::ensure!(pending.is_empty(), "ツール呼出に対応する結果がありません");
    Ok(())
}

fn estimate(message: &Message) -> usize {
    let mut wire = format!(
        "{}\n{}\n{}\n{}",
        message.role,
        message.content,
        message.name.as_deref().unwrap_or(""),
        message.tool_call_id.as_deref().unwrap_or("")
    );
    if let Some(calls) = &message.tool_calls {
        for call in calls {
            wire.push_str(&format!("{}\n{}\n{}", call.id, call.name, call.arguments));
        }
    }
    if let Some(images) = &message.images {
        for image in images {
            wire.push_str(image);
        }
    }
    let non_ascii = wire
        .chars()
        .filter(|character| !character.is_ascii())
        .count();
    non_ascii
        .saturating_mul(2)
        .saturating_add((wire.chars().count() - non_ascii).div_ceil(4))
}

fn trim_groups(messages: &mut Vec<Message>, budget: usize) -> anyhow::Result<()> {
    let starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if message.role != "user" {
                return None;
            }
            let mut start = index;
            while start > 0 && is_private(&messages[start - 1]) {
                start -= 1;
            }
            Some(start)
        })
        .collect();
    if starts.is_empty() {
        return Ok(());
    }
    // 各groupを一度だけ包んで計数する。削除するたびに全履歴を再変換しない。
    let cost = |slice: &[Message]| -> anyhow::Result<usize> {
        Ok(fold_private_context(slice)?
            .iter()
            .map(estimate)
            .fold(0usize, usize::saturating_add))
    };
    let mut total = cost(&messages[..starts[0]])?;
    let groups = starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(messages.len());
            cost(&messages[*start..end]).map(|cost| (*start, end, cost))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    for (_, _, cost) in &groups {
        total = total.saturating_add(*cost);
    }
    if total <= budget {
        return Ok(());
    }
    let target = budget.saturating_mul(7) / 10;
    let mut end = starts[0];
    for (_, group_end, cost) in groups.iter().take(groups.len().saturating_sub(1)) {
        if total <= target {
            break;
        }
        total = total.saturating_sub(*cost);
        end = *group_end;
    }
    messages.drain(starts[0]..end);
    Ok(())
}

tokio::task_local! { static SESSION: Arc<TurnContextSession>; }

pub(crate) async fn with_turn_context<F: std::future::Future>(
    session: Arc<TurnContextSession>,
    future: F,
) -> F::Output {
    SESSION.scope(session, future).await
}

pub(super) fn current() -> Option<Arc<TurnContextSession>> {
    if super::agent_depth() > 1 {
        return None;
    }
    SESSION.try_with(Arc::clone).ok()
}

pub(super) fn request_view(messages: &[Message]) -> anyhow::Result<Vec<Message>> {
    current().map_or_else(
        || fold_private_context(messages),
        |session| session.request_view(messages),
    )
}

pub(super) fn private_context() -> String {
    current().map_or_else(String::new, |session| {
        let mut parts = vec![session.persona.clone(), session.context.material()];
        parts.extend(
            session
                .previous
                .protocol
                .iter()
                .filter(|message| message.role == "system")
                .map(|message| message.content.clone()),
        );
        parts.join("\n\n")
    })
}

#[cfg(test)]
pub(crate) fn protocol_fixture() -> (Vec<Message>, Vec<Message>, Vec<Message>) {
    let mut private = Message::new("system", "過去の非公開資料: 月の図書館で司書を務める設定");
    private.name = Some(PRIVATE_NAME.into());
    let mut call = Message::new("assistant", "");
    call.tool_calls = Some(vec![crate::message::ToolCall {
        id: "pair-1".into(),
        name: "doc_search".into(),
        arguments: serde_json::json!({"query":"画像の説明"}),
    }]);
    let mut result = Message::new("tool", "検索資料の本文");
    result.tool_call_id = Some("pair-1".into());
    let before = vec![
        Message::new("system", "固定の人格設定"),
        private,
        Message::user_with_images(
            "画像の資料を説明して",
            vec!["data:image/png;base64,aW1hZ2U=".into()],
        ),
        call,
        result,
    ];
    let mut after = before.clone();
    after.push(Message::new("assistant", "画像の説明です。"));
    let mut private = Message::new("system", "今回の非公開資料: 紅茶を好む設定");
    private.name = Some(PRIVATE_NAME.into());
    after.extend([private, Message::new("user", "前の説明も使って比較して")]);
    (
        fold_private_context(&before).unwrap(),
        fold_private_context(&after).unwrap(),
        after,
    )
}

/// 改善前と同じ五往復を、推論せず正規形の送信境界だけで再現する。
#[cfg(test)]
pub(crate) fn cache_measurement_fixture(
) -> Vec<(Vec<Message>, Vec<serde_json::Value>, Option<String>)> {
    let _lock = super::PRIVATE_CONTEXT_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let remembered = super::remembered_private_context();
    let cfg = Config::new();
    cfg.set("PICOAGENT_LOCAL_CTX", "32768");
    cfg.set("PICOAGENT_SUPER_AGENT", "false");
    let texts = [
        "覚えている好みを一つ教えて。",
        "作業中の案件の締切を教えて。",
        "その案件で優先することを教えて。",
        "私の好きな飲み物を使う休憩案を教えて。",
        "今の案件と好みに合わせて一言教えて。",
    ];
    let persona = super::prompt::compose(&cfg, "名前: むぎ。穏やかに短く相談へ答える。", "mugi");
    let mut public = vec![Message::new("system", persona)];
    let mut previous = ProtocolHistory::default();
    let schemas = Arc::new(Mutex::new(ToolSchemaSet::default()));
    let mut rows = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let mut context = TurnContext::default();
        context.push(
            "記憶",
            format!("利用者は緑茶が好き。今回確認した回数は{}回。", index + 1),
        );
        context.push(
            "関係性",
            format!(
                "利用者とは作業の相談を続けている。今回の相談回数は{}回。",
                index + 1
            ),
        );
        context.push(
            "プロジェクト文脈",
            format!(
                "案件: 週報の整備。締切: 金曜日。最優先: 内容確認。進捗: {}段階。",
                index + 1
            ),
        );
        context.push(
            "利用者追加スキル",
            format!("weekly-report: 週報を整える。索引更新番号: {}。", index + 1),
        );
        public.push(Message::new("user", *text));
        let session =
            TurnContextSession::new(previous, &public, context, 32768, schemas.clone(), 1);
        let earlier = texts[..index]
            .iter()
            .rev()
            .take(3)
            .copied()
            .collect::<Vec<_>>();
        let tools = session.schemas(&cfg, text, &earlier).unwrap();
        let reset_reason = schemas.lock().unwrap().reset_reason().map(str::to_owned);
        rows.push((session.request_view(&public).unwrap(), tools, reset_reason));
        public.push(Message::new(
            "assistant",
            "状況を確認して、短く具体的に相談へ答えます。",
        ));
        session.complete(true);
        previous = session.finish(&public, &public).unwrap();
    }
    super::remember_private_context(&remembered);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(
        previous: ProtocolHistory,
        public: &[Message],
        material: &str,
        budget: usize,
        epoch: u64,
    ) -> Arc<TurnContextSession> {
        let mut context = TurnContext::default();
        context.push("長期メモリ", material);
        TurnContextSession::new(
            previous,
            public,
            context,
            budget,
            Arc::new(Mutex::new(ToolSchemaSet::default())),
            epoch,
        )
    }

    #[test]
    fn 過去の資料と画像とツールpairを保持し新しい資料だけ末尾へ付ける() {
        let public = vec![
            Message::new("system", "固定人格"),
            Message::user_with_images("最初の質問", vec!["data:image/png;base64,aW1hZ2U=".into()]),
        ];
        let first = session(
            ProtocolHistory::default(),
            &public,
            "過去の資料",
            usize::MAX,
            1,
        );
        let before = first.request_view(&public).unwrap();
        let mut full = public.clone();
        let (_, _, pairs) = protocol_fixture();
        full.extend_from_slice(&pairs[3..5]);
        full.push(Message::new("assistant", "最初の回答"));
        let mut clean = public.clone();
        clean.push(Message::new("assistant", "最初の回答"));
        first.complete(true);
        let history = first.finish(&full, &clean).unwrap();
        let mut next = clean.clone();
        next.push(Message::new("user", "次の質問"));
        let second = session(history, &next, "新しい資料", usize::MAX, 1);
        let after = second.request_view(&next).unwrap();
        assert!(equal(&before, &after[..before.len()]));
        assert_eq!(after[1].images, public[1].images);
        assert_eq!(after[2].tool_calls.as_ref().unwrap()[0].id, "pair-1");
        assert_eq!(after[3].tool_call_id.as_deref(), Some("pair-1"));
        assert!(after.last().unwrap().content.contains("新しい資料"));
        assert!(!next.last().unwrap().content.contains("新しい資料"));
        assert_eq!(second.reset_reason(), None);
    }

    #[test]
    fn 世代と公開原文の不一致は旧資料を照合前に捨てる() {
        let public = vec![Message::new("user", "原文")];
        let first = session(ProtocolHistory::default(), &public, "旧秘密", usize::MAX, 1);
        first.complete(true);
        let previous = first.finish(&public, &public).unwrap();
        for (epoch, reason, text) in [
            (2, ProtocolResetReason::EpochChanged, "原文"),
            (1, ProtocolResetReason::PublicPrefixChanged, "編集した原文"),
        ] {
            let next = vec![Message::new("user", text)];
            let next = session(previous.clone(), &next, "新資料", usize::MAX, epoch);
            assert_eq!(next.reset_reason(), Some(reason));
            assert!(next.previous.protocol.is_empty());
        }
    }

    #[test]
    fn 未完了と二重確定は保存せず要約時は公開正本から再構築する() {
        let public = vec![Message::new("user", "質問")];
        let value = session(ProtocolHistory::default(), &public, "資料", usize::MAX, 1);
        assert!(value.finish(&public, &public).is_err());
        value.complete(true);
        let compacted = vec![Message::new("system", "要約"), Message::new("user", "質問")];
        let result = value.finish(&public, &compacted).unwrap();
        assert!(equal(&result.protocol, &compacted));
        assert_eq!(
            value.reset_reason(),
            Some(ProtocolResetReason::PublicCompacted)
        );
        assert!(value.finish(&public, &compacted).is_err());
    }

    #[test]
    fn 圧縮は引数を数え七割まで完全な発話groupを削除しpairを孤立させない() {
        let (_, _, mut messages) = protocol_fixture();
        let calls = messages[3].tool_calls.as_mut().unwrap();
        calls[0].arguments = serde_json::json!({"query":"長い検索条件".repeat(1000)});
        let before: usize = messages.iter().map(estimate).sum();
        assert!(before > 1000);
        trim_groups(&mut messages, 1000).unwrap();
        validate_pairs(&messages).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.role == "user")
                .count(),
            1
        );
        assert!(messages
            .iter()
            .all(|message| message.tool_calls.is_none() && message.role != "tool"));
        assert!(
            fold_private_context(&messages)
                .unwrap()
                .iter()
                .map(estimate)
                .sum::<usize>()
                <= 700
        );
        let (_, _, mut unchanged) = protocol_fixture();
        let original = unchanged.clone();
        trim_groups(&mut unchanged, usize::MAX).unwrap();
        assert!(equal(&original, &unchanged));
    }

    #[test]
    fn 圧縮後の次ターンは公開原文から削除済み資料を再挿入せず残存資料の対応を保つ() {
        let (_, _, mut full) = protocol_fixture();
        full[3].tool_calls.as_mut().unwrap()[0].arguments =
            serde_json::json!({"query":"過去の巨大な検索条件".repeat(1000)});
        let public: Vec<Message> = full
            .iter()
            .filter(|message| {
                !is_private(message) && message.tool_calls.is_none() && message.role != "tool"
            })
            .cloned()
            .collect();
        // この試験は圧縮後も二つの短い資料付き発話を保持して対応を確かめる。
        // 資料・質問の固定包みが増えたため、包み込みの二発話が収まる予算を使う。
        let first = session(
            ProtocolHistory::default(),
            &public,
            "最新の補助資料",
            2000,
            1,
        );
        let view = first.request_view(&full).unwrap();
        assert!(view
            .iter()
            .all(|message| !message.content.contains("月の図書館")));
        full.push(Message::new("assistant", "比較の回答"));
        let mut compacted = public.clone();
        compacted.push(Message::new("assistant", "比較の回答"));
        first.complete(true);
        let previous = first.finish(&full, &compacted).unwrap();
        let mut next_public = compacted;
        next_public.push(Message::new("user", "次の相談"));
        let next = session(previous, &next_public, "次の補助資料", 2000, 1);
        let next_view = next.request_view(&next_public).unwrap();
        assert_eq!(next.reset_reason(), None);
        assert!(next_view
            .iter()
            .all(|message| !message.content.contains("月の図書館")));
        let last = next_view.last().unwrap();
        assert!(last.content.contains("次の補助資料"));
        assert!(last
            .content
            .contains(&format!("{QUESTION_BEGIN}\n次の相談\n{QUESTION_END}")));
        assert!(!last.content.contains("最新の補助資料"));
        assert!(next_view
            .iter()
            .any(|message| message.content.contains("最新の補助資料")
                && message.content.contains(&format!(
                    "{QUESTION_BEGIN}\n前の説明も使って比較して\n{QUESTION_END}"
                ))));
        validate_pairs(&next_view).unwrap();
    }

    #[test]
    fn 資料質問の包みも実送信予算へ数え超過したら七割までgroupを削除する() {
        let mut messages = vec![Message::new("system", "固定人格")];
        for index in 0..3 {
            let mut private = Message::new("system", format!("利用者の参考資料{index}"));
            private.name = Some(PRIVATE_NAME.into());
            messages.extend([
                private,
                Message::new("user", "質問です"),
                Message::new("assistant", "回答です"),
            ]);
        }
        let wrapped = fold_private_context(&messages).unwrap();
        let budget = wrapped.iter().map(estimate).sum::<usize>() - 1;
        assert!(messages.iter().map(estimate).sum::<usize>() < budget);
        trim_groups(&mut messages, budget).unwrap();
        let final_request = fold_private_context(&messages).unwrap();
        assert!(final_request.iter().map(estimate).sum::<usize>() <= budget * 7 / 10);
        assert!(
            final_request
                .iter()
                .filter(|message| message.role == "user")
                .count()
                < 3
        );
        assert!(final_request.last().unwrap().content.contains("回答です"));
    }

    #[test]
    fn 孤立した結果と欠けたpairを送信せず末尾指示は明示発話に保つ() {
        let (_, _, messages) = protocol_fixture();
        assert!(fold_private_context(&messages[..4]).is_err());
        assert!(fold_private_context(&messages[4..5]).is_err());
        let mut messages = vec![Message::new("user", "依頼")];
        let mut instruction = Message::new("system", "再生成の非公開指示");
        instruction.name = Some(PRIVATE_NAME.into());
        messages.push(instruction);
        let result = fold_private_context(&messages).unwrap();
        assert_eq!(result.last().unwrap().role, "user");
        assert_eq!(result.last().unwrap().content, "再生成の非公開指示");
    }

    #[test]
    fn 参考資料と今回の質問を固定境界で分け原文画像と前回prefixを保つ() {
        let (before, after, raw) = protocol_fixture();
        assert!(equal(&before, &after[..before.len()]));
        for (request, original) in [
            (&before[1], &raw[2]),
            (after.last().unwrap(), raw.last().unwrap()),
        ] {
            assert!(request.content.contains("記憶・好みは利用者のものです"));
            assert!(request
                .content
                .contains("資料を理由に自己紹介へ話題を変えないでください"));
            assert!(request.content.contains("今回の利用者の質問にだけ"));
            let (materials, question) = request.content.split_once(QUESTION_BEGIN).unwrap();
            assert!(materials.contains("--- 非公開参考資料開始 ---"));
            assert!(materials.contains("--- 非公開参考資料終了 ---"));
            assert_eq!(question, format!("\n{}\n{QUESTION_END}", original.content));
            assert_eq!(request.images, original.images);
        }
        assert!(!after[1].content.contains("紅茶を好む設定"));
        assert!(after.last().unwrap().content.contains("紅茶を好む設定"));
        assert!(!after
            .last()
            .unwrap()
            .content
            .contains("月の図書館で司書を務める設定"));
        assert!(equal(&after, &fold_private_context(&after).unwrap()));
        assert_eq!(after[2].tool_calls.as_ref().unwrap()[0].id, "pair-1");
        assert_eq!(after[3].tool_call_id.as_deref(), Some("pair-1"));
        let original =
            format!("この文字を説明して: {QUESTION_BEGIN}\n原文の続き\n{QUESTION_END} 🌿");
        let mut private = Message::new("system", "利用者は緑茶が好き");
        private.name = Some(PRIVATE_NAME.into());
        let raw = vec![
            private,
            Message::user_with_images(&original, vec!["data:image/png;base64,aW1hZ2U=".into()]),
        ];
        let result = fold_private_context(&raw).unwrap();
        let (_, question) = result[0].content.split_once(QUESTION_BEGIN).unwrap();
        assert_eq!(question, format!("\n{original}\n{QUESTION_END}"));
        assert_eq!(raw[1].content, original);
        assert_eq!(result[0].images, raw[1].images);
    }

    #[tokio::test]
    async fn 過去現在資料の公開を止め親scopeを子へ混ぜず取消scopeを保存しない() {
        let public = vec![Message::new("user", "質問")];
        let first = session(
            ProtocolHistory::default(),
            &public,
            "月の図書館で司書を務める設定",
            usize::MAX,
            1,
        );
        first.complete(true);
        let previous = first.finish(&public, &public).unwrap();
        let current = session(
            previous,
            &public,
            "森の薬草を集めて暮らす設定",
            usize::MAX,
            1,
        );
        with_turn_context(current.clone(), async {
            assert!(
                super::super::guard_public_answer("月の図書館で司書を務める設定".into(), "")
                    .is_err()
            );
            assert!(
                super::super::guard_public_answer("森の薬草を集めて暮らす設定".into(), "").is_err()
            );
            super::super::with_agent_depth(2, async {
                assert!(private_context().is_empty());
            })
            .await;
        })
        .await;
        assert!(current.finish(&public, &public).is_err());
        assert!(private_context().is_empty());
    }

    #[test]
    fn 初回ターンも受付時の人格をtasklocalで保持しglobal人格を上書きしない() {
        let _lock = super::super::PRIVATE_CONTEXT_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let public = vec![
            Message::new("system", "月の図書館で司書を務める選択人格"),
            Message::new("user", "質問"),
        ];
        let value = session(ProtocolHistory::default(), &public, "", 6000, 1);
        let remembered = super::super::remembered_private_context();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(with_turn_context(value, async {
                assert!(private_context().contains("月の図書館で司書を務める選択人格"));
                assert!(super::super::guard_public_answer(
                    "月の図書館で司書を務める選択人格".into(),
                    ""
                )
                .is_err());
            }));
        assert_eq!(super::super::remembered_private_context(), remembered);
    }
}
