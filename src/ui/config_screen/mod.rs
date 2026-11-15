//! The config screen: the settings as a form that looks like the player. Not
//! part of `App`: it has no Plex client, mpv or playback state.

use crate::config::{Config, SettingRow};
use crate::doctor::{self, Report};
use crate::inventory::{self, Kind, Setting};
use crate::plex::models::Section;
use crate::ui::filter::{self, ListFilter};
use crate::ui::text_input::TextInput;
use crate::ui::theme;
use crate::ui::themes;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Config,
    Theme,
    Diagnostics,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Config, Tab::Theme, Tab::Diagnostics];

    fn name(self) -> &'static str {
        match self {
            Tab::Config => "Config",
            Tab::Theme => "Theme",
            Tab::Diagnostics => "Diagnostics",
        }
    }
}

/// A quit waiting on a y/n answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitAsk {
    /// Nothing is pending: "quit? (y/n)".
    Clean,
    /// There are unsaved changes: save, or leave them.
    Dirty,
}

type CheckSlot = std::sync::Arc<std::sync::Mutex<Option<Result<Report, String>>>>;

/// What the editor pane is doing, if anything.
enum Editor {
    /// A text field: text, number, colour.
    Field {
        text: TextInput,
        /// What the field opened with. Committing exactly this changes nothing:
        /// the text is a lossy view of some values.
        initial: String,
        /// Masked as typed, and starts empty: the list only has a redacted
        /// placeholder, which must never replace the real secret.
        secret: bool,
    },
    /// One of a set: an enum, or the platform's cava sources.
    Picker {
        options: Vec<String>,
        index: usize,
        /// The themes behind `options` when this is the theme chooser.
        themes: Vec<themes::Entry>,
    },
    /// The file id for an exported theme (`x` on the Theme tab).
    Export { text: TextInput },
    /// `plex.music_section`: the libraries the server lists.
    Library(LibraryPick),
    /// `mpv.extra_args`: one argument per row.
    List(ListEdit),
}

enum Libraries {
    Loading,
    Loaded(Vec<Section>),
    /// Short, and free of the token and the URL.
    Failed(String),
}

/// The empty id (the first library found) is always the first choice and
/// "type an id" the last, so an unreachable server never locks the setting.
struct LibraryPick {
    libraries: Libraries,
    /// What `plex.music_section` holds now.
    current: String,
    /// An index into `choices()`.
    index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    /// The id to store (empty: the first library found), and its label.
    Id(String, String),
    /// Open the text field.
    Type,
}

impl LibraryPick {
    fn choices(&self) -> Vec<Choice> {
        let mut out = vec![Choice::Id(String::new(), "first one found".into())];
        let mut known = false;
        if let Libraries::Loaded(sections) = &self.libraries {
            for s in sections {
                known |= s.key == self.current;
                out.push(Choice::Id(s.key.clone(), format!("{}  · id {}", s.title, s.key)));
            }
        }
        if !self.current.is_empty() && !known {
            let note = match self.libraries {
                Libraries::Loaded(_) => "not found",
                _ => "current",
            };
            out.push(Choice::Id(self.current.clone(), format!("{} ({note})", self.current)));
        }
        out.push(Choice::Type);
        out
    }

    fn index_of_current(&self) -> usize {
        let at = self
            .choices()
            .iter()
            .position(|c| matches!(c, Choice::Id(id, _) if *id == self.current));
        at.unwrap_or(0)
    }
}

struct ListEdit {
    rows: Vec<String>,
    /// A finish with the same rows changes nothing.
    initial: Vec<String>,
    cursor: usize,
    /// The row being edited, and whether it is new (a cancelled new row goes).
    editing: Option<(TextInput, bool)>,
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

/// Looks the music libraries up for a server and token; `Err` is a short
/// reason. A field so tests can stand in for the network.
type LibraryLookup =
    std::sync::Arc<dyn Fn(String, String) -> Result<Vec<Section>, String> + Send + Sync>;

const LIBRARY_TIMEOUT: Duration = Duration::from_secs(5);

/// The changes a save would write.
struct Review {
    /// Key, old, new, and whether it is a secret (shown only as `(changed)`;
    /// its values are not kept here).
    rows: Vec<(String, String, String, bool)>,
    scroll: usize,
    /// Rows the panel showed at the last draw.
    visible: usize,
}

/// A drawn row: a group heading, or a setting by its position in `Screen::view`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Header(&'static str),
    Item(usize),
}

/// The last option of a preset picker: type the glyphs instead.
const CUSTOM: &str = "custom…";

/// The label column stops growing when the value column would drop below this:
/// room for a swatch and its hex.
const VALUE_MIN: usize = 12;
const LABEL_MIN: usize = 6;
const LIST_LEAD: usize = 1;
const LIST_GAP: usize = 2;

pub struct Screen {
    path: PathBuf,
    cfg: Config,
    /// The config as last written: the baseline for saves and the dirty check.
    saved: Config,
    tab: Tab,
    /// An index into `view()`: the settings shown, never a heading.
    cursor: usize,
    /// Its indices are positions in `visible()`; dropped when the tab changes.
    filter: ListFilter,
    /// In chars of `filter.text`.
    filter_cursor: usize,
    scroll: crate::ui::scroll::ScrollStore,
    /// The player's setting, so the screen scrolls like the player.
    scrolloff: usize,
    diag_scroll: usize,
    /// The report pane's height and scroll range, recorded when it draws:
    /// only the pane knows them, and paging and `End` need them.
    diag_visible: usize,
    diag_max: usize,
    editor: Option<Editor>,
    status: String,
    status_error: bool,
    report: Option<Report>,
    /// Why the last check produced no report.
    check_error: Option<String>,
    /// `q` and Ctrl-C both ask: a stray keypress must not lose edits.
    quit_ask: Option<QuitAsk>,
    /// A save paused on keys changed both here and in the file. Keys only.
    conflict_ask: Option<Vec<String>>,
    /// `R` waiting on y/n: how many settings it would change.
    restore_ask: Option<usize>,
    /// The checks run on their own thread: the Plex client's timeouts and
    /// retries would freeze the screen for most of a minute.
    check_slot: CheckSlot,
    /// Kept to notice a thread that ended without filling the slot.
    check_thread: Option<std::thread::JoinHandle<()>>,
    checking: bool,
    /// The thread sends once and ends; `draw` polls the channel.
    library_lookup: LibraryLookup,
    library_rx: Option<std::sync::mpsc::Receiver<Result<Vec<Section>, String>>>,
    review: Option<Review>,
}

impl Screen {
    pub fn new(path: PathBuf, cfg: Config) -> Self {
        let scrolloff = cfg.ui.scrolloff;
        Self {
            path,
            saved: cfg.clone(),
            cfg,
            tab: Tab::Config,
            cursor: 0,
            filter: ListFilter::default(),
            filter_cursor: 0,
            scroll: crate::ui::scroll::ScrollStore::default(),
            scrolloff,
            diag_scroll: 0,
            diag_visible: 0,
            diag_max: 0,
            editor: None,
            status: String::new(),
            status_error: false,
            report: None,
            check_error: None,
            quit_ask: None,
            conflict_ask: None,
            restore_ask: None,
            check_slot: std::sync::Arc::new(std::sync::Mutex::new(None)),
            check_thread: None,
            checking: false,
            library_lookup: std::sync::Arc::new(|server, token| {
                fetch_libraries(&server, &token, LIBRARY_TIMEOUT)
            }),
            library_rx: None,
            review: None,
        }
    }

