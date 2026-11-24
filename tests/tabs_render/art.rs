use super::*;

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
    let (mut app, mut terminal) = app_with_cover().await;
    let rect = app.art.shown_rect.expect("cover was not painted");
    assert!(
        cells_skipped(&mut terminal, &mut app, rect),
        "cover cells were not skipped on the second frame"
    );
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
    app.queue.items = (0..3).map(track).collect();
    app.current = Some(0);
    app.handle_msg(Msg::Resize);
    assert!(app.art.hold_until.is_some(), "a resize did not start a dwell");
    // the real dwell is 120 ms; a loaded machine could outlast it before the draw
    app.art.hold_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    assert!(!cover_drawn(&mut terminal, &mut app));

    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    app.handle_msg(Msg::ArtReady {
        thumb_path: "/library/metadata/al1/thumb".into(),
        img,
        file: None,
    });
    assert!(cover_drawn(&mut terminal, &mut app), "a new cover waited for the dwell");
}

const AL1_THUMB: &str = "/library/metadata/al1/thumb";

/// Tracks 0-1 share an album, 2 has another, 3 has no cover path; 0 plays with its cover.
async fn app_showing_album_cover(
) -> (TestApp, Terminal<TestBackend>, tokio::sync::mpsc::UnboundedReceiver<PlexCmd>) {
    let (mut app, mut plex) = new_app_with_plex().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::halfblocks());
    app.queue.items = (0..4).map(track).collect();
    app.queue.items[2].parent_rating_key = Some("al2".into());
    app.queue.items[3].parent_rating_key = None;
    app.handle_msg(playlist_pos(0));
    assert_eq!(thumb_requests(&mut plex), [AL1_THUMB]);
    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    app.handle_msg(Msg::ArtReady { thumb_path: AL1_THUMB.into(), img, file: None });
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|f| views::draw(f, &mut app)).unwrap();
    assert!(app.art.shown_rect.is_some(), "cover was not painted");
    (app, terminal, plex)
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
    app.handle_msg(playlist_pos(1));
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
    app.handle_msg(Msg::ArtReady { thumb_path: AL1_THUMB.into(), img, file: None });
    assert!(app.art.protocol.is_some());
    assert_eq!((app.art.shown_rect, app.art.prepared_rect), (shown, prepared));
    assert_eq!(app.art.hold_until, hold);
}

/// Another album's cover is requested once and can retry; a track without one drops it.
#[tokio::test]
async fn new_cover_loads_once() {
    let (mut app, _terminal, mut plex) = app_showing_album_cover().await;
    app.handle_msg(playlist_pos(2));
    app.handle_msg(playlist_pos(2));
    assert_eq!(thumb_requests(&mut plex), ["/library/metadata/al2/thumb"]);

    app.handle_msg(Msg::ArtFailed { thumb_path: "/library/metadata/al2/thumb".into() });
    app.handle_msg(playlist_pos(2));
    assert_eq!(
        thumb_requests(&mut plex),
        ["/library/metadata/al2/thumb"],
        "no retry after a failure"
    );

    app.handle_msg(playlist_pos(3));
    assert!(app.art.img.is_none() && app.art.protocol.is_none(), "the old cover stayed");
    assert!(app.art.shown_rect.is_none() && app.art.thumb.is_none());
}

