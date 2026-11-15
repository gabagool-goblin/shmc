use super::{Screen, Tab};
use crate::config::{Config, SettingRow};
use crate::inventory::{self, Kind};
use crate::ui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Screen {
    pub(super) fn changes_on(tab: Tab, changed: &[String]) -> usize {
        changed
            .iter()
            .filter(|key| {
                let theme = inventory::find(key)
                    .map(|s| inventory::is_theme_group(s.group))
                    .unwrap_or_else(|| key.starts_with("theme."));
                match tab {
                    Tab::Theme => theme,
                    Tab::Config => !theme,
                    Tab::Diagnostics => false,
                }
            })
            .count()
    }

    pub(super) fn open_review(&mut self) {
        let mut keys = self.cfg.changed_keys(&self.saved);
        if keys.is_empty() {
            self.status = "nothing to save".into();
            self.status_error = false;
            return;
        }
        let place =
            |key: &String| inventory::ALL.iter().position(|s| s.key == key).unwrap_or(usize::MAX);
        keys.sort_by_key(place);
        let current = self.values();
        let before: std::collections::BTreeMap<String, SettingRow> =
            self.saved.setting_rows().into_iter().map(|row| (row.key.clone(), row)).collect();
        // An unset colour override shows the theme's colour in its row: say
        // so rather than show a hex that was never written.
        let set_in = |cfg: &Config| cfg.flat_values().unwrap_or_default();
        let (flat_now, flat_before) = (set_in(&self.cfg), set_in(&self.saved));
        let shown = |rows: &std::collections::BTreeMap<String, SettingRow>,
                     flat: &std::collections::BTreeMap<String, toml::Value>,
                     key: &str| {
            if !flat.contains_key(key) {
                return if key.starts_with("theme.") { "(theme)" } else { "(unset)" }.to_string();
            }
            rows.get(key).map_or_else(|| "(unset)".to_string(), |r| r.current.clone())
        };
        let rows = keys
            .iter()
            .map(|key| {
                // A secret's value is never shown, old or new.
                let secret = key == "plex.token"
                    || inventory::find(key).is_some_and(|s| s.kind == Kind::Secret);
                let (old, new) = if secret {
                    (String::new(), String::new())
                } else {
                    (shown(&before, &flat_before, key), shown(&current, &flat_now, key))
                };
                (key.clone(), old, new, secret)
            })
            .collect();
        self.review = Some(Review { rows, scroll: 0, visible: 0 });
    }

    pub(super) fn handle_review(&mut self, key: KeyEvent) {
        let Some(review) = &mut self.review else {
            return;
        };
        let page = review.visible.max(1);
        match key.code {
            KeyCode::Char('s') | KeyCode::Enter => {
                self.review = None;
                self.save();
            }
            KeyCode::Esc => self.review = None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.review = None;
            }
            KeyCode::Down | KeyCode::Char('j') => review.scroll = review.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => review.scroll = review.scroll.saturating_sub(1),
            KeyCode::PageDown => review.scroll = review.scroll.saturating_add(page),
            KeyCode::PageUp => review.scroll = review.scroll.saturating_sub(page),
            _ => {}
        }
    }

    pub(super) fn dirty(&self) -> bool {
        // Not `setting_rows()`: it redacts the token, so a new one would
        // compare equal to the old.
        toml::to_string(&self.cfg).ok() != toml::to_string(&self.saved).ok()
    }

    /// True when the file was written. Only the screen's edits are written, so
    /// other changes made meanwhile survive.
    pub(super) fn save(&mut self) -> bool {
        // A missing or unreadable file is a plain save; one that does not
        // parse is refused by the writer.
        let outside = Config::outside_changes(&self.path, &self.saved).unwrap_or_default();
        let conflicts = self.conflicts(&outside);
        if !conflicts.is_empty() {
            self.quit_ask = None;
            self.status = format!(
                "also changed in the file: {}; o keeps yours, t takes the file's, Esc cancels",
                name_keys(&conflicts)
            );
            self.status_error = true;
            self.conflict_ask = Some(conflicts);
            return false;
        }
        self.write(outside.len())
    }

    /// The keys in `outside` that the screen changed too, to a different value.
    fn conflicts(&self, outside: &[String]) -> Vec<String> {
        let mine = self.cfg.changed_keys(&self.saved);
        let both: Vec<String> = outside.iter().filter(|k| mine.contains(k)).cloned().collect();
        if both.is_empty() {
            return both;
        }
        let Some(file) =
            std::fs::read_to_string(&self.path).ok().and_then(|text| Config::parse(&text).ok())
        else {
            return Vec::new();
        };
        let differ = self.cfg.changed_keys(&file);
        both.into_iter().filter(|k| differ.contains(k)).collect()
    }

    /// Back up, write the screen's changes and reload what the file holds.
    /// `kept` is how many changes made elsewhere the file still has.
    fn write(&mut self, kept: usize) -> bool {
        let backup =
            if self.path.exists() { Some(crate::setup::backup_config(&self.path)) } else { None };
        if let Err(e) = self.cfg.save_changes(&self.path, &self.saved) {
            // The screen stays open so the edit is not lost.
            self.status = format!("save failed: {e:#}");
            self.status_error = true;
            return false;
        }
        self.reload_from_file();
        self.status_error = false;
        let mut status = match backup {
            Some(Ok(_)) => {
                format!("saved {} (previous kept as config.toml.bak)", self.path.display())
            }
            Some(Err(e)) => {
                // Refusing to save would lose the edit; say what happened.
                self.status_error = true;
                format!("saved without a backup: {e:#}")
            }
            None => format!("saved {}", self.path.display()),
        };
        if kept > 0 {
            status.push_str(&format!(
                "; kept {kept} change{} made elsewhere",
                if kept == 1 { "" } else { "s" }
            ));
        }
        status.push_str("; a running player picks it up when it next starts");
        self.status = status;
        true
    }

    /// Take the file as the screen's config and baseline.
    fn reload_from_file(&mut self) {
        let theme_before = toml::to_string(&self.cfg.theme).ok();
        if let Some(mut cfg) =
            std::fs::read_to_string(&self.path).ok().and_then(|text| Config::parse(&text).ok())
        {
            cfg.theme.dir = self.cfg.theme.dir.clone();
            self.cfg = cfg;
        }
        self.saved = self.cfg.clone();
        if toml::to_string(&self.cfg.theme).ok() != theme_before {
            self.apply_theme();
        }
    }

    pub(super) fn handle_conflict_ask(&mut self, key: KeyEvent) {
        let Some(conflicts) = self.conflict_ask.clone() else {
            return;
        };
        match key.code {
            KeyCode::Char('o') => {
                self.conflict_ask = None;
                let outside = Config::outside_changes(&self.path, &self.saved).unwrap_or_default();
                let kept = outside.len().saturating_sub(conflicts.len());
                self.write(kept);
            }
            KeyCode::Char('t') => {
                self.conflict_ask = None;
                let discarded = self.cfg.changed_keys(&self.saved).len();
                self.reload_from_file();
                self.status = format!(
                    "took the file's version; discarded {discarded} change{}",
                    if discarded == 1 { "" } else { "s" }
                );
                self.status_error = false;
            }
            KeyCode::Esc => {
                self.conflict_ask = None;
                self.status.clear();
                self.status_error = false;
            }
            _ => {}
        }
    }

    fn restorable_keys(&self) -> Vec<&'static str> {
        let values = self.values();
        self.visible().into_iter().filter(|s| self.is_changed(s, &values)).map(|s| s.key).collect()
    }

    pub(super) fn reset_selected(&mut self) {
        if self.tab == Tab::Diagnostics {
            self.status = "nothing to restore on this tab".into();
            return;
        }
        let Some(setting) = self.selected() else {
            return;
        };
        let key = setting.key;
        if crate::config::NO_DEFAULT.contains(&key) {
            self.status = format!("{key} has no default to restore");
            return;
        }
        if self.cfg.restore_one(key).is_none() {
            self.status = format!("{key} already matches the default");
            return;
        }
        if self.tab == Tab::Theme {
            self.apply_theme();
        }
        // Overrides any palette problem `apply_theme` put in the status.
        self.status = if self.tab == Tab::Theme && key != "theme.name" {
            format!("removed the {key} override; press s to save")
        } else {
            format!("restored {key}; press s to save")
        };
    }

    pub(super) fn ask_to_restore(&mut self) {
        if self.tab == Tab::Diagnostics {
            self.status = "nothing to restore on this tab".into();
            return;
        }
        let n = self.restorable_keys().len();
        if n == 0 {
            self.status = "every setting on this tab already matches the default".into();
            return;
        }
        self.restore_ask = Some(n);
    }

    pub(super) fn handle_restore_ask(&mut self, key: KeyEvent) {
        self.restore_ask = None;
        self.status.clear();
        self.status_error = false;
        if key.code != KeyCode::Char('y') {
            return;
        }
        let mut done = 0;
        for k in self.restorable_keys() {
            if self.cfg.restore_one(k).is_some() {
                done += 1;
            }
        }
        if self.tab == Tab::Theme {
            // An override equal to the theme's colour is not "changed", so
            // the loop skipped it; it must not stay pinned.
            self.cfg.theme.clear_overrides();
            self.apply_theme();
        }
        if self.status.is_empty() {
            self.status = format!(
                "restored {done} setting{}; press s to save",
                if done == 1 { "" } else { "s" }
            );
        }
    }
}

/// The changes a save would write.
pub(super) struct Review {
    /// Key, old, new, and whether it is a secret (shown only as `(changed)`;
    /// its values are not kept here).
    pub(super) rows: Vec<(String, String, String, bool)>,
    pub(super) scroll: usize,
    /// Rows the panel showed at the last draw.
    pub(super) visible: usize,
}

/// `key  old → new`, each value cut to its share of the width.
pub(super) fn review_line(key: &str, old: &str, new: &str, secret: bool, width: usize) -> String {
    let key = theme::ellipsize(key, (width / 2).max(1) as u16);
    if secret {
        return format!(" {key}  (changed)");
    }
    let room = width.saturating_sub(theme::str_width(&key) + 6).max(2);
    let half = room / 2;
    format!(
        " {key}  {} → {}",
        theme::ellipsize(old, half as u16),
        theme::ellipsize(new, (room - half) as u16)
    )
}

fn name_keys(keys: &[String]) -> String {
    let shown = keys.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    match keys.len().saturating_sub(3) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}
