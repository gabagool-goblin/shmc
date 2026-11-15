use super::tables::{build_track_rows, table_frame, QueueCols};
use super::visualizer::draw_visualizer;
use super::widgets::{clean, indicator_text, no_matches, placeholder, placeholder_box};
use super::{draw_block, draw_divider};
use crate::ui::app::App;
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::{list_split, render_scrollbar};
use crate::ui::theme;
use image::DynamicImage;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Borders, Paragraph, Table, TableState};
use ratatui::Frame;
use std::time::Instant;

/// True when the cover's metadata was drawn.
pub(super) fn draw_queue_tab(f: &mut Frame, app: &mut App, area: Rect) -> bool {
    let inner = draw_block(f, area);
    if inner.width < 10 || inner.height < 5 {
        return false;
    }
    // The art pane is pixel-square, capped at `art_pct` so the queue keeps its
    // columns. With art disabled it collapses and the queue gets the full width.
    let (art_area, queue_area) = if !app.art_enabled {
        (None, inner)
    } else {
        let cols = if app.art_square && app.picker.is_some() {
            let cell = app.picker.as_ref().map(|p| p.font_size()).unwrap_or((10, 20));
            let img_h = inner.height - art_meta_rows(inner.height, app.art_metadata);
            let square = square_art_width(img_h, cell);
            let art_pct = app.art_pct.clamp(10, 90);
            let max_by_pct = (inner.width as u32 * art_pct as u32 / 100) as u16;
            let max_art = inner.width.saturating_sub(MIN_QUEUE_COLS).min(max_by_pct);
            let art_w = square
                .clamp(MIN_ART_COLS, max_art.max(MIN_ART_COLS))
                .min(inner.width.saturating_sub(1).max(1));
            Layout::horizontal([Constraint::Length(art_w), Constraint::Min(1)]).split(inner)
        } else {
            let art_pct = app.art_pct.clamp(10, 90);
            Layout::horizontal([
                Constraint::Percentage(art_pct),
                Constraint::Percentage(100 - art_pct),
            ])
            .split(inner)
        };
        // The divider hugs the pane, so the queue column gets its own gutter.
        let q = cols[1];
        let gutter = theme::spacing().inset.min(q.width);
        let q = Rect { x: q.x + gutter, width: q.width - gutter, ..q };
        (Some(cols[0]), q)
    };
    let meta_shown = art_area.is_some_and(|pane| draw_art_pane(f, app, pane));

    let viz_h = viz_height(app.visualizer_pct, queue_area.height);
    let rows = Layout::vertical([
        Constraint::Length(queue_area.height.saturating_sub(viz_h)),
        Constraint::Length(viz_h),
    ])
    .split(queue_area);
    let qarea = draw_divider(f, rows[0], Borders::BOTTOM);
    draw_queue_table(f, app, qarea);
    if viz_h > 0 {
        draw_visualizer(f, app, rows[1]);
    }
    meta_shown
}

/// The visualizer is never shorter than this, space permitting.
const MIN_VIZ_ROWS: u16 = 8;

/// The queue keeps at least this many columns beside the art.
const MIN_QUEUE_COLS: u16 = 30;

/// The art pane's floor, except on very narrow terminals.
const MIN_ART_COLS: u16 = 6;

/// Top separator + title + artist + album.
const ART_META_ROWS: u16 = 4;

/// Rows the metadata footer takes; none when it is off or would leave the cover too small.
fn art_meta_rows(height: u16, metadata: bool) -> u16 {
    if metadata && height >= ART_META_ROWS + 4 {
        ART_META_ROWS
    } else {
        0
    }
}

/// Width in cells of a pixel-square area `height` rows tall.
fn square_art_width(height: u16, cell: (u16, u16)) -> u16 {
    if height == 0 || cell.0 == 0 || cell.1 == 0 {
        return 0;
    }
    ((height as f64 * cell.1 as f64 / cell.0 as f64).round() as u16).max(1)
}

