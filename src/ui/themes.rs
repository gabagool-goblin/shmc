//! The built-in themes are the files in `themes/`, embedded at build time and
//! inspired by well-known palettes (credited in docs/themes.md); `default` is
//! in code. `[theme]` colours override them.

use crate::ui::theme::{self, Appearance, Look, Theme};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

/// In the order `presets()` lists them, after `default`.
const BUILTIN: &[(&str, &str)] = &[
    ("tokyo-night", include_str!("../../themes/tokyo-night.toml")),
    ("catppuccin-mocha", include_str!("../../themes/catppuccin-mocha.toml")),
    ("catppuccin-latte", include_str!("../../themes/catppuccin-latte.toml")),
    ("gruvbox-dark", include_str!("../../themes/gruvbox-dark.toml")),
    ("nord", include_str!("../../themes/nord.toml")),
    ("dracula", include_str!("../../themes/dracula.toml")),
];

/// `default`, then the embedded files. One that does not parse is left out; a
/// test keeps them clean.
pub fn presets() -> &'static [Entry] {
    static PRESETS: OnceLock<Vec<Entry>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        let default = Entry {
            id: "default".to_string(),
            title: "shmc default".to_string(),
            author: "shmc".to_string(),
            variant: Some(Variant::Dark),
            source: Source::Preset,
            theme: theme::DEFAULT,
            look: Look::DEFAULT,
        };
        let files = BUILTIN
            .iter()
            .filter_map(|(id, text)| parse(id, text, Source::Preset).map(|(entry, _)| entry));
        std::iter::once(default).chain(files).collect()
    })
}

pub fn preset(name: &str) -> Option<&'static Entry> {
    presets().iter().find(|p| p.id == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Preset,
    File,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub author: String,
    pub variant: Option<Variant>,
    pub source: Source,
    pub theme: Theme,
    pub look: Look,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Variant::Dark => "dark",
            Variant::Light => "light",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Available {
    pub entries: Vec<Entry>,
    pub problems: Vec<String>,
}

/// A theme is a few dozen lines; anything bigger is not one.
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_FILES: usize = 256;

/// `[a-z0-9][a-z0-9_-]*`. Checked before an id is joined onto a path, so a
/// `theme.name` such as `../x` never reaches the filesystem.
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

enum Load {
    Missing,
    /// The reason names the id, never the contents.
    Skipped(String),
    Loaded(Box<Entry>, Vec<String>),
}

fn load_file(dir: &Path, id: &str) -> Load {
    use std::io::Read;
    let path = dir.join(format!("{id}.toml"));
    // Not a symlink (it could point anywhere), nor a FIFO or device (opening
    // one can block).
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Load::Missing,
        Err(_) => return Load::Skipped(format!("theme file {id}: could not be read")),
    };
    if meta.file_type().is_symlink() {
        return Load::Skipped(format!("theme file {id}: a symlink, skipped"));
    }
    if !meta.is_file() {
        return Load::Skipped(format!("theme file {id}: not a regular file, skipped"));
    }
    let too_big = || Load::Skipped(format!("theme file {id}: over 64 KiB, skipped"));
    if meta.len() > MAX_FILE_BYTES {
        return too_big();
    }
    let mut bytes = Vec::new();
    let read =
        std::fs::File::open(&path).and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes));
    if read.is_err() {
        return Load::Skipped(format!("theme file {id}: could not be read"));
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return too_big();
    }
    let parsed = String::from_utf8(bytes).ok().and_then(|text| parse(id, &text, Source::File));
    match parsed {
        Some((entry, problems)) => Load::Loaded(Box::new(entry), problems),
        None => Load::Skipped(format!("theme file {id}: not valid TOML, skipped")),
    }
}

