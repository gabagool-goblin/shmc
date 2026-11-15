use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Let `r` change a track's rating on the server.
    pub set_ratings: bool,
}

impl std::fmt::Debug for PlexConfig {
    /// The token never appears in a debug dump.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexConfig")
            .field("server", &crate::redact::redact_url(&self.server))
            .field("token", &token_label(&self.token))
            .field("music_section", &self.music_section)
            .field("set_ratings", &self.set_ratings)
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
    /// Rows kept clear above/below the cursor before a list scrolls
    /// (vim's scrolloff). Clamped to (pane_height - 1) / 2 for short panes.
    pub scrolloff: usize,
    /// Vertically center the cursor on 'o' in Browser/Playlists; the Queue
    /// pins the playing row to the top instead.
    pub center_cursor: bool,
    /// Keep the Queue pinned to the currently playing row as playback moves.
    /// `o` still jumps once when disabled, but it does not stay armed.
    pub auto_follow: bool,
    /// Status/transport glyph set: "nerd" (Nerd Font) or "ascii".
    /// Forced to ASCII automatically when the locale is not UTF-8.
    pub icons: String,
    /// The header's volume: "meter" (the meter and the percentage) or
    /// "percent" (the percentage only).
    pub volume_display: String,
    /// The header's repeat/shuffle/single/consume flags: "icons" or "words"
    /// (Repeat Shuffle Single Consume, bright when on, dim when off).
    pub mode_flags: String,
    /// The header's rating for the playing track: "five" (five stars), "one"
    /// (a single star) or "off".
    pub rating_style: String,
    /// "full" (the now-playing header) or "compact" (the tab row carries the
    /// sleep timer, volume and flags).
    pub header: String,
    /// Queue tab: maximum art width percentage in square mode; the width
    /// split when `art_square` is false. The rest is the queue/visualizer.
    pub art_pct: u16,
    /// Keep the queue art pane pixel-square, using any spare vertical space
    /// for title/artist/album. When false, `art_pct` controls the width split.
    pub art_square: bool,
    /// Show title/artist/album below a width-capped square cover.
    pub art_metadata: bool,
    /// Padding on the limiting axis of the album art, in pixels (left/right
    /// for width-limited covers, top/bottom for height-limited covers).
    pub art_pad_px: u16,
    /// Visualizer pane height as a percentage of the queue column, at least 8
    /// rows (0 hides it). The queue always keeps at least 6 rows.
    pub visualizer_pct: u16,
    /// "adaptive" (as many bars as fit, then wider ones) or "spread" (up to
    /// `visualizer_bars`, then wider bars or gaps per `visualizer_anchor`).
    pub visualizer_mode: String,
    /// Most bars spread mode shows.
    pub visualizer_bars: u16,
    /// Spread mode: which keeps its set size, "bar" (width) or "gap"; the other
    /// widens.
    pub visualizer_anchor: String,
    /// Visualizer bar width in cells; a minimum (see `visualizer_mode`).
    pub visualizer_bar_width: u16,
    /// Gap between visualizer bars in cells; a minimum, as the bar width.
    pub visualizer_bar_gap: u16,
    /// Browser album order: "year" (oldest first), "year_desc" (newest first)
    /// or "title" (A-Z). Albums without a year go last in both year orders.
    pub album_sort: String,
    /// `<` restarts the current track once it has played for more than this
    /// many seconds, instead of going back (0 = always go back).
    pub prev_restart_secs: u64,
    /// Playlist numbering: "position" (1..N) or "none".
    pub playlist_numbering: String,
    /// Playlists tab: tracks fetched for the hover preview (0 = preview off).
    pub preview_tracks: usize,
    /// Debounce for the preview after moving the playlist cursor, in ms.
    pub preview_debounce_ms: u64,
    /// Ask before queueing a playlist with more tracks than this (0 = never).
    pub confirm_playlist_over: usize,
    /// Title, Artist and Album widths in the track tables; 0 hides a column, but
    /// not all three. A bad value is reported by `--check` and the default used.
    pub column_weights: Vec<i64>,
    /// Queue columns in display order: title, artist, album, len, track, year,
    /// codec.
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
    /// ASCII icons are used when asked for or when the locale is not UTF-8
    /// (Nerd Font glyphs would be mojibake). Font presence cannot be probed.
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

