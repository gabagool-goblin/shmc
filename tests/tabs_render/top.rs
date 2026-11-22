//! Discover's Top lists and their period.

use super::discover::{numbered, on_library, page_of, Plex};
use super::*;
use shmc::plex::discover::LibraryList;
use shmc::plex::{Album, Artist};
use shmc::ui::app::{DiscoverCmd, DiscoverMsg, LibraryOutcome};
use shmc::ui::period::{Period, PeriodKind};
use shmc::ui::when::now_unix;

const TOP_TRACKS: usize = 5;
const TOP_ALBUMS: usize = 6;
const TOP_ARTISTS: usize = 7;

type Span = Option<(i64, i64)>;

fn top_requests(plex: &mut Plex) -> Vec<(LibraryList, u64, Span)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::LoadLibrary { list, generation, span, .. }) => {
                Some((list, generation, span))
            }
            _ => None,
        })
        .collect()
}

fn spans(plex: &mut Plex) -> Vec<Span> {
    top_requests(plex).into_iter().map(|r| r.2).collect()
}

fn span_of(kind: PeriodKind, back: u32) -> Span {
    let span = Period { kind, back }.span(now_unix());
    assert_eq!(span.is_some(), kind != PeriodKind::AllTime);
    span
}

fn played(i: usize, plays: u64) -> Track {
    Track { plays: Some(plays), ..track(i) }
}

#[tokio::test]
async fn a_top_list_asks_for_the_month_and_p_cycles_the_kind() {
    let (mut app, mut plex) = on_library(TOP_TRACKS).await;
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Month, 0)]);
    app.handle_msg(key('p'));
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Year, 0)]);
    app.handle_msg(key('p'));
    assert_eq!(spans(&mut plex), [None], "all time has no bounds");
    app.handle_msg(key('p'));
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Week, 0)]);
}

#[tokio::test]
async fn braces_step_the_period_and_stop_at_the_current_one() {
    let (mut app, mut plex) = on_library(TOP_ALBUMS).await;
    sent(&mut plex);
    app.handle_msg(key('{'));
    app.handle_msg(key('{'));
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Month, 1), span_of(PeriodKind::Month, 2)]);
    app.handle_msg(key('}'));
    app.handle_msg(key('}'));
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Month, 1), span_of(PeriodKind::Month, 0)]);
    app.handle_msg(key('}'));
    assert!(spans(&mut plex).is_empty(), "the current period is the last");

    app.handle_msg(key('p'));
    app.handle_msg(key('p'));
    sent(&mut plex);
    app.handle_msg(key('{'));
    app.handle_msg(key('}'));
    assert!(spans(&mut plex).is_empty(), "all time has no neighbours");
}

#[tokio::test]
async fn the_period_keys_do_nothing_off_a_top_list() {
    let (mut app, mut plex) = on_library(0).await;
    sent(&mut plex);
    for c in ['p', '{', '}'] {
        app.handle_msg(key(c));
    }
    assert!(spans(&mut plex).is_empty());
    app.discover.sections_state.select(Some(7 + TOP_TRACKS));
    app.handle_msg(Msg::Cava(Vec::new()));
    assert_eq!(spans(&mut plex), [span_of(PeriodKind::Month, 0)], "the period was not changed");
}

#[tokio::test]
async fn a_reply_from_before_a_period_change_is_dropped_but_other_lists_still_land() {
    let (mut app, mut plex) = on_library(0).await;
    let recent = top_requests(&mut plex);
    assert_eq!(recent.last().map(|r| r.0), Some(LibraryList::RecentlyPlayed));

    app.discover.sections_state.select(Some(7 + TOP_TRACKS));
    app.handle_msg(Msg::Cava(Vec::new()));
    let first = top_requests(&mut plex);
    assert_eq!(first.len(), 1);
    app.handle_msg(key('p'));
    let second = top_requests(&mut plex);
    assert_ne!(first[0].1, second[0].1, "a new period is a new generation");

    app.handle_msg(page_of(LibraryList::TopTracks, first[0].1, 0, numbered(0..3), 3));
    assert!(app.discover.tracks().is_empty(), "the old period's reply was dropped");
    app.handle_msg(page_of(LibraryList::RecentlyPlayed, recent[0].1, 0, numbered(0..5), 5));
    app.handle_msg(page_of(LibraryList::TopTracks, second[0].1, 0, numbered(10..12), 2));
    assert_eq!(app.discover.tracks().len(), 2);

    app.discover.sections_state.select(Some(7));
    app.handle_msg(Msg::Cava(Vec::new()));
    assert!(top_requests(&mut plex).is_empty(), "Recently played kept its page");
    assert_eq!(app.discover.tracks().len(), 5);
}

