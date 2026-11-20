//! Setup: ask for the server URL and token, check them with the server, save.

use anyhow::{bail, Context, Result};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::plex::PlexClient;

pub fn can_prompt() -> bool {
    io::stdin().is_terminal()
}

/// What to print when there is no terminal to prompt on.
pub fn hint(path: &Path) -> String {
    format!(
        "config not found at {}\n\nRun 'shmc config' in a terminal to set it up, or create the file:\n  \
         [plex]\n  server = \"http://<plex-host>:32400\"\n  token  = \"<your X-Plex-Token>\"\n\n\
         Find your token: https://app.plex.tv/desktop#!/settings/account",
        path.display()
    )
}

/// `existing`'s values are offered as defaults.
pub async fn interactive(path: &PathBuf, existing: Option<Config>) -> Result<Config> {
    let d_server = existing.as_ref().map(|c| c.plex.server.as_str()).filter(|s| !s.is_empty());
    let d_token = existing.as_ref().map(|c| c.plex.token.as_str()).filter(|s| !s.is_empty());

    println!();
    println!("shmc setup");
    println!();
    println!("Config file: {}", path.display());
    println!();
    println!("Find your Plex token at https://app.plex.tv/desktop#!/settings/account");
    println!("(View XML, then copy the X-Plex-Token=... value from the URL).");
    println!();

    let server = prompt("Plex server URL", d_server)?;
    if let Some(warning) = crate::config::plain_http_warning(&server) {
        println!("  ! {warning}");
        println!();
    }
    let token = prompt_secret("Plex token", d_token)?;

    print!("checking {server} ... ");
    io::stdout().flush().ok();
    let client = PlexClient::new(&server, &token);
    match client.identity().await {
        Ok((machine, version)) => {
            println!("ok");
            println!("  Plex Media Server {version} ({})", crate::doctor::short_id(&machine));
        }
        Err(e) => {
            println!("failed");
            println!("  {e:#}");
            if !confirm("Save this config anyway?", false)? {
                bail!("setup cancelled; nothing was written");
            }
        }
    }

    // Baseline = what the file held, so only `plex.server` and `plex.token` are written.
    let baseline = existing.clone().unwrap_or_default();
    let mut cfg = existing.unwrap_or_default();
    cfg.plex.server = server;
    cfg.plex.token = token;
    cfg.save_changes(path, &baseline)?;
    println!();
    println!("saved {}", path.display());

    Config::load(path)
}

/// 0600 through a temp file: `fs::copy` would keep a hand-edited config's 0644.
pub(crate) fn backup_config(path: &Path) -> Result<PathBuf> {
    // Next to the link, not its target, which is often in a dotfiles repository.
    let backup = path.with_extension("toml.bak");
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    crate::cache::write_private_atomic(&backup, &bytes)
        .with_context(|| format!("backing up {} to {}", path.display(), backup.display()))?;
    Ok(backup)
}

/// Through the TOML form, so the schema checks the type; a bad value changes nothing.
pub(crate) fn set_by_key(cfg: &mut Config, key: &str, value: toml::Value) -> Result<()> {
    let mut table: toml::Table = toml::from_str(&toml::to_string(cfg)?)?;
    let mut parts: Vec<&str> = key.split('.').collect();
    let Some(last) = parts.pop() else {
        bail!("empty setting key");
    };
    let mut node = &mut table;
    for part in parts {
        node = node
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .with_context(|| format!("{part} is not a section"))?;
    }
    node.insert(last.to_string(), value);

    let text = toml::to_string(&table)?;
    match toml::from_str::<Config>(&text) {
        Ok(new) => {
            // Not in the TOML, so it would be lost.
            let themes_dir = cfg.theme.dir.take();
            *cfg = new;
            cfg.theme.dir = themes_dir;
            Ok(())
        }
        Err(e) => {
            // Not `e` itself: its Display quotes the line, which can be the token.
            Err(anyhow::anyhow!(crate::config::scrub_value(e.message())))
                .context("the value does not fit the setting")
        }
    }
}

