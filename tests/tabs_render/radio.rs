//! `s`: Plex radio for a track or artist, from the key to the Search tab's list.

use super::*;
use shmc::plex::models::{Album, Artist, HitKind, SearchHit};
use shmc::plex::radio::{RadioQueue, ReadOn};
use shmc::ui::app::{BrowserCol, RadioOutcome, RadioSeed, SearchDetail};
use std::collections::HashSet;

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

fn seed() -> RadioSeed {
    RadioSeed::Artist("ar1".into())
}

fn loaded(key: &str) -> Msg {
    loaded_n(key, 2)
}

/// A radio of `n` tracks, t10 on, from play queue 9.
fn loaded_n(key: &str, n: usize) -> Msg {
    let tracks: Vec<_> = (10..10 + n).map(track).collect();
    let queue = RadioQueue { id: "9".into(), last: format!("q{}", 9 + n), seed: seed() };
    Msg::RadioLoaded {
        key: key.into(),
        label: "Northwind Sounds Radio".into(),
        outcome: RadioOutcome::Tracks(tracks, queue),
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

/// `(seq, queue, skip)` of every read for more tracks.
fn reads(plex: &mut Plex) -> Vec<(u64, RadioQueue, HashSet<String>)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::RadioMore { seq, queue, skip } => Some((seq, queue, skip)),
            _ => None,
        })
        .collect()
}

fn more(seq: u64, result: ReadOn) -> Msg {
    reply(seq, QueueOutcome::RadioMore { id: "9".into(), result })
}

/// A radio of `n` tracks from the Browser, played from the top of its Search list.
async fn radio_playing(n: usize) -> (TestApp, Plex) {
    let (mut app, plex) = on_browser().await;
    app.handle_msg(key('s'));
    app.handle_msg(loaded_n("ar1", n));
    app.handle_msg(enter());
    (app, plex)
}

/// A Search album list, which is not a radio.
fn open_album(app: &mut App) {
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(0));
    app.search.detail = Some(SearchDetail {
        key: "al1".into(),
        label: "Album".into(),
        tracks: (0..3).map(track).collect(),
        state,
        matches: None,
        radio: None,
    });
    app.tab = Tab::Search;
}

#[tokio::test]
async fn enter_on_a_radio_list_plays_it_from_the_row_with_shuffle_off() {
    let (mut app, _plex) = on_browser().await;
    app.random = true;
    app.handle_msg(key('s'));
    app.handle_msg(loaded_n("ar1", 20));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert!(!app.random);
    assert_eq!(queue_keys(&app).len(), 19);
    assert_eq!(queue_keys(&app)[0], "t11");
    assert_eq!(app.radio.as_ref().map(|r| r.name.as_str()), Some("Northwind Sounds Radio"));
}

#[tokio::test]
async fn enter_on_a_list_that_is_not_a_radio_starts_none() {
    let (mut app, _plex) = new_app_with_plex().await;
    open_album(&mut app);
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app).len(), 3);
    assert!(app.radio.is_none());
}

#[tokio::test]
async fn a_station_with_plenty_ahead_asks_for_nothing() {
    let (mut app, mut plex) = radio_playing(30).await;
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(reads(&mut plex).is_empty(), "29 tracks follow the current one");
}

#[tokio::test]
async fn nearing_the_end_asks_for_more_once_while_the_read_is_in_flight() {
    let (mut app, mut plex) = radio_playing(8).await;
    let asked = reads(&mut plex);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1, RadioQueue { id: "9".into(), last: "q17".into(), seed: seed() });
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(reads(&mut plex).is_empty(), "one read in flight");
}

#[tokio::test]
async fn the_reply_appends_to_the_queue_and_mpv_and_the_next_read_starts_after_it() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    let new: Vec<_> = (100..103).map(track).collect();
    app.handle_msg(more(seq, ReadOn::Tracks(new, "q102".into())));
    assert_eq!(queue_keys(&app).len(), 11);
    assert_eq!(queue_keys(&app)[8..], ["t100", "t101", "t102"]);
    assert!(!app.status.contains("added"), "{}", app.status);
    assert!(super::queue_requests::mpv_matches_queue(&app).await);
    let next = reads(&mut plex);
    assert_eq!(next.len(), 1, "10 tracks still follow the current one");
    assert_eq!(next[0].1.last, "q102");
}

