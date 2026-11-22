//! The config schema and its defaults.

mod file;
mod paths;
mod settings;

pub use file::retired_keys;
pub(crate) use file::scrub_value;
pub use paths::{
    cache_dir, config_path, log_dir, queue_path, state_dir, themes_dir_for, usable_log_dir, NO_HOME,
};
pub use settings::{SettingChange, SettingRow, NO_DEFAULT};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The raw `[keys]` table; `ui::keymap` validates and applies it.
pub type KeyTable = BTreeMap<String, BTreeMap<String, Vec<String>>>;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub plex: PlexConfig,
    pub keys: KeyTable,
    pub mpv: MpvConfig,
    pub album_art: AlbumArtConfig,
    pub cava: CavaConfig,
    pub ui: UiConfig,
    pub theme: ThemeConfig,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlexConfig {
    /// Base URL of the Plex Media Server, e.g. "http://192.168.1.100:32400"
    pub server: String,
    /// X-Plex-Token with access to the server
    pub token: String,
    /// Which Plex music library section to use. Empty = auto (first artist section)
    pub music_section: String,
    /// Let shmc change things on the server: ratings with `r` and radio with `s`.
    pub allow_writes: bool,
    /// Send plays to Plex for play counts, last played and Now Playing.
    pub report_playback: bool,
}

impl std::fmt::Debug for PlexConfig {
    /// The token never appears in a debug dump.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexConfig")
            .field("server", &crate::redact::redact_url(&self.server))
            .field("token", &token_label(&self.token))
            .field("music_section", &self.music_section)
            .field("allow_writes", &self.allow_writes)
            .field("report_playback", &self.report_playback)
            .finish()
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("plex", &self.plex)
            .field("mpv", &self.mpv)
            .field("album_art", &self.album_art)
            .field("cava", &self.cava)
            .field("ui", &self.ui)
            .finish()
    }
}

/// Never any of the token's characters: this text is pasted into bug reports.
pub(crate) fn token_label(token: &str) -> &'static str {
    if token.is_empty() {
        "(missing)"
    } else {
        "(set)"
    }
}

/// What the header shows for the volume (`ui.volume_display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeDisplay {
    Meter,
    Percent,
    Both,
}

/// What the header shows for the playing track's rating (`ui.rating_style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingStyle {
    Five,
    One,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// vim's scrolloff, clamped to (pane_height - 1) / 2.
    pub scrolloff: usize,
    /// Centre the cursor on `o` in Browser and Playlists.
    pub center_cursor: bool,
    /// Keep the Queue pinned to the playing row as playback moves.
    pub auto_follow: bool,
    pub restore_queue: bool,
    /// "nerd" or "ascii"; ASCII when the locale is not UTF-8.
    pub icons: String,
    /// "meter" or "percent".
    pub volume_display: String,
    /// "icons" or "words".
    pub mode_flags: String,
    /// "five", "one" or "off".
    pub rating_style: String,
    /// "full" or "compact".
    pub header: String,
    /// Art width in percent: the cap when square, the split otherwise.
    pub art_pct: u16,
    /// Keep the art pane pixel-square.
    pub art_square: bool,
    /// Show title/artist/album below a width-capped square cover.
    pub art_metadata: bool,
    /// Padding on the art's limiting axis, in pixels.
    pub art_pad_px: u16,
    /// Visualizer height in percent of the queue column (0 = hidden).
    pub visualizer_pct: u16,
    /// "adaptive" (as many bars as fit) or "spread" (up to `visualizer_bars`).
    pub visualizer_mode: String,
    /// Most bars spread mode shows.
    pub visualizer_bars: u16,
    /// Spread mode: "bar" or "gap", whichever keeps its set size.
    pub visualizer_anchor: String,
    /// Minimum bar width in cells.
    pub visualizer_bar_width: u16,
    /// Minimum gap between bars in cells.
    pub visualizer_bar_gap: u16,
    /// "year", "year_desc" or "title".
    pub album_sort: String,
    /// After this many seconds `<` restarts the track (0 = always go back).
    pub prev_restart_secs: u64,
    /// Playlist numbering: "position" (1..N) or "none".
    pub playlist_numbering: String,
    /// Playlists tab: tracks fetched for the hover preview (0 = preview off).
    pub preview_tracks: usize,
    /// Debounce for the preview after moving the playlist cursor, in ms.
    pub preview_debounce_ms: u64,
    /// Ask before queueing a playlist with more tracks than this (0 = never).
    pub confirm_playlist_over: usize,
    /// Title, Artist and Album widths; 0 hides a column, but not all three.
    pub column_weights: Vec<i64>,
    pub queue_columns: Vec<String>,
}

