//! The settings as a flat list: what `shmc config --list` prints and what the config screen shows and restores.

use super::{themes_dir_for, token_label, Config, ThemeConfig};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    /// Dotted key, e.g. `ui.art_pct`.
    pub key: String,
    pub current: String,
    pub default: Option<String>,
    /// Colours are compared as colours, so `#7AA2F7` matches `#7aa2f7`.
    matches_default: bool,
}

impl SettingRow {
    pub fn is_default(&self) -> bool {
        self.matches_default
    }
}

/// The user's access to the server, not preferences: a restore keeps them.
pub const NO_DEFAULT: &[&str] = &["plex.server", "plex.token", "plex.music_section"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingChange {
    pub key: String,
    pub from: String,
    pub to: String,
}

fn key_list_text(keys: &[String]) -> String {
    if keys.is_empty() {
        "[]".into()
    } else {
        format!("[{}]", keys.join(", "))
    }
}

fn flatten_settings(table: &toml::Table, prefix: &str, out: &mut BTreeMap<String, String>) {
    for (key, value) in table {
        let key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            toml::Value::Table(section) => flatten_settings(section, &key, out),
            value => {
                out.insert(key, setting_text(value));
            }
        }
    }
}

/// A value as printed in the list: bare, not TOML-quoted. An empty string
/// needs a word, or it reads as a missing column.
fn setting_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) if s.is_empty() => "(empty)".to_string(),
        toml::Value::String(s) => s.clone(),
        // 100 rather than TOML's 100.0; it parses back as the same float.
        toml::Value::Float(f) if f.is_finite() && f.fract() == 0.0 => format!("{f:.0}"),
        other => other.to_string(),
    }
}

/// Longer values push that row's default over rather than being truncated.
const SETTING_VALUE_COLUMN_MAX: usize = 44;

impl Config {
    /// The tunables at their defaults, keeping `plex.server`, `plex.token`
    /// and `plex.music_section` ([`NO_DEFAULT`]).
    pub fn restored(&self) -> Config {
        let mut next =
            Config { plex: self.plex.clone(), keys: self.keys.clone(), ..Config::default() };
        next.theme.dir = self.theme.dir.clone();
        next
    }

    pub fn set_themes_dir(&mut self, path: &Path) {
        self.theme.dir = Some(themes_dir_for(path));
    }

    /// Every setting with its current and default value, sorted by dotted key.
    pub fn setting_rows(&self) -> Vec<SettingRow> {
        let shipped = shipped_settings();
        let (base, _) = crate::ui::theme::base_appearance(&self.theme);
        self.flat_settings()
            .into_iter()
            .map(|(key, current)| {
                let role = key.strip_prefix("theme.").filter(|role| {
                    ThemeConfig::ROLES.contains(role) || ThemeConfig::is_look_key(role)
                });
                let default = if NO_DEFAULT.contains(&key.as_str()) {
                    None
                } else if let Some(role) = role {
                    // Judged against the chosen theme, not what shmc ships:
                    // choosing a preset is not a change.
                    crate::ui::theme::role_text(&base.theme, role)
                        .or_else(|| crate::ui::theme::look_text(&base.look, role))
                } else {
                    shipped.get(&key).cloned()
                };
                let matches_default = default.as_deref().is_some_and(|d| {
                    if let Some(role) = role {
                        // An override equal to the theme's value would stay
                        // when the theme changes, so it counts as a change.
                        !self.theme.overridden(role)
                    } else {
                        d == current
                    }
                });
                SettingRow { key, current, default, matches_default }
            })
            .collect()
    }

