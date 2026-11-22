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

    app.queue.items = (0..50).map(track).collect();
    app.queue.state.select(Some(7));
    app.current = Some(7);
    app.stopped = false;
    app.duration = 257.0;
    app.position = 42.0;
    app.volume = 63.0;
    app.loop_playlist = true;
    app.random = true;
    app.bitrate_bps = 320_000.0;
    app.cava_alive = true;
    app.cava_frame = vec![255u8; 20];

    app.browser.artists.items = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Northwind Sounds {i:02}"),
            thumb: None,
            plays: None,
        })
        .collect();
    app.browser.artists.state.select(Some(3));
    app.browser.albums.items = (0..25)
        .map(|i| Album {
            rating_key: format!("al{i}"),
            title: format!("Evening Satellite {i:02}"),
            artist: "Northwind Sounds".into(),
            year: Some(2019),
            thumb: None,
            plays: None,
        })
        .collect();
    app.browser.albums.state.select(Some(2));
    app.browser.tracks.items = (0..60).map(track).collect();
    app.browser.tracks.state.select(Some(11));
    app.browser.focus = BrowserCol::Artists;
    app.browser.albums_for = Some("al2".into());
    app.browser.tracks_for = Some("al2".into());

    app.playlists.list.items = (0..25)
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
    app.playlists.list.state.select(Some(3));
    app.playlists.tracks.items = (0..12).map(track).collect();
    app.playlists.tracks.state.select(Some(5));
    app.playlists.focus = PlaylistsCol::List;
    app.playlists.tracks_for = Some("p3".into());

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
        "[Playing]",
        "Volume: ",
        " 63%",
        "0:42 / 4:17",
        "(320 kbps)",
        "rep",
        "shf",
        "one",
        "con",
        "│",
        "Queue",
        "Browser",
        "Playlists",
        "Len",
        "Title",
        "Artist",
        "Starfall! 7",
        "Evening Satellite",
        "4:17",
        "No cover", // no picker in tests
        "▼",
    ] {
        assert!(q.contains(want), "Queue frame must contain {want:?}\nframe:\n{q}");
    }

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
        "Featured Acts",    // track credit from originalTitle
        "Northwind Sounds", // album artist on the album row
        "Evening Satellite",
        "4:17",
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
        matches: None,
        radio: None,
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
}

/// The bottom row shows command input over filter typing over an applied filter over the status.
#[tokio::test]
async fn bottom_row_prefers_command_then_filter_then_status() {
    let mut app = new_app().await;
    app.queue.items = (0..50).map(track).collect();
    app.queue.state.select(Some(7));
    app.current = Some(7);
    app.stopped = false;
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

    app.queue.filter.typing = true;
    app.queue.filter.text = "sky".into();
    let titles: Vec<String> = app.queue.items.iter().map(|t| t.title.clone()).collect();
    app.queue.filter.apply(titles.len(), |i| titles[i].contains("Starfall! 1"));
    let c2 = render(&mut app, 120, 40);
    assert!(c2.contains("/ sky█"), "filter input must render\nframe:\n{c2}");
    assert!(c2.contains("Queue 11/50"), "filter counter must render\nframe:\n{c2}");

    app.queue.filter.typing = false;
    let c3 = render(&mut app, 120, 40);
    assert!(c3.contains("filter: sky (11/50)"), "filter indicator must show counts\nframe:\n{c3}");
    app.queue.filter.clear();

    app.status = "Added album: Evening Satellite (12 tracks)".into();
    let c4 = render(&mut app, 120, 40);
    assert!(c4.contains("Added album: Evening Satellite (12 tracks)"), "frame:\n{c4}");
}

#[tokio::test]
async fn transport_state_shows_badge_and_glyph() {
    let mut app = new_app().await;
    app.queue.items = (0..5).map(track).collect();
    app.queue.state.select(Some(1));
    app.current = Some(1);
    app.stopped = false;
    app.duration = 257.0;
    app.position = 42.0;
    app.tab = Tab::Queue;

    let play = render(&mut app, 120, 40);
    assert!(play.contains('>'), "progress bar must show the play glyph\nframe:\n{play}");
    app.paused = true;
    let pause = render(&mut app, 120, 40);
    assert!(pause.contains("[Paused]"), "frame:\n{pause}");
    assert!(pause.contains("||"), "pause glyph\n{pause}");
    app.paused = false;
    app.stopped = true;
    let stop = render(&mut app, 120, 40);
    assert!(stop.contains("[Stopped]"), "frame:\n{stop}");
    assert!(stop.contains('■'), "stop glyph\n{stop}");
    for frame in [&play, &pause, &stop] {
        assert!(!frame.contains('\u{23f8}'), "no U+23F8 pause glyph");
        assert!(!frame.contains('\u{fe0f}'), "no variation selector");
    }
}

