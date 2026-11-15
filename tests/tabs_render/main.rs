//! Render and app tests: every tab draws at several sizes, and the key, queue
//! and layout rules hold. mpv runs for real.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use image::{DynamicImage, RgbaImage};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use shmc::config::{KeyTable, MpvConfig, UiConfig};
use shmc::playback::{mpv, MpvEvent};
use shmc::plex::models::{AudioInfo, Playlist, Track};
use shmc::ui::app::{App, Confirm, Msg, PlaylistsCol, PlexCmd, PlexHandle, QueueAction, Tab};
use shmc::ui::queue_seq::QueueOutcome;
use shmc::ui::views;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

mod art;
mod browse;
mod keys;
mod layout;
mod queue;
mod queue_requests;
mod theme;

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
    _mpv_events: tokio::sync::mpsc::UnboundedReceiver<MpvEvent>,
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

async fn new_app_with_ui_and_keys(
    ui: UiConfig,
    keys: &KeyTable,
) -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    let guard = TAB_TEST_LOCK.lock().await;
    let cfg = MpvConfig { path: "mpv".into(), extra_args: vec!["--ao=null".into()], volume: 50.0 };
    let (mpv_handle, mpv_events) = mpv::spawn(&cfg).await.expect("spawn mpv");
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
    (TestApp { app, _mpv_events: mpv_events, _guard: guard }, plex_rx)
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

/// Columns of every `│` in a line.
fn bar_columns(line: &str) -> Vec<usize> {
    line.chars().enumerate().filter(|(_, c)| *c == '│').map(|(i, _)| i).collect()
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

fn playlist_pos(i: usize) -> Msg {
    Msg::Mpv(MpvEvent::Property { name: "playlist-pos".into(), value: Some(serde_json::json!(i)) })
}

fn run_cmd(app: &mut App, cmd: &str) {
    app.handle_msg(key(':'));
    for c in cmd.chars() {
        app.handle_msg(key(c));
    }
    app.handle_msg(enter());
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

/// Drains the Plex requests the UI has sent, oldest first.
fn sent(plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) -> Vec<PlexCmd> {
    let mut out = Vec::new();
    while let Ok(cmd) = plex.try_recv() {
        out.push(cmd);
    }
    out
}

/// The last request `pick` matches among those the UI has sent.
fn last_sent<T>(
    plex: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>,
    pick: impl FnMut(PlexCmd) -> Option<T>,
) -> Option<T> {
    sent(plex).into_iter().rev().find_map(pick)
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

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_msg(key(c));
    }
}

/// Draws one frame; whether every cell of `rect` is marked skip in it.
fn cells_skipped(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    rect: ratatui::layout::Rect,
) -> bool {
    let mut all_skipped = true;
    // check the live frame buffer: the backend only receives the diff
    terminal
        .draw(|f| {
            views::draw(f, app);
            let buf = f.buffer_mut();
            for y in rect.top()..rect.bottom() {
                for x in rect.left()..rect.right() {
                    all_skipped &= buf.cell((x, y)).is_some_and(|c| c.skip);
                }
            }
        })
        .unwrap();
    all_skipped
}

fn row_text(g: &[Vec<String>], y: u16) -> String {
    g[y as usize].concat()
}