    /// Runs until the user quits. Returns the config as left, saved or not.
    pub fn run(&mut self, terminal: &mut crate::ui::term::Term) -> Result<Config> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) => {
                    if !self.handle(key)? {
                        return Ok(self.cfg.clone());
                    }
                }
                Event::Paste(text) => self.handle_paste(text),
                _ => {}
            }
        }
    }

    fn visible(&self) -> Vec<&'static Setting> {
        match self.tab {
            Tab::Config => {
                inventory::ALL.iter().filter(|s| !inventory::is_theme_group(s.group)).collect()
            }
            Tab::Theme => {
                inventory::ALL.iter().filter(|s| inventory::is_theme_group(s.group)).collect()
            }
            Tab::Diagnostics => Vec::new(),
        }
    }

    /// The settings shown: the tab's, narrowed by the filter. `cursor` indexes this.
    fn view(&self) -> Vec<&'static Setting> {
        let all = self.visible();
        if !self.filter.is_active() {
            return all;
        }
        self.filter.indices.iter().filter_map(|&i| all.get(i).copied()).collect()
    }

    fn selected(&self) -> Option<&'static Setting> {
        self.view().get(self.cursor).copied()
    }

    /// The cursor as a position in `visible()`, filter or not.
    fn real_cursor(&self) -> usize {
        if self.filter.is_active() {
            self.filter.indices.get(self.cursor).copied().unwrap_or(0)
        } else {
            self.cursor
        }
    }

    /// Re-applies the filter, keeping the cursor on its setting or the next
    /// match. Read the position before the text changes.
    fn edit_filter(&mut self, edit: impl FnOnce(&mut String)) {
        let old = self.real_cursor();
        edit(&mut self.filter.text);
        self.refilter(old);
    }

    fn refilter(&mut self, old: usize) {
        let all = self.visible();
        let needle = self.filter.text.clone();
        self.filter.apply(all.len(), |i| {
            let s = all[i];
            filter::matches(&format!("{} {} {}", s.label, s.key, s.help), &needle)
        });
        self.cursor = if self.filter.is_active() {
            self.filter
                .indices
                .iter()
                .position(|&i| i >= old)
                .unwrap_or(self.filter.indices.len().saturating_sub(1))
        } else {
            old.min(all.len().saturating_sub(1))
        };
    }

    fn clear_filter(&mut self) {
        let old = self.real_cursor();
        self.filter.clear();
        self.filter_cursor = 0;
        self.cursor = old.min(self.visible().len().saturating_sub(1));
    }

    /// A group with no setting in view has no heading.
    fn rows(&self, view: &[&'static Setting]) -> Vec<Row> {
        let mut rows = Vec::with_capacity(view.len() + inventory::GROUPS.len());
        let groups = self.visible();
        let headed = groups.iter().any(|s| s.group != groups[0].group);
        for (i, setting) in view.iter().enumerate() {
            let first = i == 0 || view[i - 1].group != setting.group;
            if first && headed {
                rows.push(Row::Header(setting.group));
            }
            rows.push(Row::Item(i));
        }
        rows
    }

    /// `[` and `]`: the first setting of the previous or next group in view.
    fn jump_group(&mut self, dir: i32) {
        let view = self.view();
        let Some(here) = view.get(self.cursor) else {
            return;
        };
        let start_of = |mut i: usize| {
            while i > 0 && view[i - 1].group == view[i].group {
                i -= 1;
            }
            i
        };
        if dir > 0 {
            if let Some(next) = (self.cursor..view.len()).find(|&i| view[i].group != here.group) {
                self.cursor = next;
            }
        } else {
            let start = start_of(self.cursor);
            if start > 0 {
                self.cursor = start_of(start - 1);
            }
        }
    }

    fn values(&self) -> std::collections::BTreeMap<String, SettingRow> {
        self.cfg.setting_rows().into_iter().map(|row| (row.key.clone(), row)).collect()
    }

    /// Marked `*`. A setting with no default is never marked.
    fn is_changed(
        &self,
        setting: &Setting,
        values: &std::collections::BTreeMap<String, SettingRow>,
    ) -> bool {
        values
            .get(setting.key)
            .map(|row| row.default.is_some() && !row.is_default())
            .unwrap_or(false)
    }

    fn value(&self, setting: &Setting) -> String {
        self.cfg
            .setting_rows()
            .into_iter()
            .find(|row| row.key == setting.key)
            .map(|row| row.current)
            .unwrap_or_default()
    }

    fn unused_because(
        &self,
        setting: &Setting,
        values: &std::collections::BTreeMap<String, SettingRow>,
    ) -> Option<&'static str> {
        setting.unused_because(|key| values.get(key).map(|row| row.current.clone()))
    }

    fn scroll_id(&self) -> crate::ui::scroll::ScrollId {
        match self.tab {
            Tab::Theme => crate::ui::scroll::ScrollId::ConfigTheme,
            Tab::Diagnostics => crate::ui::scroll::ScrollId::ConfigChecks,
            Tab::Config => crate::ui::scroll::ScrollId::ConfigSettings,
        }
    }

    fn changes_on(tab: Tab, changed: &[String]) -> usize {
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

    fn open_review(&mut self) {
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

    fn handle_review(&mut self, key: KeyEvent) {
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

    fn dirty(&self) -> bool {
        // Not `setting_rows()`: it redacts the token, so a new one would
        // compare equal to the old.
        toml::to_string(&self.cfg).ok() != toml::to_string(&self.saved).ok()
    }

    /// True when the file was written. Only the screen's edits are written, so
    /// other changes made meanwhile survive.
    fn save(&mut self) -> bool {
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

    fn handle_conflict_ask(&mut self, key: KeyEvent) {
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

    fn reset_selected(&mut self) {
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

    fn ask_to_restore(&mut self) {
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

    fn handle_restore_ask(&mut self, key: KeyEvent) {
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

    fn apply_theme(&mut self) {
        let (appearance, problems) = theme::appearance_from_config(&self.cfg.theme);
        theme::set_appearance(appearance, self.cfg.ui.ascii_icons());
        if let Some(problem) = problems.first() {
            // Only the first; startup prints the whole list to stderr.
            self.status = problem.clone();
        }
    }

    /// Previews the theme without the user's overrides: choosing it clears them.
    fn preview_theme(&self, id: &str, themes: &[themes::Entry]) {
        if let Some(entry) = themes.iter().find(|t| t.id == id) {
            let appearance = theme::Appearance { theme: entry.theme, look: entry.look };
            theme::set_appearance(appearance, self.cfg.ui.ascii_icons());
        }
    }

    fn begin_export(&mut self) {
        if self.cfg.theme.dir.is_none() {
            self.status = "no themes directory to export to".into();
            self.status_error = true;
            return;
        }
        self.editor = Some(Editor::Export { text: TextInput::new("my-theme") });
    }

    /// Writes the palette in force (theme plus overrides) as a theme file.
    fn export_theme(&mut self, id: &str) -> bool {
        let Some(dir) = self.cfg.theme.dir.clone() else {
            return false;
        };
        let variant = themes::available(Some(&dir))
            .entries
            .iter()
            .find(|e| e.id == self.cfg.theme.name)
            .and_then(|e| e.variant);
        let effective = theme::appearance_from_config(&self.cfg.theme).0;
        match themes::export(&dir, id, variant, &effective.theme, &effective.look) {
            Ok(_) => {
                self.status = format!("exported theme {id}; it is in the chooser now");
                self.status_error = false;
                true
            }
            Err(e) => {
                self.status = format!("not exported: {e}");
                self.status_error = true;
                false
            }
        }
    }

    /// The Plex client retries transport errors with 15-second timeouts, so
    /// the checks run off the UI thread.
    fn start_check(&mut self) {
        if self.checking {
            return;
        }
        self.checking = true;
        self.check_error = None;
        let slot = self.check_slot.clone();
        let path = self.path.clone();
        let cfg = self.cfg.clone();
        self.check_thread = Some(std::thread::spawn(move || {
            let result = match tokio::runtime::Runtime::new() {
                Ok(rt) => Ok(rt.block_on(doctor::collect(&cfg, &path, false))),
                Err(e) => Err(format!("could not start the checks: {e}")),
            };
            if let Ok(mut guard) = slot.lock() {
                *guard = Some(result);
            }
        }));
    }

    fn poll_check(&mut self) {
        if !self.checking {
            return;
        }
        let landed = self.check_slot.lock().ok().and_then(|mut guard| guard.take());
        match landed {
            Some(Ok(report)) => {
                self.report = Some(report);
                self.checking = false;
            }
            Some(Err(e)) => {
                self.check_error = Some(e);
                self.checking = false;
            }
            None => {
                // The thread panicked; otherwise "checking …" would stay forever.
                if self.check_thread.as_ref().is_some_and(|t| t.is_finished()) {
                    self.check_error =
                        Some("the checks stopped unexpectedly; press d to try again".into());
                    self.checking = false;
                }
            }
        }
        if !self.checking {
            self.check_thread = None;
        }
    }

    fn handle(&mut self, key: KeyEvent) -> Result<bool> {
        if self.editor.is_some() {
            return self.handle_editor(key);
        }
        if self.review.is_some() {
            self.handle_review(key);
            return Ok(true);
        }
        if self.conflict_ask.is_some() {
            self.handle_conflict_ask(key);
            return Ok(true);
        }
        if self.restore_ask.is_some() {
            self.handle_restore_ask(key);
            return Ok(true);
        }
        if let Some(ask) = self.quit_ask {
            return Ok(self.handle_quit_ask(ask, key));
        }
        self.status.clear();
        self.status_error = false;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.ask_to_quit();
            return Ok(true);
        }
        if self.filter.typing {
            self.handle_filter(key);
            return Ok(true);
        }
        match key.code {
            KeyCode::Char('/') if self.tab != Tab::Diagnostics => {
                self.filter.typing = true;
                self.filter_cursor = self.filter.text.chars().count();
            }
            KeyCode::Esc if self.filter.is_active() => self.clear_filter(),
            KeyCode::Char('[') => self.jump_group(-1),
            KeyCode::Char(']') => self.jump_group(1),
            KeyCode::Char('q') => {
                self.ask_to_quit();
            }
            KeyCode::Char('s') => self.open_review(),
            KeyCode::Char('r') => self.reset_selected(),
            KeyCode::Char('R') => self.ask_to_restore(),
            KeyCode::Char('d') => self.start_check(),
            KeyCode::Char('x') if self.tab == Tab::Theme => self.begin_export(),
            KeyCode::Char('1') => self.switch(Tab::Config),
            KeyCode::Char('2') => self.switch(Tab::Theme),
            KeyCode::Char('3') => self.switch(Tab::Diagnostics),
            KeyCode::Tab => self.switch_next(),
            KeyCode::BackTab => self.switch_prev(),
            KeyCode::Down | KeyCode::Char('j') if self.tab == Tab::Diagnostics => {
                self.diag_scroll = self.diag_scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') if self.tab == Tab::Diagnostics => {
                self.diag_scroll = self.diag_scroll.saturating_sub(1);
            }
            KeyCode::PageDown if self.tab == Tab::Diagnostics => {
                let page = self.diag_visible.max(1);
                self.diag_scroll = self.diag_scroll.saturating_add(page);
            }
            KeyCode::PageUp if self.tab == Tab::Diagnostics => {
                let page = self.diag_visible.max(1);
                self.diag_scroll = self.diag_scroll.saturating_sub(page);
            }
            KeyCode::Home if self.tab == Tab::Diagnostics => self.diag_scroll = 0,
            KeyCode::End if self.tab == Tab::Diagnostics => {
                self.diag_scroll = self.diag_max;
            }
            KeyCode::Right | KeyCode::Char('l') if self.tab != Tab::Diagnostics => self.step(1),
            KeyCode::Left | KeyCode::Char('h') if self.tab != Tab::Diagnostics => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown => self.page(1),
            KeyCode::PageUp => self.page(-1),
            KeyCode::Home => {
                self.cursor = 0;
                self.status.clear();
            }
            KeyCode::End => {
                self.cursor = self.view().len().saturating_sub(1);
                self.status.clear();
            }
            KeyCode::Enter if self.tab == Tab::Diagnostics => self.start_check(),
            KeyCode::Enter => {
                if let Some(setting) = self.selected() {
                    self.begin_edit(setting);
                }
            }
            _ => {}
        }
        // Opening the tab runs the checks; `d` re-runs them.
        if self.tab == Tab::Diagnostics
            && self.report.is_none()
            && self.check_error.is_none()
            && !self.checking
        {
            self.start_check();
        }
        Ok(true)
    }

    /// With edits pending the question offers to save them.
    fn ask_to_quit(&mut self) {
        if self.dirty() {
            self.quit_ask = Some(QuitAsk::Dirty);
            self.status = "unsaved changes: s saves and quits, y quits without, Esc stays".into();
            self.status_error = true;
        } else {
            self.quit_ask = Some(QuitAsk::Clean);
            self.status = "quit? (y/n)".into();
            self.status_error = false;
        }
    }

    fn handle_quit_ask(&mut self, ask: QuitAsk, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => false,
            // Too easy to hit by reflex with edits pending: discarding takes `y`.
            KeyCode::Enter | KeyCode::Char('q') if ask == QuitAsk::Clean => false,
            // A wedged screen still needs an exit.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => false,
            KeyCode::Char('s') if ask == QuitAsk::Dirty => {
                // A failed or conflicted save keeps the screen open.
                !self.save()
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.quit_ask = None;
                self.status.clear();
                self.status_error = false;
                true
            }
            _ => true,
        }
    }

    fn switch(&mut self, tab: Tab) {
        self.tab = tab;
        self.editor = None;
        self.cursor = 0;
        self.filter.clear();
    }

    /// Enter keeps the filter and hands the keys back; Esc, or deleting the
    /// last character, drops it.
    fn handle_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.clear_filter(),
            KeyCode::Enter if self.filter.text.is_empty() => self.clear_filter(),
            KeyCode::Enter => self.filter.typing = false,
            KeyCode::Backspace if self.filter.text.is_empty() => self.clear_filter(),
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            _ => {
                let mut input = TextInput::at(self.filter.text.clone(), self.filter_cursor);
                let changed = input.handle(key);
                self.filter_cursor = input.cursor();
                if changed {
                    self.edit_filter(|t| *t = input.into_text());
                    if self.filter.text.is_empty() {
                        self.clear_filter();
                    }
                }
            }
        }
    }

    fn switch_next(&mut self) {
        let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.switch(Tab::ALL[(i + 1) % Tab::ALL.len()]);
    }

    fn switch_prev(&mut self) {
        let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.switch(Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()]);
    }

    fn move_cursor(&mut self, by: i32) {
        let len = self.view().len();
        if len == 0 {
            return;
        }
        let next = (self.cursor as i32 + by).clamp(0, len as i32 - 1);
        self.cursor = next as usize;
    }

    /// A page is the pane as measured at the last draw.
    fn page(&mut self, dir: i32) {
        let visible = self.scroll.visible(self.scroll_id()).max(1);
        self.move_cursor(dir * visible as i32);
    }

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

    fn begin_edit(&mut self, setting: &'static Setting) {
        let current = self.value(setting);
        match setting.kind {
            Kind::Bool => {
                let on = current.trim() != "true";
                self.set(setting, if on { "true" } else { "false" });
            }
            Kind::Secret => {
                // Start empty: the list holds a `(set)` placeholder that must
                // never replace the secret. Empty means keep it.
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

    /// `←`/`→`: step the selected setting without opening it, through `set`
    /// like a commit. Kinds with no sensible step do nothing.
    fn step(&mut self, dir: i32) {
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

    /// Every key belongs to the editor while it is open. The status is not
    /// cleared here: it belongs to the value being edited.
    fn handle_editor(&mut self, key: KeyEvent) -> Result<bool> {
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

    /// Looks up with the server and token as the screen holds them (unsaved
    /// edits count), on its own thread so a dead server cannot freeze it.
    fn begin_library(&mut self, current: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let lookup = self.library_lookup.clone();
        let (server, token) = (self.cfg.plex.server.clone(), self.cfg.plex.token.clone());
        std::thread::spawn(move || {
            let _ = tx.send(lookup(server, token));
        });
        self.library_rx = Some(rx);
        let mut pick = LibraryPick { libraries: Libraries::Loading, current, index: 0 };
        pick.index = pick.index_of_current();
        self.editor = Some(Editor::Library(pick));
    }

    fn poll_library(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.library_rx else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            // The thread panicked.
            Err(TryRecvError::Disconnected) => Err("the lookup stopped unexpectedly".into()),
        };
        self.library_rx = None;
        if let Some(Editor::Library(pick)) = &mut self.editor {
            // Keep the cursor on the same id: rows come and go.
            let was = pick.choices().get(pick.index).cloned();
            pick.libraries = match result {
                Ok(sections) => Libraries::Loaded(sections),
                Err(reason) => Libraries::Failed(reason),
            };
            let at = was.and_then(|was| {
                pick.choices().iter().position(|c| match (c, &was) {
                    (Choice::Id(a, _), Choice::Id(b, _)) => a == b,
                    (Choice::Type, Choice::Type) => true,
                    _ => false,
                })
            });
            pick.index = at.unwrap_or(0);
        }
    }

    fn library_key(&mut self, mut pick: LibraryPick, key: KeyEvent) {
        let choices = pick.choices();
        match key.code {
            KeyCode::Esc => {
                self.library_rx = None;
                self.status.clear();
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.library_rx = None;
                self.status.clear();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                pick.index = (pick.index + 1).min(choices.len().saturating_sub(1));
                self.editor = Some(Editor::Library(pick));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                pick.index = pick.index.saturating_sub(1);
                self.editor = Some(Editor::Library(pick));
            }
            KeyCode::Char('t') => self.type_library_id(pick.current),
            KeyCode::Enter => match choices.get(pick.index) {
                Some(Choice::Id(id, _)) => {
                    self.library_rx = None;
                    if let Some(setting) = self.selected() {
                        self.set(setting, id);
                    }
                }
                _ => self.type_library_id(pick.current),
            },
            _ => self.editor = Some(Editor::Library(pick)),
        }
    }

    /// For a server that cannot be asked, or an id the list lacks.
    fn type_library_id(&mut self, current: String) {
        self.library_rx = None;
        self.editor = Some(Editor::Field {
            initial: current.clone(),
            text: TextInput::new(current),
            secret: false,
        });
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

    /// A paste arrives as one event. Control characters are dropped so a
    /// trailing newline is not stored.
    fn handle_paste(&mut self, pasted: String) {
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
        let (Some(Editor::Field { text, .. }) | Some(Editor::Export { text })) = &mut self.editor
        else {
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

    /// A value that does not fit becomes a status line, not an error.
    /// False when the value was rejected.
    fn set(&mut self, setting: &Setting, text: &str) -> bool {
        // An untouched secret field commits empty, which must keep the token
        // rather than write `plex.token = ""`.
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

    fn draw(&mut self, f: &mut Frame) {
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

        // The sample under the header shows only when the colour list keeps
        // its thirteen rows.
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
        // Sized by the longest label on the tab, not the ones in view, so a
        // filter does not move it; a longer label is ellipsized.
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
                        // A colour is shown as itself, not only as hex.
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
        // On a narrow screen the least important hints go first.
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

/// The footer's hints, least important first, dropped in this order when they
/// do not fit.
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

/// The player's header with placeholder text, drawn by the same code and in
/// the borders and header mode in force.
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
    };
    if ui.compact_header() {
        views::draw_tab_row(
            f,
            header,
            style,
            &["Queue", "Browser", "Playlists", "Search", "Help"],
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

/// Queue rows (focused and unfocused cursor, a filter match, a plain row) and a
/// visualizer strip, for the colours the header does not use.
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

/// The failure reason names only the kind: the error carries the request URL,
/// which carries the token.
fn fetch_libraries(server: &str, token: &str, timeout: Duration) -> Result<Vec<Section>, String> {
    if server.trim().is_empty() {
        return Err("no server set yet".into());
    }
    let rt =
        tokio::runtime::Runtime::new().map_err(|_| "could not start the lookup".to_string())?;
    rt.block_on(async {
        let client = crate::plex::client::PlexClient::new(server, token);
        match tokio::time::timeout(timeout, client.music_sections()).await {
            Err(_) => Err("the server did not answer in time".to_string()),
            Ok(Err(_)) => {
                Err("could not ask the server (unreachable, or the token is refused)".to_string())
            }
            Ok(Ok(sections)) => Ok(sections),
        }
    })
}

/// Where a window of `rows` rows starts so `index` is in it, centred if possible.
fn window_start(len: usize, index: usize, rows: usize) -> usize {
    index.saturating_sub(rows / 2).min(len.saturating_sub(rows))
}

/// `key  old → new`, each value cut to its share of the width.
fn review_line(key: &str, old: &str, new: &str, secret: bool, width: usize) -> String {
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

/// A value the list does not offer (hand-typed, or no longer offered) is
/// added, so the highlight is the value in effect, not silently the first.
fn pick_index(options: &mut Vec<String>, current: &str) -> usize {
    if let Some(index) = options.iter().position(|option| option == current) {
        return index;
    }
    options.push(current.to_string());
    options.len() - 1
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

fn value_for(kind: Kind, text: &str) -> Result<toml::Value> {
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

/// Uses the rule shared with theme files and `shmc config --check`. Not
/// trimmed: a space is a glyph.
fn symbol_value(key: &str, text: &str) -> Result<toml::Value> {
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
fn parse_colour(text: &str) -> Option<String> {
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

fn name_keys(keys: &[String]) -> String {
    let shown = keys.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    match keys.len().saturating_sub(3) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn ctx() -> Screen {
        ctx_for(std::path::PathBuf::from("/tmp/shmc.toml"), Config::default())
    }

    fn ctx_for(path: std::path::PathBuf, cfg: Config) -> Screen {
        Screen::new(path, cfg)
    }

    fn frame(screen: &mut Screen, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| screen.draw(f)).expect("draw");
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn config_tab_shows_settings_and_help() {
        let mut screen = ctx();
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("Config"), "{text}");
        assert!(text.contains("Diagnostics"), "{text}");
        assert!(text.contains("Plex server"), "{text}");
        assert!(text.contains("Where shmc connects"), "{text}");
        assert!(text.contains("s save"), "{text}");
    }

    #[test]
    fn theme_tab_shows_swatch_and_hex() {
        let mut screen = ctx();
        screen.tab = Tab::Theme;
        let text = frame(&mut screen, 100, 40);
        assert!(text.contains("Accent"), "{text}");
        assert!(text.contains("███"), "a colour is drawn as itself: {text}");
        assert!(text.contains("#7aa2f7"), "and as its hex: {text}");
        assert!(!text.contains("Plex server"), "{text}");
    }

    #[test]
    fn changed_value_marked_against_default() {
        let mut screen = ctx();
        screen.cfg.mpv.volume = 50.0;
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("50 *"), "a changed value is marked: {text}");
        assert!(!text.contains("(missing) *"), "an untouched value has no marker: {text}");
        screen.cfg.plex.server = "https://plex.example".into();
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("https://plex.example"), "{text}");
        assert!(!text.contains("https://plex.example *"), "{text}");
    }

    #[test]
    fn long_list_gets_scrollbar() {
        let mut screen = ctx();
        let text = frame(&mut screen, 100, 18);
        assert!(text.contains('▲'), "the scrollbar is drawn: {text}");
        assert!(text.contains('▼'), "{text}");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn screen_fits_narrow_and_short_terminal() {
        for (w, h) in [(100, 30), (80, 24), (60, 20), (100, 12), (44, 10)] {
            for tab in [Tab::Config, Tab::Theme, Tab::Diagnostics] {
                let mut screen = ctx();
                screen.tab = tab;
                let text = frame(&mut screen, w, h);
                assert!(text.contains("keys"), "{tab:?} at {w}x{h} lost the footer:\n{text}");
            }
            let mut screen = ctx();
            let text = frame(&mut screen, w, h);
            assert!(text.contains("Settings"), "{w}x{h} lost the pane title:\n{text}");
        }
    }

    /// A screen on a file holding `plex.server` and `plex.token`, with `ui.art_pct` set to 55.
    fn screen_with_an_edit() -> (tempfile::TempDir, PathBuf, Screen) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n")
            .expect("write");
        let cfg = Config::read(&path).expect("read");
        let mut screen = ctx_for(path.clone(), cfg);
        let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
        screen.set(setting, "55");
        (dir, path, screen)
    }

    fn append(path: &PathBuf, text: &str) {
        let mut body = std::fs::read_to_string(path).expect("read");
        body.push_str(text);
        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn outside_edit_survives_save() {
        let (_dir, path, mut screen) = screen_with_an_edit();
        append(&path, "[ui]\nscrolloff = 9\n");
        screen.handle(key(KeyCode::Char('s'))).expect("review");
        screen.handle(key(KeyCode::Char('s'))).expect("save");

        let on_disk = Config::read(&path).expect("read");
        assert_eq!(on_disk.ui.art_pct, 55);
        assert_eq!(on_disk.ui.scrolloff, 9);
        assert_eq!(screen.cfg.ui.scrolloff, 9, "the screen shows the file");
        assert!(!screen.dirty());
    }

    #[test]
    fn real_conflict_asks_and_answers_apply() {
        for answer in [KeyCode::Char('o'), KeyCode::Char('t'), KeyCode::Esc] {
            let (_dir, path, mut screen) = screen_with_an_edit();
            append(&path, "[ui]\nart_pct = 60\n");
            let asked = std::fs::read_to_string(&path).expect("read");

            screen.handle(key(KeyCode::Char('s'))).expect("review");
            screen.handle(key(KeyCode::Char('s'))).expect("save");
            assert!(screen.conflict_ask.is_some(), "{answer:?}: it asks");
            assert_eq!(std::fs::read_to_string(&path).expect("read"), asked, "nothing written yet");
            // Another key does nothing while it is up.
            screen.handle(key(KeyCode::Char('x'))).expect("key");
            assert!(screen.conflict_ask.is_some());

            screen.handle(key(answer)).expect("answer");
            assert!(screen.conflict_ask.is_none(), "{answer:?}: the prompt is gone");
            let on_disk = Config::read(&path).expect("read").ui.art_pct;
            match answer {
                KeyCode::Char('o') => {
                    assert_eq!((on_disk, screen.cfg.ui.art_pct), (55, 55));
                    assert!(!screen.dirty());
                }
                KeyCode::Char('t') => {
                    assert_eq!((on_disk, screen.cfg.ui.art_pct), (60, 60));
                    assert!(!screen.dirty());
                }
                _ => {
                    assert_eq!((on_disk, screen.cfg.ui.art_pct), (60, 55));
                    assert!(screen.dirty());
                }
            }
        }

        // The quit prompt's `s` with a conflict pending does not close the screen.
        let (_dir, path, mut screen) = screen_with_an_edit();
        append(&path, "[ui]\nart_pct = 60\n");
        screen.ask_to_quit();
        assert!(screen.handle_quit_ask(QuitAsk::Dirty, key(KeyCode::Char('s'))));
        assert!(screen.conflict_ask.is_some());
    }

    #[test]
    fn switching_to_diagnostics_leaves_the_status_alone() {
        // The pane's hint already says what to press.
        let mut screen = ctx();
        screen.switch(Tab::Diagnostics);
        assert!(screen.status.is_empty(), "{:?}", screen.status);
    }

    #[test]
    fn quitting_asks_first() {
        let mut screen = ctx();
        screen.ask_to_quit();
        assert_eq!(screen.quit_ask, Some(QuitAsk::Clean));
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("quit? (y/n)"), "{text}");
        assert!(text.contains("y quit"), "the footer offers the answer: {text}");

        assert!(screen.handle_quit_ask(QuitAsk::Clean, key(KeyCode::Char('n'))));
        assert_eq!(screen.quit_ask, None);

        screen.ask_to_quit();
        assert!(!screen.handle_quit_ask(QuitAsk::Clean, key(KeyCode::Char('y'))));
    }

    #[test]
    fn ctrl_c_asks_instead_of_quitting() {
        let mut screen = ctx();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(
            screen.handle(ctrl_c).expect("handle"),
            "ctrl-c asks; it must not close the screen"
        );
        assert_eq!(screen.quit_ask, Some(QuitAsk::Clean));

        // A second Ctrl-C is the way out of a wedged screen.
        assert!(!screen.handle_quit_ask(QuitAsk::Clean, ctrl_c));
    }

    #[test]
    fn unsaved_changes_offer_save_on_quit() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"t\"\n")
            .expect("write");
        let cfg = Config::read(&path).expect("read");
        let mut screen = ctx_for(path.clone(), cfg);
        let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
        screen.set(setting, "55");

        screen.ask_to_quit();
        assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("unsaved changes"), "{text}");
        assert!(text.contains("save and quit"), "{text}");

        assert!(!screen.handle_quit_ask(QuitAsk::Dirty, key(KeyCode::Char('s'))));
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.contains("art_pct = 55"), "{written}");
    }

    #[test]
    fn save_writes_file_and_keeps_backup() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        let original = "[plex]\nserver = \"http://old:32400\"\ntoken = \"t\"\n";
        std::fs::write(&path, original).expect("write");
        let cfg = Config::read(&path).expect("read");
        assert_eq!(cfg.ui.art_pct, 40);

        let mut screen = ctx_for(path.clone(), cfg);
        let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
        screen.set(setting, "55");
        screen.save();

        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.contains("art_pct = 55"), "the file has the edit: {written}");
        let backup = std::fs::read_to_string(dir.path().join("config.toml.bak")).expect("backup");
        assert_eq!(backup, original, "the backup is the file exactly as it was before the save");
        assert!(screen.status.starts_with("saved"), "the status says saved: {}", screen.status);
        assert!(!screen.dirty(), "a saved screen has nothing pending");
    }

    #[test]
    fn edit_result_is_visible() {
        let mut screen = ctx();
        let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
        screen.set(setting, "55");
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("ui.art_pct = 55"), "the status line says what happened: {text}");

        let mut rejected = ctx();
        rejected.set(setting, "700");
        let text = frame(&mut rejected, 100, 30);
        assert!(text.contains("rejected"), "{text}");
        assert!(rejected.status_error, "a rejection is an error status");
    }

    #[test]
    fn rejected_value_keeps_field_open() {
        let mut screen = ctx();
        select(&mut screen, "ui.art_pct");
        enter(&mut screen);
        type_text(&mut screen, "x");
        enter(&mut screen);
        assert!(screen.status_error, "{}", screen.status);
        let Some(Editor::Field { text, .. }) = &screen.editor else {
            panic!("the field closed on a rejected value");
        };
        assert!(text.text().ends_with('x'), "{:?}", text.text());
        assert_eq!(screen.cfg.ui.art_pct, Config::default().ui.art_pct);
    }

    #[test]
    fn theme_tab_previews_header() {
        let mut screen = ctx();
        screen.tab = Tab::Theme;
        let text = frame(&mut screen, 100, 40);
        assert!(text.contains("[Playing]"), "the state badge: {text}");
        assert!(text.contains("Song name"), "the placeholder title: {text}");
        assert!(text.contains("Volume: "), "the volume meter: {text}");
        assert!(text.contains("Artist name"), "the placeholder credit: {text}");
    }

    /// The preview is a copy of the header, so it follows the two header
    /// options: the number alone, and the flags as words where they fit.
    #[test]
    fn preview_follows_header_options() {
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let plain = frame(&mut screen, 120, 40);
        assert!(plain.contains("Volume: ▁▂▃▄▅▆▇█  60%"), "{plain}");
        assert!(!plain.contains("Repeat"), "{plain}");

        screen.cfg.ui.volume_display = "percent".into();
        screen.cfg.ui.mode_flags = "words".into();
        let text = frame(&mut screen, 120, 40);
        assert!(text.contains("Volume: 60%"), "{text}");
        assert!(text.contains("Repeat Shuffle Single Consume"), "{text}");
        screen.cfg.ui.volume_display = "meter".into();
        let meter = frame(&mut screen, 120, 40);
        assert!(meter.contains("Volume: ▁▂▃▄▅▆▇█") && !meter.contains("60%"), "{meter}");

        let narrow = frame(&mut screen, 80, 40);
        assert!(narrow.contains("Rep Shu Sin Con"), "the short words take over:\n{narrow}");
        assert!(!narrow.contains("Repeat"), "{narrow}");
    }

    #[test]
    fn preview_follows_icon_set() {
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        screen.cfg.ui.icons = "ascii".into();
        let text = frame(&mut screen, 120, 40);
        assert!(text.contains("rep"), "{text}");
        assert!(!text.contains('\u{f0456}'), "{text}");
    }

    /// The preview draws through the player's header code, so it follows the
    /// unsaved header mode and border style.
    #[test]
    fn preview_follows_header_mode_and_borders() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let plain = frame(&mut screen, 120, 40);
        assert!(!plain.contains("Playlists") && !plain.contains('╭'), "{plain}");

        screen.cfg.ui.header = "compact".into();
        let mut look = theme::Look::DEFAULT;
        assert!(theme::set_spacing_text(&mut look, "borders", "rounded"));
        theme::set_look(look);
        let text = frame(&mut screen, 120, 40);
        theme::set_look(theme::Look::DEFAULT);
        assert!(text.contains(" Playlists ") && text.contains("Volume: "), "the tab row: {text}");
        assert!(text.contains('╭') && !text.contains("[Playing]"), "{text}");
    }

    #[test]
    fn checks_hint_stays_with_new_wording() {
        let mut screen = ctx();
        screen.tab = Tab::Diagnostics;
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("press d"), "{text}");
        assert!(!text.contains("again"), "nothing has run yet: {text}");

        screen.report = Some(Report::default());
        let text = frame(&mut screen, 100, 30);
        assert!(
            text.contains("press d") && text.contains("again"),
            "the hint survives the report: {text}"
        );
    }

    #[test]
    fn checks_mark_setting_warnings() {
        let mut screen = ctx();
        screen.tab = Tab::Diagnostics;
        screen.report = Some(Report {
            settings: vec![
                doctor::Entry::ok("path:   /tmp/x"),
                doctor::Entry::warn("server is not https://"),
            ],
            ..Report::default()
        });
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("  ! server is not https://"), "{text}");
        assert!(text.contains("  path:   /tmp/x"), "{text}");
    }

    #[test]
    fn keystroke_clears_last_message() {
        let mut screen = ctx();
        let setting = crate::inventory::find("ui.art_pct").expect("ui.art_pct");
        screen.set(setting, "55");
        assert!(screen.status.starts_with("ui.art_pct"), "{}", screen.status);

        screen.handle(key(KeyCode::Down)).expect("handle");
        assert!(screen.status.is_empty(), "{}", screen.status);
    }

    #[test]
    fn token_row_never_writes_placeholder_back() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"MY-SECRET-TOKEN\"\n")
            .expect("write");
        let cfg = Config::read(&path).expect("read");
        let mut screen = ctx_for(path.clone(), cfg);
        screen.cursor = crate::inventory::ALL
            .iter()
            .filter(|s| s.group != "Theme")
            .position(|s| s.key == "plex.token")
            .expect("the token is on the Config tab");

        // The list shows the redacted placeholder, which must not reach the field.
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("(set)"), "the list is redacted: {text}");

        screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
        let text = frame(&mut screen, 100, 30);
        assert!(!text.contains(" (set)_"), "the field does not carry the placeholder: {text}");
        assert!(
            text.contains("(empty; Enter keeps the current value)"),
            "the field says what empty means: {text}"
        );

        screen.handle(key(KeyCode::Enter)).expect("commit");
        screen.save();
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(
            written.contains("token = \"MY-SECRET-TOKEN\""),
            "the token survived an untouched edit: {written}"
        );
        assert!(!written.contains("(set)"), "the file has no placeholder: {written}");
    }

    #[test]
    fn typed_token_replaces_old_one() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD-TOKEN\"\n")
            .expect("write");
        let cfg = Config::read(&path).expect("read");
        let mut screen = ctx_for(path.clone(), cfg);
        screen.cursor = crate::inventory::ALL
            .iter()
            .filter(|s| s.group != "Theme")
            .position(|s| s.key == "plex.token")
            .expect("the token is on the Config tab");

        screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
        for c in "NEW-TOKEN".chars() {
            screen.handle(key(KeyCode::Char(c))).expect("type");
        }
        screen.handle(key(KeyCode::Enter)).expect("commit");
        screen.save();

        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(
            written.contains("token = \"NEW-TOKEN\""),
            "a typed token replaces the old one: {written}"
        );
        assert!(!written.contains("OLD-TOKEN"), "{written}");
        assert!(
            !screen.status.contains("NEW-TOKEN"),
            "the new token is not echoed in the status: {}",
            screen.status
        );
    }

    fn open_token_field(screen: &mut Screen) {
        screen.cursor = crate::inventory::ALL
            .iter()
            .filter(|s| s.group != "Theme")
            .position(|s| s.key == "plex.token")
            .expect("the token is on the Config tab");
        screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
    }

    fn screen_with_token(token: &str) -> (tempfile::TempDir, std::path::PathBuf, Screen) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            format!("[plex]\nserver = \"http://x:32400\"\ntoken = \"{token}\"\n"),
        )
        .expect("write");
        let cfg = Config::read(&path).expect("read");
        let screen = ctx_for(path.clone(), cfg);
        (dir, path, screen)
    }

    #[test]
    fn pasted_token_lands_in_open_field() {
        let (_dir, path, mut screen) = screen_with_token("OLD");
        open_token_field(&mut screen);
        screen.handle_paste("NEW-TOKEN\n".into());
        screen.handle(key(KeyCode::Enter)).expect("commit");
        screen.save();
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.contains("token = \"NEW-TOKEN\""), "{written}");
    }

    #[test]
    fn paste_with_no_field_open_changes_nothing() {
        let (_dir, _path, mut screen) = screen_with_token("OLD");
        screen.handle_paste("NEW-TOKEN".into());
        assert!(screen.editor.is_none());
        assert!(!screen.dirty());

        screen.begin_edit(crate::inventory::find("ui.icons").expect("ui.icons"));
        screen.handle_paste("emoji".into());
        assert!(matches!(screen.editor, Some(Editor::Picker { .. })));
        assert!(!screen.dirty());
    }

    #[test]
    fn ctrl_c_cancels_field_not_types_c() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_token("OLD");
        open_token_field(&mut screen);
        screen.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).expect("ctrl-c");
        assert!(screen.editor.is_none(), "Ctrl-C closes the field");
        screen.handle(key(KeyCode::Enter)).expect("reopen");
        screen.handle(key(KeyCode::Enter)).expect("commit untouched");
        assert_eq!(screen.cfg.plex.token, "OLD");
        assert!(!screen.dirty());
    }

    #[test]
    fn new_token_counts_as_unsaved() {
        let (_dir, _path, mut screen) = screen_with_token("OLD");
        open_token_field(&mut screen);
        for c in "NEW".chars() {
            screen.handle(key(KeyCode::Char(c))).expect("type");
        }
        screen.handle(key(KeyCode::Enter)).expect("commit");
        assert!(screen.dirty(), "the redacted list must not hide a new token");
        screen.ask_to_quit();
        assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
    }

    #[test]
    fn enter_and_q_keep_unsaved_changes() {
        for code in [KeyCode::Enter, KeyCode::Char('q')] {
            let mut screen = ctx();
            screen.cfg.ui.icons = "emoji".into();
            screen.ask_to_quit();
            assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
            assert!(screen.handle(key(code)).expect("handle"), "{code:?} stays open");
            assert_eq!(screen.quit_ask, Some(QuitAsk::Dirty));
        }
        let mut screen = ctx();
        screen.cfg.ui.icons = "emoji".into();
        screen.ask_to_quit();
        assert!(!screen.handle(key(KeyCode::Char('y'))).expect("handle"));
        let mut screen = ctx();
        screen.ask_to_quit();
        assert!(!screen.handle(key(KeyCode::Enter)).expect("handle"));
    }

    #[test]
    fn failed_check_shows_error() {
        let mut screen = ctx();
        screen.tab = Tab::Diagnostics;
        screen.checking = true;
        *screen.check_slot.lock().unwrap() = Some(Err("could not start the checks".into()));
        let text = frame(&mut screen, 100, 30);
        assert!(!screen.checking, "{text}");
        assert!(text.contains("could not start the checks"), "{text}");
        assert!(!text.contains("checking …"), "{text}");
    }

    #[test]
    fn check_thread_ending_silently_is_noticed() {
        let mut screen = ctx();
        screen.tab = Tab::Diagnostics;
        screen.checking = true;
        let thread = std::thread::spawn(|| {});
        while !thread.is_finished() {
            std::thread::yield_now();
        }
        screen.check_thread = Some(thread);
        let text = frame(&mut screen, 100, 30);
        assert!(!screen.checking, "{text}");
        assert!(text.contains("the checks stopped unexpectedly"), "{text}");
    }

    #[test]
    fn secret_masked_while_typed() {
        let mut screen = ctx();
        screen.cursor = crate::inventory::ALL
            .iter()
            .filter(|s| s.group != "Theme")
            .position(|s| s.key == "plex.token")
            .expect("the token is on the Config tab");
        screen.begin_edit(crate::inventory::find("plex.token").expect("token"));
        for c in "abc".chars() {
            screen.handle(key(KeyCode::Char(c))).expect("type");
        }
        let text = frame(&mut screen, 100, 30);
        assert!(!text.contains("abc"), "the secret is never rendered: {text}");
        assert!(text.contains("•••"), "it is masked: {text}");
    }

    #[test]
    fn picker_opens_on_current_value() {
        let mut screen = ctx();
        screen.cfg.ui.icons = "emoji".into(); // not one of the offered options
        let icons = crate::inventory::find("ui.icons").expect("ui.icons");
        screen.begin_edit(icons);
        let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
            panic!("a picker is open");
        };
        assert_eq!(options[*index], "emoji", "the highlighted option is the one in effect");
    }

    #[test]
    fn diagnostics_end_shows_last_screen() {
        let mut screen = ctx();
        screen.tab = Tab::Diagnostics;
        screen.report = Some(Report::default());
        frame(&mut screen, 100, 30); // the pane records its height
        assert!(screen.diag_visible > 1, "the pane recorded its height: {}", screen.diag_visible);

        screen.handle(key(KeyCode::PageDown)).expect("page");
        assert_eq!(
            screen.diag_scroll, screen.diag_visible,
            "a page is the pane's height, not one line"
        );

        screen.handle(key(KeyCode::End)).expect("end");
        assert_eq!(screen.diag_scroll, screen.diag_max, "End is the last page the pane can show");
    }

    #[test]
    fn help_pane_shows_what_value_changes_from() {
        let mut screen = ctx();
        screen.cfg.ui.art_pct = 55;
        screen.cursor = crate::inventory::ALL
            .iter()
            .filter(|s| s.group != "Theme")
            .position(|s| s.key == "ui.art_pct")
            .expect("art_pct is on the Config tab");
        let text = frame(&mut screen, 100, 40);
        assert!(
            text.contains("default 40") && text.contains("value   55"),
            "the pane says what it changes from: {text}"
        );
    }

    #[test]
    fn picker_lists_options() {
        let mut screen = ctx();
        let icons = crate::inventory::find("ui.icons").expect("ui.icons");
        screen.begin_edit(icons);
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("nerd"), "{text}");
        assert!(text.contains("ascii"), "{text}");
    }

    #[test]
    fn colour_field_lists_names() {
        let mut screen = ctx();
        screen.tab = Tab::Theme;
        select(&mut screen, "theme.accent");
        let accent = crate::inventory::find("theme.accent").expect("theme.accent");
        screen.begin_edit(accent);
        let text = frame(&mut screen, 100, 40);
        assert!(text.contains("#7aa2f7_"), "the field shows a cursor: {text}");
        assert!(text.contains("black"), "the ANSI names are listed: {text}");
        assert!(text.contains("yellow"), "{text}");
    }

    #[test]
    fn colour_accepts_hex_or_ansi_name() {
        assert_eq!(parse_colour("#7aa2f7").as_deref(), Some("#7aa2f7"));
        assert_eq!(parse_colour("7aa2f7").as_deref(), Some("#7aa2f7"));
        assert_eq!(parse_colour("#7AA2F7").as_deref(), Some("#7aa2f7"));
        assert_eq!(parse_colour("red").as_deref(), Some("#800000"));
        assert_eq!(parse_colour("bright blue").as_deref(), Some("#0000ff"));
        assert_eq!(parse_colour("mauve"), None);
    }

    #[test]
    fn kind_turns_text_into_field_value() {
        assert_eq!(
            value_for(Kind::Int { min: Some(0), max: Some(100) }, "55").expect("an int"),
            toml::Value::Integer(55)
        );
        assert!(value_for(Kind::Int { min: Some(0), max: Some(100) }, "700").is_err());
        assert!(value_for(Kind::Int { min: Some(0), max: None }, "seven").is_err());
        assert_eq!(value_for(Kind::Bool, "true").expect("a bool"), toml::Value::Boolean(true));
        assert_eq!(
            value_for(Kind::Pair, "512, 256").expect("a pair"),
            toml::Value::Array(vec![toml::Value::Integer(512), toml::Value::Integer(256)])
        );
        assert!(value_for(Kind::Pair, "512").is_err());
        assert_eq!(
            value_for(Kind::Triple, "40, 0, 60").expect("weights"),
            toml::Value::Array(vec![40, 0, 60].into_iter().map(toml::Value::Integer).collect())
        );
        for bad in ["40,60", "1,2,3,4", "0,0,0", "-1,5,5", "a,b,c", "1,2,20000"] {
            assert!(value_for(Kind::Triple, bad).is_err(), "{bad}");
        }
        assert_eq!(
            value_for(Kind::List, "--a, --b").expect("a list"),
            toml::Value::Array(vec![
                toml::Value::String("--a".into()),
                toml::Value::String("--b".into())
            ])
        );
    }

    fn select(screen: &mut Screen, key: &str) {
        screen.cursor = screen
            .visible()
            .iter()
            .position(|s| s.key == key)
            .unwrap_or_else(|| panic!("{key} is not on this tab"));
    }

    fn snap(cfg: &Config) -> String {
        toml::to_string(cfg).expect("serialise")
    }

    fn press(screen: &mut Screen, c: char) {
        screen.handle(key(KeyCode::Char(c))).expect("handle");
    }

    #[test]
    fn r_resets_only_the_highlighted_setting() {
        let mut screen = ctx();
        screen.cfg.ui.art_pct = 55;
        screen.cfg.ui.scrolloff = 9;
        select(&mut screen, "ui.art_pct");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.ui.art_pct, Config::default().ui.art_pct);
        assert_eq!(screen.cfg.ui.scrolloff, 9, "the other change stays");
        assert!(screen.status.contains("ui.art_pct"), "{}", screen.status);

        // Already at its default: it says so and changes nothing.
        let before = snap(&screen.cfg);
        press(&mut screen, 'r');
        assert_eq!(snap(&screen.cfg), before);
    }

    #[test]
    fn r_refuses_settings_without_default() {
        let cfg = Config {
            plex: crate::config::PlexConfig {
                server: "http://x:32400".into(),
                token: "FAKE".into(),
                music_section: "7".into(),
            },
            ..Config::default()
        };
        let mut screen = ctx_for("/tmp/shmc.toml".into(), cfg.clone());
        for key in ["plex.server", "plex.token", "plex.music_section"] {
            select(&mut screen, key);
            press(&mut screen, 'r');
            assert_eq!(snap(&screen.cfg), snap(&cfg), "{key} is untouched");
            assert!(screen.status.contains(key), "{}", screen.status);
            assert!(!screen.status.contains("FAKE"));
        }
    }

    #[test]
    fn capital_r_asks_then_restores_tab() {
        let cfg = Config {
            plex: crate::config::PlexConfig {
                server: "http://x:32400".into(),
                token: "FAKE".into(),
                music_section: "7".into(),
            },
            ..Config::default()
        };
        let mut screen = ctx_for("/tmp/shmc.toml".into(), cfg.clone());
        screen.cfg.ui.art_pct = 55;
        screen.cfg.ui.scrolloff = 9;
        let changed = snap(&screen.cfg);

        press(&mut screen, 'R');
        assert_eq!(screen.restore_ask, Some(2));
        assert_eq!(snap(&screen.cfg), changed, "nothing changes until y");

        press(&mut screen, 'n');
        assert_eq!(screen.restore_ask, None);
        assert_eq!(snap(&screen.cfg), changed, "another key cancels");
        assert!(screen.status.is_empty());

        press(&mut screen, 'R');
        press(&mut screen, 'y');
        assert_eq!(screen.restore_ask, None);
        assert_eq!(snap(&screen.cfg), snap(&cfg), "defaults back; server, token, section kept");
    }

    #[test]
    fn capital_r_without_changes_does_not_ask() {
        let mut screen = ctx();
        press(&mut screen, 'R');
        assert_eq!(screen.restore_ask, None);
        assert!(screen.status.contains("already"), "{}", screen.status);
    }

    #[test]
    fn r_on_theme_tab_restores_one_colour() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.cfg.theme.accent = Some("#112233".into());
        screen.cfg.theme.secondary = Some("#445566".into());
        screen.switch(Tab::Theme);
        screen.apply_theme();
        select(&mut screen, "theme.accent");
        press(&mut screen, 'r');
        let shipped = crate::config::ThemeConfig::default();
        assert_eq!(screen.cfg.theme.accent, None, "the override is gone, not pinned");
        assert_eq!(
            screen.cfg.theme.secondary.as_deref(),
            Some("#445566"),
            "the other colour stays"
        );
        assert_eq!(
            theme::accent_color(),
            theme::from_config(&shipped).0.accent,
            "the palette follows"
        );
        // The palette is process-wide: put the shipped one back for the rest.
        screen.cfg.theme = shipped.clone();
        screen.apply_theme();
    }

    /// `*`, `r` and `R` judge a colour against the chosen theme, not against
    /// what shmc ships.
    #[test]
    fn colours_marked_and_restored_against_chosen_theme() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        screen.set(inventory::find("theme.name").unwrap(), "catppuccin-mocha");
        let marked = |screen: &Screen, key: &str| {
            let values = screen.values();
            screen.is_changed(inventory::find(key).unwrap(), &values)
        };
        assert_eq!(screen.value(inventory::find("theme.accent").unwrap()), "#89b4fa");
        assert!(marked(&screen, "theme.name"));
        assert!(!marked(&screen, "theme.accent"), "a preset's colour is not a change");

        screen.set(inventory::find("theme.accent").unwrap(), "#112233");
        assert!(marked(&screen, "theme.accent"));
        select(&mut screen, "theme.accent");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.accent, None);
        assert!(!marked(&screen, "theme.accent"));
        assert_eq!(screen.cfg.theme.name, "catppuccin-mocha", "r on a colour keeps the theme");

        screen.set(inventory::find("theme.secondary").unwrap(), "#445566");
        press(&mut screen, 'R');
        press(&mut screen, 'y');
        assert_eq!(screen.cfg.theme.name, "default");
        assert_eq!(screen.cfg.theme.secondary, None);
        assert_eq!(theme::theme(), theme::DEFAULT);
    }

    /// `none` is a real value of the background: it beats a theme's, shows as
    /// text with no swatch, and `r` drops it back to the theme's.
    #[test]
    fn background_row_shows_swatch() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let bg = inventory::find("theme.background").unwrap();
        let row = |screen: &mut Screen| {
            frame(screen, 100, 70).lines().find(|l| l.contains("Background")).unwrap().to_string()
        };
        let marked = |screen: &Screen| screen.is_changed(bg, &screen.values());

        let line = row(&mut screen);
        assert!(line.contains("none") && !line.contains('█'), "{line}");
        assert!(!marked(&screen));

        screen.set(inventory::find("theme.name").unwrap(), "nord");
        let line = row(&mut screen);
        assert!(line.contains("#2e3440") && line.contains('█'), "{line}");
        assert!(!marked(&screen), "a preset's background is not a change");
        assert_eq!(theme::theme().background, themes::preset("nord").unwrap().theme.background);

        screen.set(bg, "none");
        assert_eq!(screen.cfg.theme.background.as_deref(), Some("none"));
        assert_eq!(theme::theme().background, None, "the palette follows");
        assert!(marked(&screen));
        select(&mut screen, "theme.background");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.background, None);
        assert!(theme::theme().background.is_some());

        screen.set(bg, "#2e3440");
        assert!(marked(&screen), "the same colour is still an override");
        press(&mut screen, 'R');
        press(&mut screen, 'y');
        assert_eq!(screen.cfg.theme.background, None);
        assert_eq!(theme::theme(), theme::DEFAULT);

        screen.set(bg, "mauve");
        assert_eq!(screen.cfg.theme.background, None, "not a colour: refused");
        assert!(screen.status_error);
    }

    /// A background paints the whole screen, preview included, and follows the
    /// picker; without one nothing is painted.
    #[test]
    fn background_paints_screen_and_follows_preview() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let bgs = |screen: &mut Screen, w: u16, h: u16| {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
            terminal.draw(|f| screen.draw(f)).expect("draw");
            let buf = terminal.backend().buffer().clone();
            // Text in the terminal's own colour would vanish on a light
            // background: every visible cell is coloured by the theme.
            let plain: Vec<&str> = buf
                .content()
                .iter()
                .filter(|c| c.fg == ratatui::style::Color::Reset && !c.symbol().trim().is_empty())
                .map(|c| c.symbol())
                .collect();
            assert!(plain.is_empty(), "drawn in the terminal's colour: {plain:?}");
            buf.content().iter().map(|c| c.bg).collect::<Vec<_>>()
        };
        assert_eq!(bgs(&mut screen, 100, 40)[0], ratatui::style::Color::Reset);

        for name in ["catppuccin-latte", "nord"] {
            let preset = themes::preset(name).unwrap();
            let entries = themes::available(None).entries;
            screen.preview_theme(name, &entries);
            for (w, h) in [(100, 40), (60, 12), (30, 40)] {
                let cells = bgs(&mut screen, w, h);
                let holes = cells.iter().filter(|c| **c == ratatui::style::Color::Reset).count();
                assert_eq!(holes, 0, "{name} at {w}x{h} left {holes} cells unpainted");
                assert!(cells.contains(&preset.theme.background.unwrap()));
            }
            screen.switch(Tab::Config);
            let holes = bgs(&mut screen, 100, 30)
                .iter()
                .filter(|c| **c == ratatui::style::Color::Reset)
                .count();
            assert_eq!(holes, 0, "{name}: the Config tab left {holes} cells unpainted");
            screen.switch(Tab::Theme);
        }
        screen.apply_theme();
    }

    #[test]
    fn theme_sample_yields_on_small_screens() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let text = frame(&mut screen, 100, 40);
        for needle in ["Song name", "Another song", "ing song", "A plain row", "█"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        // The header is still there, and the colour list keeps all its rows.
        assert!(text.contains("[Playing]"), "{text}");
        assert!(text.contains("Visualizer top"), "{text}");
        // Shorter or narrower screens drop the sample first and never panic.
        let short = frame(&mut screen, 100, 20);
        assert!(!short.contains("Another song"), "{short}");
        assert!(short.contains("[Playing]"), "{short}");
        for (w, h) in [(100, 24), (30, 40), (16, 10), (1, 1), (60, 12)] {
            frame(&mut screen, w, h);
        }
    }

    /// A queue cursor fills its row; the sample's must too, or the colours it
    /// previews do not look like the player's.
    #[test]
    fn sample_cursors_fill_rows() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).expect("terminal");
        terminal.draw(|f| screen.draw(f)).expect("draw");
        let buf = terminal.backend().buffer();
        let row_bg = |needle: &str| {
            (0..buf.area.height).find_map(|y| {
                let line: String = (0..buf.area.width)
                    .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()))
                    .collect();
                let x = line.find(needle)? as u16;
                // Well past the text, still left of the visualizer strip.
                buf.cell((x + needle.len() as u16 + 20, y)).map(|c| c.bg)
            })
        };
        assert_eq!(row_bg("Song name  Artist"), Some(theme::theme().accent));
        assert_eq!(row_bg("Another song"), Some(theme::theme().highlight));
    }

    /// A screen whose themes directory is `<tmp>/themes`, holding `files`.
    fn themed(tmp: &std::path::Path, files: &[(&str, &str)]) -> Screen {
        let dir = tmp.join("themes");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let mut cfg = Config::default();
        cfg.theme.dir = Some(dir);
        let mut screen = ctx_for(tmp.join("config.toml"), cfg);
        screen.switch(Tab::Theme);
        screen
    }

    fn enter(screen: &mut Screen) {
        screen.handle(key(KeyCode::Enter)).expect("handle");
    }

    #[test]
    fn esc_restores_palette_after_preview() {
        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(
            tmp.path(),
            &[(
                "mine.toml",
                "name = \"My Theme\"\nauthor = \"Me\"\nvariant = \"light\"\naccent = \"#0a0b0c\"\n",
            )],
        );
        screen.cfg.theme.secondary = Some("#445566".into());
        screen.apply_theme();
        let before = theme::theme();
        let open_on_mine = |screen: &mut Screen| {
            select(screen, "theme.name");
            enter(screen);
            let Some(Editor::Picker { options, .. }) = &screen.editor else {
                panic!("the chooser is not open");
            };
            let at = options.iter().position(|o| o == "mine").expect("the file is listed");
            let first = options.iter().position(|o| o == "default").unwrap();
            assert!(first < at, "presets come first");
            for _ in first..at {
                press(screen, 'j');
            }
        };

        open_on_mine(&mut screen);
        assert_eq!(theme::theme().accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
        assert_ne!(theme::theme().secondary, before.secondary, "the preview is the plain theme");
        let text = frame(&mut screen, 100, 40);
        for needle in ["mine", "My Theme", "author  Me", "variant light", "themes/mine.toml"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        assert_eq!(screen.cfg.theme.name, "default", "previewing writes nothing");

        screen.handle(key(KeyCode::Esc)).expect("handle");
        assert!(screen.editor.is_none());
        assert_eq!(theme::theme(), before, "Esc puts the old palette back");
        assert_eq!(screen.cfg.theme.name, "default");

        open_on_mine(&mut screen);
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.name, "mine");
        assert_eq!(theme::theme().accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
        assert!(screen.cfg.theme.secondary.is_none(), "choosing it cleared the override");
        screen.cfg.theme = crate::config::ThemeConfig::default();
        screen.apply_theme();
    }

    /// Styles preview while the chooser moves: Esc restores, choosing keeps,
    /// and a theme without styles removes them.
    #[test]
    fn esc_restores_previewed_styles() {
        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(
            tmp.path(),
            &[(
                "styled.toml",
                "[styles]\ncurrent = { italic = true }\ntab_active = { bold = false }\n",
            )],
        );
        assert_eq!(theme::look().styles.count(), 0);
        pick_theme(&mut screen, "styled");
        assert_eq!(theme::look().styles.count(), 2, "the preview applies them");
        assert!(theme::current().add_modifier.contains(ratatui::style::Modifier::ITALIC));
        let text = frame(&mut screen, 100, 40);
        assert!(text.contains("styles  2 set"), "{text}");
        move_picker(&mut screen, "nord");
        assert_eq!(theme::look().styles.count(), 0, "a theme without styles removes them");
        assert!(!frame(&mut screen, 100, 40).contains("styles  "));

        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(theme::look().styles.count(), 0, "Esc puts the old styles back");

        pick_theme(&mut screen, "styled");
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.name, "styled");
        assert_eq!(theme::look().styles.count(), 2, "choosing keeps them");
        pick_theme(&mut screen, "default");
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(theme::look().styles.count(), 2, "Esc restores the chosen theme's styles");
        reset_palette(&mut screen);
        assert_eq!(theme::look().styles.count(), 0);
    }

    #[test]
    fn x_exports_active_palette() {
        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(tmp.path(), &[]);
        assert!(frame(&mut screen, 100, 40).contains("x export"));

        screen.cfg.theme.accent = Some("#0a0b0c".into());
        press(&mut screen, 'x');
        assert!(
            matches!(&screen.editor, Some(Editor::Export { text }) if text.text() == "my-theme")
        );
        // An id that is not one is refused and the prompt stays to fix it.
        for _ in 0..8 {
            screen.handle(key(KeyCode::Backspace)).unwrap();
        }
        for c in "Bad Id".chars() {
            press(&mut screen, c);
        }
        enter(&mut screen);
        assert!(screen.editor.is_some() && screen.status_error, "{}", screen.status);
        for _ in 0..6 {
            screen.handle(key(KeyCode::Backspace)).unwrap();
        }
        for c in "mine".chars() {
            press(&mut screen, c);
        }
        enter(&mut screen);
        assert!(screen.editor.is_none() && !screen.status_error, "{}", screen.status);
        assert!(screen.status.contains("mine"), "{}", screen.status);
        let found = themes::available(screen.cfg.theme.dir.as_deref());
        let mine = found.entries.iter().find(|e| e.id == "mine").expect("exported");
        assert_eq!(mine.theme, theme::from_config(&screen.cfg.theme).0);
        assert_eq!(mine.theme.accent, ratatui::style::Color::Rgb(0x0a, 0x0b, 0x0c));
        screen.cfg.theme = crate::config::ThemeConfig::default();
        screen.apply_theme();
    }

    /// A colour pinned to what the theme already has is invisible; `*`, `r`
    /// and `R` must still see it, or it stays when the theme changes.
    #[test]
    fn override_equal_to_theme_colour_is_dropped() {
        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(tmp.path(), &[]);
        let pinned = theme::hex(theme::DEFAULT.accent);
        let marked = |screen: &Screen| {
            let values = screen.values();
            screen.is_changed(inventory::find("theme.accent").unwrap(), &values)
        };

        screen.cfg.theme.accent = Some(pinned.clone());
        assert!(marked(&screen));
        select(&mut screen, "theme.accent");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.accent, None);
        assert!(screen.status.contains("removed"), "{}", screen.status);
        press(&mut screen, 'r');
        assert!(screen.status.contains("already"), "{}", screen.status);

        screen.cfg.theme.accent = Some(pinned);
        press(&mut screen, 'R');
        assert_eq!(screen.restore_ask, Some(1));
        press(&mut screen, 'y');
        assert_eq!(screen.cfg.theme.accent, None);
    }

    /// The settings pane's rows, trimmed: the left 52% of the frame without
    /// its border and scrollbar column, exactly what the list is drawn into.
    fn list_lines(text: &str, w: u16) -> Vec<String> {
        let pane = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(20)])
            .split(Rect::new(0, 0, w, 1))[0]
            .width as usize;
        text.lines()
            .map(|l| {
                let cols: Vec<char> = l.chars().collect();
                let end = pane.saturating_sub(2).min(cols.len());
                cols[1.min(end)..end].iter().collect::<String>().trim_end().to_string()
            })
            .collect()
    }

    /// Where the value starts on a settings row: after the label and the first
    /// run of two spaces. None for a heading or an empty row.
    fn value_col(line: &str) -> Option<usize> {
        let cols: Vec<char> = line.chars().collect();
        let gap = (1..cols.len()).find(|&i| cols[i] == ' ' && cols[i - 1] == ' ' && i > 2)?;
        (gap..cols.len()).find(|&i| cols[i] != ' ')
    }

    #[test]
    fn long_label_is_cut_not_pushed() {
        let longest = inventory::ALL
            .iter()
            .filter(|s| !inventory::is_theme_group(s.group))
            .map(|s| theme::str_width(s.label))
            .max()
            .unwrap();
        for w in [50u16, 100, 160] {
            for tab in [Tab::Config, Tab::Theme] {
                let mut screen = ctx();
                screen.tab = tab;
                // Tall enough that every setting is on screen.
                let text = frame(&mut screen, w, 90);
                let lines = list_lines(&text, w);
                let title = lines
                    .iter()
                    .position(|l| l.contains("Settings") || l.starts_with(" Theme ─"))
                    .unwrap();
                let cols: std::collections::BTreeSet<usize> = lines
                    .iter()
                    .skip(title + 1)
                    .take_while(|l| !l.contains("changed from default"))
                    .filter_map(|l| value_col(l))
                    .collect();
                assert_eq!(cols.len(), 1, "{tab:?} at {w}: values start in {cols:?}\n{text}");
                if tab == Tab::Config && w == 100 {
                    // Sized from the labels, not a fixed 22.
                    let col = *cols.iter().next().unwrap();
                    assert!(col <= 1 + longest + LIST_GAP, "col {col}\n{text}");
                }
                if tab == Tab::Config && w == 50 {
                    assert!(lines.iter().any(|l| l.contains('…')), "a long label is cut:\n{text}");
                }
            }
        }
    }

    /// Select by key and step through the whole list, checking the cursor is
    /// always on a setting and that its heading shows when it opens a group.
    #[test]
    fn cursor_skips_group_headings() {
        let mut screen = ctx();
        let text = frame(&mut screen, 100, 90);
        let lines = list_lines(&text, 100);
        let at: Vec<usize> = inventory::GROUPS
            .iter()
            .filter(|g| !inventory::is_theme_group(g))
            .map(|g| {
                lines
                    .iter()
                    .position(|l| l.trim() == *g)
                    .unwrap_or_else(|| panic!("no heading {g}:\n{text}"))
            })
            .collect();
        assert!(at.windows(2).all(|p| p[0] < p[1]), "in GROUPS order: {at:?}");
        let theme_tab = {
            let mut s = ctx();
            s.tab = Tab::Theme;
            frame(&mut s, 100, 70)
        };
        let theme_lines = list_lines(&theme_tab, 100);
        let at: Vec<usize> = inventory::THEME_GROUPS
            .iter()
            .map(|g| {
                theme_lines
                    .iter()
                    .position(|l| l.trim() == *g)
                    .unwrap_or_else(|| panic!("no heading {g}:\n{theme_tab}"))
            })
            .collect();
        assert!(at.windows(2).all(|p| p[0] < p[1]), "in order: {at:?}");

        // Walk the list in a short pane, where it has to scroll.
        let cases = [
            (Tab::Config, 14u16),
            (Tab::Config, 20),
            (Tab::Config, 40),
            (Tab::Theme, 24),
            (Tab::Theme, 30),
            (Tab::Theme, 70),
        ];
        for (tab, h) in cases {
            let all: Vec<&Setting> = {
                let mut probe = ctx();
                probe.tab = tab;
                probe.visible()
            };
            let mut screen = ctx();
            screen.tab = tab;
            for (i, setting) in all.iter().enumerate() {
                assert_eq!(screen.selected().map(|s| s.key), Some(setting.key));
                let text = frame(&mut screen, 100, h);
                let lines = list_lines(&text, 100);
                let row = lines
                    .iter()
                    .position(|l| {
                        value_col(l).is_some() && l.trim_start().starts_with(setting.label)
                    })
                    .unwrap_or_else(|| panic!("{} is off screen at h={h}:\n{text}", setting.key));
                if i == 0 || all[i - 1].group != setting.group {
                    assert!(
                        row > 0 && lines[row - 1].trim() == setting.group,
                        "{} opens {} without its heading at h={h}:\n{text}",
                        setting.key,
                        setting.group
                    );
                }
                press(&mut screen, 'j');
            }
            assert_eq!(screen.cursor, all.len() - 1, "j stops on the last setting");
            screen.handle(key(KeyCode::Home)).unwrap();
            assert_eq!(screen.cursor, 0);
            screen.handle(key(KeyCode::End)).unwrap();
            assert_eq!(screen.selected().map(|s| s.key), Some(all[all.len() - 1].key));
            let text = frame(&mut screen, 100, h);
            assert!(text.contains(all[all.len() - 1].label), "{text}");
        }
    }

    #[test]
    fn brackets_jump_between_groups() {
        let mut screen = ctx();
        let first_of = |g: &str| screen.visible().iter().position(|s| s.group == g).unwrap();
        let (playback, art) = (first_of("Playback"), first_of("Album art"));
        press(&mut screen, ']');
        assert_eq!(screen.cursor, playback);
        screen.cursor += 1;
        press(&mut screen, ']');
        assert_eq!(screen.cursor, art);
        press(&mut screen, '[');
        assert_eq!(screen.cursor, playback);
        press(&mut screen, '[');
        assert_eq!(screen.cursor, 0);
        press(&mut screen, '[');
        assert_eq!(screen.cursor, 0, "the first group has nothing before it");
        screen.handle(key(KeyCode::End)).unwrap();
        press(&mut screen, ']');
        assert_eq!(screen.cursor, screen.visible().len() - 1, "nor the last anything after");
        screen.handle(key(KeyCode::Tab)).unwrap();
        assert_eq!(screen.tab, Tab::Theme);
    }

    fn type_filter(screen: &mut Screen, text: &str) {
        press(screen, '/');
        for c in text.chars() {
            press(screen, c);
        }
    }

    fn keys_in_view(screen: &Screen) -> Vec<&'static str> {
        screen.view().iter().map(|s| s.key).collect()
    }

    #[test]
    fn filter_hides_empty_groups() {
        let mut screen = ctx();
        type_filter(&mut screen, "plex SER");
        assert_eq!(keys_in_view(&screen), ["plex.server"], "label, any case");
        screen.handle(key(KeyCode::Esc)).unwrap();
        type_filter(&mut screen, "art_pct");
        assert_eq!(keys_in_view(&screen), ["ui.art_pct"], "key");
        screen.handle(key(KeyCode::Esc)).unwrap();
        type_filter(&mut screen, "cleartext");
        assert_eq!(keys_in_view(&screen), ["plex.server"], "help text");

        // Only the Plex group survives, so only its heading is drawn.
        screen.handle(key(KeyCode::Enter)).unwrap();
        let text = frame(&mut screen, 100, 40);
        let lines = list_lines(&text, 100);
        assert!(lines.iter().any(|l| l.trim() == "Plex"), "{text}");
        assert!(!lines.iter().any(|l| l.trim() == "Playback"), "{text}");
        assert!(text.contains("filter: cleartext (1/"), "{text}");
    }

    #[test]
    fn filtered_edit_changes_shown_setting() {
        let mut screen = ctx();
        let before = screen.cfg.ui.center_cursor;
        let (art_square, auto_follow) = (screen.cfg.ui.art_square, screen.cfg.ui.auto_follow);
        type_filter(&mut screen, "center_cursor");
        screen.handle(key(KeyCode::Enter)).unwrap();
        assert_eq!(screen.cursor, 0, "the first match, not the setting's place in the tab");
        screen.handle(key(KeyCode::Enter)).unwrap();
        assert_eq!(screen.cfg.ui.center_cursor, !before);
        assert_eq!(screen.cfg.ui.art_square, art_square);
        assert_eq!(screen.cfg.ui.auto_follow, auto_follow);
        // r on the same row restores that setting.
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.ui.center_cursor, before);
    }

    #[test]
    fn esc_clears_filter_keeps_cursor() {
        let mut screen = ctx();
        select(&mut screen, "ui.scrolloff");
        type_filter(&mut screen, "ui.");
        assert!(keys_in_view(&screen).iter().all(|k| k.contains("ui.")));
        assert_eq!(screen.selected().map(|s| s.key), Some("ui.scrolloff"));
        // Narrow to something that no longer holds it: the nearest match.
        for c in "art".chars() {
            press(&mut screen, c);
        }
        assert!(screen.selected().is_some_and(|s| s.key.starts_with("ui.art")));
        screen.handle(key(KeyCode::Enter)).unwrap();
        assert!(!screen.filter.typing && screen.filter.is_active());
        // Esc with the filter applied drops it and nothing else; q is still q.
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert!(!screen.filter.is_active() && screen.quit_ask.is_none());
        assert!(screen.cursor < screen.visible().len());
        // Esc while typing clears too, and a tab change drops it.
        type_filter(&mut screen, "zzz");
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert!(!screen.filter.is_active() && !screen.filter.typing);
        type_filter(&mut screen, "ui");
        screen.handle(key(KeyCode::Enter)).unwrap();
        screen.handle(key(KeyCode::Tab)).unwrap();
        assert!(!screen.filter.is_active());
    }

    #[test]
    fn no_match_disables_row_keys() {
        let mut screen = ctx();
        let before = snap(&screen.cfg);
        type_filter(&mut screen, "no such setting");
        screen.handle(key(KeyCode::Enter)).unwrap();
        assert!(screen.selected().is_none());
        for code in [
            KeyCode::Enter,
            KeyCode::Char('r'),
            KeyCode::Char('R'),
            KeyCode::Char('j'),
            KeyCode::Char('['),
            KeyCode::Char(']'),
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::Home,
        ] {
            screen.handle(key(code)).unwrap();
        }
        assert_eq!(snap(&screen.cfg), before);
        assert!(screen.editor.is_none());
        for (w, h) in [(100, 30), (44, 10)] {
            let text = frame(&mut screen, w, h);
            assert!(text.contains("no settings match"), "{w}x{h}\n{text}");
        }
    }

    #[test]
    fn keys_bar_drops_hints_before_clipping() {
        let mut screen = ctx();
        let wide = frame(&mut screen, 120, 24);
        assert!(
            wide.contains("groups") && wide.contains("filter") && wide.contains("←→ change"),
            "{wide}"
        );
        for w in [60u16, 44, 30] {
            let text = frame(&mut screen, w, 24);
            let bar = text
                .lines()
                .find(|l| l.contains("move"))
                .unwrap_or_else(|| panic!("no keys bar at {w}:\n{text}"));
            // `q quit` is the last hint to go, so a bar that still ends in it
            // lost whole hints rather than the end of the line.
            assert!(bar.trim_end_matches(['│', ' ']).ends_with("q quit"), "{w}: {bar}");
        }
        let narrow = frame(&mut screen, 44, 24);
        assert!(!narrow.contains("groups") && !narrow.contains("←→"), "{narrow}");
    }

    fn with_ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn field_text(screen: &Screen) -> String {
        match &screen.editor {
            Some(Editor::Field { text, .. }) | Some(Editor::Export { text }) => {
                text.text().to_string()
            }
            _ => panic!("no text input is open"),
        }
    }

    #[test]
    fn field_edits_at_cursor() {
        let mut screen = ctx();
        screen.cfg.plex.server = "http://plex.example:32400".into();
        select(&mut screen, "plex.server");
        enter(&mut screen);
        screen.handle(key(KeyCode::Home)).unwrap();
        for _ in 0..7 {
            screen.handle(key(KeyCode::Right)).unwrap();
        }
        screen.handle(key(KeyCode::Delete)).unwrap();
        press(&mut screen, 'P');
        assert_eq!(field_text(&screen), "http://Plex.example:32400");
        enter(&mut screen);
        assert_eq!(screen.cfg.plex.server, "http://Plex.example:32400");
    }

    #[test]
    fn paste_lands_at_cursor_without_newline() {
        let mut screen = ctx();
        screen.cfg.plex.server = "http://example:32400".into();
        select(&mut screen, "plex.server");
        enter(&mut screen);
        for _ in 0..13 {
            screen.handle(key(KeyCode::Left)).unwrap();
        }
        screen.handle_paste("plex.\n".into());
        assert_eq!(field_text(&screen), "http://plex.example:32400");
    }

    #[test]
    fn secret_keeps_mask_with_cursor() {
        let mut screen = ctx();
        open_token_field(&mut screen);
        for c in "abc".chars() {
            press(&mut screen, c);
        }
        screen.handle(key(KeyCode::Left)).unwrap();
        screen.handle(key(KeyCode::Left)).unwrap();
        press(&mut screen, 'X');
        let text = frame(&mut screen, 100, 30);
        assert!(!text.contains("abc") && !text.contains("aXb"), "{text}");
        assert!(text.contains("••••"), "{text}");
        enter(&mut screen);
        assert_eq!(screen.cfg.plex.token, "aXbc");
    }

    #[test]
    fn long_value_scrolls_to_keep_cursor_visible() {
        let mut screen = ctx();
        screen.cfg.plex.server = format!("http://START{}END", "x".repeat(70));
        select(&mut screen, "plex.server");
        enter(&mut screen);
        let end = frame(&mut screen, 60, 30);
        assert!(end.contains("xEND") && !end.contains("START"), "{end}");
        screen.handle(key(KeyCode::Home)).unwrap();
        let start = frame(&mut screen, 60, 30);
        assert!(start.contains("START") && !start.contains("END"), "{start}");
    }

    #[test]
    fn filter_and_export_prompt_take_cursor_keys() {
        let mut screen = ctx();
        type_filter(&mut screen, "art_pct");
        screen.handle(key(KeyCode::Home)).unwrap();
        screen.handle_paste("ui.".into());
        assert_eq!(screen.filter.text, "ui.art_pct");
        assert_eq!(keys_in_view(&screen), ["ui.art_pct"]);
        screen.handle(with_ctrl('u')).unwrap();
        assert_eq!(screen.filter.text, "art_pct");
        screen.handle(key(KeyCode::End)).unwrap();
        screen.handle(key(KeyCode::Left)).unwrap();
        screen.handle(key(KeyCode::Backspace)).unwrap();
        assert_eq!(screen.filter.text, "art_pt");
        // The filter is the one input that is already applied while typed.
        assert!(screen.filter.typing);

        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(tmp.path(), &[]);
        press(&mut screen, 'x');
        screen.handle(key(KeyCode::Home)).unwrap();
        press(&mut screen, 'a');
        screen.handle_paste("b".into());
        screen.handle(key(KeyCode::Delete)).unwrap();
        assert_eq!(field_text(&screen), "aby-theme");
    }

    #[test]
    fn arrows_step_number_within_bounds() {
        let mut screen = ctx();
        // 0-100: by 5.
        screen.cfg.ui.art_pct = 40;
        select(&mut screen, "ui.art_pct");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.art_pct, 45);
        assert!(screen.status.contains("ui.art_pct = 45"), "{}", screen.status);
        screen.cfg.ui.art_pct = 98;
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.art_pct, 100);
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.art_pct, 100, "past the end is not an error");
        assert!(screen.status.contains("limit") && !screen.status_error, "{}", screen.status);
        screen.cfg.ui.art_pct = 3;
        screen.handle(key(KeyCode::Left)).unwrap();
        assert_eq!(screen.cfg.ui.art_pct, 0);
        // 1-10: by 1.
        select(&mut screen, "ui.visualizer_bar_width");
        let before = screen.cfg.ui.visualizer_bar_width;
        press(&mut screen, 'l');
        assert_eq!(screen.cfg.ui.visualizer_bar_width, before + 1);
        screen.cfg.ui.visualizer_bar_width = 1;
        press(&mut screen, 'h');
        assert_eq!(screen.cfg.ui.visualizer_bar_width, 1);
        // No upper bound: ones; a float with a wide range: ones, no drift.
        select(&mut screen, "mpv.volume");
        screen.cfg.mpv.volume = 30.5;
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.mpv.volume, 31.5);
    }

    #[test]
    fn arrows_toggle_bool_and_cycle_enum() {
        let mut screen = ctx();
        select(&mut screen, "ui.art_square");
        let before = screen.cfg.ui.art_square;
        screen.handle(key(KeyCode::Left)).unwrap();
        assert_eq!(screen.cfg.ui.art_square, !before);
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.art_square, before);

        select(&mut screen, "ui.album_sort");
        screen.cfg.ui.album_sort = "title".into();
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.album_sort, "year", "past the last wraps to the first");
        screen.handle(key(KeyCode::Left)).unwrap();
        assert_eq!(screen.cfg.ui.album_sort, "title");
        screen.handle(key(KeyCode::Left)).unwrap();
        assert_eq!(screen.cfg.ui.album_sort, "year_desc");
    }

    #[test]
    fn right_changes_theme_like_chooser() {
        let _palette = theme::palette_test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let mut screen = themed(tmp.path(), &[("gone.toml", "name = \"Gone\"\n")]);
        select(&mut screen, "theme.name");
        let before = screen.cfg.theme.name.clone();
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_ne!(screen.cfg.theme.name, before);
        assert_eq!(theme::accent().fg, Some(theme::from_config(&screen.cfg.theme).0.accent));
        assert!(screen.status.contains("theme.name"), "{}", screen.status);
        // A name whose theme file is gone does not panic: it steps on.
        screen.cfg.theme.name = "deleted-since".into();
        screen.handle(key(KeyCode::Left)).unwrap();
        screen.cfg.theme = crate::config::ThemeConfig::default();
        screen.apply_theme();
    }

    #[test]
    fn right_ignored_on_text_list_colour_secret() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        let before = snap(&screen.cfg);
        for key_name in ["plex.server", "plex.token", "mpv.extra_args", "album_art.max_size_px"] {
            select(&mut screen, key_name);
            screen.handle(key(KeyCode::Right)).unwrap();
            screen.handle(key(KeyCode::Left)).unwrap();
            assert!(screen.status.is_empty(), "{key_name}: {}", screen.status);
        }
        screen.switch(Tab::Theme);
        select(&mut screen, "theme.accent");
        screen.handle(key(KeyCode::Right)).unwrap();
        screen.switch(Tab::Diagnostics);
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(snap(&screen.cfg), before);
        assert!(screen.editor.is_none());
    }

    #[test]
    fn right_steps_filtered_setting() {
        let mut screen = ctx();
        let before = screen.cfg.ui.center_cursor;
        let untouched = snap(&Config::default());
        type_filter(&mut screen, "center_cursor");
        screen.handle(key(KeyCode::Enter)).unwrap();
        assert_eq!(screen.cursor, 0, "position 0 of the tab is the Plex server");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.center_cursor, !before);
        screen.cfg.ui.center_cursor = before;
        assert_eq!(snap(&screen.cfg), untouched);
        // No match: nothing to step.
        screen.handle(key(KeyCode::Esc)).unwrap();
        type_filter(&mut screen, "no such setting");
        screen.handle(key(KeyCode::Enter)).unwrap();
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(snap(&screen.cfg), untouched);
    }

    /// The colour the list draws a row's label in.
    fn label_fg(screen: &mut Screen, label: &str) -> ratatui::style::Color {
        let mut terminal = Terminal::new(TestBackend::new(100, 70)).expect("terminal");
        terminal.draw(|f| screen.draw(f)).expect("draw");
        let buf = terminal.backend().buffer();
        for y in 0..buf.area.height {
            let row: String =
                (0..buf.area.width).map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol())).collect();
            // The list is the left pane; the details pane repeats the label.
            if let Some(at) = row.find(label).filter(|&at| row[..at].chars().count() < 40) {
                let x = row[..at].chars().count() as u16;
                return buf.cell((x, y)).expect("cell").fg;
            }
        }
        panic!("{label} is not drawn");
    }

    #[test]
    fn unused_setting_dimmed_with_reason() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        let (text, dim) = (theme::text().fg.unwrap(), theme::dim().fg.unwrap());
        assert_ne!(text, dim);
        assert_eq!(screen.cfg.ui.visualizer_mode, "adaptive");
        assert_eq!(label_fg(&mut screen, "Bar count"), dim);
        assert_eq!(label_fg(&mut screen, "Spread anchor"), dim);
        assert_eq!(label_fg(&mut screen, "Bar width"), text, "that one always applies");

        // On the cursor the row keeps the cursor's colours, and the details
        // pane gives the reason; its value is still shown and Enter opens it.
        select(&mut screen, "ui.visualizer_bars");
        assert_eq!(label_fg(&mut screen, "Bar count"), theme::current().fg.unwrap());
        let shown = frame(&mut screen, 100, 70);
        assert!(shown.contains("only used in spread mode"), "{shown}");
        assert!(shown.contains("Bar count"), "{shown}");
        enter(&mut screen);
        assert!(matches!(&screen.editor, Some(Editor::Field { .. })), "still editable");
        screen.handle(key(KeyCode::Esc)).unwrap();

        select(&mut screen, "ui.visualizer_mode");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.ui.visualizer_mode, "spread");
        assert_eq!(label_fg(&mut screen, "Spread anchor"), text);
        select(&mut screen, "ui.visualizer_bars");
        assert!(!frame(&mut screen, 100, 70).contains("only used in spread mode"));

        // The cava tunables follow `cava.enabled`, and the art ones the backend.
        select(&mut screen, "plex.server");
        screen.cfg.cava.enabled = false;
        screen.cfg.album_art.backend = "none".into();
        assert_eq!(label_fg(&mut screen, "Visualizer fps"), dim);
        let switch = crate::inventory::find("cava.enabled").unwrap();
        assert!(
            screen.unused_because(switch, &screen.values()).is_none(),
            "the switch itself applies"
        );
        assert_eq!(label_fg(&mut screen, "Art pane width"), dim);
        assert_eq!(label_fg(&mut screen, "Art backend"), text);
        screen.cfg.cava.enabled = true;
        screen.cfg.album_art.backend = "auto".into();
        assert_eq!(label_fg(&mut screen, "Visualizer fps"), text);
        assert_eq!(label_fg(&mut screen, "Art pane width"), text);
    }

    // Library picker

    type Gate = std::sync::mpsc::Sender<Result<Vec<Section>, String>>;
    type Asked = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

    /// A screen whose library lookup waits for the test to answer, and notes
    /// the server and token it was asked with.
    fn screen_with_a_lookup() -> (Screen, Gate, Asked) {
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = std::sync::Mutex::new(rx);
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = asked.clone();
        let mut screen = ctx();
        screen.library_lookup = std::sync::Arc::new(move |server, token| {
            seen.lock().unwrap().push((server, token));
            rx.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| Err("test never answered".into()))
        });
        select(&mut screen, "plex.music_section");
        (screen, tx, asked)
    }

    fn section(key: &str, title: &str) -> Section {
        Section { key: key.into(), title: title.into() }
    }

    /// Wait (bounded) for the lookup's answer to be taken.
    fn settle(screen: &mut Screen) {
        for _ in 0..500 {
            screen.poll_library();
            if screen.library_rx.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the lookup never landed");
    }

    #[test]
    fn library_picker_stores_id() {
        let _palette = theme::palette_test_lock();
        let (mut screen, gate, asked) = screen_with_a_lookup();
        // Not saved yet: the lookup must use what the screen holds.
        screen.cfg.plex.server = "http://unsaved.example:32400".into();
        screen.cfg.plex.token = "fake-token-for-the-test".into();
        enter(&mut screen);
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("loading"), "{text}");
        assert!(text.contains("first one found"), "the default is always there: {text}");

        gate.send(Ok(vec![section("3", "Music"), section("7", "Live")])).unwrap();
        settle(&mut screen);
        let text = frame(&mut screen, 100, 30);
        assert!(
            !text.contains("loading") && text.contains("Music") && text.contains("Live"),
            "{text}"
        );
        assert_eq!(
            asked.lock().unwrap().as_slice(),
            [("http://unsaved.example:32400".to_string(), "fake-token-for-the-test".to_string())]
        );

        // default, Music, Live: the id goes in, not the name.
        press(&mut screen, 'j');
        press(&mut screen, 'j');
        enter(&mut screen);
        assert_eq!(screen.cfg.plex.music_section, "7");
        assert!(screen.editor.is_none());
    }

    #[test]
    fn failed_lookup_leaves_picker_usable() {
        let _palette = theme::palette_test_lock();
        let (mut screen, gate, _) = screen_with_a_lookup();
        screen.cfg.plex.music_section = "3".into();
        screen.cfg.plex.token = "fake-token-for-the-test".into();
        enter(&mut screen);
        gate.send(Err("could not ask the server".into())).unwrap();
        settle(&mut screen);
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("could not ask the server"), "{text}");
        assert!(!text.contains("fake-token"), "{text}");
        // The current id is still there to keep, and the default to go back to.
        assert!(text.contains("3 (current)"), "{text}");
        // Cursor starts on the current id; up to the default and choose it.
        press(&mut screen, 'k');
        enter(&mut screen);
        assert_eq!(screen.cfg.plex.music_section, "");
    }

    #[test]
    fn unlisted_library_id_stays_selectable() {
        let _palette = theme::palette_test_lock();
        let (mut screen, gate, _) = screen_with_a_lookup();
        screen.cfg.plex.music_section = "5".into();
        screen.saved = screen.cfg.clone();
        enter(&mut screen);
        gate.send(Ok(vec![section("3", "Music")])).unwrap();
        settle(&mut screen);
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("5 (not found)"), "{text}");
        // The cursor is on it, so Enter keeps it.
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert!(screen.editor.is_none());
        assert_eq!(screen.cfg.plex.music_section, "5");
        assert!(!screen.dirty());
    }

    #[test]
    fn id_can_still_be_typed() {
        let _palette = theme::palette_test_lock();
        for via_row in [false, true] {
            let (mut screen, _gate, _) = screen_with_a_lookup();
            enter(&mut screen);
            if via_row {
                // default, then `type an id…` (nothing has loaded).
                press(&mut screen, 'j');
                enter(&mut screen);
            } else {
                press(&mut screen, 't');
            }
            assert!(matches!(screen.editor, Some(Editor::Field { .. })));
            press(&mut screen, '9');
            enter(&mut screen);
            assert_eq!(screen.cfg.plex.music_section, "9");
        }
    }

    #[test]
    fn silent_server_is_given_up_on() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let server = format!("http://{}", listener.local_addr().expect("addr"));
        let started = std::time::Instant::now();
        let err = fetch_libraries(&server, "fake-token-for-the-test", Duration::from_millis(300))
            .expect_err("nobody answers");
        assert!(started.elapsed() < Duration::from_secs(5), "it hung");
        assert!(!err.contains("fake-token") && !err.contains("127.0.0.1"), "{err}");
    }

    // The argument list

    fn type_text(screen: &mut Screen, text: &str) {
        for c in text.chars() {
            press(screen, c);
        }
    }

    fn open_args(screen: &mut Screen) {
        select(screen, "mpv.extra_args");
        enter(screen);
        assert!(matches!(screen.editor, Some(Editor::List(_))));
    }

    #[test]
    fn comma_argument_survives_editor_and_file() {
        let _palette = theme::palette_test_lock();
        let (_dir, path, mut screen) = screen_with_an_edit();
        let af = "--af=lavfi=[loudnorm,volume=2]";
        open_args(&mut screen);
        press(&mut screen, 'a');
        type_text(&mut screen, af);
        enter(&mut screen);
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(screen.cfg.mpv.extra_args, vec![af.to_string()]);
        // Through the review and the writer, and back.
        press(&mut screen, 's');
        press(&mut screen, 's');
        assert_eq!(Config::read(&path).unwrap().mpv.extra_args, vec![af.to_string()]);
        // Opened and finished untouched, the argument is not split either.
        open_args(&mut screen);
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(screen.cfg.mpv.extra_args, vec![af.to_string()]);
        assert!(!screen.dirty());
    }

    #[test]
    fn list_rows_add_move_and_delete() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.cfg.mpv.extra_args = vec!["--a".into(), "--b".into(), "--c".into()];
        screen.saved = screen.cfg.clone();
        let args = |screen: &Screen| screen.cfg.mpv.extra_args.clone();
        let done = |screen: &mut Screen| screen.handle(key(KeyCode::Esc)).unwrap();

        open_args(&mut screen);
        press(&mut screen, 'J'); // --a down
        press(&mut screen, 'J');
        press(&mut screen, 'J'); // already last: stays
        press(&mut screen, 'K'); // up one
        press(&mut screen, 'a'); // after the cursor, empty: dropped again
        enter(&mut screen);
        done(&mut screen);
        assert_eq!(args(&screen), ["--b", "--a", "--c"]);

        open_args(&mut screen);
        press(&mut screen, 'j');
        press(&mut screen, 'd'); // --a
        done(&mut screen);
        assert_eq!(args(&screen), ["--b", "--c"]);

        open_args(&mut screen);
        press(&mut screen, 'd');
        press(&mut screen, 'd');
        press(&mut screen, 'd'); // nothing left: no panic
        done(&mut screen);
        assert!(args(&screen).is_empty());
        assert!(screen.dirty());
    }

    #[test]
    fn invalid_queue_columns_list_refused() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.saved = screen.cfg.clone();
        select(&mut screen, "ui.queue_columns");
        enter(&mut screen);
        press(&mut screen, 'a');
        type_text(&mut screen, "bogus");
        enter(&mut screen);
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert!(screen.status_error, "{}", screen.status);
        assert_eq!(screen.cfg.ui.queue_columns, crate::config::default_queue_columns());
        assert!(!screen.dirty());
    }

    #[test]
    fn cancelled_row_edit_changes_nothing() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.cfg.mpv.extra_args = vec!["--a".into()];
        screen.saved = screen.cfg.clone();
        open_args(&mut screen);
        enter(&mut screen);
        type_text(&mut screen, "xyz");
        screen.handle(key(KeyCode::Esc)).unwrap(); // the row edit
        press(&mut screen, 'a');
        type_text(&mut screen, "--new");
        screen.handle(key(KeyCode::Esc)).unwrap(); // a new row, cancelled: gone
        assert!(matches!(screen.editor, Some(Editor::List(_))), "still in the list");
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(screen.cfg.mpv.extra_args, ["--a"]);
        assert!(!screen.dirty());
    }

    // The save review and the counts

    fn screen_with_a_token() -> (tempfile::TempDir, PathBuf, Screen) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://x:32400\"\ntoken = \"ZZ-OLD-SECRET\"\n")
            .unwrap();
        let cfg = Config::read(&path).expect("read");
        let screen = ctx_for(path.clone(), cfg);
        (dir, path, screen)
    }

    #[test]
    fn review_lists_changes_never_secret() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_a_token();
        for (key, value) in [
            ("ui.art_pct", "55"),
            ("cava.enabled", "false"),
            ("mpv.extra_args", "--a, --b"),
            ("plex.token", "NEW-SECRET-VALUE"),
        ] {
            screen.set(crate::inventory::find(key).unwrap(), value);
        }
        press(&mut screen, 's');
        assert!(screen.review.is_some());
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("ui.art_pct") && text.contains("55"), "{text}");
        assert!(text.contains("cava.enabled") && text.contains("false"), "{text}");
        assert!(text.contains("mpv.extra_args") && text.contains("--a"), "{text}");
        assert!(text.contains('→'), "{text}");
        let token_row = text.lines().find(|l| l.contains(" plex.token ")).expect("token row");
        assert!(token_row.contains("(changed)"), "{token_row}");
        for leaked in ["NEW-SECRET-VALUE", "ZZ-OLD-SECRET"] {
            assert!(!text.contains(leaked) && !screen.status.contains(leaked), "{leaked} leaked");
        }
    }

    #[test]
    fn long_review_scrolls() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        let bools: Vec<_> = crate::inventory::ALL
            .iter()
            .filter(|s| s.kind == Kind::Bool && s.group != "Theme")
            .collect();
        for setting in &bools {
            let on = screen.value(setting).trim() != "true";
            screen.set(setting, &on.to_string());
        }
        screen.set(crate::inventory::find("mpv.path").unwrap(), &"/very/long/path/".repeat(12));
        press(&mut screen, 's');
        let last = screen.review.as_ref().unwrap().rows.last().unwrap().0.clone();
        let text = frame(&mut screen, 80, 14);
        assert!(!text.contains(&last), "the last row starts out of view: {text}");
        assert!(!text.contains(&"/very/long/path/".repeat(12)), "cut: {text}");
        let before = text.clone();
        for _ in 0..40 {
            press(&mut screen, 'j');
        }
        let after = frame(&mut screen, 80, 14);
        assert_ne!(before, after, "j scrolls");
        assert!(after.contains(&last), "the last change is reachable: {after}");
    }

    #[test]
    fn s_with_nothing_to_save_says_so() {
        let (_dir, path, mut screen) = screen_with_a_token();
        let before = std::fs::read_to_string(&path).unwrap();
        press(&mut screen, 's');
        assert!(screen.review.is_none());
        assert!(screen.status.contains("nothing to save"), "{}", screen.status);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn review_writes_only_on_s_or_enter() {
        let _palette = theme::palette_test_lock();
        let (_dir, path, mut screen) = screen_with_a_token();
        let before = std::fs::read_to_string(&path).unwrap();
        screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
        press(&mut screen, 's');
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert!(screen.review.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "nothing written");
        assert!(screen.dirty(), "the change is still pending");

        for confirm in [KeyCode::Enter, KeyCode::Char('s')] {
            let (_dir, path, mut screen) = screen_with_a_token();
            screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
            press(&mut screen, 's');
            screen.handle(key(confirm)).unwrap();
            assert_eq!(Config::read(&path).unwrap().ui.art_pct, 55, "{confirm:?}");
            assert!(screen.review.is_none() && !screen.dirty());
            assert!(screen.status.contains("next starts"), "{}", screen.status);
        }
    }

    #[test]
    fn tab_bar_counts_unsaved_changes() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_a_token();
        let text = frame(&mut screen, 100, 30);
        assert!(!text.contains('•'), "{text}");
        screen.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
        screen.set(crate::inventory::find("cava.enabled").unwrap(), "false");
        screen.set(crate::inventory::find("theme.accent").unwrap(), "#112233");
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("Config •2") && text.contains("Theme •1"), "{text}");
        assert!(!text.contains("Diagnostics •"), "{text}");
        press(&mut screen, 's');
        press(&mut screen, 's');
        let text = frame(&mut screen, 100, 30);
        assert!(!text.contains('•'), "{text}");
    }

    #[test]
    fn modes_keep_keys_bar_whole_when_narrow() {
        let _palette = theme::palette_test_lock();
        for w in [100u16, 60, 44, 30] {
            let mut lib = ctx();
            select(&mut lib, "plex.music_section");
            enter(&mut lib);
            let mut list = ctx();
            open_args(&mut list);
            let mut review = ctx();
            review.set(crate::inventory::find("ui.art_pct").unwrap(), "55");
            press(&mut review, 's');
            for (screen, last) in
                [(&mut lib, "Esc cancel"), (&mut list, "Esc done"), (&mut review, "Esc back")]
            {
                let text = frame(screen, w, 24);
                let bar = text
                    .lines()
                    .find(|l| l.contains(last))
                    .unwrap_or_else(|| panic!("{last} lost at {w}:\n{text}"));
                assert!(bar.trim_end_matches(['│', ' ']).ends_with(last), "{w}: {bar}");
            }
        }
    }

    #[test]
    fn dimmed_setting_says_why_unused() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        assert_ne!(screen.cfg.cava.style, "custom");
        select(&mut screen, "cava.noise_reduction");
        enter(&mut screen);
        screen.handle(key(KeyCode::Backspace)).unwrap();
        screen.handle(key(KeyCode::Backspace)).unwrap();
        screen.handle(key(KeyCode::Backspace)).unwrap();
        type_text(&mut screen, "60");
        enter(&mut screen);
        assert_eq!(screen.cfg.cava.noise_reduction, 60, "still saved");
        assert!(
            screen.status.starts_with("cava.noise_reduction = 60; ")
                && screen.status.contains("custom style"),
            "{}",
            screen.status
        );
        // The key name is not clipped off a narrow status line.
        let text = frame(&mut screen, 50, 24);
        assert!(text.contains("cava.noise_reduction"), "{text}");

        // ←/→ go through the same road.
        screen.status.clear();
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.cava.noise_reduction, 65);
        assert!(screen.status.contains("custom style"), "{}", screen.status);

        // A setting that applies gets no note, and neither does this one once
        // the style that uses it is chosen.
        select(&mut screen, "ui.art_pct");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.status.matches(';').count(), 0, "{}", screen.status);
        screen.set(crate::inventory::find("cava.style").unwrap(), "custom");
        select(&mut screen, "cava.noise_reduction");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert!(!screen.status.contains(';'), "{}", screen.status);
    }

    /// A theme screen on a file with two colour overrides on the default theme.
    fn screen_with_overrides() -> (tempfile::TempDir, PathBuf, Screen) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[plex]\nserver = \"http://x:32400\"\n[theme]\nname = \"default\"\naccent = \"#112233\"\nsecondary = \"#445566\"\n",
        )
        .unwrap();
        let cfg = Config::read(&path).expect("read");
        let mut screen = ctx_for(path.clone(), cfg);
        screen.apply_theme();
        screen.switch(Tab::Theme);
        select(&mut screen, "theme.name");
        (dir, path, screen)
    }

    /// Open the chooser and move to `name`.
    fn pick_theme(screen: &mut Screen, name: &str) {
        enter(screen);
        move_picker(screen, name);
    }

    /// Move the open chooser to `name`.
    fn move_picker(screen: &mut Screen, name: &str) {
        let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
            panic!("the chooser is not open");
        };
        let (from, to) = (*index, options.iter().position(|o| o == name).expect("listed"));
        for _ in 0..to.abs_diff(from) {
            press(screen, if to > from { 'j' } else { 'k' });
        }
    }

    fn reset_palette(screen: &mut Screen) {
        screen.cfg.theme = crate::config::ThemeConfig::default();
        screen.apply_theme();
    }

    #[test]
    fn other_theme_clears_overrides_until_saved() {
        let _palette = theme::palette_test_lock();
        let (_dir, path, mut screen) = screen_with_overrides();
        pick_theme(&mut screen, "nord");
        // While moving, the plain theme is shown, not the overrides on it.
        assert_eq!(theme::theme().accent, themes::preset("nord").unwrap().theme.accent);
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.name, "nord");
        assert!(screen.cfg.theme.accent.is_none() && screen.cfg.theme.secondary.is_none());
        assert!(screen.status.contains("2 overrides cleared"), "{}", screen.status);
        assert_eq!(theme::theme().accent, themes::preset("nord").unwrap().theme.accent);

        // Nothing is gone from the file yet; the review and the count say so.
        assert!(std::fs::read_to_string(&path).unwrap().contains("#112233"));
        let text = frame(&mut screen, 100, 30);
        assert!(text.contains("Theme •3"), "{text}");
        press(&mut screen, 's');
        let text = frame(&mut screen, 100, 30);
        let row = text
            .lines()
            .find(|l| l.contains("theme.accent"))
            .expect("the removed override is listed");
        assert!(row.contains("#112233") && row.contains("(theme)"), "{row}");
        press(&mut screen, 's');
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(!file.contains("#112233") && !file.contains("#445566"), "{file}");
        let back = Config::read(&path).unwrap();
        assert_eq!(back.theme.name, "nord");
        assert!(back.theme.accent.is_none() && back.theme.secondary.is_none());
        reset_palette(&mut screen);
    }

    #[test]
    fn same_theme_and_esc_keep_overrides() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_overrides();
        let before = theme::theme();
        pick_theme(&mut screen, "nord");
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(screen.cfg.theme.name, "default");
        assert_eq!(screen.cfg.theme.accent.as_deref(), Some("#112233"));
        assert_eq!(theme::theme(), before);
        enter(&mut screen);
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.accent.as_deref(), Some("#112233"));
        assert_eq!(screen.cfg.theme.secondary.as_deref(), Some("#445566"));
        assert!(!screen.status.contains("cleared"), "{}", screen.status);
        assert!(!screen.dirty());
        reset_palette(&mut screen);
    }

    #[test]
    fn stepping_the_theme_clears_the_overrides_too() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_overrides();
        screen.cfg.theme.secondary = None;
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_ne!(screen.cfg.theme.name, "default");
        assert!(screen.cfg.theme.accent.is_none());
        assert!(screen.status.contains("1 override cleared"), "{}", screen.status);
        reset_palette(&mut screen);
    }

    /// Choosing another theme drops the spacing and symbol overrides with the
    /// colours, and the status counts all of them.
    #[test]
    fn other_theme_clears_spacing_symbol_overrides() {
        let _palette = theme::palette_test_lock();
        let (_dir, _path, mut screen) = screen_with_overrides();
        screen.cfg.theme.inset = Some(2);
        screen.cfg.theme.scroll_up = Some("^".into());
        pick_theme(&mut screen, "nord");
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.name, "nord");
        assert!(screen.cfg.theme.inset.is_none() && screen.cfg.theme.scroll_up.is_none());
        assert!(screen.status.contains("4 overrides cleared"), "{}", screen.status);
        assert_eq!(theme::spacing(), theme::Spacing::DEFAULT);
        reset_palette(&mut screen);
    }

    /// A bad symbol is refused with its key named; `r` drops an override, or
    /// says the row already matches.
    #[test]
    fn bad_symbol_is_refused() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let ellipsis = inventory::find("theme.ellipsis").unwrap();
        let before = snap(&screen.cfg);
        for bad in ["界", "", "ab", "\u{1b}"] {
            screen.set(ellipsis, bad);
            assert!(
                screen.status.starts_with("rejected") && screen.status_error,
                "{bad:?}: {}",
                screen.status
            );
            assert!(screen.status.contains("theme.ellipsis"), "{}", screen.status);
            assert_eq!(snap(&screen.cfg), before, "{bad:?} changed the config");
        }
        screen.set(inventory::find("theme.volume_meter").unwrap(), "■");
        assert!(screen.status.starts_with("rejected"), "one glyph is neither a ramp nor a pair");

        screen.set(ellipsis, "~");
        assert_eq!(screen.cfg.theme.ellipsis.as_deref(), Some("~"));
        select(&mut screen, "theme.ellipsis");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.ellipsis, None);
        assert!(screen.status.contains("removed the theme.ellipsis override"), "{}", screen.status);
        press(&mut screen, 'r');
        assert!(screen.status.contains("already matches"), "{}", screen.status);
        reset_palette(&mut screen);
    }

    /// The preview copies the player's header and visualizer strip, so it
    /// draws the symbols in force and sits its blocks apart by the gap.
    #[test]
    fn preview_follows_spacing_and_symbols() {
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let tops = |text: &str| -> Vec<usize> {
            text.lines().enumerate().filter(|(_, l)| l.starts_with('┌')).map(|(i, _)| i).collect()
        };
        let plain = frame(&mut screen, 100, 45);
        assert!(plain.contains("Volume: ▁▂▃▄▅▆▇█  60%"), "{plain}");
        assert_eq!(&tops(&plain)[..3], [0, 3, 7], "tabs, header, sample");

        let mut look = theme::Look::DEFAULT;
        assert!(theme::set_spacing(&mut look, "block_gap", 2));
        assert!(theme::set_spacing(&mut look, "inset", 2));
        assert!(theme::set_symbol(&mut look, "volume_meter", "■·"));
        assert!(theme::set_symbol(&mut look, "visualizer", "⠁⠃⠇⠏⠟⠿⡿⣿"));
        theme::set_look(look);
        let text = frame(&mut screen, 100, 45);
        // 60%: five lit cells and three unlit, so both glyphs of the pair show.
        assert!(text.contains("Volume: ■■■■■···  60%"), "{text}");
        assert_eq!(&tops(&text)[..3], [0, 3, 9], "the sample is two rows lower");
        assert!(text.contains('⣿') && text.contains('⠿'), "the strip uses the ramp:\n{text}");
        let header = text.lines().nth(4).unwrap();
        assert_eq!(
            header.find('[').map(|i| header[..i].chars().count()),
            Some(3),
            "inset 2: {header}"
        );
        for glyph in "▁▂▃▄▅▆▇".chars() {
            assert!(!text.contains(glyph), "{glyph} is still drawn:\n{text}");
        }
    }

    /// `solid` is accepted on the progress glyph rows (the screen's rule is the
    /// loader's) and stored as typed; `r` removes it like any override.
    #[test]
    fn progress_rows_accept_solid() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        for key in ["theme.progress_filled", "theme.progress_empty", "theme.progress_head"] {
            assert_eq!(symbol_value(key, "solid").unwrap(), toml::Value::String("solid".into()));
        }
        assert!(symbol_value("theme.ellipsis", "solid").is_err());
        let head = inventory::find("theme.progress_head").unwrap();
        screen.set(head, "solid");
        assert_eq!(screen.cfg.theme.progress_head.as_deref(), Some("solid"));
        select(&mut screen, "theme.progress_head");
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.progress_head, None);
    }

    /// The progress bar row is a picker of the preset names; the choice is
    /// stored as the name, and a name is all it takes.
    #[test]
    fn progress_bar_row_stores_preset_name() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let bar = inventory::find("theme.progress_bar").unwrap();
        assert_eq!(screen.value(bar), "solid");
        select(&mut screen, "theme.progress_bar");
        enter(&mut screen);
        let Some(Editor::Picker { options, .. }) = &screen.editor else {
            panic!("no picker");
        };
        assert_eq!(options, &["solid", "line", "dot", "plain", "angled", "rounded", "custom…"]);
        press(&mut screen, 'j');
        press(&mut screen, 'j');
        assert_eq!(theme::symbols().progress_head, Some('●'), "the preview follows the highlight");
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.progress_bar.as_deref(), Some("dot"));
        assert!(symbol_value("theme.progress_bar", "=").is_err(), "a name only");
    }

    /// The meter and the visualizer rows pick a preset (then `custom…` for
    /// glyphs) and store the name, not the glyphs; `r` removes it.
    #[test]
    fn meter_visualizer_rows_store_preset_name() {
        let _palette = theme::palette_test_lock();
        let mut screen = ctx();
        screen.switch(Tab::Theme);
        let meter = inventory::find("theme.volume_meter").unwrap();
        assert_eq!(screen.value(meter), "blocks", "unset is the default preset");

        select(&mut screen, "theme.volume_meter");
        enter(&mut screen);
        let Some(Editor::Picker { options, index, .. }) = &screen.editor else {
            panic!("no picker");
        };
        assert_eq!(options, &["blocks", "squares", "outline", "plain", "custom…"]);
        assert_eq!(*index, 0);
        // The sample follows the highlight, and Esc puts the old look back.
        for _ in 0..3 {
            press(&mut screen, 'j');
        }
        assert_eq!(theme::symbols().volume_meter, theme::Meter::Pair('#', '-'));
        assert!(frame(&mut screen, 100, 45).contains("Volume: #####---  60%"));
        screen.handle(key(KeyCode::Esc)).unwrap();
        assert_eq!(theme::symbols().volume_meter, theme::Symbols::DEFAULT.volume_meter);

        enter(&mut screen);
        press(&mut screen, 'j');
        enter(&mut screen);
        assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some("squares"), "the name, not ■·");
        assert_eq!(screen.value(meter), "squares");
        // ←/→ cycle the names and wrap.
        for want in ["outline", "plain", "blocks", "squares"] {
            screen.handle(key(KeyCode::Right)).unwrap();
            assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some(want));
        }
        press(&mut screen, 'r');
        assert_eq!(screen.cfg.theme.volume_meter, None);

        // Glyphs typed show as themselves and are left alone by ←/→; the
        // picker opens on `custom…`, which leads to the text field.
        screen.set(meter, "<>");
        assert_eq!(screen.value(meter), "<>");
        screen.handle(key(KeyCode::Right)).unwrap();
        assert_eq!(screen.cfg.theme.volume_meter.as_deref(), Some("<>"));
        enter(&mut screen);
        let Some(Editor::Picker { index, .. }) = &screen.editor else {
            panic!("no picker");
        };
        assert_eq!(*index, 4);
        enter(&mut screen);
        let Some(Editor::Field { initial, .. }) = &screen.editor else {
            panic!("custom… did not open the field");
        };
        assert_eq!(initial, "<>");
        reset_palette(&mut screen);
    }

    #[test]
    fn paste_during_question_does_not_edit_filter() {
        let mut screen = ctx();
        type_filter(&mut screen, "ar");
        screen.handle(with_ctrl('c')).unwrap();
        assert!(screen.quit_ask.is_some());
        screen.handle_paste("zzz".into());
        assert_eq!(screen.filter.text, "ar");
    }
}