/// Square art caps the art width at `art_pct`, so the queue keeps its Artist/Album columns.
#[tokio::test]
async fn square_art_pane_respects_the_percentage_cap() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.picker = Some(ratatui_image::picker::Picker::halfblocks());
    app.ui.art_square = true;
    app.ui.art_metadata = true;
    let track0 = track(0);
    let artist = track0.artist.clone();
    let album = track0.album.clone();
    app.queue.items = vec![track0];
    app.queue.state.select(Some(0));
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

    let divider = bars[3];
    let cover_y = frame.lines().position(|l| l.contains("No cover")).expect("cover line");
    let left_text: String = frame
        .lines()
        .skip(cover_y + 1)
        .map(|l| l.chars().take(divider).collect::<String>())
        .collect();
    assert!(left_text.contains(&artist), "metadata artist missing\n{left_text}");
    assert!(left_text.contains(&album), "metadata album missing\n{left_text}");

    app.ui.art_metadata = false;
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
async fn disabling_art_gives_the_queue_full_width() {
    let mut app = new_app().await;
    app.tab = Tab::Queue;
    app.queue.items = (0..10).map(track).collect();
    app.current = Some(0);
    app.queue.state.select(Some(0));
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

/// In a height-limited pane the empty-cover box fills the rows above the metadata.
#[tokio::test]
async fn cover_box_fills_height_limited_pane() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.ui.art_square = true;
    let mut checked = 0;
    for metadata in [true, false] {
        app.ui.art_metadata = metadata;
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

/// Track 0 playing with a thumb, art not loaded; mpv commands go to the receiver.
async fn app_awaiting_cover() -> (TestApp, tokio::sync::mpsc::UnboundedReceiver<mpv::MpvCmd>) {
    let (mut app, _plex) = new_app_with_plex().await;
    app.tab = Tab::Queue;
    app.art_enabled = true;
    app.now_playing_cover = true;
    app.queue.items = (0..4).map(track).collect();
    app.queue.items[2].parent_rating_key = Some("al2".into());
    app.handle_msg(playlist_pos(0));
    let (tx, rx) = unbounded_channel();
    app.mpv.tx = tx;
    (app, rx)
}

fn file_loaded() -> Msg {
    Msg::Mpv(mpv::MpvEvent::FileLoaded)
}

fn art_ready(thumb: &str, file: Option<&str>) -> Msg {
    let img = Arc::new(DynamicImage::ImageRgba8(RgbaImage::new(8, 8)));
    Msg::ArtReady { thumb_path: thumb.into(), img, file: file.map(Into::into) }
}

/// The paths of the covers sent to mpv so far.
fn covers_sent(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<mpv::MpvCmd>,
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let mpv::MpvCmd::AddCover(p) = cmd {
            paths.push(p);
        }
    }
    paths
}

/// Each loaded file gets the cover once, and only when its own art is ready.
#[tokio::test]
async fn cover_goes_to_mpv_once_per_file() {
    let (mut app, mut rx) = app_awaiting_cover().await;
    let p = std::path::PathBuf::from("/cache/al1.img");

    app.handle_msg(file_loaded());
    assert!(covers_sent(&mut rx).is_empty(), "sent before the art was ready");

    app.handle_msg(art_ready(AL1_THUMB, Some("/cache/al1.img")));
    assert_eq!(covers_sent(&mut rx), std::slice::from_ref(&p));

    app.handle_msg(file_loaded());
    assert_eq!(covers_sent(&mut rx), [p]);

    app.handle_msg(playlist_pos(2));
    app.handle_msg(file_loaded());
    assert!(covers_sent(&mut rx).is_empty(), "the old album's cover went to the new file");
}

/// A cover that is not cached in a file cannot be handed to mpv.
#[tokio::test]
async fn cover_needs_a_cached_file() {
    let (mut app, mut rx) = app_awaiting_cover().await;
    app.handle_msg(file_loaded());
    app.handle_msg(art_ready(AL1_THUMB, None));
    assert!(covers_sent(&mut rx).is_empty(), "no file, yet a cover was sent");
}

/// With the cover off, nothing reaches mpv.
#[tokio::test]
async fn cover_not_sent_when_off() {
    let (mut app, mut rx) = app_awaiting_cover().await;
    app.now_playing_cover = false;
    app.handle_msg(file_loaded());
    app.handle_msg(art_ready(AL1_THUMB, Some("/cache/al1.img")));
    assert!(covers_sent(&mut rx).is_empty(), "sent with the cover off");
}
