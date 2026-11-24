use super::library::{Choice, Libraries};
use super::save::review_line;
use super::{Editor, QuitAsk, Row, Screen, Tab};
use crate::config::SettingRow;
use crate::doctor::{self};
use crate::inventory::Kind;
use crate::ui::theme;
use crate::ui::themes;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

impl Screen {
    pub(super) fn draw(&mut self, f: &mut Frame) {
        self.poll_check();
        self.poll_library();
        let area = f.area();
        theme::paint_background(f.buffer_mut(), area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                // keys + status; a one-row box would clip the status.
                Constraint::Length(4),
            ])
            .split(area);

        let changed = self.cfg.changed_keys(&self.saved);
        let names: Vec<String> = Tab::ALL
            .iter()
            .map(|t| match Self::changes_on(*t, &changed) {
                0 => t.name().to_string(),
                n => format!("{} •{n}", t.name()),
            })
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let active = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        crate::ui::views::draw_tabs(f, chunks[0], &names, active);

        let panes = if self.tab == Tab::Theme {
            // Blocks like the player's, so the gap is `theme.block_gap`.
            let gap = theme::spacing().block_gap;
            let rows = preview_rows(&self.cfg.ui);
            let preview_h = if chunks[1].height >= THEME_SAMPLE_MIN_HEIGHT + gap {
                rows.header + rows.rule + gap + rows.sample
            } else {
                rows.header
            };
            let theme = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(preview_h), Constraint::Min(3)])
                .split(chunks[1]);
            draw_theme_preview(f, theme[0], &self.cfg.ui);
            theme[1]
        } else {
            chunks[1]
        };
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Min(20)])
            .split(panes);

        match self.tab {
            Tab::Diagnostics => self.draw_diagnostics(f, chunks[1]),
            _ => {
                let values = self.values();
                self.draw_list(f, body[0], &values);
                self.draw_help(f, body[1], &values);
            }
        }
        if self.review.is_some() {
            self.draw_review(f, chunks[1]);
        }
        self.draw_footer(f, chunks[2]);
    }

    fn draw_review(&mut self, f: &mut Frame, body: Rect) {
        let Some(review) = &mut self.review else {
            return;
        };
        let width = body.width.min(90);
        let n = review.rows.len();
        let area = Rect {
            x: body.x + (body.width - width) / 2,
            width,
            height: ((n + 2).min(body.height as usize)).max(3.min(body.height as usize)) as u16,
            ..body
        };
        f.render_widget(ratatui::widgets::Clear, area);
        theme::paint_background(f.buffer_mut(), area);
        let block = Block::bordered().border_style(theme::border()).title(Span::styled(
            format!(" save {n} change{}? ", if n == 1 { "" } else { "s" }),
            theme::text(),
        ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width < 2 || inner.height == 0 {
            return;
        }
        let rows_h = inner.height as usize;
        review.visible = rows_h;
        review.scroll = review.scroll.min(n.saturating_sub(rows_h));
        let text_w = inner.width.saturating_sub(1) as usize;
        let lines: Vec<Line> = review
            .rows
            .iter()
            .skip(review.scroll)
            .take(rows_h)
            .map(|(key, old, new, secret)| {
                Line::from(Span::styled(review_line(key, old, new, *secret, text_w), theme::text()))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), Rect { width: inner.width - 1, ..inner });
        crate::ui::scrollbar::render_scrollbar(f, inner, n, review.scroll, rows_h);
    }

    fn draw_list(
        &mut self,
        f: &mut Frame,
        area: ratatui::layout::Rect,
        values: &std::collections::BTreeMap<String, SettingRow>,
    ) {
        let view = self.view();
        let rows = self.rows(&view);
        let total = rows.len();

        let title = match self.tab {
            Tab::Theme => " Theme ",
            _ => " Settings ",
        };
        let block = ratatui::widgets::Block::bordered()
            .border_style(theme::border())
            .title(Span::styled(title, theme::text()))
            .title_bottom(Span::styled(" * changed from default ", theme::dim()));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // The last column is the scrollbar's.
        let list_area = ratatui::layout::Rect { width: inner.width.saturating_sub(1), ..inner };
        // By the tab's longest label, not the filtered ones, so a filter does not move it.
        let longest = self.visible().iter().map(|s| theme::str_width(s.label)).max().unwrap_or(0);
        let avail = list_area.width as usize;
        let label_w = longest
            .min(avail.saturating_sub(LIST_LEAD + LIST_GAP + VALUE_MIN).max(LABEL_MIN))
            .min(avail.saturating_sub(LIST_LEAD + LIST_GAP));
        let value_w = avail.saturating_sub(LIST_LEAD + label_w + LIST_GAP);

        let on_cursor = self.cursor.min(view.len().saturating_sub(1));
        let items: Vec<ListItem> = if view.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(" no settings match", theme::dim())))]
        } else {
            rows.iter()
                .map(|row| match row {
                    Row::Header(group) => ListItem::new(Line::from(Span::styled(
                        format!(" {group}"),
                        theme::accent().add_modifier(Modifier::BOLD),
                    ))),
                    Row::Item(i) => {
                        let setting = view[*i];
                        let current = values
                            .get(setting.key)
                            .map(|row| row.current.clone())
                            .unwrap_or_default();
                        let marker = if self.is_changed(setting, values) { " *" } else { "" };
                        // The cursor's row keeps its own style so it stays readable.
                        let unused =
                            *i != on_cursor && self.unused_because(setting, values).is_some();
                        let body = if unused { theme::dim() } else { theme::text() };
                        let label = theme::ellipsize(setting.label, label_w as u16);
                        let pad = label_w.saturating_sub(theme::str_width(&label));
                        let mut spans = vec![Span::styled(
                            format!(
                                "{}{label}{}",
                                " ".repeat(LIST_LEAD),
                                " ".repeat(pad + LIST_GAP)
                            ),
                            body,
                        )];
                        match (setting.kind, theme::parse_hex(&current)) {
                            (Kind::Color, Some(colour)) => {
                                spans.push(Span::styled("███ ", Style::default().fg(colour)));
                                spans.push(Span::styled(current, theme::dim()));
                            }
                            _ => {
                                let room = value_w.saturating_sub(marker.len());
                                spans.push(Span::styled(
                                    theme::ellipsize(&current, room as u16),
                                    body,
                                ));
                            }
                        }
                        spans.push(Span::styled(marker, theme::secondary()));
                        ListItem::new(Line::from(spans))
                    }
                })
                .collect()
        };

        let visible = inner.height as usize;
        // A group's first row keeps its heading in view.
        let cursor = self.cursor.min(view.len().saturating_sub(1));
        let at = rows.iter().position(|r| *r == Row::Item(cursor)).unwrap_or(0);
        let heading = at > 0 && matches!(rows[at - 1], Row::Header(_));
        let mut offset = self.scroll.update(self.scroll_id(), at, visible, total, self.scrolloff);
        if heading && visible >= 2 && offset > at - 1 {
            offset = at - 1;
            self.scroll.set_offset(self.scroll_id(), offset);
        }
        let mut state = ListState::default()
            .with_offset(offset)
            .with_selected(if view.is_empty() { None } else { Some(at) });
        f.render_stateful_widget(
            List::new(items).highlight_style(theme::current()),
            list_area,
            &mut state,
        );
        crate::ui::scrollbar::render_scrollbar(f, inner, total, offset, visible);
    }

    fn draw_help(
        &mut self,
        f: &mut Frame,
        area: ratatui::layout::Rect,
        values: &std::collections::BTreeMap<String, SettingRow>,
    ) {
        let block = ratatui::widgets::Block::bordered()
            .border_style(theme::border())
            .title(Span::styled(" details ", theme::text()));
        let mut lines: Vec<Line> = Vec::new();
        let width = (area.width as usize).saturating_sub(3);
        let input_line = |text: &str, cursor: usize, width: usize, end: &'static str| {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(input_spans(text, cursor, width, end, Style::default()));
            Line::from(spans)
        };

        if let Some(editor) = &self.editor {
            match editor {
                Editor::Picker { options, index, themes } => {
                    lines.push(Line::from(Span::styled("choose", theme::secondary())));
                    // Windowed to leave room for the theme details below.
                    let details = if themes.is_empty() { 0 } else { 6 };
                    let rows =
                        (area.height.saturating_sub(2) as usize).saturating_sub(1 + details).max(1);
                    let start =
                        index.saturating_sub(rows / 2).min(options.len().saturating_sub(rows));
                    for (i, option) in options.iter().enumerate().skip(start).take(rows) {
                        let shown = if option.is_empty() {
                            "(default)".to_string()
                        } else {
                            option.clone()
                        };
                        let style = if i == *index { theme::current() } else { theme::text() };
                        lines.push(Line::from(Span::styled(format!(" {shown} "), style)));
                    }
                    if let Some(id) = options.get(*index).filter(|_| !themes.is_empty()) {
                        lines.push(Line::from(""));
                        lines.extend(theme_details(id, themes));
                    }
                }
                Editor::Library(pick) => {
                    lines.push(Line::from(Span::styled(
                        "choose a music library",
                        theme::secondary(),
                    )));
                    match &pick.libraries {
                        Libraries::Loading => {
                            lines.push(Line::from(Span::styled(" loading…", theme::dim())));
                        }
                        Libraries::Failed(reason) => {
                            lines.push(Line::from(Span::styled(
                                format!(" {reason}"),
                                theme::error(),
                            )));
                        }
                        Libraries::Loaded(_) => {}
                    }
                    let choices = pick.choices();
                    let used = lines.len();
                    let rows = (area.height.saturating_sub(2) as usize).saturating_sub(used).max(1);
                    let start = window_start(choices.len(), pick.index, rows);
                    for (i, choice) in choices.iter().enumerate().skip(start).take(rows) {
                        let shown = match choice {
                            Choice::Id(_, label) => label.clone(),
                            Choice::Type => "type an id…".to_string(),
                        };
                        let style = if i == pick.index { theme::current() } else { theme::text() };
                        lines.push(Line::from(Span::styled(format!(" {shown} "), style)));
                    }
                }
                Editor::List(list) => {
                    let key = self.selected().map(|s| s.key);
                    let hint = if key == Some("ui.queue_columns") {
                        "one column per row"
                    } else {
                        "one argument per row"
                    };
                    lines.push(Line::from(Span::styled(hint, theme::secondary())));
                    if key == Some("mpv.extra_args") {
                        lines.push(Line::from(Span::styled(
                            " a --log-file here is dropped",
                            theme::dim(),
                        )));
                    }
                    if list.rows.is_empty() {
                        lines.push(Line::from(Span::styled(" (none; a adds one)", theme::dim())));
                    }
                    let rows =
                        (area.height.saturating_sub(2) as usize).saturating_sub(lines.len()).max(1);
                    let start = window_start(list.rows.len(), list.cursor, rows);
                    for (i, row) in list.rows.iter().enumerate().skip(start).take(rows) {
                        match &list.editing {
                            Some((input, _)) if i == list.cursor => {
                                lines.push(input_line(input.text(), input.cursor(), width, "_"));
                            }
                            _ => {
                                let style =
                                    if i == list.cursor { theme::current() } else { theme::text() };
                                let shown = theme::ellipsize(row, width.max(1) as u16);
                                lines.push(Line::from(Span::styled(format!(" {shown} "), style)));
                            }
                        }
                    }
                }
                Editor::Export { text } => {
                    lines.push(Line::from(Span::styled("export the theme as", theme::secondary())));
                    lines.push(input_line(text.text(), text.cursor(), width, "_"));
                    lines.push(Line::from(Span::styled(
                        " saved as themes/<id>.toml; a-z 0-9 - _",
                        theme::dim(),
                    )));
                }
                Editor::Field { text, secret, .. } => {
                    lines.push(Line::from(Span::styled("type, Enter to keep", theme::secondary())));
                    if *secret {
                        // Masked even while typed: scrollback and screenshots keep it.
                        if text.is_empty() {
                            lines.push(Line::from(Span::raw(
                                "(empty; Enter keeps the current value)",
                            )));
                        } else {
                            let mask = "•".repeat(text.char_count());
                            lines.push(input_line(&mask, text.cursor(), width, "_"));
                        }
                    } else {
                        lines.push(input_line(text.text(), text.cursor(), width, "_"));
                    }
                    if let Some(setting) = self.selected().filter(|s| s.kind == Kind::Symbol) {
                        let rule = theme::symbol_rule(setting.key.trim_start_matches("theme."));
                        lines.push(Line::from(Span::styled(format!(" {rule}"), theme::dim())));
                    }
                    if self.selected().map(|s| s.kind) == Some(Kind::Color) {
                        let hint = if self.selected().map(|s| s.key) == Some("theme.background") {
                            " #rrggbb, none, or an ANSI name:"
                        } else {
                            " #rrggbb, or an ANSI name:"
                        };
                        lines.push(Line::from(Span::styled(hint, theme::dim())));
                        lines.push(Line::from(Span::styled(
                            " black red green yellow blue magenta cyan white",
                            theme::dim(),
                        )));
                    }
                }
            }
        } else if let Some(setting) = self.selected() {
            lines.push(Line::from(Span::styled(setting.label, theme::secondary())));
            lines.push(Line::from(Span::styled(setting.help, theme::text())));
            if setting.next_start() {
                let note = "A running player applies it on the next start.";
                lines.push(Line::from(Span::styled(note, theme::dim())));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(format!("key     {}", setting.key), theme::dim())));
            let row = values.get(setting.key);
            let current = row.map(|r| r.current.clone()).unwrap_or_default();
            let default =
                row.and_then(|r| r.default.clone()).unwrap_or_else(|| "no default".to_string());
            lines.push(Line::from(Span::styled(format!("value   {current}"), theme::text())));
            lines.push(Line::from(Span::styled(format!("default {default}"), theme::text())));
            if let Some(reason) = self.unused_because(setting, values) {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(reason, theme::dim())));
            }
        } else {
            lines.push(Line::from(Span::styled("nothing selected", theme::dim())));
        }

        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    }

    fn draw_diagnostics(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let block = Block::bordered()
            .border_style(theme::border())
            .title(Span::styled(" Checks ", theme::text()));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let lines = self.report_lines();
        let total = lines.len();
        // A check can take most of a minute, so the hint is always visible.
        let (hint, hint_style) = if self.checking {
            ("checking …", theme::secondary())
        } else if self.report.is_some() {
            ("press d to run the checks again", theme::dim())
        } else {
            ("press d to run the checks", theme::dim())
        };
        let report_h = inner.height.saturating_sub(1) as usize;
        let report_area = Rect { height: report_h as u16, ..inner };
        let hint_area = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(Paragraph::new(Line::from(Span::styled(hint, hint_style))), hint_area);
        // Recorded before the early return so a later page does not use a stale height.
        self.diag_scroll = self.diag_scroll.min(total.saturating_sub(report_h));
        self.diag_visible = report_h;
        self.diag_max = total.saturating_sub(report_h);
        if report_h == 0 {
            return;
        }

        let offset = self.diag_scroll;
        let shown: Vec<Line> = lines.into_iter().skip(offset).take(report_h).collect();
        f.render_widget(
            Paragraph::new(shown).wrap(Wrap { trim: false }),
            Rect { width: report_area.width.saturating_sub(1), ..report_area },
        );
        crate::ui::scrollbar::render_scrollbar(f, report_area, total, offset, report_h);
    }

    fn report_lines(&self) -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();
        if let Some(error) = &self.check_error {
            lines.push(Line::from(Span::styled(error.clone(), theme::error())));
            return lines;
        }
        match &self.report {
            None => {}
            Some(report) => {
                lines.push(Line::from(Span::styled("config", theme::secondary())));
                for entry in &report.settings {
                    entry_lines(&mut lines, entry, false);
                }
                for section in &report.sections {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(section.title, theme::secondary())));
                    for entry in &section.entries {
                        entry_lines(&mut lines, entry, true);
                    }
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(report.summary(), theme::secondary())));
            }
        }
        lines
    }

    fn draw_footer(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let block = ratatui::widgets::Block::bordered()
            .border_style(theme::border())
            .title(Span::styled(" keys ", theme::text()));
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut spans: Vec<Span> = Vec::new();
        let keys: &[(&str, &str)] = if self.review.is_some() {
            &[("s/⏎", "save"), ("j/k", "scroll"), ("Esc", "back")]
        } else if self.conflict_ask.is_some() {
            &[("o", "keep mine"), ("t", "take the file's"), ("Esc", "cancel")]
        } else if self.restore_ask.is_some() {
            &[("y", "restore"), ("n", "cancel")]
        } else if let Some(ask) = self.quit_ask {
            match ask {
                QuitAsk::Clean => &[("y", "quit"), ("n", "stay")],
                QuitAsk::Dirty => {
                    &[("s", "save and quit"), ("y", "quit without saving"), ("Esc", "stay")]
                }
            }
        } else if self.filter.typing {
            &[("⏎", "keep"), ("Esc", "clear")]
        } else if matches!(self.editor, Some(Editor::Export { .. })) {
            &[("⏎", "export"), ("Esc", "cancel")]
        } else if matches!(self.editor, Some(Editor::Library(_))) {
            &[("↑↓", "choose"), ("⏎", "pick"), ("t", "type an id"), ("Esc", "cancel")]
        } else if let Some(Editor::List(list)) = &self.editor {
            if list.editing.is_some() {
                &[("⏎", "keep"), ("Esc", "cancel")]
            } else {
                &[("⏎", "edit"), ("a", "add"), ("d", "delete"), ("J/K", "move"), ("Esc", "done")]
            }
        } else if self.editor.is_some() {
            &[("↑↓", "choose"), ("⏎", "keep"), ("Esc", "cancel")]
        } else if self.tab == Tab::Theme {
            &[
                ("↑↓", "move"),
                ("⏎", "edit"),
                ("←→", "change"),
                ("x", "export"),
                ("/", "filter"),
                ("⇥", "tab"),
                ("s", "save"),
                ("r", "reset"),
                ("R", "restore tab"),
                ("q", "quit"),
            ]
        } else {
            &[
                ("↑↓", "move"),
                ("⏎", "edit"),
                ("←→", "change"),
                ("/", "filter"),
                ("[ ]", "groups"),
                ("⇥", "tab"),
                ("s", "save"),
                ("r", "reset"),
                ("R", "restore tab"),
                ("d", "checks"),
                ("q", "quit"),
            ]
        };
        let mut keys: Vec<(&str, &str)> = keys.to_vec();
        let width = |keys: &[(&str, &str)]| {
            keys.iter()
                .map(|(k, a)| theme::str_width(k) + 1 + theme::str_width(a) + 2)
                .sum::<usize>()
                .saturating_sub(2)
        };
        for drop in KEYS_DROP_ORDER {
            if width(&keys) <= inner.width as usize || keys.len() <= 1 {
                break;
            }
            keys.retain(|(k, _)| **k != *drop);
        }
        for (i, (key, action)) in keys.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(*key, theme::secondary()));
            spans.push(Span::styled(format!(" {action}"), theme::text()));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            ratatui::layout::Rect { height: 1, ..inner },
        );

        // A status or question takes this row over the filter.
        if inner.height > 1 && self.filter.typing && self.status.is_empty() {
            let counter =
                format!("{}/{}", self.filter.len(self.visible().len()), self.visible().len());
            let cw = theme::str_width(&counter) as u16 + 1;
            let cols = Layout::horizontal([Constraint::Min(4), Constraint::Length(cw)])
                .split(Rect { y: inner.y + 1, height: 1, ..inner });
            f.render_widget(
                Paragraph::new(Line::from(
                    vec![Span::styled("/ ", theme::secondary())]
                        .into_iter()
                        .chain(input_spans(
                            &self.filter.text,
                            self.filter_cursor,
                            (cols[0].width as usize).saturating_sub(2),
                            "█",
                            theme::text(),
                        ))
                        .collect::<Vec<_>>(),
                )),
                cols[0],
            );
            f.render_widget(
                Paragraph::new(counter).style(theme::dim()).alignment(Alignment::Right),
                cols[1],
            );
        } else if inner.height > 1
            && self.restore_ask.is_none()
            && self.status.is_empty()
            && self.filter.is_active()
        {
            let text = self.filter.indicator(self.visible().len()).unwrap_or_default();
            f.render_widget(
                Paragraph::new(Line::from(text)).style(theme::dim()).alignment(Alignment::Center),
                Rect { y: inner.y + 1, height: 1, ..inner },
            );
        } else if inner.height > 1 && (self.restore_ask.is_some() || !self.status.is_empty()) {
            let text = match self.restore_ask {
                Some(n) => format!(
                    "restore {n} setting{} on this tab to their defaults? y yes · any other key cancels",
                    if n == 1 { "" } else { "s" }
                ),
                None => self.status.clone(),
            };
            let style = if self.status_error { theme::error() } else { theme::text() };
            // Centred text wider than the box is cut at both ends; the key name leads.
            let align = if theme::str_width(&text) > inner.width as usize {
                Alignment::Left
            } else {
                Alignment::Center
            };
            f.render_widget(
                Paragraph::new(Line::from(text)).style(style).alignment(align),
                ratatui::layout::Rect { y: inner.y + 1, height: 1, ..inner },
            );
        }
    }
}

