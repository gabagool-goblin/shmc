//! `w` and `W`: Sonic Adventure stops, the build request, its path and the stops line.

use super::*;
use ratatui::widgets::TableState;
use shmc::plex::models::{Album, HitKind, SearchHit};
use shmc::ui::app::{AdventureOutcome, BrowserCol, ListKind, SearchDetail, Stop};

type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

fn stop_keys(app: &App) -> Vec<&str> {
    app.adventure.stops.iter().map(|s| s.key.as_str()).collect()
}

async fn on_queue() -> (TestApp, Plex) {
    let (mut app, plex) = new_app_with_plex().await;
    queue_app_with_art(&mut app, false);
    (app, plex)
}

fn builds(plex: &mut Plex) -> Vec<(String, Vec<String>)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::BuildAdventure { key, stops } => {
                Some((key, stops.into_iter().map(|s| s.key).collect()))
            }
            _ => None,
        })
        .collect()
}

fn detail(tracks: Vec<Track>) -> SearchDetail {
    let mut state = TableState::default();
    state.select(Some(0));
    SearchDetail {
        key: "d".into(),
        label: "Album".into(),
        tracks,
        state,
        matches: None,
        radio: None,
    }
}

/// Two stops, t2 and t5, picked on the Queue.
fn two_stops(app: &mut App) {
    app.handle_msg(key('w'));
    app.queue.state.select(Some(5));
    app.handle_msg(key('w'));
}

fn path(keys: &[usize]) -> Msg {
    Msg::AdventureBuilt {
        key: "t2,t5".into(),
        outcome: AdventureOutcome::Path(keys.iter().map(|i| track(*i)).collect()),
    }
}

#[tokio::test]
async fn p_adds_the_highlighted_track_from_each_kind_of_list() {
    let (mut app, _plex) = on_queue().await;
    app.handle_msg(key('w'));

    app.tab = Tab::Browser;
    app.browser.tracks.items = vec![track(40)];
    app.browser.tracks.state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;
    app.handle_msg(key('w'));

    app.tab = Tab::Search;
    app.search.hits = vec![SearchHit {
        kind: HitKind::Track,
        rating_key: "s1".into(),
        title: "Quiet Hours".into(),
        artist: "Mira Vale".into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: None,
        artist_key: None,
        user_rating: None,
    }];
    app.search.state.select(Some(0));
    app.handle_msg(key('w'));

    app.search.detail = Some(detail(vec![track(50)]));
    app.handle_msg(key('w'));

    assert_eq!(stop_keys(&app), ["t2", "t40", "s1", "t50"]);
}

#[tokio::test]
async fn the_last_stop_cannot_repeat_but_may_return_later() {
    let (mut app, _plex) = on_queue().await;
    app.handle_msg(key('w'));
    app.handle_msg(key('w'));
    assert_eq!(app.status, "Starfall! 2 is already the last stop");
    app.queue.state.select(Some(3));
    app.handle_msg(key('w'));
    app.queue.state.select(Some(2));
    app.handle_msg(key('w'));
    assert_eq!(stop_keys(&app), ["t2", "t3", "t2"]);
}

#[tokio::test]
async fn an_eleventh_stop_is_refused() {
    let (mut app, _plex) = on_queue().await;
    for i in 0..11 {
        app.queue.state.select(Some(i));
        app.handle_msg(key('w'));
    }
    assert_eq!(app.adventure.stops.len(), 10);
    assert_eq!(app.status, "Sonic Adventure holds 10 tracks");
}

#[tokio::test]
async fn p_on_an_album_row_only_shows_a_hint() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.albums.items = vec![Album {
        rating_key: "al1".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        artist_key: None,
        year: None,
        thumb: None,
        plays: None,
    }];
    app.browser.albums.state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.handle_msg(key('w'));
    assert!(app.adventure.stops.is_empty());
    assert_eq!(app.status, "w adds tracks");
}

#[tokio::test]
async fn esc_clears_the_stops_before_going_back() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.adventure.stops.is_empty());
    assert_eq!(app.status, "Sonic Adventure cleared");

    app.tab = Tab::Search;
    app.search.detail = Some(detail(vec![track(50)]));
    app.search.state.select(Some(0));
    app.handle_msg(key('w'));
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.adventure.stops.is_empty());
    assert!(app.search.detail.is_some(), "the first Esc only clears the stops");
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.search.detail.is_none(), "with no stops Esc goes back as before");
}

#[tokio::test]
async fn capital_p_builds_only_from_two_stops() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('w'));
    app.handle_msg(key('W'));
    assert_eq!(app.status, "add at least 2 tracks with w");
    assert!(builds(&mut plex).is_empty());
    assert!(app.search.list_request.is_none());

    app.queue.state.select(Some(5));
    app.handle_msg(key('w'));
    app.handle_msg(key('W'));
    assert_eq!(builds(&mut plex), [("t2,t5".to_string(), vec!["t2".to_string(), "t5".into()])]);
    assert_eq!(app.search.list_request.as_ref().map(|r| r.kind), Some(ListKind::Adventure));
}

