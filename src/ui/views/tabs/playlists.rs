use crate::ui::app::{App, PlaylistsCol};
use crate::ui::keymap::{Act, KeyCtx};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::render_scrollbar;
use crate::ui::theme;
use crate::ui::views::tables::{
    build_track_rows, len_col_width, plain_track_cells, table_frame, track_cols, track_headers,
    track_number_width, TrackCols, TRACK_LEGACY,
};
use crate::ui::views::widgets::{
    begin_list, build_range, clean, hl, indicator_text, no_matches, placeholder, render_bar,
    render_list, spans_clean, window,
};
use crate::ui::views::{draw_block, draw_vdivider};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Table, TableState};
use ratatui::Frame;

pub(in crate::ui::views) fn draw_playlists_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
        let raw = app.playlists.list.items.len();
        let total = app.playlists.list.filter.len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.playlists.list.filter, raw);
        let cursor = app.playlists.list.state.selected();
        let (offset, visible) = window(app, ScrollId::PlaylistList, cursor, total, content);
        if total == 0 && app.playlists.list.filter.is_active() {
            no_matches(f, content);
        } else {
            let text = app.playlists.list.filter.text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.playlists.list.filter.real(fi, raw);
                let p = &app.playlists.list.items[real];
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

    if app.playlists.tracks.items.is_empty() {
        let key = app
            .keymap
            .first_key(KeyCtx::Playlists, &Act::Activate)
            .unwrap_or_else(|| "Enter".into());
        let msg = if app.playlists.list.items.is_empty() {
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
    let raw = app.playlists.tracks.items.len();
    let total = app.playlists.tracks.filter.len(raw);
    let ind = indicator_text(&app.playlists.tracks.filter, raw);
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
        len_col_width(app.playlists.tracks.items.iter().filter_map(|t| t.duration_ms).max()),
        &tc,
    );
    let headers = track_headers(num_w, &tc);
    let frame = table_frame(f, pane, total, &cols_c, Some(&headers), ind);
    let cursor = app.playlists.tracks.state.selected();
    let visible = frame.rows.height as usize;
    let offset = app.scroll.update(
        ScrollId::PlaylistItems,
        cursor.unwrap_or(0),
        visible,
        total,
        app.scrolloff,
    );
    if total == 0 {
        if app.playlists.tracks.filter.is_active() {
            no_matches(f, frame.rows);
        }
        if let Some(bar) = frame.bar {
            render_scrollbar(f, bar, total, offset, visible);
        }
        return;
    }
    let needle = app.playlists.tracks.filter.text.as_str();
    let cur_key = app.current_track().map(|t| t.rating_key.clone());
    let rows = build_track_rows(
        &app.playlists.tracks.items,
        &app.playlists.tracks.filter,
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