/// The label column stops growing when the value column would drop below this:
/// room for a swatch and its hex.
const VALUE_MIN: usize = 12;
const LABEL_MIN: usize = 6;
const LIST_LEAD: usize = 1;
pub(super) const LIST_GAP: usize = 2;

/// Footer hints, dropped in this order when they do not fit.
const KEYS_DROP_ORDER: [&str; 12] =
    ["←→", "[ ]", "j/k", "J/K", "t", "d", "x", "R", "/", "r", "⇥", "⏎"];

/// Below this body height the sample is dropped so the colour list keeps its rows.
const THEME_SAMPLE_MIN_HEIGHT: u16 = 25;

/// Bar heights, in eighths of the strip, repeated across its width.
const SAMPLE_BARS: [u16; 12] = [6, 14, 22, 30, 26, 18, 10, 20, 28, 16, 8, 24];

/// Rows of the theme preview's parts for the borders and header in force.
struct PreviewRows {
    header: u16,
    /// The rule between the header and the sample, in `lines`.
    rule: u16,
    sample: u16,
}

fn preview_rows(ui: &crate::config::UiConfig) -> PreviewRows {
    let style = theme::spacing().borders;
    let boxed = style.boxed();
    PreviewRows {
        header: match (ui.compact_header(), boxed) {
            (false, true) => 4,
            (false, false) => 2,
            (true, true) => 3,
            (true, false) => 1,
        },
        rule: u16::from(style == theme::BorderStyle::Lines),
        sample: if boxed { 6 } else { 4 },
    }
}

