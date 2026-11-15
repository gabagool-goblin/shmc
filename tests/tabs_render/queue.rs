use super::*;
use crossterm::event::KeyEventKind;
use shmc::ui::app::BrowserCol;
use shmc::ui::scroll::{compute_offset, ScrollId};

fn end_file_error() -> Msg {
    Msg::Mpv(MpvEvent::EndFile { reason: "error".into() })
}

fn end_file_eof() -> Msg {
    Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() })
}

/// `D` stops mpv as well as emptying the queue, and drops the cover.
#[tokio::test]
async fn clear_queue_stops_and_drops_the_cover() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.queue_state.select(Some(2));
    app.current = Some(2);
    app.stopped = false;
    app.art.img = Some(Arc::new(image::DynamicImage::new_rgba8(4, 4)));

    app.clear_queue();

    assert!(app.queue.is_empty(), "the queue must be empty");
    assert_eq!(app.current, None, "nothing is playing any more");
    assert!(app.stopped, "clearing the queue must stop playback");
    assert!(app.art.img.is_none(), "the cover must go with the track");
    assert_eq!(app.position, 0.0, "the progress must reset");
    assert_eq!(app.status, "queue cleared");
}

/// `D` and `:clear` ask first: only `y` (or a second `D`) clears; an empty queue asks nothing.
#[tokio::test]
async fn clearing_the_queue_asks_first() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(key('D'));
    assert_eq!(app.queue.len(), 6, "nothing is cleared before the answer");
    assert!(app.status.contains("6 tracks"), "the question says how many: {}", app.status);
    app.handle_msg(key('x'));
    assert_eq!(app.queue.len(), 6, "another key cancels");
    assert_eq!(app.current, Some(2));

    run_cmd(&mut app, "clear");
    assert_eq!(app.queue.len(), 6, ":clear asks too");
    app.handle_msg(key('y'));
    assert!(app.queue.is_empty());
    assert!(app.stopped);

    app.queue = (0..4).map(track).collect();
    app.handle_msg(key('D'));
    app.handle_msg(key('D'));
    assert!(app.queue.is_empty(), "a second D confirms");

    app.handle_msg(key('D'));
    app.handle_msg(key('2'));
    assert_eq!(app.tab, Tab::Browser, "an empty queue asks nothing");
}

/// Key repeats of `D` must not answer the question the first press asked.
#[tokio::test]
async fn held_d_asks_but_does_not_clear() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.handle_msg(key('D'));
    for _ in 0..5 {
        app.handle_msg(Msg::Key(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('D'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))));
    }
    assert_eq!(app.queue.len(), 5, "repeats do not confirm");
    assert!(app.status.contains("5 tracks"), "the question stays up: {}", app.status);
    app.handle_msg(key('D'));
    assert!(app.queue.is_empty(), "a fresh press does");
}

/// Under a filter a consume must keep the cursor on the same item.
#[tokio::test]
async fn consume_keeps_cursor_on_item_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..30).map(track).collect();
    app.current = Some(5); // finished track is outside the filtered view
    app.stopped = false;
    app.consume = true;
    filter_queue(&mut app, "Starfall! 2");
    // matches: 2, 12, 20..=29; cursor on the last, real index 29
    let cursor = app.queue_filter.cursor_of(29).expect("visible");
    app.queue_state.select(Some(cursor));

    app.handle_msg(end_file_eof());

    let sel = app.queue_state.selected().expect("a row is highlighted");
    let real = app.queue_filter.real(sel, app.queue.len());
    assert_eq!(app.queue[real].rating_key, "t29", "the cursor moved to another item");
}

/// Removing the playing track hands playback to the one that slid in (random
/// mode: any); removing the last track stops.
#[tokio::test]
async fn removing_the_playing_track_hands_playback_on() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.remove_from_queue(2);
    assert_eq!(app.queue.len(), 4);
    assert_eq!(app.current, Some(2), "the track that slid in should take over");
    assert!(!app.stopped, "playback must carry on");

    app.current = Some(3);
    app.remove_from_queue(3);
    assert_eq!(app.queue.len(), 3);
    assert_eq!(app.current, None);
    assert!(app.stopped, "removing the last track stops playback");

    app.queue = (0..5).map(track).collect();
    app.random = true;
    app.current = Some(1);
    app.stopped = false;
    app.remove_from_queue(1);
    assert_eq!(app.queue.len(), 4);
    assert!(!app.stopped, "random mode should hand playback on too");
    assert!(app.current.is_some(), "and it should be playing somewhere");
    app.random = false;
}

