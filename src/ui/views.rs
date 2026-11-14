//! Layout: a box per region (header, tabs, main, progress row). The progress
//! row also holds the status, the `/` filter and the `:` line.

use crate::config::{QueueColumn, VolumeDisplay};
use crate::ui::app::{App, BrowserCol, PlaylistsCol, Tab};
use crate::ui::filter::{self, ListFilter};
use crate::ui::icons;
use crate::ui::keymap::{Act, HelpLine, KeyCtx};
use crate::ui::scroll::ScrollId;
use crate::ui::scrollbar::{list_split, render_scrollbar};
use crate::ui::theme;
use image::DynamicImage;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use ratatui::Frame;
use std::time::Instant;

/// The volume meter's cells: always eight, whatever its glyphs.
pub(crate) const METER_CELLS: usize = 8;

/// Meter cells to light for a volume percentage.
pub(crate) fn volume_fill(volume: f64) -> usize {
    let cells = METER_CELLS as f64;
    ((volume / 100.0) * cells).round().clamp(0.0, cells) as usize
}

/// The meter as spans, `filled` cells lit. A ramp theme has cell `i` as its
/// `i`th glyph lit or not; a pair has one glyph for lit and one for unlit.
pub(crate) fn volume_meter(filled: usize) -> Vec<Span<'static>> {
    let meter = theme::symbols().volume_meter;
    (0..METER_CELLS)
        .map(|i| {
            let lit = i < filled;
            let glyph = match meter {
                theme::Meter::Ramp(ramp) => ramp[i],
                theme::Meter::Pair(on, off) => {
                    if lit {
                        on
                    } else {
                        off
                    }
                }
            };
            Span::styled(glyph.to_string(), if lit { theme::accent() } else { theme::dim() })
        })
        .collect()
}

/// The header's volume: the meter, the percentage or both, as `display` says.
/// The config screen's preview draws through here too.
pub(crate) fn volume_spans(volume: f64, display: VolumeDisplay) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled("Volume: ", theme::accent())];
    let pct = volume.round() as i32;
    if display == VolumeDisplay::Percent {
        spans.push(Span::styled(format!("{pct}%"), theme::accent().add_modifier(Modifier::BOLD)));
        return spans;
    }
    spans.extend(volume_meter(volume_fill(volume)));
    if display == VolumeDisplay::Meter {
        return spans;
    }
    // Fixed width, or "100%" shifts the right-aligned line by a cell.
    spans.push(Span::styled(format!(" {pct:>3}%"), theme::accent().add_modifier(Modifier::BOLD)));
    spans
}

/// The mode flags as words, widest spelling first. Shuffle and Single collide
/// on their first letter, so the letters are R Z S C (Z is the shuffle key).
pub(crate) const MODE_SPELLINGS: [[&str; 4]; 3] = [
    ["Repeat", "Shuffle", "Single", "Consume"],
    ["Rep", "Shu", "Sin", "Con"],
    ["R", "Z", "S", "C"],
];

/// The four words, separators and a gap before the state on the left.
pub(crate) const MODE_WORDS_W: u16 = 35;

/// Width of both side columns, shared so the title stays centred. Fits
/// "0:42 / 4:17 (FLAC 44.1k/16)"; the flags shorten before it passes a third.
pub(crate) fn header_side_width(inner_w: u16, words: bool) -> u16 {
    let third = inner_w / 3;
    let pct = (inner_w as u32 * 15 / 100) as u16;
    let side = pct.max(27);
    if words && side.max(MODE_WORDS_W) <= third {
        return side.max(MODE_WORDS_W);
    }
    side.min(third)
}

/// The icons, or in words mode the widest spelling that fits `room`.
pub(crate) fn mode_flag_labels(words: bool, ascii: bool, room: usize) -> [&'static str; 4] {
    if !words {
        let s = icons::status(ascii);
        return [s.repeat, s.random, s.single, s.consume];
    }
    let width = |l: &[&str; 4]| l.iter().map(|w| w.chars().count()).sum::<usize>() + 3;
    MODE_SPELLINGS.into_iter().find(|l| width(l) <= room).unwrap_or(MODE_SPELLINGS[2])
}

/// The four flags as one row: on is accent and bold, off is dim.
pub(crate) fn mode_flag_spans(labels: [&'static str; 4], on: [bool; 4]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            *label,
            if on[i] { theme::accent().add_modifier(Modifier::BOLD) } else { theme::dim() },
        ));
    }
    spans
}

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
    // Without a gap, no spacer rows at all: zero-height ones would still change
    // how the solver squeezes a terminal below the minimum.
    let gap = theme::effective_block_gap(
        area.height,
        spacing.block_gap,
        theme::fixed_block_rows(compact, style),
        theme::block_gaps(compact),
    );
    let (header_h, tabs_h, progress_h) = if style.boxed() { (4, 3, 3) } else { (2, 1, 1) };
    // Compact in `none` always keeps a row above the progress row for the
    // now-playing line, so the content does not change height between tabs.
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

/// `Title - Artist`, ellipsized to `width`: the title bold, the artist secondary.
fn now_playing(app: &App, width: u16) -> Option<Line<'static>> {
    let t = app.current_track()?;
    let (title, artist) = (clean(&t.title), clean(&t.artist));
    let whole = theme::ellipsize(&format!("{title} - {artist}"), width);
    let (n_title, n_sep) = (title.chars().count(), 3);
    let part =
        |from: usize, to: usize| whole.chars().skip(from).take(to - from).collect::<String>();
    let all = whole.chars().count();
    Some(Line::from(vec![
        Span::styled(part(0, n_title.min(all)), theme::text().add_modifier(Modifier::BOLD)),
        Span::styled(part(n_title.min(all), (n_title + n_sep).min(all)), theme::text()),
        Span::styled(part((n_title + n_sep).min(all), all), theme::secondary()),
    ]))
}

/// The title sits on the rule above the progress row, padded by a space each side.
fn draw_rule_title(f: &mut Frame, app: &App, rule: Rect) {
    let pad = theme::spacing().inset + 1;
    let room = rule.width.saturating_sub(pad * 2 + 2);
    let Some(line) = now_playing(app, room) else { return };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(line.spans);
    spans.push(Span::raw(" "));
    let width = spans.iter().map(|s| theme::str_width(&s.content)).sum::<usize>() as u16;
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { x: rule.x + pad, y: rule.y, width, height: 1 }.intersection(rule),
    );
}

/// Audio info as (long, compact). Lossy tracks show the bitrate and their
/// compact form drops the rate: `44.1k/320k` would read as rate/depth.
fn format_audio(audio: &crate::plex::AudioInfo) -> (String, String) {
    let codec = audio.codec.as_deref().map(|c| c.to_ascii_uppercase());
    let rate = audio.sample_rate_hz.map(format_rate);
    let same = |s: String| (s.clone(), s);
    let with = |c: &Option<String>, rest: String| match c {
        Some(c) => format!("{c} {rest}"),
        None => rest,
    };
    match (&codec, rate, audio.bit_depth, audio.bitrate_kbps) {
        (_, Some(r), Some(d), _) => {
            (with(&codec, format!("{r}Hz/{d}bit")), with(&codec, format!("{r}/{d}")))
        }
        (Some(_), None, Some(d), _) => same(with(&codec, format!("{d}bit"))),
        (_, Some(r), None, Some(b)) => {
            (with(&codec, format!("{b}kbps/{r}Hz")), with(&codec, format!("{b}k")))
        }
        (_, None, None, Some(b)) => {
            (with(&codec, format!("{b}kbps")), with(&codec, format!("{b}k")))
        }
        (_, Some(r), None, None) => (with(&codec, format!("{r}Hz")), with(&codec, r)),
        (Some(c), None, None, None) => same(c.clone()),
        _ => (String::new(), String::new()),
    }
}

/// `0:42 / 4:17` plus the longest audio form that fits `width`; the time alone
/// is ellipsized if it does not fit.
fn header_times(times: &str, long: &str, compact: &str, width: u16) -> String {
    let forms = [long, compact].map(|info| {
        if info.is_empty() {
            String::new()
        } else {
            format!("{times} ({info})")
        }
    });
    for f in forms.iter().filter(|f| !f.is_empty()) {
        if theme::str_width(f) <= width as usize {
            return f.clone();
        }
    }
    theme::ellipsize(times, width)
}

fn format_rate(hz: u32) -> String {
    let khz = hz as f64 / 1000.0;
    let s = format!("{khz:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    format!("{s}k")
}

/// What the header shows, as plain data: the player fills it from the app and
/// the config screen's preview from placeholders, and both draw through here.
pub(crate) struct HeaderInfo {
    pub stopped: bool,
    pub paused: bool,
    /// Title, artist, album.
    pub track: Option<(String, String, String)>,
    pub times: String,
    /// Long and compact audio forms.
    pub audio: (String, String),
    pub volume: f64,
    pub volume_display: VolumeDisplay,
    pub mode_words: bool,
    pub ascii: bool,
    /// Repeat, shuffle, single, consume.
    pub flags: [bool; 4],
    pub sleep: Option<String>,
}

fn header_info(app: &App) -> HeaderInfo {
    let (mut long, mut compact) = format_audio(&app.audio);
    if long.is_empty() {
        // No Plex stream details: fall back to mpv's live bitrate.
        let kbps = (app.bitrate_bps / 1000.0).round();
        if kbps >= 1.0 {
            long = format!("{} kbps", kbps as i64);
            compact = long.clone();
        }
    }
    HeaderInfo {
        stopped: app.stopped,
        paused: app.paused,
        track: app.current_track().map(|t| (clean(&t.title), clean(&t.artist), clean(&t.album))),
        times: format!(
            "{} / {}",
            theme::format_time(app.position),
            theme::format_time(app.duration)
        ),
        audio: (long, compact),
        volume: app.volume,
        volume_display: app.volume_display,
        mode_words: app.mode_words,
        ascii: app.icons_ascii,
        flags: [app.loop_playlist, app.random, app.loop_file, app.consume],
        sleep: app.sleep_timer_label(),
    }
}

/// The header block: two rows, boxed or not by `style`.
pub(crate) fn draw_header_block(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    info: &HeaderInfo,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width < 16 || inner.height == 0 {
        return;
    }
    let has_row2 = inner.height >= 2;

    let side_w = header_side_width(inner.width, info.mode_words);
    let (left_w, right_w) = (side_w, side_w);
    let center_w = inner.width.saturating_sub(left_w + right_w + 2);

    let left = Rect { x: inner.x, y: inner.y, width: left_w, height: 1 };
    let center = Rect { x: inner.x + left_w + 1, y: inner.y, width: center_w, height: 1 };
    let right =
        Rect { x: inner.x + left_w + 1 + center_w + 1, y: inner.y, width: right_w, height: 1 };

    let (state_str, state_style) = if info.stopped {
        ("[Stopped]", theme::dim())
    } else if info.paused {
        ("[Paused]", theme::secondary())
    } else {
        ("[Playing]", theme::secondary())
    };
    f.render_widget(Paragraph::new(state_str).style(state_style), left);

    let title_line = match &info.track {
        Some((title, _, _)) => Line::from(Span::styled(
            theme::ellipsize(title, center.width),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        None => Line::from(Span::styled("No Song", theme::dim())),
    };
    f.render_widget(Paragraph::new(title_line).alignment(Alignment::Center), center);

    let vol = volume_spans(info.volume, info.volume_display);
    f.render_widget(Paragraph::new(Line::from(vol)).alignment(Alignment::Right), right);

    if has_row2 {
        let y = inner.y + 1;
        let left2 = Rect { y, ..left };
        let center2 = Rect { y, ..center };
        let right2 = Rect { y, ..right };

        f.render_widget(
            Paragraph::new(header_times(&info.times, &info.audio.0, &info.audio.1, left2.width))
                .style(theme::text()),
            left2,
        );

        let credit = match &info.track {
            Some((_, artist, album)) => {
                let mut spans =
                    vec![Span::styled(theme::ellipsize(artist, center2.width), theme::secondary())];
                if !album.is_empty() {
                    spans.push(Span::styled(" - ", theme::text()));
                    spans.push(Span::styled(theme::ellipsize(album, center2.width), theme::text()));
                }
                Line::from(spans)
            }
            None => Line::from(""),
        };
        f.render_widget(Paragraph::new(credit).alignment(Alignment::Center), center2);

        let mut icon_spans: Vec<Span> = Vec::new();
        if let Some(label) = &info.sleep {
            icon_spans.push(Span::styled(label.clone(), theme::secondary()));
            icon_spans.push(Span::raw(" "));
        }
        // The words step down to what the sleep label leaves.
        let lead: usize = icon_spans.iter().map(|s| s.content.chars().count()).sum();
        let room = (right2.width as usize).saturating_sub(lead);
        let labels = mode_flag_labels(info.mode_words, info.ascii, room);
        icon_spans.extend(mode_flag_spans(labels, info.flags));
        f.render_widget(Paragraph::new(Line::from(icon_spans)).alignment(Alignment::Right), right2);
    }

    draw_vdivider(f, inner.x + left_w, inner);
    draw_vdivider(f, inner.x + left_w + 1 + center_w, inner);
}

fn tab_name(t: Tab) -> &'static str {
    match t {
        Tab::Queue => "Queue",
        Tab::Browser => "Browser",
        Tab::Playlists => "Playlists",
        Tab::Search => "Search",
        Tab::Help => "Help",
    }
}

fn tab_spans(names: &[&str], active: usize) -> Vec<Span<'static>> {
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == active { theme::tab_active() } else { theme::tab_inactive() };
            Span::styled(format!(" {name} "), style)
        })
        .collect()
}

