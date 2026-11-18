//! The one table of (context, key) -> action: the shipped defaults and the
//! runtime keymap built from `[keys]`, so dispatch and Help cannot drift.

use crate::config::KeyTable;
use crate::ui::app::Tab;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCtx {
    Global,
    Queue,
    Browser,
    Playlists,
    Search,
    Help,
}

impl KeyCtx {
    /// The `[keys.<section>]` name; `lists` is not a context.
    pub fn config_name(self) -> &'static str {
        SECTIONS.iter().find(|(ctx, _, _)| *ctx == self).map_or("", |(_, _, name)| name)
    }

    pub fn from_config_name(name: &str) -> Option<Self> {
        SECTIONS.iter().find(|(_, _, config)| *config == name).map(|(ctx, _, _)| *ctx)
    }
}

/// Sections in the order the Help tab lists them, with their `[keys.<section>]`
/// names.
pub const SECTIONS: &[(KeyCtx, &str, &str)] = &[
    (KeyCtx::Global, "Global", "global"),
    (KeyCtx::Queue, "Queue", "queue"),
    (KeyCtx::Browser, "Browser", "browser"),
    (KeyCtx::Playlists, "Playlists", "playlists"),
    (KeyCtx::Search, "Search", "search"),
    (KeyCtx::Help, "Help", "help"),
];

/// What a key does. The default keys and each action's Help text are in
/// [`BINDINGS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Quit,
    NextTab,
    PrevTab,
    GotoTab(Tab),

    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Top,
    Bottom,

    ColumnLeft,
    ColumnRight,

    Activate,
    AddToQueue,
    PlayNext,
    MoveQueueUp,
    MoveQueueDown,
    Play,
    VisualMode,
    RemoveFromQueue,
    UndoRemove,
    ClearQueue,
    GotoAlbum,
    JumpToPlaying,
    Back,
    StartFilter,
    StartSearch,
    OpenCommand,
    TogglePause,
    NextTrack,
    PrevTrack,
    VolUp,
    VolDown,
    SeekFwd,
    SeekBack,
    ToggleRepeat,
    ToggleShuffle,
    Reshuffle,
    ToggleSingle,
    ToggleConsume,
    SleepTimer,
    Rate,
    Similar,
}

/// One name per action, shared by the parser, `--list`, `--check` and Help.
const ACT_NAMES: &[(&str, Act)] = &[
    ("quit", Act::Quit),
    ("next_tab", Act::NextTab),
    ("prev_tab", Act::PrevTab),
    ("tab_queue", Act::GotoTab(Tab::Queue)),
    ("tab_browser", Act::GotoTab(Tab::Browser)),
    ("tab_playlists", Act::GotoTab(Tab::Playlists)),
    ("tab_search", Act::GotoTab(Tab::Search)),
    ("tab_help", Act::GotoTab(Tab::Help)),
    ("move_up", Act::MoveUp),
    ("move_down", Act::MoveDown),
    ("page_up", Act::PageUp),
    ("page_down", Act::PageDown),
    ("top", Act::Top),
    ("bottom", Act::Bottom),
    ("column_left", Act::ColumnLeft),
    ("column_right", Act::ColumnRight),
    ("activate", Act::Activate),
    ("play", Act::Play),
    ("add", Act::AddToQueue),
    ("play_next", Act::PlayNext),
    ("move_track_up", Act::MoveQueueUp),
    ("move_track_down", Act::MoveQueueDown),
    ("visual", Act::VisualMode),
    ("remove", Act::RemoveFromQueue),
    ("undo", Act::UndoRemove),
    ("clear", Act::ClearQueue),
    ("show_in_browser", Act::GotoAlbum),
    ("jump_to_playing", Act::JumpToPlaying),
    ("back", Act::Back),
    ("filter", Act::StartFilter),
    ("search", Act::StartSearch),
    ("command_line", Act::OpenCommand),
    ("toggle_pause", Act::TogglePause),
    ("next_track", Act::NextTrack),
    ("prev_track", Act::PrevTrack),
    ("volume_up", Act::VolUp),
    ("volume_down", Act::VolDown),
    ("seek_forward", Act::SeekFwd),
    ("seek_back", Act::SeekBack),
    ("toggle_repeat", Act::ToggleRepeat),
    ("toggle_shuffle", Act::ToggleShuffle),
    ("reshuffle", Act::Reshuffle),
    ("toggle_single", Act::ToggleSingle),
    ("toggle_consume", Act::ToggleConsume),
    ("sleep_timer", Act::SleepTimer),
    ("rate", Act::Rate),
    ("similar", Act::Similar),
];

