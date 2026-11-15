use super::*;
use ratatui::style::Color;
use ratatui::widgets::TableState;
use shmc::plex::models::{Album, Artist};
use shmc::ui::app::{BrowserCol, SearchDetail};
use shmc::ui::theme;

fn lines_with(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
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
        quick_xml::de::from_str(include_str!("../fixtures/hubs_search_starfall.xml")).unwrap();
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
        quick_xml::de::from_str(include_str!("../fixtures/hubs_search_starfall.xml")).unwrap();
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