/// The config screen's tab bar: always boxed, whatever `theme.borders` says.
pub fn draw_tabs(f: &mut Frame, area: Rect, names: &[&str], active: usize) {
    draw_tab_strip(f, area, theme::BorderStyle::Box, names, active);
}

fn draw_tab_strip(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    names: &[&str],
    active: usize,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(tab_spans(names, active))).alignment(Alignment::Center),
        inner,
    );
}

/// Every spelling of the flags, widest first.
fn flag_label_forms(words: bool, ascii: bool) -> Vec<[&'static str; 4]> {
    let mut forms: Vec<[&'static str; 4]> = Vec::new();
    let mut room = usize::MAX;
    loop {
        let labels = mode_flag_labels(words, ascii, room);
        if forms.last() == Some(&labels) {
            return forms;
        }
        let width = labels.iter().map(|l| l.chars().count()).sum::<usize>() + 3;
        forms.push(labels);
        room = width - 1;
    }
}

/// The compact header's right side, widest first: the flags shorten before the
/// volume drops its meter, then its label.
fn compact_right_forms(info: &HeaderInfo) -> Vec<(Vec<Span<'static>>, u16)> {
    let mut volumes = vec![volume_spans(info.volume, info.volume_display)];
    if info.volume_display != VolumeDisplay::Percent {
        volumes.push(volume_spans(info.volume, VolumeDisplay::Percent));
    }
    volumes.push(vec![Span::styled(
        format!("{}%", info.volume.round() as i32),
        theme::accent().add_modifier(Modifier::BOLD),
    )]);
    let labels = flag_label_forms(info.mode_words, info.ascii);
    let mut forms = Vec::new();
    for volume in volumes {
        for l in &labels {
            let mut spans = volume.clone();
            spans.push(Span::raw("  "));
            spans.extend(mode_flag_spans(*l, info.flags));
            let width = spans.iter().map(|s| theme::str_width(&s.content)).sum::<usize>() as u16;
            forms.push((spans, width));
        }
    }
    forms
}

/// The compact header: sleep timer, tabs and right side on one row. The tabs
/// are never cut; the right side goes first, the sleep timer last.
pub(crate) fn draw_tab_row(
    f: &mut Frame,
    area: Rect,
    style: theme::BorderStyle,
    names: &[&str],
    active: usize,
    info: &HeaderInfo,
) {
    let inner = draw_block_as(f, area, style, None);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let tabs = tab_spans(names, active);
    let tabs_w = tabs.iter().map(|s| theme::str_width(&s.content)).sum::<usize>() as u16;
    let sleep_w = info.sleep.as_deref().map_or(0, |s| theme::str_width(s) as u16 + 1);

    let forms = compact_right_forms(info);
    let mut tries: Vec<(bool, Option<usize>)> = (0..forms.len()).map(|i| (true, Some(i))).collect();
    tries.extend([(true, None), (false, None)]);
    let used = |(with_sleep, right): (bool, Option<usize>)| {
        let left = if with_sleep { sleep_w } else { 0 };
        (left, right.map_or(0, |i| forms[i].1 + 1))
    };
    let pick = tries
        .into_iter()
        .find(|t| {
            let (l, r) = used(*t);
            l + tabs_w + r <= inner.width
        })
        .unwrap_or((false, None));
    let (left_used, right_used) = used(pick);

    let centred = inner.width.saturating_sub(tabs_w) / 2;
    let x =
        centred.clamp(left_used, (inner.width.saturating_sub(right_used + tabs_w)).max(left_used));
    let row = Rect { height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(tabs)),
        Rect { x: row.x + x, width: tabs_w, ..row }.intersection(row),
    );
    if let (true, Some(sleep)) = (pick.0, &info.sleep) {
        f.render_widget(
            Paragraph::new(sleep.as_str()).style(theme::secondary()),
            Rect { width: sleep_w.saturating_sub(1), ..row },
        );
    }
    if let Some(i) = pick.1 {
        let (spans, width) = &forms[i];
        f.render_widget(
            Paragraph::new(Line::from(spans.clone())),
            Rect { x: row.right() - width, width: *width, ..row },
        );
    }
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

/// An inner rule on `sides` of `area`, and the area inside it. In `none` the
/// rule is not drawn but its cell stays, so the layout is the same.
fn draw_divider(f: &mut Frame, area: Rect, sides: Borders) -> Rect {
    let block = Block::default().borders(sides).border_style(theme::border());
    let inner = block.inner(area);
    if theme::spacing().borders != theme::BorderStyle::None {
        f.render_widget(block, area);
    }
    inner
}

/// True when the cover's metadata was drawn.
fn draw_queue_tab(f: &mut Frame, app: &mut App, area: Rect) -> bool {
    let inner = draw_block(f, area);
    if inner.width < 10 || inner.height < 5 {
        return false;
    }
    // The art pane is pixel-square, capped at `art_pct` so the queue keeps its
    // columns. With art disabled it collapses and the queue gets the full width.
    let (art_area, queue_area) = if !app.art_enabled {
        (None, inner)
    } else {
        let cols = if app.art_square && app.picker.is_some() {
            let cell = app.picker.as_ref().map(|p| p.font_size()).unwrap_or((10, 20));
            let img_h = inner.height - art_meta_rows(inner.height, app.art_metadata);
            let square = square_art_width(img_h, cell);
            let art_pct = app.art_pct.clamp(10, 90);
            let max_by_pct = (inner.width as u32 * art_pct as u32 / 100) as u16;
            let max_art = inner.width.saturating_sub(MIN_QUEUE_COLS).min(max_by_pct);
            let art_w = square
                .clamp(MIN_ART_COLS, max_art.max(MIN_ART_COLS))
                .min(inner.width.saturating_sub(1).max(1));
            Layout::horizontal([Constraint::Length(art_w), Constraint::Min(1)]).split(inner)
        } else {
            let art_pct = app.art_pct.clamp(10, 90);
            Layout::horizontal([
                Constraint::Percentage(art_pct),
                Constraint::Percentage(100 - art_pct),
            ])
            .split(inner)
        };
        // The divider hugs the pane, so the queue column gets its own gutter.
        let q = cols[1];
        let gutter = theme::spacing().inset.min(q.width);
        let q = Rect { x: q.x + gutter, width: q.width - gutter, ..q };
        (Some(cols[0]), q)
    };
    let meta_shown = art_area.is_some_and(|pane| draw_art_pane(f, app, pane));

    let viz_h = viz_height(app.visualizer_pct, queue_area.height);
    let rows = Layout::vertical([
        Constraint::Length(queue_area.height.saturating_sub(viz_h)),
        Constraint::Length(viz_h),
    ])
    .split(queue_area);
    let qarea = draw_divider(f, rows[0], Borders::BOTTOM);
    draw_queue_table(f, app, qarea);
    if viz_h > 0 {
        draw_visualizer(f, app, rows[1]);
    }
    meta_shown
}

/// The visualizer is never shorter than this, space permitting.
const MIN_VIZ_ROWS: u16 = 8;
/// The queue keeps at least this many columns beside the art.
const MIN_QUEUE_COLS: u16 = 30;
/// The art pane's floor, except on very narrow terminals.
const MIN_ART_COLS: u16 = 6;
/// Top separator + title + artist + album.
const ART_META_ROWS: u16 = 4;

/// Rows the metadata footer takes; none when it is off or would leave the cover too small.
fn art_meta_rows(height: u16, metadata: bool) -> u16 {
    if metadata && height >= ART_META_ROWS + 4 {
        ART_META_ROWS
    } else {
        0
    }
}

/// Width in cells of a pixel-square area `height` rows tall.
fn square_art_width(height: u16, cell: (u16, u16)) -> u16 {
    if height == 0 || cell.0 == 0 || cell.1 == 0 {
        return 0;
    }
    ((height as f64 * cell.1 as f64 / cell.0 as f64).round() as u16).max(1)
}

/// Visualizer rows for a percentage of the queue column (0 hides it). At least
/// `MIN_VIZ_ROWS`, but the queue keeps 6 rows: the visualizer shrinks to 3, then hides.
fn viz_height(pct: u16, available: u16) -> u16 {
    if pct == 0 {
        return 0;
    }
    let want = (available as u32 * pct.min(100) as u32 / 100) as u16;
    let v = want.max(MIN_VIZ_ROWS).min(available.saturating_sub(6));
    if v < 3 {
        0
    } else {
        v
    }
}

/// True when the metadata footer was drawn.
fn draw_art_pane(f: &mut Frame, app: &mut App, pane: Rect) -> bool {
    draw_divider(f, pane, Borders::RIGHT);
    // The left gutter and the right divider are the only frame.
    let inset = theme::spacing().inset;
    let x = pane.x.saturating_sub(inset);
    let content = Rect {
        x,
        y: pane.y,
        // ends just before the divider
        width: (pane.x + pane.width).saturating_sub(1).saturating_sub(x),
        height: pane.height,
    };
    if content.width < 6 || content.height < 4 {
        return false;
    }

    if app.art_square && app.picker.is_some() {
        let meta_h = art_meta_rows(content.height, app.art_metadata);
        let art_h = content.height.saturating_sub(meta_h);
        let image_area = Rect { height: art_h, ..content };
        draw_art_image(f, app, image_area);
        if meta_h > 0 {
            let meta_area = Rect { y: content.y + art_h, height: meta_h, ..content };
            return draw_art_meta(f, app, meta_area);
        }
        return false;
    }
    draw_art_image(f, app, content);
    false
}

fn draw_art_image(f: &mut Frame, app: &mut App, content: Rect) {
    if content.width < 6 || content.height < 4 {
        return;
    }

    // A cover is placed on a canvas of the whole area, to the pixel; without
    // one the placeholder box depends only on the layout.
    let covered = app.art.img.is_some() && app.picker.is_some();
    let rect = if covered {
        Some(content)
    } else {
        art_rect(content, app.picker.as_ref().map(|p| p.font_size()))
    };
    let Some(rect) = rect else {
        placeholder(f, content, "No cover");
        return;
    };

    // The terminal already holds this image: skip its cells rather than copy
    // the protocol payload again.
    if app.art.protocol.is_some()
        && app.art.shown_rect == Some(rect)
        && app.art.prepared_rect == Some(rect)
    {
        let buf = f.buffer_mut();
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_skip(true);
                }
            }
        }
        return;
    }

    // Cells were just cleared: the cover waits (the small "No cover" box does not).
    if let Some(until) = app.art.hold_until {
        if covered && Instant::now() < until {
            return;
        }
        app.art.hold_until = None;
    }

    // ratatui-image pads a resized image at (0, 0), so centre the cover on a
    // canvas of the rect's size first.
    if app.art.protocol.is_none() || app.art.prepared_rect != Some(rect) {
        let picker = app.picker;
        let img = app.art.img.clone();
        if let (Some(picker), Some(img)) = (picker, img) {
            let cell = picker.font_size();
            if let Some(canvas) = centered_art_canvas(
                img.as_ref(),
                rect,
                cell,
                app.art_pad_px,
                app.art_max_px,
                art_padding_fill(),
            ) {
                app.art.protocol = Some(picker.new_resize_protocol(canvas));
                app.art.prepared_rect = Some(rect);
            } else {
                app.art.protocol = None;
                app.art.prepared_rect = None;
            }
        } else {
            app.art.protocol = None;
            app.art.prepared_rect = None;
        }
    }
    if let Some(prot) = app.art.protocol.as_mut() {
        // The canvas is already centred; Scale fills `rect` exactly.
        let widget =
            ratatui_image::StatefulImage::default().resize(ratatui_image::Resize::Scale(None));
        f.render_stateful_widget(widget, rect, prot);
        app.art.shown_rect = Some(rect);
        app.art.emitted = true;
    } else {
        placeholder_box(f, rect, "No cover");
        app.art.shown_rect = None;
    }
}

