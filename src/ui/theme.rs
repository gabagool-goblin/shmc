//! The palette and its text helpers; the rest of the UI gets colours only
//! through accessors. A `RwLock` so the config screen can preview colours.

use crate::config::ThemeConfig;
use ratatui::style::{Color, Modifier, Style};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Interaction: the cursor, the active tab, the filled progress bar, icons.
    pub accent: Color,
    /// Text drawn on an accent background.
    pub on_accent: Color,
    /// Default content.
    pub text: Color,
    /// Dimmed content and the inactive tab.
    pub dim: Color,
    /// Emphasis: Help keys, the header artist, filter matches.
    pub secondary: Color,
    pub error: Color,
    /// Pane and box outlines.
    pub border: Color,
    /// The cursor of a pane that does not have keyboard focus.
    pub highlight: Color,
    /// The visualizer ramp: bottom, middle, top.
    pub viz_bottom: Color,
    pub viz_mid: Color,
    pub viz_top: Color,
    /// `None` is the terminal's own background.
    pub background: Option<Color>,
}

pub const DEFAULT: Theme = Theme {
    accent: Color::Rgb(0x7a, 0xa2, 0xf7),
    on_accent: Color::Rgb(0x16, 0x1e, 0x2e),
    text: Color::Rgb(0xc0, 0xca, 0xf5),
    dim: Color::Rgb(0x56, 0x5f, 0x89),
    secondary: Color::Rgb(0xe0, 0xaf, 0x68),
    error: Color::Rgb(0xf7, 0x76, 0x8e),
    border: Color::Rgb(0x56, 0x5f, 0x89),
    highlight: Color::Rgb(0x41, 0x48, 0x68),
    viz_bottom: Color::Rgb(0xff, 0x2d, 0x78),
    viz_mid: Color::Rgb(0xb0, 0x80, 0xff),
    viz_top: Color::Rgb(0x22, 0xc0, 0xff),
    background: None,
};

impl Default for Theme {
    fn default() -> Self {
        DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    Box,
    Rounded,
    Double,
    Lines,
    None,
}

impl BorderStyle {
    pub const NAMES: [&'static str; 5] = ["box", "rounded", "double", "lines", "none"];

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "box" => Some(Self::Box),
            "rounded" => Some(Self::Rounded),
            "double" => Some(Self::Double),
            "lines" => Some(Self::Lines),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn boxed(self) -> bool {
        matches!(self, Self::Box | Self::Rounded | Self::Double)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spacing {
    /// Empty rows between the header, tabs, content and progress row.
    pub block_gap: u16,
    /// Columns inside a box's border on each side.
    pub inset: u16,
    pub borders: BorderStyle,
}

impl Spacing {
    pub const DEFAULT: Spacing = Spacing { block_gap: 0, inset: 1, borders: BorderStyle::Box };
    pub const MAX: i64 = 3;
}

/// The header reserves eight cells for the meter either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meter {
    /// Lowest first: cell `i` draws the `i`th glyph, lit or not.
    Ramp([char; 8]),
    /// Lit, unlit.
    Pair(char, char),
}

/// Each slot is one single-column character, so the layout never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbols {
    /// `None` is a cell coloured through its background, no glyph.
    pub progress_filled: Option<char>,
    pub progress_empty: Option<char>,
    pub progress_head: Option<char>,
    /// Drawn outside the bar; `None` is no cap.
    pub progress_start: Option<char>,
    pub progress_end: Option<char>,
    pub ellipsis: char,
    pub scroll_up: char,
    pub scroll_down: char,
    pub scroll_thumb: char,
    pub volume_meter: Meter,
    /// Lowest level first.
    pub visualizer: [char; 8],
}

pub const EIGHTHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Filled, empty, head, start cap, end cap: the order of [`PROGRESS_PRESETS`].
pub type ProgressGlyphs = [Option<char>; 5];

impl Symbols {
    pub fn progress(&self) -> ProgressGlyphs {
        [
            self.progress_filled,
            self.progress_empty,
            self.progress_head,
            self.progress_start,
            self.progress_end,
        ]
    }

    fn set_progress(&mut self, g: ProgressGlyphs) {
        [
            self.progress_filled,
            self.progress_empty,
            self.progress_head,
            self.progress_start,
            self.progress_end,
        ] = g;
    }

    pub const DEFAULT: Symbols = Symbols {
        progress_filled: None,
        progress_empty: None,
        progress_head: None,
        progress_start: None,
        progress_end: None,
        ellipsis: '…',
        scroll_up: '▲',
        scroll_down: '▼',
        scroll_thumb: '█',
        volume_meter: Meter::Ramp(EIGHTHS),
        visualizer: EIGHTHS,
    };
}

/// What a style's `fg` and `bg` may point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Accent,
    OnAccent,
    Text,
    Dim,
    Secondary,
    Error,
    Border,
    Highlight,
    VizBottom,
    VizMid,
    VizTop,
    Background,
}

impl Role {
    pub const ALL: [(&'static str, Role); 12] = [
        ("accent", Role::Accent),
        ("on_accent", Role::OnAccent),
        ("text", Role::Text),
        ("dim", Role::Dim),
        ("secondary", Role::Secondary),
        ("error", Role::Error),
        ("border", Role::Border),
        ("highlight", Role::Highlight),
        ("viz_bottom", Role::VizBottom),
        ("viz_mid", Role::VizMid),
        ("viz_top", Role::VizTop),
        ("background", Role::Background),
    ];

    fn name(self) -> &'static str {
        Role::ALL.iter().find(|(_, r)| *r == self).map_or("", |(n, _)| n)
    }

    fn colour(self, theme: &Theme) -> Color {
        match self {
            Role::Accent => theme.accent,
            Role::OnAccent => theme.on_accent,
            Role::Text => theme.text,
            Role::Dim => theme.dim,
            Role::Secondary => theme.secondary,
            Role::Error => theme.error,
            Role::Border => theme.border,
            Role::Highlight => theme.highlight,
            Role::VizBottom => theme.viz_bottom,
            Role::VizMid => theme.viz_mid,
            Role::VizTop => theme.viz_top,
            Role::Background => theme.background.unwrap_or(Color::Reset),
        }
    }
}

/// Looked up each time a style is built, so it follows overrides and the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRef {
    Role(Role),
    Rgb(u8, u8, u8),
}

impl ColorRef {
    pub fn parse(text: &str) -> Option<ColorRef> {
        if let Some((_, role)) = Role::ALL.iter().find(|(n, _)| *n == text.trim()) {
            return Some(ColorRef::Role(*role));
        }
        match parse_hex(text)? {
            Color::Rgb(r, g, b) => Some(ColorRef::Rgb(r, g, b)),
            _ => None,
        }
    }

