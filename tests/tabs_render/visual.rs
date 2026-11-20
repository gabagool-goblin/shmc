use super::*;
use shmc::ui::app::{BrowserCol, ListKind, ListRequest};

#[tokio::test]
async fn visual_range_above_playing_removes_and_undoes() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..7).map(track).collect();
    app.current = Some(5);
    app.queue.state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(2));
    assert!(app.visual.is_none());

    // mpv walks the playing entry down through the slots in between
    app.handle_msg(playlist_pos(3));
    assert_eq!(app.current, Some(2), "a stale position moved current");

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2", "t3", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(5));
    assert_eq!(app.queue.state.selected(), Some(1));
}

#[tokio::test]
async fn visual_range_with_playing_hands_to_next() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue.state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5"]);
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue.items[1].rating_key, "t4");
    assert!(!app.stopped);
}

#[tokio::test]
async fn visual_range_under_filter_removes_visible_rows() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..30).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
    app.stopped = false;
    // matches: t2, t20..=t29
    filter_queue(&mut app, "Starfall! 2");

    keys(&mut app, "vj");
    app.handle_msg(key('d'));
    let left = queue_keys(&app);
    assert_eq!(left.len(), 28);
    assert!(!left.contains(&"t2".to_string()) && !left.contains(&"t20".to_string()));
    assert!(left.contains(&"t3".to_string()) && left.contains(&"t21".to_string()));

    app.handle_msg(key('u'));
    let all: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
    assert_eq!(queue_keys(&app), all);
}

#[tokio::test]
async fn visual_mode_lifetime() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..6).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(3));
    app.stopped = false;

    app.handle_msg(key('v'));
    assert_eq!(app.visual.map(|v| v.anchor), Some(3));

    keys(&mut app, "JK");
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2", "t3", "t4", "t5"]);
    assert_eq!(app.visual.map(|v| v.anchor), Some(3), "J/K must not end visual mode");

    app.add_tracks(vec![track(9)], Confirm::Track { title: "Starfall! 9".into() });
    assert_eq!(app.visual.map(|v| v.anchor), Some(3), "an append keeps visual mode");

    // the finished entry above the anchor shifts it down by one
    app.consume = true;
    app.current = Some(1);
    app.handle_msg(end_file_eof());
    assert_eq!(app.visual.map(|v| v.anchor), Some(2));
    assert_eq!(app.queue.items[2].rating_key, "t3");

    app.handle_msg(key('2'));
    assert!(app.visual.is_none(), "leaving the tab ends it");

    app.handle_msg(key('1'));
    app.handle_msg(key('v'));
    app.replace_queue_and_play(vec![track(0)], Confirm::Track { title: "Starfall! 0".into() });
    assert!(app.visual.is_none(), "a replace ends it");

    app.handle_msg(key('v'));
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_mode_shows_label_and_count() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..8).map(track).collect();
    app.queue.state.select(Some(1));
    app.stopped = true;
    app.handle_msg(key('v'));
    keys(&mut app, "jj");

    for w in [120u16, 40, 20] {
        let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let text = flatten(&terminal);
        let bottom = text.lines().rev().find(|l| l.contains("VISUAL")).unwrap_or("");
        assert!(bottom.contains("-- VISUAL --"), "{w}: label clipped");
        assert_eq!(bottom.contains("3 selected"), w >= 40, "{w}: counter");
        if w == 120 {
            let (x, inside) = locate(&terminal, "Starfall! 2").expect("row");
            let (_, outside) = locate(&terminal, "Starfall! 5").expect("row");
            let buf = terminal.backend().buffer();
            assert_ne!(buf.cell((x, inside)).unwrap().bg, buf.cell((x, outside)).unwrap().bg);
        }
    }
}

/// Browser tracks t0..t9 titled "Starfall! n", the cursor on row 0.
fn on_browser_tracks(app: &mut App, n: usize) {
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks_for = Some("al1".into());
    app.browser.tracks.items = (0..n).map(track).collect();
    app.browser.tracks.state.select(Some(0));
}

fn filter_browser_tracks(app: &mut App, needle: &str) {
    let titles: Vec<String> = app.browser.tracks.items.iter().map(|t| t.title.clone()).collect();
    app.browser.tracks.filter.text = needle.into();
    app.browser.tracks.filter.apply(titles.len(), |i| titles[i].contains(needle));
}