/// One unplayable track is skipped; only a burst of failures stops playback
/// and points at `>`.
#[tokio::test]
async fn burst_of_failures_stops_playback() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.queue_state.select(Some(0));
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "one bad file must not stop playback");
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");
    assert_eq!(app.queue.len(), 5, "the queue must survive the failure");
    assert!(app.status_error, "failures are shown as errors");
    assert!(
        app.status.contains("Starfall! 2 failed to play") && app.status.contains("skipping"),
        "a skipped track must be reported: {}",
        app.status
    );

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "a short burst is still not a dead queue");

    app.handle_msg(end_file_error());
    assert!(app.stopped, "a burst of failures must stop playback");
    assert_eq!(app.failed_at, Some(2), "the FIRST failure of the burst sticks");
    assert_eq!(app.queue.len(), 5, "the queue must survive the failure");
    assert!(
        app.status.contains("Starfall! 2 failed to play") && app.status.contains("press >"),
        "the message must name the track and say how to continue: {}",
        app.status
    );

    app.handle_msg(key('>'));
    assert_eq!(app.current, Some(3), "`>` must continue after the broken track");
    assert!(!app.stopped);
    assert_eq!(app.failed_at, None, "the marker is consumed");
    assert!(app.status.is_empty(), "the key press dismisses the error");
}

/// A track that loads ends the error run; a later failure is a skip again.
#[tokio::test]
async fn successful_load_resets_failure_streak() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(1);
    app.stopped = false;

    app.handle_msg(end_file_error());
    app.handle_msg(end_file_error());
    assert!(!app.stopped, "two failures in a row are still a skip");

    app.handle_msg(playlist_pos(2));
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "time-pos".into(),
        value: Some(serde_json::json!(0.5)),
    }));
    assert_eq!(app.failed_at, None, "a good load clears the marker");

    app.handle_msg(end_file_error());
    app.handle_msg(end_file_error());
    assert!(!app.stopped, "the streak must restart after a successful load (2, not 4)");
    app.handle_msg(end_file_error());
    assert!(app.stopped, "three *consecutive* failures stop");
}

/// The position mpv reports after each failure must not end the burst.
#[tokio::test]
async fn failure_run_with_position_events_stops() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    for next in 1..=2 {
        app.handle_msg(end_file_error());
        app.handle_msg(playlist_pos(next));
    }
    app.handle_msg(end_file_error());
    assert!(app.stopped, "three failures in a row must stop playback");
    assert_eq!(app.failed_at, Some(0), "the first failed slot is kept");
}

/// With repeat on a failing last track wraps; an all-bad queue still stops.
#[tokio::test]
async fn failing_last_track_wraps_with_repeat() {
    let mut app = new_app().await;
    app.queue = (0..4).map(track).collect();
    app.current = Some(3);
    app.stopped = false;
    app.loop_playlist = true;

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "mpv wraps to the first entry");
    app.handle_msg(playlist_pos(0));
    app.handle_msg(end_file_error());
    app.handle_msg(playlist_pos(1));
    app.handle_msg(end_file_error());
    assert!(app.stopped, "an all-bad queue is bounded by the streak");
}

/// No track follows the failure: the status says so and `>` reports the end.
#[tokio::test]
async fn last_track_failure_says_pick_another() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(end_file_error());

    assert!(app.stopped);
    assert!(
        app.status.contains("pick another song"),
        "there is no track after this one: {}",
        app.status
    );

    app.handle_msg(key('>'));
    assert!(app.stopped, "there is nothing to advance to");
    assert_eq!(app.status, "end of the queue");
}

/// A second press turns repeat and single off.
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

/// Consume drops the finished track without restarting the next; the last one stops.
#[tokio::test]
async fn consume_removes_finished_entry() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(end_file_eof());

    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.queue[0].title, "Starfall! 1");
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);

    drop(app); // one app at a time: it holds the suite's lock

    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(end_file_eof());

    assert!(app.queue.is_empty());
    assert_eq!(app.current, None);
    assert!(app.stopped);
}

#[tokio::test]
async fn track_info_updates_entry_and_cache() {
    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.current = Some(0);
    let audio = AudioInfo {
        codec: Some("flac".into()),
        sample_rate_hz: Some(44100),
        bit_depth: Some(16),
        ..Default::default()
    };

    app.handle_msg(Msg::TrackInfo { rating_key: "t0".into(), audio: audio.clone() });

    assert_eq!(app.audio, audio);
    assert_eq!(app.queue[0].audio.sample_rate_hz, Some(44100));
    assert!(app.audio_cache.contains_key("t0"));
}

