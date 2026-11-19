use crate::ui::app::{App, BrowserCol};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::render_scrollbar;
use crate::ui::theme;
use crate::ui::views::tables::{len_col_width, table_frame, track_number_width};
use crate::ui::views::widgets::{
    begin_list, build_range, clean, hl, indicator_text, no_matches, render_bar, render_list,
    spans_clean, title_spans, window,
};
use crate::ui::views::{draw_block, draw_vdivider};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, ListItem, Row, Table, TableState};
use ratatui::Frame;

pub(in crate::ui::views) fn draw_browser_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let inner = draw_block(f, area);
    if inner.width < 10 || inner.height < 3 {
        return;
    }
    // Explicit divider columns, so a highlighted row cannot overwrite them.
    let cols = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Length(1),
        Constraint::Percentage(38),
        Constraint::Length(1),
        Constraint::Percentage(42),
    ])
    .split(inner);
    let artists_pane = cols[0];
    let albums_pane = cols[2];
    let tracks_pane = cols[4];

    draw_vdivider(f, cols[1].x, cols[1]);
    draw_vdivider(f, cols[3].x, cols[3]);

    let focus = app.browser.focus;

    {
        let pane = artists_pane;
        let raw = app.browser.artists.items.len();
        let total = app.browser.artists.filter.len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.browser.artists.filter, raw);
        let cursor = app.browser.artists.state.selected();
        let (offset, visible) = window(app, ScrollId::BrowserArtists, cursor, total, content);
        if total == 0 && app.browser.artists.filter.is_active() {
            no_matches(f, content);
        } else {
            let text = app.browser.artists.filter.text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.browser.artists.filter.real(fi, raw);
                let title =
                    theme::ellipsize(&clean(&app.browser.artists.items[real].title), content.width);
                items.push(ListItem::new(Line::from(spans_clean(title, text, theme::text()))));
            });
            render_list(f, content, items, cursor, offset, hl(focus == BrowserCol::Artists));
        }
        render_bar(f, bar, total, offset, visible);
    }

    // "Title (2019)", with " - Artist" only when it is not the browsed artist.
    // Too narrow: the title shortens first, then the artist goes, never the year.
    {
        let pane = albums_pane;
        let raw = app.browser.albums.items.len();
        let total = app.browser.albums.filter.len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.browser.albums.filter, raw);
        let cursor = app.browser.albums.state.selected();
        let (offset, visible) = window(app, ScrollId::BrowserAlbums, cursor, total, content);
        // unknown means no artist suffix
        let browsed = app.browser.albums_for.as_deref().and_then(|key| {
            app.browser.artists.items.iter().find(|a| a.rating_key == key).map(|a| clean(&a.title))
        });
        if total == 0 && app.browser.albums.filter.is_active() {
            no_matches(f, content);
        } else {
            let text = app.browser.albums.filter.text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.browser.albums.filter.real(fi, raw);
                let a = &app.browser.albums.items[real];
                let artist = clean(&a.artist);
                let other =
                    !artist.is_empty() && browsed.as_deref().is_some_and(|b| b != artist.as_str());
                let year = a.year.map(|y| format!(" ({y})")).unwrap_or_default();
                let title = clean(&a.title);
                let width = content.width as usize;
                let year_w = theme::str_width(&year);
                let suffix_w = if other { theme::str_width(&artist) + 3 } else { 0 };
                // the title keeps at least this much before the artist is dropped
                let keep = theme::str_width(&title).min(8);
                let show_artist = other && width >= keep + year_w + suffix_w;
                let room = width.saturating_sub(year_w + if show_artist { suffix_w } else { 0 });
                let title = theme::ellipsize(&title, room as u16);
                let mut spans = spans_clean(title, text, theme::text());
                if !year.is_empty() {
                    spans.push(Span::styled(year, theme::dim()));
                }
                if show_artist {
                    spans.push(Span::styled(" - ", theme::text()));
                    spans.push(Span::styled(artist, theme::dim()));
                }
                items.push(ListItem::new(Line::from(spans)));
            });
            render_list(f, content, items, cursor, offset, hl(focus == BrowserCol::Albums));
        }
        render_bar(f, bar, total, offset, visible);
    }

    {
        let pane = tracks_pane;
        let raw = app.browser.tracks.items.len();
        let total = app.browser.tracks.filter.len(raw);
        let ind = indicator_text(&app.browser.tracks.filter, raw);
        // The printed number is a position, so the loaded count bounds it.
        let num_w = track_number_width(raw);
        let cols_c = [
            Constraint::Length(num_w),
            Constraint::Min(10),
            Constraint::Length(len_col_width(
                app.browser.tracks.items.iter().filter_map(|t| t.duration_ms).max(),
            )),
        ];
        let frame = table_frame(f, pane, total, &cols_c, None, ind);
        let cursor = app.browser.tracks.state.selected();
        let visible = frame.rows.height as usize;
        let offset = app.scroll.update(
            ScrollId::BrowserTracks,
            cursor.unwrap_or(0),
            visible,
            total,
            app.scrolloff,
        );
        if total == 0 {
            if app.browser.tracks.filter.is_active() {
                no_matches(f, frame.rows);
            }
        } else {
            let needle = app.browser.tracks.filter.text.as_str();
            let parent_artist = parent_artist(app);
            let cur_key = app.current_track().map(|t| t.rating_key.clone());
            let end = (offset + visible).min(total);
            let selected = app.visual_range();
            let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
            for fi in offset..end {
                let real = app.browser.tracks.filter.real(fi, raw);
                let t = &app.browser.tracks.items[real];
                let playing = cur_key.as_deref() == Some(t.rating_key.as_str());
                let base = if playing {
                    theme::accent().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                };
                let dim = if playing { base } else { theme::dim() };
                let rating = app.rating_text(&t.rating_key, t.user_rating);
                let mut spans =
                    title_spans(&t.title, rating.as_deref(), frame.col_w[1], needle, base, dim);
                let artist = clean(&t.artist);
                if !artist.is_empty() && Some(artist.as_str()) != parent_artist.as_deref() {
                    spans.push(Span::styled(" - ", theme::text()));
                    spans.push(Span::styled(artist, dim));
                }
                let dur = t
                    .duration_ms
                    .map(|ms| theme::format_time(ms as f64 / 1000.0))
                    .unwrap_or_default();
                let row = Row::new(vec![
                    Cell::from(Line::from(Span::styled(
                        format!("{:>w$}", real + 1, w = frame.col_w[0] as usize),
                        dim,
                    ))),
                    Cell::from(Line::from(spans)),
                    Cell::from(Line::from(dur).right_aligned()).style(dim),
                ]);
                rows.push(match selected {
                    Some((lo, hi)) if (lo..=hi).contains(&fi) => {
                        row.style(theme::current_unfocused())
                    }
                    _ => row,
                });
            }
            let col_w: Vec<Constraint> =
                frame.col_w.iter().map(|w| Constraint::Length(*w)).collect();
            let mut state = TableState::default();
            if let Some(c) = cursor {
                state.select(Some(c.saturating_sub(offset).min(rows.len().saturating_sub(1))));
            }
            let table =
                Table::new(rows, col_w).column_spacing(1).row_highlight_style(theme::current());
            f.render_stateful_widget(table, frame.rows, &mut state);
        }
        if let Some(bar) = frame.bar {
            render_scrollbar(f, bar, total, offset, visible);
        }
    }
}

/// The cleaned artist of the album loaded in the tracks pane.
fn parent_artist(app: &App) -> Option<String> {
    let key = app.browser.tracks_for.as_deref()?;
    app.browser.albums.items.iter().find(|a| a.rating_key == key).map(|a| clean(&a.artist))
}
