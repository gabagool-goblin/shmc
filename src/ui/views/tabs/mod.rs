mod browser;
mod help;
mod playlists;
mod search;

pub(super) use browser::draw_browser_tab;
pub(super) use help::draw_help_tab;
pub(super) use playlists::draw_playlists_tab;
pub(super) use search::draw_search_tab;

use crate::ui::theme;
use crate::ui::views::widgets::clean;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// The line above a Search list: what it is, then how many rows.
pub(super) fn draw_list_label(f: &mut Frame, area: Rect, label: &str, count: &str) {
    let line = Line::from(vec![
        Span::styled("▸ ", theme::accent()),
        Span::styled(
            theme::ellipsize(&clean(label), area.width.saturating_sub(14)),
            theme::secondary(),
        ),
        Span::styled(count.to_string(), theme::dim()),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