/// Visualizer rows for a percentage of the queue column (0 hides it). At least
/// `MIN_VIZ_ROWS`, but the queue keeps 6 rows: the visualizer shrinks to 3, then hides.
fn viz_height(pct: u16, available: u16) -> u16 {
    if pct == 0 {
        return 0;
    }
    let want = (available as u32 * pct.min(100) as u32 / 100) as u16;
    let v = want.max(MIN_VIZ_ROWS).min(available.saturating_sub(6));
    if v < 3 {
        0
    } else {
        v
    }
}

/// True when the metadata footer was drawn.
fn draw_art_pane(f: &mut Frame, app: &mut App, pane: Rect) -> bool {
    draw_divider(f, pane, Borders::RIGHT);
    // The left gutter and the right divider are the only frame.
    let inset = theme::spacing().inset;
    let x = pane.x.saturating_sub(inset);
    let content = Rect {
        x,
        y: pane.y,
        // ends just before the divider
        width: (pane.x + pane.width).saturating_sub(1).saturating_sub(x),
        height: pane.height,
    };
    if content.width < 6 || content.height < 4 {
        return false;
    }

    if app.art_square && app.picker.is_some() {
        let meta_h = art_meta_rows(content.height, app.art_metadata);
        let art_h = content.height.saturating_sub(meta_h);
        let image_area = Rect { height: art_h, ..content };
        draw_art_image(f, app, image_area);
        if meta_h > 0 {
            let meta_area = Rect { y: content.y + art_h, height: meta_h, ..content };
            return draw_art_meta(f, app, meta_area);
        }
        return false;
    }
    draw_art_image(f, app, content);
    false
}

fn draw_art_image(f: &mut Frame, app: &mut App, content: Rect) {
    if content.width < 6 || content.height < 4 {
        return;
    }

    // A cover is placed on a canvas of the whole area, to the pixel; without
    // one the placeholder box depends only on the layout.
    let covered = app.art.img.is_some() && app.picker.is_some();
    let rect = if covered {
        Some(content)
    } else {
        art_rect(content, app.picker.as_ref().map(|p| p.font_size()))
    };
    let Some(rect) = rect else {
        placeholder(f, content, "No cover");
        return;
    };

    // The terminal already holds this image: skip its cells rather than copy
    // the protocol payload again.
    if app.art.protocol.is_some()
        && app.art.shown_rect == Some(rect)
        && app.art.prepared_rect == Some(rect)
    {
        let buf = f.buffer_mut();
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_skip(true);
                }
            }
        }
        return;
    }

    // Cells were just cleared: the cover waits (the small "No cover" box does not).
    if let Some(until) = app.art.hold_until {
        if covered && Instant::now() < until {
            return;
        }
        app.art.hold_until = None;
    }

    // ratatui-image pads a resized image at (0, 0), so centre the cover on a
    // canvas of the rect's size first.
    if app.art.protocol.is_none() || app.art.prepared_rect != Some(rect) {
        let picker = app.picker;
        let img = app.art.img.clone();
        if let (Some(picker), Some(img)) = (picker, img) {
            let cell = picker.font_size();
            if let Some(canvas) = centered_art_canvas(
                img.as_ref(),
                rect,
                cell,
                app.art_pad_px,
                app.art_max_px,
                art_padding_fill(),
            ) {
                app.art.protocol = Some(picker.new_resize_protocol(canvas));
                app.art.prepared_rect = Some(rect);
            } else {
                app.art.protocol = None;
                app.art.prepared_rect = None;
            }
        } else {
            app.art.protocol = None;
            app.art.prepared_rect = None;
        }
    }
    if let Some(prot) = app.art.protocol.as_mut() {
        // The canvas is already centred; Scale fills `rect` exactly.
        let widget =
            ratatui_image::StatefulImage::default().resize(ratatui_image::Resize::Scale(None));
        f.render_stateful_widget(widget, rect, prot);
        app.art.shown_rect = Some(rect);
        app.art.emitted = true;
    } else {
        placeholder_box(f, rect, "No cover");
        app.art.shown_rect = None;
    }
}

