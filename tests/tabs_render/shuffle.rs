use super::*;

#[tokio::test]
async fn shuffle_keeps_current_first_and_restores() {
    let mut app = new_app().await;
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue.state.select(Some(2));
    let original: Vec<String> = app.queue.items.iter().map(|t| t.rating_key.clone()).collect();

    app.handle_msg(key('z'));
    assert!(app.random);
    assert_eq!(app.queue.items[0].rating_key, "t2");
    assert_eq!(app.current, Some(0));
    assert_eq!(app.queue.items.len(), 6);

    app.handle_msg(key('z'));
    assert!(!app.random);
    assert_eq!(app.queue.items.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(), original);
    assert_eq!(app.current, Some(2));
}

#[tokio::test]
async fn reshuffle_keeps_the_current_track_first() {
    let mut app = new_app().await;
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(3);
    app.queue.state.select(Some(3));

    app.handle_msg(key('Z'));

    assert!(app.random);
    assert_eq!(app.queue.items[0].rating_key, "t3");
    assert_eq!(app.current, Some(0));
}

/// Sends mpv's positions for a reorder and returns the final index.
fn reorder_storm(app: &mut App, plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> usize {
    let expected = app.current.expect("a reorder only guards a playing track");
    let playing_thumb = app.current_track().and_then(|t| t.thumb.clone());
    let playing_key = app.current_track().map(|t| t.rating_key.clone());
    for pos in (0..app.queue.items.len()).filter(|&p| p != expected) {
        app.handle_msg(playlist_pos(pos));
        assert_eq!(app.current, Some(expected), "stale position {pos} moved the current track");
        assert_eq!(
            app.queue.state.selected(),
            Some(expected),
            "stale position {pos} bounced the queue cursor"
        );
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

/// `z` off reports intermediate positions that must not fetch art or bounce the cursor.
#[tokio::test]
async fn reorder_ignores_intermediate_positions() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.art_enabled = true;
    app.tab = Tab::Queue;
    app.queue.items = (0..60)
        .map(|i| Track { thumb: Some(format!("/library/metadata/t{i}/thumb")), ..track(i) })
        .collect();
    app.current = Some(20);
    app.queue.state.select(Some(20));
    app.stopped = false;

    // shuffle on pins the playing entry first
    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(0));
    assert_eq!(reorder_storm(&mut app, &mut plex), 0);

    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(20), "the playing track keeps its original slot");
    assert_eq!(app.current_track().map(|t| t.rating_key.as_str()), Some("t20"));
    assert_eq!(reorder_storm(&mut app, &mut plex), 20);

    app.handle_msg(playlist_pos(20));
    app.handle_msg(playlist_pos(21));
    assert_eq!(app.current, Some(21), "the position guard must not wedge");
}

/// `z` off is not quadratic: the time bound fails an O(n²) walk at 50 000 tracks.
#[tokio::test]
async fn shuffle_off_restores_large_queue_quickly() {
    let mut app = new_app().await;
    app.queue.items = (0..50_000).map(track).collect();
    app.current = Some(31_337);
    app.queue.state.select(Some(31_337));
    app.stopped = false;
    let original: Vec<String> = app.queue.items.iter().map(|t| t.rating_key.clone()).collect();

    let started = std::time::Instant::now();
    app.handle_msg(key('z'));
    app.handle_msg(key('z'));
    let elapsed = started.elapsed();

    let restored: Vec<String> = app.queue.items.iter().map(|t| t.rating_key.clone()).collect();
    assert_eq!(restored, original, "z off restores the original order");
    assert_eq!(app.current, Some(31_337), "playback stays on the same track");
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "shuffle off took {elapsed:?} for 50 000 tracks"
    );
}

#[tokio::test]
async fn shuffle_keeps_highlighted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..50).map(track).collect();
    app.current = Some(20);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    let cursor = app.queue.filter.cursor_of(12).expect("row is visible");
    app.queue.state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    for label in ["shuffle on", "shuffle off"] {
        app.handle_msg(key('z'));
        let selected = app.queue.state.selected().expect("a row stays highlighted");
        let real = app.queue.filter.real(selected, app.queue.items.len());
        assert_eq!(
            app.queue.items[real].rating_key, "t12",
            "{label} moved the highlight off the selected track"
        );
    }
}
