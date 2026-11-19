use super::tables::{
    build_track_rows, len_col_width, plain_track_cells, table_frame, track_cols, track_headers,
    track_number_width, HeaderSpec, TrackCols, TRACK_LEGACY,
};
use super::widgets::{
    begin_list, build_range, clean, hl, indicator_text, no_matches, placeholder, render_bar,
    render_list, spans_clean, title_spans, window,
};
use super::{draw_block, draw_vdivider};
use crate::ui::app::{App, BrowserCol, PlaylistsCol, SimilarKind};
use crate::ui::keymap::{Act, HelpLine, KeyCtx};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::{list_split, render_scrollbar};
use crate::ui::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, ListItem, Paragraph, Row, Table, TableState};
use ratatui::Frame;

pub(super) fn draw_browser_tab(f: &mut Frame, app: &mut App, area: Rect) {
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

pub(super) fn draw_playlists_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
    let list_pane = cols[0];
    let items_pane = cols[2];
    draw_vdivider(f, cols[1].x, cols[1]);

    {
        let pane = list_pane;
        let raw = app.playlists.playlists.len();
        let total = app.pl_list_filter.len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.pl_list_filter, raw);
        let cursor = app.playlists.state.selected();
        let (offset, visible) = window(app, ScrollId::PlaylistList, cursor, total, content);
        if total == 0 && app.pl_list_filter.is_active() {
            no_matches(f, content);
        } else {
            let text = app.pl_list_filter.text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.pl_list_filter.real(fi, raw);
                let p = &app.playlists.playlists[real];
                // room for the "✦ " marker and the " (count)" suffix
                let mut used = 0u16;
                if p.smart {
                    used += 2;
                }
                let count = p.item_count.map(|n| format!(" ({n})"));
                if let Some(c) = &count {
                    used += theme::str_width(c) as u16;
                }
                let title = theme::ellipsize(&clean(&p.title), content.width.saturating_sub(used));
                let mut spans: Vec<Span> = Vec::new();
                if p.smart {
                    spans.push(Span::styled("✦ ", theme::accent()));
                }
                spans.extend(spans_clean(title, text, theme::text()));
                if let Some(c) = count {
                    spans.push(Span::styled(c, theme::dim()));
                }
                items.push(ListItem::new(Line::from(spans)));
            });
            render_list(
                f,
                content,
                items,
                cursor,
                offset,
                hl(app.playlists.focus == PlaylistsCol::List),
            );
        }
        render_bar(f, bar, total, offset, visible);
    }

    if app.playlists.items.is_empty() {
        let key = app
            .keymap
            .first_key(KeyCtx::Playlists, &Act::Activate)
            .unwrap_or_else(|| "Enter".into());
        let msg = if app.playlists.playlists.is_empty() {
            "no playlists found".to_string()
        } else if app.preview_tracks == 0 {
            format!("preview off; {key} adds this playlist")
        } else {
            format!("highlight a playlist to preview, {key} adds it")
        };
        placeholder(f, items_pane, &msg);
        return;
    }
    let pane = items_pane;
    let raw = app.playlists.items.len();
    let total = app.pl_items_filter.len(raw);
    let ind = indicator_text(&app.pl_items_filter, raw);
    // Sized from the preview cap, not the previewed length, so the columns do
    // not move when the selection changes.
    let num_w = if app.numbers_playlists {
        Some(track_number_width(app.preview_tracks.max(1)))
    } else {
        None
    };
    let tc = TrackCols::plain(app.column_weights, TRACK_LEGACY);
    let cols_c = track_cols(
        num_w,
        len_col_width(app.playlists.items.iter().filter_map(|t| t.duration_ms).max()),
        &tc,
    );
    let headers = track_headers(num_w, &tc);
    let frame = table_frame(f, pane, total, &cols_c, Some(&headers), ind);
    let cursor = app.playlists.items_state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::PlaylistItems,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    if total == 0 {
        if app.pl_items_filter.is_active() {
            no_matches(f, frame.rows);
        }
        if let Some(bar) = frame.bar {
            render_scrollbar(f, bar, total, offset, visible);
        }
        return;
    }
    let needle = app.pl_items_filter.text.as_str();
    let cur_key = app.current_track().map(|t| t.rating_key.clone());
    let rows = build_track_rows(
        &app.playlists.items,
        &app.pl_items_filter,
        offset,
        visible,
        total,
        cur_key.as_deref(),
        app.visual_range(),
        |t, real, base, playing| {
            let rating = app.rating_text(&t.rating_key, t.user_rating);
            plain_track_cells(
                t,
                real,
                base,
                playing,
                &frame.col_w,
                num_w,
                &tc,
                needle,
                rating.as_deref(),
            )
        },
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

pub(super) fn draw_search_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
    let tc = TrackCols::new(
        app.column_weights,
        [1, 0, 2],
        [Constraint::Percentage(24), Constraint::Percentage(36), Constraint::Min(12)],
    );
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

    let tc = TrackCols::plain(
        app.column_weights,
        [Constraint::Percentage(34), Constraint::Percentage(24), Constraint::Min(12)],
    );
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

/// The line above a Search list: what it is, then how many rows.
fn draw_list_label(f: &mut Frame, area: Rect, label: &str, count: &str) {
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

/// A four-digit year, which also fits the "Year" header.
const YEAR_COL_WIDTH: u16 = 4;

/// Room for the "Match" header, which also fits "100%".
const MATCH_COL_WIDTH: u16 = 5;

/// A similar list drops its Match column below this pane width.
const MATCH_MIN_WIDTH: u16 = 60;

/// The widest the Help tab's key column grows, and the gap after it.
const HELP_KEY_COL_MAX: usize = 22;

const HELP_GAP: usize = 2;

pub(super) fn draw_help_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
