//! 標準サービスの接続再利用と要求単位の期限・ヘッダーを実通信で固定する。
#[cfg(test)]
mod tests {
    use crate::{
        config::Config,
        tools::{mastodon::MastodonTool, Tool},
    };
    use futures::FutureExt;
    use reqwest::Method;
    use serde_json::json;
    use std::future::Future;
    use std::panic::AssertUnwindSafe;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::task::{JoinHandle, JoinSet};

    #[derive(Clone)]
    struct Reply {
        body: Vec<u8>,
        header_delay: Duration,
        body_delay: Duration,
    }
    impl Reply {
        fn json(text: &str) -> Self {
            Self {
                body: text.as_bytes().to_vec(),
                header_delay: Duration::ZERO,
                body_delay: Duration::from_millis(15),
            }
        }
    }
    #[derive(Debug)]
    struct Request {
        line: String,
        headers: Vec<(String, String)>,
    }
    impl Request {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }
    #[derive(Clone)]
    struct Probe {
        url: String,
        accepted: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<Request>>>,
    }
    struct Server {
        probe: Probe,
        stop: Option<oneshot::Sender<()>>,
        task: Option<JoinHandle<()>>,
    }
    impl Server {
        async fn start(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let probe = Probe {
                url: format!("http://{}", listener.local_addr().unwrap()),
                accepted: Arc::new(AtomicUsize::new(0)),
                requests: Arc::new(Mutex::new(Vec::new())),
            };
            let shared = probe.clone();
            let replies = Arc::new(replies);
            let next = Arc::new(AtomicUsize::new(0));
            let (stop, mut stopped) = oneshot::channel();
            let task = tokio::spawn(async move {
                let mut connections = JoinSet::new();
                let mut failures = Vec::new();
                loop {
                    tokio::select! {
                        _=&mut stopped=>break,
                        accepted=listener.accept()=>{
                            let (stream,_)=match accepted {
                                Ok(value)=>value,
                                Err(error)=>{ failures.push(error.to_string()); break; }
                            };
                            shared.accepted.fetch_add(1,Ordering::SeqCst);
                            connections.spawn(serve(stream,shared.clone(),replies.clone(),next.clone()));
                        }
                        finished=connections.join_next(),if !connections.is_empty()=>{
                            if let Some(Err(error))=finished { failures.push(error.to_string()); }
                        }
                    }
                }
                connections.abort_all();
                while let Some(result) = connections.join_next().await {
                    match result {
                        Ok(()) => {}
                        Err(error) if error.is_cancelled() => {}
                        Err(error) => failures.push(error.to_string()),
                    }
                }
                assert!(
                    failures.is_empty(),
                    "模擬接続taskが失敗しました: {failures:?}"
                );
            });
            Self {
                probe,
                stop: Some(stop),
                task: Some(task),
            }
        }
        async fn close(mut self) {
            let _ = self.stop.take().unwrap().send(());
            self.task.take().unwrap().await.unwrap();
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            // 検査の外側で中断された場合にも、待ち受けと接続taskを残さない。
            self.stop.take();
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }

