//! The Discover tab: the station list, previews and playing a station.

use super::queue_requests::mpv_matches_queue;
use super::*;
use shmc::config::RatingStyle;
use shmc::plex::radio::{RadioQueue, RadioSeed, Station};
use shmc::ui::app::{DiscoverCmd, DiscoverCol, DiscoverMsg, Preview, StationsOutcome};

type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

const TITLES: [&str; 4] =
    ["Library Radio", "Deep Cuts Radio", "Time Travel Radio", "Random Album Radio"];

fn stations() -> Vec<Station> {
    let keys = [1, 8, 2, 3];
    TITLES
        .iter()
        .zip(keys)
        .map(|(t, k)| Station {
            key: format!("/library/sections/1/stations/{k}"),
            title: t.to_string(),
        })
        .collect()
}

fn loaded() -> Msg {
    Msg::Discover(DiscoverMsg::StationsLoaded(StationsOutcome::Stations(stations())))
}

/// Discover opened and its stations listed, writes allowed.
async fn on_discover() -> (TestApp, Plex) {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.allow_writes = true;
    app.handle_msg(key('2'));
    app.handle_msg(loaded());
    sent(&mut plex);
    (app, plex)
}

fn station_key(k: u32) -> String {
    format!("/library/sections/1/stations/{k}")
}

fn previewed(app: &mut App, k: u32, n: usize) {
    let tracks = (0..n).map(track).collect();
    let seed = RadioSeed::Station(station_key(k));
    let queue = RadioQueue { id: "9".into(), last: format!("{}", n.saturating_sub(1)), seed };
    app.handle_msg(Msg::Discover(DiscoverMsg::StationPreview {
        key: station_key(k),
        preview: Preview::Ready(tracks, queue),
    }));
}

fn previews_asked(plex: &mut Plex) -> Vec<String> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::PreviewStation { key, .. }) => Some(key),
            _ => None,
        })
        .collect()
}

fn load_requests(plex: &mut Plex) -> usize {
    sent(plex).iter().filter(|c| matches!(c, PlexCmd::Discover(DiscoverCmd::LoadStations))).count()
}

#[tokio::test]
async fn opening_discover_asks_for_the_stations_once() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    assert_eq!(load_requests(&mut plex), 1);

    app.handle_msg(key('1'));
    app.handle_msg(key_code(KeyCode::Tab));
    assert_eq!(app.tab, Tab::Discover);
    assert_eq!(load_requests(&mut plex), 0, "not while it is loading");

    app.handle_msg(loaded());
    app.handle_msg(key('1'));
    run_cmd(&mut app, "discover");
    assert_eq!(app.tab, Tab::Discover);
    assert_eq!(load_requests(&mut plex), 0, "not once it is loaded");
}

#[tokio::test]
async fn the_station_titles_are_listed_without_counts() {
    let (mut app, _plex) = on_discover().await;
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(screen.contains("Stations"), "{w}x{h}\n{screen}");
        for title in TITLES {
            // the 50-wide pane is 15 cells: a longer title ends in an ellipsis
            let whole = w >= 80 || title.chars().count() <= 15;
            let shown = if whole { title.to_string() } else { title.chars().take(10).collect() };
            let row = screen.lines().find(|l| l.contains(&shown));
            let row = row.unwrap_or_else(|| panic!("{shown:?} at {w}x{h}\n{screen}"));
            assert_eq!(row.contains('…'), !whole, "{row:?} at {w}x{h}");
            assert!(!row.contains('('), "no count on {row:?}");
        }
    }
}

#[tokio::test]
async fn highlighting_a_station_previews_it_once_without_touching_the_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.allow_writes = true;
    app.queue.items = (50..53).map(track).collect();
    app.handle_msg(key('2'));
    app.handle_msg(loaded());
    assert_eq!(previews_asked(&mut plex), [station_key(1)]);
    app.handle_msg(key('j'));
    assert_eq!(previews_asked(&mut plex), [station_key(8)]);
    assert!(render(&mut app, 120, 32).contains("loading Deep Cuts Radio…"));
    for k in ['k', 'j'] {
        app.handle_msg(key(k));
    }
    assert!(previews_asked(&mut plex).is_empty(), "each station is asked for once");

    previewed(&mut app, 8, 30);
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(screen.contains("Deep Cuts Radio (30 tracks)"), "{w}x{h}\n{screen}");
        let header = screen.lines().find(|l| l.contains("Len")).expect("table header");
        assert!(!header.contains('#'), "no number column: {header}");
    }
    assert!(render(&mut app, 120, 32).contains("Starfall! 0"));
    assert_eq!(queue_keys(&app), ["t50", "t51", "t52"]);
}

#[tokio::test]
async fn without_allow_writes_nothing_is_previewed_or_started() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    app.handle_msg(loaded());
    app.handle_msg(key('j'));
    assert!(previews_asked(&mut plex).is_empty());
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Starting a station needs plex.allow_writes"), "{screen}");
    app.handle_msg(enter());
    assert!(app.status.contains("plex.allow_writes"), "{}", app.status);
}

#[tokio::test]
async fn enter_plays_the_previewed_tracks_without_asking_again() {
    let (mut app, mut plex) = on_discover().await;
    app.queue.items = (50..53).map(track).collect();
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 30);
    sent(&mut plex);
    app.handle_msg(enter());
    assert!(previews_asked(&mut plex).is_empty());
    assert_eq!(queue_keys(&app), (0..30).map(|i| format!("t{i}")).collect::<Vec<_>>());
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);
    assert!(mpv_matches_queue(&app).await, "mpv plays the previewed tracks");
}

