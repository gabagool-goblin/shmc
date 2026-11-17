use super::{App, Confirm, ListRef, QueueAction, ShuffleSnapshot, Visual, MAX_UNDO};
use crate::playback::{LoadItem, MpvCmd};
use crate::plex::{AudioInfo, Track};
use crate::ui::keymap::{Act, KeyCtx};
use crate::ui::queue_seq::QueueOutcome;
use crate::ui::scroll::ScrollId;
use std::time::{Duration, Instant};

/// The `playlist-pos` mpv should report after a change shmc made. A consume
/// or a reorder makes mpv report stale positions first; those must be ignored.
pub(super) struct PosGuard {
    pub(super) expected: usize,
    pub(super) ignores: u32,
    /// Consume leaves at most two stale events; a reorder can leave thousands,
    /// so it is bounded by `until`.
    pub(super) max_ignores: Option<u32>,
    /// No guard may outlive its burst or it would swallow unrelated events.
    pub(super) until: Instant,
}

/// Both bursts are over in milliseconds; this is a safety net.
const POS_GUARD_TTL: Duration = Duration::from_secs(2);

impl PosGuard {
    pub(super) fn consume(expected: usize) -> Self {
        Self { expected, ignores: 0, max_ignores: Some(2), until: Instant::now() + POS_GUARD_TTL }
    }

    pub(super) fn reorder(expected: usize) -> Self {
        Self { expected, ignores: 0, max_ignores: None, until: Instant::now() + POS_GUARD_TTL }
    }
}

impl App {
    /// Failures are shown here, not sent as `Msg::Error`, so they stay in
    /// request order.
    pub(super) fn apply_queue_outcome(&mut self, outcome: QueueOutcome) {
        match outcome {
            QueueOutcome::Tracks { tracks, confirm, action } => match action {
                QueueAction::Append => self.add_tracks(tracks, confirm),
                QueueAction::Replace => {
                    // a playlist that fits in its first page gets plain `Tracks`
                    // and no pages, so its load ends here
                    self.playlist_load = None;
                    self.replace_queue_and_play(tracks, confirm);
                }
                QueueAction::PlayNext => self.play_next_tracks(tracks, confirm),
            },
            QueueOutcome::Streaming { tracks, confirm, load_id, total } => {
                let n = tracks.len();
                if self.replace_queue_and_play(tracks, confirm) {
                    if let Some(l) = self.playlist_load.as_mut().filter(|l| l.id == load_id) {
                        l.landed = true;
                        l.loaded = n;
                        l.total = total.max(n);
                    }
                } else {
                    self.cancel_playlist_load();
                }
            }
            QueueOutcome::Failed(e) => {
                tracing::error!(e);
                self.transient_err(e);
            }
            QueueOutcome::StartMissing => {
                let mut msg = String::from("that track is no longer in the playlist");
                if let Some(key) = self.keymap.first_key(KeyCtx::Playlists, &Act::Activate) {
                    msg.push_str(&format!(
                        "; press {key} on the playlist to play it from the start"
                    ));
                }
                self.transient_err(msg);
            }
        }
    }

    /// Tracks already in hand still take a request number, so they land after
    /// earlier requests that are loading.
    pub(super) fn queue_local(
        &mut self,
        tracks: Vec<Track>,
        confirm: Confirm,
        action: QueueAction,
    ) {
        let id = self.issue_seq(action == QueueAction::Replace);
        let due = self.queue_seq.deliver(id, QueueOutcome::Tracks { tracks, confirm, action });
        if due.is_empty() {
            self.transient("queued after the request that is still loading".into());
            return;
        }
        for o in due {
            self.apply_queue_outcome(o);
        }
    }

    /// Appends without playing.
    pub fn add_tracks(&mut self, tracks: Vec<Track>, confirm: Confirm) {
        match self.append_playable(tracks) {
            0 => self.transient_err("no playable media for selection".into()),
            n => self.transient(confirm.added(n)),
        }
    }

    /// The tracks that can stream, with their URLs. Built together so the
    /// queue and mpv's playlist cannot drift apart by one.
    fn playable(&self, tracks: Vec<Track>) -> (Vec<Track>, Vec<LoadItem>) {
        tracks
            .into_iter()
            .filter_map(|t| {
                let item = self.load_item(&t)?;
                Some((t, item))
            })
            .unzip()
    }

    fn load_item(&self, t: &Track) -> Option<LoadItem> {
        Some(LoadItem { url: t.stream_url(&self.server, &self.token)?, title: t.title.clone() })
    }

