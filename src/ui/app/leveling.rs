//! Gains for the leveling script.

use super::{App, PlexCmd};
use crate::playback::{leveling, Leveling, MpvCmd};
use crate::plex::{AudioInfo, Track};
use std::collections::HashSet;

#[derive(Debug, Default)]
pub(super) struct LevelingState {
    mode: Leveling,
    preamp_db: f64,
    published: HashSet<String>,
    asked: HashSet<String>,
    pub(super) ready: bool,
    warned: bool,
}

/// Bulk lists have no `<Stream>`: no sample rate, no analysis.
pub(super) fn is_full(audio: &AudioInfo) -> bool {
    audio.sample_rate_hz.is_some() || audio.bit_depth.is_some()
}

impl App {
    pub fn set_leveling(&mut self, mode: Leveling, preamp_db: f64) {
        self.leveling.mode = mode;
        self.leveling.preamp_db = preamp_db;
    }

    pub(super) fn level_around_current(&mut self) {
        if self.leveling.mode == Leveling::Off {
            return;
        }
        let Some(current) = self.current else { return };
        let next = if self.loop_file {
            current
        } else if current + 1 < self.queue.items.len() {
            current + 1
        } else if self.loop_playlist {
            0
        } else {
            current
        };
        for i in [current, next] {
            let Some(track) = self.queue.items.get(i) else { continue };
            let part = track.part_key.as_deref().and_then(leveling::part_id);
            if part.is_none_or(|p| self.leveling.published.contains(p)) {
                continue;
            }
            let track = track.clone();
            match self.full_audio(&track) {
                Some(audio) => self.publish_gain(&track, &audio),
                None if i != current && self.leveling.asked.insert(track.rating_key.clone()) => {
                    let _ = self.plex.tx.send(PlexCmd::LoadTrackInfo(track.rating_key));
                }
                None => {}
            }
        }
    }

    pub(super) fn level_track_info(&mut self, rating_key: &str, audio: &AudioInfo) {
        if self.leveling.mode == Leveling::Off {
            return;
        }
        let Some(track) = self.queue.items.iter().find(|t| t.rating_key == rating_key).cloned()
        else {
            return;
        };
        self.publish_gain(&track, audio);
    }

    pub(super) fn check_leveling_loaded(&mut self) {
        if self.leveling.mode == Leveling::Off || self.leveling.ready || self.leveling.warned {
            return;
        }
        self.leveling.warned = true;
        tracing::warn!("the leveling script did not load");
        self.transient_err("loudness leveling is off: it needs mpv 0.37 or newer with Lua".into());
    }

    fn full_audio(&self, track: &Track) -> Option<AudioInfo> {
        if is_full(&track.audio) {
            return Some(track.audio.clone());
        }
        self.audio_cache.get(&track.rating_key).cloned()
    }

    fn publish_gain(&mut self, track: &Track, audio: &AudioInfo) {
        let Some(part) = track.part_key.as_deref().and_then(leveling::part_id) else { return };
        if !self.leveling.published.insert(part.to_string()) {
            return;
        }
        let db = self.leveling.mode.gain_db(&audio.loudness, self.leveling.preamp_db);
        let _ = self.mpv.tx.send(MpvCmd::SetGain { part: part.to_string(), db });
    }
}
