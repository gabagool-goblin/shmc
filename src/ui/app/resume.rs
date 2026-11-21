//! Playing on after a dropped connection stopped playback.

use super::{App, PlexCmd};
use std::time::Duration;

const RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub(super) struct Resume {
    epoch: u64,
    waiting: Option<Waiting>,
    /// A resume was tried and nothing has loaded since.
    retried: bool,
}

#[derive(Debug)]
struct Waiting {
    at: usize,
    key: String,
    offline: bool,
}

impl App {
    /// For when failures stopped playback at `at`.
    pub(super) fn check_connection(&mut self, at: usize) {
        if self.resume.retried || self.resume.waiting.as_ref().is_some_and(|w| w.at == at) {
            return;
        }
        let Some(key) = self.queue.items.get(at).map(|t| t.rating_key.clone()) else { return };
        self.resume.epoch += 1;
        self.resume.waiting = Some(Waiting { at, key, offline: false });
        let epoch = self.resume.epoch;
        let _ = self.plex.tx.send(PlexCmd::CheckConnection { epoch, delay: Duration::ZERO });
    }

    pub(super) fn cancel_resume(&mut self) {
        self.resume.waiting = None;
        self.resume.retried = false;
    }

    pub(super) fn connection_checked(&mut self, epoch: u64, ok: bool) {
        if epoch != self.resume.epoch {
            return;
        }
        let Some(waiting) = self.resume.waiting.as_mut() else { return };
        if !ok {
            if !waiting.offline {
                waiting.offline = true;
                tracing::info!("Plex does not answer; waiting to resume");
                self.transient_err("connection lost; playback resumes when Plex answers".into());
            }
            let _ = self.plex.tx.send(PlexCmd::CheckConnection { epoch, delay: RETRY });
            return;
        }
        let Some(waiting) = self.resume.waiting.take() else { return };
        let Some(title) = self
            .queue
            .items
            .get(waiting.at)
            .filter(|t| t.rating_key == waiting.key)
            .map(|t| t.title.clone())
        else {
            return;
        };
        tracing::info!(offline = waiting.offline, "Plex answers; resuming");
        self.play_queue_index(waiting.at);
        self.resume.retried = true;
        self.transient(format!("connection back: playing {title}"));
    }
}
