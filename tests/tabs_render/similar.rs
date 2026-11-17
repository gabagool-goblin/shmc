//! `m`: sonically similar tracks, from the key to the Search tab's list.

use super::*;
use ratatui::widgets::TableState;
use shmc::plex::models::{Album, HitKind, SearchHit};
use shmc::ui::app::{BrowserCol, SimilarOutcome};

type Plex = tokio::sync::mpsc::UnboundedReceiver<PlexCmd>;

/// `(key, exclude, label)` of every similar request sent.
fn requests(plex: &mut Plex) -> Vec<(String, Option<String>, String)> {
    sent(plex)
        .into_iter()
        .filter_map(|c| match c {
            PlexCmd::LoadSimilar { key, exclude, label } => Some((key, exclude, label)),
            _ => None,
        })
        .collect()
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
    assert!(app.search.similar_request.is_none());
}

/// Only the reply for the latest `m` applies.
#[tokio::test]
async fn reply_for_an_older_request_is_dropped() {
    let (mut app, mut plex) = on_queue().await;
    app.handle_msg(key('m'));
    app.queue_state.select(Some(5));
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

/// An album or artist row is not a track.
#[tokio::test]
async fn m_on_an_album_row_sends_nothing() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.albums = vec![Album {
        rating_key: "al1".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        year: None,
        thumb: None,
    }];
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;
    app.handle_msg(key('m'));
    assert!(sent(&mut plex).is_empty());
    assert!(app.search.similar_request.is_none());
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
        });
        app.handle_msg(key('m'));
        app.handle_msg(Msg::SimilarLoaded { key: "t2".into(), label: "x".into(), outcome });
        assert_eq!(app.tab, Tab::Queue);
        assert_eq!(app.search.detail.as_ref().map(|d| d.key.as_str()), Some("al1"));
        assert!(app.search.similar_request.is_none());
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
    });
    app.tab = Tab::Search;
    let frame = render(&mut app, 80, 24);
    assert!(frame.contains("Len") && !frame.contains("Match"), "{frame}");
}