pub const DEFAULT_COLUMN_WEIGHTS: [i64; 3] = [30, 24, 46];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueColumn {
    Title,
    Artist,
    Album,
    Len,
    Track,
    Year,
    Codec,
}

impl QueueColumn {
    pub const DEFAULT: [QueueColumn; 4] =
        [QueueColumn::Title, QueueColumn::Artist, QueueColumn::Album, QueueColumn::Len];

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "len" => Self::Len,
            "track" => Self::Track,
            "year" => Self::Year,
            "codec" => Self::Codec,
            _ => return None,
        })
    }

    /// Index into the Title, Artist, Album weights.
    pub fn text_index(self) -> Option<usize> {
        match self {
            Self::Title => Some(0),
            Self::Artist => Some(1),
            Self::Album => Some(2),
            _ => None,
        }
    }
}

pub fn default_queue_columns() -> Vec<String> {
    ["title", "artist", "album", "len"].map(String::from).to_vec()
}

/// What is wrong with a `ui.queue_columns` value, if anything (keys, not values).
pub fn queue_columns_problem(cols: &[String]) -> Option<String> {
    if cols.is_empty() {
        return Some("ui.queue_columns is empty".into());
    }
    let mut seen = Vec::new();
    for name in cols {
        let Some(col) = QueueColumn::parse(name) else {
            return Some("ui.queue_columns has an unknown column name".into());
        };
        if seen.contains(&col) {
            return Some("ui.queue_columns lists a column twice".into());
        }
        seen.push(col);
    }
    if !seen.iter().any(|c| c.text_index().is_some()) {
        return Some("ui.queue_columns needs title, artist or album".into());
    }
    None
}

/// Keeps the arithmetic far from overflowing.
const COLUMN_WEIGHT_MAX: i64 = 10_000;

/// What is wrong with a `ui.column_weights` value, if anything (keys, not values).
pub fn column_weights_problem(weights: &[i64]) -> Option<String> {
    if weights.len() != 3 {
        return Some("ui.column_weights needs three numbers: Title, Artist, Album".into());
    }
    if weights.iter().any(|w| !(0..=COLUMN_WEIGHT_MAX).contains(w)) {
        return Some(format!("ui.column_weights numbers must be 0-{COLUMN_WEIGHT_MAX}"));
    }
    if weights.iter().all(|w| *w == 0) {
        return Some("ui.column_weights needs at least one column above 0".into());
    }
    None
}

impl UiConfig {
    pub fn ascii_icons(&self) -> bool {
        self.icons.eq_ignore_ascii_case("ascii") || !locale_is_utf8()
    }

    /// Anything but "meter" or "percent" is both.
    pub fn volume_mode(&self) -> VolumeDisplay {
        if self.volume_display.eq_ignore_ascii_case("meter") {
            VolumeDisplay::Meter
        } else if self.volume_display.eq_ignore_ascii_case("percent") {
            VolumeDisplay::Percent
        } else {
            VolumeDisplay::Both
        }
    }