    /// The Queue's columns: a zero-weight text column is hidden, and the
    /// default list stands in when that leaves no text column.
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlbumArtConfig {
    /// auto | kitty | iterm2 | sixel | halfblocks | none
    pub backend: String,
    /// Maximum art size in pixels [width, height]; [0, 0] = no limit
    pub max_size_px: [u32; 2],
    /// Maximum on-disk art cache in MiB; least-recently-used art is evicted
    /// beyond it. `0` disables disk caching (art is re-downloaded every time).
    pub cache_mb: u64,
}

impl Default for MpvConfig {
    fn default() -> Self {
        Self { path: "mpv".into(), extra_args: Vec::new(), volume: 100.0 }
    }
}

impl Default for AlbumArtConfig {
    fn default() -> Self {
        Self { backend: "auto".into(), max_size_px: [0, 0], cache_mb: 200 }
    }
}

/// The variable when set and absolute, otherwise `<home>/<default>`. There is
/// no relative fallback: a path under the working directory could be planted.
fn xdg_dir(var: &str, default: &str) -> Option<PathBuf> {
    resolve_dir(std::env::var(var).ok(), dirs::home_dir(), default)
}

fn resolve_dir(xdg: Option<String>, home: Option<PathBuf>, default: &str) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(default)))
}

pub const NO_HOME: &str = "cannot find a home directory: set HOME";

/// The documented location, the same on every platform.
pub fn config_path() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|dir| dir.join("shmc").join("config.toml"))
}

/// `themes/` beside the config file, so `--config FILE` carries its own themes.
pub fn themes_dir_for(path: &Path) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new("")).join("themes")
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("shmc"))
}

pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library").join("Logs").join("shmc"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        xdg_dir("XDG_STATE_HOME", ".local/state").map(|dir| dir.join("shmc"))
    }
}

/// The log directory, created 0700, or the cache dir if it cannot be made.
pub fn usable_log_dir() -> Option<PathBuf> {
    if let Some(dir) = log_dir() {
        if crate::cache::ensure_private_dir(&dir).is_ok() {
            return Some(dir);
        }
    }
    crate::cache::ensure_cache_dir()
}

/// A theme plus colour overrides. Unset is the theme's own colour, and so is
/// a bad override (reported by `--check`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
    /// The `themes/` directory beside the config file; `None` means presets
    /// only. Not a setting: it comes from the path the config was read from.
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
    /// Painted behind everything: `#rrggbb`, or `none` for the terminal's own
    /// background even when the theme has one. Unset is the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Empty rows between the header, the tab bar, the content and the
    /// progress row: 0 to 3. Unset is the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_gap: Option<i64>,
    /// Columns inside a box's border on each side: 0 to 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inset: Option<i64>,
    /// How blocks are drawn: box, rounded, double, lines or none. Unset is the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<String>,
    /// A preset name (solid, line, dot, plain, angled, rounded) that sets the
    /// five glyphs below; any of them set too replaces its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_bar: Option<String>,
    /// The progress bar's filled and empty cells and its head, one character
    /// each. Unset is the theme's; a theme that sets none draws them as colour.
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

    /// `progress_bar` is first: a preset is applied before the glyphs that
    /// replace parts of it.
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
    /// Visualizer frames per second. Higher rates cost CPU and power for
    /// little visible gain.
    pub framerate: u32,
    /// Audio source to visualize. Empty = the platform default; a loopback
    /// device name when that cannot capture.
    pub source: String,
    /// Seconds cava keeps capturing after a pause or with the visualizer
    /// hidden. `0` stops it immediately.
    pub pause_grace_secs: u64,
    /// "balanced", "smooth", "snappy" or "custom"; only "custom" reads the five
    /// settings below.
    pub style: String,
    /// cava `smoothing:noise_reduction`, 0-100: higher is smoother and
    /// slower. Only used with the custom style.
    pub noise_reduction: i64,
    /// cava `general:autosens`: adjust the gain to the loudness. Only used
    /// with the custom style.
    pub autosens: bool,
    /// cava `general:sensitivity`, 1-300 (a percentage). Only used with the
    /// custom style.
    pub sensitivity: i64,
    /// cava `general:lower_cutoff_freq`, 10-1000 Hz. Only used with the custom
    /// style.
    pub lower_cutoff_hz: i64,
    /// cava `general:higher_cutoff_freq`, 1000-20000 Hz, above the lower one.
    /// Only used with the custom style.
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

    /// The numbers cava starts with: a preset sets the smoothing, `custom` the
    /// raw keys. Out-of-range values become the default.
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

    /// Problems for `shmc config --check`: keys only, never values. The raw
    /// numbers are checked only under the custom style.
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