#[tokio::test]
async fn header_renders_a_cjk_title() {
    let mut app = new_app().await;
    app.queue.items = (0..30).map(track).collect();
    app.queue.state.select(Some(3));
    app.current = Some(3);
    app.stopped = false;
    app.tab = Tab::Queue;
    app.queue.items[3].title = "日本語の曲".into();
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
}

#[tokio::test]
async fn browser_cuts_long_artist_name() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.tracks_for = Some("al0".into());
    app.browser.albums_for = Some("al0".into());
    app.browser.albums.items = vec![Album {
        rating_key: "al0".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        year: Some(2019),
        thumb: None,
        plays: None,
    }];
    app.browser.artists.items = vec![Artist {
        rating_key: "a0".into(),
        title: "A Very Long Artist Name That Will Not Fit In One Column At All".into(),
        thumb: None,
        plays: None,
    }];
    app.browser.artists.state.select(Some(0));
    app.browser.albums.state.select(Some(0));
    app.browser.tracks.items = (0..9).map(track).collect();
    app.browser.tracks.state.select(Some(0));
    app.browser.focus = BrowserCol::Artists;
    let br = render(&mut app, 120, 40);
    assert!(br.contains('…'), "long artist name must be truncated with …\n{br}");
}

#[tokio::test]
async fn browser_track_filter_shows_counts_and_no_matches() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.tracks_for = Some("al0".into());
    app.browser.albums_for = Some("al0".into());
    app.browser.tracks.items = (0..9)
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
    app.browser.tracks.state.select(Some(0));
    app.browser.focus = BrowserCol::Tracks;
    app.browser.tracks.filter.text = "c".into();
    app.browser.tracks.filter.typing = true;
    let tracks = app.browser.tracks.items.clone();
    app.browser.tracks.filter.apply(tracks.len(), |i| tracks[i].title.to_lowercase().contains("c"));
    let f1 = render(&mut app, 120, 40);
    assert!(f1.contains("Tracks 2/9"), "live filter counter must read Tracks 2/9\n{f1}");
    assert!(f1.contains("Cyan Sea") && f1.contains("Crimson"), "matching rows shown\n{f1}");
    assert!(!f1.contains("Alpha"), "non-matching rows hidden\n{f1}");
    app.browser.tracks.filter.typing = false;
    let f2 = render(&mut app, 120, 40);
    assert!(
        f2.contains("filter: c (2/9)"),
        "after Enter the indicator moves to the column top\n{f2}"
    );
    app.browser.tracks.filter.text = "cx".into();
    app.browser
        .tracks
        .filter
        .apply(tracks.len(), |i| tracks[i].title.to_lowercase().contains("cx"));
    let f3 = render(&mut app, 120, 40);
    assert!(f3.contains("No matches"), "zero matches must show No matches\n{f3}");
}

#[tokio::test]
async fn cover_placeholder_stays_put_across_songs() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.queue.items = (0..30).map(track).collect();
    app.queue.state.select(Some(1));
    app.stopped = false;
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
}

