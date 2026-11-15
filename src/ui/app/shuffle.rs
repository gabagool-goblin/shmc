use super::queue::PosGuard;
use super::{App, ListRef, ShuffleSnapshot};
use crate::playback::MpvCmd;
use crate::plex::Track;
use std::collections::{HashMap, VecDeque};

/// Fisher–Yates shuffle of `[0, n)`, seeded from the clock.
fn shuffled_indices(n: usize) -> Vec<usize> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut indices: Vec<usize> = (0..n).collect();
    if n < 2 {
        return indices;
    }
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() ^ d.subsec_nanos() as u64)
        .unwrap_or(0)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    for i in (1..n).rev() {
        seed ^= seed >> 30;
        seed = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        seed ^= seed >> 27;
        seed = seed.wrapping_mul(0x94D0_49BB_1331_11EB);
        seed ^= seed >> 31;
        let j = (seed as usize) % (i + 1);
        indices.swap(i, j);
    }
    indices
}

/// The `Move`s that turn the queue into `order` (`order[new] = old`).
struct QueuePlan {
    /// `(from, to)` in send order.
    moves: Vec<(usize, usize)>,
    /// `position[old] = new`
    position: Vec<usize>,
    /// Where the playing entry ends up.
    play: Option<usize>,
    /// A move shifted the playing entry on the way, so mpv will report
    /// intermediate positions and a guard is needed.
    play_moved: bool,
}

/// Moves each track into place, slot 0 first. Tracks not yet placed keep their
/// order, so a track sits at `t` plus how many of them are below it.
fn queue_move_plan(order: &[usize], play: Option<usize>) -> Option<QueuePlan> {
    let n = order.len();
    let mut seen = vec![false; n];
    let mut position = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        // refuse before any move is sent: it would corrupt both queues
        if old >= n || seen[old] {
            return None;
        }
        seen[old] = true;
        position[old] = new;
    }

    let mut live = Fenwick::full(n);
    let mut moves = Vec::new();
    let mut play = play.filter(|&p| p < n);
    let mut play_moved = false;
    for (target, &item) in order.iter().enumerate() {
        let from = target + live.below(item);
        // `item` is still live, so every move is a backward one
        debug_assert!(from >= target);
        if from != target {
            moves.push((from, target));
            if let Some(at) = play {
                if at == from {
                    play = Some(target);
                    play_moved = true;
                } else if at < from && at >= target {
                    play = Some(at + 1);
                    play_moved = true;
                }
            }
        }
        live.remove(item);
    }

    Some(QueuePlan { moves, position, play, play_moved })
}

/// Spelled out so the crate does not need the very new `isolate_lowest_one`.
fn low_bit(i: usize) -> usize {
    1 << i.trailing_zeros()
}

/// Fenwick tree over "still to be placed": `below(i)` counts the live items
/// with a smaller original index.
struct Fenwick {
    tree: Vec<usize>,
    len: usize,
}

impl Fenwick {
    fn full(n: usize) -> Self {
        // every item live, built in O(n)
        let mut tree = vec![0usize; n + 1];
        for i in 1..=n {
            tree[i] += 1;
            let parent = i + low_bit(i);
            if parent <= n {
                tree[parent] += tree[i];
            }
        }
        Self { tree, len: n }
    }

    fn below(&self, i: usize) -> usize {
        let mut idx = i;
        let mut sum = 0;
        while idx > 0 {
            sum += self.tree[idx];
            idx -= low_bit(idx);
        }
        sum
    }

    fn remove(&mut self, i: usize) {
        let mut idx = i + 1;
        while idx <= self.len {
            self.tree[idx] -= 1;
            idx += low_bit(idx);
        }
    }
}

impl App {
    pub(super) fn toggle_shuffle(&mut self) {
        if self.random {
            self.disable_shuffle();
        } else {
            self.enable_shuffle();
        }
    }

    fn enable_shuffle(&mut self) {
        if self.random {
            return;
        }
        if self.queue.is_empty() {
            self.random = true;
            self.shuffle_snapshot = None;
            self.transient("shuffle on".into());
            return;
        }
        self.shuffle_snapshot = Some(ShuffleSnapshot { order: self.queue.clone() });
        let order = self.shuffled_order_keep_current();
        self.apply_queue_order(order);
        self.random = true;
        self.transient("shuffle on".into());
    }