#[tokio::test]
async fn browser_visual_range_adds_in_order() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 30);
    app.queue.items = vec![track(100)];
    // matches: t2, t20..=t29
    filter_browser_tracks(&mut app, "Starfall! 2");

    keys(&mut app, "vjj");
    assert_eq!(app.visual.map(|v| v.anchor), Some(2));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t100", "t2", "t20", "t21"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_range_inserts_after_the_current_track() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 6);
    app.queue.items = (10..13).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.browser.tracks.state.select(Some(1));

    keys(&mut app, "vjji");
    assert_eq!(queue_keys(&app), ["t10", "t1", "t2", "t3", "t11", "t12"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_range_enter_replaces_queue_with_range() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.queue.items = (10..13).map(track).collect();
    app.browser.tracks.state.select(Some(5));

    keys(&mut app, "vk");
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t4", "t5"]);
    assert_eq!(app.current, Some(0));
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_anchor_dropped_on_new_browser_tracks() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.handle_msg(key('v'));
    assert!(app.visual.is_some());

    app.browser.tracks_for = Some("al2".into());
    app.handle_msg(Msg::TracksLoaded {
        album_key: "al2".into(),
        tracks: (20..25).map(track).collect(),
    });
    assert!(app.visual.is_none());
    app.handle_msg(key('j'));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t21"], "only the row under the cursor is added");
}

#[tokio::test]
async fn visual_range_adds_playlist_tracks() {
    let mut app = new_app().await;
    on_playlist(&mut app, 0);
    app.playlists.tracks_for = Some("pl0".into());
    app.playlists.tracks.items = (0..6).map(track).collect();
    app.playlists.tracks.state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;

    keys(&mut app, "vj");
    app.handle_msg(key_code(KeyCode::Esc));
    assert!(app.visual.is_none());
    assert_eq!(app.tab, Tab::Playlists, "Esc in visual mode does not leave the tab");

    app.handle_msg(key('v'));
    keys(&mut app, "ja");
    assert_eq!(queue_keys(&app), ["t2", "t3"]);
}

#[tokio::test]
async fn browser_visual_mode_shows_range_and_label() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.browser.tracks.state.select(Some(1));
    app.stopped = true;
    app.handle_msg(key('v'));
    keys(&mut app, "jj");

    for w in [120u16, 60] {
        let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let text = flatten(&terminal);
        assert!(text.contains("-- VISUAL --"), "{w}: label missing");
        let (x, inside) = locate(&terminal, "Starfall! 2").expect("row");
        let (_, outside) = locate(&terminal, "Starfall! 5").expect("row");
        let buf = terminal.backend().buffer();
        assert_ne!(buf.cell((x, inside)).unwrap().bg, buf.cell((x, outside)).unwrap().bg, "{w}");
    }
}

/// Six tracks t0..t5 as the Search tab's album view, the cursor on row 1.
fn on_search_detail(app: &mut App) {
    app.tab = Tab::Search;
    app.search.focused = false;
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(1));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "al1".into(),
        label: "Evening Satellite".into(),
        tracks: (0..6).map(track).collect(),
        state,
        matches: None,
        radio: None,
    });
}

#[tokio::test]
async fn visual_range_adds_search_detail_tracks() {
    let mut app = new_app().await;
    on_search_detail(&mut app);

    keys(&mut app, "vjj");
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_anchor_dropped_on_new_search_detail() {
    let mut app = new_app().await;
    on_search_detail(&mut app);
    app.handle_msg(key('v'));
    assert!(app.visual.is_some());

    app.search.list_request =
        Some(ListRequest { key: "al2".into(), kind: ListKind::Album, origin: Tab::Search });
    app.handle_msg(Msg::SearchAlbumLoaded {
        key: "al2".into(),
        label: "Other".into(),
        tracks: (20..25).map(track).collect(),
    });
    assert!(app.visual.is_none());
    app.handle_msg(key('j'));
    app.handle_msg(key('a'));
    assert_eq!(queue_keys(&app), ["t21"], "only the row under the cursor is added");
}

#[tokio::test]
async fn search_detail_visual_mode_shows_range_and_label() {
    let mut app = new_app().await;
    on_search_detail(&mut app);
    app.stopped = true;
    keys(&mut app, "vjj");

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    assert!(flatten(&terminal).contains("-- VISUAL --"));
    let (x, inside) = locate(&terminal, "Starfall! 2").expect("row");
    let (_, outside) = locate(&terminal, "Starfall! 5").expect("row");
    let buf = terminal.backend().buffer();
    assert_ne!(buf.cell((x, inside)).unwrap().bg, buf.cell((x, outside)).unwrap().bg);
}

#[tokio::test]
async fn g_letter_in_visual_jumps_not_adds() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    keys(&mut app, "vjga");
    assert!(app.queue.items.is_empty());
    assert!(app.visual.is_some());
}
