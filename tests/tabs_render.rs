//! Render and app tests: every tab draws at several sizes, and the key, queue
//! and layout rules hold. mpv runs for real.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use image::{DynamicImage, RgbaImage};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::widgets::TableState;
use ratatui::Terminal;
use shmc::config::{KeyTable, MpvConfig, UiConfig};
use shmc::playback::{mpv, MpvEvent};
use shmc::plex::models::{Album, Artist, AudioInfo, HitKind, Playlist, SearchHit, Track};
use shmc::ui::app::{
    App, BrowserCol, Confirm, Msg, PlaylistsCol, PlexCmd, PlexHandle, QueueAction, SearchDetail,
    Tab,
};
use shmc::ui::queue_seq::QueueOutcome;
use shmc::ui::scroll::{compute_offset, ScrollId};
use shmc::ui::theme;
use shmc::ui::views;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

fn track(i: usize) -> Track {
    Track {
        rating_key: format!("t{i}"),
        title: format!("Starfall! {i}"),
        artist: "Northwind Sounds".into(),
        album: "Evening Satellite".into(),
        track_no: Some(i as u32),
        year: None,
        duration_ms: Some(257_293), // 4:17
        parent_rating_key: Some("al1".into()),
        grandparent_rating_key: Some("ar1".into()),
        thumb: None,
        part_key: Some(format!("/library/parts/{i}/file.mp3")),
        container: Some("mp3".into()),
        audio: AudioInfo::default(),
        playlist_item_id: None,
    }
}

fn playlist(i: usize) -> Playlist {
    Playlist {
        rating_key: format!("p{i}"),
        title: format!("Night Mix {i:02}"),
        smart: false,
        item_count: Some(12),
        duration_ms: Some(3_100_000),
        thumb: None,
        summary: None,
    }
}

/// A playlist with an explicit item total (previews are capped at `ui.preview_tracks`).
fn listed_playlist(key: &str, title: &str, item_count: u32) -> Playlist {
    Playlist {
        rating_key: key.into(),
        title: title.into(),
        smart: false,
        item_count: Some(item_count),
        duration_ms: None,
        thumb: None,
        summary: None,
    }
}

/// ASCII glyphs keep text assertions independent of the font.
fn ascii_ui() -> UiConfig {
    UiConfig { icons: "ascii".into(), ..Default::default() }
}

fn flatten(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            if let Some(cell) = buf.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::draw(f, app)).unwrap();
    flatten(&terminal)
}

/// Each test spawns a real mpv; serialize them. The guard lives with the app.
static TAB_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestApp {
    app: App,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for TestApp {
    type Target = App;
    fn deref(&self) -> &App {
        &self.app
    }
}

impl std::ops::DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.app
    }
}

async fn new_app() -> TestApp {
    new_app_with_plex().await.0
}

/// Same app, with the Plex channel kept so the test can see what the UI sent.
async fn new_app_with_plex() -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    new_app_with_ui(ascii_ui()).await
}

async fn new_app_with_ui(ui: UiConfig) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    new_app_with_ui_and_keys(ui, &KeyTable::default()).await
}

async fn new_app_with_keys(keys: &KeyTable) -> TestApp {
    new_app_with_ui_and_keys(ascii_ui(), keys).await.0
}

fn key_table(text: &str) -> KeyTable {
    toml::from_str::<shmc::config::Config>(text).unwrap().keys
}

async fn new_app_with_ui_and_keys(
    ui: UiConfig,
    keys: &KeyTable,
) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    let guard = TAB_TEST_LOCK.lock().await;
    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (mpv_handle, _events) = mpv::spawn(&cfg).await.expect("spawn mpv");
    let (plex_tx, plex_rx) = unbounded_channel();
    let (msg_tx, _msg_rx) = unbounded_channel();
    let app = App::new(
        "http://127.0.0.1:1".into(),
        "TOK".into(),
        PlexHandle { tx: plex_tx },
        mpv_handle,
        63.0,
        None,   // no image picker -> art placeholder path
        false,  // art disabled
        [0, 0], // no art size cap
        msg_tx,
        ui,
        keys,
    );
    (TestApp { app, _guard: guard }, plex_rx)
}

fn key(c: char) -> Msg {
    Msg::Key(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)))
}

fn key_code(code: KeyCode) -> Msg {
    Msg::Key(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn enter() -> Msg {
    key_code(KeyCode::Enter)
}

fn end_file_error() -> Msg {
    Msg::Mpv(MpvEvent::EndFile { reason: "error".into() })
}

fn lines_with(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

/// Columns of every `│` in a line.
fn bar_columns(line: &str) -> Vec<usize> {
    line.chars().enumerate().filter(|(_, c)| *c == '│').map(|(i, _)| i).collect()
}

/// Column where `needle` starts in a frame (ASCII needles only).
fn column_of(frame: &str, needle: &str) -> usize {
    for line in frame.lines() {
        if let Some(x) = line.find(needle) {
            return x;
        }
    }
    panic!("{needle:?} not found in the frame\n{frame}");
}

#[tokio::test]
async fn all_tabs_render_at_several_sizes() {
    let mut app = new_app().await;
    app.art_enabled = true;

    app.queue = (0..50).map(track).collect();
    app.queue_state.select(Some(7));
    app.current = Some(7);
    app.stopped = false; // App::new starts stopped; simulate an active player
    app.duration = 257.0;
    app.position = 42.0;
    app.volume = 63.0;
    app.loop_playlist = true;
    app.random = true;
    app.bitrate_bps = 320_000.0;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 20]; // full bars -> bottom row is '█'

    app.browser.artists = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Northwind Sounds {i:02}"),
            thumb: None,
        })
        .collect();
    app.browser.artists_state.select(Some(3));
    app.browser.albums = (0..25)
        .map(|i| Album {
            rating_key: format!("al{i}"),
            title: format!("Evening Satellite {i:02}"),
            artist: "Northwind Sounds".into(),
            year: Some(2019),
            thumb: None,
        })
        .collect();
    app.browser.albums_state.select(Some(2));
    app.browser.tracks = (0..60).map(track).collect();
    app.browser.tracks_state.select(Some(11));
    app.browser.focus = BrowserCol::Artists;
    app.browser.albums_for = Some("al2".into());
    app.browser.tracks_for = Some("al2".into());

    app.playlists.playlists = (0..25)
        .map(|i| Playlist {
            rating_key: format!("p{i}"),
            title: format!("Night Mix {i:02}"),
            smart: i == 3, // one smart playlist -> ✦ marker
            item_count: Some(12),
            duration_ms: Some(3_100_000),
            thumb: None,
            summary: None,
        })
        .collect();
    app.playlists.state.select(Some(3));
    app.playlists.items = (0..12).map(track).collect();
    app.playlists.items_state.select(Some(5));
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.items_for = Some("p3".into());

    let c: shmc::plex::models::MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/hubs_search_starfall.xml")).unwrap();
    app.search.hits = c.into_search_hits();
    app.search.state.select(Some(0));

    app.help_state.select(Some(0));

    for (w, h) in [(120u16, 40u16), (120, 32), (80, 24), (60, 16), (46, 12), (30, 10)] {
        for tab in [Tab::Queue, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help] {
            app.tab = tab;
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| views::draw(f, &mut app))
                .unwrap_or_else(|e| panic!("draw {tab:?} at {w}x{h} failed: {e}"));
        }
    }

    app.tab = Tab::Queue;
    let q = render(&mut app, 120, 40);
    for want in [
        "[Playing]",   // state badge (secondary)
        "Volume: ",    // volume zone
        " 63%",        // right-aligned percent
        "0:42 / 4:17", // elapsed / total
        "(320 kbps)",  // cheaply-known bitrate
        "rep",         // repeat icon on (ascii set)
        "shf",         // shuffle icon on
        "one",         // single icon
        "con",         // consume icon
        "│",           // thin header-zone divider
        "Queue",       // tab bar
        "Browser",
        "Playlists",
        "Len",   // header spelled out
        "Title", // queue table is # | Title | Artist | Album | Length
        "Artist",
        "Starfall! 7",       // rows incl. the current one
        "Evening Satellite", // album column
        "4:17",              // right-aligned Length
        "No cover",          // art placeholder (no picker in tests)
        "▼",                 // scrollbar (50 rows > visible)
    ] {
        assert!(q.contains(want), "Queue frame must contain {want:?}\nframe:\n{q}");
    }
    assert!(q.contains('█'), "visualizer must render full blocks\nframe:\n{q}");

    app.tab = Tab::Browser;
    let b = render(&mut app, 120, 40);
    for want in ["Northwind Sounds 03", "Evening Satellite 02", "4:17", "│"] {
        assert!(b.contains(want), "Browser frame must contain {want:?}\nframe:\n{b}");
    }
    assert!(
        !b.contains(" - Northwind Sounds"),
        "artist suffix must be hidden when it matches the parent\nframe:\n{b}"
    );
    app.browser.focus = BrowserCol::Tracks;
    let b2 = render(&mut app, 120, 40);
    assert!(b2.contains("Starfall! 11"), "tracks column must show rows\nframe:\n{b2}");

    app.tab = Tab::Playlists;
    let p = render(&mut app, 120, 40);
    for want in ["Night Mix 03", "✦", "Night Mix 00"] {
        assert!(p.contains(want), "Playlists frame must contain {want:?}\nframe:\n{p}");
    }
    app.playlists.focus = PlaylistsCol::Items;
    let p2 = render(&mut app, 120, 40);
    for want in ["Starfall! 5", "Len", "Title", "Artist", "Album", "4:17"] {
        assert!(p2.contains(want), "items pane must contain {want:?}\nframe:\n{p2}");
    }

    app.tab = Tab::Search;
    let s = render(&mut app, 120, 40);
    for want in [
        "Starfall!",
        "[Album]",
        "Type / Artist",
        "Len",
        "Featured Acts",     // track credit from originalTitle
        "Northwind Sounds",  // album artist on the album row
        "Evening Satellite", // album column of the track row
        "4:17",              // right-aligned Len (257293 ms)
    ] {
        assert!(s.contains(want), "Search frame must contain {want:?}\nframe:\n{s}");
    }
    let mut ds = TableState::default();
    ds.select(Some(0));
    app.search.detail = Some(SearchDetail {
        key: "al1".into(),
        label: "Evening Satellite".into(),
        tracks: (0..12).map(track).collect(),
        state: ds,
    });
    let sd = render(&mut app, 120, 40);
    assert!(
        sd.contains("Evening Satellite (12 tracks)"),
        "search detail must show label + count\nframe:\n{sd}"
    );
    app.search.detail = None;

    app.tab = Tab::Help;
    let hlp = render(&mut app, 120, 40);
    assert!(hlp.contains("Global"), "Help frame must contain \"Global\"\nframe:\n{hlp}");
    let help_len = app.keymap.help_lines().len();
    app.help_state.select(Some(help_len - 1));
    let hlp_commands = render(&mut app, 120, 40);
    for want in [
        ":vol N",
        ":play",
        ":stop",
        ":seek N",
        ":shuffle",
        ":repeat",
        ":single",
        ":consume",
        ":reshuffle",
        ":queue",
        ":browser",
        ":playlists",
        ":search",
        ":help",
        ":playing",
        ":clear",
        ":save NAME",
    ] {
        assert!(
            hlp_commands.contains(want),
            "Help command list must contain {want:?}\nframe:\n{hlp_commands}"
        );
    }
    app.help_state.select(Some(0));

    // bottom row: command input > filter typing > applied filter > status > progress
    app.tab = Tab::Queue;

    app.command.typing = true;
    app.command.text = "vol 50".into();
    app.status = "Added: Starfall! 0".into();
    let c1 = render(&mut app, 120, 40);
    assert!(c1.contains(": vol 50█"), "command input must render\nframe:\n{c1}");
    assert!(!c1.contains("Added:"), "status must yield to command input\nframe:\n{c1}");
    app.command.typing = false;
    app.command.text.clear();
    app.status.clear();

    app.queue_filter.typing = true;
    app.queue_filter.text = "sky".into();
    let titles: Vec<String> = app.queue.iter().map(|t| t.title.clone()).collect();
    app.queue_filter.apply(titles.len(), |i| titles[i].contains("Starfall! 1"));
    let c2 = render(&mut app, 120, 40);
    assert!(c2.contains("/ sky█"), "filter input must render\nframe:\n{c2}");
    assert!(c2.contains("Queue 11/50"), "filter counter must render\nframe:\n{c2}");

    app.queue_filter.typing = false;
    let c3 = render(&mut app, 120, 40);
    assert!(c3.contains("filter: sky (11/50)"), "filter indicator must show counts\nframe:\n{c3}");
    app.queue_filter.clear();

    app.status = "Added album: Evening Satellite (12 tracks)".into();
    let c4 = render(&mut app, 120, 40);
    assert!(c4.contains("Added album: Evening Satellite (12 tracks)"), "frame:\n{c4}");
    app.status.clear();

    let c5 = render(&mut app, 120, 40);
    assert!(c5.contains('>'), "progress bar must show the play glyph\nframe:\n{c5}");

    app.paused = true;
    let c6 = render(&mut app, 120, 40);
    assert!(c6.contains("[Paused]"), "frame:\n{c6}");
    app.paused = false;
    app.stopped = true;
    let c7 = render(&mut app, 120, 40);
    assert!(c7.contains("[Stopped]"), "frame:\n{c7}");
}

#[tokio::test]
async fn review_verification_120x40() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.queue = (0..30).map(track).collect();
    app.queue_state.select(Some(3));
    app.current = Some(3);
    app.stopped = false;
    app.duration = 3725.0; // 1:02:05 -> h:mm:ss
    app.position = 42.0;
    app.cava_alive = true;
    app.cava_frame = vec![220u8; 20];
    app.tab = Tab::Queue;

    app.loop_playlist = true;
    app.random = true;
    app.loop_file = true;
    app.consume = true;
    let latin = render(&mut app, 120, 40);
    let header: String = latin.lines().take(4).collect::<Vec<_>>().join("\n");
    for want in ["rep", "shf", "one", "con", "│", "Starfall! 3", "Northwind Sounds"] {
        assert!(header.contains(want), "header must contain {want:?}\n{header}");
    }
    app.queue[3].title = "日本語の曲".into();
    let cjk = render(&mut app, 120, 40);
    let header_nospace: String = cjk.lines().take(4).collect::<String>().replace(' ', "");
    assert!(
        header_nospace.contains("日本語の曲"),
        "CJK title must render in the center zone\nframe:\n{cjk}"
    );
    let row1 = cjk.lines().nth(1).unwrap_or("").replace(' ', "");
    let row2 = cjk.lines().nth(2).unwrap_or("");
    assert!(row1.contains("日本語の曲"), "row1 title\n{row1}");
    assert!(row2.contains("Northwind Sounds"), "row2 credit\n{row2}");

    app.tab = Tab::Browser;
    app.browser.tracks_for = Some("al0".into());
    app.browser.albums_for = Some("al0".into());
    app.browser.albums = vec![Album {
        rating_key: "al0".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        year: Some(2019),
        thumb: None,
    }];
    app.browser.artists = vec![Artist {
        rating_key: "a0".into(),
        title: "A Very Long Artist Name That Will Not Fit In One Column At All".into(),
        thumb: None,
    }];
    app.browser.artists_state.select(Some(0));
    app.browser.albums_state.select(Some(0));
    app.browser.tracks = (0..9).map(track).collect();
    app.browser.tracks_state.select(Some(0));
    app.browser.focus = BrowserCol::Artists;
    let br = render(&mut app, 120, 40);
    assert!(br.contains('…'), "long artist name must be truncated with …\n{br}");
    assert!(br.contains('│'), "browser must draw column dividers\n{br}");
    assert!(
        !br.contains(" - Northwind Sounds"),
        "album artist suffix hidden when equal to parent\n{br}"
    );

    app.tab = Tab::Playlists;
    assert_eq!(shmc::ui::theme::digits(9), 1);
    assert_eq!(shmc::ui::theme::digits(10), 2);
    assert_eq!(shmc::ui::theme::digits(100), 3);
    app.playlists.playlists = vec![Playlist {
        rating_key: "p0".into(),
        title: "Night Mix".into(),
        smart: false,
        item_count: Some(100),
        duration_ms: None,
        thumb: None,
        summary: None,
    }];
    app.playlists.state.select(Some(0));
    app.playlists.items = (0..100).map(track).collect();
    app.playlists.items_for = Some("p0".into());
    app.playlists.focus = PlaylistsCol::Items;
    app.playlists.items_state.select(Some(99));
    let pl = render(&mut app, 120, 40);
    for want in ["Len", "Title", "Artist", "Album", "100"] {
        assert!(pl.contains(want), "playlists items must contain {want:?}\n{pl}");
    }
    assert!(pl.contains("────"), "playlists items need a rule under the header\n{pl}");

    app.browser.tracks = (0..9)
        .map(|i| Track {
            title: [
                "Cyan Sea", "Crimson", "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta",
            ][i]
                .into(),
            artist: "X".into(),
            album: "Y".into(),
            ..track(i)
        })
        .collect();
    app.browser.tracks_state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;
    app.browser_filters[2].text = "c".into();
    app.browser_filters[2].typing = true;
    let tracks = app.browser.tracks.clone();
    app.browser_filters[2].apply(tracks.len(), |i| tracks[i].title.to_lowercase().contains("c"));
    let f1 = render(&mut app, 120, 40);
    assert!(f1.contains("Tracks 2/9"), "live filter counter must read Tracks 2/9\n{f1}");
    assert!(f1.contains("Cyan Sea") && f1.contains("Crimson"), "matching rows shown\n{f1}");
    assert!(!f1.contains("Alpha"), "non-matching rows hidden\n{f1}");
    app.browser_filters[2].typing = false;
    let f2 = render(&mut app, 120, 40);
    assert!(
        f2.contains("filter: c (2/9)"),
        "after Enter the indicator moves to the column top\n{f2}"
    );
    app.browser_filters[2].text = "cx".into();
    app.browser_filters[2].apply(tracks.len(), |i| tracks[i].title.to_lowercase().contains("cx"));
    let f3 = render(&mut app, 120, 40);
    assert!(f3.contains("No matches"), "zero matches must show No matches\n{f3}");
    app.browser_filters[2].clear();

    app.tab = Tab::Queue;
    app.queue[3].title = "Starfall! 3".into();
    app.stopped = false;
    app.paused = false;
    let play = render(&mut app, 120, 40);
    assert!(play.contains('>'), "play glyph\n{play}");
    app.paused = true;
    let pause = render(&mut app, 120, 40);
    assert!(pause.contains("||"), "pause glyph\n{pause}");
    app.paused = false;
    app.stopped = true;
    let stop = render(&mut app, 120, 40);
    assert!(stop.contains('■'), "stop glyph\n{stop}");
    for frame in [&play, &pause, &stop] {
        assert!(!frame.contains('\u{23f8}'), "no U+23F8 pause glyph");
        assert!(!frame.contains('\u{fe0f}'), "no variation selector");
    }
    app.stopped = false;

    app.current = Some(1);
    app.queue_state.select(Some(1));
    let current_key = app.queue[1].rating_key.clone();
    let original_keys: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();
    app.random = false;
    app.handle_msg(key('z'));
    assert!(app.random, "z must turn shuffle on");
    assert_eq!(app.queue[0].rating_key, current_key, "shuffle must put the current track first");
    app.handle_msg(key('z'));
    assert!(!app.random, "z must turn shuffle off");
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(),
        original_keys,
        "turning shuffle off must restore the original order"
    );
    app.handle_msg(key('Z'));
    assert!(app.random, "Z must enable/reshuffle");

    app.tab = Tab::Queue;
    let row_of = |frame: &str| frame.lines().position(|l| l.contains("No cover"));
    app.current = Some(1);
    let cov1 = render(&mut app, 120, 40);
    app.current = Some(28);
    let cov2 = render(&mut app, 120, 40);
    assert_eq!(
        row_of(&cov1),
        row_of(&cov2),
        "cover placeholder must stay in the same position across songs"
    );
    assert!(row_of(&cov1).is_some(), "cover placeholder row missing\n{cov1}");

    // The visualizer keeps drawing while paused, in about half the column.
    app.current = Some(3);
    app.paused = true;
    let h = render(&mut app, 120, 40);
    assert!(h.contains("Len"), "queue header must say Len\n{h}");
    assert!(
        !h.contains("visualizer off") && !h.contains("cava is not running"),
        "a live cava must not be reported as off\n{h}"
    );
    assert!(h.contains('█') || h.contains('▇'), "visualizer must draw bars while paused\n{h}");
    let bar_glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let viz_rows = h.lines().skip(7).filter(|l| l.chars().any(|c| bar_glyphs.contains(&c))).count();
    assert!(
        viz_rows >= 10,
        "visualizer should be tall (about half the column), got {viz_rows} rows\n{h}"
    );
    let qrows = lines_with(&h, "Evening Satellite");
    assert!(qrows >= 8, "queue must keep rows, got {qrows}\n{h}");
}

/// (x, y) of the first cell of `needle` anywhere in the rendered frame.
fn locate(terminal: &Terminal<TestBackend>, needle: &str) -> Option<(u16, u16)> {
    let buf = terminal.backend().buffer();
    let (w, h) = (buf.area.width as usize, buf.area.height as usize);
    // one char per cell: box-drawing and bar glyphs are multi-byte
    let mut flat: Vec<char> = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            flat.push(
                buf.cell((x as u16, y as u16))
                    .and_then(|c| c.symbol().chars().next())
                    .unwrap_or(' '),
            );
        }
    }
    let want: Vec<char> = needle.chars().collect();
    let pos = flat.windows(want.len()).position(|w| w == want)?;
    Some(((pos % w) as u16, (pos / w) as u16))
}

/// Foreground colour of the first cell of `needle`.
fn fg_at(terminal: &Terminal<TestBackend>, needle: &str) -> Option<Color> {
    let (x, y) = locate(terminal, needle)?;
    terminal.backend().buffer().cell((x, y))?.style().fg
}

/// Column (not byte offset) of `needle` in `line`.
fn char_x(line: &str, needle: &str) -> Option<usize> {
    let want: Vec<char> = needle.chars().collect();
    let hay: Vec<char> = line.chars().collect();
    hay.windows(want.len()).position(|w| w == want.as_slice())
}

