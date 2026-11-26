use super::*;
use shmc::playback::MpvCmd;
use shmc::ui::app::saved_queue::{self, SavedQueue};

const SERVER: &str = "http://127.0.0.1:1";

fn keys(ks: &[usize]) -> Vec<String> {
    ks.iter().map(|i| format!("t{i}")).collect()
}

fn saved(tracks: &[usize], current: Option<usize>, position: f64) -> SavedQueue {
    SavedQueue {
        server: SERVER.into(),
        tracks: keys(tracks),
        current,
        position,
        ..SavedQueue::default()
    }
}

/// Asks for `saved` and answers with tracks `found`; returns what mpv was sent.
fn restore(
    app: &mut App,
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
    saved: SavedQueue,
    found: &[usize],
) -> Vec<MpvCmd> {
    let (mpv_tx, mut mpv_rx) = unbounded_channel();
    app.mpv.tx = mpv_tx;
    app.restore_queue(saved);
    let (seq, saved) = last_sent(plex, |cmd| match cmd {
        PlexCmd::RestoreQueue { seq, saved } => Some((seq, saved)),
        _ => None,
    })
    .expect("a restore request");
    let tracks = found.iter().map(|&i| track(i)).collect();
    app.handle_msg(reply(seq, QueueOutcome::Restored { tracks, saved }));
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    std::iter::from_fn(|| mpv_rx.try_recv().ok()).collect()
}

#[tokio::test]
async fn restored_queue_comes_back_paused_where_it_was() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut s = saved(&[1, 9, 2, 3, 2], Some(3), 42.0);
    s.loop_playlist = true;
    s.consume = true;
    let sent = restore(&mut app, &mut plex, s, &[3, 1, 2]);

    assert_eq!(queue_keys(&app), keys(&[1, 2, 3, 2]), "order and repeats kept, t9 dropped");
    assert_eq!(app.current, Some(2));
    assert!(app.paused && !app.stopped);
    assert!(app.consume);
    assert!(app.status.contains("1 no longer in the library"), "{}", app.status);
    let pause = sent.iter().position(|c| matches!(c, MpvCmd::SetPause { paused: true }));
    let play = sent.iter().position(|c| matches!(c, MpvCmd::PlayIndex(2)));
    assert!(pause.is_some() && pause < play, "paused before it starts: {sent:?}");
    assert!(sent.iter().any(|c| matches!(c, MpvCmd::Append(items) if items.len() == 4)));
    assert!(sent
        .iter()
        .any(|c| matches!(c, MpvCmd::SetLoop { property: "loop-playlist", on: true })));
    assert!(matches!(sent.last(), Some(MpvCmd::SeekAbs(p)) if *p == 42.0), "{sent:?}");
}

#[tokio::test]
async fn playing_after_a_paused_restore_unpauses() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let sent = restore(&mut app, &mut plex, saved(&[1, 2], Some(0), 30.0), &[1, 2]);
    assert!(!sent.iter().any(|c| matches!(c, MpvCmd::SetPause { paused: false })), "{sent:?}");

    let (mpv_tx, mut mpv) = unbounded_channel();
    app.mpv.tx = mpv_tx;
    app.replace_queue_and_play((5..8).map(track).collect(), Confirm::Tracks);
    app.play_queue_index(1);
    let sent: Vec<MpvCmd> = std::iter::from_fn(|| mpv.try_recv().ok()).collect();
    let unpaused = |c: &MpvCmd| matches!(c, MpvCmd::SetPause { paused: false });
    let play = sent.iter().position(|c| matches!(c, MpvCmd::Play(_)));
    let row = sent.iter().position(|c| matches!(c, MpvCmd::PlayIndex(1)));
    assert!(sent[..play.unwrap()].iter().any(unpaused), "an album starts unpaused: {sent:?}");
    assert!(sent[play.unwrap()..row.unwrap()].iter().any(unpaused), "so does a queue row");
    assert!(!app.paused);
}

#[tokio::test]
async fn missing_playing_track_hands_over_from_its_start() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let sent = restore(&mut app, &mut plex, saved(&[1, 9, 2], Some(1), 42.0), &[1, 2]);

    assert_eq!(app.current, Some(1), "the track after the missing one");
    assert_eq!(app.queue.items[1].rating_key, "t2");
    assert!(!sent.iter().any(|c| matches!(c, MpvCmd::SeekAbs(_))), "{sent:?}");
}

#[tokio::test]
async fn stopped_queue_comes_back_stopped() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let sent = restore(&mut app, &mut plex, saved(&[1, 2], None, 0.0), &[1, 2]);

    assert!(app.stopped);
    assert!(!sent.iter().any(|c| matches!(c, MpvCmd::PlayIndex(_))), "{sent:?}");
}

