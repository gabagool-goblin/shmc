//! Playback reports: when a track has played long enough to count, and what
//! to tell Plex while it plays.

use super::App;
use crate::plex::report::{PlayState, Report};

/// Seconds of real play before a track is reported; a shorter track counts
/// at 90% of its length, where Plex counts a play.
const COUNTS_AFTER: f64 = 30.0;
/// Seconds of play between reports of a playing track.
const HEARTBEAT: f64 = 10.0;
/// The largest time-pos step that is play; anything else is a seek or a stall.
const MAX_STEP: f64 = 2.0;

/// The key and duration are fixed at load, so a queue change cannot redirect reports.
pub(super) struct TrackReport {
    rating_key: String,
    duration_ms: u64,
    played: f64,
    last_pos: f64,
    paused: bool,
    started: bool,
    next_beat: f64,
}

impl TrackReport {
    fn report(&self, state: PlayState, pos_secs: f64) -> Report {
        let ms = (pos_secs.max(0.0) * 1000.0) as u64;
        Report {
            rating_key: self.rating_key.clone(),
            state,
            time_ms: ms.min(self.duration_ms),
            duration_ms: self.duration_ms,
        }
    }

    fn counts_after(&self) -> f64 {
        COUNTS_AFTER.min(self.duration_ms as f64 / 1000.0 * 0.9)
    }

    fn live_state(&self) -> PlayState {
        if self.paused {
            PlayState::Paused
        } else {
            PlayState::Playing
        }
    }
}

impl App {
    fn send_report(&self, report: Report) {
        if let Some(tx) = &self.reporter {
            let _ = tx.send(report);
        }
    }

    pub(super) fn report_open(&mut self) {
        self.report_stop();
        if self.reporter.is_none() {
            return;
        }
        let Some(track) = self.current_track() else {
            return;
        };
        let Some(duration_ms) = track.duration_ms else {
            return;
        };
        self.report = Some(TrackReport {
            rating_key: track.rating_key.clone(),
            duration_ms,
            played: 0.0,
            last_pos: 0.0,
            paused: self.paused,
            started: false,
            next_beat: 0.0,
        });
    }

    pub(super) fn report_position(&mut self, pos: f64) {
        let Some(r) = self.report.as_mut() else {
            return;
        };
        let step = pos - r.last_pos;
        r.last_pos = pos;
        let mut due = None;
        if (0.0..=MAX_STEP).contains(&step) {
            if !r.paused {
                r.played += step;
            }
        } else if r.started {
            due = Some(r.live_state());
        }
        if !r.started {
            if r.played >= r.counts_after() {
                r.started = true;
                r.next_beat = r.played + HEARTBEAT;
                due = Some(r.live_state());
            }
        } else if r.played >= r.next_beat {
            r.next_beat += HEARTBEAT;
            due = Some(r.live_state());
        }
        if let Some(state) = due {
            let report = r.report(state, pos);
            self.send_report(report);
        }
    }

    pub(super) fn report_pause(&mut self, paused: bool) {
        let Some(r) = self.report.as_mut() else {
            return;
        };
        if r.paused == paused {
            return;
        }
        r.paused = paused;
        if r.started {
            let report = r.report(r.live_state(), r.last_pos);
            self.send_report(report);
        }
    }

    /// The file ended: `eof` is a completed play, anything else a track left.
    pub(super) fn report_end(&mut self, reason: &str) {
        let Some(r) = self.report.take() else {
            return;
        };
        if r.started {
            let pos = if reason == "eof" { r.duration_ms as f64 / 1000.0 } else { r.last_pos };
            self.send_report(r.report(PlayState::Stopped, pos));
        }
    }

    /// Closes a started track where it stopped.
    pub(super) fn report_stop(&mut self) {
        let Some(r) = self.report.take() else {
            return;
        };
        if r.started {
            self.send_report(r.report(PlayState::Stopped, r.last_pos));
        }
    }

    /// Called once when the UI loop ends, so a quit still reports the stop.
    pub fn finish_reporting(&mut self) {
        self.report_stop();
    }
}