    fn disable_shuffle(&mut self) {
        if !self.random {
            return;
        }
        let snapshot = self.shuffle_snapshot.take();
        if let Some(snapshot) = snapshot {
            // Match snapshot entries to queue slots by rating key: first unused
            // match, with slots bucketed per key to stay O(n).
            let order = {
                let mut remaining: HashMap<&str, VecDeque<usize>> = HashMap::new();
                for (i, t) in self.queue.iter().enumerate() {
                    remaining.entry(t.rating_key.as_str()).or_default().push_back(i);
                }
                let mut used = vec![false; self.queue.len()];
                let mut order = Vec::with_capacity(self.queue.len());
                for wanted in &snapshot.order {
                    if let Some(idx) =
                        remaining.get_mut(wanted.rating_key.as_str()).and_then(VecDeque::pop_front)
                    {
                        used[idx] = true;
                        order.push(idx);
                    }
                }
                // tracks the snapshot did not mention keep the tail slots
                for (i, used) in used.iter().enumerate() {
                    if !used {
                        order.push(i);
                    }
                }
                order
            };
            if order.len() == self.queue.len() {
                self.apply_queue_order(order);
            }
        }
        self.random = false;
        self.transient("shuffle off".into());
    }

    pub(super) fn reshuffle_queue(&mut self) {
        if self.queue.len() < 2 {
            self.transient("queue too short to shuffle".into());
            return;
        }
        if !self.random {
            self.enable_shuffle();
            return;
        }
        let order = self.shuffled_order_keep_current();
        self.apply_queue_order(order);
        self.transient("queue reshuffled".into());
    }

    pub(super) fn shuffle_new_queue(&mut self, tracks: &mut Vec<Track>) {
        if !self.random {
            self.shuffle_snapshot = None;
            return;
        }
        self.shuffle_snapshot = Some(ShuffleSnapshot { order: tracks.clone() });
        if tracks.len() < 2 {
            return;
        }
        let rest = tracks.split_off(1);
        let order = shuffled_indices(rest.len());
        for i in order {
            tracks.push(rest[i].clone());
        }
    }

    fn shuffled_order_keep_current(&self) -> Vec<usize> {
        let mut rest: Vec<usize> = (0..self.queue.len()).collect();
        let mut order = Vec::with_capacity(self.queue.len());
        if let Some(current) = self.current {
            if current < self.queue.len() {
                order.push(current);
                rest.retain(|&i| i != current);
            }
        }
        for i in shuffled_indices(rest.len()) {
            order.push(rest[i]);
        }
        order
    }

    /// Reorders the app queue and mpv's playlist in lockstep (`order[new] =
    /// old`), one track at a time from slot 0.
    fn apply_queue_order(&mut self, order: Vec<usize>) {
        if order.len() != self.queue.len() {
            return;
        }
        let old_current = self.current.filter(|&c| c < self.queue.len());
        let old_selected = self.queue_state.selected();
        // the cursor counts filtered rows; the plan is by real slot
        let old_selected_real = self.real_at(ListRef::Queue);
        let Some(plan) = queue_move_plan(&order, old_current) else {
            return;
        };
        for &(from, to) in &plan.moves {
            let _ = self.mpv.tx.send(MpvCmd::Move { from, to });
        }
        // move, don't clone: a long queue is a lot to duplicate on the UI thread
        let mut old: Vec<Option<Track>> =
            std::mem::take(&mut self.queue).into_iter().map(Some).collect();
        self.queue = order.iter().filter_map(|&i| old.get_mut(i).and_then(Option::take)).collect();
        debug_assert_eq!(self.queue.len(), order.len(), "the plan validated a permutation");
        self.removed.clear();
        self.drop_visual(ListRef::Queue);
        self.current = plan.play;
        // remap the failure marker rather than leave it on a stranger
        self.failed_at = self.failed_at.and_then(|f| plan.position.get(f).copied());
        // mpv reports every intermediate slot the playing entry crosses
        self.expect_pos = match (self.current, plan.play_moved) {
            (Some(expected), true) => Some(PosGuard::reorder(expected)),
            _ => None,
        };
        self.refilter(ListRef::Queue);
        let vis = self.vis_len(ListRef::Queue);
        if vis == 0 {
            self.queue_state.select(None);
            self.follow_queue();
            return;
        }
        // highlighted row -> new real slot -> filtered cursor; falls back to
        // the playing track, then the old cursor
        let selected = old_selected_real
            .and_then(|s| plan.position.get(s).copied())
            .and_then(|real| self.queue_filter.cursor_of(real))
            .or_else(|| self.current.and_then(|c| self.queue_filter.cursor_of(c)))
            .or_else(|| old_selected.map(|s| s.min(vis - 1)))
            .unwrap_or(0);
        self.queue_state.select(Some(selected.min(vis - 1)));
        self.follow_queue();
    }