/// Title, artist and album below the cover.
fn draw_art_meta(f: &mut Frame, app: &App, area: Rect) -> bool {
    if area.width < 10 || area.height < 3 {
        return false;
    }
    let Some(t) = app.current_track() else { return false };
    draw_divider(f, area, Borders::TOP);
    let text_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };
    if text_area.width == 0 || text_area.height == 0 {
        return false;
    }
    let lines = vec![
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.title), text_area.width),
            theme::text().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.artist), text_area.width),
            theme::secondary(),
        )),
        Line::from(Span::styled(
            theme::ellipsize(&clean(&t.album), text_area.width),
            theme::text(),
        )),
    ];
    f.render_widget(Paragraph::new(lines), text_area);
    true
}

fn digits(n: usize) -> u16 {
    theme::digits(n)
}

/// The `#` column is wide enough for every count under a thousand, so an
/// ordinary album never moves the columns beside it.
const TRACK_NUMBER_MIN_WIDTH: u16 = 3;

/// Width of the `#` column: what the largest possible number needs, with a
/// floor so a 9-track and a 100-track album start Title at the same offset.
fn track_number_width(bound: usize) -> u16 {
    digits(bound).max(TRACK_NUMBER_MIN_WIDTH)
}

/// The letterbox: transparent without a theme background, else opaque in it.
/// Some terminals draw transparent pixels over their own background.
fn art_padding_fill() -> image::Rgba<u8> {
    match theme::theme().background {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => image::Rgba([0, 0, 0, 0]),
    }
}