impl Act {
    pub fn name(&self) -> Option<&'static str> {
        ACT_NAMES.iter().find(|(_, act)| act == self).map(|(name, _)| *name)
    }

    pub fn from_name(name: &str) -> Option<Self> {
        ACT_NAMES.iter().find(|(candidate, _)| *candidate == name).map(|(_, act)| *act)
    }
}

pub struct Binding {
    pub ctx: KeyCtx,
    pub keys: &'static [&'static str],
    pub act: Act,
    pub desc: &'static str,
}

/// The default keymap. A `[keys]` entry replaces an action's keys here.
#[rustfmt::skip]
pub const BINDINGS: &[Binding] = &[
    Binding { ctx: KeyCtx::Global, keys: &["q", "C-c"], act: Act::Quit, desc: "quit" },
    Binding { ctx: KeyCtx::Global, keys: &["Tab"], act: Act::NextTab, desc: "next tab" },
    Binding { ctx: KeyCtx::Global, keys: &["S-Tab"], act: Act::PrevTab, desc: "previous tab" },
    Binding { ctx: KeyCtx::Global, keys: &["1"], act: Act::GotoTab(Tab::Queue), desc: "go to Queue tab" },
    Binding { ctx: KeyCtx::Global, keys: &["2"], act: Act::GotoTab(Tab::Browser), desc: "go to Browser tab" },
    Binding { ctx: KeyCtx::Global, keys: &["3"], act: Act::GotoTab(Tab::Playlists), desc: "go to Playlists tab" },
    Binding { ctx: KeyCtx::Global, keys: &["4"], act: Act::GotoTab(Tab::Search), desc: "go to Search tab" },
    Binding { ctx: KeyCtx::Global, keys: &["5", "?"], act: Act::GotoTab(Tab::Help), desc: "go to Help tab" },
    Binding { ctx: KeyCtx::Global, keys: &[":"], act: Act::OpenCommand, desc: "command line" },
    Binding { ctx: KeyCtx::Global, keys: &["Space"], act: Act::TogglePause, desc: "pause or resume" },
    Binding { ctx: KeyCtx::Global, keys: &[">"], act: Act::NextTrack, desc: "next track" },
    Binding { ctx: KeyCtx::Global, keys: &["<"], act: Act::PrevTrack, desc: "previous track, or restart it after a few seconds" },
    Binding { ctx: KeyCtx::Global, keys: &["."], act: Act::VolUp, desc: "volume up" },
    Binding { ctx: KeyCtx::Global, keys: &[","], act: Act::VolDown, desc: "volume down" },
    Binding { ctx: KeyCtx::Global, keys: &["]"], act: Act::SeekFwd, desc: "seek forward 5s" },
    Binding { ctx: KeyCtx::Global, keys: &["["], act: Act::SeekBack, desc: "seek back 5s" },
    Binding { ctx: KeyCtx::Global, keys: &["z"], act: Act::ToggleShuffle, desc: "shuffle on or off" },
    Binding { ctx: KeyCtx::Global, keys: &["Z"], act: Act::Reshuffle, desc: "reshuffle the queue" },
    Binding { ctx: KeyCtx::Global, keys: &["x"], act: Act::ToggleRepeat, desc: "repeat (loop playlist)" },
    Binding { ctx: KeyCtx::Global, keys: &["c"], act: Act::ToggleSingle, desc: "single (loop track)" },
    Binding { ctx: KeyCtx::Global, keys: &["C"], act: Act::ToggleConsume, desc: "consume (remove played tracks)" },
    Binding { ctx: KeyCtx::Global, keys: &["S"], act: Act::SleepTimer, desc: "sleep timer: 15, 30 or 60 minutes, or off" },
    Binding { ctx: KeyCtx::Global, keys: &["r"], act: Act::Rate, desc: "rate the playing track" },
    Binding { ctx: KeyCtx::Queue, keys: &["k", "Up"], act: Act::MoveUp, desc: "move up" },
    Binding { ctx: KeyCtx::Queue, keys: &["j", "Down"], act: Act::MoveDown, desc: "move down" },
    Binding { ctx: KeyCtx::Queue, keys: &["PgUp"], act: Act::PageUp, desc: "page up" },
    Binding { ctx: KeyCtx::Queue, keys: &["PgDn"], act: Act::PageDown, desc: "page down" },
    Binding { ctx: KeyCtx::Queue, keys: &["gg", "Home"], act: Act::Top, desc: "go to top" },
    Binding { ctx: KeyCtx::Queue, keys: &["G", "End"], act: Act::Bottom, desc: "go to bottom" },
    Binding { ctx: KeyCtx::Queue, keys: &["Enter"], act: Act::Play, desc: "play selected track" },
    Binding { ctx: KeyCtx::Queue, keys: &["v"], act: Act::VisualMode, desc: "select a range (visual mode)" },
    Binding { ctx: KeyCtx::Queue, keys: &["d"], act: Act::RemoveFromQueue, desc: "remove from queue" },
    Binding { ctx: KeyCtx::Queue, keys: &["u"], act: Act::UndoRemove, desc: "undo a removal (repeat for older ones)" },
    Binding { ctx: KeyCtx::Queue, keys: &["D"], act: Act::ClearQueue, desc: "clear queue" },
    Binding { ctx: KeyCtx::Queue, keys: &["b"], act: Act::GotoAlbum, desc: "show track in Browser" },
    Binding { ctx: KeyCtx::Queue, keys: &["m"], act: Act::Similar, desc: "sonically similar tracks" },
    Binding { ctx: KeyCtx::Queue, keys: &["o"], act: Act::JumpToPlaying, desc: "jump to the playing track and resume follow" },
    Binding { ctx: KeyCtx::Queue, keys: &["J"], act: Act::MoveQueueDown, desc: "move track down" },
    Binding { ctx: KeyCtx::Queue, keys: &["K"], act: Act::MoveQueueUp, desc: "move track up" },
    Binding { ctx: KeyCtx::Queue, keys: &["/"], act: Act::StartFilter, desc: "filter queue" },
    Binding { ctx: KeyCtx::Browser, keys: &["h", "Left"], act: Act::ColumnLeft, desc: "focus the column to the left" },
    Binding { ctx: KeyCtx::Browser, keys: &["l", "Right"], act: Act::ColumnRight, desc: "open, or focus the column to the right" },
    Binding { ctx: KeyCtx::Browser, keys: &["k", "Up"], act: Act::MoveUp, desc: "move up" },
    Binding { ctx: KeyCtx::Browser, keys: &["j", "Down"], act: Act::MoveDown, desc: "move down" },
    Binding { ctx: KeyCtx::Browser, keys: &["PgUp"], act: Act::PageUp, desc: "page up" },
    Binding { ctx: KeyCtx::Browser, keys: &["PgDn"], act: Act::PageDown, desc: "page down" },
    Binding { ctx: KeyCtx::Browser, keys: &["gg", "Home"], act: Act::Top, desc: "go to top" },
    Binding { ctx: KeyCtx::Browser, keys: &["G", "End"], act: Act::Bottom, desc: "go to bottom" },
    Binding { ctx: KeyCtx::Browser, keys: &["Enter"], act: Act::Activate, desc: "play album, or track and the rest" },
    Binding { ctx: KeyCtx::Browser, keys: &["a"], act: Act::AddToQueue, desc: "add artist, album or track to queue" },
    Binding { ctx: KeyCtx::Browser, keys: &["i"], act: Act::PlayNext, desc: "insert artist, album or track after the playing track" },
    Binding { ctx: KeyCtx::Browser, keys: &["v"], act: Act::VisualMode, desc: "select tracks (visual mode)" },
    Binding { ctx: KeyCtx::Browser, keys: &["m"], act: Act::Similar, desc: "sonically similar tracks, albums or artists" },
    Binding { ctx: KeyCtx::Browser, keys: &["o"], act: Act::JumpToPlaying, desc: "jump to playing track" },
    Binding { ctx: KeyCtx::Browser, keys: &["/"], act: Act::StartFilter, desc: "filter this column" },
    Binding { ctx: KeyCtx::Browser, keys: &["Esc", "BSpace"], act: Act::Back, desc: "back one column, or to the Queue from the artists" },
    Binding { ctx: KeyCtx::Playlists, keys: &["h", "Left"], act: Act::ColumnLeft, desc: "focus list" },
    Binding { ctx: KeyCtx::Playlists, keys: &["l", "Right"], act: Act::ColumnRight, desc: "focus tracks" },
    Binding { ctx: KeyCtx::Playlists, keys: &["k", "Up"], act: Act::MoveUp, desc: "move up" },
    Binding { ctx: KeyCtx::Playlists, keys: &["j", "Down"], act: Act::MoveDown, desc: "move down" },
    Binding { ctx: KeyCtx::Playlists, keys: &["PgUp"], act: Act::PageUp, desc: "page up" },
    Binding { ctx: KeyCtx::Playlists, keys: &["PgDn"], act: Act::PageDown, desc: "page down" },
    Binding { ctx: KeyCtx::Playlists, keys: &["gg", "Home"], act: Act::Top, desc: "go to top" },
    Binding { ctx: KeyCtx::Playlists, keys: &["G", "End"], act: Act::Bottom, desc: "go to bottom" },
    Binding { ctx: KeyCtx::Playlists, keys: &["Enter"], act: Act::Activate, desc: "play playlist, or track and the rest" },
    Binding { ctx: KeyCtx::Playlists, keys: &["a"], act: Act::AddToQueue, desc: "add playlist, or the rest of it, to queue" },
    Binding { ctx: KeyCtx::Playlists, keys: &["i"], act: Act::PlayNext, desc: "insert playlist or track after the playing track" },
    Binding { ctx: KeyCtx::Playlists, keys: &["v"], act: Act::VisualMode, desc: "select tracks (visual mode)" },
    Binding { ctx: KeyCtx::Playlists, keys: &["m"], act: Act::Similar, desc: "sonically similar tracks" },
    Binding { ctx: KeyCtx::Playlists, keys: &["o"], act: Act::JumpToPlaying, desc: "jump to playing track" },
    Binding { ctx: KeyCtx::Playlists, keys: &["/"], act: Act::StartFilter, desc: "filter this list" },
    Binding { ctx: KeyCtx::Search, keys: &["k", "Up"], act: Act::MoveUp, desc: "move up" },
    Binding { ctx: KeyCtx::Search, keys: &["j", "Down"], act: Act::MoveDown, desc: "move down" },
    Binding { ctx: KeyCtx::Search, keys: &["PgUp"], act: Act::PageUp, desc: "page up" },
    Binding { ctx: KeyCtx::Search, keys: &["PgDn"], act: Act::PageDown, desc: "page down" },
    Binding { ctx: KeyCtx::Search, keys: &["gg", "Home"], act: Act::Top, desc: "go to top" },
    Binding { ctx: KeyCtx::Search, keys: &["G", "End"], act: Act::Bottom, desc: "go to bottom" },
    Binding { ctx: KeyCtx::Search, keys: &["Enter"], act: Act::Activate, desc: "play track and the rest, or open album" },
    Binding { ctx: KeyCtx::Search, keys: &["a"], act: Act::AddToQueue, desc: "add track or album to queue" },
    Binding { ctx: KeyCtx::Search, keys: &["i"], act: Act::PlayNext, desc: "insert track or album after the playing track" },
    Binding { ctx: KeyCtx::Search, keys: &["v"], act: Act::VisualMode, desc: "select tracks (visual mode)" },
    Binding { ctx: KeyCtx::Search, keys: &["m"], act: Act::Similar, desc: "sonically similar tracks, albums or artists" },
    Binding { ctx: KeyCtx::Search, keys: &["b"], act: Act::GotoAlbum, desc: "show result in Browser" },
    Binding { ctx: KeyCtx::Search, keys: &["/"], act: Act::StartSearch, desc: "new search" },
    Binding { ctx: KeyCtx::Search, keys: &["Esc"], act: Act::Back, desc: "close album view" },
    Binding { ctx: KeyCtx::Help, keys: &["k", "Up"], act: Act::MoveUp, desc: "scroll up" },
    Binding { ctx: KeyCtx::Help, keys: &["j", "Down"], act: Act::MoveDown, desc: "scroll down" },
    Binding { ctx: KeyCtx::Help, keys: &["PgUp"], act: Act::PageUp, desc: "page up" },
    Binding { ctx: KeyCtx::Help, keys: &["PgDn"], act: Act::PageDown, desc: "page down" },
    Binding { ctx: KeyCtx::Help, keys: &["gg", "Home"], act: Act::Top, desc: "go to top" },
    Binding { ctx: KeyCtx::Help, keys: &["G", "End"], act: Act::Bottom, desc: "go to bottom" },
    Binding { ctx: KeyCtx::Help, keys: &["Esc", "q"], act: Act::Back, desc: "back to previous tab" },
];

