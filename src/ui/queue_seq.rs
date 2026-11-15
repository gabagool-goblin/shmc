//! Request order for queue changes. Replies come back in any order, so each
//! applies only after every earlier request has applied, failed or been dropped.

use super::app::{Confirm, Msg, QueueAction};
use crate::plex::models::Track;
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;

/// What one queue-changing request came to.
#[derive(Debug)]
pub enum QueueOutcome {
    Tracks {
        tracks: Vec<Track>,
        confirm: Confirm,
        action: QueueAction,
    },
    /// First page of a streamed replace; the rest follows as
    /// `Msg::PlaylistPage` tagged `load_id`. `total` is for the progress.
    Streaming {
        tracks: Vec<Track>,
        confirm: Confirm,
        load_id: u64,
        total: usize,
    },
    Failed(String),
    /// The playlist changed on the server and the picked start track is gone.
    StartMissing,
}

#[derive(Debug, Default)]
pub struct QueueSeq {
    next: u64,
    /// Everything below is settled or superseded.
    apply_from: u64,
    ready: BTreeMap<u64, QueueOutcome>,
}

impl QueueSeq {
    /// A replace supersedes everything asked before it, in flight or waiting.
    pub fn issue(&mut self, replaces: bool) -> u64 {
        let id = self.next;
        self.next += 1;
        if replaces {
            self.apply_from = id;
            self.ready.clear();
        }
        id
    }

    /// Drop everything in flight without issuing a request (a clear).
    pub fn supersede(&mut self) {
        self.apply_from = self.next;
        self.ready.clear();
    }

    /// A request was asked for and not yet applied or dropped.
    pub fn pending(&self) -> bool {
        self.apply_from < self.next
    }

    /// The outcomes now due, in request order; empty when the reply is stale or
    /// an earlier request is still pending.
    pub fn deliver(&mut self, id: u64, outcome: QueueOutcome) -> Vec<QueueOutcome> {
        if id < self.apply_from {
            tracing::debug!(id, apply_from = self.apply_from, "dropping a superseded queue reply");
            return Vec::new();
        }
        self.ready.insert(id, outcome);
        let mut due = Vec::new();
        while let Some(o) = self.ready.remove(&self.apply_from) {
            self.apply_from += 1;
            due.push(o);
        }
        due
    }
}

/// Every request must answer exactly once or later ones wait forever, so a
/// task that returns early or panics still sends a failure from `Drop`.
pub struct QueueReplyGuard {
    tx: UnboundedSender<Msg>,
    seq: u64,
    sent: bool,
}

impl QueueReplyGuard {
    pub fn new(tx: UnboundedSender<Msg>, seq: u64) -> Self {
        Self { tx, seq, sent: false }
    }

    pub fn send(mut self, outcome: QueueOutcome) {
        self.sent = true;
        let _ = self.tx.send(Msg::QueueReply { seq: self.seq, outcome });
    }
}

impl Drop for QueueReplyGuard {
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.tx.send(Msg::QueueReply {
                seq: self.seq,
                outcome: QueueOutcome::Failed("request stopped before it finished".into()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(s: &str) -> QueueOutcome {
        QueueOutcome::Failed(s.into())
    }

    fn texts(v: Vec<QueueOutcome>) -> Vec<String> {
        v.into_iter()
            .map(|o| match o {
                QueueOutcome::Failed(s) => s,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn out_of_order_replies_wait_for_older() {
        let mut s = QueueSeq::default();
        let ids: Vec<u64> = (0..3).map(|_| s.issue(false)).collect();
        assert!(s.deliver(ids[2], failed("c")).is_empty());
        assert!(s.deliver(ids[1], failed("b")).is_empty());
        assert_eq!(texts(s.deliver(ids[0], failed("a"))), ["a", "b", "c"]);
    }

    #[test]
    fn replace_supersedes_earlier_requests() {
        let mut s = QueueSeq::default();
        let old = s.issue(false);
        let early = s.issue(false);
        assert!(s.deliver(early, failed("early")).is_empty());
        let rep = s.issue(true);
        assert!(s.deliver(old, failed("old")).is_empty());
        assert_eq!(texts(s.deliver(rep, failed("rep"))), ["rep"]);
        // a late straggler stays dropped
        assert!(s.deliver(early, failed("early")).is_empty());
    }

    #[test]
    fn requests_after_a_replace_wait_for_it() {
        let mut s = QueueSeq::default();
        let rep = s.issue(true);
        let app = s.issue(false);
        assert!(s.deliver(app, failed("append")).is_empty());
        assert_eq!(texts(s.deliver(rep, failed("rep"))), ["rep", "append"]);
    }

    #[test]
    fn supersede_drops_everything_in_flight() {
        let mut s = QueueSeq::default();
        let a = s.issue(false);
        let b = s.issue(false);
        assert!(s.deliver(b, failed("b")).is_empty());
        s.supersede();
        assert!(s.deliver(a, failed("a")).is_empty());
        let c = s.issue(false);
        assert_eq!(texts(s.deliver(c, failed("c"))), ["c"]);
    }

    #[test]
    fn guard_replies_exactly_once() {
        // dropped without sending (early return or panic)
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        drop(QueueReplyGuard::new(tx, 7));
        match rx.try_recv() {
            Ok(Msg::QueueReply { seq: 7, outcome: QueueOutcome::Failed(m) }) => {
                assert!(m.contains("stopped before it finished"), "{m}");
            }
            _ => panic!("the drop guard must send a failure"),
        }
        assert!(rx.try_recv().is_err(), "exactly one reply");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        QueueReplyGuard::new(tx, 1).send(QueueOutcome::StartMissing);
        assert!(matches!(
            rx.try_recv(),
            Ok(Msg::QueueReply { seq: 1, outcome: QueueOutcome::StartMissing })
        ));
        assert!(rx.try_recv().is_err(), "exactly one reply");
    }
}
