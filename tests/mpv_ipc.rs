//! Drives a real mpv through the IPC layer, on a generated tone (`av://lavfi`)
//! and `--ao=null`: no network or audio device.

use shmc::config::MpvConfig;
use shmc::playback::{mpv, LoadItem, MpvCmd, MpvEvent};

/// The tests share audio and log resources, so they run one at a time.
/// Deadlines are long: a loaded machine can take seconds to start mpv.
static MPV_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The last events seen, so a timeout says what did arrive.
#[derive(Default)]
struct Seen(std::collections::VecDeque<String>);

impl Seen {
    fn push(&mut self, ev: &MpvEvent) {
        if self.0.len() == 12 {
            self.0.pop_front();
        }
        self.0.push_back(format!("{ev:?}"));
    }

    fn tail(&self) -> String {
        if self.0.is_empty() {
            "no events arrived".to_string()
        } else {
            format!("last events: {:?}", self.0)
        }
    }
}

/// Let a transition settle: briefly fold `playlist-pos` into `pos` and drop its
/// own late `end-file`.
async fn settle(events: &mut tokio::sync::mpsc::UnboundedReceiver<MpvEvent>, pos: &mut i64) {
    let until = tokio::time::Instant::now() + tokio::time::Duration::from_millis(400);
    loop {
        match tokio::time::timeout_at(until, events.recv()).await {
            Ok(Some(MpvEvent::Property { name, value })) if name == "playlist-pos" => {
                *pos = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
            }
            Ok(Some(_)) => {}
            _ => return,
        }
    }
}

/// Wait for mpv to report `want` by asking, as a late event can be missed. An
/// `end-file` fails unless `allow_stop`.
fn item(url: String) -> LoadItem {
    LoadItem { url, title: "Test track".into() }
}

async fn wait_for_property(
    handle: &mpv::MpvHandle,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<MpvEvent>,
    seen: &mut Seen,
    name: &str,
    want: i64,
    allow_stop: bool,
    deadline: tokio::time::Duration,
) -> i64 {
    let until = tokio::time::Instant::now() + deadline;
    loop {
        while let Ok(ev) = events.try_recv() {
            seen.push(&ev);
            match ev {
                MpvEvent::EndFile { reason } if allow_stop && reason == "stop" => {}
                MpvEvent::EndFile { reason } => panic!(
                    "playback ended while waiting for {name}={want}: {reason}; {}",
                    seen.tail()
                ),
                MpvEvent::Shutdown => panic!("mpv shut down"),
                _ => {}
            }
        }

        let value = handle
            .get_property(name)
            .await
            .unwrap_or_else(|e| panic!("get_property {name}: {e}; {}", seen.tail()));
        let value = value.as_i64().unwrap_or(-1);
        if value == want {
            return value;
        }
        if tokio::time::Instant::now() > until {
            panic!("timed out waiting for {name}={want}: mpv reports {value}; {}", seen.tail());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn mpv_plays_queue_and_reports_state() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");

    match events.recv().await {
        Some(MpvEvent::Ready) => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    let url = "av://lavfi:sine=frequency=440:duration=1".to_string();
    handle.tx.send(MpvCmd::Play(vec![item(url)])).expect("send play");

    let mut saw_playlist_pos = false;
    let mut saw_time = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    loop {
        let ev = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => panic!("event channel closed"),
            Err(_) => panic!("timed out waiting for mpv events"),
        };
        match ev {
            MpvEvent::Property { name, value } => match name.as_str() {
                "playlist-pos" => {
                    if value.as_ref().and_then(|v| v.as_i64()) == Some(0) {
                        saw_playlist_pos = true;
                    }
                }
                "time-pos" => {
                    if value.as_ref().and_then(|v| v.as_f64()).is_some() {
                        saw_time = true;
                    }
                }
                "pause" => {
                    assert_eq!(value.as_ref().and_then(|v| v.as_bool()), Some(false));
                }
                "volume" => {
                    assert_eq!(value.as_ref().and_then(|v| v.as_f64()), Some(50.0));
                }
                _ => {}
            },
            MpvEvent::EndFile { reason } => {
                assert_eq!(reason, "eof", "track should end at eof, not {reason}");
                break;
            }
            MpvEvent::Shutdown => panic!("mpv shut down early"),
            MpvEvent::Ready => {}
        }
    }

    assert!(saw_playlist_pos, "never observed playlist-pos 0");
    assert!(saw_time, "never observed time-pos");

    handle.shutdown().await;
}

#[tokio::test]
async fn mpv_gapless_advances_through_queue() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let mk = || "av://lavfi:sine=frequency=440:duration=1".to_string();
    handle.tx.send(MpvCmd::Play(vec![item(mk()), item(mk())])).expect("send play");

    let mut positions = std::collections::HashSet::new();
    let mut ends = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out");
        };
        if let MpvEvent::Property { name, value } = &ev {
            if name == "playlist-pos" {
                if let Some(p) = value.as_ref().and_then(|v| v.as_i64()) {
                    positions.insert(p);
                }
            }
        }
        if let MpvEvent::EndFile { reason } = ev {
            assert_eq!(reason, "eof");
            ends += 1;
            if ends == 2 {
                break;
            }
        }
    }
    assert!(positions.contains(&0), "never played entry 0: {positions:?}");
    assert!(positions.contains(&1), "never advanced to entry 1: {positions:?}");
    handle.shutdown().await;
}

