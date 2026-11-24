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
    /// Its `cava.enabled` is the running one.
    pub cfg: Config,
    /// Changed keys that wait for the next start.
    pub later: Vec<&'static str>,
}

/// Sends a `Msg::ConfigRead` each time the file at `path` changes.
pub fn watch(path: PathBuf, mut running: Config, tx: UnboundedSender<Msg>) {
    let cava_enabled = running.cava.enabled;
    crate::config::watch::watch(path, WATCH_EVERY, move |read| {
        let read = read.map_err(|e| format!("{e:#}")).map(|mut cfg| {
            let later = running.next_start_changes(&cfg);
            running = cfg.clone();
            cfg.cava.enabled = cava_enabled;
            Reload { cfg, later }
        });
        tx.send(Msg::ConfigRead(Box::new(read))).is_ok()
    });
}

impl App {
    pub(super) fn config_read(&mut self, read: Result<Reload, String>) {
        match read {
            Ok(reload) => self.apply_config(reload),
            Err(e) => self.transient_err(format!("{e}; settings unchanged")),
        }
    }

    fn apply_config(&mut self, Reload { cfg, mut later }: Reload) {
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
        self.art.shown_rect = None;
        self.art.protocol = None;
        self.art.prepared_rect = None;
        self.hold_art();

        self.allow_writes = cfg.plex.allow_writes;
        self.art_max_px = cfg.album_art.max_size_px;
        if !self.relevel(cfg.mpv.leveling(), cfg.mpv.preamp_db()) {
            later.push("mpv.loudness_leveling");
        }

        self.reloaded = Some(Box::new(cfg));
        match (problems.first(), later.as_slice()) {
            (Some(problem), _) => self.transient_err(problem.clone()),
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