#[tokio::test]
async fn shuffled_queue_restores_its_original_order() {
    let in_order: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
    let s = {
        let mut app = new_app().await;
        app.replace_queue_and_play((0..20).map(track).collect(), Confirm::Tracks);
        app.handle_msg(key('z'));
        assert_ne!(queue_keys(&app), in_order, "shuffled");
        app.saved_queue()
    };
    let (mut app, mut plex) = new_app_with_plex().await;
    let shuffled = s.tracks.clone();
    let found: Vec<usize> = (0..20).rev().collect();
    restore(&mut app, &mut plex, s, &found);
    assert_eq!(queue_keys(&app), shuffled);

    app.handle_msg(key('z'));
    assert_eq!(queue_keys(&app), in_order);
}

#[tokio::test]
async fn queue_from_another_server_is_not_restored() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut s = saved(&[1, 2], Some(0), 0.0);
    s.server = "http://10.0.0.2:32400".into();
    app.restore_queue(s);
    assert!(last_sent(&mut plex, |cmd| matches!(cmd, PlexCmd::RestoreQueue { .. }).then_some(()))
        .is_none());
}

#[tokio::test]
async fn quitting_before_the_restore_lands_keeps_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let s = saved(&[1, 2], Some(0), 10.0);
    saved_queue::write(&path, &s).unwrap();
    let before = std::fs::read(&path).unwrap();
    let (mut app, mut plex) = new_app_with_plex().await;
    app.restore_queue(s);
    let seq = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::RestoreQueue { seq, .. } => Some(seq),
        _ => None,
    })
    .unwrap();
    app.handle_msg(reply(seq, QueueOutcome::Failed("server unreachable".into())));

    app.save_queue(&path);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn an_emptied_queue_stays_empty_after_a_superseded_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let s = saved(&[1, 2], Some(0), 10.0);
    saved_queue::write(&path, &s).unwrap();
    let (mut app, _plex) = new_app_with_plex().await;
    app.restore_queue(s);
    app.replace_queue_and_play((5..8).map(track).collect(), Confirm::Tracks);
    app.queue.items.clear();

    app.save_queue(&path);
    assert!(!path.exists());
}

#[tokio::test]
async fn saving_writes_the_queue_and_an_empty_one_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state").join("queue.json");
    let mut app = new_app().await;
    app.replace_queue_and_play((0..3).map(track).collect(), Confirm::Tracks);

    app.save_queue(&path);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("TOK") && !text.contains("/library/parts"), "{text}");
    let saved: SavedQueue = serde_json::from_str(&text).unwrap();
    assert_eq!(saved.tracks, keys(&[0, 1, 2]));
    assert_eq!(saved.current, Some(0));

    app.clear_queue();
    app.save_queue(&path);
    assert!(!path.exists());
}

fn written(path: &std::path::Path) -> SavedQueue {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[tokio::test]
async fn a_failed_save_is_tried_again_at_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("state");
    std::fs::write(&blocked, "").unwrap();
    let path = blocked.join("queue.json");
    let mut app = new_app().await;
    app.replace_queue_and_play((0..3).map(track).collect(), Confirm::Tracks);
    let start = std::time::Instant::now();
    let at = |s| start + std::time::Duration::from_secs(s);

    app.autosave_queue(&path, at(0));
    app.autosave_queue(&path, at(30));
    std::fs::remove_file(&blocked).unwrap();
    app.autosave_queue(&path, at(60));
    assert_eq!(written(&path).tracks, keys(&[0, 1, 2]));
}

#[tokio::test]
async fn a_playing_queue_is_saved_every_30_seconds_only_when_it_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    let mut app = new_app().await;
    app.replace_queue_and_play((0..3).map(track).collect(), Confirm::Tracks);
    let start = std::time::Instant::now();
    let at = |s| start + std::time::Duration::from_secs(s);

    app.autosave_queue(&path, at(0));
    app.autosave_queue(&path, at(29));
    assert!(!path.exists(), "not before 30 s");
    app.autosave_queue(&path, at(30));
    let saved = written(&path);
    assert_eq!((saved.tracks, saved.current), (keys(&[0, 1, 2]), Some(0)));

    std::fs::remove_file(&path).unwrap();
    app.autosave_queue(&path, at(60));
    assert!(!path.exists(), "nothing changed");
    app.position = 42.0;
    app.autosave_queue(&path, at(75));
    assert!(!path.exists(), "the next save is due at 90 s");
    app.autosave_queue(&path, at(90));
    assert_eq!(written(&path).position, 42.0);
}

#[test]
fn turning_restore_queue_off_removes_the_saved_queue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.json");
    saved_queue::write(&path, &saved(&[1], Some(0), 1.0)).unwrap();
    assert_eq!(saved_queue::queue_file_after(true, Some(path.clone())), Some(path.clone()));
    assert!(path.exists());
    assert_eq!(saved_queue::queue_file_after(false, Some(path.clone())), None);
    assert!(!path.exists());
}