pub(crate) fn get_by_key(cfg: &Config, key: &str) -> Option<toml::Value> {
    let table: toml::Table = toml::from_str(&toml::to_string(cfg).ok()?).ok()?;
    let mut parts: Vec<&str> = key.split('.').collect();
    let last = parts.pop()?;
    let mut node = table;
    for part in parts {
        node = node.get(part)?.as_table()?.clone();
    }
    node.get(last).cloned()
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        match default {
            Some(d) => print!("{label} [{d}]: "),
            None => print!("{label}: "),
        }
        io::stdout().flush().ok();
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            bail!("cancelled (no input)");
        }
        let value = line.trim();
        let value =
            if value.is_empty() { default.unwrap_or("").to_string() } else { value.to_string() };
        if !value.is_empty() {
            return Ok(value);
        }
        println!("  a value is required");
    }
}

/// Read with echo off. Empty input takes `default`.
fn prompt_secret(label: &str, default: Option<&str>) -> Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use crossterm::terminal;

    let hint = if default.is_some() { " [keep existing]" } else { "" };
    print!("{label}{hint}: ");
    io::stdout().flush().ok();

    terminal::enable_raw_mode().context("enabling raw mode")?;
    let mut buf = String::new();
    let outcome = (|| -> Result<()> {
        loop {
            if let Event::Key(k) = event::read()? {
                match k.code {
                    KeyCode::Enter => return Ok(()),
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        bail!("cancelled");
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) => buf.push(c),
                    _ => {}
                }
            }
        }
    })();
    terminal::disable_raw_mode().ok();
    println!();
    outcome?;

    let value = buf.trim();
    if value.is_empty() {
        if let Some(d) = default {
            return Ok(d.to_string());
        }
        bail!("no token entered");
    }
    Ok(value.to_string())
}

fn confirm(label: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{label} {hint} ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Ok(default_yes);
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_a_value_goes_through_the_schema() {
        let mut cfg = Config::default();
        set_by_key(&mut cfg, "ui.art_pct", toml::Value::Integer(55)).unwrap();
        assert_eq!(cfg.ui.art_pct, 55);

        set_by_key(&mut cfg, "theme.accent", toml::Value::String("#abcdef".into())).unwrap();
        assert_eq!(cfg.theme.accent.as_deref(), Some("#abcdef"));

        set_by_key(
            &mut cfg,
            "mpv.extra_args",
            toml::Value::Array(vec![toml::Value::String("--no-video".into())]),
        )
        .unwrap();
        assert_eq!(cfg.mpv.extra_args, vec!["--no-video".to_string()]);

        let before = cfg.ui.art_pct;
        assert!(
            set_by_key(&mut cfg, "ui.art_pct", toml::Value::Integer(70_000)).is_err(),
            "an out-of-range value is refused"
        );
        assert_eq!(cfg.ui.art_pct, before, "and nothing changed");
    }

    #[test]
    fn typed_editors_read_value_back() {
        let mut cfg = Config::default();
        cfg.album_art.max_size_px = [512, 256];
        assert_eq!(
            get_by_key(&cfg, "album_art.max_size_px"),
            Some(toml::Value::Array(vec![toml::Value::Integer(512), toml::Value::Integer(256)]))
        );

        cfg.mpv.extra_args = vec!["--a".into(), "--b".into()];
        assert_eq!(
            get_by_key(&cfg, "mpv.extra_args"),
            Some(toml::Value::Array(vec![
                toml::Value::String("--a".into()),
                toml::Value::String("--b".into())
            ]))
        );
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn backup_private_even_if_source_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[plex]\ntoken = \"secret\"\n").unwrap();
        // hand-edited configs are typically 0644
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let backup = backup_config(&path).unwrap();
        assert_eq!(backup, tmp.path().join("config.toml.bak"));
        assert_eq!(mode(&backup), 0o600, "the backup holds the token too");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "[plex]\ntoken = \"secret\"\n");
    }

    #[test]
    fn failed_backup_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[plex]\nserver = \"http://host:32400\"\n").unwrap();
        // a directory occupies the backup path, so the rename cannot land
        std::fs::create_dir(tmp.path().join("config.toml.bak")).unwrap();
        assert!(backup_config(&path).is_err());
        let leftovers: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn new_backup_replaces_previous() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[plex]\ntoken = \"new\"\n").unwrap();
        std::fs::write(tmp.path().join("config.toml.bak"), "[plex]\ntoken = \"old\"\n").unwrap();

        let backup = backup_config(&path).unwrap();
        assert_eq!(backup, tmp.path().join("config.toml.bak"));
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "[plex]\ntoken = \"new\"\n",
            "the backup must be the config as it was before this save"
        );
    }
}