    /// Anything but "one" or "off" is five.
    pub fn rating_mode(&self) -> RatingStyle {
        if self.rating_style.eq_ignore_ascii_case("one") {
            RatingStyle::One
        } else if self.rating_style.eq_ignore_ascii_case("off") {
            RatingStyle::Off
        } else {
            RatingStyle::Five
        }
    }

    pub fn compact_header(&self) -> bool {
        self.header.eq_ignore_ascii_case("compact")
    }

    pub fn mode_words(&self) -> bool {
        self.mode_flags.eq_ignore_ascii_case("words")
    }

    /// Problems for `shmc config --check`: keys only, never values.
    pub fn enum_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !["bar", "gap"].contains(&self.visualizer_anchor.as_str()) {
            out.push("ui.visualizer_anchor is not bar or gap; using gap".to_string());
        }
        if !["full", "compact"].iter().any(|v| self.header.eq_ignore_ascii_case(v)) {
            out.push("ui.header is not full or compact; using full".to_string());
        }
        out
    }

    /// The configured weights, or the default when they are not valid.
    pub fn effective_column_weights(&self) -> [u32; 3] {
        let w = if column_weights_problem(&self.column_weights).is_none() {
            &self.column_weights[..]
        } else {
            &DEFAULT_COLUMN_WEIGHTS[..]
        };
        [w[0] as u32, w[1] as u32, w[2] as u32]
    }

    /// The configured columns, or the default when the list is not valid.
    pub fn effective_queue_columns(&self) -> Vec<QueueColumn> {
        if queue_columns_problem(&self.queue_columns).is_some() {
            return QueueColumn::DEFAULT.to_vec();
        }
        self.queue_columns.iter().filter_map(|n| QueueColumn::parse(n)).collect()
    }

    /// Whether the effective columns show no text column at the effective weights.
    pub fn queue_columns_all_hidden(&self) -> bool {
        let w = self.effective_column_weights();
        !self.effective_queue_columns().iter().any(|c| c.text_index().is_some_and(|i| w[i] > 0))
    }

    /// Zero-weight text columns hidden; the default list if none would be left.
    pub fn resolved_queue_columns(&self) -> Vec<QueueColumn> {
        let w = self.effective_column_weights();
        let mut cols = self.effective_queue_columns();
        if self.queue_columns_all_hidden() {
            cols = QueueColumn::DEFAULT.to_vec();
        }
        cols.retain(|c| c.text_index().is_none_or(|i| w[i] > 0));
        cols
    }

    pub fn numbers_playlists(&self) -> bool {
        !self.playlist_numbering.eq_ignore_ascii_case("none")
    }
}

