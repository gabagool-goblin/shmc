//! Discover's Best of lists.

use super::discover::{album, on_library, Plex};
use super::*;
use shmc::plex::discover::{BestOf, DiscoverHubs, Shelf, ShelfItems};
use shmc::ui::app::{DiscoverCmd, DiscoverCol, DiscoverMsg, OpenKind, Opened, Pane};
use shmc::ui::when::{local, now_unix};

const BEST_OF: usize = 8;

fn this_decade() -> u32 {
    local(now_unix()).expect("local time").year as u32 / 10 * 10
}

fn choice_requests(plex: &mut Plex) -> Vec<u64> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::LoadBestOf { generation }) => Some(generation),
            _ => None,
        })
        .collect()
}

fn choices(generation: u64) -> Msg {
    let d = this_decade();
    let years = vec![d + 3, d + 1, d, d - 1, d - 9];
    let decades = vec![d, d - 10, d - 20];
    Msg::Discover(DiscoverMsg::BestOf { generation, outcome: Ok((years, decades)) })
}

fn shown(app: &App) -> Vec<BestOf> {
    let Pane::Items(pane) = &app.discover.pane else { panic!("no list") };
    let ShelfItems::BestOf(v) = &pane.items else { panic!("not Best of") };
    v.clone()
}

#[tokio::test]
async fn best_of_asks_once_and_lists_this_decades_years_then_every_decade() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    assert_eq!(choice_requests(&mut plex), [0]);
    assert!(render(&mut app, 120, 32).contains("loading best of…"));
    app.handle_msg(key('k'));
    app.handle_msg(key('j'));
    assert!(choice_requests(&mut plex).is_empty(), "asked once while loading");

    app.handle_msg(choices(0));
    let d = this_decade();
    let want = [
        BestOf::Year(d + 3),
        BestOf::Year(d + 1),
        BestOf::Year(d),
        BestOf::Decade(d),
        BestOf::Decade(d - 10),
        BestOf::Decade(d - 20),
    ];
    assert_eq!(shown(&app), want);
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("▸ Best of") && !screen.contains("Best of ("), "{screen}");
    assert!(screen.contains(&format!("{}s", d - 10)), "{screen}");

    app.handle_msg(key('k'));
    app.handle_msg(key('j'));
    assert!(choice_requests(&mut plex).is_empty(), "kept once loaded");
}

#[tokio::test]
async fn a_refresh_asks_for_the_choices_again_and_drops_the_old_reply() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    assert_eq!(choice_requests(&mut plex), [0]);
    run_cmd(&mut app, "refresh");
    app.handle_msg(super::discover::loaded());
    assert_eq!(choice_requests(&mut plex), [1]);
    app.handle_msg(choices(0));
    assert!(matches!(app.discover.pane, Pane::Empty), "the old reply was dropped");
    app.handle_msg(choices(1));
    assert_eq!(shown(&app).len(), 6);
}

