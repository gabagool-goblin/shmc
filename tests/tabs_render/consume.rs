use super::*;
use shmc::ui::app::BrowserCol;

#[tokio::test]
async fn consume_keeps_cursor_on_item_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..30).map(track).collect();
    app.current = Some(5); // finished track is outside the filtered view
    app.stopped = false;
    app.consume = true;
    filter_queue(&mut app, "Starfall! 2");
    // matches: 2, 12, 20..=29
    let cursor = app.queue.filter.cursor_of(29).expect("visible");
    app.queue.state.select(Some(cursor));

    app.handle_msg(end_file_eof());

    let sel = app.queue.state.selected().expect("a row is highlighted");
    let real = app.queue.filter.real(sel, app.queue.items.len());
    assert_eq!(app.queue.items[real].rating_key, "t29", "the cursor moved to another item");
}

/// Consume drops the finished track without restarting the next; the last one stops.
#[tokio::test]
async fn consume_removes_finished_entry() {
    let mut app = new_app().await;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(end_file_eof());

    assert_eq!(app.queue.items.len(), 2);
    assert_eq!(app.queue.items[0].title, "Starfall! 1");
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);

    drop(app); // one app at a time: it holds the suite's lock

    let mut app = new_app().await;
    app.queue.items = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(end_file_eof());

    assert!(app.queue.items.is_empty());
    assert_eq!(app.current, None);
    assert!(app.stopped);
}

/// After a consume a stale pre-removal position must not move `current` past the queue.
#[tokio::test]
async fn stale_playlist_pos_keeps_current_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    assert_eq!(app.queue.items.len(), 2);
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
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    app.handle_msg(playlist_pos(2));

    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks.items = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks.state.select(Some(0));

    app.handle_msg(key('i'));
    assert_eq!(app.queue.items.len(), 3, "the track was queued after the current one");
    assert_eq!(app.queue.items[2].rating_key, "t10");
}

/// The consume guard stays armed but must not swallow the position for a later `>`.
#[tokio::test]
async fn next_after_a_consume_follows_mpv() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(0);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(end_file_eof());
    assert_eq!(app.queue.items.len(), 5);
    assert_eq!(app.current, Some(0));

    // `>` must clear the old guard
    app.consume = false;
    app.handle_msg(key('>'));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1), "current must follow mpv after `>`");
}

/// With consume `>` removes the track left; the guard swallows only the stale position.
#[tokio::test]
async fn next_with_consume_removes_left_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..4).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.items.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t2", "t3"]);
    assert_eq!(app.current, Some(1), "current is the entry that was next");

    app.handle_msg(playlist_pos(2));
    assert_eq!(app.current, Some(1));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1));

    app.handle_msg(end_file_eof());
    let keys: Vec<_> = app.queue.items.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t3"]);
}

/// `>` at the end consumes nothing; with repeat on it consumes the last entry and wraps.
#[tokio::test]
async fn next_with_consume_on_the_last_entry() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(2);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    assert_eq!(app.queue.items.len(), 3, "repeat off: nothing to leave to");

    app.loop_playlist = true;
    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.items.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t1"]);
    assert_eq!(app.current, Some(0), "wrapped to the first entry");

    app.queue.items.truncate(1);
    app.current = Some(0);
    app.handle_msg(key('>'));
    assert_eq!(app.queue.items.len(), 1, "a lone track wraps to itself");
}
