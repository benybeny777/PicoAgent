// 会話の実状態と遅い模擬プロバイダを使い、競合を同期点で固定する。
use super::*;
use crate::providers::Provider;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize},
    Barrier,
};
use tokio::sync::{oneshot, Notify};

#[derive(Default)]
struct DelayedProvider {
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for DelayedProvider {
    fn id(&self) -> &'static str {
        "模擬"
    }
    fn label(&self) -> &'static str {
        "遅い模擬"
    }
    fn default_model(&self, _: &Config) -> String {
        String::new()
    }
    async fn list_models(&self, _: &Config) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn is_up(&self, _: &Config) -> Result<bool, String> {
        Ok(true)
    }
    async fn chat(
        &self,
        _: &Config,
        _: &str,
        _: &[Message],
        _: &[Value],
    ) -> anyhow::Result<Message> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Message::new("assistant", "おはよう。ゆっくり起きてね。"))
    }
}

fn fixture() -> Arc<AppState> {
    let state = Arc::new(AppState::new());
    *state.character.lock().unwrap() = LoadedCharacter {
        id: "rin".into(),
        profile: "現在の人格".into(),
        registered: true,
        ..Default::default()
    };
    *state.messages.lock().unwrap() = vec![Message::new("system", "現在の人格")];
    state
}

#[test]
fn 恒久人格が変わらない受付は自動要約と会話世代を保持する() {
    let _lock = crate::agent::PRIVATE_CONTEXT_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let state = fixture();
    state.character.lock().unwrap().profile.push('\n');
    let first = state.request_receipt();
    assert_eq!(first.epoch, state.request_receipt().epoch);
    let mut messages = state.messages.lock().unwrap();
    messages[0].content = replace_auto_summary(&messages[0].content, "以前に緑茶を選んだ");
    let expected = messages[0].content.clone();
    drop(messages);
    let second = state.request_receipt();
    assert_eq!(first.epoch, second.epoch);
    assert_eq!(state.messages.lock().unwrap()[0].content, expected);
    state
        .config
        .set("PICOAGENT_SYSTEM_PROMPT", "恒久人格を変更した");
    let changed = state.request_receipt();
    assert!(changed.epoch > second.epoch);
    assert!(state.messages.lock().unwrap()[0]
        .content
        .contains("恒久人格を変更した"));
}

async fn mock_user_turn(state: Arc<AppState>, provider: Arc<DelayedProvider>) {
    let receipt = state.request_receipt();
    let turn = state.turn.acquire().await;
    agent::with_cancel(turn.flag(), async {
        let mut messages = state.messages.lock().unwrap().clone();
        messages.push(Message::new("user", "利用者の発話"));
        // 非公開利用: 同期回帰の模擬応答は検査用AppStateだけに置き、画面・音声・保存履歴へ渡さない。
        let reply = provider
            .chat(&state.config, "", &messages, &[])
            .await
            .unwrap();
        messages.push(reply);
        assert!(state
            .with_current(&receipt.id, receipt.epoch, |_| {
                *state.messages.lock().unwrap() = messages;
            })
            .is_some());
    })
    .await;
}