/// The player's header with placeholder text, drawn by the same code.
fn draw_theme_preview(f: &mut Frame, area: Rect, ui: &crate::config::UiConfig) {
    use crate::ui::views;
    let style = theme::spacing().borders;
    let rows = preview_rows(ui);
    let header = Rect { height: area.height.min(rows.header), ..area };
    let gap = theme::spacing().block_gap;
    if area.height >= rows.header + rows.rule + gap + rows.sample {
        if rows.rule > 0 {
            views::draw_rule(f, Rect { y: area.y + rows.header, height: 1, ..area });
        }
        draw_theme_sample(
            f,
            Rect { y: area.y + rows.header + rows.rule + gap, height: rows.sample, ..area },
        );
    }
    let info = views::HeaderInfo {
        stopped: false,
        paused: false,
        track: Some(("Song name".into(), "Artist name".into(), "Album".into())),
        times: "0:42 / 4:17".into(),
        audio: ("FLAC 44.1kHz/16bit".into(), "FLAC 44.1k/16".into()),
        // Not full: at 100% a preset's unlit glyph would never show.
        volume: 60.0,
        volume_display: ui.volume_mode(),
        mode_words: ui.mode_words(),
        ascii: ui.ascii_icons(),
        flags: [true, false, false, false],
        sleep: None,
        rating: views::rating_glyphs(ui.rating_mode(), 8, ui.ascii_icons()),
    };
    if ui.compact_header() {
        views::draw_tab_row(
            f,
            header,
            style,
            &["Queue", "Discover", "Browser", "Playlists", "Search", "Help"],
            0,
            &info,
        );
    } else {
        views::draw_header_block(f, header, style, &info);
    }
}