#[tokio::test]
async fn a_top_list_is_one_page_counted_after_copies_are_dropped() {
    let (mut app, mut plex) = on_library(TOP_TRACKS).await;
    sent(&mut plex);
    let mut tracks = numbered(0..100);
    tracks.push(Track { rating_key: "copy".into(), ..track(0) });
    app.handle_msg(page_of(LibraryList::TopTracks, 0, 0, tracks, 5000));
    app.handle_msg(key('l'));
    app.handle_msg(key('G'));
    assert!(top_requests(&mut plex).is_empty());
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("(100 tracks)"), "{screen}");
}

fn top_albums() -> Msg {
    let album = |key: &str, title: &str, plays| Album {
        plays: Some(plays),
        ..super::discover::album(key, title, "Mira Vale", 2020)
    };
    let albums = vec![album("a1", "Paper Lanterns", 1234), album("a2", "Quiet", 9)];
    let outcome = LibraryOutcome::Albums(albums);
    Msg::Discover(DiscoverMsg::Library {
        list: LibraryList::TopAlbums,
        generation: 0,
        start: 0,
        outcome,
    })
}

fn top_artists() -> Msg {
    let artist = |key: &str, title: &str, plays| Artist {
        rating_key: key.into(),
        title: title.into(),
        thumb: None,
        plays: Some(plays),
    };
    let artists = vec![artist("r1", "Mira Vale", 1234), artist("r2", "Northwind", 9)];
    let outcome = LibraryOutcome::Artists(artists);
    Msg::Discover(DiscoverMsg::Library {
        list: LibraryList::TopArtists,
        generation: 0,
        start: 0,
        outcome,
    })
}

#[tokio::test]
async fn the_period_line_and_plays_column_fit_at_every_size() {
    let (mut app, mut plex) = on_library(TOP_TRACKS).await;
    sent(&mut plex);
    let tracks = vec![played(1, 1234), played(2, 9)];
    app.handle_msg(page_of(LibraryList::TopTracks, 0, 0, tracks, 2));
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        assert!(screen.lines().all(|l| l.chars().count() <= w as usize), "{w}x{h}\n{screen}");
        assert!(screen.contains("week  month  year  all time"), "{w}x{h}\n{screen}");
        let header = screen.lines().find(|l| l.contains("Len")).expect("table header");
        let row = screen.lines().find(|l| l.contains("1 234")).map(str::to_string);
        if w >= 80 {
            let row = row.unwrap_or_else(|| panic!("plays at {w}x{h}\n{screen}"));
            assert!(header.contains("Plays") && row.contains("4:17"), "{header:?} {row:?}");
        } else {
            assert!(!header.contains("Plays") && row.is_none(), "dropped at {w}: {header:?}");
        }
    }

    app.discover.sections_state.select(Some(7 + TOP_ALBUMS));
    app.handle_msg(Msg::Cava(Vec::new()));
    sent(&mut plex);
    app.handle_msg(top_albums());
    app.discover.sections_state.select(Some(7 + TOP_ARTISTS));
    app.handle_msg(Msg::Cava(Vec::new()));
    app.handle_msg(top_artists());
    for (row, needle) in [(TOP_ALBUMS, "Paper Lanterns"), (TOP_ARTISTS, "Mira Vale")] {
        app.discover.sections_state.select(Some(7 + row));
        app.handle_msg(Msg::Cava(Vec::new()));
        for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
            let screen = render(&mut app, w, h);
            assert!(screen.lines().all(|l| l.chars().count() <= w as usize), "{w}x{h}\n{screen}");
            assert!(screen.contains("week  month  year  all time"), "{w}x{h}\n{screen}");
            let shown: String = needle.chars().take(6).collect();
            let line = screen.lines().find(|l| l.contains(&shown)).expect("a row");
            assert!(line.contains("1 234"), "plays clipped in {line:?} at {w}x{h}");
            assert!(screen.lines().any(|l| l.contains("Plays")), "{w}x{h}\n{screen}");
        }
    }
}

#[tokio::test]
async fn opening_a_top_album_leaves_the_period_line_for_the_album() {
    let (mut app, mut plex) = on_library(TOP_ALBUMS).await;
    sent(&mut plex);
    app.handle_msg(top_albums());
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: "a1".into(),
        outcome: super::discover::album_tracks(2),
    }));
    let screen = render(&mut app, 120, 32);
    assert!(!screen.contains("all time"), "{screen}");
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(render(&mut app, 120, 32).contains("week  month  year  all time"));
}