/// With no `#` column, rows still line up under the header and the art box gets
/// the full pane width (46 cells at 120x40).
#[tokio::test]
async fn queue_drops_row_numbers_for_art() {
    let mut app = new_app().await;
    app.queue = (0..50).map(track).collect();
    app.queue_state.select(Some(0));
    app.current = Some(0);
    app.stopped = false;
    app.art_enabled = true;
    app.art.img = Some(Arc::new(image::DynamicImage::new_rgba8(4, 4)));
    app.tab = Tab::Queue;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let frame = flatten(&terminal);

    let hy = frame
        .lines()
        .position(|l| l.contains("Title") && l.contains("Len"))
        .expect("queue table header");
    let header = frame.lines().nth(hy).unwrap();
    assert!(!header.contains('#'), "the queue's row-number column is gone\n{header}");

    let row = frame.lines().nth(hy + 2).expect("first queue row");
    let title_x = char_x(header, "Title").expect("Title header");
    assert_eq!(
        char_x(row, "Starfall! 0"),
        Some(title_x),
        "rows must line up under the header\n{header}\n{row}"
    );
    let len_end = char_x(header, "Len").expect("Length header") + "Len".len();
    assert_eq!(
        char_x(row, "4:17").map(|x| x + "4:17".len()),
        Some(len_end),
        "the right edge of the table must line up too\n{header}\n{row}"
    );

    // borders on the "No cover" line: window edge, box left, box right, art/queue divider
    let cover_line = frame.lines().find(|l| l.contains("No cover")).expect("cover box");
    let bars = bar_columns(cover_line);
    assert!(bars.len() >= 4, "cover line must show the box borders\n{cover_line}");
    assert_eq!(bars[2] - bars[1] + 1, 46, "the art box must be 46 cells wide\n{cover_line}");
    let left_gap = bars[1] - bars[0];
    let right_gap = bars[3] - bars[2];
    assert!(
        (left_gap as isize - right_gap as isize).abs() <= 1,
        "art must be centred: left gap {left_gap}, right gap {right_gap}\n{cover_line}"
    );
}

/// Up on the first row jumps to the last, and down on the last to the first.
#[tokio::test]
async fn arrow_keys_wrap_at_list_ends() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.queue_state.selected(), Some(5), "up from the top wraps to the bottom");

    app.handle_msg(key_code(KeyCode::Down));
    assert_eq!(app.queue_state.selected(), Some(0), "down from the bottom wraps to the top");

    app.queue_state.select(Some(3));
    app.handle_msg(key('j'));
    assert_eq!(app.queue_state.selected(), Some(4), "j moves down as usual");
    app.queue_state.select(Some(0));
    app.handle_msg(key('k'));
    assert_eq!(app.queue_state.selected(), Some(5), "k wraps too");
}

/// Under a filter the cursor is a position in the filtered view, not a list
/// index: Enter must open the matching artist, not the one at row 0.
#[tokio::test]
async fn enter_on_filtered_artist_opens_it() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Northwind Sounds {i:02}"),
            thumb: None,
        })
        .collect();

    // '/sounds 07' leaves one row: artist a7 at filtered position 0
    app.handle_msg(key('/'));
    for c in "sounds 07".chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
    assert_eq!(app.browser_filters[0].text, "sounds 07");
    assert_eq!(
        app.browser.artists_state.selected(),
        Some(0),
        "the cursor is on the filtered row 0"
    );

    app.handle_msg(enter());

    assert_eq!(app.browser.focus, BrowserCol::Albums, "Enter must drill into the albums");
    assert_eq!(
        app.browser.albums_for.as_deref(),
        Some("a7"),
        "the artist that matched is the one that opens"
    );
    assert!(app.browser_filters[0].text.is_empty(), "navigating on clears the filter");
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a7", "the worker is asked for a7's albums"),
        other => panic!("expected LoadAlbums(a7), got {other:?}"),
    }

    // unfiltered Enter still works
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists_state.select(Some(3));
    app.handle_msg(enter());
    assert_eq!(app.browser.albums_for.as_deref(), Some("a3"));
}

/// Right on an artist opens its albums, without refetching ones already on
/// screen, and opens the new artist's after moving.
#[tokio::test]
async fn right_on_an_artist_opens_its_albums() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists = (0..3)
        .map(|i| Artist { rating_key: format!("a{i}"), title: format!("Artist {i}"), thumb: None })
        .collect();
    app.browser.artists_state.select(Some(0));

    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert_eq!(app.browser.albums_for.as_deref(), Some("a0"));
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a0"),
        other => panic!("expected LoadAlbums(a0), got {other:?}"),
    }

    app.handle_msg(key_code(KeyCode::Left));
    assert_eq!(app.browser.focus, BrowserCol::Artists);
    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert!(plex_rx.try_recv().is_err(), "the albums on screen are not fetched again");

    app.handle_msg(key_code(KeyCode::Left));
    app.handle_msg(key_code(KeyCode::Down));
    app.handle_msg(key_code(KeyCode::Right));
    assert_eq!(app.browser.focus, BrowserCol::Albums);
    assert_eq!(app.browser.albums_for.as_deref(), Some("a1"));
    match plex_rx.try_recv() {
        Ok(PlexCmd::LoadAlbums(k)) => assert_eq!(k, "a1"),
        other => panic!("expected LoadAlbums(a1), got {other:?}"),
    }
}

/// Column headers share the inactive-tab colour.
#[tokio::test]
async fn queue_headers_share_the_tab_bar_colour() {
    let mut app = new_app().await;
    app.queue = (0..20).map(track).collect();
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();

    let tab = fg_at(&terminal, "Browser").expect("inactive tab cell");
    assert_eq!(Some(tab), theme::text().fg, "an inactive tab should be TEXT");
    for label in ["Title", "Artist", "Album", "Len"] {
        assert_eq!(
            fg_at(&terminal, label).expect("header cell"),
            tab,
            "{label} must match the tab bar colour"
        );
    }
}

/// Enter and `a` say what they queued or started.
#[tokio::test]
async fn enter_reports_what_it_added() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;

    app.replace_queue_and_play(vec![track(0)], Confirm::Track { title: "Starfall! 0".into() });
    assert!(!app.status_error);
    assert_eq!(app.status, "Playing: Starfall! 0");
    assert_eq!(app.queue.len(), 1);

    app.add_tracks(vec![track(7)], Confirm::Track { title: "Starfall! 7".into() });
    assert!(!app.status_error);
    assert_eq!(app.status, "Added: Starfall! 7");
    assert_eq!(app.queue.len(), 2);

    app.add_tracks(vec![track(1), track(2)], Confirm::FromList { name: "Road Trip".into() });
    assert_eq!(app.status, "Added 2 tracks from Road Trip");

    app.add_tracks((10..52).map(track).collect(), Confirm::Artist { name: "Artist".into() });
    assert!(app.status.contains("42"), "{}", app.status);
}

/// The next key press drops the status, but not one that press sets itself.
#[tokio::test]
async fn status_clears_on_next_key() {
    let mut app = new_app().await;
    app.queue = (0..10).map(track).collect();
    app.queue_state.select(Some(0));

    // disarm queue follow first; that has its own status
    app.handle_msg(key('j'));
    app.status = "Added: Starfall! 4".into();
    app.handle_msg(key('j')); // plain navigation
    assert!(app.status.is_empty(), "navigation must dismiss the status");

    app.status = "Added: Starfall! 4".into();
    app.handle_msg(key('D')); // an action that posts a message of its own
    assert!(app.status.contains("clear the queue"), "the new message must survive: {}", app.status);
}

/// `D` stops mpv as well as emptying the queue, and drops the cover.
#[tokio::test]
async fn clear_queue_stops_and_drops_the_cover() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.queue_state.select(Some(2));
    app.current = Some(2);
    app.stopped = false;
    app.art.img = Some(Arc::new(image::DynamicImage::new_rgba8(4, 4)));

    app.clear_queue();

    assert!(app.queue.is_empty(), "the queue must be empty");
    assert_eq!(app.current, None, "nothing is playing any more");
    assert!(app.stopped, "clearing the queue must stop playback");
    assert!(app.art.img.is_none(), "the cover must go with the track");
    assert_eq!(app.position, 0.0, "the progress must reset");
    assert_eq!(app.status, "queue cleared");
}

/// `D` and `:clear` ask first: only `y` (or a second `D`) clears; an empty queue asks nothing.
#[tokio::test]
async fn clearing_the_queue_asks_first() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(key('D'));
    assert_eq!(app.queue.len(), 6, "nothing is cleared before the answer");
    assert!(app.status.contains("6 tracks"), "the question says how many: {}", app.status);
    app.handle_msg(key('x'));
    assert_eq!(app.queue.len(), 6, "another key cancels");
    assert_eq!(app.current, Some(2));

    run_cmd(&mut app, "clear");
    assert_eq!(app.queue.len(), 6, ":clear asks too");
    app.handle_msg(key('y'));
    assert!(app.queue.is_empty());
    assert!(app.stopped);

    app.queue = (0..4).map(track).collect();
    app.handle_msg(key('D'));
    app.handle_msg(key('D'));
    assert!(app.queue.is_empty(), "a second D confirms");

    app.handle_msg(key('D'));
    app.handle_msg(key('2'));
    assert_eq!(app.tab, Tab::Browser, "an empty queue asks nothing");
}

/// Key repeats of `D` must not answer the question the first press asked.
#[tokio::test]
async fn held_d_asks_but_does_not_clear() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.handle_msg(key('D'));
    for _ in 0..5 {
        app.handle_msg(Msg::Key(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('D'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))));
    }
    assert_eq!(app.queue.len(), 5, "repeats do not confirm");
    assert!(app.status.contains("5 tracks"), "the question stays up: {}", app.status);
    app.handle_msg(key('D'));
    assert!(app.queue.is_empty(), "a fresh press does");
}

fn ctrl(c: char) -> Msg {
    Msg::Key(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)))
}

/// Ctrl-C quits even while typing, and a Ctrl-letter is never text.
#[tokio::test]
async fn ctrl_c_quits_while_typing() {
    for mode in ["search", "filter", "command"] {
        let mut app = new_app().await;
        app.queue = (0..3).map(track).collect();
        match mode {
            "search" => {
                app.tab = Tab::Search;
                app.search.focused = true;
            }
            "filter" => app.handle_msg(key('/')),
            _ => app.handle_msg(key(':')),
        }
        app.handle_msg(key('a'));
        app.handle_msg(ctrl('x'));
        let typed = match mode {
            "search" => app.search.input.clone(),
            "filter" => app.queue_filter.text.clone(),
            _ => app.command.text.clone(),
        };
        assert_eq!(typed, "a", "{mode}: Ctrl-X must not insert text");
        assert!(!app.should_quit, "{mode}");
        app.handle_msg(ctrl('c'));
        assert!(app.should_quit, "{mode}: Ctrl-C must quit");
    }
}

/// `nan` and `inf` parse as floats; volume must never become NaN.
#[tokio::test]
async fn vol_and_seek_reject_non_finite_numbers() {
    let mut app = new_app().await;
    for cmd in ["vol nan", "vol inf", "vol +inf", "seek nan", "seek -inf"] {
        run_cmd(&mut app, cmd);
        assert!(app.volume.is_finite(), "{cmd}");
        assert_eq!(app.volume, 63.0, "{cmd}");
        assert!(app.status_error && app.status.contains("usage"), "{cmd}: {}", app.status);
    }
}

/// Under a filter a consume must keep the cursor on the same item.
#[tokio::test]
async fn consume_keeps_cursor_on_item_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..30).map(track).collect();
    app.current = Some(5); // finished track is outside the filtered view
    app.stopped = false;
    app.consume = true;
    filter_queue(&mut app, "Starfall! 2");
    // matches: 2, 12, 20..=29; cursor on the last, real index 29
    let cursor = app.queue_filter.cursor_of(29).expect("visible");
    app.queue_state.select(Some(cursor));

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));

    let sel = app.queue_state.selected().expect("a row is highlighted");
    let real = app.queue_filter.real(sel, app.queue.len());
    assert_eq!(app.queue[real].rating_key, "t29", "the cursor moved to another item");
}

/// Show in Browser jumps to a real artist index, so it must drop the Artists filter.
#[tokio::test]
async fn goto_artist_clears_the_artists_filter() {
    let mut app = new_app().await;
    app.browser.artists = (0..10)
        .map(|i| Artist { rating_key: format!("a{i}"), title: format!("Artist {i}"), thumb: None })
        .collect();
    let titles: Vec<String> = app.browser.artists.iter().map(|a| a.title.clone()).collect();
    app.browser_filters[0].text = "Artist 1".into();
    app.browser_filters[0].apply(titles.len(), |i| titles[i].contains("Artist 1"));
    app.queue = vec![Track { grandparent_rating_key: Some("a5".into()), ..track(0) }];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;
    app.handle_msg(key('b'));

    app.handle_msg(Msg::GotoArtist(Artist {
        rating_key: "a5".into(),
        title: "Artist 5".into(),
        thumb: None,
    }));

    assert!(app.browser_filters[0].text.is_empty(), "the filter must not hide the target");
    let sel = app.browser.artists_state.selected().expect("an artist is highlighted");
    assert_eq!(app.browser.artists[sel].rating_key, "a5");
}

/// Error text can carry a stream URL; the status must not show its token.
#[tokio::test]
async fn status_never_shows_a_token() {
    let mut app = new_app().await;
    app.handle_msg(Msg::Error("GET http://h:32400/p?X-Plex-Token=SECRETTOKEN&x=1 failed".into()));
    assert!(!app.status.contains("SECRETTOKEN"), "{}", app.status);
    assert!(app.status.contains("failed"));
}

/// Removing the playing track hands playback to the one that slid in (random
/// mode: any); removing the last track stops.
#[tokio::test]
async fn removing_the_playing_track_hands_playback_on() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.remove_from_queue(2);
    assert_eq!(app.queue.len(), 4);
    assert_eq!(app.current, Some(2), "the track that slid in should take over");
    assert!(!app.stopped, "playback must carry on");

    app.current = Some(3);
    app.remove_from_queue(3);
    assert_eq!(app.queue.len(), 3);
    assert_eq!(app.current, None);
    assert!(app.stopped, "removing the last track stops playback");

    app.queue = (0..5).map(track).collect();
    app.random = true;
    app.current = Some(1);
    app.stopped = false;
    app.remove_from_queue(1);
    assert_eq!(app.queue.len(), 4);
    assert!(!app.stopped, "random mode should hand playback on too");
    assert!(app.current.is_some(), "and it should be playing somewhere");
    app.random = false;
}

/// One unplayable track is skipped; only a burst of failures stops playback
/// and points at `>`.
#[tokio::test]
async fn burst_of_failures_stops_playback() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.queue_state.select(Some(0));
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "one bad file must not stop playback");
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");
    assert_eq!(app.queue.len(), 5, "the queue must survive the failure");
    assert!(app.status_error, "failures are shown as errors");
    assert!(
        app.status.contains("Starfall! 2 failed to play") && app.status.contains("skipping"),
        "a skipped track must be reported: {}",
        app.status
    );

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "a short burst is still not a dead queue");

    app.handle_msg(end_file_error());
    assert!(app.stopped, "a burst of failures must stop playback");
    assert_eq!(app.failed_at, Some(2), "the FIRST failure of the burst sticks");
    assert_eq!(app.queue.len(), 5, "the queue must survive the failure");
    assert!(
        app.status.contains("Starfall! 2 failed to play") && app.status.contains("press >"),
        "the message must name the track and say how to continue: {}",
        app.status
    );

    app.handle_msg(key('>'));
    assert_eq!(app.current, Some(3), "`>` must continue after the broken track");
    assert!(!app.stopped);
    assert_eq!(app.failed_at, None, "the marker is consumed");
    assert!(app.status.is_empty(), "the key press dismisses the error");
}

/// A track that loads ends the error run; a later failure is a skip again.
#[tokio::test]
async fn successful_load_resets_failure_streak() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(1);
    app.stopped = false;

    app.handle_msg(end_file_error());
    app.handle_msg(end_file_error());
    assert!(!app.stopped, "two failures in a row are still a skip");

    app.handle_msg(playlist_pos(2));
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "time-pos".into(),
        value: Some(serde_json::json!(0.5)),
    }));
    assert_eq!(app.failed_at, None, "a good load clears the marker");

    app.handle_msg(end_file_error());
    app.handle_msg(end_file_error());
    assert!(!app.stopped, "the streak must restart after a successful load (2, not 4)");
    app.handle_msg(end_file_error());
    assert!(app.stopped, "three *consecutive* failures stop");
}

/// The position mpv reports after each failure must not end the burst.
#[tokio::test]
async fn failure_run_with_position_events_stops() {
    let mut app = new_app().await;
    app.queue = (0..5).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    for next in 1..=2 {
        app.handle_msg(end_file_error());
        app.handle_msg(playlist_pos(next));
    }
    app.handle_msg(end_file_error());
    assert!(app.stopped, "three failures in a row must stop playback");
    assert_eq!(app.failed_at, Some(0), "the first failed slot is kept");
}

/// With repeat on a failing last track wraps; an all-bad queue still stops.
#[tokio::test]
async fn failing_last_track_wraps_with_repeat() {
    let mut app = new_app().await;
    app.queue = (0..4).map(track).collect();
    app.current = Some(3);
    app.stopped = false;
    app.loop_playlist = true;

    app.handle_msg(end_file_error());
    assert!(!app.stopped, "mpv wraps to the first entry");
    app.handle_msg(playlist_pos(0));
    app.handle_msg(end_file_error());
    app.handle_msg(playlist_pos(1));
    app.handle_msg(end_file_error());
    assert!(app.stopped, "an all-bad queue is bounded by the streak");
}

/// No track follows the failure: the status says so and `>` reports the end.
#[tokio::test]
async fn last_track_failure_says_pick_another() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(2);
    app.stopped = false;

    app.handle_msg(end_file_error());

    assert!(app.stopped);
    assert!(
        app.status.contains("pick another song"),
        "there is no track after this one: {}",
        app.status
    );

    app.handle_msg(key('>'));
    assert!(app.stopped, "there is nothing to advance to");
    assert_eq!(app.status, "end of the queue");
}

/// A resize clears the screen, so the cover must repaint.
#[tokio::test]
async fn resize_repaints_cover() {
    let mut app = new_app().await;
    app.art.shown_rect = Some(ratatui::layout::Rect::new(1, 2, 3, 4));
    app.handle_msg(Msg::Resize);
    assert_eq!(app.art.shown_rect, None);
}

/// The cover is painted once, then its cells are skipped.
#[tokio::test]
async fn album_art_is_painted_once_then_skipped() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.art.img = Some(Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8))));
    app.art.protocol = None;
    app.art.shown_rect = None;

    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let rect = app.art.shown_rect.expect("cover was not painted");

    // check the live frame buffer: the backend only receives the diff
    let mut all_skipped = true;
    terminal
        .draw(|f| {
            views::draw(f, &mut app);
            let buf = f.buffer_mut();
            for y in rect.top()..rect.bottom() {
                for x in rect.left()..rect.right() {
                    if !buf.cell((x, y)).is_some_and(|c| c.skip) {
                        all_skipped = false;
                    }
                }
            }
        })
        .unwrap();
    assert!(all_skipped, "cover cells were not skipped on the second frame");
}

/// Queue tab with a cover, drawn once.
async fn app_with_cover() -> (TestApp, Terminal<TestBackend>) {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.art.img = Some(Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8))));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    assert!(app.art.shown_rect.is_some(), "cover was not painted");
    (app, terminal)
}

/// Draws a frame; whether the cover is in it.
fn cover_drawn(terminal: &mut Terminal<TestBackend>, app: &mut App) -> bool {
    terminal.draw(|f| views::draw(f, app)).unwrap();
    app.art.shown_rect.is_some()
}

/// Back on the Queue tab the cover waits for the dwell; the rest draws first.
#[tokio::test]
async fn cover_follows_queue_tab_after_dwell() {
    let (mut app, mut terminal) = app_with_cover().await;
    app.tab = Tab::Browser;
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    app.tab = Tab::Queue;

    assert!(!cover_drawn(&mut terminal, &mut app), "cover came with the tab");
    assert!(app.art.prepared_rect.is_some(), "canvas is kept for the dwell's end");
    assert!(app.art.hold_until.is_some(), "no dwell started");

    app.art.hold_until = Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
    assert!(cover_drawn(&mut terminal, &mut app), "cover did not appear after the dwell");
    assert!(app.art.hold_until.is_none());
}

/// A deadline from an earlier visit must not let the image through on re-entry.
#[tokio::test]
async fn each_queue_visit_gets_a_fresh_dwell() {
    let (mut app, mut terminal) = app_with_cover().await;
    app.tab = Tab::Browser;
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    app.tab = Tab::Queue;
    assert!(!cover_drawn(&mut terminal, &mut app));
    app.tab = Tab::Search;
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    assert!(app.art.shown_rect.is_none(), "an image was produced while away");
    app.art.hold_until = Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
    app.tab = Tab::Queue;
    assert!(!cover_drawn(&mut terminal, &mut app), "stale deadline let the image through");
}

/// A new cover is not delayed; a resize starts a dwell.
#[tokio::test]
async fn new_cover_is_immediate_and_resize_holds() {
    let (mut app, mut terminal) = app_with_cover().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.handle_msg(Msg::Resize);
    assert!(app.art.hold_until.is_some(), "a resize did not start a dwell");
    // the real dwell is 120 ms; a loaded machine could outlast it before the draw
    app.art.hold_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    assert!(!cover_drawn(&mut terminal, &mut app));

    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    app.handle_msg(Msg::ArtReady { thumb_path: "/library/metadata/al1/thumb".into(), img });
    assert!(cover_drawn(&mut terminal, &mut app), "a new cover waited for the dwell");
}

const AL1_THUMB: &str = "/library/metadata/al1/thumb";

/// Tracks 0-1 share an album, 2 has another, 3 has no cover path; 0 is playing with its cover shown.
async fn app_showing_album_cover(
) -> (TestApp, Terminal<TestBackend>, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.queue = (0..4).map(track).collect();
    app.queue[2].parent_rating_key = Some("al2".into());
    app.queue[3].parent_rating_key = None;
    play_pos(&mut app, 0);
    assert_eq!(thumb_requests(&mut plex), [AL1_THUMB]);
    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    app.handle_msg(Msg::ArtReady { thumb_path: AL1_THUMB.into(), img });
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    assert!(app.art.shown_rect.is_some(), "cover was not painted");
    (app, terminal, plex)
}

fn play_pos(app: &mut App, i: u64) {
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(i)),
    }));
}

fn thumb_requests(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Vec<String> {
    let mut paths = Vec::new();
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::LoadThumb { path } = cmd {
            paths.push(path);
        }
    }
    paths
}

/// The next track of the same album neither requests nor re-encodes the cover.
#[tokio::test]
async fn same_album_track_keeps_the_cover() {
    let (mut app, _terminal, mut plex) = app_showing_album_cover().await;
    let (shown, prepared) = (app.art.shown_rect, app.art.prepared_rect);
    play_pos(&mut app, 1);
    assert!(thumb_requests(&mut plex).is_empty(), "the same cover was requested again");
    assert!(app.art.protocol.is_some());
    assert_eq!((app.art.shown_rect, app.art.prepared_rect), (shown, prepared));
}

/// A cover for the path already showing changes nothing.
#[tokio::test]
async fn cover_for_shown_path_is_noop() {
    let (mut app, _terminal, _plex) = app_showing_album_cover().await;
    let (shown, prepared) = (app.art.shown_rect, app.art.prepared_rect);
    let hold = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    app.art.hold_until = hold;
    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    app.handle_msg(Msg::ArtReady { thumb_path: AL1_THUMB.into(), img });
    assert!(app.art.protocol.is_some());
    assert_eq!((app.art.shown_rect, app.art.prepared_rect), (shown, prepared));
    assert_eq!(app.art.hold_until, hold);
}