fn flatten_values(table: &toml::Table, prefix: &str, out: &mut BTreeMap<String, toml::Value>) {
    for (key, value) in table {
        let key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            toml::Value::Table(section) => flatten_values(section, &key, out),
            value => {
                out.insert(key, value.clone());
            }
        }
    }
}

/// An integer and a float are equal when numerically so (`volume = 50` in a
/// file against `50.0` in the struct).
fn same_value(a: &toml::Value, b: &toml::Value) -> bool {
    use toml::Value::{Float, Integer};
    match (a, b) {
        (Integer(i), Float(f)) | (Float(f), Integer(i)) => *i as f64 == *f,
        _ => a == b,
    }
}

/// Keys whose value differs, in `mine`'s order, and keys to delete: a cleared
/// override is absent from `mine`, not changed in it.
fn changed_and_removed<'a>(
    mine: &'a BTreeMap<String, toml::Value>,
    theirs: &'a BTreeMap<String, toml::Value>,
) -> (Vec<&'a String>, Vec<&'a String>) {
    let changed = mine
        .iter()
        .filter(|(key, value)| !theirs.get(*key).is_some_and(|old| same_value(old, value)))
        .map(|(key, _)| key)
        .collect();
    let removed = theirs.keys().filter(|key| !mine.contains_key(*key)).collect();
    (changed, removed)
}

/// A parse error safe to print. `toml` quotes the line and the value, either
/// of which can be the token: only the line number and a clean message stay.
pub(crate) fn toml_error(e: &toml::de::Error, text: &str) -> anyhow::Error {
    let line = e.span().map(|s| text.get(..s.start).unwrap_or(text).matches('\n').count() + 1);
    let message = scrub_value(e.message());
    match line {
        Some(n) => anyhow::anyhow!("config.toml does not parse (line {n}): {message}"),
        None => anyhow::anyhow!("config.toml does not parse: {message}"),
    }
}

/// Drop the value serde quotes in `invalid type: KIND VALUE, expected …`,
/// `invalid value: …` and `unknown variant VALUE, …`.
pub(crate) fn scrub_value(message: &str) -> String {
    for lead in ["invalid type: ", "invalid value: ", "unknown variant "] {
        if let Some(rest) = message.strip_prefix(lead) {
            let kind =
                if lead == "unknown variant " { "" } else { rest.split(' ').next().unwrap_or("") };
            let tail = rest.find(", expected").map_or("", |i| &rest[i..]);
            return format!(
                "{lead}{kind}{}{tail}",
                if kind.is_empty() { "" } else { " (value hidden)" }
            );
        }
    }
    message.to_string()
}

/// Set a dotted key, creating tables on the way. A trailing comment on the
/// value being replaced stays.
fn set_in_document(doc: &mut toml_edit::DocumentMut, key: &str, value: &toml::Value) -> Result<()> {
    use toml_edit::{Item, Table};
    let parts: Vec<&str> = key.split('.').collect();
    let Some((leaf, parents)) = parts.split_last() else {
        return Ok(());
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parents {
        if !table.contains_key(part) {
            let mut fresh = Table::new();
            fresh.set_implicit(true);
            table.insert(part, Item::Table(fresh));
        }
        table = table
            .get_mut(part)
            .and_then(|item| item.as_table_like_mut())
            .with_context(|| format!("cannot set {key}: {part} is not a table"))?;
    }
    // The parse error quotes the value, which can be the token.
    let mut new = value
        .to_string()
        .parse::<toml_edit::Value>()
        .map_err(|_| anyhow::anyhow!("cannot write {key}"))?;
    if let Some(old) = table.get(leaf).and_then(|item| item.as_value()) {
        *new.decor_mut() = old.decor().clone();
    }
    table.insert(leaf, Item::Value(new));
    Ok(())
}

fn remove_from_document(doc: &mut toml_edit::DocumentMut, key: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((leaf, parents)) = parts.split_last() else {
        return;
    };
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parents {
        let Some(next) = table.get_mut(part).and_then(|item| item.as_table_like_mut()) else {
            return;
        };
        table = next;
    }
    table.remove(leaf);
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

/// Anything but `https://` counts as not encrypted: the token rides in the
/// query string.
pub fn server_is_https(server: &str) -> bool {
    server.trim().to_ascii_lowercase().starts_with("https://")
}

/// A warning, never an error: plain HTTP on a LAN is legitimate. `None` for
/// https or no server yet.
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

/// Created 0700. An existing dir is tightened only at the default location:
/// with `--config` it can be any directory.
fn ensure_config_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !dir.exists() {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            return std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        }
        if config_path().as_deref().and_then(Path::parent) == Some(dir) {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Restrict `path` to 0600 and read the mode back: some filesystems accept a
/// chmod and ignore it. That only warns; refusing to save would be worse.
#[cfg(unix)]
fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting {}", path.display()))?;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("checking {}", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        eprintln!(
            "warning: {} is mode {mode:03o}, not 0600; this filesystem may ignore permissions, \
             so other local users could read the Plex token",
            path.display()
        );
    }
    Ok(())
}

