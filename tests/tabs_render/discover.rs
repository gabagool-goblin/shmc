//! The Discover tab: the station list, previews and playing a station.

use super::queue_requests::mpv_matches_queue;
use super::*;
use shmc::config::RatingStyle;
use shmc::plex::discover::{DiscoverHubs, LibraryList, Shelf, ShelfItems, TrackPage};
use shmc::plex::radio::{RadioQueue, RadioSeed, Station};
use shmc::plex::{Album, Artist};
use shmc::ui::app::{
    DiscoverCmd, DiscoverCol, DiscoverMsg, LibraryOutcome, OpenKind, Opened, Pane, Preview,
};

pub(super) type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

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

fn loaded_with(shelves: Vec<Shelf>, stations: Vec<Station>) -> Msg {
    Msg::Discover(DiscoverMsg::Loaded(DiscoverHubs { shelves, stations }))
}

pub(super) fn loaded() -> Msg {
    loaded_with(Vec::new(), stations())
}

/// Discover opened and its stations listed, writes allowed.
pub(super) async fn on_discover() -> (TestApp, Plex) {
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

pub(super) fn previewed(app: &mut App, k: u32, n: usize) {
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
    sent(plex).iter().filter(|c| matches!(c, PlexCmd::Discover(DiscoverCmd::Load))).count()
}

#[tokio::test]
async fn opening_discover_asks_for_the_hubs_once() {
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
async fn a_failed_preview_is_asked_for_again_on_the_next_visit() {
    let (mut app, mut plex) = on_discover().await;
    app.handle_msg(Msg::Discover(DiscoverMsg::StationPreview {
        key: station_key(1),
        preview: Preview::Failed("the station returned no tracks".into()),
    }));
    app.handle_msg(key('j'));
    sent(&mut plex);
    app.handle_msg(key('k'));
    assert_eq!(previews_asked(&mut plex), [station_key(1)]);
    assert!(render(&mut app, 120, 32).contains("loading Library Radio…"));
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
    assert!(app.discover.pane.is_empty());
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
    assert_eq!(app.discover.focus, DiscoverCol::Right);
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t2", "t3", "t4"]);
    assert_eq!(app.current, Some(0));
    assert!(mpv_matches_queue(&app).await);
}

#[tokio::test]
async fn until_the_hubs_arrive_the_right_side_says_loading() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    assert!(render(&mut app, 120, 32).contains("loading Discover…"));
}

#[tokio::test]
async fn a_failed_load_still_lists_the_library() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    sent(&mut plex);
    app.handle_msg(Msg::Discover(DiscoverMsg::LoadFailed));
    let screen = render(&mut app, 120, 32);
    for text in ["Library", "Recently played", "Top rated", "Never played"] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
    assert!(!screen.contains("Stations"), "{screen}");
    assert!(app.status.contains("could not load Discover"), "{}", app.status);
    assert_eq!(library_asks(&mut plex), [(LibraryList::RecentlyPlayed, 0)]);
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
    app.ui.rating_style = RatingStyle::Five;
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

pub(super) fn album(key: &str, title: &str, artist: &str, year: u32) -> Album {
    Album {
        rating_key: key.into(),
        title: title.into(),
        artist: artist.into(),
        year: Some(year),
        thumb: None,
        plays: None,
    }
}

fn shelves() -> Vec<Shelf> {
    let albums = vec![
        album("a1", "Evening Satellite", "Northwind Sounds", 2019),
        album("a2", "Paper Lanterns", "Mira Vale", 2021),
        album("a3", "A Very Long Album Title For A Narrow Pane", "Northwind Sounds", 2015),
    ];
    let artists = ["Mira Vale", "Northwind Sounds"]
        .iter()
        .enumerate()
        .map(|(i, t)| Artist {
            rating_key: format!("r{i}"),
            title: t.to_string(),
            thumb: None,
            plays: None,
        })
        .collect();
    let playlists =
        vec![listed_playlist("p1", "Morning Mix", 12), listed_playlist("p2", "Late", 3)];
    vec![
        Shelf { title: "Recently Added".into(), items: ShelfItems::Albums(albums) },
        Shelf { title: "Popular Artists".into(), items: ShelfItems::Artists(artists) },
        Shelf { title: "Recent Playlists".into(), items: ShelfItems::Playlists(playlists) },
    ]
}

async fn on_shelves() -> (TestApp, Plex) {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.allow_writes = true;
    app.handle_msg(key('2'));
    app.handle_msg(loaded_with(shelves(), stations()));
    sent(&mut plex);
    (app, plex)
}

fn cursor(app: &App) -> Option<usize> {
    app.discover.sections_state.selected()
}

pub(super) fn open_requests(plex: &mut Plex) -> Vec<(String, bool)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::Open { key, what }) => {
                Some((key, matches!(what, OpenKind::Album)))
            }
            _ => None,
        })
        .collect()
}