#[tokio::test]
async fn enter_opens_the_most_played_albums_in_the_servers_order_and_esc_goes_back() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    sent(&mut plex);
    app.handle_msg(choices(0));
    app.handle_msg(key('l'));
    app.handle_msg(key('a'));
    app.handle_msg(key('i'));
    assert!(sent(&mut plex).is_empty(), "a year is not something to queue");

    app.handle_msg(key('j'));
    app.handle_msg(enter());
    let d = this_decade();
    let asked: Vec<_> = sent(&mut plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::Open { key, what: OpenKind::BestOf(what) }) => {
                Some((key, what))
            }
            _ => None,
        })
        .collect();
    assert_eq!(asked, [(format!("best-of:{}", d + 1), BestOf::Year(d + 1))]);

    let played = |key: &str, title: &str, year, plays| shmc::plex::Album {
        plays: Some(plays),
        ..album(key, title, "Mira Vale", year)
    };
    let albums = vec![
        played("a1", "Zeta", 2024, 900),
        played("a2", "Alpha", 2021, 40),
        played("a3", "Mid", 2023, 7),
    ];
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: format!("best-of:{}", d + 1),
        outcome: Opened::Albums(albums),
    }));
    let Pane::Items(pane) = &app.discover.pane else { panic!("albums expected") };
    let ShelfItems::Albums(v) = &pane.items else { panic!("albums expected") };
    let titles: Vec<_> = v.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["Zeta", "Alpha", "Mid"]);
    for (w, h) in [(120u16, 32u16), (80, 24), (50, 40)] {
        let screen = render(&mut app, w, h);
        let row = screen.lines().find(|l| l.contains("Zeta")).expect("a row");
        assert!(row.contains("900"), "plays clipped in {row:?} at {w}x{h}");
        assert!(screen.lines().any(|l| l.contains("Plays")), "{w}x{h}\n{screen}");
    }
    assert!(render(&mut app, 120, 32).contains(&format!("Best of ▸ {} (3 albums)", d + 1)));

    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(shown(&app).len(), 6);
    let Pane::Items(pane) = &app.discover.pane else { panic!("the list is back") };
    assert_eq!(pane.state.selected(), Some(1));
}

#[tokio::test]
async fn an_empty_year_keeps_the_list_instead_of_opening_a_blank_pane() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    sent(&mut plex);
    app.handle_msg(choices(0));
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    let d = this_decade();
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: format!("best-of:{}", d + 3),
        outcome: Opened::Albums(Vec::new()),
    }));
    assert_eq!(shown(&app).len(), 6);
    let Pane::Items(pane) = &app.discover.pane else { panic!("the list stays") };
    assert_eq!(pane.state.selected(), Some(0));
    assert_eq!(app.discover.focus, DiscoverCol::Right);
}

#[tokio::test]
async fn a_server_with_no_best_of_choices_says_nothing_was_found() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    sent(&mut plex);
    let outcome = Ok((vec![this_decade() - 9], Vec::new()));
    app.handle_msg(Msg::Discover(DiscoverMsg::BestOf { generation: 0, outcome }));
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("nothing found") && !screen.contains("loading best of"), "{screen}");
}

#[tokio::test]
async fn a_best_of_reply_is_not_taken_for_the_reply_of_an_album_with_the_same_key() {
    let (mut app, mut plex) = on_library(BEST_OF).await;
    sent(&mut plex);
    app.handle_msg(choices(0));
    app.handle_msg(key('l'));
    app.handle_msg(enter());
    let best_of = sent(&mut plex)
        .into_iter()
        .find_map(|c| match c {
            PlexCmd::Discover(DiscoverCmd::Open { key, .. }) => Some(key),
            _ => None,
        })
        .expect("Best of was asked");
    app.handle_msg(key_code(KeyCode::Esc));

    let year = this_decade() + 3;
    let shelf = Shelf {
        title: "Albums".into(),
        items: ShelfItems::Albums(vec![album(&year.to_string(), "Same Key", "Mira Vale", 2020)]),
    };
    app.handle_msg(Msg::Discover(DiscoverMsg::Loaded(DiscoverHubs {
        shelves: vec![shelf],
        stations: Vec::new(),
    })));
    app.discover.sections_state.select(Some(1));
    app.handle_msg(Msg::Cava(Vec::new()));
    app.handle_msg(key('l'));
    app.handle_msg(enter());

    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: best_of,
        outcome: Opened::Albums(vec![album("a1", "Late", "Mira Vale", 2021)]),
    }));
    assert!(matches!(app.discover.pane, Pane::Items(_)), "the late Best of reply was dropped");
    app.handle_msg(Msg::Discover(DiscoverMsg::Opened {
        key: year.to_string(),
        outcome: super::discover::album_tracks(2),
    }));
    assert_eq!(app.discover.tracks().len(), 2, "the album's reply still applied");
}