#[tokio::test]
async fn a_station_plays_in_its_own_order_with_shuffle_off() {
    let (mut app, _plex) = on_discover().await;
    app.random = true;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 30);
    app.handle_msg(enter());
    assert!(!app.random);
    assert_eq!(queue_keys(&app), (0..30).map(|i| format!("t{i}")).collect::<Vec<_>>());
}

#[tokio::test]
async fn enter_before_the_preview_arrives_says_so() {
    let (mut app, _plex) = on_discover().await;
    app.queue.items = (50..53).map(track).collect();
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert!(app.status.contains("still loading Deep Cuts Radio"), "{}", app.status);
    assert_eq!(queue_keys(&app), ["t50", "t51", "t52"]);
}

#[tokio::test]
async fn a_failed_preview_shows_its_reason_and_keeps_the_queue() {
    let (mut app, _plex) = on_discover().await;
    app.queue.items = (50..53).map(track).collect();
    let failed = Preview::Failed("the station returned no tracks".into());
    app.handle_msg(Msg::Discover(DiscoverMsg::StationPreview {
        key: station_key(1),
        preview: failed,
    }));
    assert!(render(&mut app, 120, 32).contains("the station returned no tracks"));
    app.handle_msg(enter());
    assert!(app.status.contains("the station returned no tracks"), "{}", app.status);
    assert_eq!(queue_keys(&app), ["t50", "t51", "t52"]);
}

#[tokio::test]
async fn a_played_station_gets_fresh_tracks_on_the_next_visit() {
    let (mut app, mut plex) = on_discover().await;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 5);
    app.handle_msg(enter());
    sent(&mut plex);
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(previews_asked(&mut plex).is_empty(), "staying on it asks for nothing");
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Deep Cuts Radio (5 tracks)"), "{screen}");

    app.handle_msg(key('k'));
    assert!(previews_asked(&mut plex).is_empty(), "Library Radio is still loading");
    app.handle_msg(key('j'));
    assert_eq!(previews_asked(&mut plex), [station_key(8)]);
}

#[tokio::test]
async fn a_preview_for_another_station_waits_for_its_turn() {
    let (mut app, mut plex) = on_discover().await;
    app.handle_msg(key('j'));
    sent(&mut plex);
    previewed(&mut app, 1, 2);
    assert!(app.discover.tracks.is_empty() && app.discover.label.is_none());
    app.handle_msg(key('k'));
    assert!(previews_asked(&mut plex).is_empty());
    assert!(render(&mut app, 120, 32).contains("Library Radio (2 tracks)"));
}

#[tokio::test]
async fn enter_on_a_station_track_plays_it_and_the_rest() {
    let (mut app, _plex) = on_discover().await;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 5);
    app.handle_msg(key('l'));
    assert_eq!(app.discover.focus, DiscoverCol::Tracks);
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t2", "t3", "t4"]);
    assert_eq!(app.current, Some(0));
    assert!(mpv_matches_queue(&app).await);
}

#[tokio::test]
async fn without_plex_pass_radio_the_right_side_says_so() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    assert!(render(&mut app, 120, 32).contains("loading stations…"));
    app.handle_msg(Msg::Discover(DiscoverMsg::StationsLoaded(StationsOutcome::NoRadio)));
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Stations need Plex Pass radio"), "{screen}");
    assert!(app.discover.stations.is_empty());
}

#[tokio::test]
async fn a_failed_load_says_so() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    app.handle_msg(Msg::Discover(DiscoverMsg::StationsLoaded(StationsOutcome::Failed)));
    assert!(render(&mut app, 120, 32).contains("could not load stations"));
}

#[tokio::test]
async fn the_number_keys_and_tab_reach_the_new_order() {
    let mut app = new_app().await;
    for (k, tab) in [
        ('2', Tab::Discover),
        ('3', Tab::Browser),
        ('4', Tab::Playlists),
        ('5', Tab::Search),
        ('6', Tab::Help),
        ('1', Tab::Queue),
    ] {
        app.handle_msg(key(k));
        assert_eq!(app.tab, tab, "{k}");
    }
    app.handle_msg(key_code(KeyCode::Tab));
    assert_eq!(app.tab, Tab::Discover);
    app.handle_msg(key_code(KeyCode::Tab));
    assert_eq!(app.tab, Tab::Browser);
}

#[tokio::test]
async fn r_rates_the_highlighted_station_track() {
    let (mut app, mut plex) = on_discover().await;
    app.rating_style = RatingStyle::Five;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 3);
    sent(&mut plex);
    app.handle_msg(key('l'));
    app.handle_msg(key('j'));
    app.handle_msg(key('r'));
    app.handle_msg(key('4'));
    let rated = last_sent(&mut plex, |c| match c {
        PlexCmd::Rate { key, rating, .. } => Some((key, rating)),
        _ => None,
    });
    assert_eq!(rated, Some(("t1".to_string(), Some(8))));
}

#[tokio::test]
async fn v_selects_station_tracks_and_a_adds_the_range() {
    let (mut app, _plex) = on_discover().await;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 5);
    app.handle_msg(key('l'));
    app.handle_msg(key('j'));
    app.handle_msg(key('v'));
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"]);
    assert!(app.visual.is_none());

    app.handle_msg(key('v'));
    app.handle_msg(key('3'));
    assert!(app.visual.is_none(), "leaving the tab ends the selection");
}