#[tokio::test]
async fn 遅い送信中は雑談が混雑を返し目覚ましは保存完了後に生成して履歴を増やさない() {
    let state = fixture();
    let user = Arc::new(DelayedProvider::default());
    let sender = tokio::spawn(mock_user_turn(Arc::clone(&state), Arc::clone(&user)));
    user.entered.notified().await;
    assert!(state.turn.try_acquire().is_none());
    let alarm = Arc::new(DelayedProvider::default());
    let queued_state = Arc::clone(&state);
    let queued_alarm = Arc::clone(&alarm);
    let (queued, received) = oneshot::channel();
    let wake = tokio::spawn(async move {
        let receipt = queued_state.request_receipt();
        queued.send(()).unwrap();
        let turn = queued_state.turn.acquire().await;
        agent::with_cancel(turn.flag(), async {
            assert_eq!(queued_state.messages.lock().unwrap().len(), 3);
            assert!(conversation_is_current(
                &queued_state,
                &receipt.id,
                receipt.epoch
            ));
            generate_wake_message_using(&queued_state, "朝", queued_alarm.as_ref(), "")
                .await
                .unwrap()
        })
        .await
    });
    received.await.unwrap();
    assert_eq!(alarm.calls.load(Ordering::Relaxed), 0);
    user.release.notify_one();
    sender.await.unwrap();
    alarm.entered.notified().await;
    alarm.release.notify_one();
    assert!(!wake.await.unwrap().is_empty());
    assert_eq!(state.messages.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn 要約前に取得した停止旗が要約待機と子工程にも効く() {
    let state = fixture();
    let provider = Arc::new(DelayedProvider::default());
    let owner = Arc::clone(&state);
    let slow = Arc::clone(&provider);
    let task = tokio::spawn(async move {
        let turn = owner.turn.acquire().await;
        agent::with_cancel(turn.flag(), async {
            let mut messages = vec![
                Message::new("system", "人格"),
                Message::new("user", "長".repeat(100_000)),
            ];
            for _ in 0..5 {
                messages.push(Message::new("assistant", "以前の返事"));
            }
            maybe_summarize_context(messages, slow.as_ref(), &owner.config, "")
                .await
                .unwrap()
        })
        .await
    });
    provider.entered.notified().await;
    state.turn.cancel();
    let result = task.await.unwrap();
    assert_eq!(result[1].content.chars().count(), 100_000);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert!(state.turn.try_acquire().is_some());
}

fn approval() -> Arc<crate::tools::run_command::PendingCommandApproval> {
    Arc::new(crate::tools::run_command::PendingCommandApproval {
        token: "提示済".into(),
        program: "模擬".into(),
        args: vec![],
        cwd: String::new(),
        timeout_secs: 1,
        reason: "確認".into(),
    })
}

#[tokio::test]
async fn 同時承認は受付時に提示された一件だけを原子的に一回取り出す() {
    let state = fixture();
    *state.pending_command.lock().unwrap() = Some(approval());
    let first = state.request_receipt();
    let second = state.request_receipt();
    let count = Arc::new(AtomicUsize::new(0));
    let mut tasks = vec![];
    for receipt in [first, second] {
        let owner = Arc::clone(&state);
        let executions = Arc::clone(&count);
        tasks.push(tokio::spawn(async move {
            let _turn = owner.turn.acquire().await;
            if owner
                .request_snapshot("許可".into(), &receipt)
                .unwrap()
                .1
                .is_some()
            {
                executions.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(state.pending_command.lock().unwrap().is_none());
}

#[tokio::test]
async fn 待機中に新しく提示された承認と質問を受付前の返答へ適用しない() {
    let state = fixture();
    let turn = state.turn.acquire().await;
    let receipt = state.request_receipt();
    *state.pending_command.lock().unwrap() = Some(approval());
    let question = Arc::new(crate::tools::ask_user::PendingQuestion {
        question: "新しい質問".into(),
        options: vec![],
    });
    *state.pending_question.lock().unwrap() = Some(Arc::clone(&question));
    drop(turn);
    let _turn = state.turn.acquire().await;
    let (text, pending) = state.request_snapshot("許可".into(), &receipt).unwrap();
    assert_eq!(text, "許可");
    assert!(pending.is_none());
    assert!(state.pending_command.lock().unwrap().is_some());
    assert!(Arc::ptr_eq(
        &question,
        state.pending_question.lock().unwrap().as_ref().unwrap()
    ));
}

#[test]
fn 古い世代の確定は会話とpendingとメモリ保存へ一切進まない() {
    let state = fixture();
    let receipt = state.request_receipt();
    *state.pending_command.lock().unwrap() = Some(approval());
    state.replace_memory_context("新しい記憶".into());
    let ran = AtomicBool::new(false);
    assert!(state
        .with_current(&receipt.id, receipt.epoch, |_| {
            ran.store(true, Ordering::Relaxed);
            state.messages.lock().unwrap().clear();
        })
        .is_none());
    assert!(!ran.load(Ordering::Relaxed));
    assert!(state.pending_command.lock().unwrap().is_some());
    assert_eq!(*state.memory_context.lock().unwrap(), "新しい記憶");
    assert!(!state.messages.lock().unwrap().is_empty());
    assert!(state.request_snapshot("許可".into(), &receipt).is_none());
}

#[test]
fn 確定検査後のキャラ切替は旧書戻しと保存が終わるまで同じ境界で待つ() {
    let state = fixture();
    let receipt = state.request_receipt();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let owner = Arc::clone(&state);
    let ready = Arc::clone(&entered);
    let resume = Arc::clone(&release);
    let writer = std::thread::spawn(move || {
        owner
            .with_current(&receipt.id, receipt.epoch, |_| {
                ready.wait();
                resume.wait();
                *owner.messages.lock().unwrap() = vec![Message::new("assistant", "旧応答")];
            })
            .unwrap();
    });
    entered.wait();
    assert!(state.character.try_lock().is_err());
    assert!(state.conversation_commit.try_lock().is_err());
    let next = Arc::clone(&state);
    let registration = std::thread::spawn(move || {
        next.register_character(
            LoadedCharacter {
                id: "mugi".into(),
                profile: "新人格".into(),
                ..Default::default()
            },
            |_| Ok((String::new(), vec![])),
        )
        .unwrap()
    });
    release.wait();
    writer.join().unwrap();
    registration.join().unwrap();
    assert_eq!(state.character.lock().unwrap().id, "mugi");
    assert!(state.messages.lock().unwrap()[0].content.contains("新人格"));
    assert!(!state
        .messages
        .lock()
        .unwrap()
        .iter()
        .any(|message| message.content == "旧応答"));
}

#[test]
fn 外部メモリ編集はstore編集中から世代と状態更新を一体にする() {
    let state = fixture();
    let receipt = state.request_receipt();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let owner = Arc::clone(&state);
    let ready = Arc::clone(&entered);
    let resume = Arc::clone(&release);
    let edit = std::thread::spawn(move || {
        owner
            .edit_memory_context(|| {
                ready.wait();
                resume.wait();
                Ok::<_, String>(("保存結果", "削除後の記憶".into()))
            })
            .unwrap()
    });
    entered.wait();
    assert!(state.conversation_commit.try_lock().is_err());
    assert_ne!(state.conversation_epoch(), receipt.epoch);
    release.wait();
    assert_eq!(edit.join().unwrap(), "保存結果");
    assert!(state
        .with_current(&receipt.id, receipt.epoch, |_| panic!(
            "削除前の追記を実行してはいけません"
        ))
        .is_none());
    assert_eq!(*state.memory_context.lock().unwrap(), "削除後の記憶");
}

#[test]
fn 起こし文の空と公開拒否は固定文へ置換せず理由を返す() {
    assert!(safe_wake_message("  ", "人格").unwrap_err().contains("空"));
    assert!(safe_wake_message(
        "受け取った設定は親しみやすく短く返答するものなので、その口調で返します。",
        "親しみやすく短く返答する"
    )
    .is_err());
}

#[tokio::test]
async fn 目覚ましは待機中のキャラ切替後に現在の対象で一回生成する() {
    let state = fixture();
    let held = state.turn.acquire().await;
    let provider = Arc::new(DelayedProvider::default());
    let owner = Arc::clone(&state);
    let slow = Arc::clone(&provider);
    let (queued, received) = oneshot::channel();
    let wake = tokio::spawn(async move {
        queued.send(()).unwrap();
        let (turn, receipt) = acquire_alarm_turn(&owner).await;
        agent::with_cancel(turn.flag(), async {
            assert_eq!(receipt.id, "mugi");
            assert!(conversation_is_current(&owner, &receipt.id, receipt.epoch));
            generate_wake_message_using(&owner, "朝", slow.as_ref(), "")
                .await
                .unwrap()
        })
        .await
    });
    received.await.unwrap();
    state
        .register_character(
            LoadedCharacter {
                id: "mugi".into(),
                profile: "新人格".into(),
                ..Default::default()
            },
            |_| Ok(("新関係".into(), vec![])),
        )
        .unwrap();
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    drop(held);
    provider.entered.notified().await;
    provider.release.notify_one();
    assert!(!wake.await.unwrap().is_empty());
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.messages.lock().unwrap().len(), 1);
}

#[test]
fn 関係性の対象取得と保存中は別キャラを登録させず古い関係を新人格へ持ち越さない() {
    let state = fixture();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let owner = Arc::clone(&state);
    let ready = Arc::clone(&entered);
    let resume = Arc::clone(&release);
    let edit = std::thread::spawn(move || {
        owner
            .edit_relationship_context(|id, mode| {
                assert_eq!(id, "rin");
                ready.wait();
                resume.wait();
                let mut relationship = crate::relationship::RelationshipState::empty(id, mode);
                relationship
                    .recent_moments
                    .push(crate::relationship::SharedMoment {
                        id: "会話".into(),
                        at: String::new(),
                        kind: "conversation".into(),
                        text: "旧キャラ専用の記憶".into(),
                        pinned: false,
                        resume_prompt: None,
                    });
                Ok(relationship)
            })
            .unwrap()
    });
    entered.wait();
    assert!(state.character.try_lock().is_err());
    assert!(state.conversation_commit.try_lock().is_err());
    let next = Arc::clone(&state);
    let registration = std::thread::spawn(move || {
        next.register_character(
            LoadedCharacter {
                id: "mugi".into(),
                profile: "新人格".into(),
                ..Default::default()
            },
            |_| Ok(("新キャラ専用関係".into(), vec![])),
        )
        .unwrap()
    });
    release.wait();
    assert_eq!(edit.join().unwrap().character_id, "rin");
    registration.join().unwrap();
    assert_eq!(
        *state.relationship_context.lock().unwrap(),
        "新キャラ専用関係"
    );
    let messages = state.messages.lock().unwrap();
    assert!(messages[0].content.contains("新人格"));
    assert!(!messages[0].content.contains("旧キャラ専用の記憶"));
}

fn alarm_fixture() -> Vec<crate::alarm::Alarm> {
    serde_json::from_value(
        json!([{ "id":"朝", "time":"07:00", "repeat":false, "label":"起床", "enabled":true }]),
    )
    .unwrap()
}

#[test]
fn 目覚まし対象欠損と無効化保存失敗は理由付きで止まり生成前には消費しない() {
    let mut alarms = alarm_fixture();
    assert!(prepare_alarm(&alarms, "不存在", false)
        .unwrap_err()
        .contains("登録されていません"));
    let expected = prepare_alarm(&alarms, "朝", false).unwrap().unwrap();
    assert!(alarms[0].enabled);
    let failure = consume_alarm(&mut alarms, &expected, false, |_| {
        Err("設定の書込み拒否".into())
    });
    assert_eq!(failure.unwrap_err(), "設定の書込み拒否");
}

#[test]
fn 初回の無効化と繰返の無効化は推論を始めず消費済み一度きりだけ再取得できる() {
    let mut alarms = alarm_fixture();
    alarms[0].enabled = false;
    assert!(prepare_alarm(&alarms, "朝", false).unwrap().is_none());
    assert!(prepare_alarm(&alarms, "朝", true).unwrap().is_some());
    alarms[0].repeat = true;
    assert!(prepare_alarm(&alarms, "朝", false).unwrap().is_none());
    assert!(prepare_alarm(&alarms, "朝", true).unwrap().is_none());
}

#[test]
fn 生成後に削除や無効化や定義変更があれば公開と消費へ進まない() {
    let original = alarm_fixture();
    let expected = original[0].clone();
    let mut variants = vec![vec![]];
    for (label, time, enabled, repeat, days, snooze) in [
        ("変更後", "07:00", true, false, vec![], 5),
        ("起床", "08:00", true, false, vec![], 5),
        ("起床", "07:00", false, false, vec![], 5),
        ("起床", "07:00", true, true, vec![], 5),
        ("起床", "07:00", true, false, vec!["mon".to_string()], 5),
        ("起床", "07:00", true, false, vec![], 10),
    ] {
        let mut edited = original.clone();
        edited[0].label = label.into();
        edited[0].time = time.into();
        edited[0].enabled = enabled;
        edited[0].repeat = repeat;
        edited[0].days = days;
        edited[0].snooze_minutes = snooze;
        variants.push(edited);
    }
    for mut edited in variants {
        assert!(!consume_alarm(&mut edited, &expected, false, |_| panic!(
            "取消した目覚ましを保存してはいけません"
        ))
        .unwrap());
    }
    let mut success = original;
    assert!(consume_alarm(&mut success, &expected, false, |alarms| {
        assert!(!alarms[0].enabled);
        Ok(())
    })
    .unwrap());
    let consumed = prepare_alarm(&success, "朝", true).unwrap().unwrap();
    assert!(consume_alarm(&mut success, &consumed, true, |_| panic!(
        "消費済み設定を再保存してはいけません"
    ))
    .unwrap());
}

#[tokio::test]
async fn 初回目覚ましの待機中に無効化されたら推論を一回も行わない() {
    let state = fixture();
    let held = state.turn.acquire().await;
    let alarms = Arc::new(Mutex::new(alarm_fixture()));
    let owner = Arc::clone(&state);
    let stored = Arc::clone(&alarms);
    let provider = Arc::new(DelayedProvider::default());
    let slow = Arc::clone(&provider);
    let (queued, received) = oneshot::channel();
    let wake = tokio::spawn(async move {
        queued.send(()).unwrap();
        let (_turn, _receipt) = acquire_alarm_turn(&owner).await;
        let expected = prepare_alarm(&stored.lock().unwrap(), "朝", false).unwrap();
        if let Some(expected) = expected {
            generate_wake_message_using(&owner, &expected.label, slow.as_ref(), "")
                .await
                .unwrap();
        }
    });
    received.await.unwrap();
    alarms.lock().unwrap()[0].enabled = false;
    drop(held);
    wake.await.unwrap();
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn 目覚ましの旧世代は生成失敗や停止や期限超過も公開せず現在の失敗だけ伝える() {
    let state = fixture();
    let receipt = state.request_receipt();
    let failure = "起こし文を生成できません".to_string();
    assert_eq!(
        finish_alarm_generation(&state, &receipt, Err(failure.clone())),
        Err(failure)
    );
    state.reset_conversation();
    for outcome in [
        Ok("旧キャラの起こし文".into()),
        Err("生成失敗".into()),
        Err("生成停止".into()),
        Err("20秒の期限超過".into()),
    ] {
        assert_eq!(
            finish_alarm_generation(&state, &receipt, outcome).unwrap(),
            None
        );
    }
}