#[tokio::test]
async fn play_next_inserts_after_the_current_track() {
    let mut app = new_app().await;
    app.queue = vec![track(0), track(1)];
    app.current = Some(0);
    app.browser.tracks = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks_state.select(Some(1));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;

    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t11", "t1"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn queue_reorder_moves_selected_and_playing_rows() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key('J'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t1", "t0", "t2"]
    );
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue_state.selected(), Some(1));

    app.handle_msg(key('K'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t2"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn shuffle_keeps_current_first_and_restores() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(2));
    let original: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();

    app.handle_msg(key('z'));
    assert!(app.random);
    assert_eq!(app.queue[0].rating_key, "t2");
    assert_eq!(app.current, Some(0));
    assert_eq!(app.queue.len(), 6);

    app.handle_msg(key('z'));
    assert!(!app.random);
    assert_eq!(app.queue.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(), original);
    assert_eq!(app.current, Some(2));
}

#[tokio::test]
async fn reshuffle_keeps_the_current_track_first() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(3);
    app.queue_state.select(Some(3));

    app.handle_msg(key('Z'));

    assert!(app.random);
    assert_eq!(app.queue[0].rating_key, "t3");
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn queue_follow_pins_playing_until_browse() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert!(visible > 0 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(playlist_pos(80));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(80));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the playing row must be the first visible queue row"
    );
    // a second paint must not drift back into the scrolloff margin
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "top alignment must persist across repaints"
    );

    // browsing pauses the follow
    app.handle_msg(key('j'));
    assert_eq!(app.queue_state.selected(), Some(81));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the first manual step must not scroll the pane"
    );
    app.handle_msg(playlist_pos(100));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        Some(81),
        "a track change must not yank a browsing cursor"
    );
    assert_ne!(app.scroll.offset(ScrollId::Queue), 100);

    // `o` re-arms it
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(100));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 100);

    app.handle_msg(playlist_pos(110));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(110));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 110);
}

/// The first step after `o` must move one row, not rewind the pane by
/// the scrolloff margin.
#[tokio::test]
async fn first_step_after_o_does_not_scroll() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(80);
    app.queue_state.select(Some(80));
    app.stopped = false;
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    let margin = app.scrolloff.min((visible.saturating_sub(1)) / 2);
    assert!(visible > margin + 1 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(80));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 80);

    // steps up to the bottom margin keep the pane where the pin left it
    let first_scroll = 80 + visible - margin;
    for row in 81..first_scroll {
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue_state.selected(), Some(row));
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            80,
            "stepping to row {row} must not scroll the pane"
        );
    }

    // the margin scrolls one row, then normal scrolling resumes
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(first_scroll));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 81);
}

/// A move arriving before the `o` pin is drawn must scroll normally, not pin
/// the new cursor to the top (with auto-follow on or off).
#[tokio::test]
async fn move_after_o_does_not_repin_row() {
    for follow in [true, false] {
        let mut app = new_app().await;
        app.tab = Tab::Queue;
        app.queue = (0..300).map(track).collect();
        app.current = Some(80);
        app.queue_state.select(Some(80));
        app.stopped = false;
        app.auto_follow = follow;
        let _ = render(&mut app, 120, 40);
        let visible = app.scroll.visible(ScrollId::Queue);
        let before = app.scroll.offset(ScrollId::Queue);

        app.handle_msg(key('o'));
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue_state.selected(), Some(81), "auto_follow {follow}");
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            compute_offset(81, before, visible, 300, app.scrolloff),
            "auto_follow {follow}: a move before the pin is drawn must not pin row 81 to the top"
        );
    }
}

/// The view following the end-of-list wrap is not the jump.
#[tokio::test]
async fn queue_end_wrap_jumps_to_top() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(299);
    app.queue_state.select(Some(299));
    app.stopped = false;
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        300 - visible,
        "the pin clamps flush to the bottom"
    );

    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn queue_follow_does_not_fight_the_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..120).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 9");
    assert!(app.queue_filter.cursor_of(0).is_none(), "test filter must hide the playing track");

    app.handle_msg(playlist_pos(90));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        Some(0),
        "auto-follow must leave a filtered queue alone"
    );

    // `o` still works within the filtered view
    filter_queue(&mut app, "Starfall! 90");
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn queue_gg_and_shift_g_reach_the_ends() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..10).map(track).collect();
    app.queue_state.select(Some(4));

    keys(&mut app, "G");
    assert_eq!(app.queue_state.selected(), Some(9));
    keys(&mut app, "gg");
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.tab, Tab::Queue, "g no longer opens the Browser");
}

