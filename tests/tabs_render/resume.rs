use super::*;
use shmc::playback::MpvCmd;

type PlexRx = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;
type MpvRx = tokio::sync::mpsc::UnboundedReceiver<MpvCmd>;

fn end_file_error() -> Msg {
    Msg::Mpv(MpvEvent::EndFile { reason: "error".into() })
}

/// Track 2 and the two after it fail, which stops playback.
async fn stopped_by_failures() -> (TestApp, PlexRx, MpvRx) {
    let (mut app, plex) = new_app_with_plex().await;
    let (mpv_tx, mpv) = unbounded_channel();
    app.mpv.tx = mpv_tx;
    app.queue.items = (0..5).map(track).collect();
    app.current = Some(2);
    app.stopped = false;
    for _ in 0..3 {
        app.handle_msg(end_file_error());
    }
    assert!(app.stopped);
    (app, plex, mpv)
}

fn answer_check(app: &mut App, plex: &mut PlexRx, ok: bool) -> std::time::Duration {
    let (epoch, delay) = last_sent(plex, |cmd| match cmd {
        PlexCmd::CheckConnection { epoch, delay } => Some((epoch, delay)),
        _ => None,
    })
    .expect("a connection check");
    app.handle_msg(Msg::ConnectionChecked { epoch, ok });
    delay
}

fn played(mpv: &mut MpvRx) -> Vec<usize> {
    std::iter::from_fn(|| mpv.try_recv().ok())
        .filter_map(|c| match c {
            MpvCmd::PlayIndex(i) => Some(i),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn failed_track_plays_again_once_plex_answers() {
    let (mut app, mut plex, mut mpv) = stopped_by_failures().await;

    assert_eq!(answer_check(&mut app, &mut plex, false), std::time::Duration::ZERO);
    assert!(app.status.contains("connection lost"), "{}", app.status);
    assert!(answer_check(&mut app, &mut plex, false) > std::time::Duration::ZERO, "it waits");
    answer_check(&mut app, &mut plex, true);

    assert_eq!(played(&mut mpv), [2], "the first failed track, not the one after");
    assert_eq!(app.current, Some(2));
    assert!(!app.stopped);
}

/// A check on a kept-alive connection waits out the drop and answers first.
#[tokio::test]
async fn first_answer_from_plex_resumes_too() {
    let (mut app, mut plex, mut mpv) = stopped_by_failures().await;
    answer_check(&mut app, &mut plex, true);

    assert_eq!(played(&mut mpv), [2]);
}

#[tokio::test]
async fn resume_that_fails_again_is_not_retried() {
    let (mut app, mut plex, mut mpv) = stopped_by_failures().await;
    answer_check(&mut app, &mut plex, true);
    assert_eq!(played(&mut mpv), [2]);
    for _ in 0..3 {
        app.handle_msg(end_file_error());
    }

    assert!(app.stopped);
    assert!(last_sent(&mut plex, |c| matches!(c, PlexCmd::CheckConnection { .. }).then_some(()))
        .is_none());
}

#[tokio::test]
async fn playing_something_else_while_offline_cancels_the_resume() {
    let (mut app, mut plex, mut mpv) = stopped_by_failures().await;
    answer_check(&mut app, &mut plex, false);
    app.play_queue_index(4);
    answer_check(&mut app, &mut plex, true);

    assert_eq!(played(&mut mpv), [4]);
    assert_eq!(app.current, Some(4));
}

#[tokio::test]
async fn entries_mpv_fails_through_fetch_nothing_until_one_loads() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue.items = (0..5).map(track).collect();
    app.current = Some(1);
    app.stopped = false;
    app.handle_msg(end_file_error());
    sent(&mut plex);

    let pos = |p: i64| {
        Msg::Mpv(MpvEvent::Property { name: "playlist-pos".into(), value: Some(p.into()) })
    };
    app.handle_msg(pos(2));
    assert!(sent(&mut plex).iter().all(|c| !matches!(c, PlexCmd::LoadTrackInfo(_))));

    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    let info = last_sent(&mut plex, |c| match c {
        PlexCmd::LoadTrackInfo(key) => Some(key),
        _ => None,
    });
    assert_eq!(info.as_deref(), Some("t2"));
}

#[tokio::test]
async fn burst_to_the_end_of_the_queue_resumes_at_the_first_failure() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (mpv_tx, mut mpv) = unbounded_channel();
    app.mpv.tx = mpv_tx;
    app.queue.items = (0..38).map(track).collect();
    app.current = Some(29);
    app.stopped = false;
    let pos = |p: i64| {
        Msg::Mpv(MpvEvent::Property { name: "playlist-pos".into(), value: Some(p.into()) })
    };
    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    for i in 30..38 {
        app.handle_msg(pos(i));
        app.handle_msg(end_file_error());
    }
    app.handle_msg(pos(-1));
    let checks = sent(&mut plex)
        .into_iter()
        .filter(|c| matches!(c, PlexCmd::CheckConnection { .. }))
        .count();
    assert_eq!(checks, 1, "one check per burst");
    app.handle_msg(Msg::ConnectionChecked { epoch: 1, ok: false });
    answer_check(&mut app, &mut plex, true);

    assert_eq!(played(&mut mpv), [30], "the first failure, not the last entry");
}