    /// `>` while stopped: after a failure the track following the broken one,
    /// otherwise the cursor's row.
    pub(super) fn play_from_stopped(&mut self) {
        if self.queue.is_empty() {
            self.transient("queue is empty".into());
            return;
        }
        let idx = match self.failed_at {
            Some(f) if self.loop_playlist => (f + 1) % self.queue.len(),
            Some(f) => f + 1,
            None => match self.real_at(ListRef::Queue) {
                Some(real) => real,
                None => self.queue_state.selected().unwrap_or(0),
            },
        };
        if idx >= self.queue.len() {
            self.transient("end of the queue".into());
            return;
        }
        self.play_queue_index(idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// splitmix64: deterministic, so a failing random case can be replayed.
    fn prng(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn random_permutation(n: usize, seed: &mut u64) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (prng(seed) as usize) % (i + 1);
            order.swap(i, j);
        }
        order
    }

    /// The plain O(n²) version: the moves mpv must receive.
    fn reference_moves(order: &[usize]) -> (Vec<(usize, usize)>, Vec<usize>) {
        let mut working: Vec<usize> = (0..order.len()).collect();
        let mut moves = Vec::new();
        for (target, &item) in order.iter().enumerate() {
            let from =
                working.iter().position(|&i| i == item).expect("order must be a permutation");
            if from != target {
                moves.push((from, target));
                working.remove(from);
                working.insert(target, item);
            }
        }
        (moves, working)
    }

    /// `moved`: some position the entry passed through differed from its final
    /// one, i.e. mpv reports intermediate events.
    fn reference_play(order: &[usize], play: usize) -> (usize, bool) {
        let mut working: Vec<usize> = (0..order.len()).collect();
        let final_pos = order.iter().position(|&i| i == play).expect("permutation");
        let at = |w: &[usize]| w.iter().position(|&i| i == play).expect("still queued");
        let mut moved = at(&working) != final_pos;
        for (target, &item) in order.iter().enumerate() {
            let from =
                working.iter().position(|&i| i == item).expect("order must be a permutation");
            if from != target {
                working.remove(from);
                working.insert(target, item);
            }
            moved |= at(&working) != final_pos;
        }
        (final_pos, moved)
    }

    /// The app queue and mpv stay in lockstep only if they see the same moves.
    #[test]
    fn queue_move_plan_matches_the_plain_version() {
        fn check(order: &[usize], play: Option<usize>, tag: &str) {
            let plan = queue_move_plan(order, play)
                .unwrap_or_else(|| panic!("{tag}: rejected a permutation"));
            let (moves, final_order) = reference_moves(order);
            assert_eq!(plan.moves, moves, "{tag}: order = {order:?}");
            assert_eq!(final_order, order, "{tag}: the walk lands on `order`");
            for (new, &old) in order.iter().enumerate() {
                assert_eq!(plan.position[old], new, "{tag}: inverse mapping");
            }
            match play {
                Some(p) => {
                    let (final_pos, moved) = reference_play(order, p);
                    assert_eq!(plan.play, Some(final_pos), "{tag}");
                    assert_eq!(plan.play_moved, moved, "{tag}");
                }
                None => {
                    assert_eq!(plan.play, None, "{tag}");
                    assert!(!plan.play_moved, "{tag}");
                }
            }
        }

        let mut seed = 0x5EED_1234_5678_9ABC_u64;
        for round in 0..1000 {
            let n = (prng(&mut seed) % 65) as usize;
            let order = random_permutation(n, &mut seed);
            let play = (n > 0).then(|| (prng(&mut seed) as usize) % n);
            check(&order, play, &format!("round {round}"));
        }
        // larger trees, where the Fenwick tree does real work
        for n in [0, 1, 2, 3, 127, 512] {
            let order = random_permutation(n, &mut seed);
            let play = (n > 0).then(|| (prng(&mut seed) as usize) % n);
            check(&order, play, &format!("n = {n}"));
        }
    }

    /// Applying a broken order would duplicate or drop tracks in both playlists.
    #[test]
    fn queue_move_plan_refuses_a_broken_order() {
        assert!(queue_move_plan(&[0, 0], None).is_none());
        assert!(queue_move_plan(&[0, 2], None).is_none());
        assert!(queue_move_plan(&[0, 1], None).is_some());
        assert!(queue_move_plan(&[], None).is_some());
    }

    /// A quadratic restore takes seconds at 50k. The ceiling is coarse to
    /// stay unflaky under load; the scaling ratio is the real guard.
    #[test]
    fn queue_move_plan_scales_linearly() {
        let mut seed = 42_u64;
        let mut timed = |n: usize| {
            let order = random_permutation(n, &mut seed);
            let started = Instant::now();
            let plan = queue_move_plan(&order, Some(n / 2)).expect("permutation");
            let elapsed = started.elapsed();
            assert!(!plan.moves.is_empty());
            elapsed
        };
        // warm the allocator
        let _ = timed(1_000);
        let small = timed(25_000).min(timed(25_000));
        let large = timed(50_000).min(timed(50_000));
        assert!(large < Duration::from_millis(1500), "50k restore plan took {large:?}");
        assert!(
            large < small * 4 + Duration::from_millis(50),
            "doubling n scaled super-linearly: {small:?} -> {large:?}"
        );
    }
}
