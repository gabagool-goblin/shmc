//! Playback reports: when a track starts reporting and what is sent.

use super::*;
use serde_json::json;
use shmc::plex::report::{PlayState, Report};
use tokio::sync::mpsc::UnboundedReceiver;

type Sent = UnboundedReceiver<Report>;

/// Three tracks with `t1` loaded and reporting on.
async fn loaded() -> (TestApp, Sent) {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.stopped = false;
    let (tx, rx) = unbounded_channel();
    app.reporter = Some(tx);
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    (app, rx)
}

fn at(app: &mut App, secs: f64) {
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "time-pos".into(),
        value: Some(json!(secs)),
    }));
}

fn play(app: &mut App, from: u32, to: u32) {
    for s in from..=to {
        at(app, f64::from(s));
    }
}

fn pause(app: &mut App, paused: bool) {
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "pause".into(),
        value: Some(json!(paused)),
    }));
}

fn end(app: &mut App, reason: &str) {
    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: reason.into() }));
}

fn drain(rx: &mut Sent) -> Vec<Report> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn states(reports: &[Report]) -> Vec<(PlayState, u64)> {
    reports.iter().map(|r| (r.state, r.time_ms / 1000)).collect()
}

#[tokio::test]
async fn a_track_left_before_thirty_seconds_sends_nothing() {
    let (mut app, mut rx) = loaded().await;
    play(&mut app, 0, 29);
    pause(&mut app, true);
    end(&mut app, "stop");
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn a_short_track_played_to_its_end_counts() {
    let (mut app, mut rx) = loaded().await;
    app.queue[1].duration_ms = Some(20_000);
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    play(&mut app, 0, 19);
    end(&mut app, "eof");
    assert_eq!(states(&drain(&mut rx)), [(PlayState::Playing, 18), (PlayState::Stopped, 20)]);
}

#[tokio::test]
async fn a_full_play_reports_playing_paused_playing_and_the_full_duration() {
    let (mut app, mut rx) = loaded().await;
    play(&mut app, 0, 35);
    pause(&mut app, true);
    pause(&mut app, false);
    end(&mut app, "eof");
    let got = drain(&mut rx);
    assert_eq!(
        states(&got),
        [
            (PlayState::Playing, 30),
            (PlayState::Paused, 35),
            (PlayState::Playing, 35),
            (PlayState::Stopped, 257),
        ]
    );
    assert!(got.iter().all(|r| r.rating_key == "t1" && r.duration_ms == 257_293));
    assert_eq!(got[3].time_ms, 257_293);
}

#[tokio::test]
async fn a_seek_is_not_play_and_a_seek_after_the_start_is_reported() {
    let (mut app, mut rx) = loaded().await;
    play(&mut app, 0, 10);
    at(&mut app, 200.0);
    play(&mut app, 201, 219);
    assert!(drain(&mut rx).is_empty(), "19 s of play after the seek");
    at(&mut app, 220.0);
    assert_eq!(states(&drain(&mut rx)), [(PlayState::Playing, 220)]);
    at(&mut app, 100.0);
    assert_eq!(states(&drain(&mut rx)), [(PlayState::Playing, 100)]);
}

#[tokio::test]
async fn a_playing_track_is_reported_every_ten_seconds_of_play() {
    let (mut app, mut rx) = loaded().await;
    play(&mut app, 0, 52);
    assert_eq!(
        states(&drain(&mut rx)),
        [(PlayState::Playing, 30), (PlayState::Playing, 40), (PlayState::Playing, 50)]
    );
}

#[tokio::test]
async fn a_track_is_stopped_before_the_next_one_reports() {
    let (mut app, mut rx) = loaded().await;
    play(&mut app, 0, 31);
    end(&mut app, "stop");
    app.handle_msg(playlist_pos(2));
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    play(&mut app, 0, 29);
    let got = drain(&mut rx);
    assert_eq!(states(&got), [(PlayState::Playing, 30), (PlayState::Stopped, 31)]);
    assert!(got.iter().all(|r| r.rating_key == "t1"), "{got:?}");
    play(&mut app, 30, 30);
    let got = drain(&mut rx);
    assert_eq!(states(&got), [(PlayState::Playing, 30)]);
    assert_eq!(got[0].rating_key, "t2");
}

#[tokio::test]
async fn shutdown_and_finishing_stop_a_started_track_only() {
    for via_shutdown in [true, false] {
        let (mut app, mut rx) = loaded().await;
        play(&mut app, 0, 29);
        if via_shutdown {
            app.handle_msg(Msg::Mpv(MpvEvent::Shutdown));
        } else {
            app.finish_reporting();
        }
        assert!(drain(&mut rx).is_empty());

        app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
        play(&mut app, 0, 31);
        drain(&mut rx);
        if via_shutdown {
            app.handle_msg(Msg::Mpv(MpvEvent::Shutdown));
        } else {
            app.finish_reporting();
        }
        assert_eq!(states(&drain(&mut rx)), [(PlayState::Stopped, 31)]);
    }
}