    fn resolve(self, theme: &Theme) -> Color {
        match self {
            ColorRef::Role(role) => role.colour(theme),
            ColorRef::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }

    fn text(self) -> String {
        match self {
            ColorRef::Role(role) => role.name().to_string(),
            ColorRef::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Accent,
    Secondary,
    Text,
    Dim,
    Error,
    Warn,
    Border,
    Current,
    CurrentUnfocused,
    TabActive,
    TabInactive,
    ProgressFill,
    ProgressTrack,
}

impl Element {
    /// In export order. `tab_active` does not follow `current`, nor `warn` `secondary`.
    pub const ALL: [(&'static str, Element); 13] = [
        ("accent", Element::Accent),
        ("secondary", Element::Secondary),
        ("text", Element::Text),
        ("dim", Element::Dim),
        ("error", Element::Error),
        ("warn", Element::Warn),
        ("border", Element::Border),
        ("current", Element::Current),
        ("current_unfocused", Element::CurrentUnfocused),
        ("tab_active", Element::TabActive),
        ("tab_inactive", Element::TabInactive),
        ("progress_fill", Element::ProgressFill),
        ("progress_track", Element::ProgressTrack),
    ];
}

const MODIFIERS: [(&str, Modifier); 5] = [
    ("bold", Modifier::BOLD),
    ("italic", Modifier::ITALIC),
    ("underline", Modifier::UNDERLINED),
    ("reversed", Modifier::REVERSED),
    ("dim", Modifier::DIM),
];

/// Only the fields a theme lists, over the element's default style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StylePatch {
    pub fg: Option<ColorRef>,
    pub bg: Option<ColorRef>,
    pub add: Modifier,
    pub remove: Modifier,
}

impl StylePatch {
    fn is_empty(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && self.add.is_empty() && self.remove.is_empty()
    }

    fn apply(&self, mut style: Style, theme: &Theme) -> Style {
        if let Some(c) = self.fg {
            style = style.fg(c.resolve(theme));
        }
        if let Some(c) = self.bg {
            style = style.bg(c.resolve(theme));
        }
        style.add_modifier(self.add).remove_modifier(self.remove)
    }

    /// What does not fit is left out and its key returned; the rest applies.
    pub fn parse(entry: &toml::Table) -> (StylePatch, Vec<String>) {
        let mut patch =
            StylePatch { fg: None, bg: None, add: Modifier::empty(), remove: Modifier::empty() };
        let mut bad = Vec::new();
        for (key, value) in entry {
            let ok = match key.as_str() {
                "fg" => {
                    value.as_str().and_then(ColorRef::parse).map(|c| patch.fg = Some(c)).is_some()
                }
                "bg" => {
                    value.as_str().and_then(ColorRef::parse).map(|c| patch.bg = Some(c)).is_some()
                }
                other => match (MODIFIERS.iter().find(|(n, _)| *n == other), value.as_bool()) {
                    (Some((_, m)), Some(true)) => {
                        patch.add |= *m;
                        true
                    }
                    (Some((_, m)), Some(false)) => {
                        patch.remove |= *m;
                        true
                    }
                    _ => false,
                },
            };
            if !ok {
                bad.push(key.clone());
            }
        }
        (patch, bad)
    }

    /// As a theme file writes it: `{ fg = "accent", bold = true }`.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        for (key, colour) in [("fg", self.fg), ("bg", self.bg)] {
            if let Some(c) = colour {
                parts.push(format!("{key} = \"{}\"", c.text()));
            }
        }
        for (name, m) in MODIFIERS {
            if self.add.contains(m) {
                parts.push(format!("{name} = true"));
            } else if self.remove.contains(m) {
                parts.push(format!("{name} = false"));
            }
        }
        format!("{{ {} }}", parts.join(", "))
    }
}

/// One slot per [`Element`]; unset is the element's default style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Styles([Option<StylePatch>; 13]);

impl Styles {
    pub const NONE: Styles = Styles([None; 13]);

    pub fn get(&self, element: Element) -> Option<StylePatch> {
        self.0[element as usize]
    }

    /// An empty patch clears the slot.
    pub fn set(&mut self, element: Element, patch: StylePatch) {
        self.0[element as usize] = (!patch.is_empty()).then_some(patch);
    }

    pub fn count(&self) -> usize {
        self.0.iter().flatten().count()
    }
}

/// What a theme sets besides colours; styles come from theme files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub spacing: Spacing,
    pub symbols: Symbols,
    pub styles: Styles,
}

impl Look {
    pub const DEFAULT: Look =
        Look { spacing: Spacing::DEFAULT, symbols: Symbols::DEFAULT, styles: Styles::NONE };

