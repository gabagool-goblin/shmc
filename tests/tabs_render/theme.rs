use super::*;
use ratatui::style::Color;
use shmc::plex::models::Artist;
use shmc::ui::theme;

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
    app.browser.artists.items = (0..40)
        .map(|i| Artist {
            rating_key: format!("a{i}"),
            title: format!("Artist {i:02}"),
            thumb: None,
            plays: None,
        })
        .collect();
    app.browser.artists.state.select(Some(3));

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
    app.picker = Some(ratatui_image::picker::Picker::halfblocks());
    let red = image::Rgba([255, 0, 0, 255]);
    app.art.img = Some(Arc::new(DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, red))));

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
            .any(|p| first.cell(p).is_some_and(|c| c.bg == Color::Rgb(255, 0, 0))),
        "the first frame carries the image"
    );
    // the letterbox must be opaque; the corner is padding
    let corner = first.cell((rect.left(), rect.top())).unwrap();
    let bg = Color::Rgb(0x2e, 0x34, 0x40);
    assert!(corner.fg == bg && corner.bg == bg, "padding is {:?}/{:?}", corner.fg, corner.bg);
    let plain = unpainted.cell((rect.left(), rect.top())).unwrap();
    assert!(plain.fg != bg && plain.bg != bg, "without a background it is not painted");

    assert!(
        cells_skipped(&mut terminal, &mut app, rect),
        "cover cells were not skipped on the second frame"
    );
    // the backend buffer still holds the first frame's image cells
    let after = terminal.backend().buffer();
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            assert_eq!(after.cell((x, y)), first.cell((x, y)), "({x},{y}) was rewritten");
        }
    }
}

/// Restores the process-wide look when a test ends; tests hold `TAB_TEST_LOCK`.
pub(super) struct LookGuard(theme::Look);

impl LookGuard {
    pub(super) fn set(look: theme::Look) -> Self {
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

pub(super) fn buffer_of(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

const TABS: [Tab; 6] =
    [Tab::Queue, Tab::Discover, Tab::Browser, Tab::Playlists, Tab::Search, Tab::Help];

/// `block_gap` puts empty rows between the blocks; it gives way before the content's six rows.
#[tokio::test]
async fn block_gap_yields_to_content() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, false);
    for gap in 0..=3u16 {
        let _look = LookGuard::set(spaced(gap, 1));
        for h in [16u16, 18, 22, 30, 45, 60] {
            let g = grid(&mut app, 80, h);
            let eff = gap.min(h.saturating_sub(16) / 3);
            let tops: Vec<u16> = (0..h).filter(|&y| g[y as usize][0] == "┌").collect();
            let content = 7 + 2 * eff;
            assert_eq!(tops, [0, 4 + eff, content, h - 3], "gap {gap} at {h} rows");
            assert!(h - 3 - eff - content >= 6, "content keeps six rows at {h}");
            let blank = |ys: std::ops::Range<u16>| {
                ys.for_each(|y| assert!(g[y as usize].iter().all(|c| c == " "), "row {y} at {h}"));
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

    assert!(
        cells_skipped(&mut terminal, &mut app, after),
        "cover cells were not skipped on the second frame"
    );
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
    assert!(
        (4..80u16).all(|x| solid.cell((x, 28)).unwrap().symbol() != "░"),
        "the track must not be dotted"
    );
    let unplayed = theme::border().fg.unwrap();
    assert!(
        (4..80u16).any(|x| solid.cell((x, 28)).unwrap().bg == unplayed),
        "the unplayed part is a coloured background"
    );

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

/// Caps take cells and colour from the bar; a head over a solid track has it as background.
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
    app.queue.items.truncate(3); // no scrollbar thumb to confuse the comparison
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
    app.queue.items[2].title = "A very long title that cannot possibly fit in the cell".into();
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

pub(super) fn with_borders(style: theme::BorderStyle) -> theme::Look {
    theme::Look {
        spacing: theme::Spacing { borders: style, ..theme::Spacing::DEFAULT },
        ..theme::Look::DEFAULT
    }
}

pub(super) const STYLES: [theme::BorderStyle; 5] = [
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

/// Every header mode and border style keeps the tabs, content, progress row and now-playing.
#[tokio::test]
async fn every_header_border_combo_fits() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.queue.items[2].title = "Unique Song".into();
    app.position = 42.0;
    app.duration = 257.0;
    for compact in [false, true] {
        app.ui.header_compact = compact;
        for style in STYLES {
            let _look = LookGuard::set(with_borders(style));
            for (w, h) in [(120u16, 40u16), (80, 24), (50, 20), (80, 60)] {
                let (tab_y, content_y, progress_y) = chrome_rows(compact, style, h);
                let at = format!("{style:?} compact={compact} {w}x{h}");
                for tab in TABS {
                    app.tab = tab;
                    let g = grid(&mut app, w, h);
                    let tabs = row_text(&g, tab_y);
                    for name in ["Queue", "Discover", "Browser", "Playlists", "Search", "Help"] {
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

/// Each border style draws its own glyphs, and `none` draws no frame glyph.
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
                    assert!(text.contains('╭') && !text.contains('┌'), "{tab:?}");
                }
                theme::BorderStyle::Double => {
                    assert!(
                        text.contains('╔') && text.contains('╝') && !text.contains('┌'),
                        "{tab:?}"
                    );
                }
                theme::BorderStyle::Lines => {
                    assert!(!text.contains('┌') && text.contains('─'), "{tab:?}");
                }
                theme::BorderStyle::Box => {
                    assert!(text.contains('┌') && !text.contains('╭'), "{tab:?}");
                }
            }
        }
    }
}

/// Compact `none` keeps the row above the progress row, so the content height never changes.
#[tokio::test]
async fn compact_without_borders_keeps_now_playing_row() {
    let mut app = new_app().await;
    queue_app_with_art(&mut app, true);
    app.ui.art_metadata = true;
    app.queue.items[2].title = "Unique Song".into();
    app.ui.header_compact = true;
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
