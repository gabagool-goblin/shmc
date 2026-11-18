//! Reading and writing config.toml: private writes, the backup and in-place edits.

use super::{config_path, Config};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// Keys shmc no longer reads, each with what replaced it.
const RETIRED_KEYS: &[(&str, &str)] = &[("plex.set_ratings", "plex.allow_writes")];

/// The retired keys `text` still sets, with their replacements. Unknown keys are
/// ignored on load, so only `shmc config --check` can point these out.
pub fn retired_keys(text: &str) -> Vec<(&'static str, &'static str)> {
    let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
    let mut flat = BTreeMap::new();
    flatten_values(&table, "", &mut flat);
    RETIRED_KEYS.iter().copied().filter(|(old, _)| flat.contains_key(*old)).collect()
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PlexConfig;

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
