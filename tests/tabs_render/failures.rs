use super::*;

fn end_file_error() -> Msg {
    Msg::Mpv(MpvEvent::EndFile { reason: "error".into() })
}

/// One unplayable track is skipped; a burst of failures stops and points at `>`.
#[tokio::test]
async fn burst_of_failures_stops_playback() {
    let mut app = new_app().await;
    app.queue.items = (0..5).map(track).collect();
    app.queue.state.select(Some(0));
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "one bad file must not stop playback");
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");
    assert_eq!(app.queue.items.len(), 5, "the queue must survive the failure");
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
    assert_eq!(app.queue.items.len(), 5, "the queue must survive the failure");
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
    app.queue.items = (0..5).map(track).collect();
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
    app.queue.items = (0..5).map(track).collect();
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
    app.queue.items = (0..4).map(track).collect();
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
    app.queue.items = (0..3).map(track).collect();
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

/// A reorder moves `failed_at` with the failed track; an unmapped path clears it.
#[tokio::test]
async fn reorder_carries_failure_marker_with_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..5).map(track).collect();
    app.current = Some(2);
    app.queue.state.select(Some(2));
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");

    app.handle_msg(key('K'));
    assert_eq!(app.failed_at, Some(1), "a single move remaps the marker");
    assert_eq!(app.queue.items[1].rating_key, "t2");

    app.handle_msg(key('z'));
    let marker = app.failed_at.expect("a permutation keeps the marker");
    assert_eq!(
        app.queue.items[marker].rating_key, "t2",
        "shuffle must leave the marker on the failed track"
    );

    app.failed_at = Some(1);
    app.remove_from_queue(3);
    assert_eq!(app.failed_at, None, "a removal clears the marker");
}