/// Another album's cover is requested once; a failure can retry; a track
/// without a cover path drops the old one.
#[tokio::test]
async fn new_cover_loads_once() {
    let (mut app, _terminal, mut plex) = app_showing_album_cover().await;
    play_pos(&mut app, 2);
    play_pos(&mut app, 2);
    assert_eq!(thumb_requests(&mut plex), ["/library/metadata/al2/thumb"]);

    app.handle_msg(Msg::ArtFailed { thumb_path: "/library/metadata/al2/thumb".into() });
    play_pos(&mut app, 2);
    assert_eq!(
        thumb_requests(&mut plex),
        ["/library/metadata/al2/thumb"],
        "no retry after a failure"
    );

    play_pos(&mut app, 3);
    assert!(app.art.img.is_none() && app.art.protocol.is_none(), "the old cover stayed");
    assert!(app.art.shown_rect.is_none() && app.art.thumb.is_none());
}

/// The progress bar is solid background colour, not a dotted `░` track.
#[tokio::test]
async fn progress_bar_is_solid() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.stopped = false;
    app.duration = 100.0;
    app.position = 50.0;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let frame = flatten(&terminal);
    assert!(!frame.contains('░'), "the track must not be dotted\n{frame}");

    let accent = theme::accent_color();
    let track = theme::border().fg.expect("border colour");
    let buf = terminal.backend().buffer();
    let (mut filled, mut empty) = (0, 0);
    for y in 37..40 {
        for x in 0..120 {
            if let Some(cell) = buf.cell((x, y)) {
                if cell.bg == accent {
                    filled += 1;
                } else if cell.bg == track {
                    empty += 1;
                }
            }
        }
    }
    assert!(filled > 0, "no filled bar cells");
    assert!(empty > 0, "no track cells");
}

/// Highlighting a playlist requests a bounded preview of its tracks.
#[tokio::test]
async fn highlighting_a_playlist_previews_its_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;

    app.handle_msg(key('j'));

    let mut loaded = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::LoadPlaylistPreview { key, size } = cmd {
            assert_eq!(size, 500, "preview must be a bounded page");
            loaded = Some(key);
        }
    }
    assert_eq!(loaded.as_deref(), Some("p1"));
}

#[tokio::test]
async fn cached_playlist_preview_skips_the_request() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.preview_cache.insert("p1".into(), vec![track(0)]);
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;

    app.handle_msg(key('j'));

    assert_eq!(app.playlists.items.len(), 1);
    let mut requested = false;
    while let Ok(cmd) = plex.try_recv() {
        if matches!(cmd, PlexCmd::LoadPlaylistPreview { .. }) {
            requested = true;
        }
    }
    assert!(!requested, "a cache hit must not fetch again");
}

/// A failed preview is retried on the next highlight, not on every message.
#[tokio::test]
async fn failed_playlist_preview_retried_on_highlight() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.preview_debounce_ms = 0;
    let requests = |plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
        let mut n = 0;
        while let Ok(cmd) = plex.try_recv() {
            if matches!(cmd, PlexCmd::LoadPlaylistPreview { .. }) {
                n += 1;
            }
        }
        n
    };

    app.handle_msg(key('j'));
    assert_eq!(requests(&mut plex), 1);
    app.handle_msg(Msg::PreviewFailed { key: "p1".into() });
    app.handle_msg(Msg::ClearStatus(0));
    assert_eq!(requests(&mut plex), 0, "a failing server is not asked on every message");

    app.playlists.focus = PlaylistsCol::Items;
    app.handle_msg(Msg::ClearStatus(0));
    app.handle_msg(key('h')); // back to the list, p1 highlighted again
    assert_eq!(requests(&mut plex), 1, "the preview is asked for again");
}

/// An unrelated error leaves a pending Browser jump alone; only its own failure ends it.
#[tokio::test]
async fn unrelated_error_keeps_goto_jump() {
    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key('b')); // the track is on album al1 of artist ar1
    assert_eq!(app.tab, Tab::Browser, "the jump has started");

    app.handle_msg(Msg::LoadFailed { key: "p9".into(), error: "loading playlists: boom".into() });
    app.handle_msg(Msg::Error("searching: boom".into()));
    assert_eq!(app.tab, Tab::Browser, "the jump is still going");
    assert!(app.status.contains("searching"), "the error shows itself: {:?}", app.status);

    app.handle_msg(Msg::LoadFailed { key: "ar1".into(), error: "resolving artist: boom".into() });
    assert_eq!(app.tab, Tab::Queue, "the jump's own failure returns to the queue");
    assert!(app.status.contains("not found"), "{:?}", app.status);
}

fn sent(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Vec<PlexCmd> {
    let mut out = Vec::new();
    while let Ok(cmd) = plex.try_recv() {
        out.push(cmd);
    }
    out
}

fn artist_ar1() -> Artist {
    Artist { rating_key: "ar1".into(), title: "Northwind Sounds".into(), thumb: None }
}

/// Leaving the tab before Plex answers drops the jump; a late reply must not
/// pull the user back.
#[tokio::test]
async fn late_reply_ignored_after_cancelled_jump() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue = vec![track(0)];
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;
    app.handle_msg(key('b'));
    app.handle_msg(key('3'));
    sent(&mut plex);

    app.handle_msg(Msg::GotoArtist(artist_ar1()));

    assert_eq!(app.tab, Tab::Playlists);
    assert!(app.browser.artists.is_empty());
    assert!(sent(&mut plex).is_empty(), "nothing is loaded for a dropped jump");
}

/// Moving in the Browser while the jump loads also drops it, `g` jumps included.
#[tokio::test]
async fn browser_move_drops_pending_jump() {
    for keys in ["j", "gg", "ga"] {
        let (mut app, mut plex) = new_app_with_plex().await;
        app.queue = vec![track(0)];
        app.queue_state.select(Some(0));
        app.tab = Tab::Queue;
        app.handle_msg(key('g'));
        app.handle_msg(Msg::GotoArtist(artist_ar1()));
        for c in keys.chars() {
            app.handle_msg(key(c));
        }
        sent(&mut plex);

        app.handle_msg(Msg::AlbumsLoaded {
            artist_key: "ar1".into(),
            albums: vec![album_by("al1", "Evening Satellite", "Northwind Sounds", None)],
        });

        assert_eq!(app.browser.focus, BrowserCol::Artists, "after {keys}");
        assert!(sent(&mut plex).is_empty(), "no tracks are fetched for a dropped jump ({keys})");
    }
}

fn hit(kind: HitKind, key: &str, album: Option<&str>, artist: Option<&str>) -> SearchHit {
    SearchHit {
        kind,
        rating_key: key.into(),
        title: "Hit".into(),
        artist: "Artist".into(),
        subtitle: String::new(),
        duration_ms: None,
        album_key: album.map(Into::into),
        artist_key: artist.map(Into::into),
    }
}

fn reveal_hit(
    app: &mut TestApp,
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
    h: SearchHit,
) -> Vec<PlexCmd> {
    app.search.hits = vec![h];
    app.search.state.select(Some(0));
    app.tab = Tab::Search;
    app.handle_msg(key('b'));
    sent(plex)
}

fn browser_target(app: &TestApp) -> (String, Option<String>, Option<String>) {
    let artist =
        app.browser.artists[app.browser.artists_state.selected().unwrap()].rating_key.clone();
    let album =
        app.browser.albums_state.selected().map(|i| app.browser.albums[i].rating_key.clone());
    let track =
        app.browser.tracks_state.selected().map(|i| app.browser.tracks[i].rating_key.clone());
    (artist, album, track)
}

/// `b` on a result lands on the artist, the album or the track.
#[tokio::test]
async fn search_reveal_lands_on_each_kind() {
    let hits = [
        hit(HitKind::Artist, "ar1", None, None),
        hit(HitKind::Album, "al1", None, Some("ar1")),
        hit(HitKind::Track, "t1", Some("al1"), Some("ar1")),
    ];
    for h in hits {
        let kind = h.kind;
        let (mut app, mut plex) = new_app_with_plex().await;
        let cmds = reveal_hit(&mut app, &mut plex, h);
        assert!(matches!(cmds.as_slice(), [PlexCmd::ResolveArtist(k)] if k == "ar1"), "{kind:?}");
        assert_eq!(app.tab, Tab::Browser);

        app.handle_msg(Msg::GotoArtist(artist_ar1()));
        app.handle_msg(Msg::AlbumsLoaded {
            artist_key: "ar1".into(),
            albums: vec![
                album_by("al0", "Other", "Northwind Sounds", None),
                album_by("al1", "Evening Satellite", "Northwind Sounds", None),
            ],
        });
        if kind == HitKind::Artist {
            assert_eq!(app.browser.focus, BrowserCol::Artists);
            assert_eq!(browser_target(&app).0, "ar1");
            continue;
        }
        assert_eq!(app.browser.focus, BrowserCol::Tracks, "{kind:?}");
        app.handle_msg(Msg::TracksLoaded {
            album_key: "al1".into(),
            tracks: vec![track(0), Track { rating_key: "t1".into(), ..track(1) }],
        });
        let (artist, album, track) = browser_target(&app);
        assert_eq!((artist.as_str(), album.as_deref()), ("ar1", Some("al1")));
        let want = if kind == HitKind::Track { "t1" } else { "t0" };
        assert_eq!(track.as_deref(), Some(want), "{kind:?}");
    }
}

/// A failed reveal returns to Search; a hit without keys never asks Plex.
#[tokio::test]
async fn failed_reveal_returns_to_search() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let cmds = reveal_hit(&mut app, &mut plex, hit(HitKind::Track, "t1", Some("al1"), Some("ar1")));
    assert_eq!(cmds.len(), 1);
    app.handle_msg(Msg::LoadFailed { key: "ar1".into(), error: "boom".into() });
    assert_eq!(app.tab, Tab::Search);
    assert!(app.status.contains("not found"), "{:?}", app.status);

    let cmds = reveal_hit(&mut app, &mut plex, hit(HitKind::Track, "t1", None, None));
    assert!(cmds.is_empty());
    assert_eq!(app.tab, Tab::Search);
}

/// A reveal from Search obeys the same cancellation as the Queue jump.
#[tokio::test]
async fn late_reply_ignored_after_cancelled_reveal() {
    let (mut app, mut plex) = new_app_with_plex().await;
    reveal_hit(&mut app, &mut plex, hit(HitKind::Artist, "ar1", None, None));
    app.handle_msg(key('1'));

    app.handle_msg(Msg::GotoArtist(artist_ar1()));

    assert_eq!(app.tab, Tab::Queue);
    assert!(app.browser.artists.is_empty());
    assert!(sent(&mut plex).is_empty());
}

/// In the album view `b` reveals the selected track.
#[tokio::test]
async fn search_album_view_reveals_the_selected_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(1));
    app.search.detail = Some(shmc::ui::app::SearchDetail {
        key: "al1".into(),
        label: "Album".into(),
        tracks: vec![track(0), track(1)],
        state,
    });
    app.tab = Tab::Search;
    app.handle_msg(key('b'));
    assert!(matches!(sent(&mut plex).as_slice(), [PlexCmd::ResolveArtist(k)] if k == "ar1"));
    assert_eq!(app.tab, Tab::Browser);
}

#[tokio::test]
async fn large_playlist_add_asks_for_confirmation() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.playlists = vec![large];
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;

    app.handle_msg(enter());
    assert!(resolve_seqs(&mut plex).is_empty(), "a large playlist must ask before queueing");

    app.handle_msg(key('y'));
    let mut resolved = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { key, action, .. } = cmd {
            resolved = Some((key, action));
        }
    }
    assert_eq!(
        resolved,
        Some(("p0".to_string(), QueueAction::Replace)),
        "Enter on a large playlist replaces the queue"
    );
}

#[tokio::test]
async fn enter_on_a_playlist_replaces_the_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(1));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;

    app.handle_msg(enter());

    let mut resolved = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { key, action, .. } = cmd {
            resolved = Some((key, action));
        }
    }
    assert_eq!(resolved, Some(("p1".to_string(), QueueAction::Replace)));
    assert_eq!(
        app.playlists.focus,
        PlaylistsCol::List,
        "Enter must not drill into the playlist any more"
    );
}

/// The volume meter must not shift when 95% becomes 100%.
#[tokio::test]
async fn volume_digits_do_not_shift_the_meter() {
    let mut app = new_app().await;
    app.volume = 100.0;
    let full = render(&mut app, 120, 40);
    app.volume = 95.0;
    let less = render(&mut app, 120, 40);

    let x = |frame: &str| {
        frame.lines().find(|l| l.contains("Volume:")).and_then(|l| char_x(l, "Volume:"))
    };
    assert!(x(&full).is_some(), "volume meter missing\n{full}");
    assert_eq!(x(&full), x(&less), "volume digits moved the meter");
}

/// The header shows the file's audio details, not the live bitrate.
#[tokio::test]
async fn header_shows_file_audio_details() {
    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    app.duration = 257.0;
    app.position = 32.0;
    app.audio = AudioInfo {
        codec: Some("flac".into()),
        sample_rate_hz: Some(44100),
        bit_depth: Some(16),
        bitrate_kbps: Some(857),
        channels: Some(2),
    };

    let frame = render(&mut app, 120, 40);
    assert!(
        frame.contains("(FLAC 44.1k/16)"),
        "header missing parenthesized file audio details\n{frame}"
    );
    assert!(
        !frame.contains("(857 kbps)"),
        "the live bitrate should not be shown when file details exist\n{frame}"
    );
}

/// A second press turns repeat and single off.
#[tokio::test]
async fn loop_keys_toggle_off_again() {
    let mut app = new_app().await;
    app.loop_playlist = false;
    app.handle_msg(key('x'));
    assert!(app.loop_playlist);
    app.handle_msg(key('x'));
    assert!(!app.loop_playlist);

    app.loop_file = false;
    app.handle_msg(key('c'));
    assert!(app.loop_file);
    app.handle_msg(key('c'));
    assert!(!app.loop_file);
}

/// Consume drops the finished track without restarting the next; the last one stops.
#[tokio::test]
async fn consume_removes_finished_entry() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));

    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.queue[0].title, "Starfall! 1");
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);

    drop(app); // one app at a time: it holds the suite's lock

    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    app.consume = true;

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));

    assert!(app.queue.is_empty());
    assert_eq!(app.current, None);
    assert!(app.stopped);
}

/// `a` on an artist queues the artist's whole catalog.
#[tokio::test]
async fn add_on_artist_queues_its_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.artists =
        vec![Artist { rating_key: "ar1".into(), title: "Artist".into(), thumb: None }];
    app.browser.artists_state.select(Some(0));
    app.browser.focus = BrowserCol::Artists;
    app.tab = Tab::Browser;

    app.handle_msg(key('a'));

    let mut sent = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolveArtistTracks { key, action, .. } = cmd {
            sent = Some((key, action));
        }
    }
    assert_eq!(sent, Some(("ar1".to_string(), QueueAction::Append)));
}

#[tokio::test]
async fn track_info_updates_entry_and_cache() {
    let mut app = new_app().await;
    app.queue = vec![track(0)];
    app.current = Some(0);
    let audio = AudioInfo {
        codec: Some("flac".into()),
        sample_rate_hz: Some(44100),
        bit_depth: Some(16),
        ..Default::default()
    };

    app.handle_msg(Msg::TrackInfo { rating_key: "t0".into(), audio: audio.clone() });

    assert_eq!(app.audio, audio);
    assert_eq!(app.queue[0].audio.sample_rate_hz, Some(44100));
    assert!(app.audio_cache.contains_key("t0"));
}

/// A capped preview is announced in the status, fresh or from the cache (a
/// revisit fires no arrival); a complete one stays quiet.
#[tokio::test]
async fn capped_preview_is_announced() {
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![listed_playlist("p0", "Big Mix", 20_000)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_total = Some(20_000);
    app.handle_msg(Msg::PlaylistItemsLoaded {
        rating_key: "p0".into(),
        tracks: (0..100).map(track).collect(),
    });
    assert!(app.status.contains("20000"), "{}", app.status);

    let frame = render(&mut app, 120, 40);
    assert!(!frame.contains("preview 100/20000"), "the note moved to the status window\n{frame}");

    drop(app); // one app at a time: it holds the suite's lock

    // from the cache
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![listed_playlist("p0", "Big Mix", 20_000)];
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.state.select(Some(0));
    app.playlists.preview_cache.insert("p0".into(), (0..100).map(track).collect());
    app.handle_msg(key('j'));
    assert!(app.status.contains("20000"), "{}", app.status);

    drop(app);

    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_total = Some(12);
    app.handle_msg(Msg::PlaylistItemsLoaded {
        rating_key: "p0".into(),
        tracks: (0..12).map(track).collect(),
    });
    assert!(app.status.is_empty(), "a complete preview needs no notice");
}

/// The `#` column must not resize with the playlist.
#[tokio::test]
async fn playlist_item_columns_stable_across_playlists() {
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists =
        vec![listed_playlist("p0", "Small Mix", 9), listed_playlist("p1", "Big Mix", 20_000)];
    app.playlists.focus = PlaylistsCol::Items;

    app.playlists.items = (0..9).map(track).collect();
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_total = Some(9);
    let small = render(&mut app, 120, 40);
    let small_x = column_of(&small, "Starfall! 0");

    app.playlists.items = (0..100).map(track).collect();
    app.playlists.items_for = Some("p1".into());
    app.playlists.items_total = Some(20_000);
    let big = render(&mut app, 120, 40);
    let big_x = column_of(&big, "Starfall! 0");

    assert_eq!(
        small_x, big_x,
        "the Title column must not move between playlists\n--- small ---\n{small}\n--- big ---\n{big}"
    );
}

/// The `#` column must not resize with the album.
#[tokio::test]
async fn browser_track_columns_stable_across_albums() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;

    app.browser.tracks = (0..9).map(track).collect();
    app.browser.tracks_for = Some("al0".into());
    let small = render(&mut app, 120, 40);
    let small_x = column_of(&small, "Starfall! 0");

    app.browser.tracks = (0..100).map(track).collect();
    app.browser.tracks_for = Some("al1".into());
    let big = render(&mut app, 120, 40);
    let big_x = column_of(&big, "Starfall! 0");

    assert_eq!(
        small_x, big_x,
        "the Title column must not move between albums\n--- 9 tracks ---\n{small}\n--- 100 tracks ---\n{big}"
    );
}

/// A 1000-track album grows the `#` column instead of clipping.
#[tokio::test]
async fn thousand_track_album_prints_numbers() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks = (0..1000).map(track).collect();
    app.browser.tracks_for = Some("al0".into());
    app.browser.tracks_state.select(Some(999));

    let frame = render(&mut app, 120, 40);
    let row = frame
        .lines()
        .find(|l| l.contains("Starfall! 999"))
        .expect("the last track must be on screen");
    // a clipped column would read " 100 Starfall! 999"
    assert!(
        row.contains("1000 Starfall! 999"),
        "the 1000th track must print 1000 next to its title: {row:?}"
    );
}

fn run_cmd(app: &mut App, cmd: &str) {
    app.handle_msg(key(':'));
    for c in cmd.chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
}

/// The prompt draws its own colon, so `:pause` must run without it in the buffer.
#[tokio::test]
async fn colon_commands_accept_the_prompt_prefix() {
    let mut app = new_app().await;
    app.stopped = false;
    app.volume = 10.0;

    run_cmd(&mut app, "pause");
    assert!(!app.command.typing);
    assert!(
        !app.status.contains("unknown command"),
        "the prompt colon is not part of the buffer: {}",
        app.status
    );

    run_cmd(&mut app, "vol 42");
    assert_eq!(app.volume, 42.0);
    assert!(app.status.contains("volume: 42%"));

    app.queue = vec![track(0)];
    app.current = Some(0);
    app.stopped = false;
    run_cmd(&mut app, "clear");
    app.handle_msg(key('y'));
    assert!(app.queue.is_empty());
    assert_eq!(app.current, None);

    run_cmd(&mut app, "q");
    assert!(app.should_quit);
}

#[tokio::test]
async fn colon_commands_cover_playback_modes_and_tabs() {
    let mut app = new_app().await;
    app.queue = vec![track(0), track(1)];
    app.current = Some(0);
    app.stopped = false;

    run_cmd(&mut app, "stop");
    assert!(app.stopped, ":stop must keep the queue but stop playback");
    assert_eq!(app.queue.len(), 2);

    run_cmd(&mut app, "play");
    assert!(!app.stopped, ":play must restart the current track");

    app.volume = 40.0;
    run_cmd(&mut app, "vol +5");
    assert_eq!(app.volume, 45.0, ":vol +N is relative");

    run_cmd(&mut app, "seek -10");
    assert!(app.status.contains("seek -10s"));

    run_cmd(&mut app, "shuffle");
    assert!(app.random);
    run_cmd(&mut app, "repeat");
    assert!(app.loop_playlist);
    run_cmd(&mut app, "single");
    assert!(app.loop_file);
    run_cmd(&mut app, "consume");
    assert!(app.consume);

    run_cmd(&mut app, "reshuffle");
    assert!(app.random, "reshuffle leaves shuffle on");
    assert!(app.status.contains("reshuffled"));

    app.tab = Tab::Queue;
    app.current = Some(1);
    app.queue_state.select(Some(0));
    run_cmd(&mut app, "playing");
    assert_eq!(app.queue_state.selected(), Some(1));

    run_cmd(&mut app, "browser");
    assert_eq!(app.tab, Tab::Browser);
    run_cmd(&mut app, "playlists");
    assert_eq!(app.tab, Tab::Playlists);
    run_cmd(&mut app, "queue");
    assert_eq!(app.tab, Tab::Queue);
    run_cmd(&mut app, "search");
    assert_eq!(app.tab, Tab::Search);
    assert!(app.search.focused);
    app.search.focused = false;
    run_cmd(&mut app, "help");
    assert_eq!(app.tab, Tab::Help);
}

/// Square art caps the art width at `art_pct`, so the queue keeps its Artist/Album columns.
#[tokio::test]
async fn square_art_pane_respects_the_percentage_cap() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.art_square = true;
    app.art_metadata = true;
    let track0 = track(0);
    let artist = track0.artist.clone();
    let album = track0.album.clone();
    app.queue = vec![track0];
    app.queue_state.select(Some(0));
    app.current = Some(0);
    app.tab = Tab::Queue;

    let frame = render(&mut app, 120, 40);
    let cover_line = frame.lines().find(|l| l.contains("No cover")).expect("cover box");
    let bars = bar_columns(cover_line);
    assert!(bars.len() >= 4, "cover line must show the box borders\n{cover_line}");
    assert_eq!(
        bars[2] - bars[1] + 1,
        46,
        "square mode must cap the box at the configured art percentage\n{cover_line}"
    );

    // the space below the capped square holds the metadata
    let divider = bars[3];
    let cover_y = frame.lines().position(|l| l.contains("No cover")).expect("cover line");
    let left_text: String = frame
        .lines()
        .skip(cover_y + 1)
        .map(|l| l.chars().take(divider).collect::<String>())
        .collect();
    assert!(left_text.contains(&artist), "metadata artist missing\n{left_text}");
    assert!(left_text.contains(&album), "metadata album missing\n{left_text}");

    app.art_metadata = false;
    let frame = render(&mut app, 120, 40);
    let cover_line = frame.lines().find(|l| l.contains("No cover")).expect("cover box");
    let bars = bar_columns(cover_line);
    let divider = bars[3];
    let cover_y = frame.lines().position(|l| l.contains("No cover")).expect("cover line");
    let left_text: String = frame
        .lines()
        .skip(cover_y + 1)
        .map(|l| l.chars().take(divider).collect::<String>())
        .collect();
    assert!(
        !left_text.contains(&artist) && !left_text.contains(&album),
        "metadata must be absent when disabled\n{left_text}"
    );
}