    /// Every non-ASCII symbol back to the default, for `ui.icons = "ascii"`.
    pub fn for_ascii(self) -> Look {
        let (mut s, d) = (self.symbols, Symbols::DEFAULT);
        let keep = |c: &Option<char>| c.is_none_or(|c| c.is_ascii());
        if !keep(&s.progress_filled) {
            s.progress_filled = d.progress_filled;
        }
        if !keep(&s.progress_empty) {
            s.progress_empty = d.progress_empty;
        }
        if !keep(&s.progress_head) {
            s.progress_head = d.progress_head;
        }
        if !keep(&s.progress_start) {
            s.progress_start = d.progress_start;
        }
        if !keep(&s.progress_end) {
            s.progress_end = d.progress_end;
        }
        for (slot, default) in [
            (&mut s.ellipsis, d.ellipsis),
            (&mut s.scroll_up, d.scroll_up),
            (&mut s.scroll_down, d.scroll_down),
            (&mut s.scroll_thumb, d.scroll_thumb),
        ] {
            if !slot.is_ascii() {
                *slot = default;
            }
        }
        let meter_ascii = match s.volume_meter {
            Meter::Ramp(r) => r.iter().all(char::is_ascii),
            Meter::Pair(a, b) => a.is_ascii() && b.is_ascii(),
        };
        if !meter_ascii {
            s.volume_meter = d.volume_meter;
        }
        if !s.visualizer.iter().all(char::is_ascii) {
            s.visualizer = d.visualizer;
        }
        Look { symbols: s, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    pub theme: Theme,
    pub look: Look,
}

impl Default for Look {
    fn default() -> Self {
        Look::DEFAULT
    }
}

#[cfg(not(test))]
static LOOK: RwLock<Look> = RwLock::new(Look::DEFAULT);

// Per thread in unit tests so glyphs do not leak between tests.
#[cfg(test)]
thread_local! {
    static LOOK: std::cell::Cell<Look> = const { std::cell::Cell::new(Look::DEFAULT) };
}

pub fn look() -> Look {
    #[cfg(test)]
    {
        LOOK.get()
    }
    #[cfg(not(test))]
    {
        *LOOK.read().expect("look lock")
    }
}

pub fn set_look(look: Look) {
    #[cfg(test)]
    LOOK.set(look);
    #[cfg(not(test))]
    {
        *LOOK.write().expect("look lock") = look;
    }
}

/// `ascii` is `ui.icons = "ascii"`.
pub fn set_appearance(appearance: Appearance, ascii: bool) {
    set_theme(appearance.theme);
    set_look(if ascii { appearance.look.for_ascii() } else { appearance.look });
}

pub fn spacing() -> Spacing {
    look().spacing
}

pub fn symbols() -> Symbols {
    look().symbols
}

pub fn ellipsis() -> char {
    symbols().ellipsis
}

/// One character, one column wide: anything else shifts the cells the layout counts.
pub fn parse_glyph(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control() && str_width(text) == 1).then_some(c)
}

/// One character, or `solid` for none; `solid` overrides a theme's glyph.
fn parse_progress_glyph(text: &str) -> Option<Option<char>> {
    if text == "solid" {
        return Some(None);
    }
    parse_glyph(text).map(Some)
}

pub fn parse_ramp(text: &str) -> Option<[char; 8]> {
    let glyphs: Vec<char> = text.chars().collect();
    let glyphs: [char; 8] = glyphs.try_into().ok()?;
    glyphs.iter().all(|c| parse_glyph(&c.to_string()).is_some()).then_some(glyphs)
}

/// A value that is exactly a name is that preset, else glyphs; the first is the default.
pub const METER_PRESETS: &[(&str, Meter)] = &[
    ("blocks", Meter::Ramp(EIGHTHS)),
    ("squares", Meter::Pair('■', '·')),
    ("outline", Meter::Pair('■', '□')),
    ("plain", Meter::Pair('#', '-')),
];

pub const VISUALIZER_PRESETS: &[(&str, [char; 8])] =
    &[("blocks", EIGHTHS), ("ascii", ['_', '.', '-', '=', '+', '*', '#', '@'])];

/// `angled` and `rounded` need Powerline glyphs.
pub const PROGRESS_PRESETS: &[(&str, ProgressGlyphs)] = &[
    ("solid", [None; 5]),
    ("line", [Some('━'), Some('─'), None, None, None]),
    ("dot", [Some('━'), Some('─'), Some('●'), None, None]),
    ("plain", [Some('='), Some('-'), Some('>'), None, None]),
    ("angled", [None, None, Some('\u{e0b0}'), None, None]),
    ("rounded", [None, None, None, Some('\u{e0b6}'), Some('\u{e0b4}')]),
];

/// For the config screen's picker.
pub fn symbol_presets(key: &str) -> Option<Vec<&'static str>> {
    match key {
        "progress_bar" => Some(PROGRESS_PRESETS.iter().map(|(n, _)| *n).collect()),
        "volume_meter" => Some(METER_PRESETS.iter().map(|(n, _)| *n).collect()),
        "visualizer" => Some(VISUALIZER_PRESETS.iter().map(|(n, _)| *n).collect()),
        _ => None,
    }
}

pub fn parse_visualizer(text: &str) -> Option<[char; 8]> {
    match VISUALIZER_PRESETS.iter().find(|(name, _)| *name == text) {
        Some((_, ramp)) => Some(*ramp),
        None => parse_ramp(text),
    }
}

pub fn parse_meter(text: &str) -> Option<Meter> {
    if let Some((_, meter)) = METER_PRESETS.iter().find(|(name, _)| *name == text) {
        return Some(*meter);
    }
    if let Some(ramp) = parse_ramp(text) {
        return Some(Meter::Ramp(ramp));
    }
    let glyphs: Vec<char> = text.chars().collect();
    match glyphs[..] {
        [a, b] if [a, b].iter().all(|c| parse_glyph(&c.to_string()).is_some()) => {
            Some(Meter::Pair(a, b))
        }
        _ => None,
    }
}

pub fn symbol_rule(key: &str) -> &'static str {
    match key {
        "volume_meter" => {
            "must be blocks, squares, outline or plain, or 8 characters, or 2 (lit, unlit), each one column wide"
        }
        "visualizer" => "must be blocks or ascii, or 8 characters, each one column wide",
        "progress_bar" => "must be solid, line, dot, plain, angled or rounded",
        "progress_filled" | "progress_empty" | "progress_head" | "progress_start" | "progress_end" => {
            "must be one character, one column wide, or solid"
        }
        _ => "must be one character, one column wide",
    }
}

/// False, and the look untouched, when the key or the text does not fit.
pub fn set_symbol(look: &mut Look, key: &str, text: &str) -> bool {
    let s = &mut look.symbols;
    match key {
        "progress_bar" => PROGRESS_PRESETS
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(_, glyphs)| s.set_progress(*glyphs)),
        "progress_filled" => parse_progress_glyph(text).map(|c| s.progress_filled = c),
        "progress_empty" => parse_progress_glyph(text).map(|c| s.progress_empty = c),
        "progress_head" => parse_progress_glyph(text).map(|c| s.progress_head = c),
        "progress_start" => parse_progress_glyph(text).map(|c| s.progress_start = c),
        "progress_end" => parse_progress_glyph(text).map(|c| s.progress_end = c),
        "ellipsis" => parse_glyph(text).map(|c| s.ellipsis = c),
        "scroll_up" => parse_glyph(text).map(|c| s.scroll_up = c),
        "scroll_down" => parse_glyph(text).map(|c| s.scroll_down = c),
        "scroll_thumb" => parse_glyph(text).map(|c| s.scroll_thumb = c),
        "volume_meter" => parse_meter(text).map(|m| s.volume_meter = m),
        "visualizer" => parse_visualizer(text).map(|r| s.visualizer = r),
        _ => None,
    }
    .is_some()
}

/// False, and the look untouched, for an unknown key or a value outside `0..=3`.
pub fn set_spacing(look: &mut Look, key: &str, value: i64) -> bool {
    let Ok(v) = u16::try_from(value) else {
        return false;
    };
    if i64::from(v) > Spacing::MAX {
        return false;
    }
    match key {
        "block_gap" => look.spacing.block_gap = v,
        "inset" => look.spacing.inset = v,
        _ => return false,
    }
    true
}

/// False, and the look untouched, for an unknown key or word.
pub fn set_spacing_text(look: &mut Look, key: &str, text: &str) -> bool {
    match (key, BorderStyle::parse(text)) {
        ("borders", Some(style)) => look.spacing.borders = style,
        _ => return false,
    }
    true
}

pub fn spacing_text_rule(key: &str) -> &'static str {
    match key {
        "borders" => "must be box, rounded, double, lines or none",
        _ => "is not a setting",
    }
}

