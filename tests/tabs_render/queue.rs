use super::*;
use crossterm::event::KeyEventKind;
use shmc::ui::app::BrowserCol;

#[tokio::test]
async fn clear_queue_stops_and_drops_the_cover() {
    let mut app = new_app().await;
    app.queue.items = (0..6).map(track).collect();
    app.queue.state.select(Some(2));
    app.current = Some(2);
    app.stopped = false;
    app.art.img = Some(Arc::new(image::DynamicImage::new_rgba8(4, 4)));

    app.clear_queue();

    assert!(app.queue.items.is_empty(), "the queue must be empty");
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
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(key('D'));
    assert_eq!(app.queue.items.len(), 6, "nothing is cleared before the answer");
    assert!(app.status.contains("6 tracks"), "the question says how many: {}", app.status);
    app.handle_msg(key('x'));
    assert_eq!(app.queue.items.len(), 6, "another key cancels");
    assert_eq!(app.current, Some(2));

    run_cmd(&mut app, "clear");
    assert_eq!(app.queue.items.len(), 6, ":clear asks too");
    app.handle_msg(key('y'));
    assert!(app.queue.items.is_empty());
    assert!(app.stopped);

    app.queue.items = (0..4).map(track).collect();
    app.handle_msg(key('D'));
    app.handle_msg(key('D'));
    assert!(app.queue.items.is_empty(), "a second D confirms");

    app.handle_msg(key('D'));
    app.handle_msg(key('3'));
    assert_eq!(app.tab, Tab::Browser, "an empty queue asks nothing");
}

/// Key repeats of `D` must not answer the question the first press asked.
#[tokio::test]
async fn held_d_asks_but_does_not_clear() {
    let mut app = new_app().await;
    app.queue.items = (0..5).map(track).collect();
    app.handle_msg(key('D'));
    for _ in 0..5 {
        app.handle_msg(Msg::Key(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('D'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))));
    }
    assert_eq!(app.queue.items.len(), 5, "repeats do not confirm");
    assert!(app.status.contains("5 tracks"), "the question stays up: {}", app.status);
    app.handle_msg(key('D'));
    assert!(app.queue.items.is_empty(), "a fresh press does");
}

/// Removing the playing track hands playback to the next (any, in random mode); the last stops.
#[tokio::test]
async fn removing_the_playing_track_hands_playback_on() {
    let mut app = new_app().await;
    app.queue.items = (0..5).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.remove_from_queue(2);
    assert_eq!(app.queue.items.len(), 4);
    assert_eq!(app.current, Some(2), "the track that slid in should take over");
    assert!(!app.stopped, "playback must carry on");

    app.current = Some(3);
    app.remove_from_queue(3);
    assert_eq!(app.queue.items.len(), 3);
    assert_eq!(app.current, None);
    assert!(app.stopped, "removing the last track stops playback");

    app.queue.items = (0..5).map(track).collect();
    app.random = true;
    app.current = Some(1);
    app.stopped = false;
    app.remove_from_queue(1);
    assert_eq!(app.queue.items.len(), 4);
    assert!(!app.stopped, "random mode should hand playback on too");
    assert!(app.current.is_some(), "and it should be playing somewhere");
    app.random = false;
}

#[tokio::test]
async fn track_info_updates_entry_and_cache() {
    let mut app = new_app().await;
    app.queue.items = vec![track(0)];
    app.current = Some(0);
    let audio = AudioInfo {
        codec: Some("flac".into()),
        sample_rate_hz: Some(44100),
        bit_depth: Some(16),
        ..Default::default()
    };

    app.handle_msg(Msg::TrackInfo {
        rating_key: "t0".into(),
        audio: audio.clone(),
        user_rating: None,
    });

    assert_eq!(app.audio, audio);
    assert_eq!(app.queue.items[0].audio.sample_rate_hz, Some(44100));
    assert!(app.audio_cache.contains_key("t0"));
}