#[tokio::test]
async fn undo_restores_the_last_removed_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(2));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t3"]
    );
    assert_eq!(app.current, Some(1));

    app.handle_msg(key('u'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t2", "t3"]
    );
    assert_eq!(app.current, Some(1), "non-playing removal leaves playback put");
    assert_eq!(app.status, "restored: Starfall! 2");

    app.handle_msg(key('u'));
    assert_eq!(app.status, "nothing to undo");
}

#[tokio::test]
async fn repeated_undo_restores_removals_newest_first() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..8).map(track).collect();
    app.queue_state.select(Some(5));
    app.stopped = true;
    let original = queue_keys(&app);

    app.handle_msg(key('d'));
    app.queue_state.select(Some(1));
    app.handle_msg(key('v'));
    app.handle_msg(key('j'));
    app.handle_msg(key('d'));
    assert_eq!(queue_keys(&app).len(), 5);

    app.handle_msg(key('u'));
    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), original);

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), original);
}

#[tokio::test]
async fn another_queue_change_drops_the_undo_history() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.queue_state.select(Some(0));
    app.stopped = true;

    app.handle_msg(key('d'));
    app.handle_msg(key('d'));
    app.add_tracks(vec![track(9)], Confirm::Track { title: "Starfall! 9".into() });
    let after = queue_keys(&app);
    assert_eq!(after, vec!["t2", "t3", "t9"]);

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), after);
}

#[tokio::test]
async fn undo_history_is_capped_at_twenty() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..21).map(track).collect();
    app.queue_state.select(Some(0));
    app.stopped = true;

    for _ in 0..21 {
        app.handle_msg(key('d'));
    }
    assert!(app.queue.is_empty());
    for _ in 0..21 {
        app.handle_msg(key('u'));
    }
    let keys = queue_keys(&app);
    assert_eq!(keys.len(), 20);
    assert_eq!(keys[0], "t1", "the oldest removal (t0) is gone");
}

#[tokio::test]
async fn undo_after_removing_playing_keeps_next_playing() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(1));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(app.current, Some(1), "the track that slid in starts");
    assert_eq!(app.queue[1].rating_key, "t2");

    app.handle_msg(key('u'));
    assert_eq!(app.queue[1].rating_key, "t1");
    assert_eq!(app.current, Some(2), "playback stays on t2 at its new index");
    assert_eq!(app.queue[2].rating_key, "t2");
}

#[tokio::test]
async fn auto_follow_can_be_disabled() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    app.handle_msg(key('j'));
    assert!(app.status.contains("queue follow: paused"));
    app.handle_msg(key('j'));
    assert!(app.status.is_empty(), "an already-paused follow must not spam the status line");

    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: on");

    // with the option off, `o` jumps once but does not stay armed
    app.current = Some(80);
    app.auto_follow = false;
    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: off");
    let _ = render(&mut app, 120, 40);
    let jump_offset = app.scroll.offset(ScrollId::Queue);
    assert_eq!(jump_offset, 80, "the one-shot jump still pins to the top");

    // a repaint and a step leave the pane where the jump put it
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "a repaint must not move the one-shot jump"
    );
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(81));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "browsing from the one-shot jump must not scroll the pane"
    );

    let selected = app.queue_state.selected();
    app.handle_msg(playlist_pos(120));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        selected,
        "disabled auto-follow must not move the cursor"
    );
}

/// After a consume mpv can report the pre-removal index of the next track; that
/// stale position must not move `current` past the queue.
#[tokio::test]
async fn stale_playlist_pos_keeps_current_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.current, Some(1), "the next track takes the freed slot");

    app.handle_msg(playlist_pos(2));
    assert_eq!(app.current, Some(1), "a stale position must not move the current track");

    app.handle_msg(playlist_pos(9));
    assert_eq!(app.current, Some(1));
}

/// The same stale position must not make "play next" splice past the queue end.
#[tokio::test]
async fn play_next_after_consume_does_not_panic() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    app.handle_msg(playlist_pos(2));

    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks_state.select(Some(0));

    app.handle_msg(key('i'));
    assert_eq!(app.queue.len(), 3, "the track was queued after the current one");
    assert_eq!(app.queue[2].rating_key, "t10");
}