pub(super) fn album_tracks(n: usize) -> Opened {
    Opened::Tracks { tracks: (0..n).map(track).collect(), total: Some(n) }
}

#[tokio::test]
async fn refresh_keeps_the_cursor_on_its_station_and_the_preview() {
    let (mut app, mut plex) = on_discover().await;
    app.handle_msg(key('j'));
    previewed(&mut app, 8, 30);
    sent(&mut plex);

    run_cmd(&mut app, "refresh");
    assert_eq!(load_requests(&mut plex), 1);
    let mut moved = stations();
    moved.rotate_right(1);
    app.handle_msg(loaded_with(Vec::new(), moved));
    assert!(previews_asked(&mut plex).is_empty(), "the station's tracks are still cached");
    assert!(render(&mut app, 120, 32).contains("Deep Cuts Radio (30 tracks)"));
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_shelves_and_stations() {
    let (mut app, _plex) = on_shelves().await;
    run_cmd(&mut app, "refresh");
    app.handle_msg(Msg::Discover(DiscoverMsg::LoadFailed));
    let screen = render(&mut app, 120, 32);
    for text in ["Suggested", "Recently Added", "Stations", "Deep Cuts Radio", "Library"] {
        assert!(screen.contains(text), "{text}\n{screen}");
    }
    assert!(app.status.contains("could not load Discover"), "{}", app.status);
}

#[tokio::test]
async fn the_cursor_skips_headings_and_blank_rows() {
    let (mut app, _plex) = on_shelves().await;
    assert_eq!(cursor(&app), Some(1), "the first shelf, not the heading");
    app.handle_msg(key('k'));
    assert_eq!(cursor(&app), Some(20), "wraps to the last row");
    app.handle_msg(key('j'));
    assert_eq!(cursor(&app), Some(1), "and back to the first shelf");
    for want in [2, 3, 6, 7] {
        app.handle_msg(key('j'));
        assert_eq!(cursor(&app), Some(want));
    }
    app.handle_msg(key('G'));
    assert_eq!(cursor(&app), Some(20));
    for want in [19, 18, 17, 14, 13, 12, 9, 8, 7, 6, 3] {
        app.handle_msg(key('k'));
        assert_eq!(cursor(&app), Some(want));
    }
    app.handle_msg(key('g'));
    app.handle_msg(key('g'));
    assert_eq!(cursor(&app), Some(1));
}

#[tokio::test]
async fn page_keys_never_stop_on_a_heading() {
    let (mut app, _plex) = on_shelves().await;
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(key_code(KeyCode::PageDown));
    assert_eq!(cursor(&app), Some(6), "the blank row and the heading are passed");
    app.handle_msg(key_code(KeyCode::PageUp));
    assert_eq!(cursor(&app), Some(3));
}

#[tokio::test]
async fn enter_on_a_shelf_album_opens_its_tracks_and_esc_restores_the_shelf() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('l'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(open_requests(&mut plex), [("a2".to_string(), true)]);
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "a2".into(),
        outcome: album_tracks(4),
    }));
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Recently Added ▸ Paper Lanterns (4 tracks)"), "{screen}");
    assert!(screen.contains("Starfall! 3"), "{screen}");

    app.handle_msg(key_code(KeyCode::Esc));
    let Pane::Items(pane) = &app.discover.pane else { panic!("the shelf is back") };
    assert_eq!(pane.state.selected(), Some(1), "on the album that was opened");
    assert!(render(&mut app, 120, 32).contains("Recently Added (3 albums)"));
}

#[tokio::test]
async fn an_open_reply_after_the_left_cursor_moved_is_dropped() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    assert_eq!(open_requests(&mut plex).len(), 1);
    app.handle_msg(key('h'));
    app.handle_msg(key('j'));
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "a1".into(),
        outcome: album_tracks(4),
    }));
    let Pane::Items(pane) = &app.discover.pane else { panic!("still the artists shelf") };
    assert_eq!(pane.shelf, "Popular Artists");
}

