//! `m`: sonically similar tracks, from the key to the Search tab's list.

use super::*;
use ratatui::widgets::TableState;
use shmc::plex::models::{Album, Artist, HitKind, SearchHit};
use shmc::ui::app::{BrowserCol, ListKind, ListRequest, SimilarKind, SimilarOutcome};

type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

/// `(key, kind, exclude, label)` of every similar request sent.
fn requests_of_kind(plex: &mut Plex) -> Vec<(String, SimilarKind, Option<String>, String)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::LoadSimilar { key, kind, exclude, label } => Some((key, kind, exclude, label)),
            _ => None,
        })
        .collect()
}

/// `(key, exclude, label)` of every similar request sent.
fn requests(plex: &mut Plex) -> Vec<(String, Option<String>, String)> {
    requests_of_kind(plex).into_iter().map(|(k, _, e, l)| (k, e, l)).collect()
}

fn album(key: &str, title: &str) -> Album {
    Album {
        rating_key: key.into(),
        title: title.into(),
        artist: "Northwind Sounds".into(),
        year: None,
        thumb: None,
    }
}

fn album_hit(key: &str, title: &str) -> SearchHit {
    SearchHit {
        kind: HitKind::Album,
        rating_key: key.into(),
        title: title.into(),
        artist: "Violet Ferry".into(),
        subtitle: "2016".into(),
        duration_ms: None,
        album_key: None,
        artist_key: Some("ar7".into()),
        user_rating: None,
    }
}

/// Three albums at 96, 95 and 90 percent.
fn found_albums(key: &str) -> Msg {
    let rows = vec![
        (album_hit("al21", "Salt Flats"), Some(96)),
        (album_hit("al22", "Night Ledger"), Some(95)),
        (album_hit("al23", "Low Signal"), Some(90)),
    ];
    Msg::SimilarLoaded {
        key: key.into(),
        label: "Sonically similar to Evening Satellite — Northwind Sounds".into(),
        outcome: SimilarOutcome::Hits(rows),
    }
}

/// The Browser with one artist and one album, the album column in focus.
async fn on_browser() -> (TestApp, Plex) {
    let (mut app, plex) = new_app_with_plex().await;
    app.browser.artists.items =
        vec![Artist { rating_key: "ar1".into(), title: "Northwind Sounds".into(), thumb: None }];
    app.browser.artists.state.select(Some(0));
    app.browser.albums.items = vec![album("al1", "Evening Satellite")];
    app.browser.albums.state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;
    (app, plex)
}

/// Rows `10..13` with matches 96, 95 and 90.
fn found(key: &str) -> Msg {
    let rows = vec![(track(10), Some(96)), (track(11), Some(95)), (track(12), Some(90))];
    Msg::SimilarLoaded {
        key: key.into(),
        label: "Sonically similar to Starfall! 2 — Northwind Sounds".into(),
        outcome: SimilarOutcome::Tracks(rows),
    }
}

async fn on_queue() -> (TestApp, Plex) {
    let (mut app, plex) = new_app_with_plex().await;
    queue_app_with_art(&mut app, false);
    (app, plex)
}

/// The track under the cursor, and the artist its own tracks are left out by.
#[tokio::test]
async fn m_on_a_queue_row_asks_for_that_track() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('m'));
    assert_eq!(
        requests(&mut plex),
        [(
            "t2".to_string(),
            Some("ar1".to_string()),
            "Sonically similar to Starfall! 2 — Northwind Sounds".to_string()
        )]
    );
}

/// The list replaces the album view in the Search tab, with Plex's matches.
#[tokio::test]
async fn found_tracks_open_in_the_search_tab() {
    let (mut app, _plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.handle_msg(found("t2"));
    assert_eq!(app.tab, Tab::Search);
    let d = app.search.detail.as_ref().expect("the list is open");
    assert_eq!(d.label, "Sonically similar to Starfall! 2 — Northwind Sounds");
    let keys: Vec<_> = d.tracks.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t10", "t11", "t12"]);
    assert_eq!(d.matches, Some(vec![Some(96), Some(95), Some(90)]));
    assert_eq!(d.state.selected(), Some(0));
    assert!(app.search.list_request.is_none());
}

/// Only the reply for the latest `m` applies.
#[tokio::test]
async fn reply_for_an_older_request_is_dropped() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.queue.state.select(Some(5));
    app.handle_msg(key('m'));
    let asked: Vec<_> = requests(&mut plex).into_iter().map(|r| r.0).collect();
    assert_eq!(asked, ["t2", "t5"]);

    app.handle_msg(found("t2"));
    assert!(app.search.detail.is_none());
    assert_eq!(app.tab, Tab::Queue);

    app.handle_msg(found("t5"));
    assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("t5"));
}

