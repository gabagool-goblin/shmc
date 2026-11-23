//! Playback reports: sent to Plex one at a time, in order, from one task.

use super::PlexClient;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

impl PlayState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Stopped => "stopped",
        }
    }
}

/// What Plex is told about one track; `rating_key` is fixed when the track starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub rating_key: String,
    pub state: PlayState,
    pub time_ms: u64,
    pub duration_ms: u64,
}

/// Owns the sending task. Only the first failure reaches `on_first_failure`.
pub struct Reporter {
    tx: UnboundedSender<Report>,
    task: JoinHandle<()>,
}

impl Reporter {
    pub fn spawn(
        client: PlexClient,
        client_id: String,
        on_first_failure: impl FnOnce(String) + Send + 'static,
    ) -> Self {
        let (tx, mut rx) = unbounded_channel::<Report>();
        let task = tokio::spawn(async move {
            let mut failing = false;
            let mut on_first_failure = Some(on_first_failure);
            while let Some(report) = rx.recv().await {
                match client.timeline(&client_id, &report).await {
                    Ok(()) => failing = false,
                    Err(e) => {
                        if !failing {
                            tracing::warn!(
                                error = %crate::redact::escape_field(&format!("{e:#}")),
                                "playback report failed"
                            );
                        }
                        failing = true;
                        if let Some(note) = on_first_failure.take() {
                            note(format!("playback report failed: {e:#}"));
                        }
                    }
                }
            }
        });
        Self { tx, task }
    }

    pub fn sender(&self) -> UnboundedSender<Report> {
        self.tx.clone()
    }

    /// Closes the channel and waits up to `limit` for queued reports to go out.
    pub async fn finish(self, limit: Duration) {
        let Self { tx, mut task } = self;
        drop(tx);
        if tokio::time::timeout(limit, &mut task).await.is_err() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A fake PMS answering every request with `status`; `None` never answers.
    async fn serve(status: Option<u16>) -> (String, Arc<Mutex<Vec<String>>>) {
        serve_after_drops(status, 0).await
    }

    async fn serve_after_drops(
        status: Option<u16>,
        mut drops: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                log.lock().unwrap().push(req.lines().next().unwrap_or("").to_string());
                if drops > 0 {
                    drops -= 1;
                    continue;
                }
                match status {
                    Some(code) => {
                        let resp = format!(
                            "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    }
                    None => held.push(sock),
                }
            }
        });
        (base, seen)
    }

    fn report(key: &str, state: PlayState) -> Report {
        Report { rating_key: key.into(), state, time_ms: 1000, duration_ms: 2000 }
    }

    fn reporter(base: &str) -> (Reporter, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (notes, rx) = unbounded_channel();
        let note = move |n| {
            let _ = notes.send(n);
        };
        (Reporter::spawn(PlexClient::new(base, "faketoken"), "id".into(), note), rx)
    }

    #[tokio::test]
    async fn reports_arrive_in_the_order_sent() {
        let (base, seen) = serve(Some(200)).await;
        let (reporter, _notes) = reporter(&base);
        let tx = reporter.sender();
        for (key, state) in
            [("a", PlayState::Playing), ("a", PlayState::Stopped), ("b", PlayState::Playing)]
        {
            tx.send(report(key, state)).unwrap();
        }
        drop(tx);
        reporter.finish(Duration::from_secs(5)).await;
        let lines = seen.lock().unwrap().clone();
        let got: Vec<_> = lines
            .iter()
            .map(|l| {
                let url =
                    url::Url::parse(&format!("http://x{}", l.split(' ').nth(1).unwrap())).unwrap();
                let q = |k: &str| url.query_pairs().find(|(n, _)| n == k).unwrap().1.into_owned();
                (q("ratingKey"), q("state"))
            })
            .collect();
        let want = [("a", "playing"), ("a", "stopped"), ("b", "playing")];
        assert_eq!(got, want.map(|(a, b)| (a.to_string(), b.to_string())));
    }

    #[tokio::test]
    async fn failing_server_gets_each_report_once_and_the_ui_one_note() {
        let (base, seen) = serve(Some(500)).await;
        let (reporter, mut notes) = reporter(&base);
        let tx = reporter.sender();
        for _ in 0..3 {
            tx.send(report("a", PlayState::Playing)).unwrap();
        }
        drop(tx);
        reporter.finish(Duration::from_secs(5)).await;
        assert_eq!(seen.lock().unwrap().len(), 3);
        let mut errors = 0;
        while notes.try_recv().is_ok() {
            errors += 1;
        }
        assert_eq!(errors, 1);
    }

    #[tokio::test]
    async fn a_closed_connection_is_tried_again_once() {
        let (base, seen) = serve_after_drops(Some(200), 1).await;
        let (reporter, mut notes) = reporter(&base);
        reporter.sender().send(report("a", PlayState::Playing)).unwrap();
        reporter.finish(Duration::from_secs(5)).await;
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert!(notes.try_recv().is_err(), "no failure reached the UI");
    }

    #[tokio::test]
    async fn finish_gives_up_on_a_server_that_never_answers() {
        let (base, _seen) = serve(None).await;
        let (reporter, _notes) = reporter(&base);
        reporter.sender().send(report("a", PlayState::Stopped)).unwrap();
        let started = std::time::Instant::now();
        reporter.finish(Duration::from_secs(2)).await;
        assert!(started.elapsed() < Duration::from_millis(2500), "{:?}", started.elapsed());
    }
}
