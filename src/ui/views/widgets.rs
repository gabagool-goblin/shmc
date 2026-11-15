use crate::ui::app::App;
use crate::ui::filter::{self, ListFilter};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::{list_split, render_scrollbar};
use crate::ui::theme;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

/// Collapse whitespace and drop format characters that can spoof a row (bidi
/// controls, zero-widths, BOM). ZWJ and variation selectors stay.
pub(super) fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.trim().chars() {
        if is_spoofing_format(c) {
            continue;
        }
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else if c.is_control() {
            continue;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// A format character that renders as nothing or reorders the text around it.
fn is_spoofing_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}'                  // soft hyphen
        | '\u{061C}'                // arabic letter mark
        | '\u{180E}'                // mongolian vowel separator
        | '\u{200B}'                // zero-width space
        | '\u{200E}' | '\u{200F}'   // left-to-right / right-to-left mark
        | '\u{202A}'..='\u{202E}'   // bidi embedding / override
        | '\u{2060}'..='\u{2064}'   // word joiner and invisible operators
        | '\u{2066}'..='\u{2069}'   // bidi isolates
        | '\u{FEFF}'                // BOM / zero-width no-break space
    )
}

/// Ellipsized cell text, with the filter match highlighted.
pub(super) fn cell(text: &str, w: u16, needle: &str, base: Style) -> Cell<'static> {
    let shown = theme::ellipsize(&clean(text), w);
    Cell::from(Line::from(spans_clean(shown, needle, base)))
}

pub(super) fn spans_for(text: &str, needle: &str, base: Style) -> Vec<Span<'static>> {
    spans_clean(clean(text), needle, base)
}

pub(super) fn spans_clean(text: String, needle: &str, base: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(text, base)];
    }
    filter::highlight(&text, needle, base, theme::secondary())
        .into_iter()
        .map(|s| Span::styled(s.content.into_owned(), s.style))
        .collect()
}

/// The `filter: text (n/total)` line; None while empty or typing (the footer
/// shows the input then).
pub(super) fn indicator_text(f: &ListFilter, total: usize) -> Option<String> {
    if f.typing {
        None
    } else {
        f.indicator(total)
    }
}

/// Draw the filter indicator row and split off the scrollbar columns.
pub(super) fn begin_list(
    f: &mut Frame,
    pane: Rect,
    total: usize,
    filter: &ListFilter,
    raw: usize,
) -> (Rect, Option<Rect>) {
    let mut body = pane;
    if let Some(ind) = indicator_text(filter, raw) {
        if body.height > 1 {
            let r = Rect { x: body.x, y: body.y, width: body.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(Span::styled(ind, theme::dim()))), r);
            body.y += 1;
            body.height -= 1;
        }
    }
    list_split(body, total, body.height as usize)
}

pub(super) fn render_bar(
    f: &mut Frame,
    bar: Option<Rect>,
    total: usize,
    offset: usize,
    visible: usize,
) {
    if let Some(bar) = bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

/// This frame's offset and visible rows for a list.
pub(super) fn window(
    app: &mut App,
    id: ScrollId,
    cursor: Option<usize>,
    total: usize,
    area: Rect,
) -> (usize, usize) {
    let visible = area.height as usize;
    let offset = app.scroll.update(id, cursor.unwrap_or(0), visible, total, app.scrolloff);
    (offset, visible)
}

pub(super) fn build_range(
    offset: usize,
    visible: usize,
    total: usize,
    mut push: impl FnMut(usize, &mut Vec<ListItem<'static>>),
) -> Vec<ListItem<'static>> {
    let end = (offset + visible).min(total);
    let mut items = Vec::new();
    for fi in offset..end {
        push(fi, &mut items);
    }
    items
}

pub(super) fn render_list(
    f: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'static>>,
    cursor: Option<usize>,
    offset: usize,
    highlight: Style,
) {
    if area.width == 0 || area.height == 0 || items.is_empty() {
        return;
    }
    let mut state = ListState::default();
    if let Some(c) = cursor {
        state.select(Some(c.saturating_sub(offset).min(items.len() - 1)));
    }
    f.render_stateful_widget(List::new(items).highlight_style(highlight), area, &mut state);
}

/// Cursor style: full when the pane has focus, muted otherwise.
pub(super) fn hl(focused: bool) -> Style {
    if focused {
        theme::current()
    } else {
        theme::current_unfocused()
    }
}

/// An active filter with no hits.
pub(super) fn no_matches(f: &mut Frame, area: Rect) {
    placeholder(f, area, "No matches");
}

pub(super) fn placeholder(f: &mut Frame, area: Rect, msg: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let line_area = Rect { x: area.x, y, width: area.width, height: 1 };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg.to_string(), theme::dim())))
            .alignment(Alignment::Center),
        line_area,
    );
}