/// Opening an album ends the wait for a similar list, and `m` the wait for an album.
#[tokio::test]
async fn album_and_similar_requests_replace_each_other() {
    let (mut app, mut plex) = on_queue().await;
    app.search.hits = vec![shmc::plex::SearchHit {
        kind: shmc::plex::HitKind::Album,
        rating_key: "al9".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: None,
        artist_key: None,
        user_rating: None,
    }];
    app.search.state.select(Some(0));
    app.handle_msg(key('m'));
    app.tab = Tab::Search;
    app.handle_msg(enter());
    assert!(sent(&mut plex).iter().any(|c| matches!(c, PlexCmd::LoadAlbumDetail { .. })));
    app.handle_msg(found("t2"));
    assert!(app.search.detail.is_none(), "the album request replaced the similar one");

    app.tab = Tab::Queue;
    app.handle_msg(key('m'));
    app.handle_msg(Msg::SearchAlbumLoaded {
        key: "al9".into(),
        label: "Evening Satellite".into(),
        tracks: vec![track(0)],
    });
    assert!(app.search.detail.is_none(), "the similar request replaced the album one");
}

/// A track hit in the search results excludes its album artist too.
#[tokio::test]
async fn m_on_a_search_track_hit_asks_for_it() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.tab = Tab::Search;
    app.search.hits = vec![SearchHit {
        kind: HitKind::Track,
        rating_key: "s1".into(),
        title: "Quiet Hours".into(),
        artist: "Mira Vale".into(),
        subtitle: "Low Tides".into(),
        duration_ms: Some(200_000),
        album_key: Some("al9".into()),
        artist_key: Some("ar9".into()),
        user_rating: None,
    }];
    app.search.state.select(Some(0));
    app.handle_msg(key('m'));
    assert_eq!(
        requests(&mut plex),
        [(
            "s1".to_string(),
            Some("ar9".to_string()),
            "Sonically similar to Quiet Hours — Mira Vale".to_string()
        )]
    );
}

/// An album or artist in the Browser is asked about with its own key and no exclusion.
#[tokio::test]
async fn m_on_a_browser_album_or_artist_asks_for_it() {
    let (mut app, mut plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.browser.focus = BrowserCol::Artists;
    app.handle_msg(key('m'));
    assert_eq!(
        requests_of_kind(&mut plex),
        [
            (
                "al1".to_string(),
                SimilarKind::Albums,
                None,
                "Sonically similar to Evening Satellite — Northwind Sounds".to_string()
            ),
            (
                "ar1".to_string(),
                SimilarKind::Artists,
                None,
                "Sonically similar to Northwind Sounds".to_string()
            ),
        ]
    );
}

/// Found albums take the place of the search results and open like any album hit.
#[tokio::test]
async fn found_albums_replace_the_search_results() {
    let (mut app, mut plex) = on_browser().await;
    let mut state = TableState::default();
    state.select(Some(0));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "al1".into(),
        label: "Old".into(),
        tracks: vec![track(0)],
        state,
        matches: None,
        radio: None,
    });
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    assert_eq!(app.tab, Tab::Search);
    assert!(app.search.detail.is_none());
    let keys: Vec<_> = app.search.hits.iter().map(|h| h.rating_key.as_str()).collect();
    assert_eq!(keys, ["al21", "al22", "al23"]);
    let similar = app.search.similar.as_ref().expect("the list is labelled");
    assert_eq!(similar.matches, [Some(96), Some(95), Some(90)]);
    assert_eq!(app.search.state.selected(), Some(0));

    let _ = sent(&mut plex);
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(
        app.search.list_request.as_ref().map(|r| (r.key.as_str(), r.kind)),
        Some(("al22", ListKind::Album))
    );
}

/// Without Plex Pass, an unanalysed track or no result the list stays where it was.
#[tokio::test]
async fn empty_outcomes_leave_the_tab_and_the_list_alone() {
    for outcome in [SimilarOutcome::NoSonic, SimilarOutcome::NotAnalysed, SimilarOutcome::NoneFound]
    {
        let (mut app, _plex) = on_queue().await;
        let mut state = TableState::default();
        state.select(Some(0));
        app.search.detail = Some(shmc::ui::app::SearchDetail {
            key: "al1".into(),
            label: "Evening Satellite".into(),
            tracks: vec![track(0)],
            state,
            matches: None,
            radio: None,
        });
        app.handle_msg(key('m'));
        app.handle_msg(Msg::SimilarLoaded { key: "t2".into(), label: "x".into(), outcome });
        assert_eq!(app.tab, Tab::Queue);
        assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("al1"));
        assert!(app.search.list_request.is_none());
    }
}