fn locale_is_utf8() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .map(|v| {
            let up = v.to_ascii_uppercase();
            up.contains("UTF-8") || up.contains("UTF8")
        })
        .unwrap_or(true)
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            scrolloff: 4,
            center_cursor: true,
            auto_follow: true,
            restore_queue: true,
            icons: "nerd".into(),
            volume_display: "both".into(),
            rating_style: "five".into(),
            mode_flags: "icons".into(),
            header: "full".into(),
            art_pct: 40,
            art_square: true,
            art_metadata: false,
            art_pad_px: 1,
            visualizer_pct: 50,
            visualizer_mode: "adaptive".into(),
            visualizer_bars: 20,
            visualizer_anchor: "gap".into(),
            visualizer_bar_width: 3,
            visualizer_bar_gap: 1,
            album_sort: "year".into(),
            prev_restart_secs: 3,
            playlist_numbering: "position".into(),
            preview_tracks: 500,
            preview_debounce_ms: 150,
            confirm_playlist_over: 1000,
            column_weights: DEFAULT_COLUMN_WEIGHTS.to_vec(),
            queue_columns: default_queue_columns(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MpvConfig {
    pub path: String,
    /// Extra args appended to mpv's base argument list
    pub extra_args: Vec<String>,
    /// Startup volume (0-100)
    pub volume: f64,
    pub loudness_leveling: String,
    pub leveling_preamp: f64,
}

pub const LEVELING_PREAMP_DB: (f64, f64) = (-15.0, 15.0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlbumArtConfig {
    /// auto | kitty | iterm2 | sixel | halfblocks | none
    pub backend: String,
    /// Maximum art size in pixels [width, height]; [0, 0] = no limit
    pub max_size_px: [u32; 2],
    /// Art cache size in MiB, least recently used evicted first (0 = no cache).
    pub cache_mb: u64,
}

impl Default for MpvConfig {
    fn default() -> Self {
        Self {
            path: "mpv".into(),
            extra_args: Vec::new(),
            volume: 100.0,
            loudness_leveling: "off".into(),
            leveling_preamp: 0.0,
        }
    }
}

impl MpvConfig {
    pub fn leveling(&self) -> crate::playback::Leveling {
        crate::playback::Leveling::parse(&self.loudness_leveling).unwrap_or_default()
    }

    pub fn preamp_db(&self) -> f64 {
        let (lo, hi) = LEVELING_PREAMP_DB;
        if (lo..=hi).contains(&self.leveling_preamp) {
            self.leveling_preamp
        } else {
            0.0
        }
    }

    /// Problems for `shmc config --check`: keys only, never values.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if crate::playback::Leveling::parse(&self.loudness_leveling).is_none() {
            out.push("mpv.loudness_leveling is not off, track or album; using off".to_string());
        }
        if self.preamp_db() != self.leveling_preamp {
            let (lo, hi) = LEVELING_PREAMP_DB;
            out.push(format!("mpv.leveling_preamp must be {lo}-{hi}; using 0"));
        }
        out
    }
}

impl Default for AlbumArtConfig {
    fn default() -> Self {
        Self { backend: "auto".into(), max_size_px: [0, 0], cache_mb: 200 }
    }
}

/// A theme plus overrides; an unset or bad override is the theme's value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
    /// `themes/` beside the config file, `None` for presets only; not a setting.
    #[serde(skip)]
    pub dir: Option<PathBuf>,
    /// Interaction: the cursor, the active tab, the filled progress bar, icons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Text drawn on an accent background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_accent: Option<String>,
    /// Default content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Dimmed content and the inactive tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<String>,
    /// Emphasis: Help keys, the header artist, filter matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Pane and box outlines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    /// The cursor of a pane that does not have keyboard focus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    /// The visualizer ramp: bottom, middle, top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viz_bottom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viz_mid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viz_top: Option<String>,
    /// `#rrggbb`, or `none` for the terminal's own background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Empty rows between the header, tabs, content and progress row: 0 to 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_gap: Option<i64>,
    /// Columns inside a box's border on each side: 0 to 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inset: Option<i64>,
    /// box, rounded, double, lines or none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<String>,
    /// A preset for the five glyphs below; any of them set too replaces its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_bar: Option<String>,
    /// The progress bar's filled and empty cells and its head, one character each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_filled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_empty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_head: Option<String>,
    /// The bar's end caps, one character each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_end: Option<String>,
    /// The end of a text that is cut short, one character.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ellipsis: Option<String>,
    /// The scrollbar's ends and thumb, one character each.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll_thumb: Option<String>,
    /// The volume meter: 8 characters (a level ramp) or 2 (lit, unlit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_meter: Option<String>,
    /// The visualizer's bar tops: 8 characters, lowest level first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visualizer: Option<String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: "default".into(),
            dir: None,
            accent: None,
            on_accent: None,
            text: None,
            dim: None,
            secondary: None,
            error: None,
            border: None,
            highlight: None,
            viz_bottom: None,
            viz_mid: None,
            viz_top: None,
            background: None,
            block_gap: None,
            inset: None,
            borders: None,
            progress_bar: None,
            progress_filled: None,
            progress_empty: None,
            progress_head: None,
            progress_start: None,
            progress_end: None,
            ellipsis: None,
            scroll_up: None,
            scroll_down: None,
            scroll_thumb: None,
            volume_meter: None,
            visualizer: None,
        }
    }
}