#[tokio::test]
async fn the_path_opens_in_search_and_esc_returns_to_the_origin() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    app.handle_msg(key('W'));
    app.handle_msg(path(&[2, 8, 5]));
    assert_eq!(app.tab, Tab::Search);
    let d = app.search.detail.as_ref().expect("the path is open");
    assert_eq!(d.label, "Sonic Adventure: Starfall! 2 to Starfall! 5");
    let keys: Vec<_> = d.tracks.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t2", "t8", "t5"]);
    assert!(d.matches.is_none() && d.radio.is_none());
    assert!(app.adventure.stops.is_empty());
    assert!(app.search.list_request.is_none());

    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Queue);
}

#[tokio::test]
async fn stops_picked_while_a_path_builds_stay_for_the_next_one() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    app.handle_msg(key('W'));
    app.queue.state.select(Some(7));
    app.handle_msg(key('w'));
    app.handle_msg(path(&[2, 8, 5]));
    assert_eq!(stop_keys(&app), ["t7"]);
}

#[tokio::test]
async fn a_failed_build_keeps_the_stops_and_opens_nothing() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    for (outcome, said) in [
        (
            AdventureOutcome::NoPath { from: "Starfall! 2".into(), to: "Starfall! 5".into() },
            "no path from Starfall! 2 to Starfall! 5",
        ),
        (AdventureOutcome::Failed, "Sonic Adventure failed"),
        (AdventureOutcome::NoSonic, "sonic analysis needs Plex Pass"),
    ] {
        app.handle_msg(key('W'));
        app.handle_msg(Msg::AdventureBuilt { key: "t2,t5".into(), outcome });
        assert_eq!(app.status, said);
        assert_eq!(stop_keys(&app), ["t2", "t5"]);
        assert!(app.search.detail.is_none());
        assert_eq!(app.tab, Tab::Queue);
    }
}

#[tokio::test]
async fn a_newer_request_or_cleared_stops_drop_a_late_build() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    app.handle_msg(key('W'));
    app.handle_msg(key('m'));
    app.handle_msg(path(&[2, 8, 5]));
    assert!(app.search.detail.is_none());
    assert_eq!(app.tab, Tab::Queue);
    assert_eq!(stop_keys(&app), ["t2", "t5"], "the stale reply cleared nothing");

    app.handle_msg(key('W'));
    app.handle_msg(key_code(KeyCode::Esc));
    app.handle_msg(path(&[2, 8, 5]));
    assert!(app.search.detail.is_none());
}

#[tokio::test]
async fn progress_shows_while_the_build_is_pending_only() {
    let (mut app, _plex) = on_queue().await;
    two_stops(&mut app);
    let progress = |leg| Msg::AdventureProgress { key: "t2,t5".into(), leg, legs: 3 };
    app.handle_msg(progress(2));
    assert_ne!(app.status, "building path 2/3…", "nothing is pending yet");
    app.handle_msg(key('W'));
    app.handle_msg(progress(2));
    assert_eq!(app.status, "building path 2/3…");
    app.handle_msg(key('m'));
    app.handle_msg(progress(3));
    assert_eq!(app.status, "finding similar tracks…");
}

fn stops_line(app: &mut App, w: u16) -> String {
    let text = render(app, w, 30);
    text.lines().find(|l| l.contains("Sonic Adventure:")).unwrap_or_default().to_string()
}

#[tokio::test]
async fn the_stops_line_shows_and_shrinks_without_clipping() {
    let (mut app, _plex) = on_queue().await;
    app.adventure.stops = ["Alpha", "Bravo Charlie Delta Echo", "Foxtrot", "Golf Hotel India"]
        .iter()
        .enumerate()
        .map(|(i, t)| Stop { key: format!("k{i}"), title: (*t).into() })
        .collect();
    app.status.clear();
    let wide = stops_line(&mut app, 120);
    assert!(
        wide.contains(
            "Sonic Adventure: Alpha → Bravo Charlie Delta Echo → Foxtrot → Golf Hotel India · W builds · Esc clears"
        ),
        "{wide}"
    );
    let narrow = stops_line(&mut app, 50);
    assert!(narrow.contains("· W builds · Esc clears"), "the hint stays whole: {narrow}");
    assert!(narrow.contains('…'), "the titles shrink: {narrow}");
}

#[tokio::test]
async fn a_transient_message_takes_the_bottom_row_before_the_stops_line() {
    let (mut app, _plex) = on_queue().await;
    app.adventure.stops = vec![Stop { key: "a".into(), title: "Alpha".into() }];
    app.status = "searching…".into();
    let shown = render(&mut app, 100, 30);
    assert!(shown.contains("searching…"));
    assert!(!shown.contains("Sonic Adventure:"));
}