#[tokio::test]
async fn enter_on_an_artist_opens_their_albums_sorted() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('j'));
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    let asked = sent(&mut plex);
    assert!(asked.iter().any(|c| matches!(
        c,
        PlexCmd::Discover(DiscoverCmd::Open { key, what: OpenKind::Artist }) if key == "r0"
    )));
    let albums =
        vec![album("a9", "Newer", "Mira Vale", 2022), album("a8", "Older", "Mira Vale", 2001)];
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "r0".into(),
        outcome: Opened::Albums(albums),
    }));
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Popular Artists ▸ Mira Vale (2 albums)"), "{screen}");
    let (older, newer) = (screen.find("Older").unwrap(), screen.find("Newer").unwrap());
    assert!(older < newer, "oldest first by default\n{screen}");
}

#[tokio::test]
async fn enter_on_a_track_of_an_opened_album_plays_it_and_the_rest() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    sent(&mut plex);
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "a1".into(),
        outcome: album_tracks(5),
    }));
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t2", "t3", "t4"]);
    assert!(mpv_matches_queue(&app).await);
}

#[tokio::test]
async fn an_opened_playlist_plays_from_the_track_through_the_playlist_request() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    let asked = sent(&mut plex);
    assert!(asked.iter().any(|c| matches!(
        c,
        PlexCmd::Discover(DiscoverCmd::Open { key, what: OpenKind::Playlist { .. } }) if key == "p1"
    )));
    let tracks = (0..3)
        .map(|i| shmc::plex::Track { playlist_item_id: Some(format!("e{i}")), ..track(i) })
        .collect();
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "p1".into(),
        outcome: Opened::Tracks { tracks, total: Some(12) },
    }));
    assert!(render(&mut app, 120, 32).contains("Recent Playlists ▸ Morning Mix (12 tracks)"));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    let started = last_sent(&mut plex, |c| match c {
        PlexCmd::ResolvePlaylist { key, start, action, expected, .. } => {
            Some((key, start, action, expected))
        }
        _ => None,
    });
    assert_eq!(started, Some(("p1".to_string(), Some("e1".to_string()), QueueAction::Replace, 11)));
}

#[tokio::test]
async fn a_on_a_shelf_item_queues_the_album_artist_or_playlist() {
    let (mut app, mut plex) = on_shelves().await;
    app.handle_msg(key('l'));
    app.handle_msg(key('a'));
    assert!(sent(&mut plex).iter().any(|c| matches!(
        c,
        PlexCmd::ResolveAlbum { key, action: QueueAction::Append, .. } if key == "a1"
    )));

    app.handle_msg(key('h'));
    app.handle_msg(key('j'));
    app.handle_msg(key('l'));
    app.handle_msg(key('a'));
    assert!(sent(&mut plex).iter().any(|c| matches!(
        c,
        PlexCmd::ResolveArtistTracks { key, action: QueueAction::Append, .. } if key == "r0"
    )));

    app.handle_msg(key('h'));
    app.handle_msg(key('j'));
    app.handle_msg(key('l'));
    app.handle_msg(key('i'));
    assert!(sent(&mut plex).iter().any(|c| matches!(
        c,
        PlexCmd::ResolvePlaylist { key, action: QueueAction::PlayNext, .. } if key == "p1"
    )));
}

#[tokio::test]
async fn without_radio_the_shelves_show_and_the_stations_do_not() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.handle_msg(key('2'));
    app.handle_msg(loaded_with(shelves(), Vec::new()));
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("Suggested") && screen.contains("Recently Added"), "{screen}");
    assert!(!screen.contains("Stations"), "{screen}");
}

#[tokio::test]
async fn the_shelves_and_their_tables_fit_at_every_size() {
    let (mut app, _plex) = on_shelves().await;
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(screen.lines().all(|l| l.chars().count() <= w as usize), "{w}x{h}\n{screen}");
        for text in ["Suggested", "Stations"] {
            assert!(screen.contains(text), "{text} at {w}x{h}\n{screen}");
        }
        for title in ["Recently Added", "Popular Artists", "Recent Playlists"] {
            let shown: String = title.chars().take(10).collect();
            assert!(screen.contains(&shown), "{title} at {w}x{h}\n{screen}");
        }
        assert!(screen.contains("(3 albums)"), "{w}x{h}\n{screen}");
        let header = screen.lines().find(|l| l.contains("Album")).expect("album header");
        assert!(header.contains("Artist") && header.contains("Year"), "{header:?} at {w}x{h}");
        for (title, year) in [("Paper Lanterns", "2021"), ("A Very Long Album Title", "2015")] {
            let shown: String = title.chars().take(8).collect();
            let row = screen.lines().find(|l| l.contains(&shown));
            let row = row.unwrap_or_else(|| panic!("{title} at {w}x{h}\n{screen}"));
            assert!(row.contains(year), "{year} clipped in {row:?} at {w}x{h}");
            assert!(row.contains(title) || row.contains('…'), "{row:?} at {w}x{h}");
        }
    }
}

