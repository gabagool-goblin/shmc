use super::*;
use shmc::playback::MpvCmd;
use shmc::ui::app::BrowserCol;

/// With the same song twice in a playlist, the request names the entry under the cursor.
#[tokio::test]
async fn enter_on_repeated_song_names_entry() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    let mut first = track(7);
    first.playlist_item_id = Some("101".into());
    let mut second = track(7); // the same song again, a different entry
    second.playlist_item_id = Some("202".into());
    app.playlists.items = vec![first, second];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.handle_msg(enter());

    let start = last_sent(&mut plex, |cmd| match cmd {
        PlexCmd::ResolvePlaylist { start, .. } => Some(start),
        _ => None,
    })
    .flatten();
    assert_eq!(start.as_deref(), Some("202"));
}

#[tokio::test]
async fn later_replace_wins_when_replies_reversed() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    on_playlist(&mut app, 1);
    app.handle_msg(enter());
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 2);

    app.handle_msg(tracks_reply(seqs[1], 10..12, QueueAction::Replace));
    app.handle_msg(tracks_reply(seqs[0], 20..25, QueueAction::Replace));

    assert_eq!(queue_keys(&app), ["t10", "t11"], "the slow, older replace must be dropped");
}

#[tokio::test]
async fn appends_apply_in_request_order() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 1);
    app.handle_msg(key('a'));
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 2);

    app.handle_msg(tracks_reply(seqs[1], 10..12, QueueAction::Append));
    assert!(app.queue.items.is_empty(), "the second append waits for the first");
    app.handle_msg(tracks_reply(seqs[0], 20..22, QueueAction::Append));

    assert_eq!(queue_keys(&app), ["t20", "t21", "t10", "t11"]);
}

#[tokio::test]
async fn failed_request_does_not_block_next_append() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 1);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 2);
    app.handle_msg(key('a'));
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 3);

    app.handle_msg(tracks_reply(seqs[2], 30..31, QueueAction::Append));
    app.handle_msg(reply(seqs[1], QueueOutcome::Failed("resolving playlist: boom".into())));
    assert!(app.queue.items.is_empty());
    app.handle_msg(tracks_reply(seqs[0], 10..11, QueueAction::Append));

    assert_eq!(queue_keys(&app), ["t10", "t30"]);
}

#[tokio::test]
async fn clear_drops_replace_still_loading() {
    for by_key in [false, true] {
        let (mut app, mut plex) = new_app_with_plex().await;
        on_playlist(&mut app, 0);
        app.handle_msg(enter());
        let seq = resolve_seqs(&mut plex)[0];

        if by_key {
            app.tab = Tab::Queue;
            app.handle_msg(key('D'));
        } else {
            run_cmd(&mut app, "clear");
        }
        app.handle_msg(tracks_reply(seq, 10..15, QueueAction::Replace));

        let how = if by_key { "D" } else { ":clear" };
        assert!(app.queue.items.is_empty(), "{how}: the late reply refilled the cleared queue");
        assert_eq!(app.current, None, "{how}");
        assert!(app.stopped, "{how}: nothing may start playing");
    }
}

#[tokio::test]
async fn local_replace_drops_loading_playlist() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    let seq = resolve_seqs(&mut plex)[0];

    app.tab = Tab::Browser;
    app.browser.tracks.items = (0..4).map(track).collect();
    app.browser.tracks.state.select(Some(1));
    app.browser.focus = BrowserCol::Tracks;
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"]);

    app.handle_msg(tracks_reply(seq, 20..30, QueueAction::Replace));
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"], "the late playlist must be dropped");
}

#[tokio::test]
async fn local_append_lands_after_slower_remote_append() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    let seq = resolve_seqs(&mut plex)[0];

    app.tab = Tab::Browser;
    app.browser.tracks.items = (0..4).map(track).collect();
    app.browser.tracks.state.select(Some(2));
    app.browser.focus = BrowserCol::Tracks;
    app.handle_msg(key('a'));
    assert!(app.queue.items.is_empty(), "the local append waits for the playlist asked first");
    assert!(app.status.contains("queued after"), "{:?}", app.status);

    app.handle_msg(tracks_reply(seq, 20..22, QueueAction::Append));
    assert_eq!(queue_keys(&app), ["t20", "t21", "t2"]);
}

