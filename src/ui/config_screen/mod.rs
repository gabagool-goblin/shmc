//! The config screen. Not part of `App`: it has no Plex client, mpv or playback state.

use crate::config::{Config, SettingRow};
use crate::doctor::{self, Report};
use crate::inventory::{self, Setting};
use crate::plex::models::Section;
use crate::ui::filter::{self, ListFilter};
use crate::ui::text_input::TextInput;
use crate::ui::theme;
use crate::ui::themes;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::time::Duration;

mod draw;
mod edit;
mod library;
mod save;

use edit::ListEdit;
use library::{fetch_libraries, LibraryLookup, LibraryPick, LIBRARY_TIMEOUT};
use save::Review;

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
    Clean,
    Dirty,
}

type CheckSlot = std::sync::Arc<std::sync::Mutex<Option<Result<Report, String>>>>;

enum Editor {
    /// Text, number or colour.
    Field {
        text: TextInput,
        /// Committing exactly this changes nothing: the text is a lossy view of some values.
        initial: String,
        /// Masked and starts empty: the list's placeholder must never replace the secret.
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

/// A drawn row: a group heading, or a setting by its position in `Screen::view`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Header(&'static str),
    Item(usize),
}

/// The last option of a preset picker: type the glyphs instead.
const CUSTOM: &str = "custom…";

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
    scrolloff: usize,
    diag_scroll: usize,
    /// Set when the report pane draws: paging and `End` need its height.
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
    /// On a thread: the Plex client's timeouts and retries would freeze the screen.
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

    /// Returns the config as left, saved or not.
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

    /// Re-filters, keeping the cursor on its setting or the next match.
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
            KeyCode::Char('y' | 'Y') => false,
            // Too easy to hit by reflex with edits pending: discarding takes `y`.
            KeyCode::Enter | KeyCode::Char('q') if ask == QuitAsk::Clean => false,
            // A wedged screen still needs an exit.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => false,
            KeyCode::Char('s') if ask == QuitAsk::Dirty => {
                // A failed or conflicted save keeps the screen open.
                !self.save()
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
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

    /// Enter keeps the filter; Esc, or deleting its last character, drops it.
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
}

#[cfg(test)]
mod tests;
