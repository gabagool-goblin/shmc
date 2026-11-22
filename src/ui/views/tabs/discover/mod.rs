mod right;

use crate::ui::app::{App, DiscoverCol, DiscoverLoad, Row};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::list_split;
use crate::ui::theme;
use crate::ui::views::widgets::{
    build_range, clean, hl, placeholder, render_bar, render_list, window,
};
use crate::ui::views::{draw_block, draw_vdivider};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use ratatui::Frame;

pub(in crate::ui::views) fn draw_discover_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = draw_block(f, area);
    if inner.width < 10 || inner.height < 3 {
        return;
    }
    let cols = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Length(1),
        Constraint::Percentage(65),
    ])
    .split(inner);
    draw_vdivider(f, cols[1].x, cols[1]);
    draw_sections(f, app, cols[0]);
    if app.discover.pane.is_empty() {
        placeholder(f, cols[2], &right_message(app));
    } else {
        right::draw_pane(f, app, cols[2]);
    }
}

fn right_message(app: &App) -> String {
    let d = &app.discover;
    if d.rows.is_empty() {
        return match d.load {
            DiscoverLoad::NotLoaded | DiscoverLoad::Loading => "loading Discover…".into(),
            _ => "could not load Discover".into(),
        };
    }
    let selected = d.sections_state.selected().and_then(|i| d.rows.get(i));
    match selected {
        Some(Row::Library(list)) => d.library.message(*list),
        Some(Row::Station(i)) => {
            if !app.allow_writes {
                return "Starting a station needs plex.allow_writes".into();
            }
            if let Some(crate::ui::app::Preview::Failed(msg)) = d.shown_preview() {
                return msg.clone();
            }
            match d.stations.get(*i) {
                Some(station) => format!("loading {}…", clean(&station.title)),
                None => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn draw_sections(f: &mut Frame, app: &mut App, pane: Rect) {
    let total = app.discover.rows.len();
    let (content, bar) = list_split(pane, total, pane.height as usize);
    let cursor = app.discover.sections_state.selected();
    let (offset, visible) = window(app, ScrollId::DiscoverSections, cursor, total, content);
    let d = &app.discover;
    let items = build_range(offset, visible, total, |i, items: &mut Vec<ListItem>| {
        let line = match d.rows[i] {
            Row::Heading(text) => Line::from(Span::styled(text, theme::dim())),
            Row::Blank => Line::default(),
            Row::Shelf(n) => text_row(&d.shelves[n].title, content.width),
            Row::Station(n) => text_row(&d.stations[n].title, content.width),
            Row::Library(list) => text_row(list.title(), content.width),
        };
        items.push(ListItem::new(line));
    });
    render_list(f, content, items, cursor, offset, hl(d.focus == DiscoverCol::Sections));
    render_bar(f, bar, total, offset, visible);
}

fn text_row(title: &str, width: u16) -> Line<'static> {
    Line::from(Span::styled(theme::ellipsize(&clean(title), width), theme::text()))
}