/// The cover fitted and centred in pixels on a canvas the size of `area`, with
/// `pad_px` on the limiting axis and at most `max_px` (0 = unlimited).
fn centered_art_canvas(
    img: &DynamicImage,
    area: Rect,
    cell: (u16, u16),
    pad_px: u16,
    max_px: [u32; 2],
    fill: image::Rgba<u8>,
) -> Option<DynamicImage> {
    let cw = area.width as u32 * cell.0 as u32;
    let ch = area.height as u32 * cell.1 as u32;
    if cw == 0 || ch == 0 {
        return None;
    }
    let iw = img.width().max(1) as f64;
    let ih = img.height().max(1) as f64;
    let pad = (pad_px as u32).min((cw.min(ch) - 1) / 2);
    let (fit_w, fit_h) = (cw - 2 * pad, ch - 2 * pad);
    let mut scale = (fit_w as f64 / iw).min(fit_h as f64 / ih);
    for (cap, side) in [(max_px[0], iw), (max_px[1], ih)] {
        if cap > 0 {
            scale = scale.min(cap as f64 / side);
        }
    }
    let w = ((iw * scale).round() as u32).clamp(1, fit_w);
    let h = ((ih * scale).round() as u32).clamp(1, fit_h);
    let (x, y) = ((cw - w) / 2, (ch - h) / 2);
    let cover = img.resize_exact(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(cw, ch, fill);
    image::imageops::overlay(&mut canvas, &cover, x as i64, y as i64);
    Some(DynamicImage::ImageRgba8(canvas))
}

/// The whole-cell box closest to the cover's square, with an even leftover so
/// it centres exactly. `None` when the pane is too small.
fn art_rect(content: Rect, cell: Option<(u16, u16)>) -> Option<Rect> {
    let (art_w, art_h) = match cell.filter(|c| c.0 > 0 && c.1 > 0) {
        Some(cell) => square_box(content, cell),
        None => two_to_one_box(content),
    };
    if art_w < 4 || art_h < 2 {
        return None;
    }
    Some(Rect {
        x: content.x + (content.width - art_w) / 2,
        y: content.y + (content.height - art_h) / 2,
        width: art_w,
        height: art_h,
    })
}

/// The box for unknown cell sizes: 1:2 in cells, as wide as the height allows.
fn two_to_one_box(content: Rect) -> (u16, u16) {
    let mut art_w = content.width.min(content.height.saturating_mul(2));
    if (content.width - art_w) % 2 == 1 {
        art_w -= 1;
    }
    let mut art_h = art_w.div_ceil(2);
    if (content.height - art_h) % 2 == 1 {
        art_h += 1;
    }
    (art_w, art_h)
}

/// The (width, height) closest to a pixel square of the shorter side, with an
/// even leftover; a tie takes the larger box.
fn square_box(content: Rect, cell: (u16, u16)) -> (u16, u16) {
    let (cw, ch) = (cell.0 as u32, cell.1 as u32);
    let side = (content.width as u32 * cw).min(content.height as u32 * ch);
    let w0 = ((side + cw / 2) / cw).min(content.width as u32);
    let h0 = ((side + ch / 2) / ch).min(content.height as u32);
    let mut best: Option<(u32, u32, u32)> = None;
    for w in w0.saturating_sub(1)..=w0 + 1 {
        for h in h0.saturating_sub(1)..=h0 + 1 {
            if w == 0 || h == 0 || w > content.width as u32 || h > content.height as u32 {
                continue;
            }
            if (content.width as u32 - w) % 2 == 1 || (content.height as u32 - h) % 2 == 1 {
                continue;
            }
            let off = (w * cw).abs_diff(h * ch);
            if best.is_none_or(|(o, bw, bh)| off < o || (off == o && w * h > bw * bh)) {
                best = Some((off, w, h));
            }
        }
    }
    best.map_or((0, 0), |(_, w, h)| (w as u16, h as u16))
}

/// The narrowest a weighted column is drawn.
const WEIGHTED_COL_MIN: u16 = 6;

/// At the default weights each table keeps its own `defaults`, which plain
/// weights cannot reproduce.
const DEFAULT_WEIGHTS: [u32; 3] = [30, 24, 46];

/// Which of a track table's text columns (Title, Artist, Album) exist and how
/// wide; constraints, headers and cells all read it so they agree.
#[derive(Clone, Copy)]
struct TrackCols {
    /// `ui.column_weights`: Title, Artist, Album.
    weights: [u32; 3],
    /// Each table column's index into `weights` (Search puts Artist first).
    order: [usize; 3],
    defaults: [Constraint; 3],
}

impl TrackCols {
    fn new(weights: [u32; 3], order: [usize; 3], defaults: [Constraint; 3]) -> Self {
        Self { weights, order, defaults }
    }

    /// Title, Artist, Album order: Queue, Playlists and the album drill-in.
    fn plain(weights: [u32; 3], defaults: [Constraint; 3]) -> Self {
        Self::new(weights, [0, 1, 2], defaults)
    }

    fn weight(&self, col: usize) -> u32 {
        self.weights[self.order[col]]
    }

    /// Table column `col`'s position among the shown ones, None when hidden.
    fn slot(&self, col: usize) -> Option<usize> {
        (self.weight(col) > 0).then(|| (0..col).filter(|c| self.weight(*c) > 0).count())
    }

    fn constraints(&self) -> Vec<Constraint> {
        if self.weights == DEFAULT_WEIGHTS {
            return self.defaults.to_vec();
        }
        (0..3)
            .filter(|c| self.weight(*c) > 0)
            .map(|c| Constraint::Fill(self.weight(c).min(u16::MAX as u32) as u16))
            .collect()
    }

    fn headers(&self, labels: [&'static str; 3]) -> Vec<HeaderSpec> {
        (0..3).filter(|c| self.weight(*c) > 0).map(|c| HeaderSpec::left(labels[c])).collect()
    }
}

fn track_cols(num_w: Option<u16>, len_w: u16, tc: &TrackCols) -> Vec<Constraint> {
    let mut v = Vec::new();
    if let Some(w) = num_w {
        v.push(Constraint::Length(w));
    }
    v.extend(tc.constraints());
    v.push(Constraint::Length(len_w));
    v
}

const TRACK_LEGACY: [Constraint; 3] =
    [Constraint::Percentage(30), Constraint::Percentage(24), Constraint::Min(12)];

/// Width of the Len column: the longest track's time, at least `LEN_COL_MIN`.
fn len_col_width(longest_ms: Option<u64>) -> u16 {
    let w = longest_ms.map_or(0, |ms| theme::format_time(ms as f64 / 1000.0).len());
    (w as u16).max(theme::LEN_COL_MIN)
}

fn track_headers(num_w: Option<u16>, tc: &TrackCols) -> Vec<HeaderSpec> {
    let mut v = Vec::new();
    if num_w.is_some() {
        v.push(HeaderSpec::right("#"));
    }
    v.extend(tc.headers(["Title", "Artist", "Album"]));
    v.push(HeaderSpec::right("Len"));
    v
}

/// The Queue's columns in display order, with the widths of the fixed ones;
/// constraints, headers and cells all read it so they agree.
struct QueueCols {
    cols: Vec<QueueColumn>,
    weights: [u32; 3],
    len_w: u16,
    track_w: u16,
    year_w: u16,
    codec_w: u16,
}

/// Codec values longer than this are ellipsized.
const CODEC_COL_MAX: u16 = 8;

impl QueueCols {
    /// `listed` is `App::queue_columns`; on a table narrower than the rest
    /// can fit, codec, year and track go first.
    fn new(
        listed: &[QueueColumn],
        weights: [u32; 3],
        tracks: &[crate::plex::Track],
        width: u16,
    ) -> Self {
        let digits = |n: Option<u32>| n.map_or(0, |n| n.to_string().len() as u16);
        let codec = |t: &crate::plex::Track| {
            t.audio.codec.as_ref().map_or(0, |c| theme::str_width(c) as u16)
        };
        let mut qc = Self {
            cols: listed
                .iter()
                .copied()
                .filter(|c| c.text_index().is_none_or(|i| weights[i] > 0))
                .collect(),
            weights,
            len_w: len_col_width(tracks.iter().filter_map(|t| t.duration_ms).max()),
            track_w: tracks.iter().map(|t| digits(t.track_no)).max().unwrap_or(0).max(1),
            year_w: tracks.iter().map(|t| digits(t.year)).max().unwrap_or(0).max(4),
            codec_w: tracks.iter().map(codec).max().unwrap_or(0).clamp(5, CODEC_COL_MAX),
        };
        for extra in [QueueColumn::Codec, QueueColumn::Year, QueueColumn::Track] {
            if qc.min_width() <= width as u32 {
                break;
            }
            qc.cols.retain(|c| *c != extra);
        }
        qc
    }

    fn fixed_width(&self, col: QueueColumn) -> Option<u16> {
        match col {
            QueueColumn::Len => Some(self.len_w),
            QueueColumn::Track => Some(self.track_w),
            QueueColumn::Year => Some(self.year_w),
            QueueColumn::Codec => Some(self.codec_w),
            _ => None,
        }
    }

    fn weight(&self, col: QueueColumn) -> u32 {
        col.text_index().map_or(0, |i| self.weights[i])
    }

    fn min_width(&self) -> u32 {
        let width: u32 =
            self.cols.iter().map(|c| self.fixed_width(*c).unwrap_or(WEIGHTED_COL_MIN) as u32).sum();
        width + self.cols.len().saturating_sub(1) as u32
    }

    fn constraints(&self) -> Vec<Constraint> {
        if self.cols == QueueColumn::DEFAULT && self.weights == DEFAULT_WEIGHTS {
            let mut v = TRACK_LEGACY.to_vec();
            v.push(Constraint::Length(self.len_w));
            return v;
        }
        self.cols
            .iter()
            .map(|c| match self.fixed_width(*c) {
                Some(w) => Constraint::Length(w),
                None => Constraint::Fill(self.weight(*c).min(u16::MAX as u32) as u16),
            })
            .collect()
    }

    fn headers(&self) -> Vec<HeaderSpec> {
        self.cols
            .iter()
            .map(|c| match c {
                QueueColumn::Title => HeaderSpec::left("Title"),
                QueueColumn::Artist => HeaderSpec::left("Artist"),
                QueueColumn::Album => HeaderSpec::left("Album"),
                QueueColumn::Len => HeaderSpec::right("Len"),
                QueueColumn::Track => HeaderSpec::right("#"),
                QueueColumn::Year => HeaderSpec::right("Year"),
                QueueColumn::Codec => HeaderSpec::left("Codec"),
            })
            .collect()
    }

    /// `col_w` is the laid-out width of each column of `cols`.
    fn cells(
        &self,
        t: &crate::plex::Track,
        base: Style,
        playing: bool,
        col_w: &[u16],
        needle: &str,
    ) -> Vec<Cell<'static>> {
        let dim = if playing { base } else { theme::dim() };
        let right = |n: Option<u32>| {
            Cell::from(Line::from(n.map(|n| n.to_string()).unwrap_or_default()).right_aligned())
                .style(dim)
        };
        self.cols
            .iter()
            .zip(col_w)
            .map(|(c, w)| match c {
                QueueColumn::Title => cell(&t.title, *w, needle, base),
                QueueColumn::Artist => cell(&t.artist, *w, needle, base),
                QueueColumn::Album => cell(&t.album, *w, needle, base),
                QueueColumn::Len => len_cell(t, dim),
                QueueColumn::Track => right(t.track_no),
                QueueColumn::Year => right(t.year),
                QueueColumn::Codec => {
                    let codec = t.audio.codec.as_deref().unwrap_or("").to_uppercase();
                    Cell::from(theme::ellipsize(&codec, *w)).style(dim)
                }
            })
            .collect()
    }
}

fn draw_queue_table(f: &mut Frame, app: &mut App, area: Rect) {
    if area.width < 10 || area.height < 3 {
        return;
    }
    let raw = app.queue.len();
    let total = app.queue_filter.len(raw);
    let ind = indicator_text(&app.queue_filter, raw);
    let visible_rows = area.height.saturating_sub(u16::from(ind.is_some()) + 2) as usize;
    let (content, _) = list_split(area, total, visible_rows);
    let qc = QueueCols::new(&app.queue_columns, app.column_weights, &app.queue, content.width);
    let frame = table_frame(f, area, total, &qc.constraints(), Some(&qc.headers()), ind);

    let cursor = app.queue_state.selected();
    let visible = frame.rows.height as usize;
    let offset =
        app.scroll.update(ScrollId::Queue, cursor.unwrap_or(0), visible, total, app.scrolloff);
    if total == 0 {
        if app.queue_filter.is_active() {
            no_matches(f, frame.rows);
        }
        if let Some(bar) = frame.bar {
            render_scrollbar(f, bar, total, offset, visible);
        }
        return;
    }

    let needle = app.queue_filter.text.as_str();
    let current_key = app.current_track().map(|t| t.rating_key.clone());
    let rows = build_track_rows(
        &app.queue,
        &app.queue_filter,
        offset,
        visible,
        total,
        current_key.as_deref(),
        app.visual_range(),
        |t, _, base, playing| qc.cells(t, base, playing, &frame.col_w, needle),
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

/// Rows of a track table; `cells` makes one row's cells from the track, its
/// index in `tracks` and the row's base style.
#[allow(clippy::too_many_arguments)]
fn build_track_rows(
    tracks: &[crate::plex::Track],
    filter: &ListFilter,
    offset: usize,
    visible: usize,
    total: usize,
    current_key: Option<&str>,
    selected: Option<(usize, usize)>,
    cells: impl Fn(&crate::plex::Track, usize, Style, bool) -> Vec<Cell<'static>>,
) -> Vec<Row<'static>> {
    let raw = tracks.len();
    let end = (offset + visible).min(total);
    let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
    for fi in offset..end {
        let real = filter.real(fi, raw);
        let Some(t) = tracks.get(real) else { continue };
        let playing = current_key == Some(t.rating_key.as_str());
        let base =
            if playing { theme::accent().add_modifier(Modifier::BOLD) } else { theme::text() };
        let row = Row::new(cells(t, real, base, playing));
        rows.push(match selected {
            Some((lo, hi)) if (lo..=hi).contains(&fi) => row.style(theme::current_unfocused()),
            _ => row,
        });
    }
    rows
}

/// The cells of a Playlists or Search track row: optional number, the shown
/// text columns, Len.
#[allow(clippy::too_many_arguments)]
fn plain_track_cells(
    t: &crate::plex::Track,
    real: usize,
    base: Style,
    playing: bool,
    col_w: &[u16],
    num_w: Option<u16>,
    tc: &TrackCols,
    needle: &str,
) -> Vec<Cell<'static>> {
    let base_idx = if num_w.is_some() { 1 } else { 0 };
    let dim = if playing { base } else { theme::dim() };
    let mut cells: Vec<Cell> = Vec::new();
    if num_w.is_some() {
        cells.push(Cell::from(Line::from(Span::styled(
            format!("{:>w$}", real + 1, w = col_w[0] as usize),
            dim,
        ))));
    }
    for (col, text) in [&t.title, &t.artist, &t.album].into_iter().enumerate() {
        if let Some(slot) = tc.slot(col) {
            cells.push(cell(text, col_w[base_idx + slot], needle, base));
        }
    }
    cells.push(len_cell(t, dim));
    cells
}

fn len_cell(t: &crate::plex::Track, style: Style) -> Cell<'static> {
    let dur = t.duration_ms.map(|ms| theme::format_time(ms as f64 / 1000.0)).unwrap_or_default();
    Cell::from(Line::from(dur).right_aligned()).style(style)
}