    /// The plain-text list for `shmc config --list` and the editor's `l` key,
    /// shaped for pasting into a bug report.
    pub fn settings_list(&self) -> String {
        let mut rows = self.setting_rows();
        // Only here, not in the rows: the config screen edits those.
        for row in rows.iter_mut().filter(|r| r.key == "plex.server") {
            row.current = crate::redact::redact_url(&row.current);
        }
        let (keymap, _) = crate::ui::keymap::Keymap::from_table(&self.keys);
        for (ctx, _, _) in crate::ui::keymap::SECTIONS {
            for binding in crate::ui::keymap::BINDINGS.iter().filter(|b| b.ctx == *ctx) {
                let Some(name) = binding.act.name() else { continue };
                let current = keymap.bound_keys(*ctx, &binding.act);
                let default: Vec<String> =
                    binding.keys.iter().map(|key| (*key).to_string()).collect();
                rows.push(SettingRow {
                    key: format!("keys.{}.{}", ctx.config_name(), name),
                    current: key_list_text(&current),
                    default: Some(key_list_text(&default)),
                    matches_default: current == default,
                });
            }
        }
        let key_w =
            rows.iter().map(|r| r.key.chars().count()).max().unwrap_or(0).max("setting".len());
        let value_w = rows
            .iter()
            .map(|r| r.current.chars().count())
            .max()
            .unwrap_or(0)
            .clamp("current".len(), SETTING_VALUE_COLUMN_MAX);

        let mut out = String::from(
            "shmc config settings\n\
             \x20 '= default' marks a value that still matches the default.\n\
             \x20 plex.server, plex.token and plex.music_section have no default and are kept by a restore.\n\n",
        );
        let header = format!("  {:<key_w$}  {:<value_w$}  {}", "setting", "current", "default");
        out.push_str(header.trim_end());
        out.push('\n');
        for row in rows {
            let marker = if row.is_default() { "  = default" } else { "" };
            let default = row.default.as_deref().unwrap_or("—");
            let line =
                format!("  {:<key_w$}  {:<value_w$}  {}{}", row.key, row.current, default, marker);
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// Reset the tunables in place and report what changed. Writes nothing.
    pub fn restore_defaults(&mut self) -> Vec<SettingChange> {
        let before: BTreeMap<String, String> =
            self.setting_rows().into_iter().map(|row| (row.key, row.current)).collect();
        *self = self.restored();
        self.setting_rows()
            .into_iter()
            .filter_map(|row| match before.get(&row.key) {
                Some(from) if *from != row.current => {
                    Some(SettingChange { key: row.key, from: from.clone(), to: row.current })
                }
                _ => None,
            })
            .collect()
    }

    /// `None` when nothing changed: unknown key, no default, or already the
    /// default. Writes nothing.
    pub fn restore_one(&mut self, key: &str) -> Option<SettingChange> {
        let row = self.setting_rows().into_iter().find(|r| r.key == key)?;
        let dropped = key.strip_prefix("theme.").and_then(|role| {
            if ThemeConfig::is_look_key(role) {
                Some(self.theme.clear_look(role).then_some(()))
            } else {
                self.theme.slot(role).map(|slot| slot.take().map(|_| ()))
            }
        });
        if let Some(dropped) = dropped {
            // Dropped, not set to the theme's value, so it does not stay
            // pinned when the theme changes.
            dropped?;
            let to = self.setting_rows().into_iter().find(|r| r.key == key)?.current;
            return Some(SettingChange { key: row.key, from: row.current, to });
        }
        if row.default.is_none() || row.is_default() {
            return None;
        }
        let shipped = Config::default().flat_values().ok()?.remove(key)?;
        let mut next = self.clone();
        crate::setup::set_by_key(&mut next, key, shipped).ok()?;
        *self = next;
        let to = self.setting_rows().into_iter().find(|r| r.key == key)?.current;
        Some(SettingChange { key: row.key, from: row.current, to })
    }

    /// Dotted keys to display values, token redacted.
    pub(crate) fn flat_settings(&self) -> BTreeMap<String, String> {
        let mut safe = self.clone();
        // Pasted into bug reports: no fragment of the token.
        safe.plex.token = token_label(&safe.plex.token).to_string();
        let mut out = BTreeMap::new();
        if let Ok(table) = toml::Table::try_from(&safe) {
            flatten_settings(&table, "", &mut out);
        }
        // Keybindings are not settings; `settings_list` appends them.
        out.retain(|key, _| !key.starts_with("keys."));
        // An unset colour is the theme's: show what is in force.
        let (effective, _) = crate::ui::theme::appearance_from_config(&self.theme);
        for role in ThemeConfig::ROLES {
            if let Some(text) = crate::ui::theme::role_text(&effective.theme, role) {
                out.entry(format!("theme.{role}")).or_insert(text);
            }
        }
        for key in ThemeConfig::SPACING
            .iter()
            .chain(&ThemeConfig::SPACING_TEXT)
            .chain(&ThemeConfig::SYMBOLS)
        {
            if let Some(text) = crate::ui::theme::look_text(&effective.look, key) {
                out.entry(format!("theme.{key}")).or_insert(text);
            }
        }
        out
    }
}

/// Flattened defaults: the shipped config does not change at runtime.
fn shipped_settings() -> &'static BTreeMap<String, String> {
    static SHIPPED: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    SHIPPED.get_or_init(|| Config::default().flat_settings())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PlexConfig;

    #[test]
    fn settings_list_marks_defaults() {
        let mut cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "abcdef123456".into(),
                music_section: "3".into(),
                ..Default::default()
            },
            ..Config::default()
        };
        cfg.ui.art_pct = 55;
        let list = cfg.settings_list();