/// `MpvCmd::Stop` halts playback but keeps the playlist, which mpv's own
/// `stop` would wipe.
#[tokio::test]
async fn mpv_stop_keeps_the_queue_playable() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");

    match events.recv().await {
        Some(MpvEvent::Ready) => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    let url = "av://lavfi:sine=frequency=440:duration=60".to_string();
    handle.tx.send(MpvCmd::Play(vec![item(url.clone()), item(url)])).expect("send play");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for playback");
        };
        match ev {
            MpvEvent::Property { name, value } if name == "time-pos" => {
                if value.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.2 {
                    break;
                }
            }
            MpvEvent::EndFile { reason } => panic!("ended before it could play: {reason}"),
            _ => {}
        }
    }

    handle.tx.send(MpvCmd::Stop).expect("send stop");

    // playback stops …
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for the stop");
        };
        match ev {
            MpvEvent::EndFile { reason } => {
                assert_eq!(reason, "stop", "Stop must halt playback, got {reason:?}");
                break;
            }
            MpvEvent::Shutdown => panic!("mpv shut down"),
            _ => {}
        }
    }

    // … but entry 1 is still there: the queue survived, so it can be played
    handle.tx.send(MpvCmd::PlayIndex(1)).expect("send play index");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for entry 1");
        };
        match ev {
            MpvEvent::Property { name, value } if name == "playlist-pos" => {
                if value.as_ref().and_then(|v| v.as_i64()) == Some(1) {
                    break;
                }
            }
            MpvEvent::EndFile { reason } => panic!("the queue did not survive the stop: {reason}"),
            MpvEvent::Shutdown => panic!("mpv shut down"),
            _ => {}
        }
    }

    // … and what `D` does next — stop, then clear — empties mpv's playlist
    // completely (a bare clear would keep the entry it had loaded).
    handle.tx.send(MpvCmd::Stop).expect("send stop");
    handle.tx.send(MpvCmd::Clear).expect("send clear");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for the playlist to empty");
        };
        match ev {
            MpvEvent::Property { name, value } if name == "playlist-count" => {
                if value.as_ref().and_then(|v| v.as_i64()) == Some(0) {
                    break;
                }
            }
            MpvEvent::EndFile { reason } => {
                assert_eq!(reason, "stop", "Stop must halt playback, got {reason:?}");
            }
            MpvEvent::Shutdown => panic!("mpv shut down"),
            _ => {}
        }
    }
    handle.shutdown().await;
}

