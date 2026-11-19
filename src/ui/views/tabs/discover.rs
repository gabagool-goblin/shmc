use super::draw_list_label;
use crate::ui::app::{App, DiscoverCol, Preview, SimilarKind, StationsLoad};
use crate::ui::filter::ListFilter;
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::{list_split, render_scrollbar};
use crate::ui::theme;
use crate::ui::views::tables::{
    build_track_rows, len_col_width, plain_track_cells, table_frame, track_cols, track_headers,
    TrackCols, TRACK_LEGACY,
};
use crate::ui::views::widgets::{
    build_range, clean, hl, placeholder, render_bar, render_list, window,
};
use crate::ui::views::{draw_block, draw_vdivider};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Paragraph, Table, TableState};
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
    draw_stations(f, app, cols[0]);
    if app.discover.label.is_some() && app.discover.load == StationsLoad::Loaded {
        draw_station_tracks(f, app, cols[2]);
    } else {
        placeholder(f, cols[2], &right_message(app));
    }
}

fn right_message(app: &App) -> String {
    match app.discover.load {
        StationsLoad::NoRadio => return "Stations need Plex Pass radio".into(),
        StationsLoad::NotLoaded | StationsLoad::Loading => return "loading stations…".into(),
        StationsLoad::Failed => return "could not load stations".into(),
        StationsLoad::Loaded => {}
    }
    if !app.allow_writes {
        return "Starting a station needs plex.allow_writes".into();
    }
    if let Some(Preview::Failed(msg)) = app.discover.shown_preview() {
        return msg.clone();
    }
    let selected = app.discover.stations_state.selected();
    let Some(station) = selected.and_then(|i| app.discover.stations.get(i)) else {
        return "no stations found".into();
    };
    format!("loading {}…", clean(&station.title))
}

fn draw_stations(f: &mut Frame, app: &mut App, pane: Rect) {
    let label = Rect { height: 1, ..pane };
    f.render_widget(Paragraph::new(Line::from(Span::styled("Stations", theme::dim()))), label);
    let body = Rect { y: pane.y + 1, height: pane.height - 1, ..pane };
    let total = app.discover.stations.len();
    let (content, bar) = list_split(body, total, body.height as usize);
    let cursor = app.discover.stations_state.selected();
    let (offset, visible) = window(app, ScrollId::DiscoverStations, cursor, total, content);
    let items = build_range(offset, visible, total, |i, items: &mut Vec<ListItem>| {
        let title = theme::ellipsize(&clean(&app.discover.stations[i].title), content.width);
        items.push(ListItem::new(Line::from(Span::styled(title, theme::text()))));
    });
    render_list(f, content, items, cursor, offset, hl(app.discover.focus == DiscoverCol::Stations));
    render_bar(f, bar, total, offset, visible);
}

fn draw_station_tracks(f: &mut Frame, app: &mut App, pane: Rect) {
    let Some(label) = app.discover.label.clone() else { return };
    let parts = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(pane);
    let total = app.discover.tracks.len();
    draw_list_label(f, parts[0], &label, &format!(" ({})", SimilarKind::Tracks.count(total)));

    let num_w = None;
    let tc = TrackCols::plain(app.column_weights, TRACK_LEGACY);
    let longest = app.discover.tracks.iter().filter_map(|t| t.duration_ms).max();
    let cols = track_cols(num_w, len_col_width(longest), &tc);
    let headers = track_headers(num_w, &tc);
    let frame = table_frame(f, parts[1], total, &cols, Some(&headers), None);
    let cursor = app.discover.tracks_state.selected();
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
        &app.discover.tracks,
        &ListFilter::default(),
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
                "",
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