        // A changed tunable shows both values and no `= default` marker.
        let changed = list
            .lines()
            .find(|l| l.trim_start().starts_with("ui.art_pct"))
            .expect("ui.art_pct is listed");
        assert!(changed.contains("55"), "{changed}");
        assert!(changed.contains("40"), "{changed}");
        assert!(!changed.contains("= default"), "{changed}");

        // An untouched one is marked instead.
        let unchanged = list
            .lines()
            .find(|l| l.trim_start().starts_with("ui.scrolloff"))
            .expect("ui.scrolloff is listed");
        assert!(unchanged.contains("= default"), "{unchanged}");

        // Nested sections are flattened into dotted keys.
        assert!(list.contains("album_art.cache_mb"));
    }

    #[test]
    fn settings_list_shows_bindings_and_defaults() {
        let cfg = Config::parse("[keys.queue]\nremove = [\"Delete\"]\nundo = []\n").unwrap();
        let list = cfg.settings_list();
        let line = |key: &str| {
            list.lines()
                .find(|l| l.trim_start().starts_with(key))
                .unwrap_or_else(|| panic!("{key} is listed\n{list}"))
                .to_string()
        };

        let remove = line("keys.queue.remove");
        assert!(remove.contains("[Delete]") && remove.contains("[d]"), "{remove}");
        assert!(!remove.contains("= default"), "{remove}");

        let undo = line("keys.queue.undo");
        assert!(undo.contains("[]") && undo.contains("[u]"), "{undo}");
        assert!(!undo.contains("= default"), "{undo}");

        let quit = line("keys.global.quit");
        assert!(quit.contains("= default"), "{quit}");

        // not settings: the config screen does not edit them
        assert!(cfg.setting_rows().iter().all(|row| !row.key.starts_with("keys.")));
    }

    #[test]
    fn required_settings_have_no_default() {
        let cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "fake-token".into(),
                music_section: "3".into(),
                ..Default::default()
            },
            ..Config::default()
        };
        let rows = cfg.setting_rows();
        for key in ["plex.server", "plex.token", "plex.music_section"] {
            let row = rows.iter().find(|r| r.key == key).expect("listed");
            assert_eq!(row.default, None, "{key}");
            assert!(!row.is_default(), "{key}");
        }
        let scrolloff = rows.iter().find(|r| r.key == "ui.scrolloff").expect("listed");
        assert!(scrolloff.is_default());
    }

    /// An override equal to the theme's colour still counts as a change: it
    /// would stay pinned when the theme changes.
    #[test]
    fn equal_override_is_still_a_change() {
        let row = |accent: Option<&str>| {
            let mut cfg = Config::default();
            cfg.theme.accent = accent.map(String::from);
            cfg.setting_rows().into_iter().find(|r| r.key == "theme.accent").expect("listed")
        };
        assert!(row(None).is_default());
        for same in ["#7aa2f7", "#7AA2F7", "7aa2f7"] {
            assert!(!row(Some(same)).is_default(), "{same}");
        }
    }

    #[test]
    fn whole_float_displays_without_decimal() {
        assert_eq!(setting_text(&toml::Value::Float(100.0)), "100");
        assert_eq!(setting_text(&toml::Value::Float(70.5)), "70.5");
    }

    #[test]
    fn list_prints_dash_without_default() {
        let cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "fake-token".into(),
                music_section: "3".into(),
                ..Default::default()
            },
            ..Config::default()
        };
        let list = cfg.settings_list();
        for key in ["plex.server", "plex.token", "plex.music_section"] {
            let line =
                list.lines().find(|l| l.split_whitespace().next() == Some(key)).expect("listed");
            assert!(line.ends_with('—'), "{line}");
            assert!(!line.contains("= default"), "{line}");
        }
        let backend =
            list.lines().find(|l| l.trim_start().starts_with("album_art.backend")).expect("listed");
        assert!(backend.contains("= default"), "{backend}");
    }

    #[test]
    fn settings_list_never_prints_token() {
        let cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "s3cr3t-token-value".into(),
                ..PlexConfig::default()
            },
            ..Config::default()
        };
        let list = cfg.settings_list();
        assert!(!list.contains("s3cr3t-token-value"), "{list}");
        assert!(!list.contains("s3cr"), "not even a prefix: {list}");
        assert!(list.contains("(set)"), "{list}");

        // A URL pasted from Plex Web can carry the token and a password.
        let cfg = Config {
            plex: PlexConfig {
                server: "https://me:hunter2@host:32400/?X-Plex-Token=s3cr3t-in-url".into(),
                ..cfg.plex.clone()
            },
            ..cfg
        };
        let list = cfg.settings_list();
        assert!(!list.contains("s3cr3t-in-url") && !list.contains("hunter2"), "{list}");
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("s3cr3t-in-url") && !debug.contains("hunter2"), "{debug}");
    }

    #[test]
    fn restoring_defaults_resets_tunables_but_keeps_access() {
        let mut cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "secret".into(),
                music_section: "3".into(),
                ..Default::default()
            },
            ..Config::default()
        };
        cfg.ui.art_pct = 55;
        cfg.cava.framerate = 90;

        let changes = cfg.restore_defaults();
        assert_eq!(cfg.ui.art_pct, 40);
        assert_eq!(cfg.cava.framerate, 60);
        assert_eq!(cfg.plex.server, "http://host:32400");
        assert_eq!(cfg.plex.token, "secret");
        assert_eq!(cfg.plex.music_section, "3");

        let keys: Vec<&str> = changes.iter().map(|c| c.key.as_str()).collect();
        assert!(keys.contains(&"ui.art_pct"), "{keys:?}");
        assert!(keys.contains(&"cava.framerate"), "{keys:?}");
        assert!(!keys.contains(&"plex.server"), "{keys:?}");
        assert!(!keys.contains(&"plex.token"), "{keys:?}");
        assert!(!keys.contains(&"plex.music_section"), "{keys:?}");
        // Nothing differs from the default any more, so a second restore is a
        // no-op (and the editor says so instead of listing changes).
        assert!(cfg.restore_defaults().is_empty());
    }

    /// `background` is judged against the chosen theme like the colours, with
    /// `none` for a theme that has no background.
    #[test]
    fn background_listed_and_restored_against_theme() {
        let row = |cfg: &Config| {
            cfg.setting_rows().into_iter().find(|r| r.key == "theme.background").unwrap()
        };
        let mut cfg = Config::default();
        let r = row(&cfg);
        assert_eq!(
            (r.current.as_str(), r.default.as_deref(), r.is_default()),
            ("none", Some("none"), true)
        );
        assert_eq!(cfg.flat_settings()["theme.background"], "none");

        cfg.theme.name = "nord".into();
        let r = row(&cfg);
        assert_eq!(
            (r.current.as_str(), r.default.as_deref(), r.is_default()),
            ("#2e3440", Some("#2e3440"), true)
        );

        // none is a real override of a theme that has one; so is the same colour.
        cfg.theme.background = Some("none".into());
        let r = row(&cfg);
        assert_eq!((r.current.as_str(), r.is_default()), ("none", false));
        let change = cfg.restore_one("theme.background").expect("an override was dropped");
        assert_eq!((change.from.as_str(), change.to.as_str()), ("none", "#2e3440"));
        cfg.theme.background = Some("#2e3440".into());
        assert!(!row(&cfg).is_default(), "an override equal to the theme's still counts");
        assert!(cfg.restore_one("theme.background").is_some());
        assert_eq!(cfg.theme.background, None);
    }
}