/// One check result as the CLI prints it: a passed setting has no symbol,
/// a passed check a tick.
fn entry_lines(lines: &mut Vec<Line<'static>>, entry: &doctor::Entry, check: bool) {
    let (symbol, style) = match entry.severity {
        doctor::Severity::Ok if check => ("✔ ", theme::text()),
        doctor::Severity::Ok => ("", theme::text()),
        doctor::Severity::Warn => ("! ", theme::secondary()),
        doctor::Severity::Fail => ("✘ ", theme::error()),
    };
    for (i, line) in entry.text.lines().enumerate() {
        let prefix = if i == 0 { format!("  {symbol}") } else { "    ".to_string() };
        lines.push(Line::from(vec![Span::styled(prefix, style), Span::raw(line.to_string())]));
    }
}

/// Queue rows and a visualizer strip, for the colours the header does not use.
fn draw_theme_sample(f: &mut Frame, area: Rect) {
    let inner = crate::ui::views::draw_block(f, area);
    if inner.width < 24 || inner.height == 0 {
        return;
    }
    let viz_w = (inner.width / 3).min(24);
    let rows_w = inner.width - viz_w - 1;
    let line = |style: Option<ratatui::style::Style>, title: Vec<Span<'static>>, rest: &str| {
        let secondary = style.unwrap_or_else(theme::dim);
        let mut spans = vec![Span::raw(" ")];
        spans.extend(title);
        spans.push(Span::styled(format!("  {rest}"), secondary));
        let mut line = Line::from(spans);
        if let Some(style) = style {
            line = line.style(style);
        }
        line
    };
    let plain = |text: &'static str| vec![Span::styled(text, theme::text())];
    let lines = [
        line(Some(theme::current()), vec![Span::raw("Song name")], "Artist  4:17"),
        line(Some(theme::current_unfocused()), plain("Another song"), "Artist  3:05"),
        line(
            None,
            vec![
                Span::styled("A ", theme::text()),
                Span::styled("match", theme::secondary()),
                Span::styled("ing song", theme::text()),
            ],
            "Artist  5:30",
        ),
        line(None, plain("A plain row"), "Artist  2:48"),
    ];
    for (i, text) in lines.into_iter().enumerate().take(inner.height as usize) {
        let row = Rect { y: inner.y + i as u16, width: rows_w, height: 1, ..inner };
        let style = text.style;
        f.render_widget(Paragraph::new(text).style(style), row);
    }

    // A bar every other column, drawn per row from the bottom like the visualizer.
    let strip =
        Rect { x: inner.x + rows_w + 1, width: viz_w, height: inner.height.min(4), ..inner };
    let ramp = theme::symbols().visualizer;
    let eighths = ramp.len() as u16;
    for row in 0..strip.height {
        let from_bottom = strip.height - 1 - row;
        let t = if strip.height > 1 { from_bottom as f32 / (strip.height - 1) as f32 } else { 0.0 };
        let mut text = String::new();
        for col in 0..strip.width {
            let height = SAMPLE_BARS[(col / 2) as usize % SAMPLE_BARS.len()] * strip.height / 4;
            let fill = height.saturating_sub(from_bottom * eighths).min(eighths);
            text.push(if col % 2 == 0 && fill > 0 { ramp[fill as usize - 1] } else { ' ' });
        }
        f.render_widget(
            Paragraph::new(text).style(ratatui::style::Style::default().fg(theme::viz_color(t))),
            Rect { y: strip.y + row, height: 1, ..strip },
        );
    }
}