fn draw_browser_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
        let raw = app.browser.artists.len();
        let total = app.browser_filters[0].len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.browser_filters[0], raw);
        let cursor = app.browser.artists_state.selected();
        let (offset, visible) = window(app, ScrollId::BrowserArtists, cursor, total, content);
        if total == 0 && app.browser_filters[0].is_active() {
            no_matches(f, content);
        } else {
            let text = app.browser_filters[0].text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.browser_filters[0].real(fi, raw);
                let title =
                    theme::ellipsize(&clean(&app.browser.artists[real].title), content.width);
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
        let raw = app.browser.albums.len();
        let total = app.browser_filters[1].len(raw);
        let (content, bar) = begin_list(f, pane, total, &app.browser_filters[1], raw);
        let cursor = app.browser.albums_state.selected();
        let (offset, visible) = window(app, ScrollId::BrowserAlbums, cursor, total, content);
        // unknown means no artist suffix
        let browsed = app.browser.albums_for.as_deref().and_then(|key| {
            app.browser.artists.iter().find(|a| a.rating_key == key).map(|a| clean(&a.title))
        });
        if total == 0 && app.browser_filters[1].is_active() {
            no_matches(f, content);
        } else {
            let text = app.browser_filters[1].text.as_str();
            let items = build_range(offset, visible, total, |fi, items: &mut Vec<ListItem>| {
                let real = app.browser_filters[1].real(fi, raw);
                let a = &app.browser.albums[real];
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
        let raw = app.browser.tracks.len();
        let total = app.browser_filters[2].len(raw);
        let ind = indicator_text(&app.browser_filters[2], raw);
        // The printed number is a position, so the loaded count bounds it.
        let num_w = track_number_width(raw);
        let cols_c = [
            Constraint::Length(num_w),
            Constraint::Min(10),
            Constraint::Length(len_col_width(
                app.browser.tracks.iter().filter_map(|t| t.duration_ms).max(),
            )),
        ];
        let frame = table_frame(f, pane, total, &cols_c, None, ind);
        let cursor = app.browser.tracks_state.selected();
        let visible = frame.rows.height as usize;
        let offset = app.scroll.update(
            ScrollId::BrowserTracks,
            cursor.unwrap_or(0),
            visible,
            total,
            app.scrolloff,
        );
        if total == 0 {
            if app.browser_filters[2].is_active() {
                no_matches(f, frame.rows);
            }
        } else {
            let needle = app.browser_filters[2].text.as_str();
            let parent_artist = parent_artist(app);
            let cur_key = app.current_track().map(|t| t.rating_key.clone());
            let end = (offset + visible).min(total);
            let selected = app.visual_range();
            let mut rows: Vec<Row> = Vec::with_capacity(end.saturating_sub(offset));
            for fi in offset..end {
                let real = app.browser_filters[2].real(fi, raw);
                let t = &app.browser.tracks[real];
                let playing = cur_key.as_deref() == Some(t.rating_key.as_str());
                let base = if playing {
                    theme::accent().add_modifier(Modifier::BOLD)
                } else {
                    theme::text()
                };
                let dim = if playing { base } else { theme::dim() };
                let mut spans = spans_for(&t.title, needle, base);
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
    app.browser.albums.iter().find(|a| a.rating_key == key).map(|a| clean(&a.artist))
}

fn draw_playlists_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
            plain_track_cells(t, real, base, playing, &frame.col_w, num_w, &tc, needle)
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

fn draw_search_tab(f: &mut Frame, app: &mut App, area: Rect) {
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
        for (col, text) in [(1, &hit.title), (2, &hit.subtitle)] {
            if let Some(slot) = tc.slot(col) {
                cells.push(
                    Cell::from(Line::from(theme::ellipsize(&clean(text), frame.col_w[slot])))
                        .style(base),
                );
            }
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
    let label_line = Line::from(vec![
        Span::styled("▸ ", theme::accent()),
        Span::styled(
            theme::ellipsize(&clean(&label), inner.width.saturating_sub(14)),
            theme::secondary(),
        ),
        Span::styled(format!(" ({} tracks)", total), theme::dim()),
    ]);
    f.render_widget(Paragraph::new(label_line), parts[0]);

    let tc = TrackCols::plain(
        app.column_weights,
        [Constraint::Percentage(34), Constraint::Percentage(24), Constraint::Min(12)],
    );
    let mut cols = tc.constraints();
    cols.push(Constraint::Length(len_col_width(
        app.search
            .detail
            .as_ref()
            .and_then(|d| d.tracks.iter().filter_map(|t| t.duration_ms).max()),
    )));
    let mut headers = tc.headers(["Title", "Artist", "Album"]);
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
        for (col, text) in [&t.title, &t.artist, &t.album].into_iter().enumerate() {
            if let Some(slot) = tc.slot(col) {
                cells.push(
                    Cell::from(Line::from(theme::ellipsize(&clean(text), frame.col_w[slot])))
                        .style(base),
                );
            }
        }
        cells.push(Cell::from(Line::from(dur).right_aligned()).style(theme::dim()));
        rows.push(Row::new(cells));
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

/// The widest the Help tab's key column grows, and the gap after it.
const HELP_KEY_COL_MAX: usize = 22;
const HELP_GAP: usize = 2;

fn draw_help_tab(f: &mut Frame, app: &mut App, area: Rect) {
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

fn draw_bottom(f: &mut Frame, app: &mut App, area: Rect, titled: bool) {
    let style = theme::spacing().borders;
    let title =
        titled.then(|| now_playing(app, area.width.saturating_sub(4))).flatten().map(|line| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(line.spans);
            spans.push(Span::raw(" "));
            Line::from(spans)
        });
    let inner = draw_block_as(f, area, style, title);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if app.command.typing {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(": ", theme::secondary()),
                Span::styled(tail_fit(&app.command.text, inner.width, 3), theme::text()),
                Span::styled("█", theme::accent()),
            ]))
            .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    if let Some((text, scope, shown, total)) = app.filter_typing() {
        let counter = format!("{scope} {shown}/{total}");
        let cw = theme::str_width(&counter) as u16 + 1;
        let cols = Layout::horizontal([Constraint::Min(4), Constraint::Length(cw)]).split(inner);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/ ", theme::secondary()),
                Span::styled(tail_fit(&text, cols[0].width, 3), theme::text()),
                Span::styled("█", theme::accent()),
            ])),
            cols[0],
        );
        f.render_widget(
            Paragraph::new(counter).style(theme::dim()).alignment(Alignment::Right),
            cols[1],
        );
        return;
    }

    if app.tab == Tab::Search && app.search.focused {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/ ", theme::secondary()),
                Span::styled(tail_fit(&app.search.input, inner.width, 3), theme::text()),
                Span::styled("█", theme::accent()),
            ]))
            .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    if !app.status.is_empty() {
        let style = if app.status_error { theme::error() } else { theme::text() };
        f.render_widget(
            Paragraph::new(Line::from(app.status.clone()).alignment(Alignment::Center))
                .style(style),
            inner,
        );
        return;
    }

    if let Some((lo, hi)) = app.visual_range() {
        let label = "-- VISUAL --";
        let counter = format!("{} selected", hi - lo + 1);
        let lw = theme::str_width(label) as u16;
        let cw = theme::str_width(&counter) as u16 + 1;
        // the counter goes first when the label would be cut
        let cw = if inner.width >= lw + cw { cw } else { 0 };
        let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(cw)]).split(inner);
        f.render_widget(Paragraph::new(label).style(theme::secondary()), cols[0]);
        if cw > 0 {
            f.render_widget(
                Paragraph::new(counter).style(theme::dim()).alignment(Alignment::Right),
                cols[1],
            );
        }
        return;
    }

    draw_progress(f, app, inner);
}

/// The end of `text` that fits `width` cells less `reserved` (prompt and caret),
/// so what is being typed stays visible.
fn tail_fit(text: &str, width: u16, reserved: u16) -> String {
    let room = width.saturating_sub(reserved) as usize;
    let mut used = 0;
    let mut start = text.len();
    for (i, c) in text.char_indices().rev() {
        used += theme::str_width(&c.to_string());
        if used > room {
            break;
        }
        start = i;
    }
    text[start..].to_string()
}