#[tokio::test]
async fn play_next_inserts_after_the_current_track() {
    let mut app = new_app().await;
    app.queue.items = vec![track(0), track(1)];
    app.current = Some(0);
    app.browser.tracks.items = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks.state.select(Some(1));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;

    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t11", "t1"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn queue_reorder_moves_selected_and_playing_rows() {
    let mut app = new_app().await;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key('J'));

    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t1", "t0", "t2"]
    );
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue.state.selected(), Some(1));

    app.handle_msg(key('K'));
    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t2"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn undo_restores_the_last_removed_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue.state.select(Some(2));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t3"]
    );
    assert_eq!(app.current, Some(1));

    app.handle_msg(key('u'));
    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
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
    app.queue.items = (0..8).map(track).collect();
    app.queue.state.select(Some(5));
    app.stopped = true;
    let original = queue_keys(&app);

    app.handle_msg(key('d'));
    app.queue.state.select(Some(1));
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
    app.queue.items = (0..4).map(track).collect();
    app.queue.state.select(Some(0));
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
    app.queue.items = (0..21).map(track).collect();
    app.queue.state.select(Some(0));
    app.stopped = true;

    for _ in 0..21 {
        app.handle_msg(key('d'));
    }
    assert!(app.queue.items.is_empty());
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
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(1);
    app.queue.state.select(Some(1));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(app.current, Some(1), "the track that slid in starts");
    assert_eq!(app.queue.items[1].rating_key, "t2");

    app.handle_msg(key('u'));
    assert_eq!(app.queue.items[1].rating_key, "t1");
    assert_eq!(app.current, Some(2), "playback stays on t2 at its new index");
    assert_eq!(app.queue.items[2].rating_key, "t2");
}

/// Artist and album stay x-free, so a one-letter filter matches only the title.
fn named(i: usize, title: &str) -> Track {
    Track { title: title.into(), artist: "Artist".into(), album: "Album".into(), ..track(i) }
}

#[tokio::test]
async fn i_in_the_queue_does_nothing() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue.state.select(Some(2));
    app.stopped = false;

    let before: Vec<String> = app.queue.items.iter().map(|t| t.rating_key.clone()).collect();
    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.items.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(),
        before,
        "`i` must not insert anything in the Queue"
    );
    assert_eq!(app.queue.state.selected(), Some(2));
    assert_eq!(app.current, Some(1));
}

/// `>` while stopped resumes the highlighted row under a filter, not that raw index.
#[tokio::test]
async fn stopped_next_resumes_highlighted_row_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..50).map(track).collect();
    app.stopped = true;

    filter_queue(&mut app, "Starfall! 1");
    let cursor = app.queue.filter.cursor_of(12).expect("row is visible");
    app.queue.state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    app.handle_msg(key('>'));
    assert_eq!(app.current, Some(12), "`>` must resume the track the cursor highlights");
    assert_eq!(app.queue.items[12].rating_key, "t12");
}

/// Under a filter Enter must leave the highlight on the row it started.
#[tokio::test]
async fn play_queue_index_highlights_filtered_row() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..50).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    let cursor = app.queue.filter.cursor_of(13).expect("row is visible");
    app.queue.state.select(Some(cursor));
    assert_ne!(cursor, 13, "the test filter must make cursor and index differ");

    app.handle_msg(enter());

    assert_eq!(app.current, Some(13), "Enter must still start the highlighted track");
    assert_eq!(app.queue.items[13].rating_key, "t13");
    assert_eq!(
        app.queue.state.selected(),
        Some(cursor),
        "the highlight must stay on the row showing the started track"
    );
}

#[tokio::test]
async fn play_next_highlights_inserted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.queue.items = ["Apple", "Box", "Cat", "Fox", "Goat"]
        .iter()
        .enumerate()
        .map(|(i, title)| named(i, title))
        .collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "x");

    app.browser.tracks.items = vec![named(90, "Xylophone")];
    app.browser.tracks.state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;

    app.handle_msg(key('i'));

    assert_eq!(app.queue.items[1].title, "Xylophone", "queued after the current track");
    let cursor = app.queue.state.selected().expect("a row is highlighted");
    assert_eq!(cursor, app.queue.filter.cursor_of(1).expect("the inserted track is visible"));
    assert_eq!(
        app.queue.filter.real(cursor, app.queue.items.len()),
        1,
        "the highlight must stay on the inserted track"
    );
}

#[tokio::test]
async fn stop_cancels_a_replace_still_in_flight() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue.items = (0..3).map(track).collect();
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

    app.queue.items = (0..4).map(track).collect();
    app.queue.filter.text = "Starfall! 2".into();
    app.queue.filter.apply(4, |i| i == 2);
    run_cmd(&mut app, "save Road trip");
    assert_eq!(
        saves(&mut plex_rx),
        [("Road trip".to_string(), vec!["t0".into(), "t1".into(), "t2".into(), "t3".into()])]
    );

    run_cmd(&mut app, "save Again");
    assert!(saves(&mut plex_rx).is_empty(), "one save at a time");
}
