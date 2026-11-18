//! Every setting, described once. `every_setting_is_described` fails when a
//! `Config` field is added or renamed without an entry here.

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    /// A secret: never rendered, only replaced.
    Secret,
    Int {
        min: Option<i64>,
        max: Option<i64>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
    },
    Bool,
    /// One of a fixed set of values.
    Enum {
        options: &'static [&'static str],
    },
    /// One of a set that depends on the machine (cava's audio sources, the
    /// themes), found when the editor opens.
    EnumDynamic,
    /// A list, one item per line.
    List,
    /// Two numbers, `[width, height]`.
    Pair,
    /// Three whole numbers, `[title, artist, album]` weights.
    Triple,
    /// A colour: one of the 16 ANSI colours, or `#rrggbb`.
    Color,
    /// A glyph or a few (a theme's symbols): checked by `theme::set_symbol`.
    Symbol,
}

#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Dotted key: `ui.art_pct`.
    pub key: &'static str,
    /// Friendly name: "Art pane width".
    pub label: &'static str,
    /// One line: what it changes.
    pub help: &'static str,
    pub kind: Kind,
    pub group: &'static str,
    /// A row that does not apply is dimmed, and still editable.
    pub applies: Option<Applies>,
}

/// A setting that only matters while another one holds a value.
#[derive(Debug, Clone, Copy)]
pub struct Applies {
    /// The setting it depends on.
    pub key: &'static str,
    pub when: When,
    /// Shown in the details pane while the rule does not hold.
    pub reason: &'static str,
}