/// As the config shows and an export writes it; an unset progress glyph is `solid`.
pub fn look_text(look: &Look, key: &str) -> Option<String> {
    let s = &look.symbols;
    let glyph = |c: Option<char>| c.map_or_else(|| "solid".to_string(), String::from);
    Some(match key {
        "block_gap" => look.spacing.block_gap.to_string(),
        "inset" => look.spacing.inset.to_string(),
        "borders" => look.spacing.borders.name().to_string(),
        "progress_filled" => glyph(s.progress_filled),
        "progress_empty" => glyph(s.progress_empty),
        "progress_head" => glyph(s.progress_head),
        "progress_start" => glyph(s.progress_start),
        "progress_end" => glyph(s.progress_end),
        // `custom` is no value to set; the glyphs are the other rows.
        "progress_bar" => PROGRESS_PRESETS
            .iter()
            .find(|(_, g)| *g == s.progress())
            .map_or("custom", |(name, _)| *name)
            .to_string(),
        "ellipsis" => s.ellipsis.to_string(),
        "scroll_up" => s.scroll_up.to_string(),
        "scroll_down" => s.scroll_down.to_string(),
        "scroll_thumb" => s.scroll_thumb.to_string(),
        "volume_meter" => match METER_PRESETS.iter().find(|(_, m)| *m == s.volume_meter) {
            Some((name, _)) => (*name).to_string(),
            None => match s.volume_meter {
                Meter::Ramp(r) => r.iter().collect(),
                Meter::Pair(a, b) => format!("{a}{b}"),
            },
        },
        "visualizer" => match VISUALIZER_PRESETS.iter().find(|(_, r)| *r == s.visualizer) {
            Some((name, _)) => (*name).to_string(),
            None => s.visualizer.iter().collect(),
        },
        _ => return None,
    })
}

/// Chrome rows that do not depend on the gap; compact `none` keeps one for the track.
pub fn fixed_block_rows(compact: bool, style: BorderStyle) -> u16 {
    let boxed = style.boxed();
    let blocks: u16 = if compact { 2 } else { 3 };
    let header = match (compact, boxed) {
        (true, _) => 0,
        (false, true) => 4,
        (false, false) => 2,
    };
    let tabs_and_progress = if boxed { 6 } else { 2 };
    let rules = if style == BorderStyle::Lines { blocks } else { 0 };
    let np_row = u16::from(compact && style == BorderStyle::None);
    header + tabs_and_progress + rules + np_row
}

pub fn block_gaps(compact: bool) -> u16 {
    if compact {
        2
    } else {
        3
    }
}

/// The largest gap up to `block_gap` that leaves the content six rows.
pub fn effective_block_gap(height: u16, block_gap: u16, fixed: u16, gaps: u16) -> u16 {
    let room = height.saturating_sub(fixed + 6) / gaps.max(1);
    block_gap.min(room)
}

static THEME: RwLock<Theme> = RwLock::new(DEFAULT);

pub fn theme() -> Theme {
    *THEME.read().expect("theme lock")
}

pub fn set_theme(theme: Theme) {
    *THEME.write().expect("theme lock") = theme;
}

/// Held by a test that sets the palette or checks a colour; survives poisoning.
#[cfg(test)]
pub(crate) fn palette_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The chosen theme without overrides; an unknown name is the default, and a problem.
pub fn base_appearance(cfg: &ThemeConfig) -> (Appearance, Vec<String>) {
    let (found, mut problems) = crate::ui::themes::resolve_base(&cfg.name, cfg.dir.as_deref());
    match found {
        Some(appearance) => (appearance, problems),
        None => {
            problems.push(format!(
                "theme.name = {:?} is not a known theme; using the default",
                cfg.name
            ));
            (Appearance::default(), problems)
        }
    }
}

/// The chosen theme with `[theme]` overrides; a bad one is the theme's, and a problem.
pub fn appearance_from_config(cfg: &ThemeConfig) -> (Appearance, Vec<String>) {
    let (base, problems) = base_appearance(cfg);
    let (theme, more) = with_overrides(base.theme, cfg);
    let (look, most) = look_with_overrides(base.look, cfg);
    (Appearance { theme, look }, [problems, more, most].concat())
}

pub fn from_config(cfg: &ThemeConfig) -> (Theme, Vec<String>) {
    let (appearance, problems) = appearance_from_config(cfg);
    (appearance.theme, problems)
}

/// The picker uses it to preview a theme under the user's own colours.
pub fn with_overrides(base: Theme, cfg: &ThemeConfig) -> (Theme, Vec<String>) {
    let mut theme = base;
    let mut problems = Vec::new();
    for role in ThemeConfig::ROLES {
        let Some(value) = cfg.colour(role) else {
            continue;
        };
        if !set_role(&mut theme, role, value) {
            problems.push(format!(
                "theme.{role} = {value:?} is not a #rrggbb colour; using the theme's"
            ));
        }
    }
    (theme, problems)
}

/// A value that does not fit is the theme's; the problem names the key, never the value.
pub fn look_with_overrides(base: Look, cfg: &ThemeConfig) -> (Look, Vec<String>) {
    let mut look = base;
    let mut problems = Vec::new();
    for key in ThemeConfig::SPACING {
        if let Some(value) = cfg.spacing(key) {
            if !set_spacing(&mut look, key, value) {
                problems.push(format!(
                    "theme.{key} must be a whole number from 0 to {}; using the theme's",
                    Spacing::MAX
                ));
            }
        }
    }
    for key in ThemeConfig::SPACING_TEXT {
        if let Some(text) = cfg.symbol(key) {
            if !set_spacing_text(&mut look, key, text) {
                problems.push(format!("theme.{key} {}; using the theme's", spacing_text_rule(key)));
            }
        }
    }
    for key in ThemeConfig::SYMBOLS {
        if let Some(text) = cfg.symbol(key) {
            if !set_symbol(&mut look, key, text) {
                problems.push(format!("theme.{key} {}; using the theme's", symbol_rule(key)));
            }
        }
    }
    (look, problems)
}

/// The eleven always-set colours; `background` is optional and not reached here.
pub(crate) fn role_slot<'a>(theme: &'a mut Theme, role: &str) -> Option<&'a mut Color> {
    Some(match role {
        "accent" => &mut theme.accent,
        "on_accent" => &mut theme.on_accent,
        "text" => &mut theme.text,
        "dim" => &mut theme.dim,
        "secondary" => &mut theme.secondary,
        "error" => &mut theme.error,
        "border" => &mut theme.border,
        "highlight" => &mut theme.highlight,
        "viz_bottom" => &mut theme.viz_bottom,
        "viz_mid" => &mut theme.viz_mid,
        "viz_top" => &mut theme.viz_top,
        _ => return None,
    })
}

/// `#rrggbb`, or `none` for `background`; false, theme untouched, when it does not fit.
pub(crate) fn set_role(theme: &mut Theme, role: &str, value: &str) -> bool {
    if role == "background" {
        if value.trim().eq_ignore_ascii_case("none") {
            theme.background = None;
            return true;
        }
        return match parse_hex(value) {
            Some(colour) => {
                theme.background = Some(colour);
                true
            }
            None => false,
        };
    }
    match (parse_hex(value), role_slot(theme, role)) {
        (Some(colour), Some(slot)) => {
            *slot = colour;
            true
        }
        _ => false,
    }
}

/// `None` for no such role and for an unset `background`.
pub fn role_colour(theme: &Theme, role: &str) -> Option<Color> {
    if role == "background" {
        return theme.background;
    }
    let mut copy = *theme;
    role_slot(&mut copy, role).map(|c| *c)
}