fn draw_progress(f: &mut Frame, app: &App, inner: Rect) {
    if inner.width < 8 || inner.height == 0 {
        return;
    }
    let t = icons::transport(app.icons_ascii);
    // A 2-cell field, so play, pause and stop take the same width.
    let glyph = if app.stopped {
        t.stop
    } else if app.paused {
        t.pause
    } else {
        t.play
    };
    let field = format!("{glyph:<2}");
    let gw = 2u16;

    // Both halves are as wide as the duration, so the bar does not jump when
    // the position gains a digit (9:59 -> 10:00).
    let dur = theme::format_time(app.duration);
    let pos = theme::format_time(app.position);
    let half = dur.len().max(pos.len());
    let times = format!("{pos:>half$} / {dur:>half$}");
    let right_w = (times.chars().count() as u16).saturating_add(1);
    let cols = Layout::horizontal([
        Constraint::Length(gw),
        Constraint::Length(1), // one blank cell before the bar
        Constraint::Min(6),
        Constraint::Length(right_w),
    ])
    .split(inner);

    f.render_widget(Paragraph::new(Line::from(Span::styled(field, theme::accent()))), cols[0]);

    let symbols = theme::symbols();
    // A bar too narrow for its end caps drops them.
    let room = cols[2].width as usize;
    let (cap_start, cap_end) =
        if room > 2 { (symbols.progress_start, symbols.progress_end) } else { (None, None) };
    let bar_w = room - usize::from(cap_start.is_some()) - usize::from(cap_end.is_some());
    let frac = if app.stopped || app.duration <= 0.0 {
        0.0
    } else {
        (app.position / app.duration).clamp(0.0, 1.0)
    };
    let filled = ((frac * bar_w as f64).round() as usize).min(bar_w);
    // Solid bar of background-coloured cells: `░` rendered dotted in some fonts.
    let fill_style = theme::progress_fill();
    let track_style = theme::progress_track(symbols.progress_empty.is_none());
    let solid_bg =
        |style: Style| Style::default().bg(style.fg.unwrap_or(ratatui::style::Color::Reset));
    let mut spans = Vec::with_capacity(5);
    if let Some(c) = cap_start {
        let style = if filled > 0 { fill_style } else { track_style };
        spans.push(Span::styled(c.to_string(), style));
    }
    if symbols.progress_filled.is_none()
        && symbols.progress_empty.is_none()
        && symbols.progress_head.is_none()
    {
        if filled > 0 {
            spans.push(Span::styled(" ".repeat(filled), solid_bg(fill_style)));
        }
        if bar_w > filled {
            spans.push(Span::styled(" ".repeat(bar_w - filled), solid_bg(track_style)));
        }
    } else {
        // Themed glyphs are foreground; a part without one stays solid.
        let part = |glyph: Option<char>, n: usize, style: Style| match glyph {
            Some(c) => Span::styled(c.to_string().repeat(n), style),
            None => Span::styled(" ".repeat(n), solid_bg(style)),
        };
        let body = match symbols.progress_head {
            Some(_) if filled > 0 => filled - 1,
            _ => filled,
        };
        if body > 0 {
            spans.push(part(symbols.progress_filled, body, fill_style));
        }
        if let (Some(head), true) = (symbols.progress_head, filled > 0) {
            // Over a solid track the head takes its colour as background, so a
            // Powerline triangle's empty half joins the track.
            let style = match symbols.progress_empty {
                None => solid_bg(track_style).patch(fill_style),
                Some(_) => fill_style,
            };
            spans.push(Span::styled(head.to_string(), style));
        }
        if bar_w > filled {
            spans.push(part(symbols.progress_empty, bar_w - filled, track_style));
        }
    }
    if let Some(c) = cap_end {
        let style = if filled >= bar_w { fill_style } else { track_style };
        spans.push(Span::styled(c.to_string(), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), cols[2]);

    f.render_widget(
        Paragraph::new(times).style(theme::text()).alignment(Alignment::Right),
        cols[3],
    );
}

/// Left padding for the bar group; an odd leftover cell goes right.
fn viz_pad(width: usize, group_w: usize) -> usize {
    width.saturating_sub(group_w) / 2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisualizerMode {
    Adaptive,
    Spread,
}

/// Spread mode: which of bar width and gap keeps its set size while the other
/// takes up the width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpreadAnchor {
    Bar,
    Gap,
}

/// The visualizer's cells: a width per bar, a width per gap (`bars.len() - 1`)
/// and the left padding. All bars share a width, as do all gaps.
#[derive(Debug, PartialEq, Eq)]
struct VizLayout {
    bars: Vec<usize>,
    gaps: Vec<usize>,
    pad: usize,
}

impl VizLayout {
    fn group_w(&self) -> usize {
        self.bars.iter().sum::<usize>() + self.gaps.iter().sum::<usize>()
    }
}

/// Adaptive adds bars as the pane widens, then widens them. Spread stops at
/// the set count and widens what the anchor does not name. Leftover is margin.
fn visualizer_layout(
    width: usize,
    mode: VisualizerMode,
    anchor: SpreadAnchor,
    min_bar_w: u16,
    min_gap: u16,
    fixed_bars: u16,
    max_bars: usize,
) -> VizLayout {
    let width = width.max(1);
    let max_bars = max_bars.max(1);
    let min_bar_w = (min_bar_w.max(1) as usize).min(width);
    match mode {
        VisualizerMode::Adaptive => {
            let min_gap = min_gap as usize;
            let divisor = (min_bar_w + min_gap).max(1);
            let bars = ((width + min_gap) / divisor).clamp(1, max_bars);
            let bar_w = if bars == max_bars {
                (width.saturating_sub(bars.saturating_sub(1) * min_gap) / bars)
                    .max(min_bar_w)
                    .min(width)
            } else {
                min_bar_w
            };
            let group_w = bars * bar_w + bars.saturating_sub(1) * min_gap;
            VizLayout {
                bars: vec![bar_w; bars],
                gaps: vec![min_gap; bars - 1],
                pad: viz_pad(width, group_w),
            }
        }
        VisualizerMode::Spread => {
            // Both set sizes are minimums; a pane narrower than one bar gets one cut bar.
            let n0 = (fixed_bars as usize).clamp(1, max_bars);
            let set_gap = min_gap as usize;
            let n = ((width + set_gap) / (min_bar_w + set_gap)).clamp(1, n0);
            let (bar_w, gap_w) = match anchor {
                SpreadAnchor::Gap => ((width - (n - 1) * set_gap) / n, set_gap),
                SpreadAnchor::Bar if n >= 2 => (min_bar_w, (width - n * min_bar_w) / (n - 1)),
                SpreadAnchor::Bar => (min_bar_w, 0),
            };
            let gap_count = n - 1;
            let mut layout =
                VizLayout { bars: vec![bar_w; n], gaps: vec![gap_w; gap_count], pad: 0 };
            layout.pad = viz_pad(width, layout.group_w());
            layout
        }
    }
}

/// Downsample cava's bands to the bars by bucket maximum, keeping peaks.
fn sample_visualizer_bars(frame: &[u8], bars: usize) -> Vec<u8> {
    if bars == 0 {
        return Vec::new();
    }
    if frame.is_empty() {
        return vec![0; bars];
    }
    let n = frame.len();
    (0..bars)
        .map(|b| {
            let start = b * n / bars;
            let end = ((b + 1) * n / bars).max(start + 1).min(n);
            frame[start..end].iter().copied().max().unwrap_or(0)
        })
        .collect()
}

fn draw_visualizer(f: &mut Frame, app: &App, area: Rect) {
    if area.width < 4 || area.height < 1 {
        return;
    }
    if !app.cava_alive {
        placeholder(f, area, &app.cava_note);
        return;
    }
    let w = area.width as usize;
    let h = area.height as usize;
    let mode = if app.visualizer_mode.eq_ignore_ascii_case("spread") {
        VisualizerMode::Spread
    } else {
        VisualizerMode::Adaptive
    };
    let anchor = if app.visualizer_anchor.eq_ignore_ascii_case("bar") {
        SpreadAnchor::Bar
    } else {
        SpreadAnchor::Gap
    };
    let layout = visualizer_layout(
        w,
        mode,
        anchor,
        app.visualizer_bar_width,
        app.visualizer_bar_gap,
        app.visualizer_bars,
        crate::playback::cava::BARS,
    );
    let bars = layout.bars.len();
    let pad = layout.pad;
    let frame = &app.cava_frame;
    let values = sample_visualizer_bars(frame, bars);
    let denom = (h.saturating_sub(1)).max(1) as f32;
    let ramp = theme::symbols().visualizer;

    let mut lines: Vec<Line> = Vec::with_capacity(h);
    for row in 0..h {
        let from_bottom = h - 1 - row;
        let color = theme::viz_color(from_bottom as f32 / denom);
        let mut s = " ".repeat(pad);
        for (b, v) in values.iter().enumerate() {
            let level = (*v as f32 / 255.0) * (h * 8) as f32;
            let fill = (level - from_bottom as f32 * 8.0).clamp(0.0, 8.0);
            let ch = if fill <= 0.0 { ' ' } else { ramp[(fill.ceil() as usize).min(8) - 1] };
            for _ in 0..layout.bars[b] {
                s.push(ch);
            }
            if let Some(&gap) = layout.gaps.get(b) {
                for _ in 0..gap {
                    s.push(' ');
                }
            }
        }
        lines.push(Line::from(Span::styled(s, Style::default().fg(color))));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// `theme.inset` cells of horizontal padding inside a bordered box; a box
/// too narrow to keep a column is left as it is.
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

/// Collapse whitespace and drop format characters that can spoof a row (bidi
/// controls, zero-widths, BOM). ZWJ and variation selectors stay.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.trim().chars() {
        if is_spoofing_format(c) {
            continue;
        }
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else if c.is_control() {
            continue;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// A format character that renders as nothing or reorders the text around it.
fn is_spoofing_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}'                  // soft hyphen
        | '\u{061C}'                // arabic letter mark
        | '\u{180E}'                // mongolian vowel separator
        | '\u{200B}'                // zero-width space
        | '\u{200E}' | '\u{200F}'   // left-to-right / right-to-left mark
        | '\u{202A}'..='\u{202E}'   // bidi embedding / override
        | '\u{2060}'..='\u{2064}'   // word joiner and invisible operators
        | '\u{2066}'..='\u{2069}'   // bidi isolates
        | '\u{FEFF}'                // BOM / zero-width no-break space
    )
}

struct HeaderSpec {
    label: &'static str,
    right: bool,
}

impl HeaderSpec {
    fn left(label: &'static str) -> Self {
        Self { label, right: false }
    }
    fn right(label: &'static str) -> Self {
        Self { label, right: true }
    }
}

/// A table's header, rule and rows area. Column widths are computed once here
/// so header text and row ellipsizing match the Table's own layout.
struct TableFrame {
    rows: Rect,
    col_w: Vec<u16>,
    bar: Option<Rect>,
}

fn table_frame(
    f: &mut Frame,
    pane: Rect,
    total: usize,
    cols: &[Constraint],
    headers: Option<&[HeaderSpec]>,
    indicator: Option<String>,
) -> TableFrame {
    let mut body = pane;
    if let Some(txt) = indicator {
        if body.height > 1 {
            let r = Rect { x: body.x, y: body.y, width: body.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(Span::styled(txt, theme::dim()))), r);
            body.y += 1;
            body.height -= 1;
        }
    }
    let header_rows = if headers.is_some() { 2 } else { 0 };
    let visible = body.height.saturating_sub(header_rows) as usize;
    let (content, bar) = list_split(body, total, visible);

    let col_w = layout_widths(cols, content.width);

    let mut y = content.y;
    if let Some(hs) = headers {
        if content.height >= 1 {
            let mut spans: Vec<Span> = Vec::new();
            for (i, h) in hs.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let w = col_w.get(i).copied().unwrap_or(0);
                // not dim: too faint for a header
                spans.push(Span::styled(header_cell(h.label, w, h.right), theme::text()));
            }
            let r = Rect { x: content.x, y, width: content.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(spans)), r);
            y += 1;
        }
        if content.height >= 2 {
            let r = Rect { x: content.x, y, width: content.width, height: 1 };
            if theme::spacing().borders != theme::BorderStyle::None {
                f.render_widget(
                    Paragraph::new("─".repeat(content.width as usize)).style(theme::dim()),
                    r,
                );
            }
            y += 1;
        }
    }

    let rows = Rect {
        x: content.x,
        y,
        width: content.width,
        height: content.height.saturating_sub(header_rows),
    };
    TableFrame { rows, col_w, bar }
}

/// Column widths as the Table lays them out (1-cell spacing).
fn layout_widths(cols: &[Constraint], width: u16) -> Vec<u16> {
    if width == 0 {
        return vec![0; cols.len()];
    }
    if cols.iter().any(|c| matches!(c, Constraint::Fill(_))) {
        return weighted_widths(cols, width);
    }
    let probe = Rect { x: 0, y: 0, width, height: 1 };
    Layout::horizontal(cols.to_vec()).spacing(1).split(probe).iter().map(|r| r.width).collect()
}

/// Widths for `Length`s and `Fill` weights: the fills share what is left, each
/// keeping `WEIGHTED_COL_MIN` when there is room.
fn weighted_widths(cols: &[Constraint], width: u16) -> Vec<u16> {
    let fixed: u32 = cols
        .iter()
        .map(|c| match c {
            Constraint::Length(l) => *l as u32,
            _ => 0,
        })
        .sum::<u32>()
        + cols.len().saturating_sub(1) as u32;
    let room = (width as u32).saturating_sub(fixed);
    let weights: Vec<u64> =
        cols.iter().map(|c| if let Constraint::Fill(w) = c { *w as u64 } else { 0 }).collect();
    let fills = weights.iter().filter(|w| **w > 0).count() as u32;
    let min = if room >= WEIGHTED_COL_MIN as u32 * fills { WEIGHTED_COL_MIN as u64 } else { 0 };

    let mut pinned = vec![false; cols.len()];
    loop {
        let free: u64 =
            (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).map(|i| weights[i]).sum();
        let pins = pinned.iter().filter(|p| **p).count() as u64;
        let space = (room as u64).saturating_sub(pins * min);
        let low = (0..cols.len())
            .find(|&i| weights[i] > 0 && !pinned[i] && space * weights[i] / free.max(1) < min);
        match low {
            Some(i) => pinned[i] = true,
            None => break,
        }
    }
    let free: u64 =
        (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).map(|i| weights[i]).sum();
    let pins = pinned.iter().filter(|p| **p).count() as u64;
    let space = (room as u64).saturating_sub(pins * min);
    let mut out: Vec<u16> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| match c {
            Constraint::Length(l) => (*l).min(width),
            _ if pinned[i] => min as u16,
            _ => (space * weights[i] / free.max(1)) as u16,
        })
        .collect();
    // Cells lost to rounding go to the widest shares first.
    let left = space.saturating_sub(
        (0..cols.len())
            .filter(|i| weights[*i] > 0 && !pinned[*i])
            .map(|i| out[i] as u64)
            .sum::<u64>(),
    );
    let mut by_share: Vec<usize> =
        (0..cols.len()).filter(|i| weights[*i] > 0 && !pinned[*i]).collect();
    by_share.sort_by_key(|i| std::cmp::Reverse(weights[*i]));
    for i in by_share.into_iter().cycle().take(left as usize) {
        out[i] += 1;
    }
    out
}

