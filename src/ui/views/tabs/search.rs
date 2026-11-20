use super::draw_list_label;
use crate::ui::app::{App, SimilarKind};
use crate::ui::keymap::{Act, KeyCtx};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::render_scrollbar;
use crate::ui::theme;
use crate::ui::views::draw_block;
use crate::ui::views::tables::{
    len_col_width, table_frame, HeaderSpec, TrackCols, SEARCH_HIT_COLS, SEARCH_TRACK_COLS,
};
use crate::ui::views::widgets::{clean, placeholder, title_spans};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table, TableState};
use ratatui::Frame;

pub(in crate::ui::views) fn draw_search_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = draw_block(f, area);
    if inner.width < 12 || inner.height < 3 {
        return;
    }
    if app.search.detail.is_some() {
        draw_search_detail(f, app, inner);
    } else {
        draw_search_results(f, app, inner);
    }
}

fn draw_search_results(f: &mut Frame, app: &mut App, inner: Rect) {
    let total = app.search.hits.len();
    if total == 0 {
        let key = app
            .keymap
            .first_key(KeyCtx::Search, &Act::StartSearch)
            .unwrap_or_else(|| ":search".into());
        placeholder(f, inner, &format!("press {key} to search"));
        return;
    }
    if let Some(s) = app.search.similar.as_ref() {
        let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
        draw_list_label(f, parts[0], &s.label, &format!(" ({})", s.kind.count(total)));
        draw_similar_hits(f, app, parts[1]);
        return;
    }
    let tc = TrackCols::new(app.column_weights, [1, 0, 2], SEARCH_HIT_COLS);
    let mut cols = tc.constraints();
    cols.push(Constraint::Length(len_col_width(
        app.search.hits.iter().filter_map(|h| h.duration_ms).max(),
    )));
    let mut headers = tc.headers(["Type / Artist", "Title", "Album"]);
    headers.push(HeaderSpec::right("Len"));
    let frame = table_frame(f, inner, total, &cols, Some(&headers), None);
    let cursor = app.search.state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::SearchResults,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    let end = (offset + visible).min(total);
    let cur_key = app.current_track().map(|t| t.rating_key.clone());

    let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
    for i in offset..end {
        let Some(hit) = app.search.hits.get(i) else { continue };
        let playing = cur_key.as_deref() == Some(hit.rating_key.as_str());
        let base =
            if playing { theme::accent().add_modifier(Modifier::BOLD) } else { theme::text() };
        let (prefix, prefix_style) = match hit.kind {
            crate::plex::HitKind::Track => ("♪ ", theme::accent()),
            crate::plex::HitKind::Album => ("[Album] ", theme::dim()),
            crate::plex::HitKind::Artist => ("[Artist] ", theme::dim()),
        };
        let pw = theme::str_width(prefix) as u16;
        let artist = clean(&hit.artist);
        let mut cells: Vec<Cell> = Vec::with_capacity(4);
        // table order: Artist with its type marker, Title, Album
        if let Some(slot) = tc.slot(0) {
            let mut spans = vec![Span::styled(prefix, prefix_style)];
            spans.push(Span::styled(
                theme::ellipsize(&artist, frame.col_w[slot].saturating_sub(pw)),
                base,
            ));
            cells.push(Cell::from(Line::from(spans)));
        }
        let rating = (hit.kind == crate::plex::HitKind::Track)
            .then(|| app.rating_text(&hit.rating_key, hit.user_rating))
            .flatten();
        let dim = if playing { base } else { theme::dim() };
        if let Some(slot) = tc.slot(1) {
            let spans =
                title_spans(&hit.title, rating.as_deref(), frame.col_w[slot], "", base, dim);
            cells.push(Cell::from(Line::from(spans)).style(base));
        }
        if let Some(slot) = tc.slot(2) {
            let album = theme::ellipsize(&clean(&hit.subtitle), frame.col_w[slot]);
            cells.push(Cell::from(Line::from(album)).style(base));
        }
        let dur =
            hit.duration_ms.map(|ms| theme::format_time(ms as f64 / 1000.0)).unwrap_or_default();
        cells.push(Cell::from(Line::from(dur).right_aligned()).style(theme::dim()));
        rows.push(Row::new(cells));
    }

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

/// Every row of a similar albums or artists list is the same kind, so the list
/// has columns of its own instead of the mixed results' ones.
fn draw_similar_hits(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(s) = app.search.similar.as_ref() else { return };
    let albums = s.kind == SimilarKind::Albums;
    let wide = area.width >= MATCH_MIN_WIDTH;
    let total = app.search.hits.len();
    let mut cols = Vec::new();
    let mut headers = Vec::new();
    if albums {
        cols.extend([Constraint::Percentage(40), Constraint::Min(12)]);
        headers.extend([HeaderSpec::left("Artist"), HeaderSpec::left("Album")]);
        if wide {
            cols.push(Constraint::Length(YEAR_COL_WIDTH));
            headers.push(HeaderSpec::left("Year"));
        }
    } else {
        cols.push(Constraint::Min(12));
        headers.push(HeaderSpec::left("Artist"));
    }
    if wide {
        cols.push(Constraint::Length(MATCH_COL_WIDTH));
        headers.push(HeaderSpec::right("Match"));
        if albums {
            let longest = app.search.hits.iter().filter_map(|h| h.duration_ms).max();
            cols.push(Constraint::Length(len_col_width(longest)));
            headers.push(HeaderSpec::right("Len"));
        }
    }
    let frame = table_frame(f, area, total, &cols, Some(&headers), None);
    let cursor = app.search.state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::SearchResults,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    let Some(s) = app.search.similar.as_ref() else { return };
    let end = (offset + visible).min(total);
    let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
    for i in offset..end {
        let Some(hit) = app.search.hits.get(i) else { continue };
        let w = |slot: usize| frame.col_w.get(slot).copied().unwrap_or(0);
        let mut cells = vec![Cell::from(theme::ellipsize(&clean(&hit.artist), w(0)))];
        if albums {
            cells.push(Cell::from(theme::ellipsize(&clean(&hit.title), w(1))));
            if wide {
                cells.push(Cell::from(hit.subtitle.clone()).style(theme::dim()));
            }
        }
        if wide {
            let pct = s.matches.get(i).copied().flatten();
            let text = pct.map(|p| format!("{p}%")).unwrap_or_default();
            cells.push(Cell::from(Line::from(text).right_aligned()).style(theme::dim()));
            if albums {
                let dur = hit
                    .duration_ms
                    .map(|ms| theme::format_time(ms as f64 / 1000.0))
                    .unwrap_or_default();
                cells.push(Cell::from(Line::from(dur).right_aligned()).style(theme::dim()));
            }
        }
        rows.push(Row::new(cells).style(theme::text()));
    }
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

fn draw_search_detail(f: &mut Frame, app: &mut App, inner: Rect) {
    let parts = Layout::vertical([
        Constraint::Length(1), // album label
        Constraint::Min(1),
    ])
    .split(inner);

    let (label, total, cursor) = {
        let d = app.search.detail.as_ref().expect("checked by caller");
        (d.label.clone(), d.tracks.len(), d.state.selected())
    };
    draw_list_label(f, parts[0], &label, &format!(" ({})", SimilarKind::Tracks.count(total)));

    let tc = TrackCols::plain(app.column_weights, SEARCH_TRACK_COLS);
    // Match gives way first on a narrow pane, so Len and the text keep their room.
    let show_match = inner.width >= MATCH_MIN_WIDTH
        && app.search.detail.as_ref().is_some_and(|d| d.matches.is_some());
    let mut cols = tc.constraints();
    if show_match {
        cols.push(Constraint::Length(MATCH_COL_WIDTH));
    }
    cols.push(Constraint::Length(len_col_width(
        app.search
            .detail
            .as_ref()
            .and_then(|d| d.tracks.iter().filter_map(|t| t.duration_ms).max()),
    )));
    let mut headers = tc.headers(["Title", "Artist", "Album"]);
    if show_match {
        headers.push(HeaderSpec::right("Match"));
    }
    headers.push(HeaderSpec::right("Len"));
    let frame = table_frame(f, parts[1], total, &cols, Some(&headers), None);
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::SearchDetail,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );

    let selected = app.visual_range();
    let Some(d) = app.search.detail.as_ref() else { return };
    let end = (offset + visible).min(total);
    let cur_key = app.current_track().map(|t| t.rating_key.clone());
    let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
    for i in offset..end {
        let Some(t) = d.tracks.get(i) else { continue };
        let playing = cur_key.as_deref() == Some(t.rating_key.as_str());
        let base =
            if playing { theme::accent().add_modifier(Modifier::BOLD) } else { theme::text() };
        let dur =
            t.duration_ms.map(|ms| theme::format_time(ms as f64 / 1000.0)).unwrap_or_default();
        let mut cells: Vec<Cell> = Vec::with_capacity(4);
        let rating = app.rating_text(&t.rating_key, t.user_rating);
        let dim = if playing { base } else { theme::dim() };
        for (col, text) in [&t.title, &t.artist, &t.album].into_iter().enumerate() {
            if let Some(slot) = tc.slot(col) {
                if col == 0 {
                    let w = frame.col_w[slot];
                    let spans = title_spans(text, rating.as_deref(), w, "", base, dim);
                    cells.push(Cell::from(Line::from(spans)).style(base));
                    continue;
                }
                cells.push(
                    Cell::from(Line::from(theme::ellipsize(&clean(text), frame.col_w[slot])))
                        .style(base),
                );
            }
        }
        if show_match {
            let pct = d.matches.as_ref().and_then(|m| m.get(i).copied().flatten());
            let text = pct.map(|p| format!("{p}%")).unwrap_or_default();
            cells.push(Cell::from(Line::from(text).right_aligned()).style(theme::dim()));
        }
        cells.push(Cell::from(Line::from(dur).right_aligned()).style(theme::dim()));
        let row = Row::new(cells);
        rows.push(match selected {
            Some((lo, hi)) if (lo..=hi).contains(&i) => row.style(theme::current_unfocused()),
            _ => row,
        });
    }
    let constraints: Vec<Constraint> = frame.col_w.iter().map(|w| Constraint::Length(*w)).collect();
    let mut state = TableState::default();
    if let Some(c) = cursor {
        state.select(Some(c.saturating_sub(offset).min(rows.len().saturating_sub(1))));
    }
    let table =
        Table::new(rows, constraints).column_spacing(1).row_highlight_style(theme::current());
    f.render_stateful_widget(table, frame.rows, &mut state);
    if let Some(bar) = frame.bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

/// A four-digit year, which also fits the "Year" header.
const YEAR_COL_WIDTH: u16 = 4;

/// Room for the "Match" header, which also fits "100%".
const MATCH_COL_WIDTH: u16 = 5;

/// A similar list drops its Match column below this pane width.
const MATCH_MIN_WIDTH: u16 = 60;