/// `None` when `raw` is not TOML. Everything else is lenient: a bad value is
/// a problem and the default's.
fn parse(id: &str, raw: &str, source: Source) -> Option<(Entry, Vec<String>)> {
    let table = raw.parse::<toml::Table>().ok()?;

    let text = |key: &str| {
        table
            .get(key)
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    // A missing or invalid colour is the default theme's; an invalid one is reported.
    let mut theme = theme::DEFAULT;
    let mut problems = Vec::new();
    for role in crate::config::ThemeConfig::ROLES {
        let Some(value) = table.get(role) else {
            continue;
        };
        if !value.as_str().is_some_and(|v| theme::set_role(&mut theme, role, v)) {
            problems.push(format!(
                "theme file {id}: {role} is not a #rrggbb colour; using the default's"
            ));
        }
    }
    let mut look = Look::DEFAULT;
    // Unknown keys and tables are ignored so a file for a newer shmc still loads.
    let section = |name: &str| table.get(name).and_then(|v| v.as_table());
    if let Some(spacing) = section("spacing") {
        for key in crate::config::ThemeConfig::SPACING {
            let Some(value) = spacing.get(key) else {
                continue;
            };
            if !value.as_integer().is_some_and(|v| theme::set_spacing(&mut look, key, v)) {
                problems.push(format!(
                    "theme file {id}: spacing.{key} must be a whole number from 0 to {}; using the default's",
                    theme::Spacing::MAX
                ));
            }
        }
        for key in crate::config::ThemeConfig::SPACING_TEXT {
            let Some(value) = spacing.get(key) else {
                continue;
            };
            if !value.as_str().is_some_and(|v| theme::set_spacing_text(&mut look, key, v)) {
                problems.push(format!(
                    "theme file {id}: spacing.{key} {}; using the default's",
                    theme::spacing_text_rule(key)
                ));
            }
        }
    }
    if let Some(symbols) = section("symbols") {
        for key in crate::config::ThemeConfig::SYMBOLS {
            let Some(value) = symbols.get(key) else {
                continue;
            };
            if !value.as_str().is_some_and(|v| theme::set_symbol(&mut look, key, v)) {
                problems.push(format!(
                    "theme file {id}: symbols.{key} {}; using the default's",
                    theme::symbol_rule(key)
                ));
            }
        }
    }
    // Problems name the element and key, never a value.
    match table.get("styles") {
        None => {}
        Some(toml::Value::Table(styles)) => {
            for (name, entry) in styles {
                let Some((_, element)) = theme::Element::ALL.iter().find(|(n, _)| n == name) else {
                    problems.push(format!(
                        "theme file {id}: styles.{name:?} is not a known element; ignored"
                    ));
                    continue;
                };
                let Some(entry) = entry.as_table() else {
                    problems
                        .push(format!("theme file {id}: styles.{name} is not a table; ignored"));
                    continue;
                };
                let (patch, bad) = theme::StylePatch::parse(entry);
                for key in bad {
                    problems.push(format!(
                        "theme file {id}: styles.{name}: {key:?} is not a valid key or value; ignored"
                    ));
                }
                look.styles.set(*element, patch);
            }
        }
        Some(_) => problems.push(format!("theme file {id}: styles is not a table; ignored")),
    }
    let variant = match text("variant").as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("dark") => Some(Variant::Dark),
        Some("light") => Some(Variant::Light),
        _ => None,
    };
    let entry = Entry {
        id: id.to_string(),
        title: text("name").unwrap_or_else(|| id.to_string()),
        author: text("author").unwrap_or_default(),
        variant,
        source,
        theme,
        look,
    };
    Some((entry, problems))
}

/// The themes directory is looked in first, so a file named like a preset
/// replaces it. `None` when there is no such theme.
pub fn resolve_base(name: &str, dir: Option<&Path>) -> (Option<Appearance>, Vec<String>) {
    let mut problems = Vec::new();
    if let (Some(dir), true) = (dir, valid_id(name)) {
        match load_file(dir, name) {
            Load::Loaded(entry, found) => {
                return (Some(Appearance { theme: entry.theme, look: entry.look }), found);
            }
            Load::Skipped(why) => problems.push(why),
            Load::Missing => {}
        }
    }
    (preset(name).map(|p| Appearance { theme: p.theme, look: p.look }), problems)
}

/// The presets, then the files in `dir` by id; a file named like a preset takes
/// its place. Reads the directory: for the picker and `--check`, not drawing.
pub fn available(dir: Option<&Path>) -> Available {
    let mut out = Available { entries: presets().to_vec(), problems: Vec::new() };
    let Some(dir) = dir else {
        return out;
    };
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            out.problems.push(format!("themes: {} could not be read: {}", dir.display(), e.kind()));
            return out;
        }
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| !e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".toml"))
        .collect();
    names.sort();
    if names.len() > MAX_FILES {
        names.truncate(MAX_FILES);
        out.problems.push(format!("themes: only the first {MAX_FILES} files are read"));
    }
    for name in names {
        let id = name.strip_suffix(".toml").unwrap_or(&name);
        if !valid_id(id) {
            out.problems.push(format!(
                "themes: {name:?} ignored; a theme file is named like my-theme.toml (a-z, 0-9, - and _)"
            ));
            continue;
        }
        match load_file(dir, id) {
            Load::Loaded(entry, found) => {
                out.problems.extend(found);
                match out.entries.iter().position(|e| e.id == entry.id) {
                    Some(at) => out.entries[at] = *entry,
                    None => out.entries.push(*entry),
                }
            }
            Load::Skipped(why) => out.problems.push(why),
            Load::Missing => {}
        }
    }
    out
}