fn header_cell(s: &str, w: u16, right: bool) -> String {
    let t = theme::ellipsize(s, w);
    let tw = theme::str_width(&t) as u16;
    let pad = w.saturating_sub(tw) as usize;
    if right {
        format!("{}{}", " ".repeat(pad), t)
    } else {
        format!("{}{}", t, " ".repeat(pad))
    }
}

/// Ellipsized cell text, with the filter match highlighted.
fn cell(text: &str, w: u16, needle: &str, base: Style) -> Cell<'static> {
    let shown = theme::ellipsize(&clean(text), w);
    Cell::from(Line::from(spans_clean(shown, needle, base)))
}

fn spans_for(text: &str, needle: &str, base: Style) -> Vec<Span<'static>> {
    spans_clean(clean(text), needle, base)
}

fn spans_clean(text: String, needle: &str, base: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(text, base)];
    }
    filter::highlight(&text, needle, base, theme::secondary())
        .into_iter()
        .map(|s| Span::styled(s.content.into_owned(), s.style))
        .collect()
}

/// The `filter: text (n/total)` line; None while empty or typing (the footer
/// shows the input then).
fn indicator_text(f: &ListFilter, total: usize) -> Option<String> {
    if f.typing {
        None
    } else {
        f.indicator(total)
    }
}

/// Draw the filter indicator row and split off the scrollbar columns.
fn begin_list(
    f: &mut Frame,
    pane: Rect,
    total: usize,
    filter: &ListFilter,
    raw: usize,
) -> (Rect, Option<Rect>) {
    let mut body = pane;
    if let Some(ind) = indicator_text(filter, raw) {
        if body.height > 1 {
            let r = Rect { x: body.x, y: body.y, width: body.width, height: 1 };
            f.render_widget(Paragraph::new(Line::from(Span::styled(ind, theme::dim()))), r);
            body.y += 1;
            body.height -= 1;
        }
    }
    list_split(body, total, body.height as usize)
}

fn render_bar(f: &mut Frame, bar: Option<Rect>, total: usize, offset: usize, visible: usize) {
    if let Some(bar) = bar {
        render_scrollbar(f, bar, total, offset, visible);
    }
}

/// This frame's offset and visible rows for a list.
fn window(
    app: &mut App,
    id: ScrollId,
    cursor: Option<usize>,
    total: usize,
    area: Rect,
) -> (usize, usize) {
    let visible = area.height as usize;
    let offset = app.scroll.update(id, cursor.unwrap_or(0), visible, total, app.scrolloff);
    (offset, visible)
}

fn build_range(
    offset: usize,
    visible: usize,
    total: usize,
    mut push: impl FnMut(usize, &mut Vec<ListItem<'static>>),
) -> Vec<ListItem<'static>> {
    let end = (offset + visible).min(total);
    let mut items = Vec::new();
    for fi in offset..end {
        push(fi, &mut items);
    }
    items
}

fn render_list(
    f: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'static>>,
    cursor: Option<usize>,
    offset: usize,
    highlight: Style,
) {
    if area.width == 0 || area.height == 0 || items.is_empty() {
        return;
    }
    let mut state = ListState::default();
    if let Some(c) = cursor {
        state.select(Some(c.saturating_sub(offset).min(items.len() - 1)));
    }
    f.render_stateful_widget(List::new(items).highlight_style(highlight), area, &mut state);
}

/// Cursor style: full when the pane has focus, muted otherwise.
fn hl(focused: bool) -> Style {
    if focused {
        theme::current()
    } else {
        theme::current_unfocused()
    }
}

/// An active filter with no hits.
fn no_matches(f: &mut Frame, area: Rect) {
    placeholder(f, area, "No matches");
}

fn placeholder(f: &mut Frame, area: Rect, msg: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let line_area = Rect { x: area.x, y, width: area.width, height: 1 };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg.to_string(), theme::dim())))
            .alignment(Alignment::Center),
        line_area,
    );
}