/// `playlist-move` must keep the playing entry playing, or shuffle would
/// restart the song. Waits ask for the value, as in `wait_for_property`.
#[tokio::test]
async fn insert_and_move_keep_playing_entry() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let mk = |hz: u32| format!("av://lavfi:sine=frequency={hz}:duration=60");
    handle
        .tx
        .send(MpvCmd::Play(vec![item(mk(300)), item(mk(400)), item(mk(500))]))
        .expect("send play");

    let mut seen = Seen::default();
    let deadline = std::time::Duration::from_secs(25);
    wait_for_property(&handle, &mut events, &mut seen, "playlist-count", 3, false, deadline).await;
    wait_for_property(&handle, &mut events, &mut seen, "playlist-pos", 0, false, deadline).await;

    handle
        .tx
        .send(MpvCmd::Insert { index: 1, append_at: 3, items: vec![item(mk(600))] })
        .expect("send insert");
    wait_for_property(&handle, &mut events, &mut seen, "playlist-count", 4, false, deadline).await;
    // An insert is an append and a move; let both land before playing.
    let mut pos = 0i64;
    settle(&mut events, &mut pos).await;

    handle.tx.send(MpvCmd::PlayIndex(3)).expect("play index");
    let mut pos =
        wait_for_property(&handle, &mut events, &mut seen, "playlist-pos", 3, true, deadline).await;
    settle(&mut events, &mut pos).await;

    handle.tx.send(MpvCmd::Move { from: 3, to: 0 }).expect("send move");
    let mut pos =
        wait_for_property(&handle, &mut events, &mut seen, "playlist-pos", 0, false, deadline)
            .await;
    settle(&mut events, &mut pos).await;

    // Moving forward must land on the final index too (`J` and shuffle).
    handle.tx.send(MpvCmd::Move { from: 0, to: 3 }).expect("send forward move");
    wait_for_property(&handle, &mut events, &mut seen, "playlist-pos", 3, false, deadline).await;

    handle.shutdown().await;
}

/// Replies are matched to `get_property` calls by request id.
#[tokio::test]
async fn mpv_answers_get_property_requests() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let url = "av://lavfi:sine=frequency=440:duration=60".to_string();
    handle.tx.send(MpvCmd::Play(vec![item(url)])).expect("send play");

    let mut seen = Seen::default();
    let deadline = std::time::Duration::from_secs(30);
    wait_for_property(&handle, &mut events, &mut seen, "playlist-pos", 0, false, deadline).await;

    let count = handle.get_property("playlist-count").await.expect("get_property playlist-count");
    assert_eq!(count.as_i64(), Some(1), "one track is playing: {count}");

    // Each answer goes to its own request, not to whichever reply came first.
    let (a, b) =
        tokio::join!(handle.get_property("playlist-count"), handle.get_property("playlist-pos"));
    assert_eq!(a.expect("count").as_i64(), Some(1));
    assert_eq!(b.expect("pos").as_i64(), Some(0));

    // A property mpv does not have is an error, not a silent null.
    assert!(
        handle.get_property("shmc-no-such-property").await.is_err(),
        "an unknown property must come back as an error"
    );

    handle.shutdown().await;
}

/// Replacing the queue while playing must drop the old entry
/// (`playlist-clear` keeps the playing file).
#[tokio::test]
async fn mpv_replaces_the_playing_queue() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let mk = |hz: u32| format!("av://lavfi:sine=frequency={hz}:duration=60");
    handle.tx.send(MpvCmd::Play(vec![item(mk(300)), item(mk(400))])).expect("send old queue");

    let mut count = -1i64;
    let mut pos = -1i64;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for the old queue: count={count} pos={pos}");
        };
        match ev {
            MpvEvent::Property { name, value } if name == "playlist-count" => {
                count = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
            }
            MpvEvent::Property { name, value } if name == "playlist-pos" => {
                pos = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
            }
            MpvEvent::EndFile { reason } => panic!("old queue ended early: {reason}"),
            MpvEvent::Shutdown => panic!("mpv shut down"),
            _ => {}
        }
        if count == 2 && pos == 0 {
            break;
        }
    }

    let new_urls: Vec<String> = (0..5).map(|i| mk(500 + i)).collect();
    handle.tx.send(MpvCmd::Play(new_urls.into_iter().map(item).collect())).expect("send new queue");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            panic!("timed out waiting for the new queue: count={count} pos={pos}");
        };
        match ev {
            MpvEvent::Property { name, value } if name == "playlist-count" => {
                count = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
            }
            MpvEvent::Property { name, value } if name == "playlist-pos" => {
                pos = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
            }
            MpvEvent::EndFile { reason } => {
                assert_ne!(reason, "error", "new queue failed to play");
            }
            MpvEvent::Shutdown => panic!("mpv shut down"),
            _ => {}
        }
        if count == 5 && pos == 0 {
            break;
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    while let Ok(ev) = events.try_recv() {
        if let MpvEvent::Property { name, value } = ev {
            match name.as_str() {
                "playlist-count" => count = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1),
                "playlist-pos" => pos = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1),
                _ => {}
            }
        }
    }
    assert_eq!(count, 5, "old track survived the queue replacement");
    assert_eq!(pos, 0, "the new first track must be playing");

    handle.shutdown().await;
}

