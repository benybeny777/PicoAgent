// 画面通知の形式・送信・失敗からの復旧判定をここで管理する。
use std::sync::Mutex;

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub title: String,
    pub body: String,
    pub ok: bool,
    pub at: String,
    #[serde(flatten)]
    details: Map<String, Value>,
}

impl Notice {
    pub fn info(kind: &str, title: impl Into<String>, body: impl Into<String>, ok: bool) -> Self {
        Self {
            kind: kind.into(),
            key: None,
            title: title.into(),
            body: body.into(),
            ok,
            at: chrono::Local::now().to_rfc3339(),
            details: Map::new(),
        }
    }

    pub fn background_failure(key: &str, title: &str, body: impl Into<String>) -> Self {
        let mut notice = Self::info("background_failure", title, body, false);
        notice.key = Some(key.into());
        notice
    }

    pub fn recovered(key: &str, title: &str, body: &str) -> Self {
        let mut notice = Self::background_failure(key, title, body);
        notice.ok = true;
        notice
    }

    pub fn detail(mut self, key: &'static str, value: Value) -> Self {
        self.details.insert(key.into(), value);
        self
    }
}

#[derive(Default)]
struct Delivery {
    ready: bool,
    pending: Vec<Notice>,
    failures: std::collections::BTreeMap<String, Notice>,
}

impl Delivery {
    fn submit(&mut self, notice: Notice) -> Option<Notice> {
        if notice.kind == "background_failure" {
            if let Some(key) = &notice.key {
                if notice.ok {
                    self.failures.remove(key);
                } else {
                    self.failures.insert(key.clone(), notice.clone());
                }
            }
        }
        if self.ready {
            Some(notice)
        } else {
            self.pending.push(notice);
            None
        }
    }

    fn start(&mut self) {
        self.ready = false;
    }

    fn take(&mut self) -> Vec<Notice> {
        let mut pending = std::mem::take(&mut self.pending);
        if !self.ready {
            // 画面の再読込時も、未復旧の失敗は新しい画面へ知らせる。
            for notice in self.failures.values() {
                if !pending.iter().any(|item| item.key == notice.key) {
                    pending.push(notice.clone());
                }
            }
        }
        self.ready = true;
        pending
    }
}

static DELIVERY: Mutex<Delivery> = Mutex::new(Delivery {
    ready: false,
    pending: Vec::new(),
    failures: std::collections::BTreeMap::new(),
});

pub fn begin_boot() {
    DELIVERY.lock().unwrap().start();
}

pub fn flush(app: &AppHandle) {
    // 待機通知とその直後の復旧を同じイベント経路に直列で送る。
    // ロックを外してから送ると、復旧が古い失敗より先に届いてしまう。
    let mut delivery = DELIVERY.lock().unwrap();
    for notice in delivery.take() {
        send(app, &notice);
    }
}

pub fn emit(app: &AppHandle, notice: Notice) {
    let mut delivery = DELIVERY.lock().unwrap();
    let Some(notice) = delivery.submit(notice) else {
        return;
    };
    send(app, &notice);
}

fn send(app: &AppHandle, notice: &Notice) {
    if let Err(error) = app.emit("pet:notice", notice) {
        crate::log_paths::append_app_log(
            app,
            &format!("{}を画面へ通知できません: {error}", notice.title),
        );
    }
}

/// 原因が変わった失敗と復旧だけを送る。並行報告の送信順も直列化する。
#[derive(Default)]
pub struct FailureLatch(Mutex<Option<String>>);

impl FailureLatch {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub fn is_failed(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }

    pub fn report(
        &self,
        app: &AppHandle,
        key: &str,
        title: &str,
        result: Result<(), String>,
        recovered_body: &str,
    ) {
        self.report_with(key, title, result, recovered_body, |notice| {
            emit(app, notice)
        });
    }

    fn report_with(
        &self,
        key: &str,
        title: &str,
        result: Result<(), String>,
        recovered_body: &str,
        send: impl FnOnce(Notice),
    ) {
        let next = result.err();
        let mut previous = self.0.lock().unwrap();
        if *previous == next {
            return;
        }
        let notice = match &next {
            Some(error) => Notice::background_failure(key, title, error),
            None => Notice::recovered(key, title, recovered_body),
        };
        *previous = next;
        send(notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 購読前の通知は初期化で一度受け取り再読込時は未復旧だけを再送する() {
        let mut delivery = Delivery::default();
        assert!(delivery
            .submit(Notice::info("update", "更新", "新しい版", true))
            .is_none());
        assert!(delivery
            .submit(Notice::background_failure("save", "保存", "失敗"))
            .is_none());
        assert_eq!(delivery.take().len(), 2);
        assert!(delivery.take().is_empty());
        delivery.start();
        assert_eq!(delivery.take().len(), 1);
        assert!(delivery
            .submit(Notice::recovered("save", "保存", "復旧"))
            .is_some());
        delivery.start();
        assert!(delivery.take().is_empty());
    }

    #[test]
    fn 失敗を間引き原因変更と復旧後の再失敗を知らせる() {
        let latch = FailureLatch::new();
        let mut sent = Vec::new();
        for result in [
            Ok(()),
            Err("保存不可".into()),
            Err("保存不可".into()),
            Err("容量不足".into()),
            Ok(()),
            Ok(()),
            Err("保存不可".into()),
        ] {
            latch.report_with("save", "保存", result, "復旧", |notice| {
                sent.push(notice)
            });
        }
        assert_eq!(sent.len(), 4);
        assert_eq!(
            sent.iter().map(|n| n.ok).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        assert_eq!(sent[1].body, "容量不足");
    }

    #[test]
    fn 共通キーと種類固有のキーと時刻を維持する() {
        let notice = Notice::info("durable_job", "処理", "完了", true)
            .detail("jobId", serde_json::json!("job-1"));
        let value = serde_json::to_value(notice).unwrap();
        for key in ["kind", "title", "body", "ok", "at", "jobId"] {
            assert!(value.get(key).is_some());
        }
        assert!(chrono::DateTime::parse_from_rfc3339(value["at"].as_str().unwrap()).is_ok());
        assert_eq!(
            serde_json::to_value(Notice::recovered("save", "保存", "復旧")).unwrap()["key"],
            "save"
        );
    }

    #[test]
    fn 通知送信の正本を迂回しない() {
        let violations: Vec<_> = crate::guard_scan::all_sources()
            .iter()
            .filter(|source| source.path != std::path::Path::new("src-tauri/src/notice.rs"))
            .filter(|source| source.body().contains("\"pet:notice\""))
            .map(|source| &source.name)
            .collect();
        assert!(
            violations.is_empty(),
            "通知はnotice::emitを使用してください: {violations:?}"
        );
    }
}
