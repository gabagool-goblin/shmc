use crate::ui::app::App;
use crate::ui::keymap::HelpLine;
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::list_split;
use crate::ui::theme;
use crate::ui::views::draw_block;
use crate::ui::views::widgets::{render_bar, render_list, window};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use ratatui::Frame;

/// The widest the Help tab's key column grows, and the gap after it.
const HELP_KEY_COL_MAX: usize = 22;

const HELP_GAP: usize = 2;

pub(in crate::ui::views) fn draw_help_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = draw_block(f, area);
    if inner.width < 10 || inner.height < 2 {
        return;
    }
    let lines = app.keymap.help_lines();
    let total = lines.len();
    let (content, bar) = list_split(inner, total, inner.height as usize);
    let cursor = app.help_state.selected();
    let (offset, visible) = window(app, ScrollId::Help, cursor, total, content);
    let end = (offset + visible).min(total);

    // One key column, capped to leave the descriptions room, so they all start
    // in the same place.
    let widest = lines
        .iter()
        .filter_map(|l| match l {
            HelpLine::Keys { keys, .. } => Some(theme::str_width(keys)),
            HelpLine::Section(_) => None,
        })
        .max()
        .unwrap_or(0);
    let key_w = widest.min(HELP_KEY_COL_MAX).min(content.width as usize / 2);
    let desc_w = (content.width as usize).saturating_sub(key_w + HELP_GAP);

    let mut items = Vec::with_capacity(end - offset);
    for line in &lines[offset..end] {
        items.push(match line {
            HelpLine::Section(title) => ListItem::new(Line::from(Span::styled(
                *title,
                theme::accent().add_modifier(Modifier::BOLD),
            ))),
            HelpLine::Keys { keys, desc } => {
                let shown = theme::ellipsize(keys, key_w as u16);
                let pad = key_w + HELP_GAP - theme::str_width(&shown);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{shown}{}", " ".repeat(pad)), theme::secondary()),
                    Span::styled(theme::ellipsize(desc, desc_w as u16), theme::text()),
                ]))
            }
        });
    }
    render_list(f, content, items, cursor, offset, theme::current());
    render_bar(f, bar, total, offset, visible);
}