/// The consume guard stays armed (mpv sends no position for the advance); the
/// position for the user's `>` must not be swallowed by it.
#[tokio::test]
async fn next_after_a_consume_follows_mpv() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..6).map(track).collect();
    app.current = Some(0);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    assert_eq!(app.queue.len(), 5);
    assert_eq!(app.current, Some(0));

    // consume off: `>` only moves, and must clear the old guard
    app.consume = false;
    app.handle_msg(key('>'));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1), "current must follow mpv after `>`");
}

/// With consume `>` removes the track you leave; the guard swallows the
/// pre-removal position mpv may report, but not the real one.
#[tokio::test]
async fn next_with_consume_removes_left_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t2", "t3"]);
    assert_eq!(app.current, Some(1), "current is the entry that was next");

    app.handle_msg(playlist_pos(2));
    assert_eq!(app.current, Some(1));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1));

    app.handle_msg(end_file_eof());
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t3"]);
}

/// `>` with nothing to advance to consumes nothing; with repeat on it consumes the last entry and wraps.
#[tokio::test]
async fn next_with_consume_on_the_last_entry() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(2);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    assert_eq!(app.queue.len(), 3, "repeat off: nothing to leave to");

    app.loop_playlist = true;
    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t1"]);
    assert_eq!(app.current, Some(0), "wrapped to the first entry");

    app.queue.truncate(1);
    app.current = Some(0);
    app.handle_msg(key('>'));
    assert_eq!(app.queue.len(), 1, "a lone track wraps to itself");
}

/// Plays the position events mpv sends during a reorder; the stale ones must
/// not move `current` or the cursor, or fetch art. Returns the final index.
fn reorder_storm(app: &mut App, plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> usize {
    let expected = app.current.expect("a reorder only guards a playing track");
    let playing_thumb = app.current_track().and_then(|t| t.thumb.clone());
    let playing_key = app.current_track().map(|t| t.rating_key.clone());
    for pos in (0..app.queue.len()).filter(|&p| p != expected) {
        app.handle_msg(playlist_pos(pos));
        assert_eq!(app.current, Some(expected), "stale position {pos} moved the current track");
        assert_eq!(
            app.queue_state.selected(),
            Some(expected),
            "stale position {pos} bounced the queue cursor"
        );
        // Only the playing track may be fetched
        for cmd in sent(plex) {
            match cmd {
                PlexCmd::LoadThumb { path } => assert_eq!(
                    Some(&path),
                    playing_thumb.as_ref(),
                    "stale position {pos} fetched art for a track that is not playing"
                ),
                PlexCmd::LoadTrackInfo(key) => assert_eq!(
                    Some(&key),
                    playing_key.as_ref(),
                    "stale position {pos} fetched metadata for a track that is not playing"
                ),
                other => panic!("stale position {pos} triggered {other:?}"),
            }
        }
    }
    app.handle_msg(playlist_pos(expected));
    assert_eq!(app.current, Some(expected));
    expected
}

/// `z` off reports ~n intermediate positions; they must not fetch art
/// for other tracks or bounce the cursor.
#[tokio::test]
async fn reorder_ignores_intermediate_positions() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.art_enabled = true;
    app.tab = Tab::Queue;
    app.queue = (0..60)
        .map(|i| Track { thumb: Some(format!("/library/metadata/t{i}/thumb")), ..track(i) })
        .collect();
    app.current = Some(20);
    app.queue_state.select(Some(20));
    app.stopped = false;

    // shuffle on pins the playing entry first
    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(0));
    assert_eq!(reorder_storm(&mut app, &mut plex), 0);

    // shuffle off drags it back to slot 20
    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(20), "the playing track keeps its original slot");
    assert_eq!(app.current_track().map(|t| t.rating_key.as_str()), Some("t20"));
    assert_eq!(reorder_storm(&mut app, &mut plex), 20);

    // the guard must not outlive its burst
    app.handle_msg(playlist_pos(20));
    app.handle_msg(playlist_pos(21));
    assert_eq!(app.current, Some(21), "the position guard must not wedge");
}

/// `z` off must not be quadratic: the wall-clock bound fails an O(n²) walk at
/// 50 000 tracks.
#[tokio::test]
async fn shuffle_off_restores_large_queue_quickly() {
    let mut app = new_app().await;
    app.queue = (0..50_000).map(track).collect();
    app.current = Some(31_337);
    app.queue_state.select(Some(31_337));
    app.stopped = false;
    let original: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();

    let started = std::time::Instant::now();
    app.handle_msg(key('z')); // shuffle on (snapshot + reorder)
    app.handle_msg(key('z')); // ... and back off (the restore path)
    let elapsed = started.elapsed();

    let restored: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();
    assert_eq!(restored, original, "z off restores the original order");
    assert_eq!(app.current, Some(31_337), "playback stays on the same track");
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "shuffle off took {elapsed:?} for 50 000 tracks"
    );
}

