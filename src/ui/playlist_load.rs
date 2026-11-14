//! Loading a big playlist while it plays. The first page is the `QueueSeq`
//! reply; the rest come as `Msg::PlaylistPage` until the load is superseded.

use super::app::{Confirm, Msg, QueueAction};
use super::queue_seq::{QueueOutcome, QueueReplyGuard};
use crate::plex::client::{
    entry_key, PageWalk, MAX_PAGES, PLAYLIST_FIRST_PAGE, PLAYLIST_PAGE_SIZE,
};
use crate::plex::{PlexClient, Track};
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// The app's side of a playlist that is still arriving.
#[derive(Debug)]
pub struct PlaylistLoad {
    /// The request's seq; every page message carries it.
    pub id: u64,
    /// Set when the load is superseded; the worker checks it before each page.
    pub cancel: Arc<AtomicBool>,
    /// The first page is in the queue.
    pub landed: bool,
    pub loaded: usize,
    pub total: usize,
}

impl PlaylistLoad {
    pub fn new(id: u64) -> Self {
        Self { id, cancel: Arc::new(AtomicBool::new(false)), landed: false, loaded: 0, total: 0 }
    }

    /// `6 000 / 24 627`
    pub fn progress(&self) -> String {
        format!("{} / {}", group(self.loaded), group(self.total))
    }
}

/// 24627 -> "24 627"
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

pub struct PlaylistRequest {
    pub seq: u64,
    pub key: String,
    pub confirm: Confirm,
    pub action: QueueAction,
    /// An `entry_key`: start from that entry.
    pub start: Option<String>,
    /// Only a plain replace streams: an append or insert would interleave with
    /// other queue changes, and a shuffled replace needs the whole list.
    pub stream: bool,
    /// The total to show when the server reports no `totalSize`.
    pub expected: usize,
    pub cancel: Arc<AtomicBool>,
}

/// Tells the app a streamed load ended without a final page, so its state
/// never outlives the task.
struct LoadEnd {
    tx: UnboundedSender<Msg>,
    id: u64,
    /// Finished, or cancelled (the app already forgot it).
    quiet: bool,
    error: String,
}

impl Drop for LoadEnd {
    fn drop(&mut self) {
        if !self.quiet {
            let error = std::mem::take(&mut self.error);
            let _ = self.tx.send(Msg::PlaylistLoadStopped { load_id: self.id, error });
        }
    }
}

async fn next_page(
    client: &PlexClient,
    key: &str,
    walk: &mut PageWalk,
    size: usize,
) -> Result<Vec<Track>> {
    let offset = walk.offset;
    let page = client
        .playlist_page(key, offset, size)
        .await
        .with_context(|| format!("loading the page at offset {offset}"))?;
    Ok(walk.accept(page.tracks, page.total, size, entry_key))
}

/// The tracks from the entry `start` onward, or `None` when it is gone.
/// An entry, not a song: a playlist can hold the same song twice.
fn from_start(mut tracks: Vec<Track>, start: &str) -> Option<Vec<Track>> {
    let pos = tracks.iter().position(|t| entry_key(t) == start)?;
    Some(tracks.split_off(pos))
}