/// Replace `path` via a 0600 temp file: `fs::write` would leave the token
/// readable until a chmod. A symlinked config stays a link.
fn write_private(path: &Path, body: &str) -> Result<()> {
    use std::io::Write;

    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = resolved.as_path();

    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".to_string());
    // pid + nanos: two instances, or a crashed run's leftover, must not collide.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.tmp.{}.{nanos}", std::process::id()));

    let outcome = (|| -> Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(body.as_bytes()).with_context(|| format!("writing {}", tmp.display()))?;
        drop(file);
        #[cfg(unix)]
        set_private(&tmp)?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

impl Config {
    pub fn load(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            bail!("{}", crate::setup::hint(path));
        }
        let cfg = Self::read(path)?;
        if cfg.needs_setup() {
            bail!(
                "config at {} must set both plex.server and plex.token\n\nRun 'shmc config' to set it up.",
                path.display()
            );
        }
        Ok(cfg)
    }

    /// Parse without requiring `plex.server` and `plex.token`, so setup can
    /// prefill from a half-written file.
    pub fn read(path: &PathBuf) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg = Self::parse(&text)?;
        cfg.set_themes_dir(path);
        Ok(cfg)
    }

    /// [`Config::read`] on text in hand, and silent: stderr is the alternate
    /// screen while the config screen is up.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| toml_error(&e, text))
    }

    pub fn needs_setup(&self) -> bool {
        self.plex.server.trim().is_empty() || self.plex.token.trim().is_empty()
    }

    /// An existing file is edited in place against its own current contents
    /// (see [`Config::save_changes`]); a missing one is written in full.
    pub fn save(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            return self.save_full(path);
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let on_disk = Self::parse(&text)?;
        self.save_changes(path, &on_disk)
    }

    fn save_full(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                ensure_config_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
            }
        }
        // Not `unwrap_or_default()`: that would write just the header, wiping
        // `plex.server` and `plex.token`.
        let serialised = toml::to_string_pretty(self).context("serialising the config")?;
        let body = format!(
            "# shmc configuration — written by `shmc setup`\n\
             # Holds your Plex token: keep it private (chmod 600).\n\
             \n\
             {serialised}"
        );
        write_private(path, &body)
    }

    /// Write only what differs from `baseline`, keeping comments, order, unknown
    /// keys and edits made elsewhere. A file that does not parse is left alone.
    pub fn save_changes(&self, path: &Path, baseline: &Config) -> Result<()> {
        use toml_edit::DocumentMut;
        if !path.exists() {
            return self.save_full(path);
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        // The parse error quotes the offending line, which can be the token.
        let mut doc = text
            .parse::<DocumentMut>()
            .map_err(|_| anyhow::anyhow!("config.toml does not parse, so nothing was written"))?;
        let mine = self.flat_values()?;
        let theirs = baseline.flat_values()?;
        let (changed, removed) = changed_and_removed(&mine, &theirs);
        for key in changed {
            if let Some(value) = mine.get(key) {
                set_in_document(&mut doc, key, value)?;
            }
        }
        for key in removed {
            remove_from_document(&mut doc, key);
        }
        write_private(path, &doc.to_string())
    }

    pub fn changed_keys(&self, baseline: &Config) -> Vec<String> {
        let (Ok(mine), Ok(theirs)) = (self.flat_values(), baseline.flat_values()) else {
            return Vec::new();
        };
        let (changed, removed) = changed_and_removed(&mine, &theirs);
        changed.into_iter().chain(removed).cloned().collect()
    }

    /// What someone else changed in the file since `baseline` was read.
    pub fn outside_changes(path: &Path, baseline: &Config) -> Result<Vec<String>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Self::parse(&text)?.changed_keys(baseline))
    }

    pub(crate) fn flat_values(&self) -> Result<BTreeMap<String, toml::Value>> {
        let table = toml::Table::try_from(self).context("serialising the config")?;
        let mut out = BTreeMap::new();
        flatten_values(&table, "", &mut out);
        Ok(out)
    }

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

    #[test]
    fn dirs_resolve_without_a_relative_fallback() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(resolve_dir(None, None, ".config"), None);
        assert_eq!(resolve_dir(Some("rel/dir".into()), None, ".config"), None);
        assert_eq!(resolve_dir(Some("/xdg".into()), None, ".config"), Some(PathBuf::from("/xdg")));
        assert_eq!(
            resolve_dir(Some("rel".into()), home, ".config"),
            Some(PathBuf::from("/home/me/.config"))
        );
    }

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

    #[cfg(unix)]
    #[test]
    fn save_through_symlink_keeps_link() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("dotfiles");
        std::fs::create_dir(&elsewhere).unwrap();
        let target = elsewhere.join("config.toml");
        std::fs::write(&target, "[plex]\nserver = \"http://old:32400\"\n").unwrap();
        let link = tmp.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_private(&link, "[plex]\nserver = \"http://new:32400\"\n").unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "[plex]\nserver = \"http://new:32400\"\n"
        );
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1, "temp left");
    }

    #[test]
    fn unparsable_config_never_shows_token_line() {
        for bad in ["token = FAKETOKEN123abc", "token = 98765", "token = \"FAKETOKEN123abc\" !"] {
            let text = format!("[plex]\nserver = \"http://h:1\"\n{bad}\n");
            let Err(e) = Config::parse(&text) else {
                panic!("{bad} parsed");
            };
            let shown = format!("{e:#}");
            assert!(!shown.contains("FAKETOKEN123abc"), "{shown}");
            assert!(!shown.contains("98765"), "{shown}");
            assert!(!shown.contains(bad), "{shown}");
            assert!(shown.contains("line 3"), "{shown}");
        }
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
        // no scheme at all is not https
        assert!(!server_is_https("192.168.1.100:32400"));
        assert!(!server_is_https(""));
        // nothing configured yet: nothing to warn about
        assert!(plain_http_warning("").is_none());
    }

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
    fn save_keeps_hand_written_keys_table() {
        let before = "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n\n\
                      [keys.queue]\nremove = [\"d\", \"Delete\"] # mine\nundo = []\n";
        let (_dir, path) = saved_file(before);
        let baseline = Config::read(&path).unwrap();
        let mut cfg = baseline.clone();
        cfg.ui.art_pct = 55;
        cfg.save_changes(&path, &baseline).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("remove = [\"d\", \"Delete\"] # mine"), "{after}");
        assert!(after.contains("undo = []"), "{after}");
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

    #[test]
    fn missing_fields_fall_back_to_current_defaults() {
        // A config without `album_art.cache_mb` and `cava.source` gets their
        // defaults (not zero / not "off").
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
    fn save_round_trips_and_keeps_settings() {
        let dir = std::env::temp_dir().join(format!("shmc-cfg-test-{}", std::process::id()));
        let path = dir.join("config.toml");

        let mut cfg = Config {
            plex: PlexConfig {
                server: "http://192.168.1.100:32400".into(),
                token: "tok\"en".into(),
                ..PlexConfig::default()
            },
            ..Config::default()
        };
        cfg.ui.icons = "ascii".into();
        cfg.mpv.extra_args = vec!["--gapless-audio=yes".into()];
        cfg.save(&path).unwrap();

        let back = Config::load(&path).unwrap();
        assert_eq!(back.plex.server, "http://192.168.1.100:32400");
        assert_eq!(back.plex.token, "tok\"en");
        assert_eq!(back.ui.icons, "ascii");
        assert_eq!(back.mpv.extra_args, vec!["--gapless-audio=yes".to_string()]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("shmc").join(format!("{name}.toml"));
        (tmp, path)
    }

    #[cfg(unix)]
    #[test]
    fn save_into_existing_dir_keeps_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shmc");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        Config::default().save(&dir.join("config.toml")).unwrap();
        assert_eq!(mode(&dir), 0o755, "a --config directory is not shmc's to chmod");
    }

    #[test]
    fn save_writes_private_file_no_leftovers() {
        let (_tmp, path) = scratch("config");
        let cfg = Config {
            plex: PlexConfig {
                server: "http://host:32400".into(),
                token: "secret".into(),
                ..PlexConfig::default()
            },
            ..Config::default()
        };
        let names = || -> Vec<String> {
            std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        };

        cfg.save(&path).unwrap();
        assert_eq!(names(), vec!["config.toml".to_string()], "no temp file left");
        #[cfg(unix)]
        {
            assert_eq!(mode(&path), 0o600, "a new config holds the token");
            assert_eq!(mode(path.parent().unwrap()), 0o700, "shmc's config directory");

            // A hand-edited config is usually 0644; saving over it must tighten it.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            cfg.save(&path).unwrap();
            assert_eq!(mode(&path), 0o600, "an existing 0644 config must be replaced 0600");
            assert_eq!(names(), vec!["config.toml".to_string()], "no temp file left");
        }
    }

    #[test]
    fn failed_save_removes_temp_file() {
        // A directory at the target makes the final rename fail after the temp
        // file was written: the old target stays and nothing is left behind.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shmc");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::create_dir(&path).unwrap();

        assert!(Config::default().save(&path).is_err());
        assert!(path.is_dir(), "the failed save must not replace the target");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["config.toml".to_string()]);
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

    fn saved_file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn save_keeps_comments_and_unknown_keys() {
        let tail = "\n[future]\nx = 1\n";
        let shapes = [
            // A `[ui]` table.
            "# my config\n[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n\n[ui]\nart_pct = 40 # note\nnewer = true\n",
            // The same setting as a top-level dotted key.
            "# my config\nplex.server = \"http://x:32400\"\nplex.token = \"OLD\"\nui.art_pct = 40 # note\nui.newer = true\n",
        ];
        for shape in shapes {
            let before = format!("{shape}{tail}");
            let (_dir, path) = saved_file(&before);
            let baseline = Config::read(&path).unwrap();
            let mut cfg = baseline.clone();
            cfg.ui.art_pct = 55;
            cfg.save_changes(&path, &baseline).unwrap();

            let after = std::fs::read_to_string(&path).unwrap();
            let (old, new): (Vec<_>, Vec<_>) = (before.lines().collect(), after.lines().collect());
            assert_eq!(old.len(), new.len(), "{after}");
            for (o, n) in old.iter().zip(&new) {
                if o.contains("art_pct") {
                    assert_eq!(n.replace("55", "40"), *o, "{after}");
                    assert!(n.contains("55"), "{after}");
                } else {
                    assert_eq!(o, n, "{after}");
                }
            }
        }
    }

    #[test]
    fn outside_change_survives_save() {
        let (_dir, path) = saved_file("[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n");
        let baseline = Config::read(&path).unwrap();
        let mut cfg = baseline.clone();
        cfg.ui.art_pct = 55;

        // Someone else edits another key after the baseline was read.
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("[ui]\nscrolloff = 9\n");
        std::fs::write(&path, text).unwrap();
        cfg.save_changes(&path, &baseline).unwrap();

        let now = Config::read(&path).unwrap();
        assert_eq!(now.ui.art_pct, 55);
        assert_eq!(now.ui.scrolloff, 9);
    }

    #[test]
    fn save_creates_a_missing_plex_table() {
        let before = "# mine\n[ui]\nart_pct = 40 # note\n";
        let (_dir, path) = saved_file(before);
        let baseline = Config::read(&path).unwrap();
        let mut cfg = baseline.clone();
        cfg.plex.server = "http://x:32400".into();
        cfg.save_changes(&path, &baseline).unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(before), "{after}");
        assert!(after.contains("[plex]"), "{after}");
        assert_eq!(Config::read(&path).unwrap().plex.server, "http://x:32400");
    }

    #[test]
    fn old_theme_table_becomes_overrides() {
        let cfg = Config::parse("[theme]\naccent = \"#abcdef\"\n").unwrap();
        assert_eq!(cfg.theme.name, "default");
        assert_eq!(cfg.theme.accent.as_deref(), Some("#abcdef"));
        assert_eq!(cfg.theme.secondary, None);

        let fresh = toml::to_string(&Config::default()).unwrap();
        let table: toml::Table = fresh.parse().unwrap();
        let keys: Vec<&String> = table["theme"].as_table().unwrap().keys().collect();
        assert_eq!(keys, ["name"], "{fresh}");
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

    #[test]
    fn clearing_override_removes_only_its_key() {
        let (_dir, path) = saved_file(
            "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n[theme]\n# mine\nname = \"nord\"\naccent = \"#abcdef\"\nsecondary = \"#123456\"\n",
        );
        let baseline = Config::read(&path).unwrap();
        let mut cfg = baseline.clone();
        cfg.restore_one("theme.accent").expect("an override was cleared");
        cfg.save_changes(&path, &baseline).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("accent"), "{text}");
        assert!(text.contains("# mine") && text.contains("secondary"), "{text}");
        assert_eq!(Config::read(&path).unwrap().theme.name, "nord");
        // What the screen compares to notices the removal.
        assert_eq!(cfg.changed_keys(&baseline), ["theme.accent"]);
    }

    #[test]
    fn unparsable_file_left_alone() {
        let garbage = "[plex]\ntoken = \"OLD\n[[[ nope\n";
        let (_dir, path) = saved_file(garbage);
        let cfg = Config::default();
        let err = cfg.save_changes(&path, &Config::default());
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), garbage);
    }

    /// Spacing and symbols behave like the colours: unset shows the theme's
    /// value, an equal override is still a change, and `r` drops it.
    #[test]
    fn override_saved_as_its_own_key() {
        let (dir, path) = saved_file(
            "[plex]\nserver = \"http://x:32400\"\ntoken = \"OLD\"\n[theme]\nname = \"mine\"\n",
        );
        std::fs::create_dir(dir.path().join("themes")).unwrap();
        std::fs::write(
            dir.path().join("themes/mine.toml"),
            "[spacing]\ninset = 2\n[symbols]\nellipsis = \"~\"\n",
        )
        .unwrap();
        let baseline = {
            let mut cfg = Config::read(&path).unwrap();
            cfg.set_themes_dir(&path);
            cfg
        };
        let row = |cfg: &Config, key: &str| {
            cfg.setting_rows().into_iter().find(|r| r.key == key).unwrap()
        };
        // Unset: the theme's value is shown, and it is the default.
        let (inset, ellipsis) = (row(&baseline, "theme.inset"), row(&baseline, "theme.ellipsis"));
        assert_eq!((inset.current.as_str(), inset.is_default()), ("2", true));
        assert_eq!((ellipsis.current.as_str(), ellipsis.is_default()), ("~", true));
        assert_eq!(row(&baseline, "theme.scroll_up").current, "▲", "no theme value: the default");
        assert_eq!(baseline.flat_settings()["theme.block_gap"], "0");

        let mut cfg = baseline.clone();
        cfg.theme.inset = Some(2); // the theme's own value, as an override
        cfg.theme.ellipsis = Some("…".into());
        assert!(!row(&cfg, "theme.inset").is_default(), "equal to the theme's, still an override");
        assert_eq!(cfg.changed_keys(&baseline), ["theme.ellipsis", "theme.inset"]);
        cfg.save_changes(&path, &baseline).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("inset = 2") && text.contains("ellipsis = \"…\""), "{text}");

        let saved = Config::read(&path).unwrap();
        let mut again = saved.clone();
        again.set_themes_dir(&path);
        let change = again.restore_one("theme.ellipsis").expect("the override was dropped");
        assert_eq!((change.from.as_str(), change.to.as_str()), ("…", "~"));
        assert!(again.restore_one("theme.ellipsis").is_none(), "unset already matches");
        again.save_changes(&path, &saved).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("ellipsis") && text.contains("inset = 2"), "{text}");
    }
}