#[tokio::test]
async fn each_read_skips_every_track_the_radio_added() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, _, skip) = reads(&mut plex).remove(0);
    let first: HashSet<String> = (10..18).map(|i| format!("t{i}")).collect();
    assert_eq!(skip, first);
    app.handle_msg(more(seq, ReadOn::Tracks(vec![track(100)], "q100".into())));
    let (_, _, skip) = reads(&mut plex).remove(0);
    assert_eq!(skip.len(), 9);
    assert!(skip.contains("t100"));
}

#[tokio::test]
async fn a_restarted_station_appends_and_reads_on_from_its_new_play_queue() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    let queue = RadioQueue { id: "12".into(), last: "q5".into(), seed: seed() };
    app.handle_msg(more(seq, ReadOn::Restarted(vec![track(200), track(201)], queue.clone())));
    assert_eq!(queue_keys(&app)[8..], ["t200", "t201"]);
    assert!(app.radio.is_some());
    assert!(super::queue_requests::mpv_matches_queue(&app).await);
    let (_, next, skip) = reads(&mut plex).remove(0);
    assert_eq!(next, queue);
    assert!(skip.contains("t201"));
}

#[tokio::test]
async fn a_replace_ends_the_radio_and_a_late_reply_changes_nothing() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    open_album(&mut app);
    app.handle_msg(enter());
    assert!(app.radio.is_none());
    app.handle_msg(more(seq, ReadOn::Tracks(vec![track(100)], "q100".into())));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2"]);
    assert!(reads(&mut plex).is_empty());
}

#[tokio::test]
async fn a_clear_ends_the_radio_and_a_late_reply_changes_nothing() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    app.clear_queue();
    assert!(app.radio.is_none());
    app.handle_msg(more(seq, ReadOn::Tracks(vec![track(100)], "q100".into())));
    assert!(app.queue.items.is_empty());
}

#[tokio::test]
async fn an_exhausted_station_ends_the_radio_with_its_name() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    app.handle_msg(more(seq, ReadOn::Exhausted));
    assert!(app.radio.is_none());
    assert_eq!(app.status, "Northwind Sounds Radio has no more tracks");
    assert_eq!(queue_keys(&app).len(), 8);
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(reads(&mut plex).is_empty());
}

#[tokio::test]
async fn a_dropped_play_queue_ends_the_radio() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    app.handle_msg(more(seq, ReadOn::Gone));
    assert!(app.radio.is_none());
    assert_eq!(app.status, "radio ended: the station no longer exists");
}

#[tokio::test]
async fn a_failed_read_asks_again_only_once_the_playing_track_changes() {
    let (mut app, mut plex) = radio_playing(8).await;
    let (seq, ..) = reads(&mut plex).remove(0);
    app.handle_msg(more(seq, ReadOn::Failed("boom".into())));
    assert!(app.radio.is_some(), "the radio stays");
    assert!(!app.status.contains("boom"), "{}", app.status);
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(reads(&mut plex).is_empty());

    app.current = Some(1);
    app.handle_msg(Msg::Cava(Vec::new()));
    assert_eq!(reads(&mut plex).len(), 1);
}

#[tokio::test]
async fn shuffle_is_refused_while_a_radio_plays() {
    let (mut app, _plex) = radio_playing(30).await;
    app.handle_msg(key('z'));
    assert!(!app.random);
    assert_eq!(app.status, "shuffle is off while radio plays");
    assert_eq!(queue_keys(&app)[0], "t10");
}

#[tokio::test]
async fn adding_a_track_keeps_the_radio() {
    let (mut app, _plex) = radio_playing(30).await;
    app.handle_msg(key('a'));
    assert!(app.radio.is_some());
}

#[tokio::test]
async fn the_queue_names_the_station_only_while_a_radio_plays() {
    for (w, h) in [(120u16, 32u16), (60, 30)] {
        let (mut app, _plex) = radio_playing(30).await;
        app.handle_msg(key('1'));
        let screen = render(&mut app, w, h);
        let rows: Vec<&str> = screen.lines().collect();
        let label = rows.iter().position(|l| l.contains("▸ Northwind Sounds Radio"));
        let label = label.unwrap_or_else(|| panic!("{w}x{h}\n{screen}"));
        let header = rows.iter().position(|l| l.contains("Title")).expect("table header");
        assert_eq!(header, label + 1, "{w}x{h}\n{screen}");
        assert!(screen.contains("Starfall! 10"), "{w}x{h}\n{screen}");

        app.clear_queue();
        let screen = render(&mut app, w, h);
        assert!(!screen.contains("▸ Northwind"), "{w}x{h}\n{screen}");
    }
}