#[tokio::test]
async fn browser_track_enter_replaces_with_album_remainder() {
    let mut app = new_app().await;
    app.queue = vec![track(9)];
    app.current = Some(0);
    app.stopped = false;
    app.browser.tracks = (0..5).map(track).collect();
    app.browser.artists_state.select(Some(0));
    app.browser.tracks_state.select(Some(2));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;

    app.handle_msg(enter());

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t2", "t3", "t4"]
    );
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped);
}

#[tokio::test]
async fn browser_album_enter_replaces_queue_with_album() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.albums = vec![Album {
        rating_key: "al1".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        year: Some(2024),
        thumb: None,
    }];
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;

    app.handle_msg(enter());

    let mut sent = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolveAlbum { key, action, .. } = cmd {
            sent = Some((key, action));
        }
    }
    assert_eq!(sent, Some(("al1".to_string(), QueueAction::Replace)));
}

#[tokio::test]
async fn playlist_track_enter_replaces_from_that_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items = (0..4).map(track).collect();
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;

    app.handle_msg(enter());

    let mut sent = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { key, action, start, .. } = cmd {
            sent = Some((key, action, start));
        }
    }
    assert_eq!(sent, Some(("p0".to_string(), QueueAction::Replace, Some("t1".to_string()))));
}

/// A track row asks before replacing or appending a huge playlist, counting
/// the remainder from that track, and keeps the start track once confirmed.
#[tokio::test]
async fn large_playlist_track_asks_remainder_or_start() {
    let cases: [(fn() -> Msg, QueueAction); 2] =
        [(enter, QueueAction::Replace), (|| key('a'), QueueAction::Append)];
    for (press, want) in cases {
        let (mut app, mut plex) = new_app_with_plex().await;
        let mut large = playlist(0);
        large.item_count = Some(5000);
        app.playlists.playlists = vec![large];
        app.playlists.items_for = Some("p0".into());
        app.playlists.items = (0..4).map(track).collect();
        app.playlists.items_state.select(Some(1));
        app.playlists.focus = PlaylistsCol::Items;
        app.tab = Tab::Playlists;
        app.confirm_playlist_over = 1000;

        app.handle_msg(press());

        assert!(resolve_seqs(&mut plex).is_empty(), "{want:?}: a large playlist must ask first");
        assert!(
            app.status.contains("4999"),
            "{want:?}: the prompt must count the remainder: {:?}",
            app.status
        );

        app.handle_msg(key('y'));
        let mut sent = None;
        while let Ok(cmd) = plex.try_recv() {
            if let PlexCmd::ResolvePlaylist { key, action, start, .. } = cmd {
                sent = Some((key, action, start));
            }
        }
        assert_eq!(
            sent,
            Some(("p0".to_string(), want, Some("t1".to_string()))),
            "y must keep the start track, not use the whole playlist"
        );
    }
}

/// A key press after the prompt timed out belongs to the list, not the old question.
#[tokio::test]
async fn key_after_expired_prompt_not_swallowed() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.playlists = vec![large];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items = (0..4).map(track).collect();
    app.playlists.items_state.select(Some(0));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;
    // times out at once, so the next key press is late
    app.confirm_timeout_secs = 0;

    app.handle_msg(enter());
    assert!(app.status.contains("tracks?"), "the prompt must be showing: {:?}", app.status);

    app.handle_msg(key('j'));

    assert_eq!(
        app.playlists.items_state.selected(),
        Some(1),
        "the key press must move the cursor, not cancel the question"
    );
    assert!(
        !app.status.contains("cancelled"),
        "the question must not swallow the key: {:?}",
        app.status
    );
    assert!(resolve_seqs(&mut plex).is_empty(), "an expired question must not queue anything");
}

/// `i` inserts one track, so nothing to confirm.
#[tokio::test]
async fn play_next_large_playlist_inserts_one_track() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let mut large = playlist(0);
    large.item_count = Some(5000);
    app.playlists.playlists = vec![large];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items = (0..4).map(track).collect();
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.tab = Tab::Playlists;
    app.confirm_playlist_over = 1000;
    app.queue = vec![track(0)];
    app.current = Some(0);

    app.handle_msg(key('i'));

    assert!(resolve_seqs(&mut plex).is_empty(), "play-next never queues a whole playlist");
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.queue[1].rating_key, "t1");
}

/// `g <letter>` in Help has nothing to jump to and reports no miss.
#[tokio::test]
async fn g_letter_in_help_says_nothing() {
    let mut app = new_app().await;
    app.tab = Tab::Help;

    app.handle_msg(key('g'));
    app.handle_msg(key('a'));

    assert_eq!(app.tab, Tab::Help);
    assert!(app.status.is_empty(), "Help has no titles to jump to: {:?}", app.status);
}

/// Help descriptions start in one column at every width; one that does not fit
/// ends in an ellipsis rather than being clipped.
#[tokio::test]
async fn help_descriptions_share_column_and_ellipsize() {
    let mut app = new_app().await;
    app.tab = Tab::Help;
    let descs: Vec<&str> = app
        .keymap
        .help_lines()
        .iter()
        .filter_map(|l| match l {
            shmc::ui::keymap::HelpLine::Keys { desc, .. } => Some(*desc),
            _ => None,
        })
        .collect();
    for w in [30u16, 46, 60, 80, 120, 200] {
        // tall enough that the list does not scroll
        let frame = render(&mut app, w, 150);
        let mut cols = std::collections::BTreeSet::new();
        let mut cut = false;
        let pane = frame
            .lines()
            .skip_while(|l| !l.contains(" Global "))
            .take_while(|l| !l.starts_with('└'));
        for line in pane {
            let cells: Vec<char> = line.chars().collect();
            let inner: String =
                cells[2.min(cells.len())..cells.len().saturating_sub(2)].iter().collect();
            let chars: Vec<char> = inner.chars().collect();
            let Some(gap) = (1..chars.len()).find(|&i| chars[i] == ' ' && chars[i - 1] == ' ')
            else {
                continue;
            };
            let Some(start) = (gap..chars.len()).find(|&i| chars[i] != ' ') else {
                continue;
            };
            cols.insert(start);
            let shown: String = chars[start..].iter().collect::<String>().trim_end().to_string();
            let whole = descs.iter().any(|d| *d == shown);
            let prefix = shown.strip_suffix('…').map(|p| descs.iter().any(|d| d.starts_with(p)));
            assert!(
                whole || prefix == Some(true),
                "{w}: {shown:?} is neither a description nor an ellipsized one\n{frame}"
            );
            cut |= prefix == Some(true);
        }
        assert_eq!(cols.len(), 1, "{w}: descriptions start in {cols:?}\n{frame}");
        assert_eq!(cut, w <= 60, "{w}: ellipsis only where a row does not fit\n{frame}");
    }
}

#[tokio::test]
async fn huge_sleep_timer_is_clamped() {
    let mut app = new_app().await;
    run_cmd(&mut app, "sleep 4294967295");

    assert_eq!(
        app.sleep_timer_label().as_deref(),
        Some("sleep 1440m"),
        "an absurd duration is clamped to a day"
    );
    assert!(app.status.contains("clamped"), "the clamp must be visible: {:?}", app.status);
}

/// A vanished playlist start must not ride on `Msg::Error`: a pending Browser jump
/// would report it as "Album not found".
#[tokio::test]
async fn vanished_playlist_start_reported() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.playlists.playlists = vec![playlist(0)];
    app.playlists.state.select(Some(0));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
    app.handle_msg(enter());
    let seq = resolve_seqs(&mut plex)[0];
    app.handle_msg(reply(seq, QueueOutcome::StartMissing));

    assert!(
        app.status.contains("no longer in the playlist"),
        "the refusal must say what happened: {:?}",
        app.status
    );
    assert!(app.status_error, "it is shown as an error");
}

#[tokio::test]
async fn hints_name_rebound_keys() {
    let keys = key_table("[keys.queue]\nremove = [\"X\"]\n[keys.playlists]\nactivate = [\"Z\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.queue_state.select(Some(0));
    app.handle_msg(key('v'));
    assert!(app.status.contains("X remove"), "{:?}", app.status);
    assert!(!app.status.contains("d remove"), "{:?}", app.status);

    app.visual = None;
    app.tab = Tab::Playlists;
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.playlists =
        vec![listed_playlist("p0", "A", 5000), listed_playlist("p1", "B", 5000)];
    app.playlists.state.select(Some(0));
    let frame = render(&mut app, 120, 30);
    assert!(frame.contains("Z adds it"), "{frame}");
    app.playlists.preview_cache.insert("p1".into(), (0..3).map(track).collect());
    app.handle_msg(key('j'));
    assert!(app.status.contains("preview:"), "{:?}", app.status);
    assert!(app.status.contains("Z adds all"), "{:?}", app.status);
}

/// Newlines in a title or error are escaped; the status is one line.
#[tokio::test]
async fn status_message_stays_on_one_line() {
    let mut app = new_app().await;

    app.transient("Night\nMix".into());
    assert_eq!(app.status, "Night\\nMix");

    app.transient_err("bad\r\nthing".into());
    assert_eq!(app.status, "bad\\r\\nthing");
    assert!(app.status_error);
}

#[tokio::test]
async fn play_next_inserts_after_the_current_track() {
    let mut app = new_app().await;
    app.queue = vec![track(0), track(1)];
    app.current = Some(0);
    app.browser.tracks = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks_state.select(Some(1));
    app.browser.focus = BrowserCol::Tracks;
    app.tab = Tab::Browser;

    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t11", "t1"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn queue_reorder_moves_selected_and_playing_rows() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.tab = Tab::Queue;

    app.handle_msg(key('J'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t1", "t0", "t2"]
    );
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue_state.selected(), Some(1));

    app.handle_msg(key('K'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t2"]
    );
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn shuffle_keeps_current_first_and_restores() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(2));
    let original: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();

    app.handle_msg(key('z'));
    assert!(app.random);
    assert_eq!(app.queue[0].rating_key, "t2");
    assert_eq!(app.current, Some(0));
    assert_eq!(app.queue.len(), 6);

    app.handle_msg(key('z'));
    assert!(!app.random);
    assert_eq!(app.queue.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(), original);
    assert_eq!(app.current, Some(2));
}

#[tokio::test]
async fn reshuffle_keeps_the_current_track_first() {
    let mut app = new_app().await;
    app.queue = (0..6).map(track).collect();
    app.current = Some(3);
    app.queue_state.select(Some(3));

    app.handle_msg(key('Z'));

    assert!(app.random);
    assert_eq!(app.queue[0].rating_key, "t3");
    assert_eq!(app.current, Some(0));
}

#[tokio::test]
async fn search_track_enter_replaces_with_album_remainder() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.search.hits = vec![SearchHit {
        kind: HitKind::Track,
        rating_key: "t9".into(),
        title: "Track".into(),
        artist: "Artist".into(),
        subtitle: "Album".into(),
        duration_ms: Some(1000),
        album_key: None,
        artist_key: None,
    }];
    app.search.state.select(Some(0));
    app.tab = Tab::Search;

    app.handle_msg(enter());

    let mut sent = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolveTrack { key, action, rest_of_album, .. } = cmd {
            sent = Some((key, action, rest_of_album));
        }
    }
    assert_eq!(sent, Some(("t9".to_string(), QueueAction::Replace, true)));
}

#[tokio::test]
async fn browser_album_right_drills_into_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.browser.albums = vec![Album {
        rating_key: "al1".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        year: Some(2024),
        thumb: None,
    }];
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app.tab = Tab::Browser;

    app.handle_msg(key_code(KeyCode::Right));

    assert_eq!(app.browser.focus, BrowserCol::Tracks);
    let mut sent = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::LoadTracks(key) = cmd {
            sent = Some(key);
        }
    }
    assert_eq!(sent.as_deref(), Some("al1"));
}

#[tokio::test]
async fn queue_follow_pins_playing_until_browse() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert!(visible > 0 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(80)),
    }));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(80));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the playing row must be the first visible queue row"
    );
    // a second paint must not drift back into the scrolloff margin
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "top alignment must persist across repaints"
    );

    // browsing pauses the follow
    app.handle_msg(key('j'));
    assert_eq!(app.queue_state.selected(), Some(81));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        80,
        "the first manual step must not scroll the pane"
    );
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(100)),
    }));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        Some(81),
        "a track change must not yank a browsing cursor"
    );
    assert_ne!(app.scroll.offset(ScrollId::Queue), 100);

    // `o` re-arms it
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(100));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 100);

    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(110)),
    }));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(110));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 110);
}

/// The first step after `o` must move one row, not rewind the pane by
/// the scrolloff margin.
#[tokio::test]
async fn first_step_after_o_does_not_scroll() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(80);
    app.queue_state.select(Some(80));
    app.stopped = false;
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    let margin = app.scrolloff.min((visible.saturating_sub(1)) / 2);
    assert!(visible > margin + 1 && visible < 300, "unexpected queue height {visible}");

    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(80));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 80);

    // steps up to the bottom margin keep the pane where the pin left it
    let first_scroll = 80 + visible - margin;
    for row in 81..first_scroll {
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue_state.selected(), Some(row));
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            80,
            "stepping to row {row} must not scroll the pane"
        );
    }

    // the margin scrolls one row, then normal scrolling resumes
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(first_scroll));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 81);
}

/// A move arriving before the `o` pin is drawn must scroll normally, not pin
/// the new cursor to the top (with auto-follow on or off).
#[tokio::test]
async fn move_after_o_does_not_repin_row() {
    for follow in [true, false] {
        let mut app = new_app().await;
        app.tab = Tab::Queue;
        app.queue = (0..300).map(track).collect();
        app.current = Some(80);
        app.queue_state.select(Some(80));
        app.stopped = false;
        app.auto_follow = follow;
        let _ = render(&mut app, 120, 40);
        let visible = app.scroll.visible(ScrollId::Queue);
        let before = app.scroll.offset(ScrollId::Queue);

        app.handle_msg(key('o'));
        app.handle_msg(key('j'));
        let _ = render(&mut app, 120, 40);
        assert_eq!(app.queue_state.selected(), Some(81), "auto_follow {follow}");
        assert_eq!(
            app.scroll.offset(ScrollId::Queue),
            compute_offset(81, before, visible, 300, app.scrolloff),
            "auto_follow {follow}: a move before the pin is drawn must not pin row 81 to the top"
        );
    }
}

/// The view following the end-of-list wrap is not the jump.
#[tokio::test]
async fn queue_end_wrap_jumps_to_top() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(299);
    app.queue_state.select(Some(299));
    app.stopped = false;
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    let visible = app.scroll.visible(ScrollId::Queue);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        300 - visible,
        "the pin clamps flush to the bottom"
    );

    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn queue_follow_does_not_fight_the_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..120).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 9");
    assert!(app.queue_filter.cursor_of(0).is_none(), "test filter must hide the playing track");

    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(90)),
    }));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        Some(0),
        "auto-follow must leave a filtered queue alone"
    );

    // `o` still works within the filtered view
    filter_queue(&mut app, "Starfall! 90");
    app.handle_msg(key('o'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.scroll.offset(ScrollId::Queue), 0);
}

#[tokio::test]
async fn disabling_art_gives_the_queue_full_width() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..10).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let frame = flatten(&terminal);
    let (x, _) = locate(&terminal, "Title").expect("queue header");
    assert!(
        x < 10,
        "the queue must start at the left edge when art is off (Title at x={x})\n{frame}"
    );
    assert!(!frame.contains("No cover"), "no empty art placeholder\n{frame}");
}

#[tokio::test]
async fn rebound_key_replaces_old_binding() {
    let keys = key_table("[keys.queue]\nremove = [\"Delete\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.queue_state.select(Some(1));

    app.handle_msg(key('d'));
    assert_eq!(app.queue.len(), 4, "d is not remove any more");

    app.handle_msg(key_code(KeyCode::Delete));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t2", "t3"]
    );
}

#[tokio::test]
async fn ctrl_c_quits_when_quit_unbound() {
    let keys = key_table("[keys.global]\nquit = []\n");
    let mut app = new_app_with_keys(&keys).await;

    app.handle_msg(Msg::Key(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))));

    assert!(app.should_quit, "Ctrl-C is reserved for quitting");
}

#[tokio::test]
async fn clear_confirm_accepts_rebound_key() {
    let keys = key_table("[keys.queue]\nclear = [\"X\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    app.handle_msg(key('D'));
    assert_eq!(app.queue.len(), 3, "D is not clear any more");

    app.handle_msg(key('X'));
    assert_eq!(app.queue.len(), 3, "the first press only asks");
    assert!(app.status.contains('X'), "the question names the key: {}", app.status);

    app.handle_msg(key('X'));
    assert!(app.queue.is_empty(), "the rebound key confirms too");
}

#[tokio::test]
async fn queue_gg_and_shift_g_reach_the_ends() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..10).map(track).collect();
    app.queue_state.select(Some(4));

    keys(&mut app, "G");
    assert_eq!(app.queue_state.selected(), Some(9));
    keys(&mut app, "gg");
    assert_eq!(app.queue_state.selected(), Some(0));
    assert_eq!(app.tab, Tab::Queue, "g no longer opens the Browser");
}

#[tokio::test]
async fn gg_runs_what_it_is_bound_to() {
    let keys = key_table("[keys.browser]\ntop = [\"Home\"]\nbottom = [\"gg\"]\n");
    let mut app = new_app_with_keys(&keys).await;
    app.tab = Tab::Browser;
    app.browser.artists = (0..10)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Artist {i:02}"),
            thumb: None,
        })
        .collect();
    app.browser.artists_state.select(Some(0));

    app.handle_msg(key('g'));
    app.handle_msg(key('g'));

    assert_eq!(app.browser.artists_state.selected(), Some(9));
}

#[tokio::test]
async fn undo_restores_the_last_removed_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(2));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t3"]
    );
    assert_eq!(app.current, Some(1));

    app.handle_msg(key('u'));
    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>(),
        vec!["t0", "t1", "t2", "t3"]
    );
    assert_eq!(app.current, Some(1), "non-playing removal leaves playback put");
    assert_eq!(app.status, "restored: Starfall! 2");

    app.handle_msg(key('u'));
    assert_eq!(app.status, "nothing to undo");
}

#[tokio::test]
async fn repeated_undo_restores_removals_newest_first() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..8).map(track).collect();
    app.queue_state.select(Some(5));
    app.stopped = true;
    let original = queue_keys(&app);

    app.handle_msg(key('d'));
    app.queue_state.select(Some(1));
    app.handle_msg(key('v'));
    app.handle_msg(key('j'));
    app.handle_msg(key('d'));
    assert_eq!(queue_keys(&app).len(), 5);

    app.handle_msg(key('u'));
    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), original);

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), original);
}

#[tokio::test]
async fn another_queue_change_drops_the_undo_history() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.queue_state.select(Some(0));
    app.stopped = true;

    app.handle_msg(key('d'));
    app.handle_msg(key('d'));
    app.add_tracks(vec![track(9)], Confirm::Track { title: "Starfall! 9".into() });
    let after = queue_keys(&app);
    assert_eq!(after, vec!["t2", "t3", "t9"]);

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), after);
}

#[tokio::test]
async fn undo_history_is_capped_at_twenty() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..21).map(track).collect();
    app.queue_state.select(Some(0));
    app.stopped = true;

    for _ in 0..21 {
        app.handle_msg(key('d'));
    }
    assert!(app.queue.is_empty());
    for _ in 0..21 {
        app.handle_msg(key('u'));
    }
    let keys = queue_keys(&app);
    assert_eq!(keys.len(), 20);
    assert_eq!(keys[0], "t1", "the oldest removal (t0) is gone");
}

#[tokio::test]
async fn undo_after_removing_playing_keeps_next_playing() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(1));
    app.stopped = false;

    app.handle_msg(key('d'));
    assert_eq!(app.current, Some(1), "the track that slid in starts");
    assert_eq!(app.queue[1].rating_key, "t2");

    app.handle_msg(key('u'));
    assert_eq!(app.queue[1].rating_key, "t1");
    assert_eq!(app.current, Some(2), "playback stays on t2 at its new index");
    assert_eq!(app.queue[2].rating_key, "t2");
}

#[tokio::test]
async fn paste_into_filter_inserts_and_refilters() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..12).map(track).collect();
    app.queue_state.select(Some(0));
    app.queue_filter.typing = true;

    app.handle_msg(Msg::Paste("Starfall! 9".into()));

    assert_eq!(app.queue_filter.text, "Starfall! 9");
    let (text, _, shown, total) = app.filter_typing().expect("still typing");
    assert_eq!(text, "Starfall! 9");
    assert_eq!((shown, total), (1, 12), "the paste must re-apply the filter");
}

#[tokio::test]
async fn paste_with_no_focus_changes_nothing() {
    for tab in [Tab::Browser, Tab::Queue] {
        let mut app = new_app().await;
        app.tab = tab;
        app.queue = (0..3).map(track).collect();
        app.browser_filters[0].text = "Northwind".into();
        app.search.input = "kept".into();

        // `/` and `\r` in the paste must not start a filter or a search
        app.handle_msg(Msg::Paste("/night\r".into()));

        assert_eq!(app.browser_filters[0].text, "Northwind", "{tab:?}");
        assert!(!app.browser_filters[0].typing, "{tab:?}");
        assert!(app.queue_filter.text.is_empty(), "{tab:?}: no filter was started");
        assert!(!app.queue_filter.typing, "{tab:?}");
        assert_eq!(app.search.input, "kept", "{tab:?}");
        assert!(app.search.last_query.is_empty(), "{tab:?}");
        assert!(!app.search.focused, "{tab:?}: a paste must not steal focus");
        assert!(!app.command.typing, "{tab:?}");
        assert!(!app.status.contains("searching"), "{tab:?}: {}", app.status);
        assert_eq!(app.tab, tab);
    }
}

#[tokio::test]
async fn pasted_control_characters_are_text_not_actions() {
    let mut app = new_app().await;
    app.tab = Tab::Search;
    app.search.focused = true;

    app.handle_msg(Msg::Paste("a\rb\u{1b}[A".into()));

    // `\r` must not run the search, Esc must not defocus
    assert_eq!(app.search.input, "a\rb\u{1b}[A");
    assert!(app.search.last_query.is_empty());
    assert!(app.search.focused);
    assert!(app.status.is_empty(), "{}", app.status);
}

#[tokio::test]
async fn search_history_recalled_with_up_down() {
    let mut app = new_app().await;
    app.tab = Tab::Search;
    app.search.focused = true;
    app.search.input = "first".into();
    app.handle_msg(enter());
    app.search.focused = true;
    app.search.input = "second".into();
    app.handle_msg(enter());

    app.search.focused = true;
    app.search.input.clear();
    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.search.input, "second");
    app.handle_msg(key_code(KeyCode::Up));
    assert_eq!(app.search.input, "first");
    app.handle_msg(key_code(KeyCode::Down));
    assert_eq!(app.search.input, "second");
    app.handle_msg(key_code(KeyCode::Down));
    assert!(app.search.input.is_empty());
}