/// An input line `width` columns wide, with the cursor drawn over its char (or
/// as `end` after the last). Scrolls to keep the cursor in view.
fn input_spans(
    text: &str,
    cursor: usize,
    width: usize,
    end: &'static str,
    style: Style,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let cell = chars.get(cursor).map_or_else(|| end.to_string(), |c| c.to_string());
    let cell_w = theme::str_width(&cell).max(1);
    let width_of = |s: &[char]| theme::str_width(&s.iter().collect::<String>());
    let mut start = 0;
    while start < cursor && width_of(&chars[start..cursor]) + cell_w > width {
        start += 1;
    }
    let mut used = width_of(&chars[start..cursor]) + cell_w;
    let mut stop = (cursor + 1).min(chars.len());
    while stop < chars.len() {
        let w = theme::str_width(&chars[stop].to_string());
        if used + w > width {
            break;
        }
        used += w;
        stop += 1;
    }
    let before: String = chars[start..cursor].iter().collect();
    let after: String = chars.get(cursor + 1..stop).unwrap_or(&[]).iter().collect();
    let cursor_style =
        if chars.get(cursor).is_some() { style.add_modifier(Modifier::REVERSED) } else { style };
    vec![Span::styled(before, style), Span::styled(cell, cursor_style), Span::styled(after, style)]
}

