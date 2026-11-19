//! `s`: Plex radio for a track or artist, from the key to the Search tab's list.

use super::*;
use shmc::plex::models::{Album, Artist, HitKind, SearchHit};
use shmc::ui::app::{BrowserCol, RadioOutcome, RadioSeed};

type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

/// `(key, seed, label)` of every radio request sent.
fn requests(plex: &mut Plex) -> Vec<(String, RadioSeed, String)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::StartRadio { key, seed, label } => Some((key, seed, label)),
            _ => None,
        })
        .collect()
}

/// The Browser with one artist and one album, the artist column in focus.
async fn on_browser() -> (TestApp, Plex) {
    let (mut app, plex) = new_app_with_plex().await;
    app.allow_writes = true;
    app.browser.artists.items =
        vec![Artist { rating_key: "ar1".into(), title: "Northwind Sounds".into(), thumb: None }];
    app.browser.artists.state.select(Some(0));
    app.browser.albums.items = vec![Album {
        rating_key: "al1".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        year: None,
        thumb: None,
    }];
    app.browser.albums.state.select(Some(0));
    app.browser.focus = BrowserCol::Artists;
    app.tab = Tab::Browser;
    (app, plex)
}

fn loaded(key: &str) -> Msg {
    Msg::RadioLoaded {
        key: key.into(),
        label: "Northwind Sounds Radio".into(),
        outcome: RadioOutcome::Tracks(vec![track(10), track(11)]),
    }
}

#[tokio::test]
async fn s_on_an_artist_or_a_track_asks_for_its_radio() {
    let (mut app, mut plex) = on_browser().await;
    app.handle_msg(key('s'));
    assert_eq!(
        requests(&mut plex),
        [("ar1".to_string(), RadioSeed::Artist("ar1".into()), "Northwind Sounds Radio".into())]
    );

    queue_app_with_art(&mut app, false);
    app.handle_msg(key('s'));
    assert_eq!(
        requests(&mut plex),
        [("t2".to_string(), RadioSeed::Track("t2".into()), "Starfall! 2 Radio".into())]
    );
}

#[tokio::test]
async fn without_allow_writes_nothing_is_sent() {
    let (mut app, mut plex) = on_browser().await;
    app.allow_writes = false;
    app.handle_msg(key('s'));
    assert!(requests(&mut plex).is_empty());
    assert!(app.status.contains("plex.allow_writes"), "{}", app.status);
}

#[tokio::test]
async fn s_on_an_album_sends_nothing() {
    let (mut app, mut plex) = on_browser().await;
    app.browser.focus = BrowserCol::Albums;
    app.handle_msg(key('s'));
    assert!(requests(&mut plex).is_empty());
}

#[tokio::test]
async fn s_on_search_hits_asks_for_tracks_and_artists_only() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.allow_writes = true;
    app.tab = Tab::Search;
    let hit = |kind, key: &str| SearchHit {
        kind,
        rating_key: key.into(),
        title: "Quiet Hours".into(),
        artist: "Mira Vale".into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: None,
        artist_key: None,
        user_rating: None,
    };
    app.search.hits =
        vec![hit(HitKind::Track, "s1"), hit(HitKind::Artist, "s2"), hit(HitKind::Album, "s3")];
    for i in 0..3 {
        app.search.state.select(Some(i));
        app.handle_msg(key('s'));
    }
    let got: Vec<_> = requests(&mut plex).into_iter().map(|r| (r.0, r.1)).collect();
    assert_eq!(
        got,
        [
            ("s1".to_string(), RadioSeed::Track("s1".into())),
            ("s2".to_string(), RadioSeed::Artist("s2".into())),
        ]
    );
}

/// The tracks open in the Search tab, and Esc goes back to where `s` was pressed.
#[tokio::test]
async fn found_tracks_open_in_the_search_tab_and_esc_goes_back() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('s'));
    app.handle_msg(loaded("ar1"));
    assert_eq!(app.tab, Tab::Search);
    let d = app.search.detail.as_ref().expect("the list is open");
    assert_eq!(d.label, "Northwind Sounds Radio");
    let keys: Vec<_> = d.tracks.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t10", "t11"]);
    assert_eq!(d.state.selected(), Some(0));

    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Browser);
}

/// A reply is dropped once another list request has replaced the radio one.
#[tokio::test]
async fn reply_for_a_request_no_longer_pending_is_ignored() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('s'));
    app.handle_msg(loaded("ar9"));
    assert!(app.search.detail.is_none());
    app.handle_msg(key('m'));
    app.handle_msg(loaded("ar1"));
    assert!(app.search.detail.is_none());
    assert_eq!(app.tab, Tab::Browser);
}