/// First line of a file shmc exported: the only kind it will overwrite.
pub const EXPORT_MARK: &str = "# exported by shmc";

/// Writes `themes/<id>.toml` via a temp file, 0644. Refuses a bad id, a
/// non-file, or a file shmc did not export.
pub fn export(
    dir: &Path,
    id: &str,
    variant: Option<Variant>,
    theme: &Theme,
    look: &Look,
) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    if !valid_id(id) {
        return Err(format!(
            "{id:?} is not a theme id; use a-z, 0-9, - and _, starting with a letter or digit"
        ));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("could not create the themes directory: {e}"))?;
    let dest = dir.join(format!("{id}.toml"));
    let foreign =
        || format!("{id}.toml exists and was not written by shmc export; not overwritten");
    match std::fs::symlink_metadata(&dest) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => return Err(foreign()),
        Ok(_) => {
            let mut head = Vec::new();
            std::fs::File::open(&dest)
                .and_then(|f| f.take(EXPORT_MARK.len() as u64 + 2).read_to_end(&mut head))
                .map_err(|e| format!("could not read {id}.toml: {e}"))?;
            let first = head.split(|b| *b == b'\n').next().unwrap_or(&[]);
            if first.strip_suffix(b"\r").unwrap_or(first) != EXPORT_MARK.as_bytes() {
                return Err(foreign());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("could not check {id}.toml: {e}")),
    }

    let quoted = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut body = format!("{EXPORT_MARK}\nname = {}\n", quoted(id));
    if let Some(variant) = variant {
        body.push_str(&format!("variant = \"{}\"\n", variant.name()));
    }
    for role in crate::config::ThemeConfig::ROLES {
        // No background is no line: a missing key is none.
        if let Some(colour) = theme::role_colour(theme, role) {
            body.push_str(&format!("{role} = \"{}\"\n", theme::hex(colour)));
        }
    }
    // Only what differs from the default.
    let differs = |key: &str| theme::look_text(look, key) != theme::look_text(&Look::DEFAULT, key);
    let spacing: Vec<&str> =
        crate::config::ThemeConfig::SPACING.into_iter().filter(|k| differs(k)).collect();
    let words: Vec<&str> =
        crate::config::ThemeConfig::SPACING_TEXT.into_iter().filter(|k| differs(k)).collect();
    if !spacing.is_empty() || !words.is_empty() {
        body.push_str("\n[spacing]\n");
        for key in spacing {
            body.push_str(&format!(
                "{key} = {}\n",
                theme::look_text(look, key).unwrap_or_default()
            ));
        }
        for key in words {
            body.push_str(&format!(
                "{key} = {}\n",
                quoted(&theme::look_text(look, key).unwrap_or_default())
            ));
        }
    }
    // The preset's name when the five glyphs are one, else the glyphs; never both.
    let named = theme::look_text(look, "progress_bar").is_some_and(|n| n != "custom");
    let symbols: Vec<&str> = crate::config::ThemeConfig::SYMBOLS
        .into_iter()
        .filter(|k| differs(k))
        .filter(|k| match *k {
            "progress_bar" => named,
            k if k.starts_with("progress_") => !named,
            _ => true,
        })
        .collect();
    if !symbols.is_empty() {
        body.push_str("\n[symbols]\n");
        for key in symbols {
            body.push_str(&format!(
                "{key} = {}\n",
                quoted(&theme::look_text(look, key).unwrap_or_default())
            ));
        }
    }

    if look.styles.count() > 0 {
        body.push_str("\n[styles]\n");
        for (name, element) in theme::Element::ALL {
            if let Some(patch) = look.styles.get(element) {
                body.push_str(&format!("{name} = {}\n", patch.text()));
            }
        }
    }

    let tmp = dir.join(format!(".{id}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o644);
    }
    let result = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(body.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, &dest));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not write {id}.toml: {e}"));
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn names_unique_first_is_default() {
        let ids: Vec<&str> = presets().iter().map(|p| p.id.as_str()).collect();
        // Theme names are config values: these must not change.
        assert_eq!(
            ids,
            [
                "default",
                "tokyo-night",
                "catppuccin-mocha",
                "catppuccin-latte",
                "gruvbox-dark",
                "nord",
                "dracula"
            ]
        );
        assert_eq!(presets()[0].theme, theme::DEFAULT);
        assert_eq!(theme::DEFAULT.background, None, "default draws on the terminal's own");
        let latte = preset("catppuccin-latte").unwrap();
        assert_eq!(latte.variant, Some(Variant::Light));
        for p in presets().iter().skip(1) {
            assert!(p.theme.background.is_some(), "{} carries its own background", p.id);
            assert!(p.variant.is_some(), "{} has a variant", p.id);
        }
    }

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn theme_files_read_leniently() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write(
            dir,
            "full.toml",
            "name = \"Full One\"\nauthor = \"Me\"\nvariant = \"light\"\nfuture_key = 1\n\
             accent = \"#AABBCC\"\non_accent = \"010203\"\ntext = \"#111111\"\ndim = \"#222222\"\n\
             secondary = \"#333333\"\nerror = \"#444444\"\nborder = \"#555555\"\nhighlight = \"#666666\"\n\
             viz_bottom = \"#777777\"\nviz_mid = \"#888888\"\nviz_top = \"#999999\"\n",
        );
        write(dir, "sparse.toml", "accent = \"#102030\"\n");
        write(dir, "with-bg.toml", "background = \"#0a0b0c\"\n");
        write(dir, "no-bg.toml", "background = \"none\"\n");
        write(dir, "bad-bg.toml", "background = \"SECRET-BG\"\n");
        write(dir, "bad-colour.toml", "secondary = \"SECRET-NOT-A-COLOUR\"\ntext = 5\n");
        write(dir, "huge.toml", &format!("# {}\n", "x".repeat(70 * 1024)));
        write(dir, "broken.toml", "accent = = =\n");
        write(dir, "Bad Name.toml", "accent = \"#102030\"\n");
        write(dir, "notes.txt", "not a theme");
        std::fs::create_dir(dir.join("adir.toml")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("sparse.toml"), dir.join("linked.toml")).unwrap();
        // A file named like a preset replaces it, in the preset's place.
        write(dir, "nord.toml", "name = \"My Nord\"\naccent = \"#010101\"\n");

        let found = available(Some(dir));
        let ids: Vec<&str> = found.entries.iter().map(|e| e.id.as_str()).collect();
        let mut expected: Vec<&str> = presets().iter().map(|p| p.id.as_str()).collect();
        expected.extend(["bad-bg", "bad-colour", "full", "no-bg", "sparse", "with-bg"]);
        assert_eq!(ids, expected, "presets, then files by id; nord replaced in place");
        let by = |id: &str| found.entries.iter().find(|e| e.id == id).unwrap();

        let full = by("full");
        assert_eq!((full.title.as_str(), full.author.as_str()), ("Full One", "Me"));
        assert_eq!((full.variant, full.source), (Some(Variant::Light), Source::File));
        assert_eq!(full.theme.accent, Color::Rgb(0xaa, 0xbb, 0xcc), "case is free");
        assert_eq!(full.theme.on_accent, Color::Rgb(1, 2, 3), "the # is optional");
        assert_eq!(full.theme.viz_top, Color::Rgb(0x99, 0x99, 0x99));

        let sparse = by("sparse");
        assert_eq!(sparse.title, "sparse", "the title defaults to the id");
        assert_eq!(sparse.theme.accent, Color::Rgb(0x10, 0x20, 0x30));
        assert_eq!(sparse.theme.text, theme::DEFAULT.text, "a missing colour is the default's");

        assert_eq!(by("with-bg").theme.background, Some(Color::Rgb(10, 11, 12)));
        assert_eq!(by("no-bg").theme.background, None);
        assert_eq!(sparse.theme.background, None, "a missing background is the default's: none");
        assert_eq!(by("bad-bg").theme.background, None);

        assert_eq!(by("bad-colour").theme.secondary, theme::DEFAULT.secondary);
        assert_eq!(by("nord").source, Source::File);
        assert_eq!(by("nord").title, "My Nord");
        assert_eq!(by("dracula").source, Source::Preset);

        let problems = found.problems.join("\n");
        for needle in [
            "bad-bg: background",
            "bad-colour: secondary",
            "bad-colour: text",
            "huge",
            "broken",
            "Bad Name",
        ] {
            assert!(problems.contains(needle), "{needle}\n{problems}");
        }
        assert!(
            !problems.contains("SECRET-NOT-A-COLOUR") && !problems.contains("SECRET-BG"),
            "no file text: {problems}"
        );
        assert!(!problems.contains("notes.txt") && !problems.contains("adir"), "{problems}");
        #[cfg(unix)]
        assert!(problems.contains("linked") && !ids.contains(&"linked"), "{problems}");

        // The same file is the base for its name.
        let (base, _) = resolve_base("nord", Some(dir));
        assert_eq!(
            base.unwrap().theme.accent,
            Color::Rgb(1, 1, 1),
            "the file wins over the preset"
        );
        assert_eq!(resolve_base("nord", None).0.map(|a| a.theme), preset("nord").map(|p| p.theme));
        // A missing directory is normal.
        let none = available(Some(&dir.join("nowhere")));
        assert!(none.problems.is_empty() && none.entries.len() == presets().len());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_is_a_problem() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let found = available(Some(&dir));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
        assert_eq!(found.entries.len(), presets().len());
    }

    #[test]
    fn non_id_name_reads_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir(&dir).unwrap();
        write(tmp.path(), "x.toml", "accent = \"#010101\"\n");
        let absolute = tmp.path().join("x").to_string_lossy().into_owned();
        for name in ["../x", absolute.as_str(), "X", "a/b", ".hidden", ""] {
            assert!(!valid_id(name), "{name}");
            let (theme, problems) = resolve_base(name, Some(&dir));
            assert!(theme.is_none() && problems.is_empty(), "{name}: {problems:?}");
        }
    }

    #[test]
    fn override_beats_file_beats_default() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "mine.toml", "accent = \"#111111\"\ntext = \"#222222\"\n");
        let cfg = crate::config::ThemeConfig {
            name: "mine".into(),
            dir: Some(tmp.path().to_path_buf()),
            accent: Some("#333333".into()),
            ..Default::default()
        };
        let (palette, problems) = theme::from_config(&cfg);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(palette.accent, Color::Rgb(0x33, 0x33, 0x33), "override");
        assert_eq!(palette.text, Color::Rgb(0x22, 0x22, 0x22), "file");
        assert_eq!(palette.secondary, theme::DEFAULT.secondary, "default");
    }

    #[test]
    fn export_overwrites_only_own_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        let mocha = preset("catppuccin-mocha").unwrap();
        let path = export(&dir, "mine", Some(Variant::Dark), &mocha.theme, &Look::DEFAULT).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().next(), Some(EXPORT_MARK));
        assert!(!text.contains("author"), "an empty author is left out");
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let loaded = found.entries.iter().find(|e| e.id == "mine").unwrap();
        assert_eq!(loaded.theme, mocha.theme, "all twelve roles survive");
        assert_eq!(loaded.variant, Some(Variant::Dark));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o044, 0o044, "a theme is not secret: {mode:o}");
        }

        // A theme with no background writes no line, and loads back as none;
        // a preset without one is not the same as one with one.
        let nobg = Theme { background: None, ..mocha.theme };
        let path = export(&dir, "plain", None, &nobg, &Look::DEFAULT).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("background"));
        let found = available(Some(&dir));
        assert_eq!(found.entries.iter().find(|e| e.id == "plain").unwrap().theme, nobg);

        // Its own export is replaced; nothing is left beside it.
        export(&dir, "mine", None, &theme::DEFAULT, &Look::DEFAULT).unwrap();
        let again = available(Some(&dir));
        assert_eq!(again.entries.iter().find(|e| e.id == "mine").unwrap().theme, theme::DEFAULT);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "no temp file left");

        // A file somebody wrote is not.
        write(&dir, "theirs.toml", "accent = \"#010101\"\n");
        let err = export(&dir, "theirs", None, &theme::DEFAULT, &Look::DEFAULT).unwrap_err();
        assert!(err.contains("theirs") && !err.contains("#010101"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("theirs.toml")).unwrap(),
            "accent = \"#010101\"\n"
        );
        assert!(export(&dir, "../escape", None, &theme::DEFAULT, &Look::DEFAULT).is_err());

        // A symlink at the destination is not followed, even to a file that
        // would pass for shmc's own.
        #[cfg(unix)]
        {
            let target = tmp.path().join("elsewhere.toml");
            std::fs::write(&target, format!("{EXPORT_MARK}\n")).unwrap();
            std::os::unix::fs::symlink(&target, dir.join("linked.toml")).unwrap();
            assert!(export(&dir, "linked", None, &theme::DEFAULT, &Look::DEFAULT).is_err());
            assert_eq!(std::fs::read_to_string(&target).unwrap(), format!("{EXPORT_MARK}\n"));
        }
    }

    /// A theme file may set spacing and symbols; what does not fit is
    /// reported by file and key — never by its text — and is the default's.
    #[test]
    fn theme_file_bad_value_named_not_quoted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write(
            dir,
            "roomy.toml",
            "accent = \"#102030\"\n[spacing]\nblock_gap = 2\ninset = 3\nborders = \"Lines\"\nfuture = 1\n\
             [symbols]\nellipsis = \"~\"\nvolume_meter = \"#.\"\nvisualizer = \"12345678\"\n\
             progress_head = \">\"\n[future-table]\nx = 1\n",
        );
        write(
            dir,
            "bad.toml",
            "[spacing]\nblock_gap = 9\ninset = \"SECRET-INSET\"\nborders = \"SECRET-BORDERS\"\n[symbols]\nellipsis = \"SECRET界\"\n\
             volume_meter = \"abc\"\nscroll_up = 5\nscroll_down = \"v\"\n",
        );
        let found = available(Some(dir));
        let by = |id: &str| found.entries.iter().find(|e| e.id == id).unwrap();

        let roomy = by("roomy").look;
        assert_eq!(
            roomy.spacing,
            theme::Spacing { block_gap: 2, inset: 3, borders: theme::BorderStyle::Lines }
        );
        assert_eq!(roomy.symbols.ellipsis, '~');
        assert_eq!(roomy.symbols.volume_meter, theme::Meter::Pair('#', '.'));
        assert_eq!(roomy.symbols.visualizer, ['1', '2', '3', '4', '5', '6', '7', '8']);
        assert_eq!(roomy.symbols.progress_head, Some('>'));
        assert_eq!(roomy.symbols.scroll_up, theme::Symbols::DEFAULT.scroll_up, "unset");
        assert_eq!(by("nord").look, Look::DEFAULT, "a preset defines none");
        assert_eq!(by("roomy").theme.accent, Color::Rgb(0x10, 0x20, 0x30), "colours still load");

        let bad = by("bad").look;
        assert_eq!(bad.spacing, theme::Spacing::DEFAULT);
        assert_eq!(bad.symbols.ellipsis, theme::Symbols::DEFAULT.ellipsis);
        assert_eq!(bad.symbols.scroll_down, 'v', "a good key beside bad ones is kept");
        let problems = found.problems.join("\n");
        for key in [
            "bad: spacing.block_gap",
            "bad: spacing.inset",
            "bad: spacing.borders",
            "bad: symbols.ellipsis",
            "bad: symbols.volume_meter",
            "bad: symbols.scroll_up",
        ] {
            assert!(problems.contains(key), "{key}\n{problems}");
        }
        assert!(!problems.contains("SECRET") && !problems.contains("abc"), "{problems}");
        assert!(!problems.contains("roomy"), "{problems}");
    }

    /// Every shipped theme file loads cleanly and every key in it is one shmc
    /// reads, since unknown keys are skipped silently. `themes/` is what is
    /// built in; `themes/extra/` is opt-in.
    #[test]
    fn shipped_theme_files_load_clean() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/themes"));
        let mut top = Vec::new();
        for dir in [root.to_path_buf(), root.join("extra")] {
            let tmp = tempfile::tempdir().unwrap();
            for file in std::fs::read_dir(&dir).unwrap().flatten() {
                let name = file.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".toml") {
                    continue;
                }
                let text = std::fs::read_to_string(file.path()).unwrap();
                write(tmp.path(), &name, &text);
                if dir == root {
                    top.push(name.trim_end_matches(".toml").to_string());
                }

                let (mut scratch_theme, mut scratch_look) = (Theme::default(), Look::DEFAULT);
                for (key, value) in text.parse::<toml::Table>().unwrap() {
                    let known = match (key.as_str(), &value) {
                        ("name" | "author" | "variant" | "styles", _) => true,
                        ("spacing", toml::Value::Table(t)) => t.iter().all(|(k, v)| match v {
                            toml::Value::String(s) => {
                                theme::set_spacing_text(&mut scratch_look, k, s)
                            }
                            toml::Value::Integer(n) => theme::set_spacing(&mut scratch_look, k, *n),
                            _ => false,
                        }),
                        ("symbols", toml::Value::Table(t)) => t.iter().all(|(k, v)| {
                            v.as_str().is_some_and(|s| theme::set_symbol(&mut scratch_look, k, s))
                        }),
                        (role, toml::Value::String(s)) => {
                            theme::set_role(&mut scratch_theme, role, s)
                        }
                        _ => false,
                    };
                    assert!(known, "{key} in themes/{name} is ignored");
                }
            }
            let found = available(Some(tmp.path()));
            assert!(found.problems.is_empty(), "{}: {:?}", dir.display(), found.problems);
            if dir != root {
                let vga = found.entries.iter().find(|e| e.id == "vga").unwrap();
                assert_eq!(vga.look.spacing.borders, theme::BorderStyle::Double);
                let fill = vga.look.styles.get(theme::Element::ProgressFill).unwrap();
                assert_eq!(fill.fg, Some(theme::ColorRef::Role(theme::Role::Secondary)));
            }
        }

        top.sort();
        let mut built_in: Vec<&str> = presets().iter().skip(1).map(|p| p.id.as_str()).collect();
        built_in.sort();
        assert_eq!(top, built_in, "a file in themes/ is built in, and extra/ never is");
    }

    /// An export carries only what differs from the default, so a theme that
    /// sets none has no tables, and what it does set loads back unchanged.
    #[test]
    fn export_writes_only_changed_spacing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        let plain = export(&dir, "plain", None, &theme::DEFAULT, &Look::DEFAULT).unwrap();
        let text = std::fs::read_to_string(plain).unwrap();
        assert!(!text.contains('['), "a default look writes no table:\n{text}");

        let mut look = Look::DEFAULT;
        assert!(theme::set_spacing(&mut look, "inset", 2));
        assert!(theme::set_spacing_text(&mut look, "borders", "rounded"));
        for (key, value) in [
            ("ellipsis", "~"),
            ("volume_meter", "■·"),
            ("progress_filled", "▰"),
            ("visualizer", "12345678"),
        ] {
            assert!(theme::set_symbol(&mut look, key, value));
        }
        let path = export(&dir, "mine", None, &theme::DEFAULT, &look).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("[spacing]") && text.contains("inset = 2"), "{text}");
        assert!(
            !text.contains("block_gap") && !text.contains("scroll_up"),
            "only what differs:\n{text}"
        );
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        assert_eq!(found.entries.iter().find(|e| e.id == "mine").unwrap().look, look);
    }

    /// `progress_bar` applies before the glyphs whatever the order in the file,
    /// and an export of a preset or of glyphs loads back.
    #[test]
    fn progress_preset_loads_first_and_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir(&dir).unwrap();
        write(&dir, "tweaked.toml", "[symbols]\nprogress_head = \"~\"\nprogress_bar = \"dot\"\n");
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let tweaked = found.entries.iter().find(|e| e.id == "tweaked").unwrap().look;
        assert_eq!(tweaked.symbols.progress(), [Some('━'), Some('─'), Some('~'), None, None]);

        let mut named = Look::DEFAULT;
        assert!(theme::set_symbol(&mut named, "progress_bar", "rounded"));
        let text =
            std::fs::read_to_string(export(&dir, "named", None, &theme::DEFAULT, &named).unwrap())
                .unwrap();
        assert!(
            text.contains("progress_bar = \"rounded\"") && !text.contains("progress_start"),
            "{text}"
        );
        let text = std::fs::read_to_string(
            export(&dir, "custom", None, &theme::DEFAULT, &tweaked).unwrap(),
        )
        .unwrap();
        assert!(!text.contains("progress_bar") && text.contains("progress_filled"), "{text}");
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        for (id, look) in [("named", named), ("custom", tweaked)] {
            assert_eq!(found.entries.iter().find(|e| e.id == id).unwrap().look, look, "{id}");
        }
    }

    /// A preset name in a theme file is that preset; an export writes the name
    /// when the glyphs are a preset's, and the glyphs otherwise; both load back.
    #[test]
    fn presets_and_literals_round_trip_through_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir(&dir).unwrap();
        write(
            &dir,
            "named.toml",
            "[symbols]\nvolume_meter = \"outline\"\nvisualizer = \"ascii\"\n",
        );
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let named = found.entries.iter().find(|e| e.id == "named").unwrap().look;
        assert_eq!(named.symbols.volume_meter, theme::Meter::Pair('■', '□'));
        assert_eq!(named.symbols.visualizer[0], '_');

        for (meter, viz) in [("squares", "ascii"), ("<>", "12345678")] {
            let mut look = Look::DEFAULT;
            assert!(theme::set_symbol(&mut look, "volume_meter", meter));
            assert!(theme::set_symbol(&mut look, "visualizer", viz));
            let path = export(&dir, "out", None, &theme::DEFAULT, &look).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains(&format!("volume_meter = \"{meter}\"")), "{text}");
            assert!(text.contains(&format!("visualizer = \"{viz}\"")), "{text}");
            let back = available(Some(&dir)).entries.into_iter().find(|e| e.id == "out").unwrap();
            assert_eq!(back.look, look, "{meter}");
        }
    }

    /// `[styles]`: what fits applies, what does not is ignored and reported by
    /// file, element and key — never by its value — and a newer file still loads.
    #[test]
    fn theme_styles_read_leniently() {
        use ratatui::style::Modifier;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write(
            dir,
            "styled.toml",
            "[styles]\ncurrent = { fg = \"on_accent\", bg = \"#AABBCC\", bold = false, italic = true }\n\
             dim = { underline = true, reversed = true, dim = false }\ntab_active = { bg = \"background\" }\n\
             error = {}\n",
        );
        write(dir, "only-styles.toml", "[styles]\nsecondary = { bold = true }\n");
        write(
            dir,
            "bad.toml",
            "[styles]\nfuture_element = { bold = true }\ntext = \"SECRET-ENTRY\"\n\
             accent = { fg = \"SECRET-COLOUR\", flash = true, italic = \"SECRET-BOOL\", bold = true }\n",
        );
        write(dir, "not-table.toml", "styles = \"SECRET-STYLES\"\n");
        let found = available(Some(dir));
        let by = |id: &str| found.entries.iter().find(|e| e.id == id).unwrap().look.styles;
        let none = Modifier::empty();

        let styled = by("styled");
        let current = styled.get(theme::Element::Current).unwrap();
        assert_eq!(current.fg, Some(theme::ColorRef::Role(theme::Role::OnAccent)));
        assert_eq!(current.bg, Some(theme::ColorRef::Rgb(0xaa, 0xbb, 0xcc)), "hex, any case");
        assert_eq!((current.add, current.remove), (Modifier::ITALIC, Modifier::BOLD));
        let dim = styled.get(theme::Element::Dim).unwrap();
        assert_eq!(
            (dim.add, dim.remove),
            (Modifier::UNDERLINED | Modifier::REVERSED, Modifier::DIM)
        );
        assert_eq!(
            styled.get(theme::Element::TabActive).unwrap().bg,
            Some(theme::ColorRef::Role(theme::Role::Background))
        );
        assert_eq!(styled.get(theme::Element::Error), None, "an empty entry sets nothing");
        assert_eq!(styled.count(), 3);
        assert_eq!(by("only-styles").get(theme::Element::Secondary).unwrap().add, Modifier::BOLD);
        assert_eq!(by("nord"), theme::Styles::NONE, "a preset sets none");

        // What fits in a bad entry still applies.
        let bad = by("bad");
        let accent = bad.get(theme::Element::Accent).unwrap();
        assert_eq!((accent.fg, accent.add, accent.remove), (None, Modifier::BOLD, none));
        assert_eq!(bad.count(), 1, "an unknown element and a non-table entry set nothing");
        assert_eq!(by("not-table"), theme::Styles::NONE);

        let problems = found.problems.join("\n");
        for needle in [
            "bad: styles.\"future_element\"",
            "bad: styles.text is not a table",
            "bad: styles.accent: \"fg\"",
            "bad: styles.accent: \"flash\"",
            "bad: styles.accent: \"italic\"",
            "not-table: styles is not a table",
        ] {
            assert!(problems.contains(needle), "{needle}\n{problems}");
        }
        assert!(!problems.contains("SECRET"), "no values: {problems}");
        assert!(!problems.contains("styled:") && !problems.contains("only-styles"), "{problems}");
    }

    /// An export keeps the styles of a styled theme, in a stable order and only
    /// the fields that were set; a theme without any writes no table.
    #[test]
    fn export_keeps_styles_and_loads_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        let plain = export(&dir, "plain", None, &theme::DEFAULT, &Look::DEFAULT).unwrap();
        assert!(!std::fs::read_to_string(plain).unwrap().contains("styles"));
        let preset =
            export(&dir, "from-nord", None, &preset("nord").unwrap().theme, &Look::DEFAULT)
                .unwrap();
        assert!(!std::fs::read_to_string(preset).unwrap().contains("styles"));

        write(
            &dir,
            "src.toml",
            "[styles]\ntab_active = { italic = true }\ncurrent = { bold = false, fg = \"secondary\", bg = \"#010203\" }\n",
        );
        let src = available(Some(&dir)).entries.into_iter().find(|e| e.id == "src").unwrap();
        let path = export(&dir, "copy", None, &src.theme, &src.look).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let current = text.find("current = ").expect(&text);
        assert!(current < text.find("tab_active = ").unwrap(), "stable element order:\n{text}");
        assert!(
            text.contains("current = { fg = \"secondary\", bg = \"#010203\", bold = false }"),
            "{text}"
        );
        assert!(
            text.contains("tab_active = { italic = true }") && text.matches(" = {").count() == 2,
            "{text}"
        );
        let found = available(Some(&dir));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let back = found.entries.iter().find(|e| e.id == "copy").unwrap();
        assert_eq!(back.look.styles, src.look.styles);
    }
}