/// Where a window of `rows` rows starts so `index` is in it, centred if possible.
fn window_start(len: usize, index: usize, rows: usize) -> usize {
    index.saturating_sub(rows / 2).min(len.saturating_sub(rows))
}

fn theme_details(id: &str, themes: &[themes::Entry]) -> Vec<Line<'static>> {
    let Some(entry) = themes.iter().find(|t| t.id == id) else {
        return vec![Line::from(Span::styled("not a known theme", theme::dim()))];
    };
    let author = if entry.author.is_empty() { "—" } else { entry.author.as_str() };
    let variant = entry.variant.map_or("—", themes::Variant::name);
    let source = match entry.source {
        themes::Source::Preset => "built in".to_string(),
        themes::Source::File => format!("file themes/{}.toml", entry.id),
    };
    let mut lines = vec![
        Line::from(Span::styled(entry.title.clone(), theme::secondary())),
        Line::from(Span::styled(format!("author  {author}"), theme::text())),
        Line::from(Span::styled(format!("variant {variant}"), theme::text())),
        Line::from(Span::styled(format!("source  {source}"), theme::text())),
    ];
    let styles = entry.look.styles.count();
    if styles > 0 {
        lines.push(Line::from(Span::styled(
            format!("styles  {styles} set by this theme"),
            theme::text(),
        )));
    }
    lines
}