/// Applies `needle` as the queue filter, matching on the title.
fn filter_queue(app: &mut App, needle: &str) {
    let titles: Vec<String> = app.queue.iter().map(|t| t.title.clone()).collect();
    app.queue_filter.text = needle.into();
    app.queue_filter.apply(titles.len(), |i| titles[i].contains(needle));
}

/// A track with the given title; artist and album stay x-free so a one-letter
/// filter matches only the title (the real filter checks all three columns).
fn named(i: usize, title: &str) -> Track {
    Track { title: title.into(), artist: "Artist".into(), album: "Album".into(), ..track(i) }
}

/// `i` is not a Queue binding.
#[tokio::test]
async fn i_in_the_queue_does_nothing() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(2));
    app.stopped = false;

    let before: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();
    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(),
        before,
        "`i` must not insert anything in the Queue"
    );
    assert_eq!(app.queue_state.selected(), Some(2));
    assert_eq!(app.current, Some(1));
}

/// Under a filter `z`/`Z` must keep the highlight on the same track.
#[tokio::test]
async fn shuffle_keeps_highlighted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.current = Some(20);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    // real 12 ("Starfall! 12") is the 4th matching row
    let cursor = app.queue_filter.cursor_of(12).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    for label in ["shuffle on", "shuffle off"] {
        app.handle_msg(key('z'));
        let selected = app.queue_state.selected().expect("a row stays highlighted");
        let real = app.queue_filter.real(selected, app.queue.len());
        assert_eq!(
            app.queue[real].rating_key, "t12",
            "{label} moved the highlight off the selected track"
        );
    }
}

/// `>` while stopped must resume the highlighted row under a filter, not the row at that raw index.
#[tokio::test]
async fn stopped_next_resumes_highlighted_row_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.stopped = true;

    filter_queue(&mut app, "Starfall! 1");
    // real 12 ("Starfall! 12") is the 4th matching row
    let cursor = app.queue_filter.cursor_of(12).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    app.handle_msg(key('>'));
    assert_eq!(app.current, Some(12), "`>` must resume the track the cursor highlights");
    assert_eq!(app.queue[12].rating_key, "t12");
}

/// Under a filter Enter must leave the highlight on the row it started.
#[tokio::test]
async fn play_queue_index_highlights_filtered_row() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    // real 13 ("Starfall! 13") is the 5th matching row
    let cursor = app.queue_filter.cursor_of(13).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 13, "the test filter must make cursor and index differ");

    app.handle_msg(enter());

    assert_eq!(app.current, Some(13), "Enter must still start the highlighted track");
    assert_eq!(app.queue[13].rating_key, "t13");
    assert_eq!(
        app.queue_state.selected(),
        Some(cursor),
        "the highlight must stay on the row showing the started track"
    );
}

/// A reorder carries `failed_at` to the failed track's new slot; paths
/// without an exact mapping clear it.
#[tokio::test]
async fn reorder_carries_failure_marker_with_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..5).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(2));
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");

    app.handle_msg(key('K'));
    assert_eq!(app.failed_at, Some(1), "a single move remaps the marker");
    assert_eq!(app.queue[1].rating_key, "t2");

    app.handle_msg(key('z'));
    let marker = app.failed_at.expect("a permutation keeps the marker");
    assert_eq!(
        app.queue[marker].rating_key, "t2",
        "shuffle must leave the marker on the failed track"
    );

    app.failed_at = Some(1);
    app.remove_from_queue(3);
    assert_eq!(app.failed_at, None, "a removal clears the marker");
}

/// Under a filter `i` must highlight the inserted track.
#[tokio::test]
async fn play_next_highlights_inserted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.queue = ["Apple", "Box", "Cat", "Fox", "Goat"]
        .iter()
        .enumerate()
        .map(|(i, title)| named(i, title))
        .collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "x");

    app.browser.tracks = vec![named(90, "Xylophone")];
    app.browser.tracks_state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;

    app.handle_msg(key('i'));

    assert_eq!(app.queue[1].title, "Xylophone", "queued after the current track");
    let cursor = app.queue_state.selected().expect("a row is highlighted");
    assert_eq!(cursor, app.queue_filter.cursor_of(1).expect("the inserted track is visible"));
    assert_eq!(
        app.queue_filter.real(cursor, app.queue.len()),
        1,
        "the highlight must stay on the inserted track"
    );
}