/// What the other setting must be, as `shmc config --list` prints it
/// (`true`/`false` for a yes/no).
#[derive(Debug, Clone, Copy)]
pub enum When {
    Is(&'static str),
    IsNot(&'static str),
}

impl Setting {
    /// Why this setting has no effect right now, or None when it has (or
    /// always does). `current` reads a setting's value by key.
    pub fn unused_because(&self, current: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
        let rule = self.applies?;
        let now = current(rule.key)?;
        let holds = match rule.when {
            When::Is(v) => now == v,
            When::IsNot(v) => now != v,
        };
        (!holds).then_some(rule.reason)
    }
}

pub const GROUPS: &[&str] = &[
    "Plex",
    "Playback",
    "Album art",
    "Visualizer",
    "Interface",
    "Playlists",
    "Theme",
    "Spacing",
    "Symbols",
];

/// The groups the Theme tab lists; every other group is on the Config tab.
pub const THEME_GROUPS: &[&str] = &["Theme", "Spacing", "Symbols"];

pub fn is_theme_group(group: &str) -> bool {
    THEME_GROUPS.contains(&group)
}

/// Grouped for reading, ordered by key inside a group. `shmc config --list`
/// sorts its own rows.
pub const ALL: &[Setting] = &[
    Setting {
        key: "plex.server",
        label: "Plex server",
        help: "Where shmc connects. Plain HTTP sends the token in cleartext.",
        kind: Kind::Text,
        group: "Plex",
        applies: None,
    },
    Setting {
        key: "plex.token",
        label: "Plex token",
        help: "Your Plex token. Never shown, never written to a log.",
        kind: Kind::Secret,
        group: "Plex",
        applies: None,
    },
    Setting {
        key: "plex.music_section",
        label: "Music library",
        help: "Which music library to use, chosen by name from the server. Empty = the first one found.",
        kind: Kind::Text,
        group: "Plex",
        applies: None,
    },
    Setting {
        key: "plex.allow_writes",
        label: "Allow changes",
        help: "Let shmc change things on the server, such as ratings.",
        kind: Kind::Bool,
        group: "Plex",
        applies: None,
    },
    Setting {
        key: "plex.report_playback",
        label: "Report playback",
        help: "Send plays to Plex for play counts, last played and Now Playing.",
        kind: Kind::Bool,
        group: "Plex",
        applies: None,
    },
    Setting {
        key: "mpv.extra_args",
        label: "mpv arguments",
        help: "Extra mpv arguments, one per row; mpv ignores your mpv.conf, and a --log-file is dropped.",
        kind: Kind::List,
        group: "Playback",
        applies: None,
    },
    Setting {
        key: "mpv.path",
        label: "mpv path",
        help: "Where mpv lives: a full path, or a bare name looked up on $PATH.",
        kind: Kind::Text,
        group: "Playback",
        applies: None,
    },
    Setting {
        key: "mpv.volume",
        label: "Start volume",
        help: "The volume mpv starts with, 0-100.",
        kind: Kind::Float {
            min: Some(0.0),
            max: Some(100.0),
        },
        group: "Playback",
        applies: None,
    },
    Setting {
        key: "album_art.backend",
        label: "Art backend",
        help: "How covers are drawn: auto, kitty, iterm2, sixel, halfblocks or none.",
        kind: Kind::Enum {
            options: &["auto", "kitty", "iterm2", "sixel", "halfblocks", "none"],
        },
        group: "Album art",
        applies: None,
    },
    Setting {
        key: "album_art.cache_mb",
        label: "Art cache",
        help: "Cap on the cached artwork, in MB. 0 = do not cache.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Album art",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "album_art.max_size_px",
        label: "Art size",
        help: "Cap on the encoded cover, [width, height]. 0 on an axis = unlimited.",
        kind: Kind::Pair,
        group: "Album art",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "cava.enabled",
        label: "Visualizer",
        help: "Show the visualizer pane. Needs the cava binary.",
        kind: Kind::Bool,
        group: "Visualizer",
        applies: None,
    },
    Setting {
        key: "cava.framerate",
        label: "Visualizer fps",
        help: "Frames per second. Higher is smoother and uses more CPU.",
        kind: Kind::Int {
            min: Some(crate::config::CAVA_FRAMERATE.0 as i64),
            max: Some(crate::config::CAVA_FRAMERATE.1 as i64),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "cava.path",
        label: "cava path",
        help: "Where cava lives: a full path, or a bare name looked up on $PATH.",
        kind: Kind::Text,
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "cava.pause_grace_secs",
        label: "Tap grace",
        help: "Seconds to keep the audio tap after playback pauses. 0 = release at once.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "cava.source",
        label: "Audio source",
        help: "Empty = the platform default; the checks show the fallback order.",
        kind: Kind::EnumDynamic,
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "cava.style",
        label: "Visualizer style",
        help: "How the bars move; custom uses the settings below. Next cava start.",
        kind: Kind::Enum {
            options: &["balanced", "smooth", "snappy", "custom"],
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "cava.noise_reduction",
        label: "Noise reduction",
        help: "Smoothing, 0-100: higher is smoother and slower. Next cava start.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(100),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.style", when: When::Is("custom"), reason: "only used with the custom style" }),
    },
    Setting {
        key: "cava.autosens",
        label: "Auto gain",
        help: "Let cava adjust the gain to the loudness. Next cava start.",
        kind: Kind::Bool,
        group: "Visualizer",
        applies: Some(Applies { key: "cava.style", when: When::Is("custom"), reason: "only used with the custom style" }),
    },
    Setting {
        key: "cava.sensitivity",
        label: "Sensitivity",
        help: "Gain in percent, 1-300; the starting gain with auto gain. Next cava start.",
        kind: Kind::Int {
            min: Some(1),
            max: Some(300),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.style", when: When::Is("custom"), reason: "only used with the custom style" }),
    },
    Setting {
        key: "cava.lower_cutoff_hz",
        label: "Low cutoff",
        help: "Lowest frequency shown, in Hz (10-1000). Next cava start.",
        kind: Kind::Int {
            min: Some(10),
            max: Some(1000),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.style", when: When::Is("custom"), reason: "only used with the custom style" }),
    },
    Setting {
        key: "cava.higher_cutoff_hz",
        label: "High cutoff",
        help: "Highest frequency shown, 1000-20000 Hz, above the low cutoff. Next cava start.",
        kind: Kind::Int {
            min: Some(1000),
            max: Some(20000),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.style", when: When::Is("custom"), reason: "only used with the custom style" }),
    },
    Setting {
        key: "ui.visualizer_bar_gap",
        label: "Bar gap",
        help: "Smallest gap between visualizer bars, in cells.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "ui.visualizer_bar_width",
        label: "Bar width",
        help: "Smallest bar width, in cells. A narrow pane shows fewer bars, not thinner ones.",
        kind: Kind::Int {
            min: Some(1),
            max: Some(10),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "ui.visualizer_bars",
        label: "Bar count",
        help: "Spread mode: the most bars shown; fewer on a narrow pane.",
        kind: Kind::Int {
            min: Some(1),
            max: Some(96),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "ui.visualizer_mode", when: When::Is("spread"), reason: "only used in spread mode" }),
    },
    Setting {
        key: "ui.visualizer_mode",
        label: "Visualizer mode",
        help: "adaptive (as many bars as fit) or spread (up to Bar count).",
        kind: Kind::Enum {
            options: &["adaptive", "spread"],
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "ui.visualizer_pct",
        label: "Visualizer height",
        help: "Height of the visualizer pane, in percent of the queue column.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(100),
        },
        group: "Visualizer",
        applies: Some(Applies { key: "cava.enabled", when: When::Is("true"), reason: "only used while the visualizer is on" }),
    },
    Setting {
        key: "ui.visualizer_anchor",
        label: "Spread anchor",
        help: "Spread mode: bar or gap stays at its set size; the other widens.",
        kind: Kind::Enum {
            options: &["bar", "gap"],
        },
        group: "Visualizer",
        applies: Some(Applies { key: "ui.visualizer_mode", when: When::Is("spread"), reason: "only used in spread mode" }),
    },
    Setting {
        key: "ui.art_metadata",
        label: "Cover metadata",
        help: "Show the title, artist and album under the cover.",
        kind: Kind::Bool,
        group: "Interface",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "ui.art_pad_px",
        label: "Cover padding",
        help: "Padding around the cover, in pixels.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Interface",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "ui.art_pct",
        label: "Art pane width",
        help: "Width of the art pane, in percent of the queue column. 0 = hidden.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(100),
        },
        group: "Interface",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "ui.art_square",
        label: "Square cover",
        help: "Keep the cover square instead of filling the pane.",
        kind: Kind::Bool,
        group: "Interface",
        applies: Some(Applies { key: "album_art.backend", when: When::IsNot("none"), reason: "only used while album art is on" }),
    },
    Setting {
        key: "ui.album_sort",
        label: "Album order",
        help: "Order of the Browser's albums: year, year_desc (newest first) or title.",
        kind: Kind::Enum {
            options: &["year", "year_desc", "title"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.prev_restart_secs",
        label: "Restart after",
        help: "Previous restarts the track after this many seconds, else goes back. 0 = always go back.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(60),
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.auto_follow",
        label: "Auto-follow",
        help: "Keep the playing row pinned to the top of the queue.",
        kind: Kind::Bool,
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.center_cursor",
        label: "Centre cursor",
        help: "Centre the cursor when jumping to the playing track.",
        kind: Kind::Bool,
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.column_weights",
        label: "Column weights",
        help: "Title, Artist, Album: relative widths of the track tables. 0 = hidden.",
        kind: Kind::Triple,
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.queue_columns",
        label: "Queue columns",
        help: "Queue columns in order: title, artist, album, len, track, year, codec.",
        kind: Kind::List,
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.confirm_playlist_over",
        label: "Confirm playlists",
        help: "Ask before queueing a playlist larger than this. 0 = never ask.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.icons",
        label: "Icons",
        help: "nerd (needs a Nerd Font) or ascii.",
        kind: Kind::Enum {
            options: &["nerd", "ascii"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.header",
        label: "Header",
        help: "full or compact (no header; the tab row shows the timer, volume and flags).",
        kind: Kind::Enum {
            options: &["full", "compact"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.volume_display",
        label: "Volume",
        help: "The header's volume: meter, percent (the number) or both.",
        kind: Kind::Enum {
            options: &["meter", "percent", "both"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.rating_style",
        label: "Rating",
        help: "The header's rating: five stars, one star or off.",
        kind: Kind::Enum {
            options: &["five", "one", "off"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.mode_flags",
        label: "Mode flags",
        help: "The header's repeat, shuffle, single and consume flags: icons or words (bright when on).",
        kind: Kind::Enum {
            options: &["icons", "words"],
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.scrolloff",
        label: "Scrolloff",
        help: "Rows kept visible past the cursor when a list scrolls.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Interface",
        applies: None,
    },
    Setting {
        key: "ui.playlist_numbering",
        label: "Row numbers",
        help: "Number the playlist rows: position or none.",
        kind: Kind::Enum {
            options: &["position", "none"],
        },
        group: "Playlists",
        applies: None,
    },
    Setting {
        key: "ui.preview_debounce_ms",
        label: "Preview delay",
        help: "Wait before loading a playlist preview, in ms.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Playlists",
        applies: None,
    },
    Setting {
        key: "ui.preview_tracks",
        label: "Preview tracks",
        help: "How many tracks a playlist preview loads. 0 = no preview.",
        kind: Kind::Int {
            min: Some(0),
            max: None,
        },
        group: "Playlists",
        applies: None,
    },
    Setting {
        key: "theme.name",
        label: "Theme",
        help: "The base theme. The colours, spacing and symbols below override it.",
        kind: Kind::EnumDynamic,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.accent",
        label: "Accent",
        help: "The cursor, the active tab, the filled progress bar, the icons.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.on_accent",
        label: "Text on accent",
        help: "The colour of text drawn on an accent background.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.text",
        label: "Text",
        help: "Normal text.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.dim",
        label: "Dimmed",
        help: "Dimmed content, and the inactive tab.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.secondary",
        label: "Secondary",
        help: "Help keys, the artist in the header, filter matches.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.error",
        label: "Error",
        help: "Errors.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.border",
        label: "Border",
        help: "The outlines of every pane and box.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.highlight",
        label: "Unfocused cursor",
        help: "The cursor of a pane that does not have keyboard focus.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.viz_bottom",
        label: "Visualizer bottom",
        help: "The visualizer ramp, at the bottom of the pane.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.viz_mid",
        label: "Visualizer middle",
        help: "The visualizer ramp, in the middle.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.viz_top",
        label: "Visualizer top",
        help: "The visualizer ramp, at the top of the pane.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.background",
        label: "Background",
        help: "Painted behind everything. none = keep the terminal's background, even on a theme that sets one.",
        kind: Kind::Color,
        group: "Theme",
        applies: None,
    },
    Setting {
        key: "theme.block_gap",
        label: "Block gap",
        help: "Empty rows between the header, the tab bar, the content and the progress bar: 0 to 3.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(3),
        },
        group: "Spacing",
        applies: None,
    },
    Setting {
        key: "theme.borders",
        label: "Borders",
        help: "box, rounded, double, lines (one rule between blocks) or none. Lines and none leave out the frames.",
        kind: Kind::Enum {
            options: &["box", "rounded", "double", "lines", "none"],
        },
        group: "Spacing",
        applies: None,
    },
    Setting {
        key: "theme.inset",
        label: "Inset",
        help: "Columns between a box's border and what is inside it, each side: 0 to 3.",
        kind: Kind::Int {
            min: Some(0),
            max: Some(3),
        },
        group: "Spacing",
        applies: None,
    },
    Setting {
        key: "theme.progress_bar",
        label: "Progress bar",
        help: "solid, line, dot, plain, angled or rounded (the last two need Powerline glyphs). Rows below replace a part.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.progress_filled",
        label: "Progress filled",
        help: "The progress bar's played part. One character, or solid for a plain coloured bar.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.progress_empty",
        label: "Progress empty",
        help: "The progress bar's unplayed part. One character, or solid for a plain coloured bar.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.progress_head",
        label: "Progress head",
        help: "The progress bar's last played cell. One character, or solid for a plain coloured bar.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.progress_start",
        label: "Progress start cap",
        help: "A cap before the progress bar. One character, or solid for none.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.progress_end",
        label: "Progress end cap",
        help: "A cap after the progress bar. One character, or solid for none.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.ellipsis",
        label: "Ellipsis",
        help: "What ends a text that is cut short, one character.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.scroll_up",
        label: "Scroll up",
        help: "The top end of a scrollbar, one character.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.scroll_down",
        label: "Scroll down",
        help: "The bottom end of a scrollbar, one character.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.scroll_thumb",
        label: "Scroll thumb",
        help: "The scrollbar's thumb, one character.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.volume_meter",
        label: "Volume meter",
        help: "A preset (blocks, squares, outline, plain), or 8 characters low to high, or 2: lit then unlit.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
    Setting {
        key: "theme.visualizer",
        label: "Visualizer glyphs",
        help: "A preset (blocks, ascii), or 8 characters from the lowest level to a full cell.",
        kind: Kind::Symbol,
        group: "Symbols",
        applies: None,
    },
];

pub fn find(key: &str) -> Option<&'static Setting> {
    ALL.iter().find(|s| s.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::collections::BTreeSet;

    #[test]
    fn every_setting_is_described() {
        let keys = Config::default().flat_settings();
        let described: BTreeSet<&str> = ALL.iter().map(|s| s.key).collect();
        for key in keys.keys() {
            assert!(described.contains(key.as_str()), "{key} has no description");
        }
        // and the other way round
        for setting in ALL {
            assert!(keys.contains_key(setting.key), "{} is not a setting any more", setting.key);
        }
    }

    #[test]
    fn every_setting_has_label_and_help() {
        for setting in ALL {
            assert!(!setting.label.is_empty(), "{} has no label", setting.key);
            assert!(!setting.help.is_empty(), "{} has no help", setting.key);
            assert!(!setting.help.contains('\n'), "{} help must be one line", setting.key);
            assert!(
                setting.help.chars().count() <= 110,
                "{} help is too long for the pane: {}",
                setting.key,
                setting.help
            );
            assert!(
                GROUPS.contains(&setting.group),
                "{} has an unknown group {}",
                setting.key,
                setting.group
            );
        }
    }

    #[test]
    fn applies_rules_name_real_setting_and_value() {
        let keys = Config::default().flat_settings();
        for setting in ALL {
            let Some(rule) = setting.applies else { continue };
            assert!(keys.contains_key(rule.key), "{}: {} is not a setting", setting.key, rule.key);
            assert_ne!(rule.key, setting.key, "{} depends on itself", setting.key);
            let (When::Is(v) | When::IsNot(v)) = rule.when;
            match find(rule.key).map(|s| s.kind) {
                Some(Kind::Enum { options }) => assert!(
                    options.contains(&v),
                    "{}: {v:?} is not an option of {}",
                    setting.key,
                    rule.key
                ),
                Some(Kind::Bool) => assert!(v == "true" || v == "false", "{}: {v:?}", setting.key),
                _ => {}
            }
            assert!(!rule.reason.is_empty(), "{} has no reason", setting.key);
        }
    }

    #[test]
    fn keys_and_labels_are_unique() {
        let mut keys = BTreeSet::new();
        let mut labels = BTreeSet::new();
        for setting in ALL {
            assert!(keys.insert(setting.key), "{} is listed twice", setting.key);
            assert!(
                labels.insert(setting.label),
                "{} reuses the label {:?}",
                setting.key,
                setting.label
            );
        }
    }

    #[test]
    fn enum_options_match_what_code_accepts() {
        // The code that reads a setting is the authority; an option it
        // accepts must be offered here.
        assert_eq!(options("ui.icons"), Some(&["nerd", "ascii"][..]), "ui.icons");
        assert_eq!(
            options("ui.volume_display"),
            Some(&["meter", "percent", "both"][..]),
            "ui.volume_display"
        );
        assert_eq!(
            options("ui.rating_style"),
            Some(&["five", "one", "off"][..]),
            "ui.rating_style"
        );
        assert_eq!(options("ui.header"), Some(&["full", "compact"][..]), "ui.header");
        assert_eq!(
            options("theme.borders"),
            Some(&["box", "rounded", "double", "lines", "none"][..]),
            "theme.borders"
        );
        assert_eq!(options("ui.mode_flags"), Some(&["icons", "words"][..]), "ui.mode_flags");
        assert_eq!(
            options("ui.visualizer_mode"),
            Some(&["adaptive", "spread"][..]),
            "ui.visualizer_mode"
        );
        assert_eq!(
            options("ui.visualizer_anchor"),
            Some(&["bar", "gap"][..]),
            "ui.visualizer_anchor"
        );
        assert_eq!(
            options("ui.album_sort"),
            Some(&["year", "year_desc", "title"][..]),
            "ui.album_sort"
        );
        assert_eq!(
            options("ui.playlist_numbering"),
            Some(&["position", "none"][..]),
            "ui.playlist_numbering"
        );
        assert_eq!(
            options("album_art.backend"),
            Some(&["auto", "kitty", "iterm2", "sixel", "halfblocks", "none"][..]),
            "album_art.backend"
        );
    }

    #[test]
    fn enum_has_two_options_at_least() {
        for setting in ALL {
            if let Kind::Enum { options } = setting.kind {
                assert!(
                    options.len() >= 2,
                    "{} is an enum with {} option(s)",
                    setting.key,
                    options.len()
                );
            }
        }
    }

    #[test]
    fn int_defaults_are_within_their_range() {
        let values = Config::default().flat_values().unwrap();
        for setting in ALL {
            let Kind::Int { min, max } = setting.kind else { continue };
            let Some(n) = values.get(setting.key).and_then(|v| v.as_integer()) else { continue };
            assert!(min.is_none_or(|m| n >= m), "{} default {n} is below its minimum", setting.key);
            assert!(max.is_none_or(|m| n <= m), "{} default {n} is above its maximum", setting.key);
        }
    }

    fn options(key: &str) -> Option<&'static [&'static str]> {
        match find(key)?.kind {
            Kind::Enum { options } => Some(options),
            _ => None,
        }
    }
}
