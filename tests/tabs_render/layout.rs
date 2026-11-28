use super::*;
use ratatui::widgets::TableState;
use shmc::plex::models::{Album, Artist};
use shmc::ui::app::{BrowserCol, SearchDetail};
use shmc::ui::theme;

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
            artist_key: None,
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
async fn browser_cuts_long_artist_name() {
    let mut app = new_app().await;
    app.tab = Tab::Browser;
    app.browser.tracks_for = Some("al0".into());
    app.browser.albums_for = Some("al0".into());
    app.browser.albums.items = vec![Album {
        rating_key: "al0".into(),
        title: "Evening Satellite".into(),
        artist: "Northwind Sounds".into(),
        artist_key: None,
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