/// Five queued tracks, `current` playing, with mpv commands sent to the returned receiver.
async fn playing_app(
    current: usize,
    position: f64,
) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<mpv::MpvCmd>) {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..5).map(track).collect();
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
    assert_eq!(app.queue.len(), 5);

    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// Early in a track, or with the threshold at 0, `<` goes back.
#[tokio::test]
async fn prev_goes_back_early_or_threshold_off() {
    let (mut app, mut rx) = playing_app(2, 1.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
    drop(app); // the next app needs the shared lock

    let (mut app, mut rx) = playing_app(2, 30.0).await;
    app.prev_restart_secs = 0;
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

/// `:stop` drops a replace still in flight, so it neither replaces the queue
/// nor starts playing when it lands.
#[tokio::test]
async fn stop_cancels_a_replace_still_in_flight() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 1);

    app.handle_msg(key(':'));
    for c in "stop".chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
    assert!(app.status.contains("cancelled"), "{}", app.status);

    app.handle_msg(tracks_reply(seqs[0], 10..12, QueueAction::Replace));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2"]);
    assert!(app.stopped, "a cancelled replace must not start playback");
}

#[tokio::test]
async fn visual_range_above_playing_removes_and_undoes() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..7).map(track).collect();
    app.current = Some(5);
    app.queue_state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(2));
    assert!(app.visual.is_none());

    // mpv walks the playing entry down through the slots in between
    app.handle_msg(playlist_pos(3));
    assert_eq!(app.current, Some(2), "a stale position moved current");

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2", "t3", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(5));
    assert_eq!(app.queue_state.selected(), Some(1));
}

#[tokio::test]
async fn visual_range_with_playing_hands_to_next() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5"]);
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue[1].rating_key, "t4");
    assert!(!app.stopped);
}

#[tokio::test]
async fn visual_range_under_filter_removes_visible_rows() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..30).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;
    // matches: t2, t20..=t29
    filter_queue(&mut app, "Starfall! 2");

    keys(&mut app, "vj");
    app.handle_msg(key('d'));
    let left = queue_keys(&app);
    assert_eq!(left.len(), 28);
    assert!(!left.contains(&"t2".to_string()) && !left.contains(&"t20".to_string()));
    assert!(left.contains(&"t3".to_string()) && left.contains(&"t21".to_string()));

    app.handle_msg(key('u'));
    let all: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
    assert_eq!(queue_keys(&app), all);
}

#[tokio::test]
async fn visual_mode_lifetime() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..6).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(3));
    app.stopped = false;

    app.handle_msg(key('v'));
    assert_eq!(app.visual.map(|v| v.anchor), Some(3));

    keys(&mut app, "JK");
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2", "t3", "t4", "t5"]);
    assert_eq!(app.visual.map(|v| v.anchor), Some(3), "J/K must not end visual mode");

    app.add_tracks(vec![track(9)], Confirm::Track { title: "Starfall! 9".into() });
    assert_eq!(app.visual.map(|v| v.anchor), Some(3), "an append keeps visual mode");

    // the finished entry above the anchor shifts it down by one
    app.consume = true;
    app.current = Some(1);
    app.handle_msg(end_file_eof());
    assert_eq!(app.visual.map(|v| v.anchor), Some(2));
    assert_eq!(app.queue[2].rating_key, "t3");

    app.handle_msg(key('2'));
    assert!(app.visual.is_none(), "leaving the tab ends it");

    app.handle_msg(key('1'));
    app.handle_msg(key('v'));
    app.replace_queue_and_play(vec![track(0)], Confirm::Track { title: "Starfall! 0".into() });
    assert!(app.visual.is_none(), "a replace ends it");

    app.handle_msg(key('v'));
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_mode_shows_label_and_count() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..8).map(track).collect();
    app.queue_state.select(Some(1));
    app.stopped = true;
    app.handle_msg(key('v'));
    keys(&mut app, "jj");

    for w in [120u16, 40, 20] {
        let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let text = flatten(&terminal);
        let bottom = text.lines().rev().find(|l| l.contains("VISUAL")).unwrap_or("");
        assert!(bottom.contains("-- VISUAL --"), "{w}: label clipped");
        assert_eq!(bottom.contains("3 selected"), w >= 40, "{w}: counter");
        if w == 120 {
            let (x, inside) = locate(&terminal, "Starfall! 2").expect("row");
            let (_, outside) = locate(&terminal, "Starfall! 5").expect("row");
            let buf = terminal.backend().buffer();
            assert_ne!(buf.cell((x, inside)).unwrap().bg, buf.cell((x, outside)).unwrap().bg);
        }
    }
}

