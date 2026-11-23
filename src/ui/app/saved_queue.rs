//! The queue kept across restarts (`ui.restore_queue`).

use super::{App, ListRef, PlexCmd, ShuffleSnapshot};
use crate::playback::MpvCmd;
use crate::plex::Track;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Never a stream URL, so never the token.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedQueue {
    pub server: String,
    pub tracks: Vec<String>,
    /// Index into `tracks`; `None` when stopped.
    pub current: Option<usize>,
    pub position: f64,
    pub unshuffled: Option<Vec<String>>,
    pub random: bool,
    pub loop_playlist: bool,
    pub loop_file: bool,
    pub consume: bool,
}

/// `Ok(None)` when nothing was saved.
pub fn read(path: &Path) -> anyhow::Result<Option<SavedQueue>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let saved: SavedQueue = serde_json::from_slice(&bytes)?;
    // a key goes into a request path
    let keys = saved.tracks.iter().chain(saved.unshuffled.iter().flatten());
    if !keys.into_iter().all(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit())) {
        anyhow::bail!("a rating key is not a number");
    }
    Ok(Some(saved))
}

pub fn write(path: &Path, saved: &SavedQueue) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        crate::cache::ensure_private_dir(dir)?;
    }
    crate::cache::write_private_atomic(path, &serde_json::to_vec(saved)?)?;
    Ok(())
}

pub fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("cannot remove the saved queue: {e}"),
    }
}

#[derive(Debug, Default)]
pub(super) struct Restore {
    /// While set, an empty queue at quit keeps the file.
    pub(super) unapplied: bool,
    /// Queue index and position, applied once that entry loads.
    pub(super) seek: Option<(usize, f64)>,
}

fn tracks_word(n: usize) -> &'static str {
    if n == 1 {
        "track"
    } else {
        "tracks"
    }
}

impl App {
    /// The file to save the queue to on quit; `None` when off.
    pub fn restore_saved_queue(&mut self, enabled: bool) -> Option<PathBuf> {
        let path = crate::config::queue_path()?;
        if !enabled {
            remove(&path);
            return None;
        }
        match read(&path) {
            Ok(Some(saved)) => self.restore_queue(saved),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("cannot read the saved queue: {e:#}");
                self.transient_err("saved queue unreadable; starting empty".into());
            }
        }
        Some(path)
    }

    pub fn saved_queue(&self) -> SavedQueue {
        let keys = |tracks: &[Track]| tracks.iter().map(|t| t.rating_key.clone()).collect();
        let current = self.current.filter(|&c| !self.stopped && c < self.queue.items.len());
        SavedQueue {
            server: self.server.clone(),
            tracks: keys(&self.queue.items),
            current,
            position: if current.is_some() { self.position } else { 0.0 },
            unshuffled: self
                .shuffle_snapshot
                .as_ref()
                .filter(|_| self.random)
                .map(|s| keys(&s.order)),
            random: self.random,
            loop_playlist: self.loop_playlist,
            loop_file: self.loop_file,
            consume: self.consume,
        }
    }

    pub fn restore_queue(&mut self, saved: SavedQueue) {
        if saved.tracks.is_empty() {
            return;
        }
        if saved.server != self.server {
            tracing::info!("not restoring a queue saved for another server");
            return;
        }
        let seq = self.issue_seq(true);
        self.restore.unapplied = true;
        let _ = self.plex.tx.send(PlexCmd::RestoreQueue { seq, saved });
    }

    pub fn save_queue(&self, path: &Path) {
        if self.queue.items.is_empty() {
            if !self.restore.unapplied {
                remove(path);
            }
            return;
        }
        if let Err(e) = write(path, &self.saved_queue()) {
            tracing::warn!("cannot save the queue: {e:#}");
        }
    }

    /// `tracks` has each saved key at most once, in any order.
    pub(super) fn apply_restore(&mut self, tracks: Vec<Track>, saved: SavedQueue) {
        self.restore.unapplied = false;
        let by_key: HashMap<String, Track> = tracks
            .into_iter()
            .filter(|t| self.load_item(t).is_some())
            .map(|t| (t.rating_key.clone(), t))
            .collect();
        let mut queue = Vec::with_capacity(saved.tracks.len());
        let mut current = None;
        let mut position = saved.position;
        for (i, key) in saved.tracks.iter().enumerate() {
            let found = by_key.get(key);
            if saved.current == Some(i) {
                current = Some(queue.len());
                if found.is_none() {
                    position = 0.0;
                }
            }
            queue.extend(found.cloned());
        }
        let gone = saved.tracks.len() - queue.len();
        if queue.is_empty() {
            self.transient_err(
                "saved queue not restored: its tracks are no longer in the library".into(),
            );
            return;
        }
        let current = current.filter(|&c| c < queue.len());

        self.playlist_load = None;
        self.radio = None;
        let (queue, items) = self.playable(queue);
        let n = queue.len();
        self.queue.items = queue;
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.queue.filter.clear();
        self.refilter(ListRef::Queue);
        self.random = saved.random;
        self.shuffle_snapshot = saved.unshuffled.filter(|_| saved.random).map(|keys| {
            ShuffleSnapshot { order: keys.iter().filter_map(|k| by_key.get(k).cloned()).collect() }
        });
        self.consume = saved.consume;
        for (property, on) in
            [("loop-playlist", saved.loop_playlist), ("loop-file", saved.loop_file)]
        {
            let _ = self.mpv.tx.send(MpvCmd::SetLoop { property, on });
        }
        let _ = self.mpv.tx.send(MpvCmd::Append(items));
        match current {
            Some(c) => {
                let _ = self.mpv.tx.send(MpvCmd::SetPause { paused: true });
                self.start_queue_index(c);
                self.paused = true;
                self.position = position;
                self.restore.seek = (position > 0.0).then_some((c, position));
            }
            None => {
                self.stopped = true;
                self.queue.state.select(Some(0));
            }
        }
        let mut msg = format!("queue restored: {n} {}", tracks_word(n));
        if gone > 0 {
            msg.push_str(&format!("; {gone} no longer in the library"));
        }
        self.transient(msg);
    }

    pub(super) fn seek_restored(&mut self) {
        let Some((idx, position)) = self.restore.seek.take() else { return };
        if self.current == Some(idx) {
            let _ = self.mpv.tx.send(MpvCmd::SeekAbs(position));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(keys: &[&str]) -> SavedQueue {
        SavedQueue {
            server: "http://plex:32400".into(),
            tracks: keys.iter().map(|k| k.to_string()).collect(),
            current: Some(1),
            position: 12.5,
            random: true,
            unshuffled: Some(vec!["2".into(), "1".into()]),
            ..SavedQueue::default()
        }
    }

    #[test]
    fn written_queue_reads_back_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("queue.json");
        assert_eq!(read(&path).unwrap(), None);
        write(&path, &saved(&["1", "2"])).unwrap();

        assert_eq!(read(&path).unwrap(), Some(saved(&["1", "2"])));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
    }

    #[test]
    fn key_that_is_not_a_number_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        for bad in ["1/../../status", "1?x=1", ""] {
            write(&path, &saved(&["1", bad])).unwrap();
            assert!(read(&path).is_err(), "{bad:?}");
        }
    }
}