/// `#rrggbb`, or `none` for no background.
pub fn role_text(theme: &Theme, role: &str) -> Option<String> {
    match role {
        "background" => Some(theme.background.map_or_else(|| "none".into(), hex)),
        _ => role_colour(theme, role).map(hex),
    }
}

pub fn hex(colour: Color) -> String {
    match colour {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => other.to_string(),
    }
}

/// First in a frame; sets only `bg`, so a widget's own bg and the cover stay.
pub fn paint_background(buf: &mut ratatui::buffer::Buffer, area: ratatui::layout::Rect) {
    if let Some(bg) = theme().background {
        buf.set_style(area, Style::default().bg(bg));
    }
}

pub fn accent() -> Style {
    styled(Element::Accent, Style::default().fg(theme().accent))
}

pub fn accent_color() -> Color {
    theme().accent
}

pub fn on_accent() -> Style {
    Style::default().fg(theme().on_accent).add_modifier(Modifier::BOLD)
}

fn styled(element: Element, base: Style) -> Style {
    match look().styles.get(element) {
        Some(patch) => patch.apply(base, &theme()),
        None => base,
    }
}

pub fn secondary() -> Style {
    styled(Element::Secondary, Style::default().fg(theme().secondary))
}

pub fn text() -> Style {
    styled(Element::Text, Style::default().fg(theme().text))
}

pub fn dim() -> Style {
    styled(Element::Dim, Style::default().fg(theme().dim))
}

pub fn border() -> Style {
    styled(Element::Border, Style::default().fg(theme().border))
}

pub fn current() -> Style {
    styled(Element::Current, current_base())
}

fn current_base() -> Style {
    Style::default().fg(theme().on_accent).bg(theme().accent).add_modifier(Modifier::BOLD)
}

pub fn current_unfocused() -> Style {
    styled(Element::CurrentUnfocused, Style::default().fg(theme().text).bg(theme().highlight))
}

pub fn tab_active() -> Style {
    styled(Element::TabActive, current_base())
}

pub fn tab_inactive() -> Style {
    styled(Element::TabInactive, Style::default().fg(theme().text))
}

pub fn progress_fill() -> Style {
    styled(Element::ProgressFill, Style::default().fg(theme().accent))
}

/// A solid track defaults to the border colour, a glyph track to the dim one.
pub fn progress_track(solid: bool) -> Style {
    let base =
        if solid { border().fg.unwrap_or(theme().border) } else { dim().fg.unwrap_or(theme().dim) };
    styled(Element::ProgressTrack, Style::default().fg(base))
}

pub fn error() -> Style {
    styled(Element::Error, Style::default().fg(theme().error))
}

pub fn warn() -> Style {
    styled(Element::Warn, Style::default().fg(theme().secondary))
}

/// The leading `#` is optional.
pub fn parse_hex(value: &str) -> Option<Color> {
    let hex = value.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// By absolute row, so only tall bars reach the top colour.
pub fn viz_color(t_from_bottom: f32) -> Color {
    let t = t_from_bottom.clamp(0.0, 1.0);
    let palette = theme();
    let (from, to, f) = if t <= 0.5 {
        (palette.viz_bottom, palette.viz_mid, t / 0.5)
    } else {
        (palette.viz_mid, palette.viz_top, (t - 0.5) / 0.5)
    };
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (from, to) else {
        return to;
    };
    Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
}

pub fn format_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--:--".into();
    }
    let s = secs as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// Fits `59:59`; a table widens it to its longest time.
pub const LEN_COL_MIN: u16 = 5;

/// Width of a right-aligned row number column.
pub fn digits(n: usize) -> u16 {
    n.max(1).to_string().len() as u16
}

/// VS16 (U+FE0F) is zero-width and makes its base a 2-cell emoji.
fn char_width(c: char, next: Option<char>) -> usize {
    if c == '\u{fe0f}' {
        return 0;
    }
    let base = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    if next == Some('\u{fe0f}') {
        base.max(2)
    } else {
        base
    }
}

pub fn str_width(s: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().map(|(i, &c)| char_width(c, chars.get(i + 1).copied())).sum()
}

/// Truncate to `w` columns with the ellipsis, never splitting off a VS16.
pub fn ellipsize(s: &str, w: u16) -> String {
    ellipsize_with(s, w, ellipsis())
}