/// Browser tracks t0..t9 titled "Starfall! n", the cursor on row 0.
fn on_browser_tracks(app: &mut App, n: usize) {
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks_for = Some("al1".into());
    app.browser.tracks = (0..n).map(track).collect();
    app.browser.tracks_state.select(Some(0));
}

fn filter_browser_tracks(app: &mut App, needle: &str) {
    let titles: Vec<String> = app.browser.tracks.iter().map(|t| t.title.clone()).collect();
    app.browser_filters[2].text = needle.into();
    app.browser_filters[2].apply(titles.len(), |i| titles[i].contains(needle));
}

#[tokio::test]
async fn browser_visual_range_adds_in_order() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 30);
    app.queue = vec![track(100)];
    // matches: t2, t20..=t29
    filter_browser_tracks(&mut app, "Starfall! 2");

    keys(&mut app, "vjj");
    assert_eq!(app.visual.map(|v| v.anchor), Some(2));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t100", "t2", "t20", "t21"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_range_inserts_after_the_current_track() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 6);
    app.queue = (10..13).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.browser.tracks_state.select(Some(1));

    keys(&mut app, "vjji");
    assert_eq!(queue_keys(&app), ["t10", "t1", "t2", "t3", "t11", "t12"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_range_enter_replaces_queue_with_range() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.queue = (10..13).map(track).collect();
    app.browser.tracks_state.select(Some(5));

    keys(&mut app, "vk");
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t4", "t5"]);
    assert_eq!(app.current, Some(0));
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_anchor_dropped_on_new_browser_tracks() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.handle_msg(key('v'));
    assert!(app.visual.is_some());

    app.browser.tracks_for = Some("al2".into());
    app.handle_msg(Msg::TracksLoaded {
        album_key: "al2".into(),
        tracks: (20..25).map(track).collect(),
    });
    assert!(app.visual.is_none());
    app.handle_msg(key('j'));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t21"], "only the row under the cursor is added");
}

#[tokio::test]
async fn visual_range_adds_playlist_tracks() {
    let mut app = new_app().await;
    on_playlist(&mut app, 0);
    app.playlists.items_for = Some("pl0".into());
    app.playlists.items = (0..6).map(track).collect();
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;

    keys(&mut app, "vj");
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.visual.is_none());
    assert_eq!(app.tab, Tab::Playlists, "Esc in visual mode does not leave the tab");

    app.handle_msg(key('v'));
    keys(&mut app, "ja");
    assert_eq!(queue_keys(&app), ["t2", "t3"]);
}

#[tokio::test]
async fn browser_visual_mode_shows_range_and_label() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.browser.tracks_state.select(Some(1));
    app.stopped = true;
    app.handle_msg(key('v'));
    keys(&mut app, "jj");

    for w in [120u16, 60] {
        let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let text = flatten(&terminal);
        assert!(text.contains("-- VISUAL --"), "{w}: label missing");
        let (x, inside) = locate(&terminal, "Starfall! 2").expect("row");
        let (_, outside) = locate(&terminal, "Starfall! 5").expect("row");
        let buf = terminal.backend().buffer();
        assert_ne!(buf.cell((x, inside)).unwrap().bg, buf.cell((x, outside)).unwrap().bg, "{w}");
    }
}

#[tokio::test]
async fn g_letter_in_visual_jumps_not_adds() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    keys(&mut app, "vjga");
    assert!(app.queue.is_empty());
    assert!(app.visual.is_some());
}

#[tokio::test]
async fn save_sends_whole_queue_even_when_filtered() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    let saves = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
        sent(rx)
            .into_iter()
            .filter_map(|cmd| match cmd {
                PlexCmd::SavePlaylist { name, keys } => Some((name, keys)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    run_cmd(&mut app, "save Road trip");
    assert!(saves(&mut plex_rx).is_empty(), "an empty queue sends nothing");

    app.queue = (0..4).map(track).collect();
    app.queue_filter.text = "Starfall! 2".into();
    app.queue_filter.apply(4, |i| i == 2);
    run_cmd(&mut app, "save Road trip");
    assert_eq!(
        saves(&mut plex_rx),
        [("Road trip".to_string(), vec!["t0".into(), "t1".into(), "t2".into(), "t3".into()])]
    );

    run_cmd(&mut app, "save Again");
    assert!(saves(&mut plex_rx).is_empty(), "one save at a time");
}