/// mpv logs the full `loadfile` URL, token included; the redacting FIFO must
/// keep the token off disk.
#[tokio::test]
async fn mpv_log_reaches_disk_redacted() {
    let _guard = MPV_TEST_LOCK.lock().await;

    // Unique per run, so a stale line cannot hide a leak.
    let token = format!("SECRETTOKEN{}", std::process::id());
    let log_path = shmc::config::log_dir().expect("log dir").join("mpv.log");

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    // Port 9 is closed, so mpv fails to open this and logs the failure.
    let url = format!("http://127.0.0.1:9/x?X-Plex-Token={token}");
    handle.tx.send(MpvCmd::Play(vec![item(url)])).expect("send play");

    // Wait for the redacted URL, so a missing log cannot pass.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(&log_path) {
            if text.contains("X-Plex-Token=<redacted>") {
                assert!(
                    !text.contains(&token),
                    "the token reached {}:\n{text}",
                    log_path.display()
                );
                break;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("mpv.log never received a redacted stream URL");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // The token appears on several lines, not just the first.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let text = std::fs::read_to_string(&log_path).expect("mpv.log exists");
    assert!(!text.contains(&token), "the token reached {}:\n{text}", log_path.display());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log_path).expect("mpv.log metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mpv.log must be private to the user");
    }

    handle.shutdown().await;
}

/// An mpv that exits before it listens fails the start at once, not after the
/// 5 s connect timeout, and leaves no log FIFO (its reader would wait forever).
#[tokio::test]
async fn failed_mpv_start_leaves_no_log_pipe() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "/usr/bin/false".into(), extra_args: vec![], volume: 50.0 };
    let started = std::time::Instant::now();
    let result = mpv::spawn(&cfg).await;
    let took = started.elapsed();
    assert!(result.is_err(), "mpv exited at once, so the start must fail");
    assert!(took < std::time::Duration::from_secs(3), "took {took:?}");

    let ours = format!("mpv-{}-", std::process::id());
    let left: Vec<_> = std::fs::read_dir(shmc::cache::ensure_cache_dir().expect("cache dir"))
        .expect("cache dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&ours) && n.ends_with(".pipe"))
        .collect();
    assert!(left.is_empty(), "log FIFO left behind: {left:?}");
}

/// The IPC socket lives in the private (0700) run dir, not the shared temp
/// dir, and does not outlive mpv.
#[tokio::test]
async fn mpv_socket_private_and_removed_on_exit() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let sock = handle.socket_path().to_path_buf();
    let run = shmc::cache::private_run_dir().expect("create the run dir");
    assert_eq!(sock.parent(), Some(run.as_path()), "the IPC socket escaped the run dir");
    assert!(sock.exists(), "mpv did not create {}", sock.display());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode =
            std::fs::metadata(&run).expect("run dir metadata").permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "the socket directory must be private");
        let sock_mode =
            std::fs::metadata(&sock).expect("socket metadata").permissions().mode() & 0o777;
        assert_eq!(sock_mode, 0o600, "the socket itself must be 0600");
    }

    handle.shutdown().await;
    assert!(!sock.exists(), "shutdown must remove the socket at {}", sock.display());
}

/// A handle dropped without `shutdown` leaves no socket either.
#[tokio::test]
async fn dropping_the_mpv_handle_removes_its_socket() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready
    let sock = handle.socket_path().to_path_buf();
    assert!(sock.exists());
    drop(handle);
    drop(events);
    assert!(!sock.exists(), "socket left behind at {}", sock.display());
}

