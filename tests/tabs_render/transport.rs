use super::*;

#[tokio::test]
async fn loop_keys_toggle_off_again() {
    let mut app = new_app().await;
    app.loop_playlist = false;
    app.handle_msg(key('x'));
    assert!(app.loop_playlist);
    app.handle_msg(key('x'));
    assert!(!app.loop_playlist);

    app.loop_file = false;
    app.handle_msg(key('c'));
    assert!(app.loop_file);
    app.handle_msg(key('c'));
    assert!(!app.loop_file);
}

/// Five queued tracks, `current` playing, with mpv commands sent to the returned receiver.
async fn playing_app(
    current: usize,
    position: f64,
) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<mpv::MpvCmd>) {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..5).map(track).collect();
    app.current = Some(current);
    app.stopped = false;
    app.position = position;
    let (tx, rx) = unbounded_channel();
    app.mpv.tx = tx;
    (app, rx)
}

/// Past the threshold `<` restarts the track; a second `<` goes back.
#[tokio::test]
async fn prev_restarts_track_past_threshold() {
    let (mut app, mut rx) = playing_app(2, 10.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SeekAbs(t)) if t == 0.0));
    assert_eq!(app.position, 0.0);
    assert_eq!(app.current, Some(2));
    assert_eq!(app.queue.items.len(), 5);

    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// The Prev media key restarts a track past the threshold, like `<`.
#[tokio::test]
async fn media_prev_restarts_the_track() {
    let (mut app, mut rx) = playing_app(2, 10.0).await;
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Prev)));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SeekAbs(t)) if t == 0.0));
    assert_eq!(app.current, Some(2));
}

/// The Next media key removes the skipped track under consume, like `>`.
#[tokio::test]
async fn media_next_with_consume_removes_the_skipped_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..4).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Next)));
    let keys: Vec<_> = app.queue.items.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t2", "t3"]);
    assert_eq!(app.current, Some(1), "current is the entry that was next");
}

/// The Stop media key stops playback and keeps the queue; mpv keeps running.
#[tokio::test]
async fn media_stop_stops_without_quitting() {
    let (mut app, mut rx) = playing_app(2, 10.0).await;
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Stop)));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Stop)));
    assert!(app.stopped);
    assert_eq!(app.queue.items.len(), 5);
    assert!(!app.should_quit);
}

/// Play and Pause set the state, so the play-only key never pauses; stopped, they do nothing.
#[tokio::test]
async fn media_play_and_pause_set_the_state() {
    let (mut app, mut rx) = playing_app(2, 10.0).await;
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Play)));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SetPause { paused: false })));
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Pause)));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SetPause { paused: true })));

    app.stopped = true;
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Play)));
    app.handle_msg(Msg::Mpv(mpv::MpvEvent::MediaKey(mpv::MediaKey::Pause)));
    assert!(rx.try_recv().is_err());
}

/// Early in a track, or with the threshold at 0, `<` goes back.
#[tokio::test]
async fn prev_goes_back_early_or_threshold_off() {
    let (mut app, mut rx) = playing_app(2, 1.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
    drop(app); // the next app needs the shared lock

    let (mut app, mut rx) = playing_app(2, 30.0).await;
    app.ui.prev_restart_secs = 0;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// On the first entry `<` restarts (mpv would stop), or wraps with repeat on.
#[tokio::test]
async fn prev_on_first_entry_restarts_unless_repeat() {
    let (mut app, mut rx) = playing_app(0, 1.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SeekAbs(t)) if t == 0.0));
    assert!(!app.stopped);
    assert_eq!(app.current, Some(0));
    drop(app);

    let (mut app, mut rx) = playing_app(0, 1.0).await;
    app.loop_playlist = true;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}