// a big playlist streams in after its first page

/// Enter on the first playlist as a streaming replace: its seq and the worker's cancel flag.
fn start_load(
    app: &mut App,
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
) -> (u64, Arc<std::sync::atomic::AtomicBool>) {
    on_playlist(app, 0);
    app.handle_msg(enter());
    let mut found: Vec<_> = sent(plex)
        .into_iter()
        .filter_map(|cmd| match cmd {
            PlexCmd::ResolvePlaylist { seq, stream, cancel, .. } => Some((seq, stream, cancel)),
            _ => None,
        })
        .collect();
    assert!(found.iter().all(|(_, stream, _)| *stream), "a replace streams");
    let (seq, _, cancel) = found.pop().expect("a ResolvePlaylist was sent");
    (seq, cancel)
}

/// The first page landing: 200 tracks of a 5,000-track playlist.
fn land_head(app: &mut App, seq: u64) {
    app.handle_msg(reply(
        seq,
        QueueOutcome::Streaming {
            tracks: (0..200).map(track).collect(),
            confirm: Confirm::Playlist { name: "x".into() },
            load_id: seq,
            total: 5000,
        },
    ));
}

/// Whether the playlist request the UI sent streams (`None`: it sent none).
fn resolve_streams(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Option<bool> {
    let mut out = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { stream, .. } = cmd {
            out = Some(stream);
        }
    }
    out
}

fn page(load_id: u64, range: std::ops::Range<usize>, last: bool) -> Msg {
    Msg::PlaylistPage { load_id, tracks: range.map(track).collect(), last }
}

/// What mpv's playlist holds, in order, as the stream URLs the app queued.
async fn mpv_matches_queue(app: &App) -> bool {
    let list = app.mpv.get_property("playlist").await.expect("mpv playlist");
    let in_mpv: Vec<String> = list
        .as_array()
        .expect("a playlist array")
        .iter()
        .map(|e| e["filename"].as_str().unwrap_or_default().to_string())
        .collect();
    let queued: Vec<String> = app
        .queue
        .items
        .iter()
        .map(|t| t.stream_url("http://127.0.0.1:1", "TOK").unwrap())
        .collect();
    in_mpv == queued
}

/// mpv's reader stops when nobody receives its events, so the test app holds them.
#[tokio::test]
async fn mpv_answers_after_sending_an_event() {
    let app = new_app().await;
    let _ = app.mpv.tx.send(MpvCmd::SetPause { paused: true });
    // mpv sends property changes on its next loop, after answering requests
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(app.mpv.get_property("pause").await.expect("mpv answers"), true);
}

#[tokio::test]
async fn streamed_playlist_plays_at_once_and_grows() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    assert_eq!(app.queue.items.len(), 200);
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped, "playback starts with the first page");

    app.handle_msg(page(seq, 200..230, false));
    assert!(app.status.contains("230 / 5 000"), "{}", app.status);
    app.handle_msg(page(seq, 230..260, true));

    assert_eq!(queue_keys(&app), (0..260).map(|i| format!("t{i}")).collect::<Vec<_>>());
    assert!(mpv_matches_queue(&app).await, "mpv's playlist must match the queue");
    assert!(app.status.is_empty(), "the last page clears the progress: {}", app.status);
    app.handle_msg(key('a'));
    assert_eq!(resolve_streams(&mut plex), Some(false), "an append does not stream");
}

#[tokio::test]
async fn superseded_load_ignores_late_pages() {
    for how in ["replace", "D", ":clear"] {
        let (mut app, mut plex) = new_app_with_plex().await;
        let (seq, cancel) = start_load(&mut app, &mut plex);
        land_head(&mut app, seq);
        match how {
            "replace" => {
                on_playlist(&mut app, 1);
                app.handle_msg(enter());
            }
            "D" => {
                app.tab = Tab::Queue;
                app.handle_msg(key('D'));
                app.handle_msg(key('D'));
            }
            _ => {
                run_cmd(&mut app, "clear");
                app.handle_msg(key('y'));
            }
        }
        assert!(
            cancel.load(std::sync::atomic::Ordering::Relaxed),
            "{how}: the worker must be told"
        );

        let before = queue_keys(&app);
        app.handle_msg(page(seq, 200..230, false));
        assert_eq!(queue_keys(&app), before, "{how}: a late page changed the queue");
    }
}