/// Enter plays from the selected row, as in the album view.
#[tokio::test]
async fn enter_replaces_the_queue_from_the_selected_row() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.handle_msg(found("t2"));
    let _ = sent(&mut plex);
    app.handle_msg(key('j'));
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t11", "t12"]);
}

/// `m` inside the list asks about the row under the cursor.
#[tokio::test]
async fn m_in_the_similar_list_asks_for_the_selected_row() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.handle_msg(found("t2"));
    let _ = sent(&mut plex);
    app.handle_msg(key('j'));
    app.handle_msg(key('m'));
    let asked: Vec<_> = requests(&mut plex).into_iter().map(|r| r.0).collect();
    assert_eq!(asked, ["t11"]);
}

fn end_of(g: &[Vec<String>], needle: &str) -> Option<(usize, usize)> {
    g.iter().enumerate().find_map(|(y, row)| {
        let line = row.concat();
        let at = line.find(needle)?;
        Some((line[..at].chars().count() + needle.chars().count(), y))
    })
}

/// Match sits beside Len, right-aligned, and drops out on a narrow terminal.
#[tokio::test]
async fn match_column_shows_wide_and_drops_narrow() {
    let (mut app, _plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.handle_msg(found("t2"));

    let wide = grid(&mut app, 80, 24);
    let (header_end, _) = end_of(&wide, "Match").expect("Match header at 80 columns");
    let (value_end, _) = end_of(&wide, "96%").expect("96% at 80 columns");
    assert_eq!(header_end, value_end, "values align under the header's right edge");

    let narrow = render(&mut app, 50, 24);
    assert!(!narrow.contains("Match"), "Match is dropped at 50 columns:\n{narrow}");
    assert!(!narrow.contains("96%"));
    assert!(narrow.contains("Len"));
    assert_eq!(narrow.matches("4:17").count(), 3, "every Len value fits:\n{narrow}");
}

/// The album view has no Match column.
#[tokio::test]
async fn album_view_has_no_match_column() {
    let (mut app, _plex) = on_queue().await;
    let mut state = TableState::default();
    state.select(Some(0));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "al1".into(),
        label: "Evening Satellite".into(),
        tracks: (0..3).map(track).collect(),
        state,
        matches: None,
        radio: None,
    });
    app.tab = Tab::Search;
    let frame = render(&mut app, 80, 24);
    assert!(frame.contains("Len") && !frame.contains("Match"), "{frame}");
}

/// A similar albums list has Artist, Album, Year and Match; a narrow pane keeps the names.
#[tokio::test]
async fn similar_albums_show_label_and_match() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    for (w, h) in [(80, 24), (100, 60)] {
        let g = grid(&mut app, w, h);
        for (header, value) in [("Year", "2016"), ("Match", "96%")] {
            let (header_end, _) = end_of(&g, header).expect("header");
            let (value_end, _) = end_of(&g, value).expect("value");
            assert_eq!(header_end, value_end, "{value} sits under {header} at {w}x{h}");
        }
        let frame = render(&mut app, w, h);
        assert!(
            frame
                .contains("▸ Sonically similar to Evening Satellite — Northwind Sounds (3 albums)"),
            "{frame}"
        );
        assert!(!frame.contains("[Album]") && !frame.contains("Title"), "{frame}");
    }
    let narrow = render(&mut app, 50, 24);
    for gone in ["Year", "Match", "96%", "Len"] {
        assert!(!narrow.contains(gone), "{gone} at 50 columns:\n{narrow}");
    }
    assert!(narrow.contains("(3 albums)"), "{narrow}");
    for name in ["Violet Ferry", "Salt Flats", "Night Ledger", "Low Signal"] {
        assert!(narrow.contains(name), "{name} is clipped at 50 columns:\n{narrow}");
    }
}

/// A similar artists list is the names and Match, without the results' other columns.
#[tokio::test]
async fn similar_artists_show_names_and_match() {
    let (mut app, _plex) = on_browser().await;
    app.browser.focus = BrowserCol::Artists;
    app.handle_msg(key('m'));
    let artist = |key: &str, name: &str| SearchHit {
        kind: HitKind::Artist,
        rating_key: key.into(),
        title: String::new(),
        artist: name.into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: None,
        artist_key: None,
        user_rating: None,
    };
    let rows =
        vec![(artist("ar2", "Glass Harbour"), Some(96)), (artist("ar3", "Violet Ferry"), Some(95))];
    app.handle_msg(Msg::SimilarLoaded {
        key: "ar1".into(),
        label: "Sonically similar to Northwind Sounds".into(),
        outcome: SimilarOutcome::Hits(rows),
    });
    for (w, h) in [(80, 24), (100, 60)] {
        let g = grid(&mut app, w, h);
        let (header_end, _) = end_of(&g, "Match").expect("Match header");
        let (value_end, _) = end_of(&g, "96%").expect("96%");
        assert_eq!(header_end, value_end, "values align under the header at {w}x{h}");
        let frame = render(&mut app, w, h);
        assert!(frame.contains("Glass Harbour") && frame.contains("(2 artists)"), "{frame}");
        for gone in ["[Artist]", "Title", "Album", "Len"] {
            assert!(!frame.contains(gone), "{gone} at {w}x{h}:\n{frame}");
        }
    }
    let narrow = render(&mut app, 50, 24);
    assert!(narrow.contains("Violet Ferry") && !narrow.contains("Match"), "{narrow}");
}

