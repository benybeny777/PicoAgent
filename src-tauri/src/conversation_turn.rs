// 会話ターンの待合せと、現在のターンだけへ効く停止要求を所有する。
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};

#[derive(Default)]
pub struct ConversationTurn {
    serial: AsyncMutex<()>,
    current: Mutex<Option<Arc<AtomicBool>>>,
}

pub struct TurnGuard<'a> {
    owner: &'a ConversationTurn,
    flag: Arc<AtomicBool>,
    _serial: MutexGuard<'a, ()>,
}

impl ConversationTurn {
    pub async fn acquire(&self) -> TurnGuard<'_> {
        self.start(self.serial.lock().await)
    }

    pub fn try_acquire(&self) -> Option<TurnGuard<'_>> {
        self.serial.try_lock().ok().map(|guard| self.start(guard))
    }

    fn start<'a>(&'a self, serial: MutexGuard<'a, ()>) -> TurnGuard<'a> {
        let flag = Arc::new(AtomicBool::new(false));
        *self.current.lock().unwrap() = Some(Arc::clone(&flag));
        TurnGuard {
            owner: self,
            flag,
            _serial: serial,
        }
    }

    pub fn cancel(&self) {
        if let Some(flag) = self.current.lock().unwrap().as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

impl TurnGuard<'_> {
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        let mut current = self.owner.current.lock().unwrap();
        if current
            .as_ref()
            .is_some_and(|flag| Arc::ptr_eq(flag, &self.flag))
        {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn 停止要求は実行ターンだけに効き終了後の次ターンへ残らない() {
        let turns = ConversationTurn::default();
        turns.cancel();
        let first = turns.acquire().await;
        let flag = first.flag();
        assert!(!flag.load(Ordering::Relaxed));
        turns.cancel();
        assert!(flag.load(Ordering::Relaxed));
        assert!(turns.try_acquire().is_none());
        drop(first);
        let second = turns.acquire().await;
        assert!(!second.flag().load(Ordering::Relaxed));
        assert!(flag.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn 処理破棄でも停止旗と排他を解除する() {
        let turns = Arc::new(ConversationTurn::default());
        let (ready, received) = oneshot::channel();
        let owner = Arc::clone(&turns);
        let task = tokio::spawn(async move {
            let turn = owner.acquire().await;
            ready.send(turn.flag()).unwrap();
            std::future::pending::<()>().await;
        });
        let flag = received.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        turns.cancel();
        assert!(!flag.load(Ordering::Relaxed));
        assert!(turns.try_acquire().is_some());
    }

    #[tokio::test]
    async fn 早期エラーも停止旗を解除しタスクローカルは同じ旗を配下へ渡す() {
        let turns = ConversationTurn::default();
        let result = {
            let turn = turns.acquire().await;
            let flag = turn.flag();
            crate::agent::with_cancel(Arc::clone(&flag), async {
                assert!(Arc::ptr_eq(&flag, &crate::agent::cancel_flag().unwrap()));
                Err::<(), _>("失敗")
            })
            .await
        };
        assert!(result.is_err());
        assert!(turns.current.lock().unwrap().is_none());
        assert!(turns.try_acquire().is_some());
    }
}