/// The `:` commands, listed in Help and run by command mode.
pub const COMMANDS: &[(&str, &str)] = &[
    (":q / :quit", "quit"),
    (":n / :next", "next track"),
    (":p / :prev", "previous track"),
    (":pause", "toggle pause"),
    (":play / :resume", "play or resume"),
    (":stop", "stop (keep queue)"),
    (":seek N", "seek N seconds (negative = back)"),
    (":vol N", "set volume (0-100, or ±N relative)"),
    (":shuffle", "shuffle on or off"),
    (":repeat", "toggle repeat playlist"),
    (":single", "toggle single track"),
    (":consume", "toggle consume"),
    (":sleep N", "sleep timer: N minutes, or off"),
    (":reshuffle", "reshuffle the queue"),
    (":queue", "go to Queue tab"),
    (":browser", "go to Browser tab"),
    (":playlists", "go to Playlists tab"),
    (":search", "go to Search tab and start typing"),
    (":help", "go to Help tab"),
    (":playing", "jump to the playing track"),
    (":clear", "clear the queue"),
    (":save NAME", "save the queue as a new Plex playlist"),
];

/// `[keys.<section>]` entries grouped by the section they apply to.
type SectionOverrides = Vec<(KeyCtx, Vec<(Act, Vec<String>)>)>;

