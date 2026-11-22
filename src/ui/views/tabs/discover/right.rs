use super::super::draw_list_label;
use crate::plex::discover::LibraryList;
use crate::plex::discover::ShelfItems;
use crate::ui::app::{App, Pane, TracksSource};
use crate::ui::filter::ListFilter;
use crate::ui::period::PeriodKind;
use crate::ui::playlist_load::group;
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::render_scrollbar;
use crate::ui::theme;
use crate::ui::views::tables::{
    build_track_rows, len_col_width, plain_track_cells, table_frame, track_cols, track_headers,
    HeaderSpec, TrackCols, TRACK_LEGACY,
};
use crate::ui::views::widgets::cell;
use crate::ui::when::played_at;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

/// Wide enough for "2025-11-20", the longest Played text.
const PLAYED_COL: u16 = 10;

const PLAYED_MIN_PANE: u16 = 44;

pub(super) fn draw_pane(f: &mut Frame, app: &mut App, pane: Rect) {
    let period = app.discover.period_line();
    let parts = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(period.is_some() as u16),
        Constraint::Min(1),
    ])
    .split(pane);
    let (label, count) = match &app.discover.pane {
        Pane::Tracks(p) => (p.label(), counted(p.total, "track")),
        Pane::Items(p) => (p.label(), counted(p.items.len(), noun(&p.items))),
        Pane::Empty => return,
    };
    draw_list_label(f, parts[0], &label, &format!(" ({count})"));
    if let Some(kind) = period {
        draw_period_line(f, parts[1], kind);
    }
    match app.discover.pane {
        Pane::Tracks(_) => draw_tracks(f, app, parts[2]),
        _ => draw_items(f, app, parts[2]),
    }
}

