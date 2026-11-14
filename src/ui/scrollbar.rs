//! The scrollbar every scrollable list draws through. A pane loses its gutter
//! and bar column only when the list overflows.

use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::Frame;

/// Split `pane` into `(content, bar)`: on overflow the content loses a blank
/// gutter and the bar column, otherwise it keeps the whole pane.
pub fn list_split(pane: Rect, total: usize, visible: usize) -> (Rect, Option<Rect>) {
    if visible > 0 && total > visible && pane.width >= 4 {
        let content = Rect { width: pane.width - 2, ..pane };
        let bar = Rect { x: pane.x + pane.width - 1, y: pane.y, width: 1, height: pane.height };
        (content, Some(bar))
    } else {
        (pane, None)
    }
}

/// The thumb's first row and length. The length depends only on
/// `total`/`visible`, so it never changes while scrolling.
fn thumb_geometry(total: usize, visible: usize, offset: usize, track: usize) -> (usize, usize) {
    if track == 0 || visible == 0 || total <= visible {
        return (0, 0);
    }
    let thumb_len =
        ((track as f64 * visible as f64 / total as f64).round() as usize).clamp(1, track);
    let max_offset = total - visible;
    let travel = track - thumb_len;
    let start = if max_offset == 0 {
        0
    } else {
        ((offset.min(max_offset) as f64 / max_offset as f64) * travel as f64).round() as usize
    };
    (start.min(travel), thumb_len)
}

/// Draw the ▲/▼ bar into the 1-cell column from [`list_split`]; `offset` is the
/// first visible row.
pub fn render_scrollbar(f: &mut Frame, area: Rect, total: usize, offset: usize, visible: usize) {
    if visible == 0 || area.width == 0 || total <= visible {
        return;
    }
    let track = area.height.saturating_sub(2) as usize; // ▲ and ▼ take two rows
    let (start, len) = thumb_geometry(total, visible, offset, track);
    if len == 0 {
        return;
    }

    let x = area.x + area.width - 1;
    let top = area.y;
    let bottom = area.y + area.height - 1;
    let symbols = theme::symbols();
    let buf = f.buffer_mut();
    buf.set_string(x, top, symbols.scroll_up.to_string(), theme::dim());
    for row in 0..track {
        let y = top + 1 + row as u16;
        if row >= start && row < start + len {
            buf.set_string(x, y, symbols.scroll_thumb.to_string(), theme::accent());
        } else {
            buf.set_string(x, y, "║", theme::dim());
        }
    }
    buf.set_string(x, bottom, symbols.scroll_down.to_string(), theme::dim());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_length_is_constant_while_scrolling() {
        let (_, len) = thumb_geometry(228, 20, 0, 28);
        for offset in 0..=228 - 20 {
            assert_eq!(
                thumb_geometry(228, 20, offset, 28).1,
                len,
                "thumb changed size at offset {offset}"
            );
        }
    }

    #[test]
    fn thumb_touches_bottom_on_last_page() {
        let total = 20;
        let visible = 12;
        let track = 28;
        assert_eq!(thumb_geometry(total, visible, 0, track).0, 0);
        let (start, len) = thumb_geometry(total, visible, total - visible, track);
        assert_eq!(start + len, track, "thumb must reach the bottom");
    }

    #[test]
    fn thumb_start_is_monotonic() {
        let mut previous = 0;
        for offset in 0..=228 - 20 {
            let start = thumb_geometry(228, 20, offset, 28).0;
            assert!(start >= previous, "thumb moved backwards at {offset}");
            previous = start;
        }
    }

    #[test]
    fn geometry_handles_edges_and_clamps() {
        assert_eq!(thumb_geometry(10, 10, 0, 20), (0, 0));
        assert_eq!(thumb_geometry(10, 20, 0, 20), (0, 0));
        assert_eq!(thumb_geometry(100, 0, 0, 20), (0, 0));
        assert_eq!(thumb_geometry(100, 10, 0, 0), (0, 0));

        let (start, len) = thumb_geometry(100, 20, 9_999, 30);
        assert_eq!(start + len, 30, "offsets past the end clamp to the bottom");
        assert!(len <= 30);
    }
}
