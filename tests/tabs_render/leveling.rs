use super::*;
use shmc::playback::{Leveling, MpvCmd};
use shmc::plex::Loudness;

fn analysed(lufs: f64) -> AudioInfo {
    AudioInfo {
        sample_rate_hz: Some(44100),
        loudness: Loudness { track_lufs: Some(lufs), ..Loudness::default() },
        ..AudioInfo::default()
    }
}

fn gains(mpv: &mut tokio::sync::mpsc::UnboundedReceiver<MpvCmd>) -> Vec<(String, f64)> {
    std::iter::from_fn(|| mpv.try_recv().ok())
        .filter_map(|c| match c {
            MpvCmd::SetGain { part, db } => Some((part, db)),
            _ => None,
        })
        .collect()
}

async fn leveled_app(
    mode: Leveling,
) -> (
    TestApp,
    tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
    tokio::sync::mpsc::UnboundedReceiver<MpvCmd>,
) {
    let (mut app, plex) = new_app_with_plex().await;
    let (mpv_tx, mpv) = unbounded_channel();
    app.mpv.tx = mpv_tx;
    app.set_leveling(mode, 0.0);
    app.queue.items = (0..3).map(track).collect();
    (app, plex, mpv)
}

#[tokio::test]
async fn playing_track_and_the_next_get_their_gains() {
    let (mut app, mut plex, mut mpv) = leveled_app(Leveling::Track).await;
    app.queue.items[0].audio = analysed(-8.0);
    app.play_queue_index(0);

    assert_eq!(gains(&mut mpv), [("0".to_string(), -10.0)]);
    let asked: Vec<String> = sent(&mut plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::LoadTrackInfo(key) => Some(key),
            _ => None,
        })
        .collect();
    assert!(asked.contains(&"t1".to_string()), "the next track is read ahead: {asked:?}");

    let info =
        Msg::TrackInfo { rating_key: "t1".into(), audio: analysed(-20.0), user_rating: None };
    app.handle_msg(info);
    assert_eq!(gains(&mut mpv), [("1".to_string(), 2.0)]);
}

#[tokio::test]
async fn unanalysed_track_is_published_at_zero() {
    let (mut app, _plex, mut mpv) = leveled_app(Leveling::Album).await;
    app.queue.items[0].audio = AudioInfo { sample_rate_hz: Some(44100), ..AudioInfo::default() };
    app.play_queue_index(0);

    assert_eq!(gains(&mut mpv), [("0".to_string(), 0.0)], "so the script does not wait");
}

#[tokio::test]
async fn leveling_off_publishes_nothing() {
    let (mut app, mut plex, mut mpv) = leveled_app(Leveling::Off).await;
    app.queue.items[0].audio = analysed(-8.0);
    app.play_queue_index(0);

    assert!(gains(&mut mpv).is_empty());
    assert!(sent(&mut plex).iter().all(|c| !matches!(c, PlexCmd::LoadTrackInfo(k) if k == "t1")));
}

#[tokio::test]
async fn missing_script_is_reported_once() {
    let (mut app, _plex, _mpv) = leveled_app(Leveling::Track).await;
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    assert!(app.status.contains("leveling is off"), "{}", app.status);

    app.status.clear();
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    assert!(app.status.is_empty(), "{}", app.status);
}

#[tokio::test]
async fn loaded_script_is_not_reported() {
    let (mut app, _plex, _mpv) = leveled_app(Leveling::Track).await;
    let ready = Msg::Mpv(MpvEvent::Property {
        name: shmc::playback::mpv::LEVELING_READY.into(),
        value: Some(true.into()),
    });
    app.handle_msg(ready);
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    assert!(!app.status.contains("leveling"), "{}", app.status);
}

#[tokio::test]
async fn play_next_prepares_the_new_next_tracks_gain() {
    let (mut app, _plex, mut mpv) = leveled_app(Leveling::Track).await;
    app.queue.items[0].audio = analysed(-8.0);
    app.play_queue_index(0);
    gains(&mut mpv);

    let mut next = track(7);
    next.audio = analysed(-14.0);
    app.play_next_tracks(vec![next], Confirm::Tracks);
    app.handle_msg(Msg::Resize);

    assert_eq!(gains(&mut mpv), [("7".to_string(), -4.0)]);
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn failed_next_track_lookup_is_asked_again_when_the_playing_track_changes() {
    let (mut app, mut plex, _mpv) = leveled_app(Leveling::Track).await;
    app.queue.items[0].audio = analysed(-8.0);
    let asks = |plex: &mut _| {
        sent(plex).iter().filter(|c| matches!(c, PlexCmd::LoadTrackInfo(k) if k == "t1")).count()
    };
    app.play_queue_index(0);
    assert_eq!(asks(&mut plex), 1);

    app.handle_msg(Msg::TrackInfoFailed { rating_key: "t1".into() });
    app.handle_msg(Msg::Resize);
    assert_eq!(asks(&mut plex), 0, "not on every message");

    app.play_queue_index(0);
    assert_eq!(asks(&mut plex), 1);
}

#[tokio::test]
async fn unwritable_script_is_blamed_instead_of_the_mpv_version() {
    let (mut app, _plex, _mpv) = leveled_app(Leveling::Track).await;
    app.mpv.script_unwritten = true;
    app.handle_msg(Msg::Mpv(MpvEvent::FileLoaded));
    assert!(app.status.contains("cannot write its mpv script"), "{}", app.status);
    assert!(!app.status.contains("mpv 0.38"), "{}", app.status);
}

#[tokio::test]
async fn a_save_that_turns_leveling_off_zeroes_the_published_gains() {
    let (mut app, _plex, mut mpv) = leveled_app(Leveling::Track).await;
    let _appearance = super::reload::AppearanceGuard::new();
    app.queue.items[0].audio = analysed(-8.0);
    app.play_queue_index(0);
    assert_eq!(gains(&mut mpv), [("0".to_string(), -10.0)]);

    app.handle_msg(super::reload::saved("[mpv]\nloudness_leveling = \"off\"\n", &[]));
    assert_eq!(gains(&mut mpv), [("0".to_string(), 0.0)]);
}

#[tokio::test]
async fn turning_leveling_on_without_its_script_waits_for_the_next_start() {
    let (mut app, _plex, mut mpv) = leveled_app(Leveling::Off).await;
    let _appearance = super::reload::AppearanceGuard::new();
    app.handle_msg(super::reload::saved("[mpv]\nloudness_leveling = \"track\"\n", &[]));
    assert!(app.status.contains("mpv.loudness_leveling on the next start"), "{}", app.status);
    app.queue.items[0].audio = analysed(-8.0);
    app.play_queue_index(0);
    assert!(gains(&mut mpv).is_empty());
}