#[tokio::test]
async fn the_artist_and_playlist_shelves_have_their_own_columns() {
    let (mut app, _plex) = on_shelves().await;
    app.handle_msg(key('j'));
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(screen.contains("(2 artists)"), "{w}x{h}\n{screen}");
        assert!(screen.contains("│Artist "), "{w}x{h}\n{screen}");
        assert!(!screen.contains("Year"), "{w}x{h}\n{screen}");
    }
    app.handle_msg(key('j'));
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        let header = screen.lines().find(|l| l.contains("Tracks")).expect("header");
        assert!(header.contains("Playlist"), "{header:?} at {w}x{h}");
        assert!(header.contains("Tracks"), "{header:?} at {w}x{h}");
        let row = screen.lines().find(|l| l.contains("Morning Mix")).expect("row");
        assert!(row.trim_end().ends_with("12") || row.contains("12 "), "{row:?} at {w}x{h}");
    }
}

pub(super) fn library_requests(plex: &mut Plex) -> Vec<(LibraryList, u64, usize)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::LoadLibrary { list, generation, start, .. }) => {
                Some((list, generation, start))
            }
            _ => None,
        })
        .collect()
}

fn library_asks(plex: &mut Plex) -> Vec<(LibraryList, usize)> {
    library_requests(plex).into_iter().map(|(l, _, s)| (l, s)).collect()
}

pub(super) fn page_of(
    list: LibraryList,
    generation: u64,
    start: usize,
    tracks: Vec<Track>,
    total: usize,
) -> Msg {
    let rows = tracks.len();
    let page = TrackPage { tracks, rows, total: Some(total) };
    Msg::Discover(DiscoverMsg::Library {
        list,
        generation,
        start,
        outcome: LibraryOutcome::Tracks(page),
    })
}

pub(super) fn numbered(range: std::ops::Range<usize>) -> Vec<Track> {
    range.map(track).collect()
}

pub(super) async fn on_library(row: usize) -> (TestApp, Plex) {
    let (mut app, plex) = on_discover().await;
    app.discover.sections_state.select(Some(7 + row));
    app.handle_msg(Msg::Cava(Vec::new()));
    (app, plex)
}

#[tokio::test]
async fn highlighting_a_library_list_asks_for_its_first_page_once() {
    let (mut app, mut plex) = on_library(0).await;
    assert_eq!(library_asks(&mut plex), [(LibraryList::RecentlyPlayed, 0)]);
    app.handle_msg(key('j'));
    app.handle_msg(key('k'));
    assert!(library_requests(&mut plex).iter().all(|r| r.0 != LibraryList::RecentlyPlayed));

    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, numbered(0..5), 5));
    app.handle_msg(key('j'));
    app.handle_msg(key('k'));
    let again = library_asks(&mut plex);
    assert!(!again.contains(&(LibraryList::RecentlyPlayed, 0)), "kept once loaded: {again:?}");
    assert_eq!(app.discover.tracks().len(), 5);
}

#[tokio::test]
async fn nearing_the_loaded_end_asks_for_the_next_page_once_and_never_past_the_total() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, numbered(0..100), 250));
    assert!(library_requests(&mut plex).is_empty(), "the cursor is at the top");

    app.handle_msg(key('l'));
    app.handle_msg(key('G'));
    app.handle_msg(Msg::Cava(Vec::new()));
    assert_eq!(
        library_asks(&mut plex),
        [(LibraryList::RecentlyPlayed, 100)],
        "once while in flight"
    );

    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 100, numbered(100..200), 250));
    assert!(library_requests(&mut plex).is_empty(), "far from the end again");
    app.handle_msg(key('G'));
    assert_eq!(library_asks(&mut plex), [(LibraryList::RecentlyPlayed, 200)]);

    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 200, numbered(200..250), 250));
    app.handle_msg(key('G'));
    assert!(library_requests(&mut plex).is_empty(), "all 250 rows are loaded");
    assert_eq!(app.discover.tracks().len(), 250);
}