impl ThemeConfig {
    pub const ROLES: [&'static str; 12] = [
        "accent",
        "on_accent",
        "text",
        "dim",
        "secondary",
        "error",
        "border",
        "highlight",
        "viz_bottom",
        "viz_mid",
        "viz_top",
        "background",
    ];

    pub const SPACING: [&'static str; 2] = ["block_gap", "inset"];

    /// Spacing keys that are words, not numbers.
    pub const SPACING_TEXT: [&'static str; 1] = ["borders"];

    /// `progress_bar` first: a preset applies before the glyphs that replace it.
    pub const SYMBOLS: [&'static str; 12] = [
        "progress_bar",
        "progress_filled",
        "progress_empty",
        "progress_head",
        "progress_start",
        "progress_end",
        "ellipsis",
        "scroll_up",
        "scroll_down",
        "scroll_thumb",
        "volume_meter",
        "visualizer",
    ];

    pub fn is_look_key(key: &str) -> bool {
        Self::SPACING.contains(&key)
            || Self::SPACING_TEXT.contains(&key)
            || Self::SYMBOLS.contains(&key)
    }

    pub fn colour(&self, role: &str) -> Option<&str> {
        match role {
            "accent" => self.accent.as_deref(),
            "on_accent" => self.on_accent.as_deref(),
            "text" => self.text.as_deref(),
            "dim" => self.dim.as_deref(),
            "secondary" => self.secondary.as_deref(),
            "error" => self.error.as_deref(),
            "border" => self.border.as_deref(),
            "highlight" => self.highlight.as_deref(),
            "viz_bottom" => self.viz_bottom.as_deref(),
            "viz_mid" => self.viz_mid.as_deref(),
            "viz_top" => self.viz_top.as_deref(),
            "background" => self.background.as_deref(),
            _ => None,
        }
    }

    pub fn spacing(&self, key: &str) -> Option<i64> {
        match key {
            "block_gap" => self.block_gap,
            "inset" => self.inset,
            _ => None,
        }
    }

    pub fn symbol(&self, key: &str) -> Option<&str> {
        match key {
            "borders" => self.borders.as_deref(),
            "progress_bar" => self.progress_bar.as_deref(),
            "progress_filled" => self.progress_filled.as_deref(),
            "progress_empty" => self.progress_empty.as_deref(),
            "progress_head" => self.progress_head.as_deref(),
            "progress_start" => self.progress_start.as_deref(),
            "progress_end" => self.progress_end.as_deref(),
            "ellipsis" => self.ellipsis.as_deref(),
            "scroll_up" => self.scroll_up.as_deref(),
            "scroll_down" => self.scroll_down.as_deref(),
            "scroll_thumb" => self.scroll_thumb.as_deref(),
            "volume_meter" => self.volume_meter.as_deref(),
            "visualizer" => self.visualizer.as_deref(),
            _ => None,
        }
    }

    pub fn overridden(&self, key: &str) -> bool {
        self.colour(key).is_some() || self.spacing(key).is_some() || self.symbol(key).is_some()
    }

    pub fn override_count(&self) -> usize {
        Self::ROLES
            .iter()
            .chain(&Self::SPACING)
            .chain(&Self::SPACING_TEXT)
            .chain(&Self::SYMBOLS)
            .filter(|key| self.overridden(key))
            .count()
    }

    /// False when there was no override (or no such key).
    pub fn clear_look(&mut self, key: &str) -> bool {
        match key {
            "block_gap" => self.block_gap.take().is_some(),
            "inset" => self.inset.take().is_some(),
            "borders" => self.borders.take().is_some(),
            "progress_bar" => self.progress_bar.take().is_some(),
            "progress_filled" => self.progress_filled.take().is_some(),
            "progress_empty" => self.progress_empty.take().is_some(),
            "progress_head" => self.progress_head.take().is_some(),
            "progress_start" => self.progress_start.take().is_some(),
            "progress_end" => self.progress_end.take().is_some(),
            "ellipsis" => self.ellipsis.take().is_some(),
            "scroll_up" => self.scroll_up.take().is_some(),
            "scroll_down" => self.scroll_down.take().is_some(),
            "scroll_thumb" => self.scroll_thumb.take().is_some(),
            "volume_meter" => self.volume_meter.take().is_some(),
            "visualizer" => self.visualizer.take().is_some(),
            _ => false,
        }
    }

    pub fn slot(&mut self, role: &str) -> Option<&mut Option<String>> {
        match role {
            "accent" => Some(&mut self.accent),
            "on_accent" => Some(&mut self.on_accent),
            "text" => Some(&mut self.text),
            "dim" => Some(&mut self.dim),
            "secondary" => Some(&mut self.secondary),
            "error" => Some(&mut self.error),
            "border" => Some(&mut self.border),
            "highlight" => Some(&mut self.highlight),
            "viz_bottom" => Some(&mut self.viz_bottom),
            "viz_mid" => Some(&mut self.viz_mid),
            "viz_top" => Some(&mut self.viz_top),
            "background" => Some(&mut self.background),
            _ => None,
        }
    }

    pub fn clear_overrides(&mut self) {
        let name = std::mem::take(&mut self.name);
        let dir = self.dir.take();
        *self = Self { name, dir, ..Self::default() };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CavaConfig {
    /// Enable the visualizer (requires the `cava` binary)
    pub enabled: bool,
    pub path: String,
    /// Visualizer frames per second.
    pub framerate: u32,
    /// Audio source; empty = the platform default.
    pub source: String,
    /// Seconds cava keeps capturing when paused or hidden (0 = stop at once).
    pub pause_grace_secs: u64,
    /// "balanced", "smooth", "snappy" or "custom", which reads the five below.
    pub style: String,
    /// cava `smoothing:noise_reduction`.
    pub noise_reduction: i64,
    /// cava `general:autosens`.
    pub autosens: bool,
    /// cava `general:sensitivity`.
    pub sensitivity: i64,
    /// cava `general:lower_cutoff_freq`.
    pub lower_cutoff_hz: i64,
    /// cava `general:higher_cutoff_freq`, above the lower one.
    pub higher_cutoff_hz: i64,
}

/// Allowed ranges of the raw visualizer numbers, as (min, max).
pub const CAVA_NOISE_REDUCTION: (i64, i64) = (0, 100);
pub const CAVA_SENSITIVITY: (i64, i64) = (1, 300);
pub const CAVA_LOWER_CUTOFF_HZ: (i64, i64) = (10, 1000);
pub const CAVA_HIGHER_CUTOFF_HZ: (i64, i64) = (1000, 20000);
pub const CAVA_FRAMERATE: (u32, u32) = (1, 120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CavaSettings {
    pub noise_reduction: i64,
    pub autosens: bool,
    pub sensitivity: i64,
    pub lower_cutoff_hz: i64,
    pub higher_cutoff_hz: i64,
}

impl CavaConfig {
    pub fn framerate(&self) -> u32 {
        self.framerate.clamp(CAVA_FRAMERATE.0, CAVA_FRAMERATE.1)
    }

    /// A preset sets the smoothing, `custom` the raw keys; out-of-range is the default.
    pub fn settings(&self) -> CavaSettings {
        let d = CavaConfig::default();
        let in_range = |v: i64, (lo, hi): (i64, i64), default: i64| {
            if (lo..=hi).contains(&v) {
                v
            } else {
                default
            }
        };
        let custom = self.style == "custom";
        if !custom {
            return CavaSettings {
                noise_reduction: crate::playback::cava::preset_noise_reduction(&self.style),
                autosens: d.autosens,
                sensitivity: d.sensitivity,
                lower_cutoff_hz: d.lower_cutoff_hz,
                higher_cutoff_hz: d.higher_cutoff_hz,
            };
        }
        let mut s = CavaSettings {
            noise_reduction: in_range(
                self.noise_reduction,
                CAVA_NOISE_REDUCTION,
                d.noise_reduction,
            ),
            autosens: self.autosens,
            sensitivity: in_range(self.sensitivity, CAVA_SENSITIVITY, d.sensitivity),
            lower_cutoff_hz: in_range(
                self.lower_cutoff_hz,
                CAVA_LOWER_CUTOFF_HZ,
                d.lower_cutoff_hz,
            ),
            higher_cutoff_hz: in_range(
                self.higher_cutoff_hz,
                CAVA_HIGHER_CUTOFF_HZ,
                d.higher_cutoff_hz,
            ),
        };
        if s.lower_cutoff_hz >= s.higher_cutoff_hz {
            s.lower_cutoff_hz = d.lower_cutoff_hz;
            s.higher_cutoff_hz = d.higher_cutoff_hz;
        }
        s
    }

    /// Problems for `shmc config --check`: keys only, never values.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.framerate() != self.framerate {
            let (lo, hi) = CAVA_FRAMERATE;
            out.push(format!("cava.framerate must be {lo}-{hi}; using {}", self.framerate()));
        }
        if !["balanced", "smooth", "snappy", "custom"].contains(&self.style.as_str()) {
            out.push(
                "cava.style is not balanced, smooth, snappy or custom; using balanced".to_string(),
            );
            return out;
        }
        if self.style != "custom" {
            return out;
        }
        for (key, v, (lo, hi)) in [
            ("cava.noise_reduction", self.noise_reduction, CAVA_NOISE_REDUCTION),
            ("cava.sensitivity", self.sensitivity, CAVA_SENSITIVITY),
            ("cava.lower_cutoff_hz", self.lower_cutoff_hz, CAVA_LOWER_CUTOFF_HZ),
            ("cava.higher_cutoff_hz", self.higher_cutoff_hz, CAVA_HIGHER_CUTOFF_HZ),
        ] {
            if !(lo..=hi).contains(&v) {
                out.push(format!("{key} must be {lo}-{hi}; using the default"));
            }
        }
        // A substituted default never clashes with the other one.
        let both_valid = (CAVA_LOWER_CUTOFF_HZ.0..=CAVA_LOWER_CUTOFF_HZ.1)
            .contains(&self.lower_cutoff_hz)
            && (CAVA_HIGHER_CUTOFF_HZ.0..=CAVA_HIGHER_CUTOFF_HZ.1).contains(&self.higher_cutoff_hz);
        if both_valid && self.lower_cutoff_hz >= self.higher_cutoff_hz {
            out.push(
                "cava.lower_cutoff_hz must be below cava.higher_cutoff_hz; using the defaults"
                    .to_string(),
            );
        }
        out
    }
}

impl Default for CavaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "cava".into(),
            framerate: 60,
            source: String::new(),
            pause_grace_secs: 30,
            style: "balanced".into(),
            noise_reduction: 77,
            autosens: true,
            sensitivity: 100,
            lower_cutoff_hz: 50,
            higher_cutoff_hz: 10000,
        }
    }
}

