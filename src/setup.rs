//! Setup: sign in through plex.tv or ask for the server URL and token, check them, save.

use anyhow::{bail, Context, Result};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::plex::account::{self, PlexTv};
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

pub async fn interactive(path: &PathBuf, existing: Option<Config>) -> Result<Config> {
    println!();
    println!("shmc setup");
    println!();
    println!("Config file: {}", path.display());
    println!();
    println!("  1) Sign in with plex.tv");
    println!("  2) Enter a server URL and token");
    let signing_in = loop {
        match prompt("Choose", Some("1"))?.as_str() {
            "1" => break true,
            "2" => break false,
            _ => println!("  1 or 2"),
        }
    };
    let (server, token) =
        if signing_in { sign_in(path).await? } else { typed(existing.as_ref()).await? };

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

async fn typed(existing: Option<&Config>) -> Result<(String, String)> {
    let d_server = existing.map(|c| c.plex.server.as_str()).filter(|s| !s.is_empty());
    let d_token = existing.map(|c| c.plex.token.as_str()).filter(|s| !s.is_empty());
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
    Ok((server, token))
}

fn client_id_beside(path: &Path) -> Result<String> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    crate::config::ensure_config_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
    crate::plex::client_id::load_or_create(dir).context("storing the client id")
}

async fn sign_in(path: &Path) -> Result<(String, String)> {
    let tv = PlexTv::new(&client_id_beside(path)?);
    let pin = tv.create_pin().await?;
    println!();
    println!("Go to https://plex.tv/link and enter the code  {}", pin.code);
    println!(
        "Waiting for plex.tv... (the code lasts {} minutes; Ctrl-C cancels)",
        pin.expires_in.as_secs() / 60
    );
    let account_token = tv.wait_for_token(&pin, account::POLL_EVERY).await?;
    println!("signed in");

    let mut servers = tv.servers(&account_token).await?;
    let server = match servers.len() {
        0 => bail!("this Plex account has no servers; the config was not changed"),
        1 => servers.remove(0),
        n => {
            println!();
            for (i, s) in servers.iter().enumerate() {
                let shared = if s.owned { "" } else { " (shared)" };
                println!("  {}) {}{shared}", i + 1, s.name);
            }
            let pick = loop {
                match prompt("Server", Some("1"))?.parse::<usize>() {
                    Ok(i) if (1..=n).contains(&i) => break i - 1,
                    _ => println!("  a number from 1 to {n}"),
                }
            };
            servers.remove(pick)
        }
    };

    println!();
    print!("checking {} ... ", server.name);
    io::stdout().flush().ok();
    let addresses = server.addresses();
    let Some((i, machine, version)) =
        account::first_answering(&addresses, &server.token, account::ANSWER_WITHIN).await
    else {
        println!("failed");
        bail!(
            "none of its {} addresses answered from here; the config was not changed",
            addresses.len()
        );
    };
    let address = &addresses[i];
    let kind = match (address.relay, address.local, address.https) {
        (true, _, _) => "relay",
        (_, true, true) => "local, https",
        (_, true, false) => "local, http",
        (_, false, _) => "remote, https",
    };
    println!("ok");
    println!("  {} ({kind})", address.uri);
    println!("  Plex Media Server {version} ({})", crate::doctor::short_id(&machine));
    if address.relay {
        println!("  ! a relay is slow; shmc uses it because no direct address answered");
    }
    if let Some(warning) = crate::config::plain_http_warning(&address.uri) {
        println!("  ! {warning}");
    }
    Ok((address.uri.clone(), server.token))
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
    fn a_first_sign_in_creates_the_config_dir() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("shmc");
        let id = client_id_beside(&dir.join("config.toml")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("client-id")).unwrap(), id);
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

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