#[tokio::test]
async fn a_page_from_before_a_refresh_or_for_the_wrong_offset_is_dropped() {
    let (mut app, mut plex) = on_library(0).await;
    let first = library_requests(&mut plex);
    let generation = first.last().expect("a first request").1;

    run_cmd(&mut app, "refresh");
    app.handle_msg(loaded());
    let again = library_requests(&mut plex);
    assert_eq!(again, [(LibraryList::RecentlyPlayed, generation + 1, 0)], "the cursor stayed");
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, generation, 0, numbered(0..5), 5));
    assert!(app.discover.tracks().is_empty(), "the old page was dropped");

    app.handle_msg(page_of(LibraryList::RecentlyPlayed, generation + 1, 50, numbered(0..5), 90));
    assert!(app.discover.tracks().is_empty(), "wrong offset");
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, generation + 1, 0, numbered(0..5), 5));
    assert_eq!(app.discover.tracks().len(), 5);
}

#[tokio::test]
async fn the_same_song_on_two_pages_is_kept_once_and_offsets_count_server_rows() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    let same = |i: usize, title: &str| shmc::plex::Track { title: title.into(), ..track(i) };
    let first = vec![same(1, "Same Song"), track(2)];
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, first, 4));
    assert_eq!(library_asks(&mut plex), [(LibraryList::RecentlyPlayed, 2)]);
    let second = vec![same(3, "SAME SONG"), track(4)];
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 2, second, 4));
    let keys: Vec<_> = app.discover.tracks().iter().map(|t| t.rating_key.clone()).collect();
    assert_eq!(keys, ["t1", "t2", "t4"]);
}

#[tokio::test]
async fn enter_on_a_recently_played_track_queues_it_and_the_rows_loaded_so_far() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, numbered(0..6), 6));
    app.handle_msg(key('l'));
    app.handle_msg(key('j'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t2", "t3", "t4", "t5"]);
    assert!(mpv_matches_queue(&app).await);
}

#[tokio::test]
async fn never_played_opens_an_album_in_place_and_esc_restores_the_list() {
    let (mut app, mut plex) = on_library(2).await;
    assert_eq!(library_asks(&mut plex), [(LibraryList::NeverPlayed, 0)]);
    let albums = vec![
        album("n1", "Quiet One", "Mira Vale", 2020),
        album("n2", "Quiet Two", "Mira Vale", 2021),
    ];
    app.handle_msg(Msg::Discover(DiscoverMsg::Library {
        list: LibraryList::NeverPlayed,
        generation: 0,
        start: 0,
        outcome: LibraryOutcome::Albums(albums),
    }));
    app.handle_msg(key('l'));
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(open_requests(&mut plex), [("n2".to_string(), true)]);
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "n2".into(),
        outcome: album_tracks(3),
    }));
    assert!(render(&mut app, 120, 32).contains("Never played ▸ Quiet Two (3 tracks)"));
    app.handle_msg(key_code(KeyCode::Esc));
    let Pane::Items(pane) = &app.discover.pane else { panic!("the list is back") };
    assert_eq!(pane.state.selected(), Some(1));
}

#[tokio::test]
async fn the_played_column_fits_where_there_is_room_and_goes_before_it_clips() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    let at = 1_700_000_000;
    let tracks = (0..3)
        .map(|i| shmc::plex::Track { last_viewed_at: Some(at + i * 86_400), ..track(i as usize) })
        .collect();
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, tracks, 3));
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(
            screen.contains("Library") && screen.contains("Recently played"),
            "{w}x{h}\n{screen}"
        );
        assert!(screen.lines().all(|l| l.chars().count() <= w as usize), "{w}x{h}\n{screen}");
        let needle = if w >= 80 { "Starfall! 1" } else { "Starfall" };
        let row = screen.lines().find(|l| l.contains(needle)).expect("a track row");
        assert!(row.contains("4:17"), "Len clipped: {row:?} at {w}x{h}");
        let header = screen.lines().find(|l| l.contains("Len")).expect("table header");
        if w >= 80 {
            let played = shmc::ui::when::played_at(at + 86_400);
            assert!(
                header.contains("Played") && row.contains(&played),
                "{played} in {row:?} at {w}x{h}"
            );
        } else {
            assert!(!header.contains("Played"), "dropped at {w}: {header:?}");
        }
    }
}

#[tokio::test]
async fn enter_on_a_loaded_library_row_moves_to_its_list() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, 0, 0, numbered(0..5), 5));
    assert_eq!(app.discover.focus, DiscoverCol::Sections);
    app.handle_msg(enter());
    assert_eq!(app.discover.focus, DiscoverCol::Right);
}
