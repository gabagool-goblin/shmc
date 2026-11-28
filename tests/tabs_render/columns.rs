use super::*;
use ratatui::widgets::TableState;
use shmc::ui::app::{BrowserCol, SearchDetail};

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
    assert_eq!(app.ui.column_weights, [30, 24, 46]);
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
        app.ui.column_weights = [80, 10, 10];
        assert!(title_w(&mut app, 120) > base + 10, "{table:?}: a heavier Title is wider");
        app.ui.column_weights = [30, 24, 46];
    }

    for (weights, hidden) in
        [([50, 0, 50], "Artist"), ([0, 40, 60], "Title"), ([60, 40, 0], "Album")]
    {
        app.ui.column_weights = weights;
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