pub fn ellipsize_with(s: &str, w: u16, ellipsis: char) -> String {
    let w = w as usize;
    if str_width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    if w == 1 {
        return ellipsis.into();
    }
    let target = w - 1;
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut acc = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let cw = char_width(chars[i], chars.get(i + 1).copied());
        if acc + cw > target {
            break;
        }
        out.push(chars[i]);
        acc += cw;
        i += 1;
    }
    out.push(ellipsis);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_ascii() {
        assert_eq!(ellipsize("hello", 10), "hello");
        assert_eq!(ellipsize("hello", 5), "hello");
        assert_eq!(ellipsize("hello", 4), "hel…");
        assert_eq!(ellipsize("hello", 1), "…");
        assert_eq!(ellipsize("hello", 0), "");
    }

    #[test]
    fn ellipsize_cjk_counts_double() {
        // デスロウ is 4 wide chars = 8 columns
        assert_eq!(ellipsize("デスロウ", 8), "デスロウ");
        assert_eq!(ellipsize("デスロウ", 5), "デス…");
        assert_eq!(ellipsize("デスロウx", 6), "デス…");
    }

    #[test]
    fn format_time_units() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(42.0), "0:42");
        assert_eq!(format_time(257.0), "4:17");
        assert_eq!(format_time(3599.0), "59:59");
        assert_eq!(format_time(3600.0), "1:00:00");
        assert_eq!(format_time(3725.0), "1:02:05");
        assert_eq!(format_time(-1.0), "--:--");
        assert_eq!(format_time(f64::NAN), "--:--");
    }

    #[test]
    fn str_width_handles_emoji_and_vs16() {
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("デス"), 4);
        assert_eq!(str_width("\u{263a}\u{fe0f}"), 2);
        assert_eq!(str_width("\u{263a}"), 1);
        assert_eq!(str_width("a\u{263a}\u{fe0f}b"), 4);
    }

    #[test]
    fn ellipsize_emoji_keeps_vs16_attached() {
        // 3 columns: "😀" (2) + ellipsis (1)
        assert_eq!(ellipsize("😀abc", 3), "😀…");
        assert_eq!(ellipsize("\u{263a}\u{fe0f}xy", 3), "\u{263a}\u{fe0f}…");
    }

    #[test]
    fn row_number_width_grows_9_10_100() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(99), 2);
        assert_eq!(digits(100), 3);
    }

    /// Run `f` under another palette; the caller holds `palette_test_lock()`.
    fn with_palette<T>(palette: Theme, f: impl FnOnce() -> T) -> T {
        let previous = theme();
        set_theme(palette);
        let out = f();
        set_theme(previous);
        out
    }

    #[test]
    fn override_beats_preset() {
        let mocha = crate::ui::themes::preset("catppuccin-mocha").unwrap().theme;
        let cfg = ThemeConfig {
            name: "catppuccin-mocha".into(),
            accent: Some("#123456".into()),
            text: Some("#12345".into()),
            ..ThemeConfig::default()
        };
        let (theme, problems) = from_config(&cfg);
        assert_eq!(theme.accent, Color::Rgb(0x12, 0x34, 0x56));
        // A typo falls back to the theme's colour, not the default's.
        assert_eq!(theme.text, mocha.text);
        assert_eq!(theme.secondary, mocha.secondary);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("theme.text"), "{problems:?}");

        let cfg = ThemeConfig { name: "nope".into(), ..ThemeConfig::default() };
        let (theme, problems) = from_config(&cfg);
        assert_eq!(theme, DEFAULT);
        assert!(problems[0].contains("theme.name"), "{problems:?}");
    }

    #[test]
    fn background_none_clears_override() {
        let nord = crate::ui::themes::preset("nord").unwrap().theme;
        let with = |name: &str, bg: Option<&str>| {
            from_config(&ThemeConfig {
                name: name.into(),
                background: bg.map(Into::into),
                ..ThemeConfig::default()
            })
        };
        assert_eq!(with("default", None).0.background, None, "the default has none");
        assert_eq!(with("nord", None).0.background, nord.background, "the theme's");
        let (t, problems) = with("nord", Some("#010203"));
        assert_eq!(t.background, Some(Color::Rgb(1, 2, 3)), "an override beats the theme");
        assert!(problems.is_empty(), "{problems:?}");
        let (t, problems) = with("nord", Some("none"));
        assert_eq!(t.background, None, "none clears the preset's");
        assert!(problems.is_empty(), "{problems:?}");
        let (t, problems) = with("nord", Some("mauve"));
        assert_eq!(t.background, nord.background, "a bad one is the theme's");
        assert!(problems.len() == 1 && problems[0].contains("theme.background"), "{problems:?}");
    }

    #[test]
    fn visualizer_ramp_runs_bottom_to_top() {
        let _palette = palette_test_lock();
        with_palette(DEFAULT, || {
            assert_eq!(viz_color(0.0), DEFAULT.viz_bottom);
            assert_eq!(viz_color(0.5), DEFAULT.viz_mid);
            assert_eq!(viz_color(1.0), DEFAULT.viz_top);
            let quarter = viz_color(0.25);
            assert_ne!(quarter, DEFAULT.viz_bottom);
            assert_ne!(quarter, DEFAULT.viz_mid);
            // 0.15 is 30% of the way from the bottom colour to the middle one.
            assert_eq!(viz_color(0.15), Color::Rgb(0xe7, 0x46, 0xa1));
        });
    }

    #[test]
    fn colour_parses_with_or_without_hash() {
        assert_eq!(parse_hex("#7aa2f7"), Some(Color::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(parse_hex("7aa2f7"), Some(Color::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(parse_hex("  #7aa2f7 "), Some(Color::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(parse_hex("##7aa2f7"), None);
        assert_eq!(parse_hex("#7aa2f"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(parse_hex(""), None);
    }

    /// Run `f` under another look; the caller holds `palette_test_lock()`.
    fn with_look<T>(look: Look, f: impl FnOnce() -> T) -> T {
        let previous = self::look();
        set_look(look);
        let out = f();
        set_look(previous);
        out
    }

    /// By display width, not length.
    #[test]
    fn symbol_lengths_are_checked() {
        for good in ["x", "…", "▶", "█", " ", "·"] {
            assert!(parse_glyph(good).is_some(), "{good:?}");
        }
        for bad in
            ["", "ab", "界", "😀", "\t", "\u{1b}", "\u{200b}", "e\u{301}", "\u{263a}\u{fe0f}"]
        {
            assert!(parse_glyph(bad).is_none(), "{bad:?}");
        }
        assert_eq!(parse_ramp("▁▂▃▄▅▆▇█"), Some(EIGHTHS));
        assert!(parse_ramp("▁▂▃▄▅▆▇").is_none(), "seven");
        assert!(parse_ramp("▁▂▃▄▅▆▇██").is_none(), "nine");
        assert!(parse_ramp("▁▂▃▄▅▆▇界").is_none(), "one wide glyph spoils it");
        assert_eq!(parse_meter("▁▂▃▄▅▆▇█"), Some(Meter::Ramp(EIGHTHS)));
        assert_eq!(parse_meter("■·"), Some(Meter::Pair('■', '·')));
        for bad in ["", "■", "■··", "■界", "abcdefg"] {
            assert!(parse_meter(bad).is_none(), "{bad:?}");
        }
        let mut look = Look::DEFAULT;
        assert!(!set_symbol(&mut look, "visualizer", "■·"));
        assert!(!set_symbol(&mut look, "ellipsis", "界"));
        assert_eq!(look, Look::DEFAULT, "a refused value changes nothing");
    }

    #[test]
    fn ellipsize_uses_active_glyph() {
        let _palette = palette_test_lock();
        let mut look = Look::DEFAULT;
        assert!(set_symbol(&mut look, "ellipsis", "~"));
        with_look(look, || {
            assert_eq!(ellipsize("hello", 4), "hel~");
            assert_eq!(ellipsize("hello", 1), "~");
            assert_eq!(ellipsize("デスロウx", 6), "デス~", "wide text still counts two");
        });
        assert_eq!(ellipsize("hello", 4), "hel…", "the default is back");
    }

    /// `solid` beats a theme's glyph, from the config and from a theme file.
    #[test]
    fn solid_overrides_a_themes_progress_glyph() {
        let mut base = Look::DEFAULT;
        for key in ["progress_filled", "progress_empty", "progress_head"] {
            assert!(set_symbol(&mut base, key, "#"));
        }
        let cfg = ThemeConfig {
            progress_empty: Some("solid".into()),
            progress_head: Some("solid".into()),
            ..ThemeConfig::default()
        };
        let (look, problems) = look_with_overrides(base, &cfg);
        assert!(problems.is_empty(), "{problems:?}");
        let s = look.symbols;
        assert_eq!((s.progress_filled, s.progress_empty, s.progress_head), (Some('#'), None, None));
        assert_eq!(look_text(&look, "progress_head").as_deref(), Some("solid"));

        let mut file = Look::DEFAULT;
        assert!(set_symbol(&mut file, "progress_filled", "solid"), "a theme file may say it too");
        assert!(!set_symbol(&mut file, "ellipsis", "solid"), "only the progress glyphs take it");
    }

    /// A preset sets all five slots; a glyph set after it replaces its part only.
    #[test]
    fn part_after_preset_wins() {
        for (name, glyphs) in PROGRESS_PRESETS {
            let mut look = Look::DEFAULT;
            assert!(set_symbol(&mut look, "progress_bar", name), "{name}");
            assert_eq!(look.symbols.progress(), *glyphs, "{name}");
            assert_eq!(look_text(&look, "progress_bar").as_deref(), Some(*name));
            for c in glyphs.iter().flatten() {
                assert!(parse_glyph(&c.to_string()).is_some(), "{name}: {c:?} is not one column");
            }
            if *name != "solid" {
                assert!(!set_symbol(&mut look, "progress_head", name), "{name}");
            }
        }
        let rounded = PROGRESS_PRESETS.iter().find(|(n, _)| *n == "rounded").unwrap().1;
        assert_eq!(rounded, [None, None, None, Some('\u{e0b6}'), Some('\u{e0b4}')]);
        let mut look = Look::DEFAULT;
        for bad in ["", "=", "━", "Line", "line "] {
            assert!(!set_symbol(&mut look, "progress_bar", bad), "{bad:?}");
        }
        assert_eq!(look, Look::DEFAULT, "a refused value changes nothing");

        let cfg = ThemeConfig {
            progress_head: Some("solid".into()),
            progress_end: Some("]".into()),
            progress_bar: Some("dot".into()),
            ..ThemeConfig::default()
        };
        let (look, problems) = look_with_overrides(Look::DEFAULT, &cfg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(look.symbols.progress(), [Some('━'), Some('─'), None, None, Some(']')]);
        assert_eq!(look_text(&look, "progress_bar").as_deref(), Some("custom"));
    }

    /// `ui.icons = "ascii"`: a preset with a glyph that is not ASCII is solid.
    #[test]
    fn ascii_mode_makes_progress_presets_solid() {
        for (name, glyphs) in PROGRESS_PRESETS {
            let mut look = Look::DEFAULT;
            assert!(set_symbol(&mut look, "progress_bar", name));
            let want =
                if glyphs.iter().flatten().all(char::is_ascii) { *glyphs } else { [None; 5] };
            assert_eq!(look.for_ascii().symbols.progress(), want, "{name}");
        }
    }

    /// `ui.icons = "ascii"`: a theme's non-ASCII symbols give way to the defaults.
    #[test]
    fn ascii_mode_drops_only_non_ascii_symbols() {
        let _palette = palette_test_lock();
        let mut look = Look::DEFAULT;
        for (key, value) in [
            ("ellipsis", "~"),
            ("scroll_up", "↑"),
            ("progress_filled", "#"),
            ("progress_empty", "░"),
            ("volume_meter", "[]"),
            ("visualizer", "12345678"),
        ] {
            assert!(set_symbol(&mut look, key, value), "{key}");
        }
        let kept = look.for_ascii().symbols;
        assert_eq!(kept.ellipsis, '~');
        assert_eq!(kept.scroll_up, Symbols::DEFAULT.scroll_up, "↑ is not ASCII");
        assert_eq!(kept.progress_filled, Some('#'));
        assert_eq!(kept.progress_empty, None, "░ is not ASCII");
        assert_eq!(kept.volume_meter, Meter::Pair('[', ']'));
        assert_eq!(kept.visualizer, ['1', '2', '3', '4', '5', '6', '7', '8']);

        let before = (theme(), self::look());
        set_appearance(Appearance { theme: DEFAULT, look }, true);
        assert_eq!(symbols().scroll_up, Symbols::DEFAULT.scroll_up);
        assert_eq!(ellipsis(), '~');
        set_appearance(Appearance { theme: DEFAULT, look }, false);
        assert_eq!(symbols().scroll_up, '↑');
        set_theme(before.0);
        set_look(before.1);
    }

    /// Override beats theme beats default; a bad value is the theme's, named by key.
    #[test]
    fn override_beats_theme_beats_default() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("mine.toml"),
            "[spacing]\nblock_gap = 2\ninset = 3\n[symbols]\nellipsis = \"~\"\nscroll_up = \"^\"\n",
        )
        .unwrap();
        let mut cfg = ThemeConfig {
            name: "mine".into(),
            dir: Some(tmp.path().to_path_buf()),
            inset: Some(0),
            scroll_up: Some("A".into()),
            ..ThemeConfig::default()
        };
        let (appearance, problems) = appearance_from_config(&cfg);
        assert!(problems.is_empty(), "{problems:?}");
        let look = appearance.look;
        assert_eq!(
            look.spacing,
            Spacing { block_gap: 2, inset: 0, ..Spacing::DEFAULT },
            "inset: override; gap: theme"
        );
        assert_eq!(look.symbols.scroll_up, 'A', "override over the theme's ^");
        assert_eq!(look.symbols.ellipsis, '~', "the theme's");
        assert_eq!(look.symbols.scroll_down, Symbols::DEFAULT.scroll_down, "the default");

        cfg.block_gap = Some(7);
        cfg.scroll_up = Some("SECRET界".into());
        let (appearance, problems) = appearance_from_config(&cfg);
        assert_eq!(appearance.look.spacing.block_gap, 2, "a bad override is the theme's");
        assert_eq!(appearance.look.symbols.scroll_up, '^');
        let text = problems.join("\n");
        assert!(text.contains("theme.block_gap") && text.contains("theme.scroll_up"), "{text}");
        assert!(!text.contains("SECRET") && !text.contains('7'), "keys only: {text}");
    }

    /// Matched without case; a bad word is named by key and leaves the theme's.
    #[test]
    fn bad_border_word_is_named() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("mine.toml"), "[spacing]\nborders = \"rounded\"\n").unwrap();
        let mut cfg = ThemeConfig {
            name: "mine".into(),
            dir: Some(tmp.path().to_path_buf()),
            ..ThemeConfig::default()
        };
        let (appearance, problems) = appearance_from_config(&cfg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(appearance.look.spacing.borders, BorderStyle::Rounded, "the theme's");

        cfg.borders = Some("LINES".into());
        assert_eq!(
            appearance_from_config(&cfg).0.look.spacing.borders,
            BorderStyle::Lines,
            "override over theme"
        );

        cfg.borders = Some("SECRETframes".into());
        let (appearance, problems) = appearance_from_config(&cfg);
        assert_eq!(
            appearance.look.spacing.borders,
            BorderStyle::Rounded,
            "a bad override is the theme's"
        );
        let text = problems.join("\n");
        assert!(text.contains("theme.borders") && !text.contains("SECRET"), "{text}");
    }

    #[test]
    fn block_gap_yields_before_content_loses_rows() {
        // 10 rows are header, tab bar and progress; the content keeps 6.
        for (height, set, got) in [
            (30, 3, 3),
            (25, 3, 3),
            (24, 3, 2),
            (19, 3, 1),
            (18, 3, 0),
            (10, 3, 0),
            (60, 1, 1),
            (30, 0, 0),
        ] {
            assert_eq!(effective_block_gap(height, set, 10, 3), got, "{height} rows, gap {set}");
            let used = 10 + 3 * got;
            assert!(height < 16 || height - used >= 6, "{height}");
        }
    }

    #[test]
    fn fixed_rows_follow_header_and_borders() {
        let rows = |compact, style| fixed_block_rows(compact, style);
        let got = [
            BorderStyle::Box,
            BorderStyle::Rounded,
            BorderStyle::Double,
            BorderStyle::Lines,
            BorderStyle::None,
        ]
        .map(|s| (rows(false, s), rows(true, s)));
        assert_eq!(got, [(10, 6), (10, 6), (10, 6), (7, 4), (4, 3)]);
        assert_eq!(
            effective_block_gap(20, 3, 4, 3),
            3,
            "a borderless layout has room for more gap"
        );
    }

    #[test]
    fn preset_names_map_to_glyphs() {
        let meter = |text: &str| parse_meter(text);
        assert_eq!(meter("blocks"), Some(Meter::Ramp(EIGHTHS)));
        assert_eq!(meter("squares"), Some(Meter::Pair('■', '·')));
        assert_eq!(meter("outline"), Some(Meter::Pair('■', '□')));
        assert_eq!(meter("plain"), Some(Meter::Pair('#', '-')));
        assert_eq!(parse_visualizer("blocks"), Some(EIGHTHS));
        assert_eq!(parse_visualizer("ascii"), Some(['_', '.', '-', '=', '+', '*', '#', '@']));
        assert_eq!(meter("■·"), Some(Meter::Pair('■', '·')));
        assert_eq!(meter("12345678"), Some(Meter::Ramp(['1', '2', '3', '4', '5', '6', '7', '8'])));
        assert_eq!(parse_visualizer("12345678").map(|r| r[7]), Some('8'));
        assert!(meter("square").is_none() && parse_visualizer("Ascii").is_none(), "no near misses");
        for (name, m) in METER_PRESETS {
            let glyphs: Vec<char> = match m {
                Meter::Ramp(r) => r.to_vec(),
                Meter::Pair(a, b) => vec![*a, *b],
            };
            assert!(glyphs.iter().all(|c| parse_glyph(&c.to_string()).is_some()), "{name}");
        }
        for (name, r) in VISUALIZER_PRESETS {
            assert!(r.iter().all(|c| parse_glyph(&c.to_string()).is_some()), "{name}");
        }
        let mut look = Look::DEFAULT;
        assert_eq!(look_text(&look, "volume_meter").as_deref(), Some("blocks"));
        assert!(set_symbol(&mut look, "volume_meter", "■·"));
        assert_eq!(look_text(&look, "volume_meter").as_deref(), Some("squares"));
        assert!(set_symbol(&mut look, "volume_meter", "<>"));
        assert_eq!(look_text(&look, "volume_meter").as_deref(), Some("<>"));
    }

    /// A preset name is neither a valid literal nor the length of one.
    #[test]
    fn preset_names_never_read_as_glyphs() {
        let names =
            METER_PRESETS.iter().map(|(n, _)| *n).chain(VISUALIZER_PRESETS.iter().map(|(n, _)| *n));
        for name in names {
            let n = name.chars().count();
            assert!(n != 2 && n != 8, "{name} is the length of a literal");
            assert!(parse_ramp(name).is_none(), "{name}");
            let mut chars = name.chars();
            let pair = chars.next().zip(chars.next()).filter(|_| n == 2);
            assert!(pair.is_none(), "{name}");
        }
    }

    /// `ui.icons = "ascii"` treats a preset like its glyphs.
    #[test]
    fn ascii_mode_replaces_non_ascii_presets_only() {
        let looked = |meter: &str, viz: &str| {
            let mut look = Look::DEFAULT;
            assert!(set_symbol(&mut look, "volume_meter", meter));
            assert!(set_symbol(&mut look, "visualizer", viz));
            look.for_ascii().symbols
        };
        for meter in ["squares", "outline"] {
            assert_eq!(
                looked(meter, "ascii").volume_meter,
                Symbols::DEFAULT.volume_meter,
                "{meter}"
            );
        }
        let kept = looked("plain", "ascii");
        assert_eq!(kept.volume_meter, Meter::Pair('#', '-'));
        assert_eq!(kept.visualizer, VISUALIZER_PRESETS[1].1);
        assert_eq!(looked("blocks", "blocks").visualizer, EIGHTHS);
    }

    fn patch(
        fg: Option<ColorRef>,
        bg: Option<ColorRef>,
        add: Modifier,
        remove: Modifier,
    ) -> StylePatch {
        StylePatch { fg, bg, add, remove }
    }

    fn styles_of(element: Element, p: StylePatch) -> Look {
        let mut look = Look::DEFAULT;
        look.styles.set(element, p);
        look
    }

    /// A patch changes only the fields it lists; roles are read at call time.
    #[test]
    fn style_patch_changes_only_listed_fields() {
        let _palette = palette_test_lock();
        let before = theme();
        set_theme(DEFAULT);
        let shipped = current();
        assert_eq!(shipped.bg, Some(DEFAULT.accent));
        assert_eq!(shipped.fg, Some(DEFAULT.on_accent));
        assert!(shipped.add_modifier.contains(Modifier::BOLD));
        let none = Modifier::empty();

        let italic = with_look(
            styles_of(Element::Current, patch(None, None, Modifier::ITALIC, none)),
            current,
        );
        assert_eq!((italic.fg, italic.bg), (shipped.fg, shipped.bg), "colours stay");
        assert!(
            italic.add_modifier.contains(Modifier::BOLD | Modifier::ITALIC),
            "bold stays, italic added"
        );

        let unbold = with_look(
            styles_of(Element::Current, patch(None, None, none, Modifier::BOLD)),
            current,
        );
        assert!(
            !unbold.add_modifier.contains(Modifier::BOLD)
                && unbold.sub_modifier.contains(Modifier::BOLD)
        );
        assert_eq!((unbold.fg, unbold.bg), (shipped.fg, shipped.bg));

        let secondary_fg = patch(Some(ColorRef::Role(Role::Secondary)), None, none, none);
        let recoloured = with_look(styles_of(Element::Current, secondary_fg), current);
        assert_eq!(recoloured.fg, Some(DEFAULT.secondary));
        assert_eq!(recoloured.bg, shipped.bg, "only the foreground");
        assert_eq!(recoloured.add_modifier, shipped.add_modifier);

        let other = Theme { secondary: Color::Rgb(1, 2, 3), ..DEFAULT };
        let fixed = patch(Some(ColorRef::Rgb(9, 9, 9)), None, none, none);
        with_look(styles_of(Element::Current, secondary_fg), || {
            set_theme(other);
            assert_eq!(
                current().fg,
                Some(Color::Rgb(1, 2, 3)),
                "a role follows the palette now in force"
            );
        });
        with_look(styles_of(Element::Current, fixed), || {
            assert_eq!(current().fg, Some(Color::Rgb(9, 9, 9)), "#rrggbb is fixed");
            assert_eq!(tab_active(), current_base(), "tab_active is its own element");
            assert_eq!(dim(), Style::default().fg(other.dim), "an unlisted element is today's");
        });
        set_theme(before);
    }
}