#[tokio::test]
async fn g_letter_jumps_to_first_matching_title() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Artists;
    app.browser.artists = ["Alpha", "Beta", "Bravo", "Charlie"]
        .iter()
        .enumerate()
        .map(|(i, title)| Artist {
            rating_key: format!("a{i}"),
            title: (*title).into(),
            thumb: None,
        })
        .collect();
    app.browser.artists_state.select(Some(0));

    app.handle_msg(key('g'));
    app.handle_msg(key('b'));
    assert_eq!(app.browser.artists_state.selected(), Some(1));

    // 'a' is append here, but after `g` it jumps
    app.handle_msg(key('g'));
    app.handle_msg(key('a'));
    assert_eq!(app.browser.artists_state.selected(), Some(0));
    assert_eq!(app.status, "jump: A");
    assert!(plex_rx.try_recv().is_err(), "'g a' must not append the selection");

    // likewise 'c' (loop-file)
    app.handle_msg(key('g'));
    app.handle_msg(key('c'));
    assert_eq!(app.browser.artists_state.selected(), Some(3));
    assert!(!app.loop_file, "'g c' must jump, not toggle loop-file");

    app.handle_msg(key('g'));
    app.handle_msg(key('d'));
    assert_eq!(app.browser.artists_state.selected(), Some(3));
    assert!(app.status.contains("no row starts with D"));
}

#[tokio::test]
async fn sleep_timer_cycles_and_stops_at_deadline() {
    let mut app = new_app().await;
    app.handle_msg(key('S'));
    assert!(
        app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 15m")),
        "first S sets 15 minutes"
    );
    app.handle_msg(key('S'));
    assert!(app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 30m")));
    app.handle_msg(key('S'));
    assert!(app.sleep_timer_label().is_some_and(|s| s.starts_with("sleep 60m")));
    app.handle_msg(key('S'));
    assert!(app.sleep_timer_label().is_none());

    // firing the posted timer stops playback and clears it
    app.handle_msg(key('S'));
    app.stopped = false;
    app.handle_msg(Msg::SleepTimer { epoch: 5 });
    assert!(app.stopped);
    assert_eq!(app.status, "sleep timer: playback stopped");
    assert!(app.sleep_timer_label().is_none());
}

#[tokio::test]
async fn auto_follow_can_be_disabled() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..300).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
    app.stopped = false;

    app.handle_msg(key('j'));
    assert!(app.status.contains("queue follow: paused"));
    app.handle_msg(key('j'));
    assert!(app.status.is_empty(), "an already-paused follow must not spam the status line");

    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: on");

    // with the option off, `o` jumps once but does not stay armed
    app.current = Some(80);
    app.auto_follow = false;
    app.handle_msg(key('o'));
    assert_eq!(app.status, "queue follow: off");
    let _ = render(&mut app, 120, 40);
    let jump_offset = app.scroll.offset(ScrollId::Queue);
    assert_eq!(jump_offset, 80, "the one-shot jump still pins to the top");

    // a repaint and a step leave the pane where the jump put it
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "a repaint must not move the one-shot jump"
    );
    app.handle_msg(key('j'));
    let _ = render(&mut app, 120, 40);
    assert_eq!(app.queue_state.selected(), Some(81));
    assert_eq!(
        app.scroll.offset(ScrollId::Queue),
        jump_offset,
        "browsing from the one-shot jump must not scroll the pane"
    );

    let selected = app.queue_state.selected();
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(120)),
    }));
    let _ = render(&mut app, 120, 40);
    assert_eq!(
        app.queue_state.selected(),
        selected,
        "disabled auto-follow must not move the cursor"
    );
}

/// After a consume mpv can report the pre-removal index of the next track; that
/// stale position must not move `current` past the queue.
#[tokio::test]
async fn stale_playlist_pos_keeps_current_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.current, Some(1), "the next track takes the freed slot");

    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(2)),
    }));
    assert_eq!(app.current, Some(1), "a stale position must not move the current track");

    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(9)),
    }));
    assert_eq!(app.current, Some(1));
}

/// The same stale position must not make "play next" splice past the queue end.
#[tokio::test]
async fn play_next_after_consume_does_not_panic() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    app.handle_msg(Msg::Mpv(MpvEvent::Property {
        name: "playlist-pos".into(),
        value: Some(serde_json::json!(2)),
    }));

    app.tab = Tab::Browser;
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks = (0..3).map(|i| track(10 + i)).collect();
    app.browser.tracks_state.select(Some(0));

    app.handle_msg(key('i'));
    assert_eq!(app.queue.len(), 3, "the track was queued after the current one");
    assert_eq!(app.queue[2].rating_key, "t10");
}

fn playlist_pos(i: usize) -> Msg {
    Msg::Mpv(MpvEvent::Property { name: "playlist-pos".into(), value: Some(serde_json::json!(i)) })
}

/// The consume guard stays armed (mpv sends no position for the advance); the
/// position for the user's `>` must not be swallowed by it.
#[tokio::test]
async fn next_after_a_consume_follows_mpv() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..6).map(track).collect();
    app.current = Some(0);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    assert_eq!(app.queue.len(), 5);
    assert_eq!(app.current, Some(0));

    // consume off: `>` only moves, and must clear the old guard
    app.consume = false;
    app.handle_msg(key('>'));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1), "current must follow mpv after `>`");
}

/// With consume `>` removes the track you leave; the guard swallows the
/// pre-removal position mpv may report, but not the real one.
#[tokio::test]
async fn next_with_consume_removes_left_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t2", "t3"]);
    assert_eq!(app.current, Some(1), "current is the entry that was next");

    app.handle_msg(playlist_pos(2));
    assert_eq!(app.current, Some(1));
    app.handle_msg(playlist_pos(1));
    assert_eq!(app.current, Some(1));

    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t3"]);
}

/// `>` with nothing to advance to consumes nothing; with repeat on it consumes the last entry and wraps.
#[tokio::test]
async fn next_with_consume_on_the_last_entry() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..3).map(track).collect();
    app.current = Some(2);
    app.consume = true;
    app.stopped = false;

    app.handle_msg(key('>'));
    assert_eq!(app.queue.len(), 3, "repeat off: nothing to leave to");

    app.loop_playlist = true;
    app.handle_msg(key('>'));
    let keys: Vec<_> = app.queue.iter().map(|t| t.rating_key.as_str()).collect();
    assert_eq!(keys, ["t0", "t1"]);
    assert_eq!(app.current, Some(0), "wrapped to the first entry");

    app.queue.truncate(1);
    app.current = Some(0);
    app.handle_msg(key('>'));
    assert_eq!(app.queue.len(), 1, "a lone track wraps to itself");
}

/// Plays the position events mpv sends during a reorder; the stale ones must
/// not move `current` or the cursor, or fetch art. Returns the final index.
fn reorder_storm(app: &mut App, plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> usize {
    let expected = app.current.expect("a reorder only guards a playing track");
    let playing_thumb = app.current_track().and_then(|t| t.thumb.clone());
    let playing_key = app.current_track().map(|t| t.rating_key.clone());
    for pos in (0..app.queue.len()).filter(|&p| p != expected) {
        app.handle_msg(playlist_pos(pos));
        assert_eq!(app.current, Some(expected), "stale position {pos} moved the current track");
        assert_eq!(
            app.queue_state.selected(),
            Some(expected),
            "stale position {pos} bounced the queue cursor"
        );
        // Only the playing track may be fetched
        while let Ok(cmd) = plex.try_recv() {
            match cmd {
                PlexCmd::LoadThumb { path } => assert_eq!(
                    Some(&path),
                    playing_thumb.as_ref(),
                    "stale position {pos} fetched art for a track that is not playing"
                ),
                PlexCmd::LoadTrackInfo(key) => assert_eq!(
                    Some(&key),
                    playing_key.as_ref(),
                    "stale position {pos} fetched metadata for a track that is not playing"
                ),
                other => panic!("stale position {pos} triggered {other:?}"),
            }
        }
    }
    app.handle_msg(playlist_pos(expected));
    assert_eq!(app.current, Some(expected));
    expected
}

/// `z` off reports ~n intermediate positions; they must not fetch art
/// for other tracks or bounce the cursor.
#[tokio::test]
async fn reorder_ignores_intermediate_positions() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.art_enabled = true;
    app.tab = Tab::Queue;
    app.queue = (0..60)
        .map(|i| Track { thumb: Some(format!("/library/metadata/t{i}/thumb")), ..track(i) })
        .collect();
    app.current = Some(20);
    app.queue_state.select(Some(20));
    app.stopped = false;

    // shuffle on pins the playing entry first
    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(0));
    assert_eq!(reorder_storm(&mut app, &mut plex), 0);

    // shuffle off drags it back to slot 20
    app.handle_msg(key('z'));
    assert_eq!(app.current, Some(20), "the playing track keeps its original slot");
    assert_eq!(app.current_track().map(|t| t.rating_key.as_str()), Some("t20"));
    assert_eq!(reorder_storm(&mut app, &mut plex), 20);

    // the guard must not outlive its burst
    app.handle_msg(playlist_pos(20));
    app.handle_msg(playlist_pos(21));
    assert_eq!(app.current, Some(21), "the position guard must not wedge");
}

/// `z` off must not be quadratic: the wall-clock bound fails an O(n²) walk at
/// 50 000 tracks.
#[tokio::test]
async fn shuffle_off_restores_large_queue_quickly() {
    let mut app = new_app().await;
    app.queue = (0..50_000).map(track).collect();
    app.current = Some(31_337);
    app.queue_state.select(Some(31_337));
    app.stopped = false;
    let original: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();

    let started = std::time::Instant::now();
    app.handle_msg(key('z')); // shuffle on (snapshot + reorder)
    app.handle_msg(key('z')); // ... and back off (the restore path)
    let elapsed = started.elapsed();

    let restored: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();
    assert_eq!(restored, original, "z off restores the original order");
    assert_eq!(app.current, Some(31_337), "playback stays on the same track");
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "shuffle off took {elapsed:?} for 50 000 tracks"
    );
}

/// Applies `needle` as the queue filter, matching on the title.
fn filter_queue(app: &mut App, needle: &str) {
    let titles: Vec<String> = app.queue.iter().map(|t| t.title.clone()).collect();
    app.queue_filter.text = needle.into();
    app.queue_filter.apply(titles.len(), |i| titles[i].contains(needle));
}

/// A track with the given title; artist and album stay x-free so a one-letter
/// filter matches only the title (the real filter checks all three columns).
fn named(i: usize, title: &str) -> Track {
    Track { title: title.into(), artist: "Artist".into(), album: "Album".into(), ..track(i) }
}

/// `i` is not a Queue binding.
#[tokio::test]
async fn i_in_the_queue_does_nothing() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..4).map(track).collect();
    app.current = Some(1);
    app.queue_state.select(Some(2));
    app.stopped = false;

    let before: Vec<String> = app.queue.iter().map(|t| t.rating_key.clone()).collect();
    app.handle_msg(key('i'));

    assert_eq!(
        app.queue.iter().map(|t| t.rating_key.clone()).collect::<Vec<_>>(),
        before,
        "`i` must not insert anything in the Queue"
    );
    assert_eq!(app.queue_state.selected(), Some(2));
    assert_eq!(app.current, Some(1));
}

/// Under a filter `z`/`Z` must keep the highlight on the same track.
#[tokio::test]
async fn shuffle_keeps_highlighted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.current = Some(20);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    // real 12 ("Starfall! 12") is the 4th matching row
    let cursor = app.queue_filter.cursor_of(12).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    for label in ["shuffle on", "shuffle off"] {
        app.handle_msg(key('z'));
        let selected = app.queue_state.selected().expect("a row stays highlighted");
        let real = app.queue_filter.real(selected, app.queue.len());
        assert_eq!(
            app.queue[real].rating_key, "t12",
            "{label} moved the highlight off the selected track"
        );
    }
}

/// `>` while stopped must resume the highlighted row under a filter, not the row at that raw index.
#[tokio::test]
async fn stopped_next_resumes_highlighted_row_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.stopped = true;

    filter_queue(&mut app, "Starfall! 1");
    // real 12 ("Starfall! 12") is the 4th matching row
    let cursor = app.queue_filter.cursor_of(12).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 12, "the test filter must make cursor and index differ");

    app.handle_msg(key('>'));
    assert_eq!(app.current, Some(12), "`>` must resume the track the cursor highlights");
    assert_eq!(app.queue[12].rating_key, "t12");
}

/// Under a filter Enter must leave the highlight on the row it started.
#[tokio::test]
async fn play_queue_index_highlights_filtered_row() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..50).map(track).collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "Starfall! 1");
    // real 13 ("Starfall! 13") is the 5th matching row
    let cursor = app.queue_filter.cursor_of(13).expect("row is visible");
    app.queue_state.select(Some(cursor));
    assert_ne!(cursor, 13, "the test filter must make cursor and index differ");

    app.handle_msg(enter());

    assert_eq!(app.current, Some(13), "Enter must still start the highlighted track");
    assert_eq!(app.queue[13].rating_key, "t13");
    assert_eq!(
        app.queue_state.selected(),
        Some(cursor),
        "the highlight must stay on the row showing the started track"
    );
}

/// A reorder carries `failed_at` to the failed track's new slot; paths
/// without an exact mapping clear it.
#[tokio::test]
async fn reorder_carries_failure_marker_with_track() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..5).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(2));
    app.stopped = false;

    app.handle_msg(end_file_error());
    assert_eq!(app.failed_at, Some(2), "the broken track is remembered");

    app.handle_msg(key('K'));
    assert_eq!(app.failed_at, Some(1), "a single move remaps the marker");
    assert_eq!(app.queue[1].rating_key, "t2");

    app.handle_msg(key('z'));
    let marker = app.failed_at.expect("a permutation keeps the marker");
    assert_eq!(
        app.queue[marker].rating_key, "t2",
        "shuffle must leave the marker on the failed track"
    );

    app.failed_at = Some(1);
    app.remove_from_queue(3);
    assert_eq!(app.failed_at, None, "a removal clears the marker");
}

/// Under a filter `i` must highlight the inserted track.
#[tokio::test]
async fn play_next_highlights_inserted_track_under_filter() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.queue = ["Apple", "Box", "Cat", "Fox", "Goat"]
        .iter()
        .enumerate()
        .map(|(i, title)| named(i, title))
        .collect();
    app.current = Some(0);
    app.stopped = false;

    filter_queue(&mut app, "x");

    app.browser.tracks = vec![named(90, "Xylophone")];
    app.browser.tracks_state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;

    app.handle_msg(key('i'));

    assert_eq!(app.queue[1].title, "Xylophone", "queued after the current track");
    let cursor = app.queue_state.selected().expect("a row is highlighted");
    assert_eq!(cursor, app.queue_filter.cursor_of(1).expect("the inserted track is visible"));
    assert_eq!(
        app.queue_filter.real(cursor, app.queue.len()),
        1,
        "the highlight must stay on the inserted track"
    );
}

/// `g` only steals letters: `g 2` still switches tabs and `gg` is still Top.
#[tokio::test]
async fn g_keeps_non_letter_bindings() {
    let mut app = new_app().await;
    app.tab = Tab::Playlists;
    app.playlists.playlists = ["Bravo", "Charlie", "Delta"]
        .iter()
        .enumerate()
        .map(|(i, title)| Playlist { title: (*title).into(), ..playlist(i) })
        .collect();
    app.playlists.state.select(Some(2));

    app.handle_msg(key('g'));
    app.handle_msg(key('b'));
    assert_eq!(app.playlists.state.selected(), Some(0), "'g b' jumps");
    assert_eq!(app.tab, Tab::Playlists);

    app.handle_msg(key('g'));
    app.handle_msg(key('2'));
    assert_eq!(app.tab, Tab::Browser, "'g 2' must still switch tabs");

    app.tab = Tab::Playlists;
    app.playlists.state.select(Some(2));
    app.handle_msg(key('g'));
    app.handle_msg(key('g'));
    assert_eq!(app.playlists.state.selected(), Some(0), "'gg' goes to the top");
}

/// The sleep timer firing while paused must not leave `paused` set.
#[tokio::test]
async fn sleep_timer_clears_a_pending_pause() {
    let mut app = new_app().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.paused = true;

    app.handle_msg(key('S'));
    app.handle_msg(Msg::SleepTimer { epoch: 1 });

    assert!(app.stopped, "the timer stops playback");
    assert!(!app.paused, "the timer must not leave the player paused");
}

// request order for queue changes

/// Sequence numbers of the queue requests the UI sent, oldest first.
fn resolve_seqs(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Vec<u64> {
    let mut out = Vec::new();
    while let Ok(cmd) = plex.try_recv() {
        match cmd {
            PlexCmd::ResolvePlaylist { seq, .. }
            | PlexCmd::ResolveTrack { seq, .. }
            | PlexCmd::ResolveAlbum { seq, .. }
            | PlexCmd::ResolveArtistTracks { seq, .. } => out.push(seq),
            _ => {}
        }
    }
    out
}

fn reply(seq: u64, outcome: QueueOutcome) -> Msg {
    Msg::QueueReply { seq, outcome }
}

/// A resolved reply of tracks `range`, so the queue shows which one won.
fn tracks_reply(seq: u64, range: std::ops::Range<usize>, action: QueueAction) -> Msg {
    reply(
        seq,
        QueueOutcome::Tracks {
            tracks: range.map(track).collect(),
            confirm: Confirm::Playlist { name: "x".into() },
            action,
        },
    )
}

fn queue_keys(app: &App) -> Vec<String> {
    app.queue.iter().map(|t| t.rating_key.clone()).collect()
}

fn on_playlist(app: &mut App, i: usize) {
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.state.select(Some(i));
    app.playlists.focus = PlaylistsCol::List;
    app.tab = Tab::Playlists;
}

/// With the same song twice in a playlist, the request names the entry under the cursor.
#[tokio::test]
async fn enter_on_repeated_song_names_entry() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    let mut first = track(7);
    first.playlist_item_id = Some("101".into());
    let mut second = track(7); // the same song again, a different entry
    second.playlist_item_id = Some("202".into());
    app.playlists.items = vec![first, second];
    app.playlists.items_for = Some("p0".into());
    app.playlists.items_state.select(Some(1));
    app.playlists.focus = PlaylistsCol::Items;
    app.handle_msg(enter());

    let mut start = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { start: s, .. } = cmd {
            start = s;
        }
    }
    assert_eq!(start.as_deref(), Some("202"));
}

#[tokio::test]
async fn later_replace_wins_when_replies_reversed() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    on_playlist(&mut app, 1);
    app.handle_msg(enter());
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 2);

    app.handle_msg(tracks_reply(seqs[1], 10..12, QueueAction::Replace));
    app.handle_msg(tracks_reply(seqs[0], 20..25, QueueAction::Replace));

    assert_eq!(queue_keys(&app), ["t10", "t11"], "the slow, older replace must be dropped");
}

#[tokio::test]
async fn appends_apply_in_request_order() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 1);
    app.handle_msg(key('a'));
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 2);

    app.handle_msg(tracks_reply(seqs[1], 10..12, QueueAction::Append));
    assert!(app.queue.is_empty(), "the second append waits for the first");
    app.handle_msg(tracks_reply(seqs[0], 20..22, QueueAction::Append));

    assert_eq!(queue_keys(&app), ["t20", "t21", "t10", "t11"]);
}

#[tokio::test]
async fn failed_request_does_not_block_next_append() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 1);
    app.handle_msg(key('a'));
    on_playlist(&mut app, 2);
    app.handle_msg(key('a'));
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 3);

    app.handle_msg(tracks_reply(seqs[2], 30..31, QueueAction::Append));
    app.handle_msg(reply(seqs[1], QueueOutcome::Failed("resolving playlist: boom".into())));
    assert!(app.queue.is_empty());
    app.handle_msg(tracks_reply(seqs[0], 10..11, QueueAction::Append));

    assert_eq!(queue_keys(&app), ["t10", "t30"]);
}

#[tokio::test]
async fn clear_drops_replace_still_loading() {
    for by_key in [false, true] {
        let (mut app, mut plex) = new_app_with_plex().await;
        on_playlist(&mut app, 0);
        app.handle_msg(enter());
        let seq = resolve_seqs(&mut plex)[0];

        if by_key {
            app.tab = Tab::Queue;
            app.handle_msg(key('D'));
        } else {
            run_cmd(&mut app, "clear");
        }
        app.handle_msg(tracks_reply(seq, 10..15, QueueAction::Replace));

        let how = if by_key { "D" } else { ":clear" };
        assert!(app.queue.is_empty(), "{how}: the late reply refilled the cleared queue");
        assert_eq!(app.current, None, "{how}");
        assert!(app.stopped, "{how}: nothing may start playing");
    }
}

#[tokio::test]
async fn local_replace_drops_loading_playlist() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    let seq = resolve_seqs(&mut plex)[0];

    app.tab = Tab::Browser;
    app.browser.tracks = (0..4).map(track).collect();
    app.browser.tracks_state.select(Some(1));
    app.browser.focus = BrowserCol::Tracks;
    app.handle_msg(enter());
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"]);

    app.handle_msg(tracks_reply(seq, 20..30, QueueAction::Replace));
    assert_eq!(queue_keys(&app), ["t1", "t2", "t3"], "the late playlist must be dropped");
}

#[tokio::test]
async fn local_append_lands_after_slower_remote_append() {
    let (mut app, mut plex) = new_app_with_plex().await;
    on_playlist(&mut app, 0);
    app.handle_msg(key('a'));
    let seq = resolve_seqs(&mut plex)[0];

    app.tab = Tab::Browser;
    app.browser.tracks = (0..4).map(track).collect();
    app.browser.tracks_state.select(Some(2));
    app.browser.focus = BrowserCol::Tracks;
    app.handle_msg(key('a'));
    assert!(app.queue.is_empty(), "the local append waits for the playlist asked first");
    assert!(app.status.contains("queued after"), "{:?}", app.status);

    app.handle_msg(tracks_reply(seq, 20..22, QueueAction::Append));
    assert_eq!(queue_keys(&app), ["t20", "t21", "t2"]);
}

/// One frame as rows of cell symbols, for measuring blank cells.
fn grid(app: &mut App, w: u16, h: u16) -> Vec<Vec<String>> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::draw(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect())
        .collect()
}

fn queue_app_with_art(app: &mut App, art: bool) {
    app.art_enabled = art;
    app.picker = art.then(|| ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.queue = (0..30).map(track).collect();
    app.queue_state.select(Some(2));
    app.current = Some(2);
    app.stopped = false;
    app.tab = Tab::Queue;
}

/// The queue's text sits one cell off the art divider, as on the right border.
#[tokio::test]
async fn queue_text_gutter_matches_both_sides() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    for w in [80u16, 100, 120, 161, 200] {
        let g = grid(&mut app, w, 30);
        let hy = g.iter().position(|r| r.concat().contains("Title")).expect("table header");
        let row = &g[hy];
        let divider = (2..w as usize - 1).find(|&x| row[x] == "│").expect("art divider");
        let first = (divider + 1..w as usize - 1).find(|&x| row[x] != " ").unwrap();
        let last = (divider + 1..w as usize - 1).rev().find(|&x| row[x] != " ").unwrap();
        assert_eq!(first - divider - 1, 1, "left gutter at width {w}");
        assert_eq!(w as usize - 2 - last, 1, "right gutter at width {w}");
    }
}