/// One row of a keymap: a context's action with the keys it answers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    pub ctx: KeyCtx,
    pub keys: Vec<String>,
    pub act: Act,
    pub desc: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    entries: Vec<MapEntry>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            entries: BINDINGS
                .iter()
                .map(|binding| MapEntry {
                    ctx: binding.ctx,
                    keys: binding.keys.iter().map(|key| (*key).to_string()).collect(),
                    act: binding.act,
                    desc: binding.desc,
                })
                .collect(),
        }
    }
}

impl Keymap {
    /// The default keymap with `[keys]` applied. Any problem rejects the whole
    /// table: a half-applied keymap would be a mystery to explain.
    pub fn from_table(table: &KeyTable) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut overrides: SectionOverrides = Vec::new();
        let mut lists: Vec<(Act, Vec<String>)> = Vec::new();

        for (section, actions) in table {
            let is_lists = section == "lists";
            let ctx = KeyCtx::from_config_name(section);
            if !is_lists && ctx.is_none() {
                if actions.is_empty() {
                    problems.push(format!("keys.{section}: unknown section"));
                }
                for action in actions.keys() {
                    problems.push(format!("keys.{section}.{action}: unknown section"));
                }
                continue;
            }
            for (action_name, keys) in actions {
                let Some(act) = Act::from_name(action_name) else {
                    problems.push(format!("keys.{section}.{action_name}: unknown action"));
                    continue;
                };
                if keys.iter().any(|key| !is_key_name(key)) {
                    problems.push(format!("keys.{section}.{action_name}: unknown key name"));
                }
                if keys.iter().any(|key| key == "C-c") {
                    problems.push(format!("keys.{section}.{action_name}: C-c is reserved"));
                }
                let allowed = if is_lists {
                    is_movement(&act)
                } else {
                    let ctx = ctx.expect("known context checked above");
                    BINDINGS.iter().any(|b| b.ctx == ctx && b.act == act)
                };
                if !allowed {
                    problems
                        .push(format!("keys.{section}.{action_name}: not allowed in this section"));
                    continue;
                }
                if is_lists {
                    lists.push((act, keys.clone()));
                } else {
                    let ctx = ctx.expect("known context checked above");
                    match overrides.iter_mut().find(|(c, _)| *c == ctx) {
                        Some((_, entries)) => entries.push((act, keys.clone())),
                        None => overrides.push((ctx, vec![(act, keys.clone())])),
                    }
                }
            }
        }

