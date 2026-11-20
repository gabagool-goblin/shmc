//! Layout: a box per region; the progress row also holds the status, filter and `:` line.

mod bottom;
mod header;
mod queue;
mod tables;
mod tabs;
mod visualizer;
mod widgets;

pub use header::draw_tabs;
pub(crate) use header::{draw_header_block, draw_tab_row, rating_glyphs, rating_short, HeaderInfo};

use crate::ui::app::{App, Tab};
use crate::ui::theme;
use bottom::{draw_bottom, draw_rule_title, now_playing};
use header::{draw_tab_strip, header_info, tab_name};
use queue::draw_queue_tab;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use tabs::{
    draw_browser_tab, draw_discover_tab, draw_help_tab, draw_playlists_tab, draw_search_tab,
};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    theme::paint_background(f.buffer_mut(), area);
    // Leaving the Queue clears the cover's cells, so returning repaints it.
    if app.tab != Tab::Queue {
        app.art.shown_rect = None;
        app.art.hold_until = None;
        app.art.away = true;
    } else if app.art.away {
        // The cover follows after a dwell so the screen is not held behind it.
        app.art.away = false;
        app.hold_art();
    }
    let spacing = theme::spacing();
    let style = spacing.borders;
    let compact = app.header_compact;
    // Zero-height spacers would still change how the solver squeezes a small terminal.
    let gap = theme::effective_block_gap(
        area.height,
        spacing.block_gap,
        theme::fixed_block_rows(compact, style),
        theme::block_gaps(compact),
    );
    let (header_h, tabs_h, progress_h) = if style.boxed() { (4, 3, 3) } else { (2, 1, 1) };
    // Compact `none` keeps a now-playing row, so the content height is the same on every tab.
    let np_row = compact && style == theme::BorderStyle::None;
    let mut blocks = Vec::new();
    if !compact {
        blocks.push((Slot::Header, Constraint::Length(header_h)));
    }
    blocks.push((Slot::Tabs, Constraint::Length(tabs_h)));
    blocks.push((Slot::Content, Constraint::Min(6)));
    blocks.push((Slot::Bottom, Constraint::Length(progress_h + u16::from(np_row))));
    let last = blocks.len() - 1;
    let mut parts = Vec::new();
    for (i, (slot, constraint)) in blocks.into_iter().enumerate() {
        parts.push((slot, constraint));
        if i < last {
            if style == theme::BorderStyle::Lines {
                parts.push((Slot::Rule, Constraint::Length(1)));
            }
            if gap > 0 {
                parts.push((Slot::Gap, Constraint::Length(gap)));
            }
        }
    }
    let rects = Layout::vertical(parts.iter().map(|(_, c)| *c)).split(area);
    let at = |want: Slot| parts.iter().position(|(slot, _)| *slot == want).map(|i| rects[i]);
    let rules: Vec<Rect> = parts
        .iter()
        .zip(rects.iter())
        .filter(|((slot, _), _)| *slot == Slot::Rule)
        .map(|(_, r)| *r)
        .collect();
    let (tabs, content, bottom) =
        (at(Slot::Tabs).unwrap(), at(Slot::Content).unwrap(), at(Slot::Bottom).unwrap());

    if let Some(header) = at(Slot::Header) {
        draw_header_block(f, header, style, &header_info(app));
    }
    let names: Vec<&str> = Tab::ALL.iter().map(|t| tab_name(*t)).collect();
    let active = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    if compact {
        draw_tab_row(f, tabs, style, &names, active, &header_info(app));
    } else {
        draw_tab_strip(f, tabs, style, &names, active);
    }
    for rule in &rules {
        draw_rule(f, *rule);
    }

    let meta_shown = match app.tab {
        Tab::Queue => draw_queue_tab(f, app, content),
        Tab::Discover => {
            draw_discover_tab(f, app, content);
            false
        }
        Tab::Browser => {
            draw_browser_tab(f, app, content);
            false
        }
        Tab::Playlists => {
            draw_playlists_tab(f, app, content);
            false
        }
        Tab::Search => {
            draw_search_tab(f, app, content);
            false
        }
        Tab::Help => {
            draw_help_tab(f, app, content);
            false
        }
    };

    // The cover's metadata is the now-playing display where it shows.
    let show_np = compact && !meta_shown;
    let mut progress = bottom;
    if np_row {
        let row = Rect { height: bottom.height.min(1), ..bottom };
        progress = Rect { y: bottom.y + row.height, height: bottom.height - row.height, ..bottom };
        if show_np {
            let inner = inset_h(row);
            if let Some(line) = now_playing(app, inner.width) {
                f.render_widget(Paragraph::new(line), inner);
            }
        }
    }
    if show_np && style == theme::BorderStyle::Lines {
        if let Some(rule) = rules.last() {
            draw_rule_title(f, app, *rule);
        }
    }
    draw_bottom(f, app, progress, show_np && style.boxed());
}

#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Header,
    Tabs,
    Content,
    Bottom,
    Rule,
    Gap,
}

pub(crate) fn draw_rule(f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    f.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(theme::border()),
        Rect { height: 1, ..area },
    );
}

/// A frame for `style`: boxes (optionally titled) or none.
fn block_for(style: theme::BorderStyle, title: Option<Line<'static>>) -> Block<'static> {
    let block = match style {
        theme::BorderStyle::Box => Block::bordered(),
        theme::BorderStyle::Rounded => Block::bordered().border_type(BorderType::Rounded),
        theme::BorderStyle::Double => Block::bordered().border_type(BorderType::Double),
        theme::BorderStyle::Lines | theme::BorderStyle::None => return Block::default(),
    }
    .border_style(theme::border());
    match title {
        Some(title) => block.title(title),
        None => block,
    }
}

/// Draws the block chrome of `style` and returns the area for its content.
pub(crate) fn draw_block_as(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    title: Option<Line<'static>>,
) -> Rect {
    let block = block_for(style, title);
    let inner = inset_h(block.inner(area));
    f.render_widget(block, area);
    inner
}

/// [`draw_block_as`] in the player's border style.
pub(crate) fn draw_block(f: &mut Frame, area: Rect) -> Rect {
    draw_block_as(f, area, theme::spacing().borders, None)
}

/// An inner rule on `sides`; in `none` its cell stays undrawn, so the layout is the same.
fn draw_divider(f: &mut Frame, area: Rect, sides: Borders) -> Rect {
    let block = Block::default().borders(sides).border_style(theme::border());
    let inner = block.inner(area);
    if theme::spacing().borders != theme::BorderStyle::None {
        f.render_widget(block, area);
    }
    inner
}

/// `theme.inset` columns of padding inside a box, unless that leaves no column.
pub(crate) fn inset_h(r: Rect) -> Rect {
    let inset = theme::spacing().inset;
    if r.width > inset * 2 {
        Rect { x: r.x + inset, y: r.y, width: r.width - inset * 2, height: r.height }
    } else {
        r
    }
}

/// A full-height vertical divider at column `x`.
pub(crate) fn draw_vdivider(f: &mut Frame, x: u16, area: Rect) {
    if area.height == 0 || theme::spacing().borders == theme::BorderStyle::None {
        return;
    }
    let r = Rect { x, y: area.y, width: 1, height: area.height };
    f.render_widget(Block::default().borders(Borders::LEFT).border_style(theme::border()), r);
}