fn album_loaded(key: &str) -> Msg {
    Msg::SearchAlbumLoaded {
        key: key.into(),
        label: "Salt Flats".into(),
        tracks: vec![track(0), track(1)],
    }
}

/// Esc undoes each step from the Browser's `m` through an album and a similar tracks list.
#[tokio::test]
async fn esc_walks_back_through_similar_lists_to_the_browser() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    app.handle_msg(enter());
    app.handle_msg(album_loaded("al21"));
    assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("al21"));
    app.handle_msg(key('m'));
    app.handle_msg(found("t0"));
    assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("t0"));

    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("al21"));
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.search.detail.is_none());
    assert_eq!(app.search.hits.len(), 3, "the similar albums are shown");
    assert_eq!(app.tab, Tab::Search);
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Browser);
}

/// Esc from a similar list made from search results brings the results back.
#[tokio::test]
async fn esc_restores_the_results_a_similar_list_replaced() {
    let (mut app, _plex) = new_app_with_plex().await;
    app.tab = Tab::Search;
    app.search.hits = vec![album_hit("al8", "Evening Satellite"), album_hit("al9", "Low Tides")];
    app.search.state.select(Some(1));
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al9"));
    assert_eq!(app.search.hits.len(), 3);
    assert!(app.search.similar.is_some());

    app.handle_msg(key_code(KeyCode::Esc));
    let keys: Vec<_> = app.search.hits.iter().map(|h| h.rating_key.as_str()).collect();
    assert_eq!(keys, ["al8", "al9"]);
    assert_eq!(app.search.state.selected(), Some(1));
    assert!(app.search.similar.is_none());
    assert_eq!(app.tab, Tab::Search);
}

#[tokio::test]
async fn a_new_search_clears_the_way_back() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    app.search.last_query = "tides".into();
    app.handle_msg(Msg::SearchResults {
        query: "tides".into(),
        hits: vec![album_hit("al5", "Low Tides")],
    });
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Search);
    assert_eq!(app.search.hits.len(), 1);
}

#[tokio::test]
async fn leaving_the_tab_clears_the_way_back() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    app.handle_msg(key_code(KeyCode::BackTab));
    assert_eq!(app.tab, Tab::Playlists);
    app.handle_msg(key_code(KeyCode::Tab));
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Search, "Esc has no tab to return to");
}

/// Going on from Help to another tab forgets it too.
#[tokio::test]
async fn leaving_through_help_clears_the_way_back() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    app.handle_msg(key_code(KeyCode::Tab));
    app.handle_msg(key_code(KeyCode::Tab));
    assert_eq!(app.tab, Tab::Queue);
    for _ in 0..4 {
        app.handle_msg(key_code(KeyCode::Tab));
    }
    assert_eq!(app.tab, Tab::Search);
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Search, "Esc has no tab to return to");
}

#[tokio::test]
async fn help_keeps_the_way_back() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    app.handle_msg(key_code(KeyCode::Tab));
    assert_eq!(app.tab, Tab::Help);
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Search);
    app.handle_msg(key_code(KeyCode::Esc));
    assert_eq!(app.tab, Tab::Browser);
}

/// A long walk drops its middle steps and keeps the way back to the first tab.
#[tokio::test]
async fn the_way_back_keeps_its_first_tab() {
    let (mut app, _plex) = on_browser().await;
    app.handle_msg(key('m'));
    app.handle_msg(found_albums("al1"));
    for i in 0..30 {
        app.search.list_request =
            Some(ListRequest { key: format!("al{i}"), kind: ListKind::Album, origin: Tab::Search });
        app.handle_msg(album_loaded(&format!("al{i}")));
    }
    for _ in 0..25 {
        if app.tab == Tab::Search {
            app.handle_msg(key_code(KeyCode::Esc));
        }
    }
    assert_eq!(app.tab, Tab::Browser);
}