/// A question asked after mpv is gone fails at once instead of waiting out the
/// ten-second reply timeout.
#[tokio::test]
async fn get_property_fails_after_mpv_exit() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready
    handle.shutdown().await;
    while let Some(ev) = events.recv().await {
        if matches!(ev, MpvEvent::Shutdown) {
            break;
        }
    }

    let started = std::time::Instant::now();
    assert!(handle.get_property("volume").await.is_err());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "waited {:?} for a dead mpv",
        started.elapsed()
    );
}

/// The burst is bigger than the socket buffers: a writer that does not keep
/// draining mpv's replies deadlocks and never reaches `playlist-play-index`.
#[tokio::test]
async fn long_queue_start_does_not_deadlock() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    const N: usize = 300;
    let urls: Vec<String> =
        (0..N).map(|i| format!("av://lavfi:sine=frequency={}:duration=1", 200 + i)).collect();
    handle.tx.send(MpvCmd::Play(urls.into_iter().map(item).collect())).expect("send play");

    let mut last_pos = -1i64;
    let mut last_count = -1i64;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let ev = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => panic!("event channel closed"),
            Err(_) => panic!(
                "timed out: mpv IPC stalled (playlist-pos={last_pos}, playlist-count={last_count})"
            ),
        };
        match ev {
            MpvEvent::Property { name, value } => match name.as_str() {
                "playlist-pos" => last_pos = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1),
                "playlist-count" => {
                    last_count = value.as_ref().and_then(|v| v.as_i64()).unwrap_or(-1);
                }
                _ => {}
            },
            MpvEvent::Shutdown => panic!("mpv shut down early"),
            _ => {}
        }
        if last_count == N as i64 && last_pos == 0 {
            break;
        }
    }
    handle.shutdown().await;
}

/// The user's mpv.conf and scripts must not apply: they could write the
/// token-bearing URL to disk. `MPV_HOME` is where mpv looks for both.
#[tokio::test]
async fn mpv_ignores_the_users_config_and_scripts() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let home = std::env::temp_dir().join(format!("shmc-mpv-home-{}", std::process::id()));
    std::fs::create_dir_all(home.join("scripts")).expect("create mpv home");
    std::fs::write(home.join("mpv.conf"), "mute=yes\n").expect("write mpv.conf");
    std::fs::write(
        home.join("scripts").join("speed.lua"),
        "mp.set_property_native('speed', 1.5)\n",
    )
    .expect("write script");

    // Every test that spawns mpv holds the lock, so nothing else reads the
    // environment meanwhile.
    std::env::set_var("MPV_HOME", &home);
    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let spawned = mpv::spawn(&cfg).await;
    std::env::remove_var("MPV_HOME");
    let (handle, mut events) = spawned.expect("spawn mpv");
    let _ = events.recv().await; // Ready

    let mute = handle.get_property("mute").await.expect("get mute");
    let speed = handle.get_property("speed").await.expect("get speed");
    handle.shutdown().await;
    let _ = std::fs::remove_dir_all(&home);

    assert_eq!(mute.as_bool(), Some(false), "mpv.conf was applied: mute={mute}");
    assert_eq!(speed.as_f64(), Some(1.0), "a script ran: speed={speed}");
}

#[tokio::test]
async fn media_title_is_the_queue_entry_title() {
    let _guard = MPV_TEST_LOCK.lock().await;

    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (handle, mut events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    match events.recv().await {
        Some(MpvEvent::Ready) => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    let title = "Queue title, with a comma";
    let url = "av://lavfi:sine=frequency=440:duration=60".to_string();
    handle.tx.send(MpvCmd::Play(vec![LoadItem { url, title: title.into() }])).expect("send play");

    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut last = String::new();
    loop {
        if let Ok(v) = handle.get_property("media-title").await {
            last = v.as_str().unwrap_or_default().to_string();
            if last == title {
                break;
            }
        }
        assert!(tokio::time::Instant::now() < until, "media-title stayed {last:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    handle.shutdown().await;
}