pub fn server_is_https(server: &str) -> bool {
    server.trim().to_ascii_lowercase().starts_with("https://")
}

/// Only a warning: plain HTTP on a LAN is legitimate.
pub fn plain_http_warning(server: &str) -> Option<String> {
    let server = server.trim();
    if server.is_empty() || server_is_https(server) {
        return None;
    }
    Some(
        "plex.server is not https://: the token travels in cleartext; outside your own network, use https:// or a VPN."
            .to_string(),
    )
}

impl Config {
    pub fn needs_setup(&self) -> bool {
        self.plex.server.trim().is_empty() || self.plex.token.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_queue_columns_named_and_default() {
        let list = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for bad in [&["title", "nope"][..], &["title", "title"], &[], &["len", "year"]] {
            let problem = queue_columns_problem(&list(bad)).expect("a problem");
            assert!(problem.contains("ui.queue_columns"), "{bad:?}");
            let ui = UiConfig { queue_columns: list(bad), ..UiConfig::default() };
            assert_eq!(ui.effective_queue_columns(), QueueColumn::DEFAULT, "{bad:?}");
        }
        assert!(queue_columns_problem(&default_queue_columns()).is_none());
    }

    #[test]
    fn zero_weight_hides_text_column() {
        let ui = UiConfig {
            queue_columns: ["album", "track", "title", "len"].map(String::from).to_vec(),
            column_weights: vec![30, 24, 0],
            ..UiConfig::default()
        };
        use QueueColumn::*;
        assert_eq!(ui.resolved_queue_columns(), [Track, Title, Len]);
        let ui = UiConfig { queue_columns: ["album", "len"].map(String::from).to_vec(), ..ui };
        assert!(ui.queue_columns_all_hidden());
        assert_eq!(ui.resolved_queue_columns(), [Title, Artist, Len]);
    }

    #[test]
    fn bad_column_weights_named_and_default() {
        let with = |w: &[i64]| UiConfig { column_weights: w.to_vec(), ..UiConfig::default() };
        assert_eq!(with(&[10, 0, 5]).effective_column_weights(), [10, 0, 5]);
        for bad in [&[30, 24][..], &[1, 2, 3, 4], &[-1, 5, 5], &[0, 0, 0], &[1, 2, 99_999]] {
            assert_eq!(with(bad).effective_column_weights(), [30, 24, 46], "{bad:?}");
            assert!(column_weights_problem(bad).is_some(), "{bad:?}");
        }
        assert!(column_weights_problem(&DEFAULT_COLUMN_WEIGHTS).is_none());
    }

    #[test]
    fn needs_setup_until_both_are_present() {
        let mut c = Config::default();
        assert!(c.needs_setup());
        c.plex.server = "http://host:32400".into();
        assert!(c.needs_setup());
        c.plex.token = "abc".into();
        assert!(!c.needs_setup());
        c.plex.token = "   ".into();
        assert!(c.needs_setup());
    }

    #[test]
    fn server_must_be_https_to_be_encrypted() {
        assert!(server_is_https("https://plex.example:32400"));
        assert!(plain_http_warning("https://plex.example:32400").is_none());
        assert!(!server_is_https("http://192.168.1.100:32400"));
        assert!(plain_http_warning("http://192.168.1.100:32400").is_some());
        assert!(!server_is_https("192.168.1.100:32400"));
        assert!(!server_is_https(""));
        assert!(plain_http_warning("").is_none());
    }

    #[test]
    fn missing_fields_fall_back_to_current_defaults() {
        // Missing keys get their defaults, not zero or off.
        let cfg: Config = toml::from_str(
            "[plex]\nserver = \"http://host:32400\"\ntoken = \"t\"\n\
             [album_art]\nbackend = \"iterm2\"\n",
        )
        .unwrap();
        assert_eq!(cfg.album_art.backend, "iterm2");
        assert_eq!(cfg.album_art.cache_mb, 200);
        assert_eq!(cfg.cava.source, "");
        assert_eq!(cfg.cava.framerate, 60);
        assert!(cfg.ui.auto_follow, "missing auto_follow defaults to on");
        assert_eq!(cfg.ui.prev_restart_secs, 3);
        assert_eq!(cfg.ui.album_sort, "year");
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "s3cr3t-token-value".into(),
                ..PlexConfig::default()
            },
            ..Config::default()
        };
        let text = format!("{cfg:?}");
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(text.contains("(set)"), "{text}");
    }
}