fn draw_period_line(f: &mut Frame, area: Rect, active: PeriodKind) {
    let mut spans = vec![Span::raw("  ")];
    for kind in PeriodKind::ALL {
        let style = if kind == active { theme::accent() } else { theme::dim() };
        spans.push(Span::styled(kind.name(), style));
        spans.push(Span::raw("  "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn noun(items: &ShelfItems) -> &'static str {
    match items {
        ShelfItems::Albums(_) => "album",
        ShelfItems::Artists(_) => "artist",
        ShelfItems::Playlists(_) => "playlist",
    }
}

fn counted(n: usize, noun: &str) -> String {
    format!("{} {noun}{}", group(n), if n == 1 { "" } else { "s" })
}

fn draw_tracks(f: &mut Frame, app: &mut App, area: Rect) {
    let Pane::Tracks(pane) = &app.discover.pane else { return };
    let total = pane.tracks.len();
    let num_w = None;
    let tc = TrackCols::plain(app.column_weights, TRACK_LEGACY);
    let longest = pane.tracks.iter().filter_map(|t| t.duration_ms).max();
    let widest_plays = pane.tracks.iter().filter_map(|t| t.plays).max().unwrap_or(0);
    let extra = match pane.source {
        TracksSource::Library(LibraryList::RecentlyPlayed) => Some(("Played", PLAYED_COL)),
        TracksSource::Library(LibraryList::TopTracks) => Some(("Plays", plays_width(widest_plays))),
        _ => None,
    }
    .filter(|_| area.width >= PLAYED_MIN_PANE);
    let mut cols = track_cols(num_w, len_col_width(longest), &tc);
    let mut headers = track_headers(num_w, &tc);
    if let Some((title, width)) = extra {
        cols.insert(cols.len() - 1, Constraint::Length(width));
        headers.insert(headers.len() - 1, HeaderSpec::right(title));
    }
    let frame = table_frame(f, area, total, &cols, Some(&headers), None);
    let cursor = pane.state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::DiscoverTracks,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    let cur_key = app.current_track().map(|t| t.rating_key.clone());
    let rows = build_track_rows(
        &pane.tracks,
        &ListFilter::default(),
        offset,
        visible,
        total,
        cur_key.as_deref(),
        app.visual_range(),
        |t, real, base, playing| {
            let rating = app.rating_text(&t.rating_key, t.user_rating);
            let mut cells = plain_track_cells(
                t,
                real,
                base,
                playing,
                &frame.col_w,
                num_w,
                &tc,
                "",
                rating.as_deref(),
            );
            if extra.is_some() {
                let text = match pane.source {
                    TracksSource::Library(LibraryList::TopTracks) => count_text(t.plays),
                    _ => t.last_viewed_at.map(played_at).unwrap_or_default(),
                };
                let style = if playing { base } else { theme::dim() };
                let cell = Cell::from(ratatui::text::Line::from(text).right_aligned()).style(style);
                cells.insert(cells.len() - 1, cell);
            }
            cells
        },
    );
    render_rows(f, frame.rows, &frame.col_w, rows, cursor, offset);
    if let Some(bar) = frame.bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

fn plays_width(widest: u64) -> u16 {
    (group(widest as usize).chars().count() as u16).max(5)
}

fn count_text(plays: Option<u64>) -> String {
    plays.map(|n| group(n as usize)).unwrap_or_default()
}

fn draw_items(f: &mut Frame, app: &mut App, area: Rect) {
    let Pane::Items(pane) = &app.discover.pane else { return };
    let total = pane.items.len();
    let (cols, headers) = item_columns(&pane.items, pane.plays);
    let frame = table_frame(f, area, total, &cols, Some(&headers), None);
    let cursor = pane.state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::DiscoverItems,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    let end = (offset + visible).min(total);
    let w = &frame.col_w;
    let text = theme::text();
    let plays_cell = |plays: Option<u64>| {
        Cell::from(Line::from(count_text(plays)).right_aligned()).style(theme::dim())
    };
    let rows: Vec<Row> = (offset..end)
        .map(|i| match &pane.items {
            ShelfItems::Albums(v) => {
                let a = &v[i];
                let year = a.year.map(|y| y.to_string()).unwrap_or_default();
                let mut cells = vec![
                    cell(&a.title, w[0], "", text),
                    cell(&a.artist, w[1], "", theme::dim()),
                    Cell::from(ratatui::text::Line::from(year).right_aligned()).style(theme::dim()),
                ];
                if pane.plays {
                    cells.push(plays_cell(a.plays));
                }
                Row::new(cells)
            }
            ShelfItems::Artists(v) => {
                let mut cells = vec![cell(&v[i].title, w[0], "", text)];
                if pane.plays {
                    cells.push(plays_cell(v[i].plays));
                }
                Row::new(cells)
            }
            ShelfItems::Playlists(v) => {
                let count = v[i].item_count.map(|n| n.to_string()).unwrap_or_default();
                Row::new(vec![
                    cell(&v[i].title, w[0], "", text),
                    Cell::from(ratatui::text::Line::from(count).right_aligned())
                        .style(theme::dim()),
                ])
            }
        })
        .collect();
    render_rows(f, frame.rows, w, rows, cursor, offset);
    if let Some(bar) = frame.bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

fn item_columns(items: &ShelfItems, plays: bool) -> (Vec<Constraint>, Vec<HeaderSpec>) {
    let (mut cols, mut headers) = match items {
        ShelfItems::Albums(_) => (
            vec![Constraint::Fill(3), Constraint::Fill(2), Constraint::Length(4)],
            vec![HeaderSpec::left("Album"), HeaderSpec::left("Artist"), HeaderSpec::right("Year")],
        ),
        ShelfItems::Artists(_) => (vec![Constraint::Fill(1)], vec![HeaderSpec::left("Artist")]),
        ShelfItems::Playlists(v) => {
            let widest = v.iter().filter_map(|p| p.item_count).max().unwrap_or(0).to_string().len();
            (
                vec![Constraint::Fill(1), Constraint::Length(widest.max(6) as u16)],
                vec![HeaderSpec::left("Playlist"), HeaderSpec::right("Tracks")],
            )
        }
    };
    if plays {
        let widest = match items {
            ShelfItems::Albums(v) => v.iter().filter_map(|a| a.plays).max(),
            ShelfItems::Artists(v) => v.iter().filter_map(|a| a.plays).max(),
            _ => None,
        };
        cols.push(Constraint::Length(plays_width(widest.unwrap_or(0))));
        headers.push(HeaderSpec::right("Plays"));
    }
    (cols, headers)
}

fn render_rows(
    f: &mut Frame,
    area: Rect,
    col_w: &[u16],
    rows: Vec<Row>,
    cursor: Option<usize>,
    offset: usize,
) {
    let widths: Vec<Constraint> = col_w.iter().map(|w| Constraint::Length(*w)).collect();
    let mut state = TableState::default();
    if let Some(c) = cursor {
        state.select(Some(c.saturating_sub(offset).min(rows.len().saturating_sub(1))));
    }
    let table = Table::new(rows, widths).column_spacing(1).row_highlight_style(theme::current());
    f.render_stateful_widget(table, area, &mut state);
}