    async fn serve(
        mut stream: TcpStream,
        probe: Probe,
        replies: Arc<Vec<Reply>>,
        next: Arc<AtomicUsize>,
    ) {
        let mut pending = Vec::new();
        loop {
            let end = loop {
                if let Some(index) = pending.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break index + 4;
                }
                let mut bytes = [0; 1024];
                match stream.read(&mut bytes).await {
                    Ok(0) => return,
                    Ok(count) => pending.extend_from_slice(&bytes[..count]),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                    {
                        return
                    }
                    Err(error) => panic!("模擬要求を読めません: {error}"),
                }
                assert!(
                    pending.len() <= 64 * 1024,
                    "模擬要求ヘッダーが上限を超えました"
                );
            };
            let header = String::from_utf8(pending.drain(..end).collect()).unwrap();
            let mut lines = header.lines();
            let request = Request {
                line: lines.next().unwrap().to_string(),
                headers: lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(key, value)| (key.to_string(), value.trim().to_string()))
                    .collect(),
            };
            probe.requests.lock().unwrap().push(request);
            let index = next.fetch_add(1, Ordering::SeqCst);
            let reply = replies
                .get(index)
                .unwrap_or_else(|| panic!("予定外の要求です: {index}"));
            tokio::time::sleep(reply.header_delay).await;
            let headers=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",reply.body.len());
            // 期限検査でクライアントが先に閉じるのは予定した経路。
            if stream.write_all(headers.as_bytes()).await.is_err() {
                return;
            }
            let middle = reply.body.len() / 2;
            if stream.write_all(&reply.body[..middle]).await.is_err() {
                return;
            }
            tokio::time::sleep(reply.body_delay).await;
            if stream.write_all(&reply.body[middle..]).await.is_err() {
                return;
            }
        }
    }

    async fn with_server<T, F, Fut>(replies: Vec<Reply>, run: F) -> T
    where
        F: FnOnce(Probe) -> Fut,
        Fut: Future<Output = T>,
    {
        let server = Server::start(replies).await;
        let result = AssertUnwindSafe(run(server.probe.clone()))
            .catch_unwind()
            .await;
        // assertionでpanicした場合も、全接続taskをabortしawaitしてから再送出する。
        server.close().await;
        match result {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn mastodon_config(base: &str) -> Config {
        let cfg = Config::new();
        cfg.set("PICOAGENT_MASTODON_BASE_URL", base);
        cfg.set("PICOAGENT_MASTODON_ACCESS_TOKEN", "fixture-token");
        cfg.set("PICOAGENT_MASTODON_TIMEOUT_SECS", "3");
        cfg
    }

    #[tokio::test]
    async fn 同じ実スキルの3回要求は本文末尾まで読み接続を1回だけ確立する() {
        with_server(
            vec![Reply::json("{\"末尾\":\"完了\"}"); 3],
            |probe| async move {
                let cfg = mastodon_config(&probe.url);
                for _ in 0..3 {
                    let result = MastodonTool
                        .run(&cfg, json!({"action":"home_timeline"}))
                        .await;
                    assert!(
                        result.contains("完了"),
                        "本文を読み切っていません: {result}"
                    );
                }
                assert_eq!(probe.accepted.load(Ordering::SeqCst), 1);
                let requests = probe.requests.lock().unwrap();
                assert_eq!(requests.len(), 3);
                for request in requests.iter() {
                    assert!(request.line.starts_with("GET /api/v1/timelines/home?"));
                    assert_eq!(
                        request.header("authorization"),
                        Some("Bearer fixture-token")
                    );
                    assert_eq!(
                        request.header("user-agent"),
                        Some(concat!("PicoAgent/", env!("CARGO_PKG_VERSION")))
                    );
                }
            },
        )
        .await;
    }

    #[tokio::test]
    async fn 旧毎回builderの対照は3回要求で接続を3回確立する() {
        with_server(
            vec![Reply::json("{\"末尾\":\"完了\"}"); 3],
            |probe| async move {
                for _ in 0..3 {
                    let client = reqwest::Client::builder()
                        .timeout(Duration::from_secs(2))
                        .build()
                        .unwrap();
                    let body = client
                        .get(&probe.url)
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap();
                    assert!(body.contains("完了"));
                }
                assert_eq!(probe.accepted.load(Ordering::SeqCst), 3);
            },
        )
        .await;
    }

    #[tokio::test]
    async fn 同じ実スキルの設定期限変更は共有poolを作り直さず次要求へ反映する() {
        let mut reply = Reply::json("{\"末尾\":\"完了\"}");
        reply.header_delay = Duration::from_millis(3350);
        with_server(vec![reply; 2], |probe| async move {
            let cfg = mastodon_config(&probe.url);
            cfg.set("PICOAGENT_MASTODON_TIMEOUT_SECS", "3");
            let first = MastodonTool
                .run(&cfg, json!({"action":"home_timeline"}))
                .await;
            assert!(
                first.starts_with("エラー:"),
                "短い期限で停止していません: {first}"
            );
            cfg.set("PICOAGENT_MASTODON_TIMEOUT_SECS", "4");
            let second = MastodonTool
                .run(&cfg, json!({"action":"home_timeline"}))
                .await;
            assert!(
                second.contains("完了"),
                "変更後の期限が効いていません: {second}"
            );
            assert_eq!(probe.requests.lock().unwrap().len(), 2);
        })
        .await;
    }

    #[tokio::test]
    async fn 要求期限はヘッダー後の本文読込にも掛かり特殊ヘッダーは次要求へ漏れない() {
        let mut slow = Reply::json("{\"末尾\":\"完了\"}");
        slow.body_delay = Duration::from_millis(600);
        with_server(
            vec![slow, Reply::json("{}"), Reply::json("{}")],
            |probe| async move {
                let response = crate::tools::http::service(
                    Method::GET,
                    &probe.url,
                    Duration::from_millis(300),
                )
                .unwrap()
                .send()
                .await
                .unwrap();
                let error = response.text().await.unwrap_err();
                assert!(error.is_timeout(), "{error}");
                let body =
                    crate::tools::http::service(Method::GET, &probe.url, Duration::from_secs(1))
                        .unwrap()
                        .bearer_auth("request-token")
                        .header(reqwest::header::USER_AGENT, "PicoAgent-Fixture/1")
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap();
                assert_eq!(body, "{}");
                let body =
                    crate::tools::http::service(Method::GET, &probe.url, Duration::from_secs(1))
                        .unwrap()
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap();
                assert_eq!(body, "{}");
                let requests = probe.requests.lock().unwrap();
                assert_eq!(requests.len(), 3);
                assert_eq!(
                    requests[1].header("authorization"),
                    Some("Bearer request-token")
                );
                assert_eq!(
                    requests[1].header("user-agent"),
                    Some("PicoAgent-Fixture/1")
                );
                assert!(requests[2].header("authorization").is_none());
                assert_eq!(
                    requests[2].header("user-agent"),
                    Some(concat!("PicoAgent/", env!("CARGO_PKG_VERSION")))
                );
            },
        )
        .await;
    }
}