/// Bars are centred between the pane's edges; an odd leftover cell is always
/// on the right, in both modes, art on or off.
#[tokio::test]
async fn visualizer_is_centred_in_its_pane() {
    let mut app = new_app().await;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 96];
    for art in [false, true] {
        for mode in ["adaptive", "spread"] {
            app.visualizer_mode = mode.into();
            queue_app_with_art(&mut app, art);
            for w in 40u16..=250 {
                let g = grid(&mut app, w, 30);
                let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
                let first = row.iter().position(|c| c == "█").unwrap();
                let edge_l = row[..first].iter().rposition(|c| c == "│").unwrap();
                let edge_r = row.iter().rposition(|c| c == "│").unwrap();
                let last = row.iter().rposition(|c| c == "█").unwrap();
                let (l, r) = (first - edge_l - 1, edge_r - last - 1);
                let case = format!("art={art} {mode} w={w}: blank {l} left, {r} right");
                assert!(l.abs_diff(r) <= 1, "{case}");
                assert!((l + r) % 2 == 1 || l == r, "{case}");
                assert!(l <= r, "{case}");
            }
        }
    }
}

/// Spread mode draws the set number of bars once they fit (fewer before), with
/// even bars and gaps; the anchor keeps its size and the other grows.
#[tokio::test]
async fn spread_visualizer_even_bars_and_gaps() {
    let mut app = new_app().await;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 96];
    queue_app_with_art(&mut app, false);
    app.visualizer_mode = "spread".into();
    app.visualizer_bars = 12;
    app.visualizer_bar_width = 2;
    app.visualizer_bar_gap = 1;
    for anchor in ["gap", "bar"] {
        app.visualizer_anchor = anchor.into();
        let g = grid(&mut app, 32, 30);
        let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
        let runs =
            (0..row.len()).filter(|&x| row[x] == "█" && (x == 0 || row[x - 1] != "█")).count();
        let cells = row.iter().filter(|c| *c == "█").count();
        assert!(runs < 12 && cells >= 2 * runs, "{anchor}: {runs} bars, {cells} cells");
        let mut last = (0, 0);
        for w in [60u16, 100, 160, 250] {
            let g = grid(&mut app, w, 30);
            let row = g.iter().rev().find(|r| r.iter().any(|c| c == "█")).expect("bars");
            let (mut bars, mut gaps): (Vec<usize>, Vec<usize>) = (vec![], vec![]);
            let mut x = row.iter().position(|c| c == "█").unwrap();
            let end = row.iter().rposition(|c| c == "█").unwrap();
            while x <= end {
                let on = row[x] == "█";
                let len = row[x..=end].iter().take_while(|c| (*c == "█") == on).count();
                if on {
                    bars.push(len)
                } else {
                    gaps.push(len)
                }
                x += len;
            }
            let case = format!("{anchor} w={w}: bars {bars:?} gaps {gaps:?}");
            assert_eq!(bars.len(), 12, "{case}");
            assert!(
                bars.iter().all(|&b| b == bars[0]) && gaps.iter().all(|&x| x == gaps[0]),
                "{case}"
            );
            let (bar, gap) = (bars[0], gaps[0]);
            match anchor {
                "gap" => assert_eq!(gap, 1, "{case}"),
                _ => assert_eq!(bar, 2, "{case}"),
            }
            assert!(bar >= last.0 && gap >= last.1, "{case}: grows with the pane");
            last = (bar, gap);
        }
        assert!(if anchor == "gap" { last.0 > 2 } else { last.1 > 1 });
    }
}

/// When the art pane is height-limited, the empty-cover box fills it; the
/// pane is cut for the image's rows, not the metadata footer's.
#[tokio::test]
async fn cover_box_fills_height_limited_pane() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.art_square = true;
    let mut checked = 0;
    for metadata in [true, false] {
        app.art_metadata = metadata;
        for (w, hs) in [(250u16, 18u16..=60), (200, 18..=50)] {
            for h in hs {
                let g = grid(&mut app, w, h);
                let corner = |r: &Vec<String>, c: &str| (1..w as usize - 1).find(|&x| r[x] == c);
                let Some(row) = g.iter().find(|r| corner(r, "┌").is_some()) else { continue };
                let bx = corner(row, "┌").unwrap();
                let rx = corner(row, "┐").unwrap();
                let divider = (rx + 1..w as usize).find(|&x| row[x] == "│").unwrap();
                let (l, r) = (bx - 1, divider - 1 - rx);
                let case = format!("{w}x{h} metadata={metadata}: gaps {l} left, {r} right");
                assert!(l <= 1 && r <= 1 && l == r, "{case}");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no frame had a cover box to measure");
}

/// Times over an hour are shown whole, right-aligned with shorter ones.
#[tokio::test]
async fn durations_over_an_hour_are_not_clipped() {
    let mut app = new_app().await;
    let long = |i: usize, secs: u64| Track { duration_ms: Some(secs * 1000), ..track(i) };
    let tracks = vec![long(0, 3725), long(1, 36_000), long(2, 599)];
    queue_app_with_art(&mut app, false);
    app.queue = tracks.clone();
    app.browser.tracks = tracks;
    app.browser.tracks_for = Some("al1".into());
    for tab in [Tab::Queue, Tab::Browser] {
        app.tab = tab;
        let frame = render(&mut app, 100, 30);
        let lines: Vec<&str> = frame.lines().collect();
        let end = |needle: &str| {
            let l = lines
                .iter()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing\n{frame}"));
            char_x(l, needle).unwrap() + needle.len()
        };
        assert_eq!(end("1:02:05"), end("10:00:00"), "{tab:?}\n{frame}");
        assert_eq!(end("9:59"), end("10:00:00"), "{tab:?}\n{frame}");
        if tab == Tab::Queue {
            assert_eq!(end("Len"), end("10:00:00"), "{frame}");
        }
    }
}

/// The title and credit line are centred on the frame at any width.
#[tokio::test]
async fn header_title_is_centred_on_the_frame() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.queue = vec![track(0)];
    app.current = Some(0);
    for w in 60u16..=250 {
        let frame = render(&mut app, w, 30);
        let lines: Vec<&str> = frame.lines().collect();
        for (row, needle) in [(1, "Starfall! 0"), (2, "Northwind Sounds - Evening Satellite")] {
            let Some(x) = char_x(lines[row], needle) else {
                // the credit is ellipsized in a narrow centre, the title never
                assert!(row == 2 && w < 100, "{needle:?} missing at width {w}\n{frame}");
                continue;
            };
            // twice the distance between the text's centre and the frame's
            let off = (2 * x + needle.len()) as isize - w as isize;
            assert!(off.abs() <= 1, "{needle:?} is {off} half-cells off centre at width {w}");
        }
    }
}

/// The bar's end cell stays put when the position gains a digit.
#[tokio::test]
async fn progress_bar_stable_when_position_gains_digit() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.duration = 7200.0;
    let mut ends = Vec::new();
    for pos in [0.0, 599.0, 600.0, 3599.0, 3600.0] {
        app.position = pos;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer();
        let text = flatten(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        // the last such line (the header shows the times too)
        let y = lines.iter().rposition(|l| l.contains("/ 2:00:00")).expect("time label") as u16;
        let last =
            (0..100).rev().find(|&x| buf.cell((x, y)).unwrap().bg != Color::Reset).expect("bar");
        ends.push(last);
    }
    assert!(ends.windows(2).all(|p| p[0] == p[1]), "bar end moved: {ends:?}");
}

// a big playlist streams in after its first page

/// Enter on the first playlist as a streaming replace: its seq and the worker's cancel flag.
fn start_load(
    app: &mut App,
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
) -> (u64, Arc<std::sync::atomic::AtomicBool>) {
    on_playlist(app, 0);
    app.handle_msg(enter());
    let mut found = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { seq, stream, cancel, .. } = cmd {
            assert!(stream, "a replace streams");
            found = Some((seq, cancel));
        }
    }
    found.expect("a ResolvePlaylist was sent")
}

/// The first page landing: 200 tracks of a 5,000-track playlist.
fn land_head(app: &mut App, seq: u64) {
    app.handle_msg(reply(
        seq,
        QueueOutcome::Streaming {
            tracks: (0..200).map(track).collect(),
            confirm: Confirm::Playlist { name: "x".into() },
            load_id: seq,
            total: 5000,
        },
    ));
}

/// Whether the playlist request the UI sent streams (`None`: it sent none).
fn resolve_streams(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Option<bool> {
    let mut out = None;
    while let Ok(cmd) = plex.try_recv() {
        if let PlexCmd::ResolvePlaylist { stream, .. } = cmd {
            out = Some(stream);
        }
    }
    out
}

fn page(load_id: u64, range: std::ops::Range<usize>, last: bool) -> Msg {
    Msg::PlaylistPage { load_id, tracks: range.map(track).collect(), last }
}

/// What mpv's playlist holds, in order, as the stream URLs the app queued.
async fn mpv_matches_queue(app: &App) -> bool {
    let list = app.mpv.get_property("playlist").await.expect("mpv playlist");
    let in_mpv: Vec<String> = list
        .as_array()
        .expect("a playlist array")
        .iter()
        .map(|e| e["filename"].as_str().unwrap_or_default().to_string())
        .collect();
    let queued: Vec<String> =
        app.queue.iter().map(|t| t.stream_url("http://127.0.0.1:1", "TOK").unwrap()).collect();
    in_mpv == queued
}

#[tokio::test]
async fn streamed_playlist_plays_at_once_and_grows() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    assert_eq!(app.queue.len(), 200);
    assert_eq!(app.current, Some(0));
    assert!(!app.stopped, "playback starts with the first page");

    app.handle_msg(page(seq, 200..230, false));
    assert!(app.status.contains("230 / 5 000"), "{}", app.status);
    app.handle_msg(page(seq, 230..260, true));

    assert_eq!(queue_keys(&app), (0..260).map(|i| format!("t{i}")).collect::<Vec<_>>());
    assert!(mpv_matches_queue(&app).await, "mpv's playlist must match the queue");
    assert!(app.status.is_empty(), "the last page clears the progress: {}", app.status);
    app.handle_msg(key('a'));
    assert_eq!(resolve_streams(&mut plex), Some(false), "an append does not stream");
}

#[tokio::test]
async fn superseded_load_ignores_late_pages() {
    for how in ["replace", "D", ":clear"] {
        let (mut app, mut plex) = new_app_with_plex().await;
        let (seq, cancel) = start_load(&mut app, &mut plex);
        land_head(&mut app, seq);
        match how {
            "replace" => {
                on_playlist(&mut app, 1);
                app.handle_msg(enter());
            }
            "D" => {
                app.tab = Tab::Queue;
                app.handle_msg(key('D'));
                app.handle_msg(key('D'));
            }
            _ => {
                run_cmd(&mut app, "clear");
                app.handle_msg(key('y'));
            }
        }
        assert!(
            cancel.load(std::sync::atomic::Ordering::Relaxed),
            "{how}: the worker must be told"
        );

        let before = queue_keys(&app);
        app.handle_msg(page(seq, 200..230, false));
        assert_eq!(queue_keys(&app), before, "{how}: a late page changed the queue");
    }
}

#[tokio::test]
async fn loading_playlist_refuses_add_shuffle_move() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.handle_msg(page(seq, 200..230, false));

    // `d` is allowed, its undo is not
    app.tab = Tab::Queue;
    app.handle_msg(key('d'));
    let before = queue_keys(&app);
    assert_eq!(before.len(), 229);

    for k in ['u', 'J', 'K', 'z', 'Z'] {
        app.handle_msg(key(k));
        assert_eq!(queue_keys(&app), before, "{k}");
        assert!(app.status.contains("still loading (230 / 5 000)"), "{k}: {}", app.status);
        assert!(!app.random, "{k}");
    }
    run_cmd(&mut app, "shuffle");
    assert!(!app.random);
    run_cmd(&mut app, "reshuffle");
    on_playlist(&mut app, 1);
    for k in ['a', 'i'] {
        app.handle_msg(key(k));
        assert!(app.status.contains("still loading"), "{k}: {}", app.status);
    }
    assert_eq!(queue_keys(&app), before);
    assert!(resolve_seqs(&mut plex).is_empty(), "no request may be sent");
    assert!(mpv_matches_queue(&app).await);

    app.handle_msg(enter());
    assert_eq!(resolve_seqs(&mut plex).len(), 1);
}

/// A playlist that fits in the first page must not stay marked as loading.
#[tokio::test]
async fn small_playlist_leaves_nothing_loading() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    app.handle_msg(reply(
        seq,
        QueueOutcome::Tracks {
            tracks: (0..60).map(track).collect(),
            confirm: Confirm::Playlist { name: "x".into() },
            action: QueueAction::Replace,
        },
    ));
    assert_eq!(app.queue.len(), 60);

    app.tab = Tab::Queue;
    app.handle_msg(key('z'));
    assert!(app.random, "shuffle works: {}", app.status);
    app.handle_msg(key('J'));
    assert!(!app.status.contains("still loading"), "{}", app.status);
}

#[tokio::test]
async fn remove_and_play_while_loading_keep_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.tab = Tab::Queue;
    app.handle_msg(key('d')); // remove t0, the playing one
    app.play_queue_index(5);
    app.handle_msg(page(seq, 200..250, false));
    app.handle_msg(page(seq, 250..260, true));

    assert_eq!(app.queue.len(), 259);
    assert_eq!(app.queue.last().unwrap().rating_key, "t259");
    assert!(mpv_matches_queue(&app).await);
    assert!(app.current.is_some_and(|c| c < app.queue.len()));
}

#[tokio::test]
async fn page_failure_keeps_loaded_tracks() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    land_head(&mut app, seq);
    app.handle_msg(page(seq, 200..230, false));
    app.handle_msg(Msg::PlaylistLoadStopped { load_id: seq, error: "boom".into() });

    assert_eq!(app.queue.len(), 230);
    assert!(app.status.contains("stopped at 230 / 5 000: boom"), "{}", app.status);
    assert!(app.status_error);
    app.tab = Tab::Playlists;
    app.handle_msg(key('a'));
    assert!(resolve_streams(&mut plex).is_some(), "{}", app.status);
}

#[tokio::test]
async fn failed_first_page_does_not_lock_queue() {
    let (mut app, mut plex) = new_app_with_plex().await;
    let (seq, _) = start_load(&mut app, &mut plex);
    app.handle_msg(Msg::PlaylistLoadStopped { load_id: seq, error: "stopped early".into() });
    app.handle_msg(reply(seq, QueueOutcome::Failed("resolving playlist: boom".into())));
    assert!(app.queue.is_empty());

    app.handle_msg(key('a'));
    assert!(resolve_streams(&mut plex).is_some(), "{}", app.status);
}

#[tokio::test]
async fn shuffled_replace_does_not_stream() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.handle_msg(key('z'));
    assert!(app.random);
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    assert_eq!(resolve_streams(&mut plex), Some(false), "shuffle needs the whole list");
}

fn album_y(key: &str, title: &str, year: Option<u32>) -> Album {
    Album { rating_key: key.into(), title: title.into(), artist: "A".into(), year, thumb: None }
}

/// Missing years sort last in both year orders; ties go by title; an unknown mode is `year`.
#[test]
fn albums_sort_by_mode_with_missing_years() {
    use shmc::ui::app::sort_albums;
    let set = || {
        vec![
            album_y("1", "beta", Some(2010)),
            album_y("2", "Zed", None),
            album_y("3", "Alpha", Some(2010)),
            album_y("4", "gamma", Some(1999)),
            album_y("5", "alpha", None),
            album_y("6", "Alpha", Some(1990)),
        ]
    };
    let keys = |mode: &str| {
        let mut v = set();
        sort_albums(&mut v, mode);
        v.iter().map(|a| a.rating_key.clone()).collect::<Vec<_>>().join("")
    };
    assert_eq!(keys("year"), "643152");
    assert_eq!(keys("year_desc"), "314652");
    assert_eq!(keys("title"), "635142");
    assert_eq!(keys("Year_Desc"), "314652");
    assert_eq!(keys("sideways"), keys("year"));
}

/// Albums are ordered by `ui.album_sort`, not the server's order.
#[tokio::test]
async fn albums_loaded_applies_the_configured_order() {
    let mut app = new_app().await;
    app.album_sort = "year_desc".into();
    app.browser.albums_for = Some("a1".into());
    app.handle_msg(Msg::AlbumsLoaded {
        artist_key: "a1".into(),
        albums: vec![
            album_y("old", "Old", Some(1990)),
            album_y("none", "None", None),
            album_y("new", "New", Some(2020)),
        ],
    });
    let keys: Vec<&str> = app.browser.albums.iter().map(|a| a.rating_key.as_str()).collect();
    assert_eq!(keys, ["new", "old", "none"]);
}

async fn browser_with_albums(albums: Vec<Album>) -> TestApp {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.artists =
        vec![Artist { rating_key: "a0".into(), title: "Browsed Artist".into(), thumb: None }];
    app.browser.artists_state.select(Some(0));
    app.browser.albums_for = Some("a0".into());
    app.browser.albums = albums;
    app.browser.albums_state.select(Some(0));
    app.browser.focus = BrowserCol::Albums;
    app
}

fn album_by(key: &str, title: &str, artist: &str, year: Option<u32>) -> Album {
    Album { rating_key: key.into(), title: title.into(), artist: artist.into(), year, thumb: None }
}

/// The rendered line holding `needle`.
fn row_with(screen: &str, needle: &str) -> String {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?}\n{screen}"))
        .to_string()
}

/// A row is `Title (year)`; the artist follows only when it differs from the
/// browsed one; no year, no parentheses.
#[tokio::test]
async fn album_rows_show_year_and_differing_artist() {
    let mut app = browser_with_albums(vec![
        album_by("1", "Mine", "Browsed Artist", Some(2019)),
        album_by("2", "Compilation", "Other Artist", Some(2020)),
        album_by("3", "Undated", "Browsed Artist", None),
    ])
    .await;
    assert!(app.browser.tracks_for.is_none());
    for w in [100u16, 140, 200] {
        let s = render(&mut app, w, 30);
        let mine = row_with(&s, "Mine");
        assert!(mine.contains("Mine (2019)"), "{mine}");
        assert!(!mine.contains("Mine (2019) -"), "{mine}");
        assert!(row_with(&s, "Compilation").contains("Compilation (2020) - Other Artist"));
        let undated = row_with(&s, "Undated");
        assert!(!undated.contains("Undated ("), "{undated}");
        assert!(!undated.contains("Undated -"), "{undated}");
    }
}

/// In a narrow pane the artist suffix goes first and the year stays, even with wide characters.
#[tokio::test]
async fn narrow_album_rows_drop_artist_first() {
    let mut app = browser_with_albums(vec![
        album_by("1", "Compilation Of Things", "Other Artist", Some(2020)),
        album_by("2", "日本語日本語日本語日本語日本語日本語", "Browsed Artist", Some(2018)),
    ])
    .await;
    for w in 50u16..=120 {
        let s = render(&mut app, w, 30);
        let comp = s.lines().find(|l| l.contains("Compilation") || l.contains("(2020)"));
        let comp = comp.unwrap_or_else(|| panic!("w={w}\n{s}"));
        assert!(comp.contains("(2020)"), "w={w}: year lost\n{comp}");
        if !comp.contains("Other") {
            assert!(!comp.contains(" - "), "w={w}: {comp}");
        }
        let cjk = s.lines().find(|l| l.contains("(2018)"));
        assert!(cjk.is_some(), "w={w}: wide title pushed the year out\n{s}");
    }
    // the suffix really is dropped somewhere in the range
    let s = render(&mut app, 60, 30);
    let comp = row_with(&s, "(2020)");
    assert!(!comp.contains("Other"), "{comp}");
}

/// Five queued tracks, `current` playing, with mpv commands sent to the returned receiver.
async fn playing_app(
    current: usize,
    position: f64,
) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<mpv::MpvCmd>) {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..5).map(track).collect();
    app.current = Some(current);
    app.stopped = false;
    app.position = position;
    let (tx, rx) = unbounded_channel();
    app.mpv.tx = tx;
    (app, rx)
}

/// Past the threshold `<` restarts the track; a second `<` goes back.
#[tokio::test]
async fn prev_restarts_track_past_threshold() {
    let (mut app, mut rx) = playing_app(2, 10.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SeekAbs(t)) if t == 0.0));
    assert_eq!(app.position, 0.0);
    assert_eq!(app.current, Some(2));
    assert_eq!(app.queue.len(), 5);

    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// Early in a track, or with the threshold at 0, `<` goes back.
#[tokio::test]
async fn prev_goes_back_early_or_threshold_off() {
    let (mut app, mut rx) = playing_app(2, 1.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
    drop(app); // the next app needs the shared lock

    let (mut app, mut rx) = playing_app(2, 30.0).await;
    app.prev_restart_secs = 0;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// On the first entry `<` restarts (mpv would stop), or wraps with repeat on.
#[tokio::test]
async fn prev_on_first_entry_restarts_unless_repeat() {
    let (mut app, mut rx) = playing_app(0, 1.0).await;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::SeekAbs(t)) if t == 0.0));
    assert!(!app.stopped);
    assert_eq!(app.current, Some(0));
    drop(app);

    let (mut app, mut rx) = playing_app(0, 1.0).await;
    app.loop_playlist = true;
    app.handle_msg(key('<'));
    assert!(matches!(rx.try_recv(), Ok(mpv::MpvCmd::Prev)));
}

/// Restores the process-wide palette when a test ends; tests hold `TAB_TEST_LOCK`.
struct PaletteGuard(theme::Theme);

impl PaletteGuard {
    fn set(palette: theme::Theme) -> Self {
        let previous = theme::theme();
        theme::set_theme(palette);
        PaletteGuard(previous)
    }
}

impl Drop for PaletteGuard {
    fn drop(&mut self) {
        theme::set_theme(self.0);
    }
}

fn preset_theme(name: &str) -> theme::Theme {
    shmc::ui::themes::preset(name).unwrap().theme
}

/// A background fills every unstyled cell on every tab and changes nothing else.
#[tokio::test]
async fn background_fills_unstyled_cells_on_every_tab() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 20];
    app.browser.artists = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Artist {i:02}"),
            thumb: None,
        })
        .collect();
    app.browser.artists_state.select(Some(3));

    for name in ["catppuccin-latte", "nord"] {
        let themed = preset_theme(name);
        let bg = themed.background.expect("a preset with a background");
        let plain = theme::Theme { background: None, ..themed };
        for tab in [Tab::Queue, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help] {
            app.tab = tab;
            for (w, h) in [(120u16, 40u16), (80, 24), (50, 14)] {
                let draw = |app: &mut App, palette| {
                    let _g = PaletteGuard::set(palette);
                    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                    terminal.draw(|f| views::draw(f, app)).unwrap();
                    terminal.backend().buffer().clone()
                };
                let without = draw(&mut app, plain);
                let with = draw(&mut app, themed);
                let mut holes = 0;
                for (a, b) in without.content().iter().zip(with.content()) {
                    let expected = if a.bg == Color::Reset { bg } else { a.bg };
                    assert_eq!(b.bg, expected, "{name} {tab:?} {w}x{h}");
                    holes += usize::from(b.bg == Color::Reset);
                    assert_eq!((a.symbol(), a.fg, a.modifier), (b.symbol(), b.fg, b.modifier));
                    // text in the terminal's own colour vanishes on a light preset
                    assert!(
                        b.fg != Color::Reset || b.symbol().trim().is_empty(),
                        "{name} {tab:?} {w}x{h}: {:?} is drawn in the terminal's colour",
                        b.symbol()
                    );
                }
                assert_eq!(holes, 0, "{name} {tab:?} {w}x{h}: cells left in the terminal's colour");
                // with no background the margin has no bg at all
                assert_eq!(without.content()[0].bg, Color::Reset);
            }
        }
    }
}