/// The bordered "No cover" box.
fn placeholder_box(f: &mut Frame, rect: Rect, msg: &str) {
    if rect.width < 3 || rect.height < 3 || theme::spacing().borders == theme::BorderStyle::None {
        placeholder(f, rect, msg);
        return;
    }
    let block = Block::bordered().border_style(theme::border());
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let line_area =
        Rect { x: inner.x, y: inner.y + inner.height / 2, width: inner.width, height: 1 };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg.to_string(), theme::dim())))
            .alignment(Alignment::Center),
        line_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_weight_keeps_column_readable() {
        let cols = |w: [u16; 3]| {
            let mut v: Vec<Constraint> = w.iter().map(|w| Constraint::Fill(*w)).collect();
            v.push(Constraint::Length(5));
            v
        };
        for width in [30u16, 61, 100, 157] {
            let w = weighted_widths(&cols([1, 1, 98]), width);
            assert!(w[0] >= WEIGHTED_COL_MIN && w[1] >= WEIGHTED_COL_MIN, "{width}: {w:?}");
            // the gaps and the Len column are not shared out
            assert_eq!(w.iter().sum::<u16>() + 3, width, "{width}: {w:?}");
        }
        // too narrow for the minimum
        let w = weighted_widths(&cols([1, 1, 98]), 14);
        assert_eq!(w.iter().sum::<u16>() + 3, 14, "{w:?}");
    }

    #[test]
    fn visualizer_share_does_not_starve_queue() {
        assert_eq!(viz_height(50, 28), 14);
        assert_eq!(viz_height(50, 40), 20);
        // capped so the queue keeps 6 rows
        assert_eq!(viz_height(100, 28), 22);
        assert_eq!(viz_height(80, 10), 4);
        assert_eq!(viz_height(0, 28), 0);
        // the floor
        assert_eq!(viz_height(50, 17), 8);
        assert_eq!(viz_height(50, 14), 8);
        assert_eq!(viz_height(50, 13), 7);
        // tight: the 3-row floor
        assert_eq!(viz_height(50, 9), 3);
        // too tight: hidden
        assert_eq!(viz_height(50, 8), 0);
        assert_eq!(viz_height(50, 6), 0);
        assert_eq!(viz_height(50, 4), 0);
    }

    /// The box is centred exactly, with equal leftover on both sides of both
    /// axes, whatever the pane and cell size.
    #[test]
    fn placeholder_box_centred_in_cells() {
        let cells =
            [Some((8, 18)), Some((16, 36)), Some((22, 46)), Some((10, 20)), Some((9, 16)), None];
        for cell in cells {
            for width in 4..=80u16 {
                for height in 4..=40u16 {
                    let content = Rect { x: 3, y: 5, width, height };
                    let Some(r) = art_rect(content, cell) else { continue };
                    let what = format!("{cell:?} {width}x{height}");
                    assert_eq!(r.x - content.x, content.right() - r.right(), "{what}");
                    assert_eq!(r.y - content.y, content.bottom() - r.bottom(), "{what}");
                    assert!(r.width <= width && r.height <= height, "{what}");
                    let Some((cw, ch)) = cell else { continue };
                    let (pw, ph) = (r.width as u32 * cw as u32, r.height as u32 * ch as u32);
                    let one = cw.max(ch) as u32;
                    let side = (width as u32 * cw as u32).min(height as u32 * ch as u32);
                    assert!(pw.max(ph).abs_diff(side) <= one, "{what}: {pw}x{ph} vs {side}");
                    // Only a box that fills an axis can be forced off square.
                    if r.width < width && r.height < height {
                        assert!(pw.abs_diff(ph) <= one, "{what}: {pw}x{ph}");
                    }
                }
            }
        }
        // 16x36 px cells, 40x18 cells is 640x648 px.
        let r = art_rect(Rect { x: 0, y: 0, width: 40, height: 18 }, Some((16, 36))).expect("box");
        assert_eq!((r.width, r.height), (40, 18));

        // Without a cell size: 2:1.
        for (w, h, want) in
            [(44, 28, (44, 22)), (45, 40, (45, 24)), (30, 10, (20, 10)), (21, 12, (21, 12))]
        {
            let r = art_rect(Rect { x: 0, y: 0, width: w, height: h }, None).expect("box");
            assert_eq!((r.width, r.height), want, "{w}x{h}");
        }

        // too small
        assert_eq!(art_rect(Rect { x: 0, y: 0, width: 3, height: 10 }, None), None);
        assert_eq!(art_rect(Rect { x: 0, y: 0, width: 10, height: 1 }, None), None);
    }

    /// Spread mode, both anchors, every width.
    #[test]
    fn spread_layout_adaptive_with_cap() {
        let group = |n: usize, bar: usize, gap: usize| n * bar + (n - 1) * gap;
        for anchor in [SpreadAnchor::Bar, SpreadAnchor::Gap] {
            for cap in [1u16, 2, 3, 24, 96] {
                for w0 in 1..=6u16 {
                    for g0 in 0..=4u16 {
                        for w in 1..=300usize {
                            let l = visualizer_layout(
                                w,
                                VisualizerMode::Spread,
                                anchor,
                                w0,
                                g0,
                                cap,
                                96,
                            );
                            let case =
                                format!("{anchor:?} w={w} cap={cap} bar={w0} gap={g0} -> {l:?}");
                            let n = l.bars.len();
                            let (bar, gap) = (l.bars[0], l.gaps.first().copied().unwrap_or(0));
                            let (w0, g0, cap) = (w0 as usize, g0 as usize, cap as usize);
                            assert!(n >= 1 && l.gaps.len() == n - 1, "{case}");
                            assert!(l.bars.iter().all(|&b| b == bar), "{case}");
                            assert!(l.gaps.iter().all(|&x| x == gap), "{case}");
                            let used = l.group_w();
                            assert!(used <= w, "{case}");
                            assert!(l.pad.abs_diff(w - used - l.pad) <= 1, "{case}");
                            assert!(l.pad <= w - used - l.pad, "{case}");

                            if w < w0 {
                                // one cut bar
                                assert_eq!((n, bar), (1, w), "{case}");
                                continue;
                            }
                            let n_fit = (w + g0) / (w0 + g0);
                            assert_eq!(n, cap.min(n_fit), "{case}");
                            assert!(bar >= w0 && (n == 1 || gap >= g0), "{case}");
                            match anchor {
                                SpreadAnchor::Gap => {
                                    assert!(n == 1 || gap == g0, "{case}");
                                    assert!(group(n, bar + 1, gap) > w, "{case}");
                                }
                                SpreadAnchor::Bar => {
                                    assert_eq!(bar, w0, "{case}");
                                    if n > 1 {
                                        assert!(group(n, bar, gap + 1) > w, "{case}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// (bars, bar width, gap, left margin) by hand.
    #[test]
    fn spread_layout_by_example() {
        let cells = |w, anchor, bw, gap, count| {
            let l = visualizer_layout(w, VisualizerMode::Spread, anchor, bw, gap, count, 96);
            (l.bars.len(), l.bars[0], l.gaps.first().copied().unwrap_or(0), l.pad)
        };
        // 24 bars of 3 with gaps of 2 need 118: 115 shows 23, not thinner ones
        assert_eq!(cells(115, SpreadAnchor::Gap, 3, 2, 24), (23, 3, 2, 1));
        assert_eq!(cells(115, SpreadAnchor::Bar, 3, 2, 24), (23, 3, 2, 1));
        assert_eq!(cells(118, SpreadAnchor::Gap, 3, 2, 24), (24, 3, 2, 0));
        assert_eq!(cells(160, SpreadAnchor::Gap, 3, 2, 24), (24, 4, 2, 9));
        assert_eq!(cells(160, SpreadAnchor::Bar, 3, 2, 24), (24, 3, 3, 9));
        assert_eq!(cells(300, SpreadAnchor::Gap, 2, 1, 20), (20, 14, 1, 0));
        assert_eq!(cells(300, SpreadAnchor::Bar, 2, 1, 20), (20, 2, 13, 6));
        // a lone bar
        assert_eq!(cells(30, SpreadAnchor::Gap, 2, 1, 1), (1, 30, 0, 0));
        assert_eq!(cells(30, SpreadAnchor::Bar, 2, 1, 1), (1, 2, 0, 14));
        assert_eq!(cells(2, SpreadAnchor::Bar, 5, 1, 8), (1, 2, 0, 0));
    }

    /// `m:ss` and `mm:ss` share a width; an hour-long track widens it just enough.
    #[test]
    fn len_column_as_wide_as_its_times() {
        assert_eq!(len_col_width(None), theme::LEN_COL_MIN);
        assert_eq!(len_col_width(Some(257_293)), 5); // 4:17
        assert_eq!(len_col_width(Some(2_520_000)), 5); // 42:00
        assert_eq!(len_col_width(Some(3_725_000)), 7); // 1:02:05
        assert_eq!(len_col_width(Some(36_000_000)), 8); // 10:00:00
    }

    /// (bar count, bar width, gap, margin) at several widths.
    #[test]
    fn adaptive_layout_at_several_widths() {
        let cells = |w, bw, gap| {
            let l =
                visualizer_layout(w, VisualizerMode::Adaptive, SpreadAnchor::Gap, bw, gap, 20, 96);
            let n = l.bars.len();
            assert!(
                l.bars.iter().all(|&b| b == l.bars[0]) && l.gaps.iter().all(|&g| g == l.gaps[0])
            );
            (n, l.bars[0], l.gaps.first().copied().unwrap_or(0), l.pad)
        };
        assert_eq!(cells(40, 2, 1), (13, 2, 1, 1));
        assert_eq!(cells(58, 2, 1), (19, 2, 1, 1));
        assert_eq!(cells(100, 2, 1), (33, 2, 1, 1));
        assert_eq!(cells(250, 2, 1), (83, 2, 1, 1));
        assert_eq!(cells(297, 2, 1), (96, 2, 1, 5));
        assert_eq!(cells(400, 2, 1), (96, 3, 1, 8));
        assert_eq!(cells(100, 3, 2), (20, 3, 2, 1));
        assert_eq!(cells(4, 3, 2), (1, 3, 0, 0));
        assert_eq!(cells(10, 1, 0), (10, 1, 0, 0));
    }

    #[test]
    fn visualizer_sampling_preserves_peaks() {
        let frame: Vec<u8> = (0..96).collect();
        assert_eq!(sample_visualizer_bars(&frame, 4), vec![23, 47, 71, 95]);
        assert_eq!(sample_visualizer_bars(&[], 3), vec![0, 0, 0]);
    }

    #[test]
    fn volume_meter_fills_every_cell_at_full() {
        assert_eq!(volume_fill(0.0), 0);
        assert_eq!(volume_fill(50.0), 4);
        assert_eq!(volume_fill(100.0), METER_CELLS);
    }

    #[test]
    fn square_art_width_matches_the_cell_aspect() {
        assert_eq!(square_art_width(30, (10, 20)), 60);
        assert_eq!(square_art_width(30, (10, 10)), 30);
        assert_eq!(square_art_width(0, (10, 20)), 0);
    }

    /// The opaque pixels' gaps to the canvas edge (left, right, top, bottom) and size.
    fn cover_gaps(canvas: &DynamicImage) -> ([u32; 4], (u32, u32)) {
        let rgba = canvas.to_rgba8();
        let (w, h) = rgba.dimensions();
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
        for (x, y, p) in rgba.enumerate_pixels() {
            if p[3] > 0 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        assert!(x0 <= x1 && y0 <= y1, "the cover is on the canvas");
        ([x0, w - 1 - x1, y0, h - 1 - y1], (x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Opposite gaps differ by at most a pixel, the limiting axis has the
    /// padding, and the cap holds.
    #[test]
    fn cover_centred_in_pixels_at_every_size() {
        let covers = [(300u32, 300u32), (400, 200), (200, 400), (300, 290)];
        for cell in [(8u16, 18u16), (16, 36), (22, 46)] {
            // both parities of both axes, without a slow sweep
            let pads: &[u16] = if cell == (16, 36) { &[1, 3] } else { &[1] };
            for &pad in pads {
                for width in [20u16, 25, 31, 36, 42, 47, 53, 58, 64, 69] {
                    for height in [8u16, 11, 15, 18, 22, 25, 29, 32] {
                        for (iw, ih) in covers {
                            for cap in [[0u32, 0u32], [200, 300]] {
                                let tag = format!(
                                    "cell {cell:?} pad {pad} pane {width}x{height} cover {iw}x{ih} cap {cap:?}"
                                );
                                let area = Rect::new(0, 0, width, height);
                                let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                                    iw,
                                    ih,
                                    image::Rgb([9, 9, 9]),
                                ));
                                let canvas = centered_art_canvas(
                                    &img,
                                    area,
                                    cell,
                                    pad,
                                    cap,
                                    image::Rgba([0, 0, 0, 0]),
                                )
                                .expect("canvas");
                                assert_eq!(
                                    (canvas.width(), canvas.height()),
                                    (width as u32 * cell.0 as u32, height as u32 * cell.1 as u32),
                                    "{tag}"
                                );
                                let ([l, r, t, b], (w, h)) = cover_gaps(&canvas);
                                assert!(l.abs_diff(r) <= 1, "horizontal {l}/{r}: {tag}");
                                assert!(t.abs_diff(b) <= 1, "vertical {t}/{b}: {tag}");
                                if cap == [0, 0] {
                                    let pad = pad as u32;
                                    let wide = l.abs_diff(pad) <= 1 && r.abs_diff(pad) <= 1;
                                    let tall = t.abs_diff(pad) <= 1 && b.abs_diff(pad) <= 1;
                                    assert!(wide || tall, "limiting axis has the padding: {tag}");
                                } else {
                                    assert!(
                                        w <= cap[0] && h <= cap[1],
                                        "{w}x{h} over the cap: {tag}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// With a theme background the letterbox is that colour, opaque; without
    /// one it stays transparent.
    #[test]
    fn cover_padding_takes_theme_background() {
        let _palette = theme::palette_test_lock();
        let img =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(100, 50, image::Rgb([9, 9, 9])));
        let area = Rect::new(0, 0, 10, 10);
        let canvas =
            |fill| centered_art_canvas(&img, area, (8, 16), 1, [0, 0], fill).unwrap().to_rgba8();
        // a wide cover in a tall pane: the top-left pixel is padding
        assert_eq!(canvas(art_padding_fill()).get_pixel(0, 0).0[3], 0);
        let mut palette = theme::theme();
        palette.background = Some(ratatui::style::Color::Rgb(0xef, 0xf1, 0xf5));
        let previous = theme::theme();
        theme::set_theme(palette);
        let fill = art_padding_fill();
        theme::set_theme(previous);
        assert_eq!(fill.0, [0xef, 0xf1, 0xf5, 255]);
        let with = canvas(fill);
        assert_eq!(with.get_pixel(0, 0).0, [0xef, 0xf1, 0xf5, 255]);
        assert_eq!(with.get_pixel(40, 80).0, [9, 9, 9, 255], "the cover itself is untouched");
    }

    #[test]
    fn audio_info_formats_long_and_compact() {
        let info = |codec: &str, rate: Option<u32>, depth: Option<u32>, kbps: Option<u32>| {
            crate::plex::AudioInfo {
                codec: Some(codec.into()),
                sample_rate_hz: rate,
                bit_depth: depth,
                bitrate_kbps: kbps,
                ..Default::default()
            }
        };
        let fmt = |a| format_audio(&a);
        let both = |l: &str, c: &str| (l.to_string(), c.to_string());
        assert_eq!(
            fmt(info("flac", Some(44100), Some(16), None)),
            both("FLAC 44.1kHz/16bit", "FLAC 44.1k/16")
        );
        assert_eq!(
            fmt(info("flac", Some(96000), Some(24), None)),
            both("FLAC 96kHz/24bit", "FLAC 96k/24")
        );
        // the compact lossy form drops the rate
        assert_eq!(
            fmt(info("mp3", Some(44100), None, Some(320))),
            both("MP3 320kbps/44.1kHz", "MP3 320k")
        );
        assert_eq!(fmt(info("mp3", None, None, Some(128))), both("MP3 128kbps", "MP3 128k"));
        assert_eq!(fmt(info("flac", None, Some(16), None)), both("FLAC 16bit", "FLAC 16bit"));
        assert_eq!(fmt(info("ogg", Some(48000), None, None)), both("OGG 48kHz", "OGG 48k"));
        assert_eq!(fmt(info("wav", None, None, None)), both("WAV", "WAV"));
        assert_eq!(fmt(crate::plex::AudioInfo::default()), both("", ""));
        assert_eq!(format_rate(44100), "44.1k");
        assert_eq!(format_rate(48000), "48k");
        assert_eq!(format_rate(22050), "22.05k");
    }

    #[test]
    fn header_times_steps_down_to_what_fits() {
        let (long, compact) = ("FLAC 44.1kHz/16bit", "FLAC 44.1k/16");
        let t = "0:42 / 4:17";
        let long_full = "0:42 / 4:17 (FLAC 44.1kHz/16bit)"; // 32 cells
        let compact_full = "0:42 / 4:17 (FLAC 44.1k/16)"; // 27 cells
        assert_eq!(header_times(t, long, compact, 32), long_full);
        assert_eq!(header_times(t, long, compact, 31), compact_full);
        assert_eq!(header_times(t, long, compact, 27), compact_full);
        assert_eq!(header_times(t, long, compact, 26), t);
        assert_eq!(header_times(t, long, compact, 11), t);
        assert_eq!(header_times(t, long, compact, 6), "0:42 …");
        assert_eq!(header_times(t, "", "", 40), t);
        // wide characters count in cells (15 chars, 16 cells)
        assert_eq!(header_times(t, "音声", "音", 15), t);
        assert_eq!(header_times(t, "音声", "音", 16), format!("{t} (音)"));
    }

    #[test]
    fn clean_drops_spoofing_format_characters() {
        assert_eq!(clean("abc\u{202E}def"), "abcdef");
        assert_eq!(clean("a\u{200B}b"), "ab");
        assert_eq!(clean("a\u{FEFF}b"), "ab");
        // emoji and shaping controls survive
        assert_eq!(clean("a\u{200D}b\u{FE0F}"), "a\u{200D}b\u{FE0F}");
        assert_eq!(clean("a\u{200C}b"), "a\u{200C}b");
    }
}