/// The bordered "No cover" box.
pub(super) fn placeholder_box(f: &mut Frame, rect: Rect, msg: &str) {
    if rect.width < 3 || rect.height < 3 || theme::spacing().borders == theme::BorderStyle::None {
        placeholder(f, rect, msg);
        return;
    }
    let block = Block::bordered().border_style(theme::border());
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let line_area =
        Rect { x: inner.x, y: inner.y + inner.height / 2, width: inner.width, height: 1 };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg.to_string(), theme::dim())))
            .alignment(Alignment::Center),
        line_area,
    );
}

#[cfg(test)]
mod tests {
    use super::super::queue::art_rect;
    use super::*;

    /// The box is centred exactly, with equal leftover on both sides of both
    /// axes, whatever the pane and cell size.
    #[test]
    fn placeholder_box_centred_in_cells() {
        let cells =
            [Some((8, 18)), Some((16, 36)), Some((22, 46)), Some((10, 20)), Some((9, 16)), None];
        for cell in cells {
            for width in 4..=80u16 {
                for height in 4..=40u16 {
                    let content = Rect { x: 3, y: 5, width, height };
                    let Some(r) = art_rect(content, cell) else { continue };
                    let what = format!("{cell:?} {width}x{height}");
                    assert_eq!(r.x - content.x, content.right() - r.right(), "{what}");
                    assert_eq!(r.y - content.y, content.bottom() - r.bottom(), "{what}");
                    assert!(r.width <= width && r.height <= height, "{what}");
                    let Some((cw, ch)) = cell else { continue };
                    let (pw, ph) = (r.width as u32 * cw as u32, r.height as u32 * ch as u32);
                    let one = cw.max(ch) as u32;
                    let side = (width as u32 * cw as u32).min(height as u32 * ch as u32);
                    assert!(pw.max(ph).abs_diff(side) <= one, "{what}: {pw}x{ph} vs {side}");
                    // Only a box that fills an axis can be forced off square.
                    if r.width < width && r.height < height {
                        assert!(pw.abs_diff(ph) <= one, "{what}: {pw}x{ph}");
                    }
                }
            }
        }
        // 16x36 px cells, 40x18 cells is 640x648 px.
        let r = art_rect(Rect { x: 0, y: 0, width: 40, height: 18 }, Some((16, 36))).expect("box");
        assert_eq!((r.width, r.height), (40, 18));

        // Without a cell size: 2:1.
        for (w, h, want) in
            [(44, 28, (44, 22)), (45, 40, (45, 24)), (30, 10, (20, 10)), (21, 12, (21, 12))]
        {
            let r = art_rect(Rect { x: 0, y: 0, width: w, height: h }, None).expect("box");
            assert_eq!((r.width, r.height), want, "{w}x{h}");
        }

        // too small
        assert_eq!(art_rect(Rect { x: 0, y: 0, width: 3, height: 10 }, None), None);
        assert_eq!(art_rect(Rect { x: 0, y: 0, width: 10, height: 1 }, None), None);
    }

    #[test]
    fn clean_drops_spoofing_format_characters() {
        assert_eq!(clean("abc\u{202E}def"), "abcdef");
        assert_eq!(clean("a\u{200B}b"), "ab");
        assert_eq!(clean("a\u{FEFF}b"), "ab");
        // emoji and shaping controls survive
        assert_eq!(clean("a\u{200D}b\u{FE0F}"), "a\u{200D}b\u{FE0F}");
        assert_eq!(clean("a\u{200C}b"), "a\u{200C}b");
    }
}
