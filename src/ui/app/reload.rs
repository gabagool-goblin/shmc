//! A saved config applied to the running player.

use super::ui_settings::{player_ui, UiSettings};
use super::{App, Msg};
use crate::config::Config;
use crate::ui::keymap::Keymap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const WATCH_EVERY: Duration = Duration::from_secs(1);

pub struct Reload {
    pub cfg: Config,
    /// Changed keys that wait for the next start.
    pub later: Vec<&'static str>,
}

/// What the player started with that a saved config cannot add.
#[derive(Clone, Copy)]
pub struct Startup {
    pub cava: bool,
    pub client_id: bool,
}

/// Sends a `Msg::ConfigRead` each time the file at `path` changes.
pub fn watch(path: PathBuf, mut running: Config, startup: Startup, tx: UnboundedSender<Msg>) {
    crate::config::watch::watch(path, WATCH_EVERY, move |read| {
        let read = read.map_err(|e| format!("{e:#}")).map(|cfg| {
            let reload = settle(&running, cfg.clone(), startup);
            running = cfg;
            reload
        });
        tx.send(Msg::ConfigRead(Box::new(read))).is_ok()
    });
}

/// `cfg` with what `startup` lacks kept off, and the keys that wait for the next start.
pub fn settle(running: &Config, mut cfg: Config, startup: Startup) -> Reload {
    let mut later = running.next_start_changes(&cfg);
    let mut wait = |on: bool, was: bool, key| {
        if on && !was {
            later.push(key);
        }
    };
    let shown = |c: &Config| c.cava.enabled && c.ui.visualizer_pct > 0;
    if !startup.cava && running.cava.enabled == cfg.cava.enabled {
        wait(shown(&cfg), shown(running), "ui.visualizer_pct");
    }
    cfg.cava.enabled = startup.cava;
    if !startup.client_id {
        wait(cfg.plex.allow_writes, running.plex.allow_writes, "plex.allow_writes");
        cfg.plex.allow_writes = false;
    }
    Reload { cfg, later }
}

impl App {
    pub(super) fn config_read(&mut self, read: Result<Reload, String>) {
        match read {
            Ok(reload) => self.apply_config(reload),
            Err(e) => self.transient_err(format!("{e}; settings unchanged")),
        }
    }

    fn apply_config(&mut self, Reload { cfg, mut later }: Reload) {
        let art =
            |app: &Self| (app.ui.art_layout(), crate::ui::theme::theme(), crate::ui::theme::look());
        let art_before = art(self);
        let mut problems = Vec::new();
        let (appearance, theme_problems) = crate::ui::theme::appearance_from_config(&cfg.theme);
        crate::ui::theme::set_appearance(appearance, cfg.ui.ascii_icons());
        problems.extend(theme_problems);

        let (keymap, key_problems) = Keymap::from_table(&cfg.keys);
        self.keymap = keymap;
        self.pending_g = false;
        problems.extend(key_problems.into_iter().map(|p| format!("{p}; using the default keys")));

        let auto_follow = self.ui.auto_follow;
        self.ui = UiSettings::new(&player_ui(&cfg));
        if self.ui.auto_follow != auto_follow {
            self.queue_follow = self.ui.auto_follow;
        }
        if art(self) != art_before {
            self.art.shown_rect = None;
            self.art.protocol = None;
            self.art.prepared_rect = None;
            self.hold_art();
        }

        self.allow_writes = cfg.plex.allow_writes;
        if !self.relevel(cfg.mpv.leveling(), cfg.mpv.preamp_db()) {
            later.push("mpv.loudness_leveling");
        }

        self.reloaded = Some(Box::new(cfg));
        match (problems.first(), later.as_slice()) {
            (Some(problem), []) => self.transient_err(problem.clone()),
            (Some(problem), keys) => {
                self.transient_err(format!("{problem}; {} on the next start", and_list(keys)));
            }
            (None, []) => self.transient("settings applied".into()),
            (None, keys) => {
                self.transient(format!("settings applied; {} on the next start", and_list(keys)));
            }
        }
    }
}

fn and_list(keys: &[&str]) -> String {
    match keys {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visualizer_without_cava_waits_for_the_next_start() {
        let mut running = Config::default();
        running.cava.enabled = true;
        running.ui.visualizer_pct = 0;
        let mut saved = running.clone();
        saved.ui.visualizer_pct = 30;
        let reload = settle(&running, saved, Startup { cava: false, client_id: true });
        assert_eq!(reload.later, ["ui.visualizer_pct"]);
        assert_eq!(player_ui(&reload.cfg).visualizer_pct, 0);
    }

    #[test]
    fn the_art_size_cap_waits_for_the_next_start() {
        let running = Config::default();
        let mut saved = running.clone();
        saved.album_art.max_size_px = [1200, 1200];
        let reload = settle(&running, saved, Startup { cava: true, client_id: true });
        assert_eq!(reload.later, ["album_art.max_size_px"]);
    }

    #[test]
    fn writes_without_a_client_id_wait_for_the_next_start() {
        let running = Config::default();
        let mut saved = running.clone();
        saved.plex.allow_writes = true;
        let reload = settle(&running, saved, Startup { cava: true, client_id: false });
        assert_eq!(reload.later, ["plex.allow_writes"]);
        assert!(!reload.cfg.plex.allow_writes);
    }
}