/// A background neither re-emits the cover nor erases it.
#[tokio::test]
async fn background_leaves_cover_painted_once() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::from_fontsize((10, 20)));
    app.art.img = Some(Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8))));

    // without a background
    let unpainted = {
        let _g = PaletteGuard::set(theme::Theme { background: None, ..theme::theme() });
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|f| views::draw(f, &mut app)).unwrap();
        terminal.backend().buffer().clone()
    };
    app.art.protocol = None;
    app.art.prepared_rect = None;
    app.art.shown_rect = None;

    // with one
    let _g = PaletteGuard::set(preset_theme("nord"));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let rect = app.art.shown_rect.expect("cover was not painted");
    let first = terminal.backend().buffer().clone();
    assert!(
        (rect.top()..rect.bottom())
            .flat_map(|y| (rect.left()..rect.right()).map(move |x| (x, y)))
            .any(|p| first.cell(p).is_some_and(|c| c.symbol() != " ")),
        "the first frame carries the image"
    );
    // the letterbox must be opaque: some terminals draw transparent pixels over
    // their own background (a dark band on a light theme); the corner is padding
    let corner = first.cell((rect.left(), rect.top())).unwrap();
    let bg = Color::Rgb(0x2e, 0x34, 0x40);
    assert!(corner.fg == bg && corner.bg == bg, "padding is {:?}/{:?}", corner.fg, corner.bg);
    let plain = unpainted.cell((rect.left(), rect.top())).unwrap();
    assert!(plain.fg != bg && plain.bg != bg, "without a background it is not painted");

    let mut skipped = true;
    terminal
        .draw(|f| {
            views::draw(f, &mut app);
            let buf = f.buffer_mut();
            for y in rect.top()..rect.bottom() {
                for x in rect.left()..rect.right() {
                    skipped &= buf.cell((x, y)).is_some_and(|c| c.skip);
                }
            }
        })
        .unwrap();
    assert!(skipped, "cover cells were not skipped on the second frame");
    // the backend buffer still holds the first frame's image cells
    let after = terminal.backend().buffer();
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            assert_eq!(after.cell((x, y)), first.cell((x, y)), "({x},{y}) was rewritten");
        }
    }
}

/// Columns where each label starts, on the first line with them all.
fn header_cols(frame: &str, labels: &[&str]) -> Vec<usize> {
    for line in frame.lines() {
        if labels.iter().all(|l| line.contains(l)) {
            return labels.iter().map(|l| line[..line.find(l).unwrap()].chars().count()).collect();
        }
    }
    panic!("no header {labels:?} in the frame\n{frame}");
}

/// Column where `text` starts on the first row below `header`'s line, if any.
fn cell_col(frame: &str, header: &str, text: &str) -> Option<usize> {
    let line =
        frame.lines().skip_while(|l| !l.contains(header)).skip(1).find(|l| l.contains(text))?;
    Some(line[..line.find(text).unwrap()].chars().count())
}

#[derive(Clone, Copy, Debug)]
enum Table {
    Queue,
    Playlist,
    Results,
    Detail,
}

const TABLES: [Table; 4] = [Table::Queue, Table::Playlist, Table::Results, Table::Detail];

impl Table {
    fn labels(self) -> [&'static str; 4] {
        match self {
            // "Type / Artist" is cut when its column is narrow
            Table::Results => ["Type", "Title", "Album", "Len"],
            _ => ["Title", "Artist", "Album", "Len"],
        }
    }
}

async fn app_with_every_table() -> TestApp {
    let mut app = new_app().await;
    app.queue = (0..50).map(track).collect();
    app.queue_state.select(Some(7));
    app.playlists.playlists = (0..3).map(playlist).collect();
    app.playlists.items = (0..12).map(track).collect();
    app.playlists.items_state.select(Some(5));
    app.playlists.focus = PlaylistsCol::Items;
    app.playlists.items_for = Some("p0".into());
    let c: shmc::plex::models::MediaContainer =
        quick_xml::de::from_str(include_str!("fixtures/hubs_search_starfall.xml")).unwrap();
    app.search.hits = c.into_search_hits();
    app.search.state.select(Some(0));
    app
}

fn render_table(app: &mut App, table: Table, w: u16, h: u16) -> String {
    app.tab = match table {
        Table::Queue => Tab::Queue,
        Table::Playlist => Tab::Playlists,
        Table::Results | Table::Detail => Tab::Search,
    };
    app.search.detail = None;
    if let Table::Detail = table {
        let mut state = TableState::default();
        state.select(Some(0));
        app.search.detail = Some(SearchDetail {
            key: "al1".into(),
            label: "Evening Satellite".into(),
            tracks: (0..12).map(track).collect(),
            state,
        });
    }
    render(app, w, h)
}

/// Header columns of each table at three widths, at the default weights.
#[tokio::test]
async fn default_column_weights_unchanged() {
    let mut app = app_with_every_table().await;
    assert_eq!(app.column_weights, [30, 24, 46]);
    type Starts = [(u16, [usize; 4]); 3];
    let expected: [(Table, Starts); 4] = [
        (Table::Queue, [(60, [2, 19, 33, 53]), (100, [2, 31, 55, 93]), (160, [2, 49, 87, 153])]),
        (
            Table::Playlist,
            [(60, [26, 33, 40, 55]), (100, [40, 60, 75, 95]), (160, [61, 92, 118, 155])],
        ),
        (Table::Results, [(60, [2, 16, 38, 55]), (100, [2, 26, 62, 95]), (160, [2, 40, 98, 155])]),
        (Table::Detail, [(60, [2, 22, 36, 55]), (100, [2, 36, 60, 95]), (160, [2, 56, 94, 155])]),
    ];
    for (table, widths) in expected {
        for (w, cols) in widths {
            let frame = render_table(&mut app, table, w, 40);
            assert_eq!(header_cols(&frame, &table.labels()), cols, "{table:?} at {w}");
        }
    }
}

/// One setting drives all four tables: a heavy Title widens it in each, and a
/// weight of 0 hides that column with the rest still under their headers.
#[tokio::test]
async fn column_weights_apply_to_every_track_table() {
    let mut app = app_with_every_table().await;
    for table in TABLES {
        // Title's width is the gap to the next column; Results has Title second.
        let title_w = |app: &mut App, w: u16| {
            let c = header_cols(&render_table(app, table, w, 40), &table.labels());
            match table {
                Table::Results => c[2] - c[1],
                _ => c[1] - c[0],
            }
        };
        let base = title_w(&mut app, 120);
        app.column_weights = [80, 10, 10];
        assert!(title_w(&mut app, 120) > base + 10, "{table:?}: a heavier Title is wider");
        app.column_weights = [30, 24, 46];
    }

    for (weights, hidden) in
        [([50, 0, 50], "Artist"), ([0, 40, 60], "Title"), ([60, 40, 0], "Album")]
    {
        app.column_weights = weights;
        for table in TABLES {
            for (w, h) in [(60, 24), (100, 40), (160, 70)] {
                let frame = render_table(&mut app, table, w, h);
                let labels = table.labels();
                let gone = match (hidden, table) {
                    ("Artist", Table::Results) => 0,
                    ("Artist", _) => 1,
                    ("Title", Table::Results) => 1,
                    ("Title", _) => 0,
                    _ => 2,
                };
                for (i, label) in labels.iter().enumerate() {
                    let header = frame.lines().find(|l| l.contains("Len")).unwrap();
                    assert_eq!(
                        header.contains(label),
                        i != gone,
                        "{table:?} {weights:?} {w}x{h}: {label}\n{frame}"
                    );
                }
                let cells = match table {
                    Table::Results => ["Nor", "Sta", "Eve"],
                    _ => ["Sta", "Nor", "Eve"],
                };
                for (i, text) in cells.iter().enumerate() {
                    let found = cell_col(&frame, labels[3], text);
                    // the Results artist cell leads with "♪ "; album names there can start like an artist's
                    let lead = if matches!(table, Table::Results) && i == 0 { 2 } else { 0 };
                    if i != gone {
                        assert_eq!(
                            found,
                            Some(header_cols(&frame, &[labels[i]])[0] + lead),
                            "{table:?} {weights:?} {w}x{h}: {text} not under {}\n{frame}",
                            labels[i]
                        );
                    } else if !matches!(table, Table::Results) {
                        assert_eq!(
                            found, None,
                            "{table:?} {weights:?} {w}x{h}: {text} still drawn\n{frame}"
                        );
                    }
                }
                assert!(
                    frame.contains("4:17"),
                    "{table:?} {weights:?} {w}x{h}: Len clipped\n{frame}"
                );
            }
        }
    }
}

/// Restores the process-wide look when a test ends; tests hold `TAB_TEST_LOCK`.
struct LookGuard(theme::Look);

impl LookGuard {
    fn set(look: theme::Look) -> Self {
        let previous = theme::look();
        theme::set_look(look);
        LookGuard(previous)
    }
}

impl Drop for LookGuard {
    fn drop(&mut self) {
        theme::set_look(self.0);
    }
}

fn spaced(block_gap: u16, inset: u16) -> theme::Look {
    theme::Look {
        spacing: theme::Spacing { block_gap, inset, ..theme::Spacing::DEFAULT },
        ..theme::Look::DEFAULT
    }
}

fn with_symbols(set: &[(&str, &str)]) -> theme::Look {
    let mut look = theme::Look::DEFAULT;
    for (key, value) in set {
        assert!(theme::set_symbol(&mut look, key, value), "{key}");
    }
    look
}

fn buffer_of(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

const TABS: [Tab; 5] = [Tab::Queue, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help];

/// Default spacing: header rows 0-3, tab bar 4-6, content down to three rows
/// from the bottom, progress box last, one cell of padding.
#[tokio::test]
async fn default_spacing_unchanged() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    assert_eq!(theme::look(), theme::Look::DEFAULT);
    for h in [16u16, 30, 60] {
        let g = grid(&mut app, 80, h);
        let tops: Vec<u16> = (0..h).filter(|&y| g[y as usize][0] == "┌").collect();
        assert_eq!(tops, [0, 4, 7, h - 3], "boxes at {h} rows");
        assert_eq!(g[1].iter().position(|c| c == "[").unwrap(), 2, "padding of one");
    }
}

/// `block_gap` puts empty rows between the blocks; it gives way before the content's six rows.
#[tokio::test]
async fn block_gap_yields_to_content() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    for gap in 1..=3u16 {
        let _look = LookGuard::set(spaced(gap, 1));
        for h in [16u16, 18, 22, 30, 45, 60] {
            let g = grid(&mut app, 80, h);
            let eff = gap.min(h.saturating_sub(16) / 3);
            let tops: Vec<u16> = (0..h).filter(|&y| g[y as usize][0] == "┌").collect();
            let content = 7 + 2 * eff;
            assert_eq!(tops, [0, 4 + eff, content, h - 3], "gap {gap} at {h} rows");
            assert!(h - 3 - eff - content >= 6, "content keeps six rows at {h}");
            let blank = |ys: std::ops::Range<u16>| {
                ys.for_each(|y| assert!(g[y as usize].iter().all(|c| c == " "), "row {y} at {h}"))
            };
            blank(4..4 + eff);
            blank(7 + eff..content);
            blank(h - 3 - eff..h - 3);
        }
    }
}

/// `inset` pads every box; at any width the content box keeps both borders.
#[tokio::test]
async fn narrow_inset_box_never_clipped() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    for inset in 0..=3u16 {
        let _look = LookGuard::set(spaced(0, inset));
        for tab in TABS {
            app.tab = tab;
            for w in [8u16, 12, 20, 30, 80, 120] {
                let g = grid(&mut app, w, 30);
                for (y, row) in g.iter().enumerate().take(27).skip(7) {
                    let edges = ["│", "┌", "┐", "└", "┘"];
                    assert!(
                        edges.contains(&row[0].as_str())
                            && edges.contains(&row[w as usize - 1].as_str()),
                        "{tab:?} inset {inset} w={w} row {y}: {:?}",
                        row.concat()
                    );
                }
            }
        }
        app.tab = Tab::Queue;
        let g = grid(&mut app, 80, 30);
        assert_eq!(
            g[1].iter().position(|c| c == "[").unwrap(),
            1 + inset as usize,
            "header, inset {inset}"
        );
    }
}

/// A gap and an inset move the cover's rect; the canvas is rebuilt and then skipped.
#[tokio::test]
async fn gap_and_inset_move_cover_then_skipped() {
    let (mut app, mut terminal) = app_with_cover().await;
    let before = app.art.shown_rect.unwrap();
    let _look = LookGuard::set(spaced(2, 2));
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    let after = app.art.shown_rect.expect("the cover moved, it did not vanish");
    assert_ne!(before, after);
    assert!(after.y >= before.y + 2, "{before:?} -> {after:?}: the gap pushes it down");
    assert!(after.x <= before.x + 1, "{before:?} -> {after:?}");
    assert_eq!(app.art.prepared_rect, Some(after), "the canvas is for the new rect");

    let mut skipped = true;
    terminal
        .draw(|f| {
            views::draw(f, &mut app);
            let buf = f.buffer_mut();
            for y in after.top()..after.bottom() {
                for x in after.left()..after.right() {
                    skipped &= buf.cell((x, y)).is_some_and(|c| c.skip);
                }
            }
        })
        .unwrap();
    assert!(skipped, "cover cells were not skipped on the second frame");
    assert_eq!(app.art.shown_rect, Some(after), "the same rect is kept, not re-prepared");
}

/// Progress glyphs draw in the foreground; with none set the bar is solid colour.
#[tokio::test]
async fn progress_glyphs_only_when_theme_sets_them() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.stopped = false;
    app.paused = false;
    app.position = 100.0;
    app.duration = 200.0;
    let accent = theme::theme().accent;
    let bar = |buf: &ratatui::buffer::Buffer, glyphs: &str| -> Vec<usize> {
        // from column 4: the transport glyph before the bar is `>` in ASCII
        (4..80usize)
            .filter(|&x| glyphs.contains(buf.cell((x as u16, 28)).unwrap().symbol()))
            .collect()
    };

    let solid = buffer_of(&mut app, 80, 30);
    assert!(bar(&solid, "#>-").is_empty());
    let lit: Vec<usize> =
        (4..80).filter(|&x| solid.cell((x as u16, 28)).unwrap().bg == accent).collect();
    assert!(!lit.is_empty(), "the played part is a coloured background");

    let _look = LookGuard::set(with_symbols(&[
        ("progress_filled", "#"),
        ("progress_empty", "-"),
        ("progress_head", ">"),
    ]));
    let themed = buffer_of(&mut app, 80, 30);
    let cells = bar(&themed, "#>-");
    let filled = bar(&themed, "#>");
    assert_eq!(cells.len(), lit.len() + bar(&themed, "-").len(), "the same cells as the solid bar");
    assert_eq!(filled.len(), lit.len(), "half played, as before");
    let head = bar(&themed, ">");
    assert_eq!(head, [*filled.last().unwrap()], "the head is the last played cell");
    let at = |x: usize| themed.cell((x as u16, 28)).unwrap().clone();
    assert_eq!(at(filled[0]).fg, accent);
    assert_eq!(at(*cells.last().unwrap()).fg, theme::theme().dim);
    assert_eq!(at(filled[0]).bg, Color::Reset, "a glyph is drawn in the foreground");
}

/// Caps take cells from the bar and take the colour of the part they touch; a
/// head over a solid track has the track as background.
#[tokio::test]
async fn progress_caps_and_head_take_theme_colours() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.stopped = false;
    app.paused = false;
    app.duration = 200.0;
    let (accent, track) = (theme::theme().accent, theme::border().fg.unwrap());
    let find = |buf: &ratatui::buffer::Buffer, glyph: &str| -> Vec<ratatui::buffer::Cell> {
        (0..80u16)
            .map(|x| buf.cell((x, 28)).unwrap().clone())
            .filter(|c| c.symbol() == glyph)
            .collect()
    };
    let bar_cells = |buf: &ratatui::buffer::Buffer| {
        (0..80u16).filter(|&x| [accent, track].contains(&buf.cell((x, 28)).unwrap().bg)).count()
    };
    app.position = 100.0;
    let solid = bar_cells(&buffer_of(&mut app, 80, 30));

    let _look = LookGuard::set(with_symbols(&[("progress_bar", "rounded")]));
    for (position, start, end) in
        [(100.0, accent, track), (0.0, track, track), (200.0, accent, accent)]
    {
        app.position = position;
        let buf = buffer_of(&mut app, 80, 30);
        let (a, b) = (find(&buf, "\u{e0b6}"), find(&buf, "\u{e0b4}"));
        assert_eq!((a.len(), b.len()), (1, 1), "one cap each at {position}");
        assert_eq!((a[0].fg, b[0].fg), (start, end), "cap colours at {position}");
        assert_eq!((a[0].bg, b[0].bg), (Color::Reset, Color::Reset), "no background of their own");
        assert_eq!(bar_cells(&buf), solid - 2, "the caps come out of the bar at {position}");
    }
    drop(_look);

    app.position = 100.0;
    let _look = LookGuard::set(with_symbols(&[("progress_bar", "angled")]));
    let head = find(&buffer_of(&mut app, 80, 30), "\u{e0b0}");
    assert_eq!((head.len(), head[0].fg, head[0].bg), (1, accent, track), "a head on a solid track");
    drop(_look);

    let _look = LookGuard::set(with_symbols(&[("progress_bar", "dot")]));
    let head = find(&buffer_of(&mut app, 80, 30), "●");
    assert_eq!(
        (head[0].fg, head[0].bg),
        (accent, Color::Reset),
        "over a glyph track: foreground only"
    );
}

/// The meter keeps eight cells whatever its glyphs: a ramp, or a lit/unlit pair.
#[tokio::test]
async fn volume_meter_keeps_eight_cells() {
    let mut app = new_app().await;
    app.volume = 50.0;
    let header = |app: &mut App| grid(app, 100, 30)[1].concat();
    assert!(header(&mut app).contains("Volume: ▁▂▃▄▅▆▇█ "), "default");
    {
        let _look = LookGuard::set(with_symbols(&[("volume_meter", "abcdefgh")]));
        assert!(header(&mut app).contains("Volume: abcdefgh "));
        let buf = buffer_of(&mut app, 100, 30);
        let v = (0..99u16)
            .find(|&x| {
                buf.cell((x, 1)).unwrap().symbol() == "V"
                    && buf.cell((x + 1, 1)).unwrap().symbol() == "o"
            })
            .unwrap();
        let fg = |cell: u16| buf.cell((v + 8 + cell, 1)).unwrap().fg;
        assert_eq!((fg(3), fg(4)), (theme::theme().accent, theme::theme().dim), "4 of 8 lit");
    }
    let _look = LookGuard::set(with_symbols(&[("volume_meter", "squares")]));
    assert!(header(&mut app).contains("Volume: ■■■■···· "));
    let buf = buffer_of(&mut app, 100, 30);
    let v = (0..99u16)
        .find(|&x| {
            buf.cell((x, 1)).unwrap().symbol() == "V"
                && buf.cell((x + 1, 1)).unwrap().symbol() == "o"
        })
        .unwrap();
    let cell = |i: u16| buf.cell((v + 8 + i, 1)).unwrap().clone();
    assert_eq!((cell(3).symbol(), cell(3).fg), ("■", theme::theme().accent));
    assert_eq!((cell(4).symbol(), cell(4).fg), ("·", theme::theme().dim));
    app.volume = 100.0;
    assert!(header(&mut app).contains("Volume: ■■■■■■■■ "));
}

/// With a ramp the screen is the default one with each level glyph swapped for its place.
#[tokio::test]
async fn visualizer_draws_ramp_over_eighths() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.queue.truncate(3); // no scrollbar thumb to confuse the comparison
    app.cava_alive = true;
    app.cava_frame = (0..96).map(|i| (i * 7 % 256) as u8).collect();
    let (w, h) = (80u16, 30u16);
    let plain = buffer_of(&mut app, w, h);
    let _look = LookGuard::set(with_symbols(&[("visualizer", "12345678")]));
    let themed = buffer_of(&mut app, w, h);
    let mut swapped = 0;
    for y in 7..h - 3 {
        for x in 0..w {
            let was = plain.cell((x, y)).unwrap().symbol();
            let now = themed.cell((x, y)).unwrap().symbol();
            match "▁▂▃▄▅▆▇█".chars().position(|c| c.to_string() == was) {
                Some(i) => {
                    swapped += 1;
                    assert_eq!(now, ["1", "2", "3", "4", "5", "6", "7", "8"][i], "({x},{y})");
                }
                None => assert_eq!(now, was, "({x},{y})"),
            }
        }
    }
    assert!(swapped > 10, "the bars were drawn: {swapped}");
    assert!(grid(&mut app, 100, 30)[1].concat().contains("▁▂▃▄▅▆▇█"));
}

/// The scrollbar's ends and thumb come from the symbols.
#[tokio::test]
async fn scrollbar_draws_own_glyphs() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    let plain = grid(&mut app, 80, 24);
    let _look = LookGuard::set(with_symbols(&[
        ("scroll_up", "A"),
        ("scroll_down", "V"),
        ("scroll_thumb", "#"),
    ]));
    let themed = grid(&mut app, 80, 24);
    let mut seen = 0;
    for (a, b) in plain.iter().zip(&themed).skip(7) {
        for (was, now) in a.iter().zip(b) {
            let want = match was.as_str() {
                "▲" => "A",
                "▼" => "V",
                "█" => "#",
                other => other,
            };
            seen += (want != was) as usize;
            assert_eq!(now, want);
        }
    }
    assert!(seen >= 3, "an up arrow, a thumb and a down arrow: {seen}");
}

/// A cut text ends with the ellipsis symbol in force.
#[tokio::test]
async fn cut_text_ends_with_ellipsis() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.queue[2].title = "A very long title that cannot possibly fit in the cell".into();
    let cut = |app: &mut App| {
        grid(app, 50, 24).iter().map(|r| r.concat()).find(|r| r.contains("A very long")).unwrap()
    };
    assert!(cut(&mut app).contains('…'));
    let _look = LookGuard::set(with_symbols(&[("ellipsis", "~")]));
    let row = cut(&mut app);
    assert!(row.contains('~') && !row.contains('…'), "{row}");
}