pub async fn run(client: PlexClient, tx: UnboundedSender<Msg>, req: PlaylistRequest) {
    let PlaylistRequest { seq, key, confirm, action, start, stream, expected, cancel } = req;
    let reply = QueueReplyGuard::new(tx.clone(), seq);

    if !stream {
        let outcome = match client.playlist_items(&key).await {
            Ok(tracks) => match start.as_deref() {
                None => QueueOutcome::Tracks { tracks, confirm, action },
                Some(s) => match from_start(tracks, s) {
                    Some(tracks) => QueueOutcome::Tracks { tracks, confirm, action },
                    // never silently replace their queue with the whole playlist
                    None => QueueOutcome::StartMissing,
                },
            },
            Err(e) => QueueOutcome::Failed(format!("resolving playlist: {e:#}")),
        };
        reply.send(outcome);
        return;
    }

    let mut end = LoadEnd { tx: tx.clone(), id: seq, quiet: false, error: "stopped early".into() };
    let mut walk = PageWalk::default();
    // the first page that holds the start track is the head
    let mut skipped = 0;
    let mut head = None;
    for _ in 0..MAX_PAGES {
        if cancel.load(Ordering::Relaxed) {
            end.quiet = true;
            return;
        }
        let size = if walk.offset == 0 { PLAYLIST_FIRST_PAGE } else { PLAYLIST_PAGE_SIZE };
        let page = match next_page(&client, &key, &mut walk, size).await {
            Ok(p) => p,
            Err(e) => {
                reply.send(QueueOutcome::Failed(format!("resolving playlist: {e:#}")));
                return;
            }
        };
        match start.as_deref() {
            None => head = Some(page),
            Some(s) => {
                let n = page.len();
                match from_start(page, s) {
                    Some(tracks) => {
                        skipped += n - tracks.len();
                        head = Some(tracks);
                    }
                    None => skipped += n,
                }
            }
        }
        if head.is_some() || walk.done {
            break;
        }
    }
    let Some(head) = head else {
        reply.send(QueueOutcome::StartMissing);
        return;
    };
    if cancel.load(Ordering::Relaxed) {
        end.quiet = true;
        return;
    }
    let mut loaded = head.len();
    if walk.done {
        end.quiet = true;
        reply.send(QueueOutcome::Tracks { tracks: head, confirm, action });
        return;
    }
    let total = walk.total.map(|t| t.saturating_sub(skipped)).unwrap_or(expected);
    reply.send(QueueOutcome::Streaming { tracks: head, confirm, load_id: seq, total });

    for _ in 0..MAX_PAGES {
        if cancel.load(Ordering::Relaxed) {
            end.quiet = true;
            return;
        }
        let page = next_page(&client, &key, &mut walk, PLAYLIST_PAGE_SIZE).await;
        if cancel.load(Ordering::Relaxed) {
            end.quiet = true;
            return;
        }
        match page {
            Ok(tracks) => {
                loaded += tracks.len();
                let last = walk.done;
                let _ = tx.send(Msg::PlaylistPage { load_id: seq, tracks, last });
                if last {
                    end.quiet = true;
                    return;
                }
            }
            Err(e) => {
                end.error = format!("{e:#}");
                tracing::warn!(loaded, "playlist load stopped: {e:#}");
                return;
            }
        }
    }
    end.error = format!("more than {MAX_PAGES} pages of playlist {key}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::{playlist_xml, serve_pages};
    use std::sync::Mutex;
    use tokio::sync::mpsc::unbounded_channel;

    /// `n` entries; honours paging and reports totalSize.
    async fn server(n: usize) -> (PlexClient, Arc<Mutex<Vec<String>>>) {
        let (base, log) = serve_pages(move |start, size, _| {
            let s = start.min(n);
            Some(playlist_xml(s, (s + size).min(n), Some(n), |i| format!("k{i}")))
        })
        .await;
        (PlexClient::new(&base, "faketoken-cccc"), log)
    }

    fn request(stream: bool, start: Option<&str>) -> (PlaylistRequest, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let req = PlaylistRequest {
            seq: 4,
            key: "9".into(),
            confirm: Confirm::Playlist { name: "p".into() },
            action: QueueAction::Replace,
            start: start.map(String::from),
            stream,
            expected: 1,
            cancel: cancel.clone(),
        };
        (req, cancel)
    }

    async fn run_all(client: PlexClient, req: PlaylistRequest) -> Vec<Msg> {
        let (tx, mut rx) = unbounded_channel();
        run(client, tx, req).await;
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    fn kind(m: &Msg) -> &'static str {
        match m {
            Msg::QueueReply { .. } => "queue reply",
            Msg::PlaylistPage { .. } => "page",
            Msg::PlaylistLoadStopped { .. } => "stopped",
            _ => "other",
        }
    }

    fn reply_of(m: &Msg) -> &QueueOutcome {
        match m {
            Msg::QueueReply { seq: 4, outcome } => outcome,
            other => panic!("expected the queue reply, got a {}", kind(other)),
        }
    }

    #[tokio::test]
    async fn streamed_load_replies_then_pages_in_order() {
        let (client, log) = server(5237).await;
        let (req, _) = request(true, None);
        let msgs = run_all(client, req).await;

        let QueueOutcome::Streaming { tracks, load_id: 4, total: 5237, .. } = reply_of(&msgs[0])
        else {
            panic!("first reply was a {}", kind(&msgs[0]));
        };
        assert_eq!(tracks.len(), 200);
        let pages: Vec<(usize, bool, String)> = msgs[1..]
            .iter()
            .map(|m| match m {
                Msg::PlaylistPage { load_id: 4, tracks, last } => {
                    (tracks.len(), *last, tracks[0].rating_key.clone())
                }
                other => panic!("unexpected {}", kind(other)),
            })
            .collect();
        assert_eq!(
            pages,
            vec![
                (2500, false, "k200".into()),
                (2500, false, "k2700".into()),
                (37, true, "k5200".into())
            ]
        );
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 4, "one request per page: {log:?}");
        assert!(log[0].contains("Container-Start=0&X-Plex-Container-Size=200"));
        assert!(log[1].contains("Container-Start=200&X-Plex-Container-Size=2500"));
    }

    #[tokio::test]
    async fn small_playlist_is_one_plain_reply() {
        let (client, _) = server(150).await;
        let (req, _) = request(true, None);
        let msgs = run_all(client, req).await;
        assert_eq!(msgs.len(), 1);
        assert!(
            matches!(reply_of(&msgs[0]), QueueOutcome::Tracks { tracks, .. } if tracks.len() == 150)
        );
    }

    #[tokio::test]
    async fn start_track_in_first_page_slices_head() {
        let (client, _) = server(5237).await;
        let (req, _) = request(true, Some("120"));
        let msgs = run_all(client, req).await;
        let QueueOutcome::Streaming { tracks, total, .. } = reply_of(&msgs[0]) else { panic!() };
        assert_eq!((tracks[0].rating_key.as_str(), tracks.len()), ("k120", 80));
        assert_eq!(*total, 5237 - 120);
    }

    #[tokio::test]
    async fn start_track_beyond_first_page_found() {
        let (client, _) = server(5237).await;
        let (req, _) = request(true, Some("300"));
        let msgs = run_all(client, req).await;
        // found in the second page (200..2700)
        let QueueOutcome::Streaming { tracks, total, .. } = reply_of(&msgs[0]) else { panic!() };
        assert_eq!((tracks[0].rating_key.as_str(), tracks.len()), ("k300", 2400));
        assert_eq!(*total, 5237 - 300);
        let rest: usize = msgs[1..]
            .iter()
            .map(|m| match m {
                Msg::PlaylistPage { tracks, .. } => tracks.len(),
                other => panic!("unexpected {}", kind(other)),
            })
            .sum();
        assert_eq!(tracks.len() + rest, 5237 - 300);
    }

    /// Enter on the second copy of a song starts from the second copy.
    #[tokio::test]
    async fn start_is_the_entry_not_the_song() {
        let (base, _) = serve_pages(move |start, size, _| {
            let s = start.min(300);
            let key = |i: usize| {
                if i == 5 || i == 40 {
                    "same-song".to_string()
                } else {
                    format!("k{i}")
                }
            };
            Some(playlist_xml(s, (s + size).min(300), Some(300), key))
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-dddd");
        let (req, _) = request(true, Some("40"));
        let msgs = run_all(client, req).await;
        let QueueOutcome::Streaming { tracks, total, .. } = reply_of(&msgs[0]) else { panic!() };
        assert_eq!(tracks[0].title, "T40");
        assert_eq!(*total, 300 - 40);
    }

    #[tokio::test]
    async fn vanished_start_track_ends_load() {
        let (client, _) = server(2600).await;
        let (req, _) = request(true, Some("gone"));
        let msgs = run_all(client, req).await;
        assert!(matches!(reply_of(&msgs[0]), QueueOutcome::StartMissing));
        assert!(matches!(&msgs[1], Msg::PlaylistLoadStopped { load_id: 4, .. }));
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn later_page_failure_ends_load() {
        let (base, _) = serve_pages(|start, size, _| {
            (start < 2700).then(|| {
                playlist_xml(start, start + size.min(5000 - start), Some(5000), |i| format!("k{i}"))
            })
        })
        .await;
        let (req, _) = request(true, None);
        let msgs = run_all(PlexClient::new(&base, "faketoken-cccc"), req).await;
        assert!(matches!(reply_of(&msgs[0]), QueueOutcome::Streaming { .. }));
        assert!(matches!(&msgs[1], Msg::PlaylistPage { last: false, .. }));
        let Msg::PlaylistLoadStopped { load_id: 4, error } = &msgs[2] else {
            panic!("third was a {}", kind(&msgs[2]))
        };
        assert!(error.contains("offset 2700") && !error.contains("faketoken"), "{error}");
        assert_eq!(msgs.len(), 3);
    }

    #[tokio::test]
    async fn first_page_failure_is_failed_reply() {
        let (base, _) = serve_pages(|_, _, _| None).await;
        let (req, _) = request(true, None);
        let msgs = run_all(PlexClient::new(&base, "faketoken-cccc"), req).await;
        assert!(matches!(reply_of(&msgs[0]), QueueOutcome::Failed(_)));
        assert!(matches!(&msgs[1], Msg::PlaylistLoadStopped { load_id: 4, .. }));
    }

    #[tokio::test]
    async fn cancelled_load_stops_fetching() {
        let (client, log) = server(8000).await;
        let (req, cancel) = request(true, None);
        let (tx, mut rx) = unbounded_channel();
        let task = tokio::spawn(run(client, tx, req));
        // cancel before the first tail page lands
        let first = rx.recv().await.unwrap();
        assert!(matches!(reply_of(&first), QueueOutcome::Streaming { .. }));
        cancel.store(true, Ordering::Relaxed);
        task.await.unwrap();
        let mut pages = 0;
        while let Ok(m) = rx.try_recv() {
            assert!(!matches!(m, Msg::PlaylistLoadStopped { .. }), "a cancelled load is quiet");
            pages += usize::from(matches!(m, Msg::PlaylistPage { .. }));
        }
        assert_eq!(pages, 0, "no page is delivered after the cancel");
        assert!(log.lock().unwrap().len() <= 2, "{:?}", log.lock().unwrap());
    }

    #[tokio::test]
    async fn append_gets_whole_playlist_in_one_reply() {
        let (client, log) = server(5237).await;
        let (mut req, _) = request(false, None);
        req.action = QueueAction::Append;
        let msgs = run_all(client, req).await;
        assert_eq!(msgs.len(), 1);
        assert!(
            matches!(reply_of(&msgs[0]), QueueOutcome::Tracks { tracks, .. } if tracks.len() == 5237)
        );
        assert_eq!(log.lock().unwrap().len(), 3);
    }

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(group(6000), "6 000");
        assert_eq!(group(24627), "24 627");
        assert_eq!(group(37), "37");
    }
}