/// Title, artist and album below the cover.
fn draw_art_meta(f: &mut Frame, app: &App, area: Rect) -> bool {
    if area.width < 10 || area.height < 3 {
        return false;
    }
    let Some(t) = app.current_track() else { return false };
    draw_divider(f, area, Borders::TOP);
    let text_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };
    if text_area.width == 0 || text_area.height == 0 {
        return false;
    }
    let lines = vec![
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.title), text_area.width),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.artist), text_area.width),
            theme::secondary(),
        )),
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.album), text_area.width),
            theme::text(),
        )),
    ];
    f.render_widget(Paragraph::new(lines), text_area);
    true
}

/// The letterbox: transparent without a theme background, else opaque in it.
/// Some terminals draw transparent pixels over their own background.
fn art_padding_fill() -> image::Rgba<u8> {
    match theme::theme().background {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => image::Rgba([0, 0, 0, 0]),
    }
}

/// The cover fitted and centred in pixels on a canvas the size of `area`, with
/// `pad_px` on the limiting axis and at most `max_px` (0 = unlimited).
fn centered_art_canvas(
    img: &DynamicImage,
    area: Rect,
    cell: (u16, u16),
    pad_px: u16,
    max_px: [u32; 2],
    fill: image::Rgba<u8>,
) -> Option<DynamicImage> {
    let cw = area.width as u32 * cell.0 as u32;
    let ch = area.height as u32 * cell.1 as u32;
    if cw == 0 || ch == 0 {
        return None;
    }
    let iw = img.width().max(1) as f64;
    let ih = img.height().max(1) as f64;
    let pad = (pad_px as u32).min((cw.min(ch) - 1) / 2);
    let (fit_w, fit_h) = (cw - 2 * pad, ch - 2 * pad);
    let mut scale = (fit_w as f64 / iw).min(fit_h as f64 / ih);
    for (cap, side) in [(max_px[0], iw), (max_px[1], ih)] {
        if cap > 0 {
            scale = scale.min(cap as f64 / side);
        }
    }
    let w = ((iw * scale).round() as u32).clamp(1, fit_w);
    let h = ((ih * scale).round() as u32).clamp(1, fit_h);
    let (x, y) = ((cw - w) / 2, (ch - h) / 2);
    let cover = img.resize_exact(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(cw, ch, fill);
    image::imageops::overlay(&mut canvas, &cover, x as i64, y as i64);
    Some(DynamicImage::ImageRgba8(canvas))
}

/// The whole-cell box closest to the cover's square, with an even leftover so
/// it centres exactly. `None` when the pane is too small.
pub(super) fn art_rect(content: Rect, cell: Option<(u16, u16)>) -> Option<Rect> {
    let (art_w, art_h) = match cell.filter(|c| c.0 > 0 && c.1 > 0) {
        Some(cell) => square_box(content, cell),
        None => two_to_one_box(content),
    };
    if art_w < 4 || art_h < 2 {
        return None;
    }
    Some(Rect {
        x: content.x + (content.width - art_w) / 2,
        y: content.y + (content.height - art_h) / 2,
        width: art_w,
        height: art_h,
    })
}

/// The box for unknown cell sizes: 1:2 in cells, as wide as the height allows.
fn two_to_one_box(content: Rect) -> (u16, u16) {
    let mut art_w = content.width.min(content.height.saturating_mul(2));
    if (content.width - art_w) % 2 == 1 {
        art_w -= 1;
    }
    let mut art_h = art_w.div_ceil(2);
    if (content.height - art_h) % 2 == 1 {
        art_h += 1;
    }
    (art_w, art_h)
}

/// The (width, height) closest to a pixel square of the shorter side, with an
/// even leftover; a tie takes the larger box.
fn square_box(content: Rect, cell: (u16, u16)) -> (u16, u16) {
    let (cw, ch) = (cell.0 as u32, cell.1 as u32);
    let side = (content.width as u32 * cw).min(content.height as u32 * ch);
    let w0 = ((side + cw / 2) / cw).min(content.width as u32);
    let h0 = ((side + ch / 2) / ch).min(content.height as u32);
    let mut best: Option<(u32, u32, u32)> = None;
    for w in w0.saturating_sub(1)..=w0 + 1 {
        for h in h0.saturating_sub(1)..=h0 + 1 {
            if w == 0 || h == 0 || w > content.width as u32 || h > content.height as u32 {
                continue;
            }
            if (content.width as u32 - w) % 2 == 1 || (content.height as u32 - h) % 2 == 1 {
                continue;
            }
            let off = (w * cw).abs_diff(h * ch);
            if best.is_none_or(|(o, bw, bh)| off < o || (off == o && w * h > bw * bh)) {
                best = Some((off, w, h));
            }
        }
    }
    best.map_or((0, 0), |(_, w, h)| (w as u16, h as u16))
}

fn draw_queue_table(f: &mut Frame, app: &mut App, area: Rect) {
    if area.width < 10 || area.height < 3 {
        return;
    }
    let raw = app.queue.len();
    let total = app.queue_filter.len(raw);
    let ind = indicator_text(&app.queue_filter, raw);
    let visible_rows = area.height.saturating_sub(u16::from(ind.is_some()) + 2) as usize;
    let (content, _) = list_split(area, total, visible_rows);
    let qc = QueueCols::new(&app.queue_columns, app.column_weights, &app.queue, content.width);
    let frame = table_frame(f, area, total, &qc.constraints(), Some(&qc.headers()), ind);

    let cursor = app.queue_state.selected();
    let visible = frame.rows.height as usize;
    let offset =
        app.scroll.update(ScrollId::Queue, cursor.unwrap_or(0), visible, total, app.scrolloff);
    if total == 0 {
        if app.queue_filter.is_active() {
            no_matches(f, frame.rows);
        }
        if let Some(bar) = frame.bar {
            render_scrollbar(f, bar, total, offset, visible);
        }
        return;
    }

    let needle = app.queue_filter.text.as_str();
    let current_key = app.current_track().map(|t| t.rating_key.clone());
    let rows = build_track_rows(
        &app.queue,
        &app.queue_filter,
        offset,
        visible,
        total,
        current_key.as_deref(),
        app.visual_range(),
        |t, _, base, playing| qc.cells(t, base, playing, &frame.col_w, needle),
    );

    let col_w: Vec<Constraint> = frame.col_w.iter().map(|w| Constraint::Length(*w)).collect();
    let mut state = TableState::default();
    if let Some(c) = cursor {
        state.select(Some(c.saturating_sub(offset).min(rows.len().saturating_sub(1))));
    }
    let table = Table::new(rows, col_w).column_spacing(1).row_highlight_style(theme::current());
    f.render_stateful_widget(table, frame.rows, &mut state);
    if let Some(bar) = frame.bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visualizer_share_does_not_starve_queue() {
        assert_eq!(viz_height(50, 28), 14);
        assert_eq!(viz_height(50, 40), 20);
        // capped so the queue keeps 6 rows
        assert_eq!(viz_height(100, 28), 22);
        assert_eq!(viz_height(80, 10), 4);
        assert_eq!(viz_height(0, 28), 0);
        // the floor
        assert_eq!(viz_height(50, 17), 8);
        assert_eq!(viz_height(50, 14), 8);
        assert_eq!(viz_height(50, 13), 7);
        // tight: the 3-row floor
        assert_eq!(viz_height(50, 9), 3);
        // too tight: hidden
        assert_eq!(viz_height(50, 8), 0);
        assert_eq!(viz_height(50, 6), 0);
        assert_eq!(viz_height(50, 4), 0);
    }

    #[test]
    fn square_art_width_matches_the_cell_aspect() {
        assert_eq!(square_art_width(30, (10, 20)), 60);
        assert_eq!(square_art_width(30, (10, 10)), 30);
        assert_eq!(square_art_width(0, (10, 20)), 0);
    }

    /// The opaque pixels' gaps to the canvas edge (left, right, top, bottom) and size.
    fn cover_gaps(canvas: &DynamicImage) -> ([u32; 4], (u32, u32)) {
        let rgba = canvas.to_rgba8();
        let (w, h) = rgba.dimensions();
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
        for (x, y, p) in rgba.enumerate_pixels() {
            if p[3] > 0 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        assert!(x0 <= x1 && y0 <= y1, "the cover is on the canvas");
        ([x0, w - 1 - x1, y0, h - 1 - y1], (x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Opposite gaps differ by at most a pixel, the limiting axis has the
    /// padding, and the cap holds.
    #[test]
    fn cover_centred_in_pixels_at_every_size() {
        let covers = [(300u32, 300u32), (400, 200), (200, 400), (300, 290)];
        for cell in [(8u16, 18u16), (16, 36), (22, 46)] {
            // both parities of both axes, without a slow sweep
            let pads: &[u16] = if cell == (16, 36) { &[1, 3] } else { &[1] };
            for &pad in pads {
                for width in [20u16, 25, 31, 36, 42, 47, 53, 58, 64, 69] {
                    for height in [8u16, 11, 15, 18, 22, 25, 29, 32] {
                        for (iw, ih) in covers {
                            for cap in [[0u32, 0u32], [200, 300]] {
                                let tag = format!(
                                    "cell {cell:?} pad {pad} pane {width}x{height} cover {iw}x{ih} cap {cap:?}"
                                );
                                let area = Rect::new(0, 0, width, height);
                                let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                                    iw,
                                    ih,
                                    image::Rgb([9, 9, 9]),
                                ));
                                let canvas = centered_art_canvas(
                                    &img,
                                    area,
                                    cell,
                                    pad,
                                    cap,
                                    image::Rgba([0, 0, 0, 0]),
                                )
                                .expect("canvas");
                                assert_eq!(
                                    (canvas.width(), canvas.height()),
                                    (width as u32 * cell.0 as u32, height as u32 * cell.1 as u32),
                                    "{tag}"
                                );
                                let ([l, r, t, b], (w, h)) = cover_gaps(&canvas);
                                assert!(l.abs_diff(r) <= 1, "horizontal {l}/{r}: {tag}");
                                assert!(t.abs_diff(b) <= 1, "vertical {t}/{b}: {tag}");
                                if cap == [0, 0] {
                                    let pad = pad as u32;
                                    let wide = l.abs_diff(pad) <= 1 && r.abs_diff(pad) <= 1;
                                    let tall = t.abs_diff(pad) <= 1 && b.abs_diff(pad) <= 1;
                                    assert!(wide || tall, "limiting axis has the padding: {tag}");
                                } else {
                                    assert!(
                                        w <= cap[0] && h <= cap[1],
                                        "{w}x{h} over the cap: {tag}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// With a theme background the letterbox is that colour, opaque; without
    /// one it stays transparent.
    #[test]
    fn cover_padding_takes_theme_background() {
        let _palette = theme::palette_test_lock();
        let img =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(100, 50, image::Rgb([9, 9, 9])));
        let area = Rect::new(0, 0, 10, 10);
        let canvas =
            |fill| centered_art_canvas(&img, area, (8, 16), 1, [0, 0], fill).unwrap().to_rgba8();
        // a wide cover in a tall pane: the top-left pixel is padding
        assert_eq!(canvas(art_padding_fill()).get_pixel(0, 0).0[3], 0);
        let mut palette = theme::theme();
        palette.background = Some(ratatui::style::Color::Rgb(0xef, 0xf1, 0xf5));
        let previous = theme::theme();
        theme::set_theme(palette);
        let fill = art_padding_fill();
        theme::set_theme(previous);
        assert_eq!(fill.0, [0xef, 0xf1, 0xf5, 255]);
        let with = canvas(fill);
        assert_eq!(with.get_pixel(0, 0).0, [0xef, 0xf1, 0xf5, 255]);
        assert_eq!(with.get_pixel(40, 80).0, [9, 9, 9, 255], "the cover itself is untouched");
    }
}
