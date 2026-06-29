// tracer.rs - ツール実行とエージェントループの分散トレーシング（レイテンシ計測）。
//
// AppState に Tracer を持ち、agent.rs の各ツール呼び出しで計測する。
// `pet_traces` コマンドで直近スナップショットとツール別集計を取得できる。

use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;

/// 1回のツール実行のトレース記録。
#[derive(Clone, Debug, Serialize)]
pub struct TraceEntry {
    /// ツール名（スキル名）
    pub tool: String,
    /// 実行時間（ミリ秒）
    pub duration_ms: u64,
    /// 成功 / 失敗（エラー文字列で返ってきた場合 false）
    pub success: bool,
}

/// AppState に埋め込むトレース収集器。スレッドセーフ。
pub struct Tracer {
    entries: Mutex<Vec<TraceEntry>>,
    max_entries: usize,
}

impl Tracer {
    pub fn new(max_entries: usize) -> Self {
        Tracer {
            entries: Mutex::new(Vec::new()),
            max_entries,
        }
    }

    /// ツール実行スパンを開始する。SpanGuard を drop するまで計測中。
    pub fn start_span(name: impl Into<String>) -> SpanGuard {
        SpanGuard {
            name: name.into(),
            start: Instant::now(),
        }
    }

    /// トレースエントリを追加する。max_entries を超えた分は古い順に削除する。
    pub fn record(&self, entry: TraceEntry) {
        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);
        let len = entries.len();
        if len > self.max_entries {
            entries.drain(0..len - self.max_entries);
        }
    }

    /// 直近 N 件のスナップショットを返す。
    pub fn snapshot(&self) -> Vec<TraceEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// ツール別の平均レイテンシと呼び出し回数をまとめて返す（降順）。
    pub fn summary(&self) -> Vec<serde_json::Value> {
        use std::collections::HashMap;
        let entries = self.entries.lock().unwrap();
        let mut totals: HashMap<String, (u64, u64, u64)> = HashMap::new(); // (sum_ms, count, err_count)
        for e in entries.iter() {
            let t = totals.entry(e.tool.clone()).or_default();
            t.0 += e.duration_ms;
            t.1 += 1;
            if !e.success {
                t.2 += 1;
            }
        }
        let mut result: Vec<serde_json::Value> = totals
            .into_iter()
            .map(|(tool, (sum, count, errors))| {
                serde_json::json!({
                    "tool": tool,
                    "avgMs": sum / count.max(1),
                    "callCount": count,
                    "errorCount": errors,
                })
            })
            .collect();
        result.sort_by(|a, b| {
            b["avgMs"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&a["avgMs"].as_u64().unwrap_or(0))
        });
        result
    }
}

/// ツール実行スパンのタイミングガード。`finish` を呼ぶと (名前, 経過時間) を返す。
pub struct SpanGuard {
    pub name: String,
    pub start: Instant,
}

impl SpanGuard {
    pub fn finish(self) -> (String, u64) {
        (self.name, self.start.elapsed().as_millis() as u64)
    }
}