        if !problems.is_empty() {
            return (Self::default(), problems);
        }

        let mut map = Self::default();
        let overridden = |ctx: KeyCtx, act: &Act| {
            overrides.iter().any(|(c, acts)| *c == ctx && acts.iter().any(|(a, _)| a == act))
        };
        for entry in &mut map.entries {
            if let Some((_, keys)) = lists.iter().find(|(act, _)| *act == entry.act) {
                if !overridden(entry.ctx, &entry.act) {
                    entry.keys = keys.clone();
                }
            }
        }
        for (ctx, entries) in &overrides {
            for (act, keys) in entries {
                if let Some(entry) = map.entries.iter_mut().find(|e| e.ctx == *ctx && e.act == *act)
                {
                    entry.keys = keys.clone();
                }
            }
        }

        problems.extend(map.conflicts());
        if problems.is_empty() {
            (map, problems)
        } else {
            (Self::default(), problems)
        }
    }

    /// Duplicate keys within one context. Global and a context may share a
    /// key: the context wins by design.
    fn conflicts(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (ctx, _, _) in SECTIONS {
            let mut seen: Vec<(&str, &Act)> = Vec::new();
            for entry in self.entries.iter().filter(|entry| entry.ctx == *ctx) {
                for key in &entry.keys {
                    match seen.iter().find(|(seen_key, _)| *seen_key == key.as_str()) {
                        Some((_, other)) if *other != &entry.act => problems.push(format!(
                            "keys.{}.{}: {} is already bound to {}",
                            ctx.config_name(),
                            entry.act.name().unwrap_or("?"),
                            key,
                            other.name().unwrap_or("?")
                        )),
                        Some(_) => {}
                        None => seen.push((key.as_str(), &entry.act)),
                    }
                }
            }
        }
        problems
    }

    fn global_entry(&self, key_id: &str) -> Option<&MapEntry> {
        self.entries
            .iter()
            .find(|b| b.ctx == KeyCtx::Global && b.keys.iter().any(|key| key == key_id))
    }

    /// Context bindings win over Global ones ('q' goes back in Help).
    pub fn lookup(&self, ctx: KeyCtx, key_id: &str) -> Option<&Act> {
        self.entries
            .iter()
            .find(|b| b.ctx == ctx && b.keys.iter().any(|key| key == key_id))
            .or_else(|| self.global_entry(key_id))
            .map(|b| &b.act)
    }

    /// True when `ctx` itself binds the key, not just Global.
    pub fn is_context_key(&self, ctx: KeyCtx, key_id: &str) -> bool {
        self.entries.iter().any(|b| b.ctx == ctx && b.keys.iter().any(|key| key == key_id))
    }

    /// The first key that runs `act` in `ctx`, for a hint. A Global key the
    /// context has taken over does not count.
    pub fn first_key(&self, ctx: KeyCtx, act: &Act) -> Option<String> {
        self.entries
            .iter()
            .filter(|entry| (entry.ctx == ctx || entry.ctx == KeyCtx::Global) && &entry.act == act)
            .flat_map(|entry| &entry.keys)
            .find(|key| self.lookup(ctx, key) == Some(act))
            .cloned()
    }

    /// The keys an action answers to in one context, for `--list`.
    pub fn bound_keys(&self, ctx: KeyCtx, act: &Act) -> Vec<String> {
        self.entries
            .iter()
            .find(|entry| entry.ctx == ctx && &entry.act == act)
            .map(|entry| entry.keys.clone())
            .unwrap_or_default()
    }

    /// Context keys that also answer a different Global action, leaving out
    /// the ones the defaults already have (`q` in Help).
    pub fn shadows(&self) -> Vec<Shadow> {
        let default = Self::default();
        let mut out = Vec::new();
        for entry in self.entries.iter().filter(|entry| entry.ctx != KeyCtx::Global) {
            for key in &entry.keys {
                if let Some(global) = self.global_entry(key) {
                    let shipped = default.lookup(entry.ctx, key) == Some(&entry.act)
                        && default.global_entry(key).map(|g| &g.act) == Some(&global.act);
                    if global.act != entry.act && !shipped {
                        out.push(Shadow {
                            ctx: entry.ctx,
                            key: key.clone(),
                            act: entry.act,
                            global: global.act,
                        });
                    }
                }
            }
        }
        out
    }

    pub fn help_lines(&self) -> Vec<HelpLine> {
        let mut out = Vec::new();
        for (ctx, title, _) in SECTIONS {
            out.push(HelpLine::Section(title));
            for entry in
                self.entries.iter().filter(|entry| entry.ctx == *ctx && !entry.keys.is_empty())
            {
                out.push(HelpLine::Keys { keys: entry.keys.join(" / "), desc: entry.desc });
            }
        }
        // Sequences and typing-only keys are not BINDINGS rows.
        out.push(HelpLine::Section("Sequences and input"));
        out.push(HelpLine::Keys {
            keys: "g <letter>".into(),
            desc: "jump to the first title starting with that letter",
        });
        out.push(HelpLine::Keys {
            keys: "Up / Down".into(),
            desc: "while searching: recall a previous search",
        });
        out.push(HelpLine::Section("Command line"));
        for (cmd, desc) in COMMANDS {
            out.push(HelpLine::Keys { keys: (*cmd).to_string(), desc });
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    pub ctx: KeyCtx,
    pub key: String,
    pub act: Act,
    pub global: Act,
}

fn is_movement(act: &Act) -> bool {
    matches!(
        act,
        Act::MoveUp
            | Act::MoveDown
            | Act::PageUp
            | Act::PageDown
            | Act::Top
            | Act::Bottom
            | Act::ColumnLeft
            | Act::ColumnRight
    )
}

const NAMED_KEYS: &[&str] = &[
    "Space", "Enter", "Esc", "Tab", "S-Tab", "BSpace", "Up", "Down", "Left", "Right", "PgUp",
    "PgDn", "Home", "End", "Delete", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10",
    "F11", "F12",
];

fn is_single_key(text: &str, allow_space: bool) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if !c.is_control() && (allow_space || c != ' '))
        && chars.next().is_none()
}

