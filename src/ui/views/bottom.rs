use super::draw_block_as;
use super::widgets::clean;
use crate::ui::app::{App, Tab};
use crate::ui::icons;
use crate::ui::keymap::Act;
use crate::ui::theme;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// `Title - Artist`, ellipsized to `width`: the title bold, the artist secondary.
/// The rating follows the title while the title fits whole beside it.
pub(super) fn now_playing(app: &App, width: u16) -> Option<Line<'static>> {
    let t = app.current_track()?;
    let (title, artist) = (clean(&t.title), clean(&t.artist));
    if let Some(rating) = app.rating_label() {
        let used = theme::str_width(&title) + 1 + theme::str_width(&rating) + 3;
        if let Some(room) = (width as usize).checked_sub(used).filter(|r| *r > 0) {
            return Some(Line::from(vec![
                Span::styled(title, theme::text().add_modifier(Modifier::BOLD)),
                Span::styled(" ", theme::text()),
                Span::styled(rating, theme::accent()),
                Span::styled(" - ", theme::text()),
                Span::styled(theme::ellipsize(&artist, room as u16), theme::secondary()),
            ]));
        }
    }
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
pub(super) fn draw_rule_title(f: &mut Frame, app: &App, rule: Rect) {
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

pub(super) fn draw_bottom(f: &mut Frame, app: &mut App, area: Rect, titled: bool) {
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

    if let Some(line) = adventure_line(app, inner.width) {
        f.render_widget(
            Paragraph::new(line).style(theme::secondary()).alignment(Alignment::Center),
            inner,
        );
        return;
    }

    draw_progress(f, app, inner);
}

/// The stops line, at most `width` cells wide; the titles shrink before the hint does.
fn adventure_line(app: &App, width: u16) -> Option<String> {
    let stops = &app.adventure.stops;
    if stops.is_empty() {
        return None;
    }
    let ctx = app.key_ctx();
    let mut tail = String::new();
    for (act, what) in [(Act::BuildAdventure, "builds"), (Act::Back, "clears")] {
        if let Some(key) = app.keymap.first_key(ctx, &act) {
            tail.push_str(&format!(" · {key} {what}"));
        }
    }
    let head = "Sonic Adventure: ";
    let room = (width as usize).saturating_sub(theme::str_width(head) + theme::str_width(&tail));
    let titles: Vec<String> = stops.iter().map(|s| clean(&s.title)).collect();
    let joined = |cap: u16| {
        let shown: Vec<String> = titles.iter().map(|t| theme::ellipsize(t, cap)).collect();
        shown.join(" → ")
    };
    let widest = titles.iter().map(|t| theme::str_width(t)).max().unwrap_or(0) as u16;
    let list = (1..=widest)
        .rev()
        .map(joined)
        .find(|l| theme::str_width(l) <= room)
        .unwrap_or_else(|| theme::ellipsize(&joined(widest), room as u16));
    Some(theme::ellipsize(&format!("{head}{list}{tail}"), width))
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