#[tokio::test]
async fn loading_playlist_refuses_add_shuffle_move() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.handle_msg(page(seq, 200..230, false));

    // `d` is allowed, its undo is not
    app.tab = Tab::Queue;
    app.handle_msg(key('d'));
    let before = queue_keys(&app);
    assert_eq!(before.len(), 229);

    for k in ['u', 'J', 'K', 'z', 'Z'] {
        app.handle_msg(key(k));
        assert_eq!(queue_keys(&app), before, "{k}");
        assert!(app.status.contains("still loading (230 / 5 000)"), "{k}: {}", app.status);
        assert!(!app.random, "{k}");
    }
    run_cmd(&mut app, "shuffle");
    assert!(!app.random);
    run_cmd(&mut app, "reshuffle");
    on_playlist(&mut app, 1);
    for k in ['a', 'i'] {
        app.handle_msg(key(k));
        assert!(app.status.contains("still loading"), "{k}: {}", app.status);
    }
    assert_eq!(queue_keys(&app), before);
    assert!(resolve_seqs(&mut plex).is_empty(), "no request may be sent");
    assert!(mpv_matches_queue(&app).await);

    app.handle_msg(enter());
    assert_eq!(resolve_seqs(&mut plex).len(), 1);
}

/// A playlist that fits in the first page must not stay marked as loading.
#[tokio::test]
async fn small_playlist_leaves_nothing_loading() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    app.handle_msg(reply(
        seq,
        QueueOutcome::Tracks {
            tracks: (0..60).map(track).collect(),
            confirm: Confirm::Playlist { name: "x".into() },
            action: QueueAction::Replace,
        },
    ));
    assert_eq!(app.queue.items.len(), 60);

    app.tab = Tab::Queue;
    app.handle_msg(key('z'));
    assert!(app.random, "shuffle works: {}", app.status);
    app.handle_msg(key('J'));
    assert!(!app.status.contains("still loading"), "{}", app.status);
}

#[tokio::test]
async fn remove_and_play_while_loading_keep_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.tab = Tab::Queue;
    app.handle_msg(key('d')); // remove t0, the playing one
    app.play_queue_index(5);
    app.handle_msg(page(seq, 200..250, false));
    app.handle_msg(page(seq, 250..260, true));

    assert_eq!(app.queue.items.len(), 259);
    assert_eq!(app.queue.items.last().unwrap().rating_key, "t259");
    assert!(mpv_matches_queue(&app).await);
    assert!(app.current.is_some_and(|c| c < app.queue.items.len()));
}

#[tokio::test]
async fn page_failure_keeps_loaded_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.handle_msg(page(seq, 200..230, false));
    app.handle_msg(Msg::PlaylistLoadStopped { load_id: seq, error: "boom".into() });

    assert_eq!(app.queue.items.len(), 230);
    assert!(app.status.contains("stopped at 230 / 5 000: boom"), "{}", app.status);
    assert!(app.status_error);
    app.tab = Tab::Playlists;
    app.handle_msg(key('a'));
    assert!(resolve_streams(&mut plex).is_some(), "{}", app.status);
}

#[tokio::test]
async fn failed_first_page_does_not_lock_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    app.handle_msg(Msg::PlaylistLoadStopped { load_id: seq, error: "stopped early".into() });
    app.handle_msg(reply(seq, QueueOutcome::Failed("resolving playlist: boom".into())));
    assert!(app.queue.items.is_empty());

    app.handle_msg(key('a'));
    assert!(resolve_streams(&mut plex).is_some(), "{}", app.status);
}

#[tokio::test]
async fn shuffled_replace_does_not_stream() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.handle_msg(key('z'));
    assert!(app.random);
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    assert_eq!(resolve_streams(&mut plex), Some(false), "shuffle needs the whole list");
}