/// The key names a `[keys]` entry may use, exactly what [`key_id`] produces
/// (plus the `gg` sequence).
pub fn is_key_name(name: &str) -> bool {
    if name == "gg" || NAMED_KEYS.contains(&name) {
        return true;
    }
    match name.strip_prefix("C-") {
        Some(rest) => is_single_key(rest, true),
        None => is_single_key(name, false),
    }
}

pub enum HelpLine {
    Section(&'static str),
    Keys { keys: String, desc: &'static str },
}

/// Normalize a key event to a registry key id ("" = never matches).
pub fn key_id(key: &KeyEvent) -> String {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            return format!("C-{c}");
        }
    }
    match key.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "S-Tab".into(),
        KeyCode::Backspace => "BSpace".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::F(n) if (1..=12).contains(&n) => format!("F{n}"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_toml(text: &str) -> (Keymap, Vec<String>) {
        let cfg: crate::config::Config = toml::from_str(text).unwrap();
        Keymap::from_table(&cfg.keys)
    }

    #[test]
    fn no_duplicate_keys_per_context() {
        for (ctx, _, _) in SECTIONS {
            for a in BINDINGS.iter().filter(|b| b.ctx == *ctx) {
                for b in BINDINGS.iter().filter(|b| b.ctx == *ctx) {
                    if std::ptr::eq(a, b) {
                        continue;
                    }
                    for k in a.keys {
                        assert!(!b.keys.contains(k), "key {k:?} bound twice in {ctx:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_binding_has_description_and_name() {
        for b in BINDINGS {
            assert!(!b.desc.is_empty(), "binding {}/{:?} has no description", b.ctx as u8, b.keys);
            let name = b.act.name().expect("an action without a name");
            assert_eq!(Act::from_name(name), Some(b.act), "{name}");
        }
    }

    #[test]
    fn key_entries_replace_and_unbind() {
        let (map, problems) = from_toml(
            "[keys.global]\nnext_track = [\">\", \"n\"]\n\n\
             [keys.queue]\nundo = []\nmove_down = [\"y\"]\n\n\
             [keys.lists]\nmove_down = [\"w\"]\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        // an entry replaces the action's keys
        assert_eq!(map.lookup(KeyCtx::Global, ">"), Some(&Act::NextTrack));
        assert_eq!(map.lookup(KeyCtx::Global, "n"), Some(&Act::NextTrack));
        // [] unbinds
        assert_eq!(map.lookup(KeyCtx::Queue, "u"), None);
        // lists reaches every context with the action
        for ctx in [KeyCtx::Browser, KeyCtx::Playlists, KeyCtx::Search, KeyCtx::Help] {
            assert_eq!(map.lookup(ctx, "w"), Some(&Act::MoveDown), "{ctx:?}");
            assert_eq!(map.lookup(ctx, "j"), None, "{ctx:?}");
        }
        // ... but a per-context entry wins
        assert_eq!(map.lookup(KeyCtx::Queue, "y"), Some(&Act::MoveDown));
        assert_eq!(map.lookup(KeyCtx::Queue, "w"), None);
        assert_eq!(map.lookup(KeyCtx::Queue, "j"), None);
    }

    #[test]
    fn bad_keys_table_rejected_whole() {
        let cases = [
            ("[keys.nope]\nnext_track = [\"n\"]\n", "keys.nope.next_track"),
            ("[keys.queue]\nnope = [\"d\"]\n", "keys.queue.nope"),
            ("[keys.browser]\nremove = [\"d\"]\n", "keys.browser.remove"),
            ("[keys.queue]\nremove = [\"Nope\"]\n", "keys.queue.remove"),
            ("[keys.queue]\nundo = [\"d\"]\n", "keys.queue.undo"),
            ("[keys.global]\nquit = [\"C-c\"]\n", "keys.global.quit"),
            ("[keys.lists]\nquit = [\"x\"]\n", "keys.lists.quit"),
        ];
        for (text, path) in cases {
            let (map, problems) = from_toml(text);
            assert_eq!(map, Keymap::default(), "{text}");
            assert!(problems.iter().any(|problem| problem.contains(path)), "{text}: {problems:?}");
        }
    }

    #[test]
    fn help_follows_rebound_keys() {
        let (map, problems) = from_toml("[keys.queue]\nremove = [\"Delete\"]\nundo = []\n");
        assert!(problems.is_empty(), "{problems:?}");
        let lines = map.help_lines();
        let keys = |desc: &str| {
            lines.iter().find_map(|line| match line {
                HelpLine::Keys { keys, desc: d } if *d == desc => Some(keys.clone()),
                _ => None,
            })
        };
        assert_eq!(keys("remove from queue").as_deref(), Some("Delete"));
        assert_eq!(keys("undo a removal (repeat for older ones)"), None);
    }

    #[test]
    fn hints_and_shadows_follow_context() {
        let (map, problems) = from_toml("[keys.queue]\nremove = [\">\"]\n");
        assert!(problems.is_empty(), "{problems:?}");
        // `>` removes in the Queue, so it must not be offered for next track
        assert_eq!(map.first_key(KeyCtx::Queue, &Act::NextTrack), None);
        assert_eq!(map.first_key(KeyCtx::Browser, &Act::NextTrack).as_deref(), Some(">"));
        let shadows = map.shadows();
        assert_eq!(shadows.len(), 1, "{shadows:?}");
        assert_eq!((shadows[0].ctx, shadows[0].global), (KeyCtx::Queue, Act::NextTrack));
        assert!(Keymap::default().shadows().is_empty());
    }

    #[test]
    fn context_specific_beats_global() {
        let map = Keymap::default();
        // 'q' quits globally but goes back in Help
        assert_eq!(map.lookup(KeyCtx::Help, "q"), Some(&Act::Back));
        assert_eq!(map.lookup(KeyCtx::Queue, "q"), Some(&Act::Quit));
        // no Global '/' binding to fall back to
        assert_eq!(map.lookup(KeyCtx::Help, "/"), None);
        assert_eq!(map.lookup(KeyCtx::Search, "/"), Some(&Act::StartSearch));
    }

    #[test]
    fn key_id_normalization() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_id(&ctrl_c), "C-c");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)), "Space");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)), "PgDn");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)), "S-Tab");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)), "Delete");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)), "F1");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)), "F12");
        assert_eq!(key_id(&KeyEvent::new(KeyCode::F(13), KeyModifiers::NONE)), "");
    }
}
