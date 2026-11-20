use super::{Editor, Screen, CUSTOM};
use crate::config::Config;
use crate::inventory::{Kind, Setting};
use crate::ui::text_input::TextInput;
use crate::ui::theme;
use crate::ui::themes;
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Screen {
    /// `custom…` in a preset picker: a text field for glyphs or a preset name.
    fn begin_custom(&mut self, setting: &'static Setting) {
        let text = self.value(setting);
        self.editor = Some(Editor::Field {
            initial: text.clone(),
            text: TextInput::new(text),
            secret: false,
        });
    }

    /// Shows a preset while the picker moves; `Esc` puts the old look back.
    fn preview_preset(&self, option: &str) {
        let Some(setting) = self.selected().filter(|s| s.kind == Kind::Symbol) else {
            return;
        };
        let key = setting.key.trim_start_matches("theme.");
        if option == CUSTOM || theme::symbol_presets(key).is_none() {
            return;
        }
        let mut preview = self.cfg.clone();
        let _ = crate::setup::set_by_key(
            &mut preview,
            setting.key,
            toml::Value::String(option.to_string()),
        );
        let (appearance, _) = theme::appearance_from_config(&preview.theme);
        theme::set_appearance(appearance, self.cfg.ui.ascii_icons());
    }

    /// The values the picker and the arrow keys cycle for an enum setting.
    fn enum_options(&self, setting: &Setting) -> Vec<String> {
        match setting.kind {
            Kind::Enum { options } => options.iter().map(|o| o.to_string()).collect(),
            Kind::EnumDynamic if setting.key == "theme.name" => {
                themes::available(self.cfg.theme.dir.as_deref())
                    .entries
                    .iter()
                    .map(|t| t.id.clone())
                    .collect()
            }
            Kind::EnumDynamic => cava_sources(&self.cfg),
            _ => Vec::new(),
        }
    }

    pub(super) fn begin_edit(&mut self, setting: &'static Setting) {
        let current = self.value(setting);
        match setting.kind {
            Kind::Bool => {
                let on = current.trim() != "true";
                self.set(setting, if on { "true" } else { "false" });
            }
            Kind::Secret => {
                // Empty: the list's `(set)` placeholder must never replace the secret.
                self.editor = Some(Editor::Field {
                    text: TextInput::default(),
                    initial: String::new(),
                    secret: true,
                });
            }
            Kind::Enum { .. } => {
                let mut options = self.enum_options(setting);
                let index = pick_index(&mut options, &current);
                self.editor = Some(Editor::Picker { options, index, themes: Vec::new() });
            }
            Kind::Symbol
                if theme::symbol_presets(setting.key.trim_start_matches("theme.")).is_some() =>
            {
                let mut options: Vec<String> =
                    theme::symbol_presets(setting.key.trim_start_matches("theme."))
                        .unwrap_or_default()
                        .iter()
                        .map(|o| o.to_string())
                        .collect();
                let index = options.iter().position(|o| *o == current).unwrap_or(options.len());
                options.push(CUSTOM.to_string());
                self.editor = Some(Editor::Picker { options, index, themes: Vec::new() });
            }
            Kind::EnumDynamic if setting.key == "theme.name" => {
                // Read on open, so a theme file added meanwhile is there.
                let themes = themes::available(self.cfg.theme.dir.as_deref()).entries;
                let mut options = self.enum_options(setting);
                let index = pick_index(&mut options, &current);
                self.editor = Some(Editor::Picker { options, index, themes });
            }
            Kind::EnumDynamic => {
                let mut options = self.enum_options(setting);
                let index = pick_index(&mut options, &current);
                self.editor = Some(Editor::Picker { options, index, themes: Vec::new() });
            }
            Kind::List => {
                // A comma-separated field could not hold `--af=lavfi=[a,b]`.
                let rows: Vec<String> = match crate::setup::get_by_key(&self.cfg, setting.key) {
                    Some(toml::Value::Array(items)) => {
                        items.iter().map(|item| item.as_str().unwrap_or("").to_string()).collect()
                    }
                    _ => Vec::new(),
                };
                self.editor = Some(Editor::List(ListEdit {
                    initial: rows.clone(),
                    rows,
                    cursor: 0,
                    editing: None,
                }));
            }
            Kind::Text if setting.key == "plex.music_section" => {
                let id = if current == "(empty)" { String::new() } else { current };
                self.begin_library(id);
            }
            Kind::Pair | Kind::Triple => {
                let text = match crate::setup::get_by_key(&self.cfg, setting.key) {
                    Some(toml::Value::Array(items)) if items.len() == 2 || items.len() == 3 => {
                        items.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
                    }
                    _ => current.clone(),
                };
                self.editor = Some(Editor::Field {
                    initial: text.clone(),
                    text: TextInput::new(text),
                    secret: false,
                });
            }
            _ => {
                let text = if current == "(empty)" { String::new() } else { current.clone() };
                self.editor = Some(Editor::Field {
                    initial: text.clone(),
                    text: TextInput::new(text),
                    secret: false,
                });
            }
        }
    }

    /// `←`/`→`: step the setting without opening it, through `set` like a commit.
    pub(super) fn step(&mut self, dir: i32) {
        let Some(setting) = self.selected() else {
            return;
        };
        let current = self.value(setting);
        let current = current.trim();
        match setting.kind {
            Kind::Bool => {
                self.set(setting, if current != "true" { "true" } else { "false" });
            }
            Kind::Int { min, max } => {
                let Ok(now) = current.parse::<i64>() else {
                    return;
                };
                // Ones would take forever over a wide range.
                let by = match (min, max) {
                    (Some(lo), Some(hi)) if hi - lo >= 100 => 5,
                    _ => 1,
                };
                let mut next = now.saturating_add(dir as i64 * by);
                next = next.max(min.unwrap_or(i64::MIN)).min(max.unwrap_or(i64::MAX));
                if next == now {
                    self.at_limit(setting);
                } else {
                    self.set(setting, &next.to_string());
                }
            }
            Kind::Float { min, max } => {
                let Ok(now) = current.parse::<f64>() else {
                    return;
                };
                let by = match (min, max) {
                    (Some(lo), Some(hi)) if hi - lo < 10.0 => 0.1,
                    _ => 1.0,
                };
                let mut next = now + dir as f64 * by;
                next = next.max(min.unwrap_or(f64::MIN)).min(max.unwrap_or(f64::MAX));
                // 0.1 steps drift (0.30000000000000004).
                next = (next * 1000.0).round() / 1000.0;
                if next == now {
                    self.at_limit(setting);
                } else {
                    self.set(setting, &next.to_string());
                }
            }
            Kind::Enum { .. } | Kind::EnumDynamic => {
                let mut options = self.enum_options(setting);
                if options.is_empty() {
                    return;
                }
                let index = pick_index(&mut options, current);
                let next = (index as i64 + dir as i64).rem_euclid(options.len() as i64) as usize;
                self.set(setting, &options[next]);
            }
            // Cycles preset names; custom glyphs are left alone.
            Kind::Symbol => {
                let key = setting.key.trim_start_matches("theme.");
                let Some(names) = theme::symbol_presets(key) else {
                    return;
                };
                let Some(at) = names.iter().position(|n| *n == current) else {
                    return;
                };
                let next = (at as i64 + dir as i64).rem_euclid(names.len() as i64) as usize;
                self.set(setting, names[next]);
            }
            Kind::Text | Kind::Secret | Kind::List | Kind::Pair | Kind::Triple | Kind::Color => {}
        }
    }

    fn at_limit(&mut self, setting: &Setting) {
        self.status = format!("{} is at its limit", setting.key);
        self.status_error = false;
    }

    /// The status is left alone: it is about the value being edited.
    pub(super) fn handle_editor(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(editor) = self.editor.take() else {
            return Ok(true);
        };
        match editor {
            Editor::Field { mut text, initial, secret } => match key.code {
                // Ctrl-C cancels like Esc; it must not type a 'c'.
                KeyCode::Esc => self.cancel_field(),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.cancel_field();
                }
                KeyCode::Enter => {
                    if let Some(setting) = self.selected() {
                        if !secret && text.text() == initial {
                            self.status = format!("{} unchanged", setting.key);
                            self.status_error = false;
                            // Undoes any live colour preview.
                            self.apply_theme();
                        } else if !self.set(setting, text.text()) {
                            // Keep the field so the value can be fixed.
                            self.editor = Some(Editor::Field { text, initial, secret });
                        }
                    }
                }
                _ => {
                    let changed = text.handle(key);
                    self.editor = Some(Editor::Field { text, initial, secret });
                    if changed {
                        self.preview();
                    }
                }
            },
            Editor::Export { mut text } => match key.code {
                KeyCode::Esc => self.status.clear(),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.status.clear();
                }
                KeyCode::Enter => {
                    if !self.export_theme(text.text().trim()) {
                        // Keep the field so the id can be fixed.
                        self.editor = Some(Editor::Export { text });
                    }
                }
                _ => {
                    text.handle(key);
                    self.editor = Some(Editor::Export { text });
                }
            },
            Editor::Library(pick) => self.library_key(pick, key),
            Editor::List(list) => self.list_key(list, key),
            Editor::Picker { options, mut index, themes } => match key.code {
                KeyCode::Esc => {
                    self.apply_theme();
                    self.status.clear();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    index = (index + 1).min(options.len().saturating_sub(1));
                    self.preview_theme(&options[index], &themes);
                    self.preview_preset(&options[index]);
                    self.editor = Some(Editor::Picker { options, index, themes });
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    index = index.saturating_sub(1);
                    self.preview_theme(&options[index], &themes);
                    self.preview_preset(&options[index]);
                    self.editor = Some(Editor::Picker { options, index, themes });
                }
                KeyCode::Enter => {
                    let value = options[index].as_str();
                    if let Some(setting) = self.selected() {
                        if value == CUSTOM && setting.kind == Kind::Symbol {
                            self.apply_theme();
                            self.begin_custom(setting);
                        } else {
                            self.set(setting, value);
                        }
                    }
                }
                _ => {
                    self.editor = Some(Editor::Picker { options, index, themes });
                }
            },
        }
        Ok(true)
    }

    fn list_key(&mut self, mut list: ListEdit, key: KeyEvent) {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((mut input, is_new)) = list.editing.take() {
            match key.code {
                KeyCode::Esc => list.drop_if_new(is_new),
                _ if ctrl_c => list.drop_if_new(is_new),
                KeyCode::Enter => {
                    let text = input.into_text().trim().to_string();
                    if text.is_empty() {
                        list.remove();
                    } else {
                        list.rows[list.cursor] = text;
                    }
                }
                _ => {
                    input.handle(key);
                    list.editing = Some((input, is_new));
                }
            }
            self.editor = Some(Editor::List(list));
            return;
        }
        match key.code {
            KeyCode::Esc => return self.finish_list(list),
            _ if ctrl_c => return self.finish_list(list),
            KeyCode::Down | KeyCode::Char('j') => {
                list.cursor = (list.cursor + 1).min(list.rows.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => list.cursor = list.cursor.saturating_sub(1),
            KeyCode::Enter if !list.rows.is_empty() => {
                let row = list.rows[list.cursor].clone();
                list.editing = Some((TextInput::new(row), false));
            }
            KeyCode::Char('a') => {
                let at = if list.rows.is_empty() { 0 } else { list.cursor + 1 };
                list.rows.insert(at, String::new());
                list.cursor = at;
                list.editing = Some((TextInput::default(), true));
            }
            KeyCode::Char('d') if !list.rows.is_empty() => list.remove(),
            KeyCode::Char('J') if list.cursor + 1 < list.rows.len() => {
                list.rows.swap(list.cursor, list.cursor + 1);
                list.cursor += 1;
            }
            KeyCode::Char('K') if list.cursor > 0 && list.cursor < list.rows.len() => {
                list.rows.swap(list.cursor, list.cursor - 1);
                list.cursor -= 1;
            }
            _ => {}
        }
        self.editor = Some(Editor::List(list));
    }

    /// Esc on the list: the rows become the setting.
    fn finish_list(&mut self, list: ListEdit) {
        let Some(setting) = self.selected() else {
            return;
        };
        if list.rows == list.initial {
            self.status = format!("{} unchanged", setting.key);
            self.status_error = false;
            return;
        }
        if setting.key == "ui.queue_columns" {
            if let Some(problem) = crate::config::queue_columns_problem(&list.rows) {
                self.status = format!("rejected: {problem}");
                self.status_error = true;
                return;
            }
        }
        let value = toml::Value::Array(list.rows.into_iter().map(toml::Value::String).collect());
        self.apply(setting, value);
    }

    fn cancel_field(&mut self) {
        // Undoes a live colour preview.
        self.apply_theme();
        self.status.clear();
    }

    /// Control characters are dropped, so a trailing newline is not stored.
    pub(super) fn handle_paste(&mut self, pasted: String) {
        if self.review.is_some()
            || self.conflict_ask.is_some()
            || self.restore_ask.is_some()
            || self.quit_ask.is_some()
        {
            return;
        }
        let clean: String = pasted.chars().filter(|c| !c.is_control()).collect();
        if self.filter.typing && self.editor.is_none() {
            let mut input = TextInput::at(self.filter.text.clone(), self.filter_cursor);
            input.insert_str(&clean);
            self.filter_cursor = input.cursor();
            self.edit_filter(|t| *t = input.into_text());
            return;
        }
        if let Some(Editor::List(ListEdit { editing: Some((input, _)), .. })) = &mut self.editor {
            input.insert_str(&clean);
            return;
        }
        let Some(Editor::Field { text, .. } | Editor::Export { text }) = &mut self.editor else {
            return;
        };
        text.insert_str(&clean);
        self.preview();
    }

    fn preview(&mut self) {
        let Some(setting) = self.selected() else {
            return;
        };
        if setting.kind != Kind::Color {
            return;
        }
        let Some(Editor::Field { text, .. }) = &self.editor else {
            return;
        };
        let Some(colour) = colour_for(setting.key, text.text()) else {
            return;
        };
        let mut preview = self.cfg.clone();
        set_key(&mut preview, setting.key, colour);
        theme::set_theme(theme::from_config(&preview.theme).0);
    }

    /// A bad value becomes a status line, not an error; false when it was rejected.
    pub(super) fn set(&mut self, setting: &Setting, text: &str) -> bool {
        // An untouched secret commits empty: keep the token, never write it empty.
        if setting.kind == Kind::Secret && text.trim().is_empty() {
            self.status = format!("{} unchanged", setting.key);
            self.status_error = false;
            return true;
        }
        let value = match if setting.kind == Kind::Symbol {
            symbol_value(setting.key, text)
        } else {
            colour_for(setting.key, text)
                .filter(|_| setting.kind == Kind::Color)
                .map(toml::Value::String)
                .map_or_else(|| value_for(setting.kind, text), Ok)
        } {
            Ok(value) => value,
            Err(e) => {
                // "rejected" first: the reason alone reads like the new value.
                self.status = format!("rejected: {e:#}");
                self.status_error = true;
                return false;
            }
        };
        self.apply(setting, value)
    }

    fn apply(&mut self, setting: &Setting, value: toml::Value) -> bool {
        let old_name = self.cfg.theme.name.clone();
        match crate::setup::set_by_key(&mut self.cfg, setting.key, value) {
            Ok(()) => {
                // Overrides made on the old theme do not follow a new one.
                let mut cleared = 0;
                if setting.key == "theme.name" && self.cfg.theme.name != old_name {
                    cleared = self.cfg.theme.override_count();
                    self.cfg.theme.clear_overrides();
                }
                self.status = format!("{} = {}", setting.key, self.value(setting));
                // The value is kept; say why a dimmed row has no effect.
                let values = self.values();
                if let Some(reason) = self.unused_because(setting, &values) {
                    self.status.push_str(&format!("; {reason}"));
                }
                if cleared > 0 {
                    let s = if cleared == 1 { "" } else { "s" };
                    self.status.push_str(&format!("; {cleared} override{s} cleared"));
                }
                self.status_error = false;
                if setting.key.starts_with("theme.") || setting.key == "ui.icons" {
                    self.apply_theme();
                }
                true
            }
            Err(e) => {
                self.status = format!("rejected: {e:#}");
                self.status_error = true;
                false
            }
        }
    }
}

pub(super) struct ListEdit {
    pub(super) rows: Vec<String>,
    /// A finish with the same rows changes nothing.
    initial: Vec<String>,
    pub(super) cursor: usize,
    /// The row being edited, and whether it is new (a cancelled new row goes).
    pub(super) editing: Option<(TextInput, bool)>,
}

impl ListEdit {
    fn remove(&mut self) {
        if self.cursor < self.rows.len() {
            self.rows.remove(self.cursor);
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn drop_if_new(&mut self, is_new: bool) {
        if is_new {
            self.remove();
        }
    }
}

/// A value the list does not offer is added, so the highlight is the value in effect.
fn pick_index(options: &mut Vec<String>, current: &str) -> usize {
    if let Some(index) = options.iter().position(|option| option == current) {
        return index;
    }
    options.push(current.to_string());
    options.len() - 1
}

fn cava_sources(cfg: &Config) -> Vec<String> {
    let mut out = vec![String::new()];
    let default_source = crate::config::CavaConfig { source: String::new(), ..cfg.cava.clone() };
    for (_, source) in crate::playback::cava::input_modes(&default_source) {
        if !out.contains(&source) {
            out.push(source);
        }
    }
    out
}

pub(super) fn value_for(kind: Kind, text: &str) -> Result<toml::Value> {
    let text = text.trim();
    Ok(match kind {
        Kind::Text | Kind::Secret | Kind::Enum { .. } | Kind::EnumDynamic => {
            toml::Value::String(text.to_string())
        }
        Kind::Int { min, max } => {
            let value: i64 =
                text.parse().with_context(|| format!("{text:?} is not a whole number"))?;
            if let Some(min) = min {
                anyhow::ensure!(value >= min, "the range starts at {min}");
            }
            if let Some(max) = max {
                anyhow::ensure!(value <= max, "the range ends at {max}");
            }
            toml::Value::Integer(value)
        }
        Kind::Float { min, max } => {
            let value: f64 = text.parse().with_context(|| format!("{text:?} is not a number"))?;
            if let Some(min) = min {
                anyhow::ensure!(value >= min, "the range starts at {min}");
            }
            if let Some(max) = max {
                anyhow::ensure!(value <= max, "the range ends at {max}");
            }
            toml::Value::Float(value)
        }
        Kind::Bool => toml::Value::Boolean(text == "true"),
        Kind::Color => {
            let colour = parse_colour(text)
                .with_context(|| format!("{text:?} is not a colour; use #rrggbb"))?;
            toml::Value::String(colour)
        }
        // Checked against its key, untrimmed (a space is a glyph): `symbol_value`.
        Kind::Symbol => toml::Value::String(text.to_string()),
        Kind::List => toml::Value::Array(
            text.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| toml::Value::String(s.to_string()))
                .collect(),
        ),
        Kind::Pair => {
            let parts: Vec<&str> = text.split(',').map(str::trim).collect();
            anyhow::ensure!(parts.len() == 2, "use W,H (0,0 for no limit)");
            let parse =
                |s: &str| s.parse::<i64>().with_context(|| format!("{s:?} is not a number"));
            toml::Value::Array(vec![
                toml::Value::Integer(parse(parts[0])?),
                toml::Value::Integer(parse(parts[1])?),
            ])
        }
        Kind::Triple => {
            let parts: Vec<&str> = text.split(',').map(str::trim).collect();
            anyhow::ensure!(parts.len() == 3, "use Title,Artist,Album");
            let mut weights = Vec::with_capacity(3);
            for part in parts {
                let n: i64 =
                    part.parse().with_context(|| format!("{part:?} is not a whole number"))?;
                weights.push(n);
            }
            if let Some(problem) = crate::config::column_weights_problem(&weights) {
                anyhow::bail!(problem);
            }
            toml::Value::Array(weights.into_iter().map(toml::Value::Integer).collect())
        }
    })
}

/// The rule theme files and `--check` use; not trimmed, as a space is a glyph.
pub(super) fn symbol_value(key: &str, text: &str) -> Result<toml::Value> {
    let name = key.strip_prefix("theme.").unwrap_or(key);
    let mut probe = theme::Look::DEFAULT;
    anyhow::ensure!(
        theme::set_symbol(&mut probe, name, text),
        "{key} {}",
        theme::symbol_rule(name)
    );
    Ok(toml::Value::String(text.to_string()))
}

/// Lowercase `#rrggbb`, or an ANSI name as the hex it stands for.
pub(super) fn parse_colour(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(ratatui::style::Color::Rgb(r, g, b)) = theme::parse_hex(text) {
        return Some(format!("#{r:02x}{g:02x}{b:02x}"));
    }
    let name = text.to_ascii_lowercase().replace(' ', "-");
    const ANSI: &[(&str, &str)] = &[
        ("black", "#000000"),
        ("red", "#800000"),
        ("green", "#008000"),
        ("yellow", "#808000"),
        ("blue", "#000080"),
        ("magenta", "#800080"),
        ("cyan", "#008080"),
        ("white", "#c0c0c0"),
        ("bright-black", "#808080"),
        ("bright-red", "#ff0000"),
        ("bright-green", "#00ff00"),
        ("bright-yellow", "#ffff00"),
        ("bright-blue", "#0000ff"),
        ("bright-magenta", "#ff00ff"),
        ("bright-cyan", "#00ffff"),
        ("bright-white", "#ffffff"),
    ];
    ANSI.iter().find(|(n, _)| *n == name).map(|(_, hex)| (*hex).to_string())
}

/// [`parse_colour`], plus `none` for the background (turns the theme's off).
fn colour_for(key: &str, text: &str) -> Option<String> {
    if key == "theme.background" && text.trim().eq_ignore_ascii_case("none") {
        return Some("none".into());
    }
    parse_colour(text)
}

fn set_key(cfg: &mut Config, key: &str, colour: String) {
    let value = toml::Value::String(colour);
    let _ = crate::setup::set_by_key(cfg, key, value);
}