/// Cells on the accent background: the cursor row and the active tab.
fn accent_cells(buf: &ratatui::buffer::Buffer) -> Vec<(u16, ratatui::style::Style)> {
    let accent = theme::accent_color();
    let mut out = Vec::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let c = buf.cell((x, y)).unwrap();
            if c.bg == accent && !c.symbol().trim().is_empty() {
                out.push((y, c.style()));
            }
        }
    }
    out
}

/// A style on `current` reaches the cursor row only, not the tab; the default is unchanged.
#[tokio::test]
async fn styles_reach_drawn_cells() {
    use ratatui::style::Modifier;
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    let _look = LookGuard::set(theme::Look::DEFAULT);
    let plain = accent_cells(&buffer_of(&mut app, 80, 30));
    assert!(
        plain.iter().any(|(y, _)| *y == 5) && plain.iter().any(|(y, _)| *y > 7),
        "tab and cursor row"
    );
    for (_, st) in &plain {
        assert_eq!(st.fg, Some(theme::on_accent().fg.unwrap()));
        assert!(
            st.add_modifier.contains(Modifier::BOLD) && !st.add_modifier.contains(Modifier::ITALIC)
        );
    }

    let mut look = theme::Look::DEFAULT;
    look.styles.set(
        theme::Element::Current,
        theme::StylePatch::parse(&"italic = true".parse::<toml::Table>().unwrap()).0,
    );
    theme::set_look(look);
    let styled = accent_cells(&buffer_of(&mut app, 80, 30));
    for (y, st) in &styled {
        let italic = st.add_modifier.contains(Modifier::ITALIC);
        assert_eq!(italic, *y > 7, "row {y}: the cursor row is italic, the tab is not");
        assert!(st.add_modifier.contains(Modifier::BOLD), "bold stays");
    }
}

#[tokio::test]
async fn progress_bar_takes_fill_and_track_styles() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.stopped = false;
    app.paused = false;
    app.position = 100.0;
    app.duration = 200.0;
    let (fill, track) = (Color::Rgb(0xff, 0xff, 0x55), Color::Rgb(0x55, 0x55, 0x55));
    let styled = |mut look: theme::Look| {
        for (element, color) in
            [(theme::Element::ProgressFill, "#ffff55"), (theme::Element::ProgressTrack, "#555555")]
        {
            let patch = format!("fg = \"{color}\"").parse::<toml::Table>().unwrap();
            look.styles.set(element, theme::StylePatch::parse(&patch).0);
        }
        look
    };

    let _look = LookGuard::set(styled(theme::Look::DEFAULT));
    let buf = buffer_of(&mut app, 80, 30);
    let bgs: Vec<Color> = (4..60).map(|x| buf.cell((x, 28)).unwrap().bg).collect();
    assert!(bgs.contains(&fill) && bgs.contains(&track), "solid bar: {bgs:?}");

    theme::set_look(styled(with_symbols(&[("progress_filled", "#"), ("progress_empty", "-")])));
    let buf = buffer_of(&mut app, 80, 30);
    let fg_of = |glyph: &str| {
        (4..80).map(|x| buf.cell((x, 28)).unwrap()).find(|c| c.symbol() == glyph).map(|c| c.fg)
    };
    assert_eq!(fg_of("#"), Some(fill));
    assert_eq!(fg_of("-"), Some(track));
}

/// A header at `w` columns for the two header options. The app is dropped on return (it holds the lock).
async fn header_with(volume_display: &str, mode_flags: &str, w: u16) -> Terminal<TestBackend> {
    let ui = UiConfig {
        volume_display: volume_display.into(),
        mode_flags: mode_flags.into(),
        ..ascii_ui()
    };
    let (mut app, _plex) = new_app_with_ui(ui).await;
    app.volume = 40.0;
    app.loop_playlist = true;
    let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    terminal
}

/// Columns of the header's two zone dividers.
fn dividers(terminal: &Terminal<TestBackend>) -> Vec<u16> {
    let buf = terminal.backend().buffer();
    (1..buf.area.width - 1)
        .filter(|x| buf.cell((*x, 1)).is_some_and(|c| c.symbol() == "│"))
        .collect()
}

/// Mode words shorten with the width, from full words to R Z S C, never icons,
/// and the title stays centred.
#[tokio::test]
async fn mode_words_step_down_by_width() {
    let s = shmc::ui::icons::status(true);
    // (header width, an on label, an off label, the spelling, text that must not show)
    let cases: [(u16, &str, &str, &str, &str); 4] = [
        (110, "Repeat", "Shuffle", "Repeat Shuffle Single Consume", ""),
        (140, "Repeat", "Shuffle", "Repeat Shuffle Single Consume", ""),
        (100, "Rep", "Shu", "Rep Shu Sin Con", "Repeat"),
        (80, "Rep", "Shu", "Rep Shu Sin Con", "Repeat"),
    ];
    for (w, on, off, spelled, absent) in cases {
        let term = header_with("both", "words", w).await;
        let text = flatten(&term);
        let row2 = text.lines().nth(2).unwrap();
        assert!(row2.contains(spelled), "{w}: {row2}");
        assert!(absent.is_empty() || !text.contains(absent), "{w}: {row2}");
        assert_eq!(fg_at(&term, on), Some(theme::accent().fg.unwrap()), "{w}: on");
        assert_eq!(fg_at(&term, off), Some(theme::dim().fg.unwrap()), "{w}: off");
        assert!(!row2.contains(s.repeat) && !row2.contains(s.consume), "{w}: no icons: {row2}");
        let divs = dividers(&term);
        assert_eq!(divs.len(), 2, "{w}: {divs:?}");
        assert_eq!(divs[0] + divs[1], w - 1, "{w}: the title column is centred: {divs:?}");
        let title = char_x(text.lines().nth(1).unwrap(), "No Song").unwrap() as u16 + 3;
        assert!(title.abs_diff(w / 2) <= 1, "{w}: the title is at {title}");
    }
    for w in [44u16, 48] {
        let term = header_with("both", "words", w).await;
        let text = flatten(&term);
        let row2 = text.lines().nth(2).unwrap();
        assert!(row2.contains("R Z S C") && !text.contains("Rep"), "{w}: {row2}");
        assert_eq!(fg_at(&term, "R Z S C"), Some(theme::accent().fg.unwrap()), "{w}: on");
        assert_eq!(fg_at(&term, "Z S C"), Some(theme::dim().fg.unwrap()), "{w}: off");
        assert!(!row2.contains(s.repeat), "{w}: no icons: {row2}");
        let divs = dividers(&term);
        assert_eq!(divs[0] + divs[1], w - 1, "{w}: {divs:?}");
    }
}

/// With the sleep label sharing the row the words step down, not turn to icons.
#[tokio::test]
async fn sleep_label_steps_words_not_icons() {
    let ui = UiConfig { mode_flags: "words".into(), ..ascii_ui() };
    let (mut app, _plex) = new_app_with_ui(ui).await;
    run_cmd(&mut app, "sleep 15");
    let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
    term.draw(|f| views::draw(f, &mut app)).unwrap();
    let text = flatten(&term);
    let row2 = text.lines().nth(2).unwrap();
    assert!(row2.contains("sleep 15m Rep Shu Sin Con"), "{row2}");
    assert!(
        !row2.contains("Repeat") && !row2.contains(shmc::ui::icons::status(true).repeat),
        "{row2}"
    );
}

/// Each `ui.volume_display` value draws what it names (unknown is both) without moving the columns.
#[tokio::test]
async fn volume_display_meter_number_or_both() {
    for w in [80u16, 120] {
        let both = header_with("both", "icons", w).await;
        for (value, meter, number) in [
            ("meter", true, false),
            ("percent", false, true),
            ("both", true, true),
            ("bogus", true, true),
        ] {
            let term = header_with(value, "icons", w).await;
            let text = flatten(&term);
            let row = text.lines().nth(1).unwrap();
            assert_eq!(row.contains('▁'), meter, "{value} {w}: {row}");
            assert_eq!(row.contains("40%"), number, "{value} {w}: {row}");
            assert_eq!(dividers(&term), dividers(&both), "{value} {w}: the columns do not move");
        }
    }
}

/// The `:` line, the `/` filter and the search input keep the caret and the
/// tail in view when the text is wider than the progress row.
#[tokio::test]
async fn long_prompt_keeps_caret_and_tail_visible() {
    for (open, tab) in [(':', Tab::Queue), ('/', Tab::Queue), ('/', Tab::Search)] {
        let mut app = new_app().await;
        app.tab = tab;
        app.handle_msg(key(open));
        for c in "abcdefghijklmnopqrstuvwxyz0123456789TAIL".chars() {
            app.handle_msg(key(c));
        }
        let text = render(&mut app, 40, 20);
        let bottom = text.lines().nth(18).unwrap();
        assert!(bottom.contains('█') && bottom.contains("TAIL"), "{open} {tab:?}: {bottom}");
    }
}

/// `:stop` drops a replace still in flight, so it neither replaces the queue
/// nor starts playing when it lands.
#[tokio::test]
async fn stop_cancels_a_replace_still_in_flight() {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.queue = (0..3).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    on_playlist(&mut app, 0);
    app.handle_msg(enter());
    let seqs = resolve_seqs(&mut plex);
    assert_eq!(seqs.len(), 1);

    app.handle_msg(key(':'));
    for c in "stop".chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
    assert!(app.status.contains("cancelled"), "{}", app.status);

    app.handle_msg(tracks_reply(seqs[0], 10..12, QueueAction::Replace));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2"]);
    assert!(app.stopped, "a cancelled replace must not start playback");
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_msg(key(c));
    }
}

#[tokio::test]
async fn visual_range_above_playing_removes_and_undoes() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..7).map(track).collect();
    app.current = Some(5);
    app.queue_state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(2));
    assert!(app.visual.is_none());

    // mpv walks the playing entry down through the slots in between
    play_pos(&mut app, 3);
    assert_eq!(app.current, Some(2), "a stale position moved current");

    app.handle_msg(key('u'));
    assert_eq!(queue_keys(&app), ["t0", "t1", "t2", "t3", "t4", "t5", "t6"]);
    assert_eq!(app.current, Some(5));
    assert_eq!(app.queue_state.selected(), Some(1));
}

#[tokio::test]
async fn visual_range_with_playing_hands_to_next() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..6).map(track).collect();
    app.current = Some(2);
    app.queue_state.select(Some(1));
    app.stopped = false;

    keys(&mut app, "vjjd");
    assert_eq!(queue_keys(&app), ["t0", "t4", "t5"]);
    assert_eq!(app.current, Some(1));
    assert_eq!(app.queue[1].rating_key, "t4");
    assert!(!app.stopped);
}

#[tokio::test]
async fn visual_range_under_filter_removes_visible_rows() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue = (0..30).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(0));
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
    app.queue = (0..6).map(track).collect();
    app.current = Some(0);
    app.queue_state.select(Some(3));
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
    app.handle_msg(Msg::Mpv(MpvEvent::EndFile { reason: "eof".into() }));
    assert_eq!(app.visual.map(|v| v.anchor), Some(2));
    assert_eq!(app.queue[2].rating_key, "t3");

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
    app.queue = (0..8).map(track).collect();
    app.queue_state.select(Some(1));
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
    app.browser.tracks = (0..n).map(track).collect();
    app.browser.tracks_state.select(Some(0));
}

fn filter_browser_tracks(app: &mut App, needle: &str) {
    let titles: Vec<String> = app.browser.tracks.iter().map(|t| t.title.clone()).collect();
    app.browser_filters[2].text = needle.into();
    app.browser_filters[2].apply(titles.len(), |i| titles[i].contains(needle));
}

#[tokio::test]
async fn browser_visual_range_adds_in_order() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 30);
    app.queue = vec![track(100)];
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
    app.queue = (10..13).map(track).collect();
    app.current = Some(0);
    app.stopped = false;
    app.browser.tracks_state.select(Some(1));

    keys(&mut app, "vjji");
    assert_eq!(queue_keys(&app), ["t10", "t1", "t2", "t3", "t11", "t12"]);
    assert!(app.visual.is_none());
}

#[tokio::test]
async fn visual_range_enter_replaces_queue_with_range() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    app.queue = (10..13).map(track).collect();
    app.browser.tracks_state.select(Some(5));

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
    app.playlists.items_for = Some("pl0".into());
    app.playlists.items = (0..6).map(track).collect();
    app.playlists.items_state.select(Some(1));
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
    app.browser.tracks_state.select(Some(1));
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

#[tokio::test]
async fn g_letter_in_visual_jumps_not_adds() {
    let mut app = new_app().await;
    on_browser_tracks(&mut app, 8);
    keys(&mut app, "vjga");
    assert!(app.queue.is_empty());
    assert!(app.visual.is_some());
}

#[tokio::test]
async fn save_sends_whole_queue_even_when_filtered() {
    let (mut app, mut plex_rx) = new_app_with_plex().await;
    let saves = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
        let mut out = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            if let PlexCmd::SavePlaylist { name, keys } = cmd {
                out.push((name, keys));
            }
        }
        out
    };

    run_cmd(&mut app, "save Road trip");
    assert!(saves(&mut plex_rx).is_empty(), "an empty queue sends nothing");

    app.queue = (0..4).map(track).collect();
    app.queue_filter.text = "Starfall! 2".into();
    app.queue_filter.apply(4, |i| i == 2);
    run_cmd(&mut app, "save Road trip");
    assert_eq!(
        saves(&mut plex_rx),
        [("Road trip".to_string(), vec!["t0".into(), "t1".into(), "t2".into(), "t3".into()])]
    );

    run_cmd(&mut app, "save Again");
    assert!(saves(&mut plex_rx).is_empty(), "one save at a time");
}

fn with_borders(style: theme::BorderStyle) -> theme::Look {
    theme::Look {
        spacing: theme::Spacing { borders: style, ..theme::Spacing::DEFAULT },
        ..theme::Look::DEFAULT
    }
}

const STYLES: [theme::BorderStyle; 5] = [
    theme::BorderStyle::Box,
    theme::BorderStyle::Rounded,
    theme::BorderStyle::Double,
    theme::BorderStyle::Lines,
    theme::BorderStyle::None,
];

/// Rows of the tab row, the first content row and the progress row.
fn chrome_rows(compact: bool, style: theme::BorderStyle, h: u16) -> (u16, u16, u16) {
    use theme::BorderStyle::*;
    match (compact, style) {
        (false, Box | Rounded | Double) => (5, 7, h - 2),
        (true, Box | Rounded | Double) => (1, 3, h - 2),
        (false, Lines) => (3, 5, h - 1),
        (true, Lines) => (0, 2, h - 1),
        (false, None) => (2, 3, h - 1),
        (true, None) => (0, 1, h - 1),
    }
}

fn row_text(g: &[Vec<String>], y: u16) -> String {
    g[y as usize].concat()
}

/// Every header mode and border style keeps the tabs whole, the progress row
/// and what is playing on every tab, and the content between them.
#[tokio::test]
async fn every_header_border_combo_fits() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.queue[2].title = "Unique Song".into();
    app.position = 42.0;
    app.duration = 257.0;
    for compact in [false, true] {
        app.header_compact = compact;
        for style in STYLES {
            let _look = LookGuard::set(with_borders(style));
            for (w, h) in [(120u16, 40u16), (80, 24), (50, 20), (80, 60)] {
                let (tab_y, content_y, progress_y) = chrome_rows(compact, style, h);
                let at = format!("{style:?} compact={compact} {w}x{h}");
                for tab in TABS {
                    app.tab = tab;
                    let g = grid(&mut app, w, h);
                    let tabs = row_text(&g, tab_y);
                    for name in ["Queue", "Browser", "Playlists", "Search", "Help"] {
                        assert!(
                            tabs.contains(&format!(" {name} ")),
                            "{name} on {tab:?}, {at}: {tabs:?}"
                        );
                    }
                    assert!(
                        row_text(&g, progress_y).contains("0:42 / 4:17"),
                        "progress on {tab:?}, {at}"
                    );
                    let all: String = g.iter().map(|r| r.concat()).collect();
                    assert!(all.contains("Unique Song"), "title on {tab:?}, {at}");
                }
                // The queue's own header starts the content, which ends at the progress row.
                app.tab = Tab::Queue;
                app.art_enabled = false;
                let g = grid(&mut app, w, h);
                app.art_enabled = true;
                let header_y = content_y + u16::from(style.boxed());
                assert!(
                    row_text(&g, header_y).contains("Title"),
                    "content starts at {content_y}, {at}"
                );
                if style == theme::BorderStyle::Lines {
                    let rules: Vec<u16> =
                        (0..h).filter(|&y| g[y as usize].iter().all(|c| c == "─")).collect();
                    // Compact puts the title on the last rule.
                    let want: Vec<u16> = if compact { vec![1] } else { vec![2, 4, h - 2] };
                    assert_eq!(rules, want, "full-width rules, {at}");
                    if compact {
                        let last = row_text(&g, h - 2);
                        assert!(
                            last.starts_with('─') && last.contains("Unique Song"),
                            "{at}: {last:?}"
                        );
                    }
                }
            }
        }
    }
}

/// `none` draws no frame glyph anywhere, `rounded` rounds the corners, `double`
/// draws double lines and `lines` has no boxes.
#[tokio::test]
async fn border_styles_draw_their_glyphs() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    let frame_glyphs = ['─', '│', '┌', '┐', '└', '┘', '╭', '╮', '╰', '╯', '╔', '╗', '╚', '╝'];
    for style in STYLES {
        let _look = LookGuard::set(with_borders(style));
        for tab in TABS {
            app.tab = tab;
            let text = render(&mut app, 80, 30);
            let found: Vec<char> =
                frame_glyphs.iter().copied().filter(|c| text.contains(*c)).collect();
            match style {
                theme::BorderStyle::None => assert!(found.is_empty(), "none on {tab:?}: {found:?}"),
                theme::BorderStyle::Rounded => {
                    assert!(text.contains('╭') && !text.contains('┌'), "{tab:?}")
                }
                theme::BorderStyle::Double => {
                    assert!(
                        text.contains('╔') && text.contains('╝') && !text.contains('┌'),
                        "{tab:?}"
                    )
                }
                theme::BorderStyle::Lines => {
                    assert!(!text.contains('┌') && text.contains('─'), "{tab:?}")
                }
                theme::BorderStyle::Box => {
                    assert!(text.contains('┌') && !text.contains('╭'), "{tab:?}")
                }
            }
        }
    }
}

/// Narrow compact rows give up the right side, then the sleep timer, before a
/// tab is cut. The playback state is not repeated there.
#[tokio::test]
async fn compact_row_steps_down_before_tab_clips() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.header_compact = true;
    run_cmd(&mut app, "sleep 15");
    let row = |app: &mut App, w: u16| row_text(&grid(app, w, 24), 1);
    for w in [120u16, 80, 60, 50, 46] {
        let text = row(&mut app, w);
        for name in ["Queue", "Browser", "Playlists", "Search", "Help"] {
            assert!(text.contains(&format!(" {name} ")), "{name} at {w}: {text:?}");
        }
        assert!(!text.contains('>'), "no state glyph at {w}: {text:?}");
    }
    let wide = row(&mut app, 120);
    let left_of_tabs = wide.find("sleep 15m").zip(wide.find("Queue")).is_some_and(|(s, q)| s < q);
    assert!(left_of_tabs, "the timer on the left: {wide:?}");
    assert!(wide.contains("Volume: ") && wide.contains('▁'), "the meter at 120: {wide:?}");
    let mid = row(&mut app, 80);
    assert!(
        mid.contains("sleep 15m") && mid.contains('%') && !mid.contains('▁'),
        "a shorter volume at 80: {mid:?}"
    );
    let narrow = row(&mut app, 60);
    assert!(
        narrow.contains("sleep 15m") && !narrow.contains('%'),
        "only the timer at 60: {narrow:?}"
    );
    let tight = row(&mut app, 46);
    assert!(!tight.contains("sleep"), "{tight:?}");
}

/// In compact `none` the row above the progress row is kept whatever the
/// cover metadata does, so the content never changes height between tabs.
#[tokio::test]
async fn compact_without_borders_keeps_now_playing_row() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.art_metadata = true;
    app.queue[2].title = "Unique Song".into();
    app.header_compact = true;
    let _look = LookGuard::set(with_borders(theme::BorderStyle::None));
    let h = 30;
    app.tab = Tab::Queue;
    let queue = grid(&mut app, 100, h);
    assert!(row_text(&queue, h - 2).trim().is_empty(), "blank while the cover metadata shows it");
    assert!(row_text(&queue, h - 1).contains(" / "), "the progress row");
    app.tab = Tab::Browser;
    let browser = grid(&mut app, 100, h);
    assert!(row_text(&browser, h - 2).contains("Unique Song"), "the title above the progress row");
    assert!(row_text(&browser, h - 1).contains(" / "));
    assert!(row_text(&queue, 1).contains("Title"), "content starts under the tab row");
}

fn queue_with_columns(cols: &[&str]) -> UiConfig {
    UiConfig { queue_columns: cols.iter().map(|c| c.to_string()).collect(), ..ascii_ui() }
}

fn tagged_queue(app: &mut App) {
    app.queue = (0..50)
        .map(|i| Track {
            year: Some(2008),
            audio: AudioInfo { codec: Some("flac".into()), ..AudioInfo::default() },
            ..track(i)
        })
        .collect();
    app.queue_state.select(Some(7));
    app.tab = Tab::Queue;
}

#[tokio::test]
async fn queue_columns_follow_listed_order() {
    let (mut app, _plex) =
        new_app_with_ui(queue_with_columns(&["track", "title", "year", "codec", "len"])).await;
    tagged_queue(&mut app);
    for (w, h) in [(70, 24), (100, 40), (160, 70)] {
        let frame = render(&mut app, w, h);
        let hd = header_cols(&frame, &["#", "Title", "Year", "Codec", "Len"]);
        assert!(hd.windows(2).all(|p| p[0] < p[1]), "{w}x{h}: order {hd:?}\n{frame}");
        // right-aligned columns line up by their last cell
        let end = |text: &str| cell_col(&frame, "Codec", text).map(|c| c + text.len());
        assert_eq!(cell_col(&frame, "Codec", "Starfall"), Some(hd[1]), "{w}x{h}\n{frame}");
        assert_eq!(cell_col(&frame, "Codec", "FLAC"), Some(hd[3]), "{w}x{h}\n{frame}");
        assert_eq!(end("2008"), Some(hd[2] + 4), "{w}x{h}\n{frame}");
        assert_eq!(end("4:17"), Some(hd[4] + 3), "{w}x{h}\n{frame}");
    }
}

#[tokio::test]
async fn narrow_queue_drops_codec_year_track_first() {
    let (mut app, _plex) =
        new_app_with_ui(queue_with_columns(&["track", "title", "year", "codec", "len"])).await;
    tagged_queue(&mut app);
    let shown = |app: &mut App, w: u16| {
        let frame = render(app, w, 24);
        assert!(
            frame.contains("Title") && frame.contains("Len"),
            "{w}: title and len stay\n{frame}"
        );
        ["#", "Year", "Codec"].map(|h| frame.contains(h))
    };
    assert_eq!(shown(&mut app, 40), [true, true, true]);
    assert_eq!(shown(&mut app, 30), [true, true, false]);
    assert_eq!(shown(&mut app, 24), [true, false, false]);
    assert_eq!(shown(&mut app, 20), [false, false, false]);
}