    /// New tracks join the end of the pre-shuffle order, so turning shuffle
    /// off keeps them.
    fn extend_shuffle_snapshot(&mut self, tracks: &[Track]) {
        if !self.random {
            return;
        }
        match self.shuffle_snapshot.as_mut() {
            Some(snapshot) => snapshot.order.extend_from_slice(tracks),
            None => self.shuffle_snapshot = Some(ShuffleSnapshot { order: tracks.to_vec() }),
        }
    }

    /// Put the playable `tracks` at the end of the app queue and of mpv's
    /// playlist, in step. Returns how many there were.
    fn append_playable(&mut self, tracks: Vec<Track>) -> usize {
        let (playable, items) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            return 0;
        }
        self.extend_shuffle_snapshot(&playable);
        self.queue.extend(playable);
        self.removed.clear();
        let _ = self.mpv.tx.send(MpvCmd::Append(items));
        self.refilter(ListRef::Queue);
        if self.queue_state.selected().is_none() {
            self.queue_state.select(Some(0));
        }
        n
    }

    /// Dropped when its load was superseded.
    pub(super) fn apply_playlist_page(&mut self, load_id: u64, tracks: Vec<Track>, last: bool) {
        let Some(l) = self.playlist_load.as_mut().filter(|l| l.id == load_id && l.landed) else {
            tracing::debug!(load_id, "dropping a page of a superseded playlist load");
            return;
        };
        l.loaded += tracks.len();
        l.total = l.total.max(l.loaded);
        let progress = l.progress();
        self.append_playable(tracks);
        if last {
            self.playlist_load = None;
            self.dismiss_status();
        } else {
            self.transient(format!("loading playlist {progress}"));
        }
    }

    /// Forget the load (late pages are ignored) and stop the worker.
    fn cancel_playlist_load(&mut self) {
        if let Some(l) = self.playlist_load.take() {
            l.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// A replace also supersedes a playlist that is still loading.
    pub(super) fn issue_seq(&mut self, replaces: bool) -> u64 {
        if replaces {
            self.cancel_playlist_load();
        }
        self.queue_seq.issue(replaces)
    }

    pub(super) fn supersede_queue(&mut self) {
        self.cancel_playlist_load();
        self.queue_seq.supersede();
    }

    /// Anything that adds, reorders or reshuffles would land among tracks that
    /// have not arrived yet. Says so and returns true.
    pub(super) fn refuse_while_loading(&mut self) -> bool {
        let Some(l) = &self.playlist_load else { return false };
        let msg = if l.landed {
            format!("playlist still loading ({})", l.progress())
        } else {
            "playlist still loading".into()
        };
        self.transient(msg);
        true
    }

    pub fn replace_queue_and_play(&mut self, tracks: Vec<Track>, confirm: Confirm) -> bool {
        let (mut playable, _) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            self.transient_err("no playable media for selection".into());
            return false;
        }
        self.shuffle_new_queue(&mut playable);
        let (playable, items) = self.playable(playable);
        self.queue = playable;
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        let _ = self.mpv.tx.send(MpvCmd::Play(items));
        self.current = Some(0);
        self.expect_pos = None;
        self.stopped = false;
        self.position = 0.0;
        self.failed_at = None;
        self.err_streak = 0;
        self.queue_filter.clear();
        self.refilter(ListRef::Queue);
        self.queue_state.select(Some(0));
        self.fetch_art_for_current();
        self.fetch_audio_for_current();
        self.transient(confirm.playing(n));
        self.follow_queue();
        true
    }

    /// Inserts after the current track without interrupting it.
    pub fn play_next_tracks(&mut self, tracks: Vec<Track>, confirm: Confirm) {
        let (playable, items) = self.playable(tracks);
        let n = playable.len();
        if n == 0 {
            self.transient_err("no playable media for selection".into());
            return;
        }
        if self.queue.is_empty() && self.current.is_none() {
            self.replace_queue_and_play(playable, confirm);
            return;
        }
        // `current` can be one past the end after a consume, before mpv corrects it
        let index = self.current.map_or(0, |c| (c + 1).min(self.queue.len()));
        let append_at = self.queue.len();
        self.expect_pos = None;
        self.extend_shuffle_snapshot(&playable);
        let _ = self.mpv.tx.send(MpvCmd::Insert { index, append_at, items });
        self.queue.splice(index..index, playable);
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        // inserting shifts later indices (see `failed_at`)
        self.failed_at = None;
        self.refilter(ListRef::Queue);
        self.select_queue_row(index);
        self.transient(format!("queued next: {n} track{}", if n == 1 { "" } else { "s" }));
    }

    /// Put the queue cursor on real index `real`. The cursor counts filtered
    /// rows, so a row the filter hides keeps the old cursor, clamped.
    fn select_queue_row(&mut self, real: usize) {
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
            return;
        }
        let sel = self
            .queue_filter
            .cursor_of(real)
            .unwrap_or_else(|| self.queue_state.selected().unwrap_or(0));
        self.queue_state.select(Some(sel.min(vis - 1)));
    }

    pub fn play_queue_index(&mut self, idx: usize) {
        if idx >= self.queue.len() {
            return;
        }
        let _ = self.mpv.tx.send(MpvCmd::PlayIndex(idx));
        self.current = Some(idx);
        self.expect_pos = None;
        self.stopped = false;
        self.failed_at = None;
        self.err_streak = 0;
        self.select_queue_row(idx);
        self.fetch_art_for_current();
        self.fetch_audio_for_current();
        self.follow_queue();
    }

    /// Where index `i` lands when the entry at `from` moves to `to`.
    fn shifted(i: usize, from: usize, to: usize) -> usize {
        if i == from {
            to
        } else if from < i && to >= i {
            i - 1
        } else if from > i && to <= i {
            i + 1
        } else {
            i
        }
    }

    /// J/K. Refused while filtered: the visible row would silently name a
    /// different slot.
    pub(super) fn move_queue(&mut self, dir: i32) {
        if self.random {
            self.transient("turn shuffle off to reorder".into());
            return;
        }
        if self.queue_filter.is_active() {
            self.transient("clear the queue filter to reorder".into());
            return;
        }
        let Some(from) = self.real_at(ListRef::Queue) else { return };
        let to = if dir < 0 {
            let Some(t) = from.checked_sub(1) else { return };
            t
        } else if from + 1 < self.queue.len() {
            from + 1
        } else {
            return;
        };
        let item = self.queue.remove(from);
        self.queue.insert(to, item);
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        let _ = self.mpv.tx.send(MpvCmd::Move { from, to });
        if let Some(c) = self.current {
            self.current = Some(Self::shifted(c, from, to));
        }
        if let Some(f) = self.failed_at {
            self.failed_at = Some(Self::shifted(f, from, to));
        }
        self.refilter(ListRef::Queue);
        self.queue_state.select(Some(to.min(self.vis_len(ListRef::Queue).saturating_sub(1))));
    }

    pub(super) fn start_visual(&mut self) {
        let l = self.active_list();
        if !matches!(
            l,
            ListRef::Queue | ListRef::BTracks | ListRef::PlItems | ListRef::SearchDetail
        ) {
            self.transient("visual mode works in track lists".into());
            return;
        }
        if self.cursor_of(l).is_none() {
            return;
        }
        let Some(anchor) = self.real_at(l) else { return };
        self.visual = Some(Visual { list: l, anchor });
        let ctx = self.key_ctx();
        let acts: &[(&Act, &str)] = if l == ListRef::Queue {
            &[(&Act::RemoveFromQueue, "remove")]
        } else {
            &[(&Act::AddToQueue, "add"), (&Act::PlayNext, "insert"), (&Act::Activate, "play")]
        };
        let mut hint = String::from("visual: move to select");
        for (act, label) in acts {
            if let Some(key) = self.keymap.first_key(ctx, act) {
                hint.push_str(&format!(" · {key} {label}"));
            }
        }
        hint.push_str(" · Esc cancel");
        self.transient(hint);
    }

    pub(super) fn drop_visual(&mut self, l: ListRef) {
        if self.visual.is_some_and(|v| v.list == l) {
            self.visual = None;
        }
    }

    /// The selection as inclusive visible-row positions; None while the anchor
    /// is filtered out or the cursor is in another list.
    pub fn visual_range(&self) -> Option<(usize, usize)> {
        let v = self.visual?;
        if v.list != self.active_list() {
            return None;
        }
        let a = match self.filter(v.list) {
            Some(f) => f.cursor_of(v.anchor)?,
            None => (v.anchor < self.total_len(v.list)).then_some(v.anchor)?,
        };
        let c = self.cursor_of(v.list)?;
        Some((a.min(c), a.max(c)))
    }

    pub(super) fn remove_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            self.visual = None;
            return;
        };
        let len = self.queue.len();
        let indices: Vec<usize> = (lo..=hi).map(|fi| self.queue_filter.real(fi, len)).collect();
        self.remove_indices(&indices);
    }

    /// Queues a track-list selection and leaves visual mode; a refused add
    /// keeps it.
    pub(super) fn queue_visual_range(&mut self, action: QueueAction) {
        if action != QueueAction::Replace && self.refuse_while_loading() {
            return;
        }
        let Some((lo, hi)) = self.visual_range() else {
            self.visual = None;
            return;
        };
        let Some(l) = self.visual.map(|v| v.list) else { return };
        let (items, filter) = match (l, self.search.detail.as_ref()) {
            (ListRef::BTracks, _) => (&self.browser.tracks, Some(&self.browser_filters[2])),
            (ListRef::PlItems, _) => (&self.playlists.items, Some(&self.pl_items_filter)),
            (ListRef::SearchDetail, Some(d)) => (&d.tracks, None),
            _ => return,
        };
        let tracks: Vec<Track> = (lo..=hi)
            .map(|fi| items[filter.map_or(fi, |f| f.real(fi, items.len()))].clone())
            .collect();
        self.visual = None;
        let confirm = match tracks.as_slice() {
            [t] => Confirm::Track { title: t.title.clone() },
            _ => Confirm::Tracks,
        };
        self.queue_local(tracks, confirm, action);
    }

    pub fn remove_from_queue(&mut self, idx: usize) {
        self.remove_indices(&[idx]);
    }

    /// `indices`: ascending, unique, real queue indices.
    fn remove_indices(&mut self, indices: &[usize]) {
        if indices.is_empty() || indices.last().is_some_and(|&i| i >= self.queue.len()) {
            return;
        }
        self.drop_visual(ListRef::Queue);
        if self.removed.len() == MAX_UNDO {
            self.removed.remove(0);
        }
        self.removed.push(indices.iter().map(|&i| (i, self.queue[i].clone())).collect());
        // descending: each index is still valid when mpv gets to it
        for &i in indices.iter().rev() {
            self.queue.remove(i);
            let _ = self.mpv.tx.send(MpvCmd::Remove(i));
        }
        self.failed_at = None;
        self.expect_pos = None;
        let first_row = indices
            .first()
            .and_then(|&i| self.queue_filter.cursor_of(i))
            .or(self.queue_state.selected())
            .unwrap_or(0);
        // the playing track was pulled out and nothing has replaced it yet
        let mut resume = None;
        if let Some(c) = self.current {
            let below = indices.iter().filter(|&&i| i < c).count();
            if indices.binary_search(&c).is_ok() {
                // mpv stops when its playing entry is removed; hand over to the
                // track that slid into the slot
                self.current = None;
                resume = Some(c - below);
            } else {
                self.current = Some(c - below);
                if below >= 2 {
                    self.expect_pos = Some(PosGuard::reorder(c - below));
                }
            }
        }
        self.refilter(ListRef::Queue);
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
        } else {
            self.queue_state.select(Some(first_row.min(vis - 1)));
        }
        if let Some(idx) = resume {
            if idx < self.queue.len() {
                // the cursor stays put; `o` jumps to the playing track
                let _ = self.mpv.tx.send(MpvCmd::PlayIndex(idx));
                self.current = Some(idx);
                self.stopped = false;
                self.failed_at = None;
                self.err_streak = 0;
                self.fetch_art_for_current();
                self.fetch_audio_for_current();
                self.follow_queue();
            } else {
                self.position = 0.0;
                self.duration = 0.0;
                self.stopped = true;
                self.audio = AudioInfo::default();
            }
        }
    }

    pub fn undo_remove(&mut self) {
        let Some(batch) = self.removed.last() else {
            self.transient("nothing to undo".into());
            return;
        };
        let Some(items) = batch.iter().map(|(_, t)| self.load_item(t)).collect::<Option<Vec<_>>>()
        else {
            self.transient_err("cannot restore: no playable media".into());
            return;
        };
        let Some(batch) = self.removed.pop() else { return };
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        self.failed_at = None;
        let old_current = self.current;
        let first_index = batch[0].0.min(self.queue.len());
        let mut start = 0;
        while start < batch.len() {
            let mut end = start + 1;
            while end < batch.len() && batch[end].0 == batch[end - 1].0 + 1 {
                end += 1;
            }
            let len = self.queue.len();
            let index = batch[start].0.min(len);
            let _ = self.mpv.tx.send(MpvCmd::Insert {
                index,
                append_at: len,
                items: items[start..end].to_vec(),
            });
            self.queue.splice(index..index, batch[start..end].iter().map(|(_, t)| t.clone()));
            if let Some(c) = self.current {
                if index <= c {
                    self.current = Some(c + (end - start));
                }
            }
            start = end;
        }
        // Insert appends and then moves each track into place
        if let (Some(old), Some(new)) = (old_current, self.current) {
            if new - old >= 2 {
                self.expect_pos = Some(PosGuard::reorder(new));
            }
        }
        self.refilter(ListRef::Queue);
        self.select_queue_row(first_index);
        if batch.len() == 1 {
            self.transient(format!("restored: {}", batch[0].1.title));
        } else {
            self.transient(format!("restored {} tracks", batch.len()));
        }
    }

    /// The track at `idx` finished and mpv has already advanced. Never send
    /// `playlist-play-index` here: it would restart the next track at 0:00.
    pub(super) fn consume_finished(&mut self, idx: usize) {
        if idx >= self.queue.len() {
            return;
        }
        self.removed.clear();
        // the anchor stays on its track, or on the next one if that was it
        self.visual = self.visual.and_then(|v| {
            if v.list != ListRef::Queue {
                return Some(v);
            }
            let a = if idx < v.anchor { v.anchor - 1 } else { v.anchor };
            (self.queue.len() > 1).then(|| Visual { anchor: a.min(self.queue.len() - 2), ..v })
        });
        // The cursor counts filtered rows: remember which item it is on.
        let sel_real = self.real_at(ListRef::Queue);
        self.queue.remove(idx);
        self.failed_at = None;
        let _ = self.mpv.tx.send(MpvCmd::Remove(idx));

        self.refilter(ListRef::Queue);
        if let Some(r) = sel_real.filter(|&r| r != idx) {
            let moved = if r > idx { r - 1 } else { r };
            if let Some(c) = self.queue_filter.cursor_of(moved) {
                self.queue_state.select(Some(c));
            }
        }

        let next = if idx < self.queue.len() {
            Some(idx)
        } else if self.loop_playlist && !self.queue.is_empty() {
            // repeat: mpv wraps to the first remaining entry
            Some(0)
        } else {
            None
        };
        // mpv may report the pre-removal position of the next track
        self.expect_pos = next.map(PosGuard::consume);
        match next {
            Some(i) => {
                self.current = Some(i);
                self.stopped = false;
                self.position = 0.0;
                self.duration = 0.0;
                self.fetch_art_for_current();
                self.fetch_audio_for_current();
                self.follow_queue();
            }
            None => {
                self.current = None;
                self.stopped = true;
                self.position = 0.0;
                self.duration = 0.0;
                self.audio = AudioInfo::default();
            }
        }

        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
        } else {
            let sel = self.queue_state.selected().unwrap_or(0).min(vis - 1);
            self.queue_state.select(Some(sel));
        }
    }

    pub(super) fn ask_clear_queue(&mut self) {
        if self.queue.is_empty() {
            // nothing to lose, but a replace still loading must go
            self.supersede_queue();
            self.clear_queue();
            return;
        }
        self.pending_playlist = None;
        self.pending_clear = Some(Instant::now());
        let key = self
            .keymap
            .first_key(KeyCtx::Queue, &Act::ClearQueue)
            .unwrap_or_else(|| ":clear".into());
        self.transient_secs(
            format!(
                "clear the queue ({} tracks)? press {key} again or y to confirm · any other key cancels",
                self.queue.len()
            ),
            self.confirm_timeout_secs,
            false,
        );
    }

    pub fn clear_queue(&mut self) {
        self.queue.clear();
        self.shuffle_snapshot = None;
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.expect_pos = None;
        self.scroll.clear_top(ScrollId::Queue);
        self.current = None;
        self.stopped = true;
        self.position = 0.0;
        self.duration = 0.0;
        self.queue_state.select(None);
        self.refilter(ListRef::Queue);
        // a bare `playlist-clear` leaves mpv playing the loaded entry
        let _ = self.mpv.tx.send(MpvCmd::Stop);
        let _ = self.mpv.tx.send(MpvCmd::Clear);
        self.art.clear();
        self.audio = AudioInfo::default();
        self.cava_frame = vec![0; crate::playback::cava::BARS];
        self.failed_at = None;
        self.err_streak = 0;
        self.transient("queue cleared".into());
    }
}