/// The visualizer keeps drawing while paused, in about half the column.
#[tokio::test]
async fn visualizer_keeps_drawing_while_paused() {
    let mut app = new_app().await;
    app.art_enabled = true;
    app.queue.items = (0..30).map(track).collect();
    app.queue.state.select(Some(3));
    app.current = Some(3);
    app.stopped = false;
    app.duration = 3725.0;
    app.position = 42.0;
    app.paused = true;
    app.tab = Tab::Queue;
    let bar_glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let bar_rows = |frame: &str| {
        frame.lines().skip(7).filter(|l| l.chars().any(|c| bar_glyphs.contains(&c))).count()
    };

    app.cava_alive = true;
    app.cava_frame = vec![220u8; 20];
    let h = render(&mut app, 120, 40);
    assert!(
        !h.contains("visualizer off") && !h.contains("cava is not running"),
        "a live cava must not be reported as off\n{h}"
    );
    app.cava_alive = false;
    let off = render(&mut app, 120, 40);
    let viz_rows = bar_rows(&h).saturating_sub(bar_rows(&off));
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

/// With no `#` column, rows line up under the header and the art box takes the full width.
#[tokio::test]
async fn queue_drops_row_numbers_for_art() {
    let mut app = new_app().await;
    app.queue.items = (0..50).map(track).collect();
    app.queue.state.select(Some(0));
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
    app.queue.items = (0..20).map(track).collect();
    app.queue.state.select(Some(0));
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
    app.queue.items = vec![track(0)];
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
        ..AudioInfo::default()
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
    app.playlists.list.items =
        vec![listed_playlist("p0", "Small Mix", 9), listed_playlist("p1", "Big Mix", 20_000)];
    app.playlists.focus = PlaylistsCol::Items;

    app.playlists.tracks.items = (0..9).map(track).collect();
    app.playlists.tracks_for = Some("p0".into());
    app.playlists.tracks_total = Some(9);
    let small = render(&mut app, 120, 40);
    let small_x = column_of(&small, "Starfall! 0");

    app.playlists.tracks.items = (0..100).map(track).collect();
    app.playlists.tracks_for = Some("p1".into());
    app.playlists.tracks_total = Some(20_000);
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

    app.browser.tracks.items = (0..9).map(track).collect();
    app.browser.tracks_for = Some("al0".into());
    let small = render(&mut app, 120, 40);
    let small_x = column_of(&small, "Starfall! 0");

    app.browser.tracks.items = (0..100).map(track).collect();
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
    app.browser.tracks.items = (0..1000).map(track).collect();
    app.browser.tracks_for = Some("al0".into());
    app.browser.tracks.state.select(Some(999));

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

/// Help descriptions start in one column at every width, and ellipsize rather than clip.
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
        let frame = render(&mut app, w, 200);
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

/// Bars are centred, an odd leftover cell on the right, in both modes, art on or off.
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

/// Spread mode: the set number of bars once they fit, even bars and gaps, the anchor fixed.
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
                    bars.push(len);
                } else {
                    gaps.push(len);
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
    app.queue.items = tracks.clone();
    app.browser.tracks.items = tracks;
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
    app.queue.items = vec![track(0)];
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
    app.queue.items = (0..50).map(track).collect();
    app.queue.state.select(Some(7));
    app.playlists.list.items = (0..3).map(playlist).collect();
    app.playlists.tracks.items = (0..12).map(track).collect();
    app.playlists.tracks.state.select(Some(5));
    app.playlists.focus = PlaylistsCol::Items;
    app.playlists.tracks_for = Some("p0".into());
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
            matches: None,
            radio: None,
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
            [(60, [26, 36, 46, 55]), (100, [40, 60, 75, 95]), (160, [61, 92, 118, 155])],
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

/// One weight drives all four tables; 0 hides that column with the rest under their headers.
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
                    // the Results artist cell leads with "♪ "
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

/// A header at `w` columns; the app is dropped on return, as it holds the lock.
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

/// Mode words shorten with the width down to R Z S C, never icons; the title stays centred.
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

/// Each `ui.volume_display` draws what it names (unknown is both), the columns fixed.
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

/// Narrow compact rows drop the right side, then the sleep timer, before a tab is cut.
#[tokio::test]
async fn compact_row_steps_down_before_tab_clips() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    app.header_compact = true;
    run_cmd(&mut app, "sleep 15");
    let row = |app: &mut App, w: u16| row_text(&grid(app, w, 24), 1);
    for w in [120u16, 80, 60, 50, 48] {
        let text = row(&mut app, w);
        for name in ["Queue", "Discover", "Browser", "Playlists", "Search", "Help"] {
            assert!(text.contains(&format!(" {name} ")), "{name} at {w}: {text:?}");
        }
        assert!(!text.contains('>'), "no state glyph at {w}: {text:?}");
    }
    let wide = row(&mut app, 120);
    let left_of_tabs = wide.find("sleep 15m").zip(wide.find("Queue")).is_some_and(|(s, q)| s < q);
    assert!(left_of_tabs, "the timer on the left: {wide:?}");
    assert!(wide.contains("Volume: ") && wide.contains('▁'), "the meter at 120: {wide:?}");
    let mid = row(&mut app, 90);
    assert!(
        mid.contains("sleep 15m") && mid.contains('%') && !mid.contains('▁'),
        "a shorter volume at 90: {mid:?}"
    );
    let narrow = row(&mut app, 66);
    assert!(
        narrow.contains("sleep 15m") && !narrow.contains('%'),
        "only the timer at 66: {narrow:?}"
    );
    let tight = row(&mut app, 48);
    assert!(!tight.contains("sleep"), "{tight:?}");
}

fn queue_with_columns(cols: &[&str]) -> UiConfig {
    UiConfig { queue_columns: cols.iter().map(|c| c.to_string()).collect(), ..ascii_ui() }
}

fn tagged_queue(app: &mut App) {
    app.queue.items = (0..50)
        .map(|i| Track {
            year: Some(2008),
            audio: AudioInfo { codec: Some("flac".into()), ..AudioInfo::default() },
            ..track(i)
        })
        .collect();
    app.queue.state.select(Some(7));
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
