//! Scrolling shared by every list: one function turns the cursor into an
//! offset, and [`ScrollStore`] keeps the offsets per list.

/// Rows between the cursor and an edge before the view scrolls (vim's scrolloff).
pub const DEFAULT_SCROLL_OFF: usize = 4;

/// The margin shrinks in a short pane and gives way at the list's ends.
pub fn compute_offset(
    cursor: usize,
    offset: usize,
    visible: usize,
    total: usize,
    scrolloff: usize,
) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    let cursor = cursor.min(total.saturating_sub(1));
    if total <= visible {
        return 0;
    }
    let margin = scrolloff.min((visible.saturating_sub(1)) / 2);
    let max_off = total - visible;

    let mut off = offset.min(max_off);
    if cursor < off + margin {
        off = cursor.saturating_sub(margin);
    } else if cursor + margin >= off + visible {
        off = (cursor + margin + 1).saturating_sub(visible);
    }
    off.min(max_off)
}

/// Offset that centres `cursor` in the pane.
pub fn center_offset(cursor: usize, visible: usize, total: usize) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    let cursor = cursor.min(total.saturating_sub(1));
    cursor.saturating_sub(visible / 2).min(total.saturating_sub(visible))
}

/// Offset that puts `cursor` on the top row, or as near as the list's end allows.
pub fn top_offset(cursor: usize, visible: usize, total: usize) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    cursor.min(total.saturating_sub(1)).min(total.saturating_sub(visible))
}

/// The cursor after a page jump; `dir` is +1 down, -1 up.
pub fn page_jump(cursor: usize, visible: usize, total: usize, dir: i32) -> usize {
    if total == 0 {
        return 0;
    }
    let page = visible.max(1) as i64;
    let next = cursor as i64 + dir as i64 * page;
    next.clamp(0, total as i64 - 1) as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollId {
    Queue,
    BrowserArtists,
    BrowserAlbums,
    BrowserTracks,
    PlaylistList,
    PlaylistItems,
    SearchResults,
    SearchDetail,
    DiscoverStations,
    DiscoverTracks,
    Help,
    /// The config screen's three panes.
    ConfigSettings,
    ConfigTheme,
    ConfigChecks,
}

/// Offsets, plus the visible heights recorded at draw time for page jumps.
#[derive(Debug, Default)]
pub struct ScrollStore {
    offsets: [usize; 14],
    vis: [usize; 14],
    /// The next draw centres this list's cursor.
    pub center_once: Option<ScrollId>,
    /// Queue auto-follow: the cursor stays on the top row until the user browses.
    pub top_lock: Option<ScrollId>,
    /// Like `top_lock`, for one draw (`o` with auto-follow off).
    pub top_once: Option<ScrollId>,
    /// Kept after release so the first step does not rewind the view by the margin.
    top_anchor: Option<(ScrollId, usize)>,
}

impl ScrollStore {
    pub fn offset(&self, id: ScrollId) -> usize {
        self.offsets[id as usize]
    }

    /// Pin the cursor to the top row ('o', or a track change under auto-follow).
    pub fn pin_top(&mut self, id: ScrollId) {
        self.top_lock = Some(id);
        self.top_once = None;
        self.top_anchor = None;
    }

    /// Pin the cursor to the top row for one draw.
    pub fn pin_top_once(&mut self, id: ScrollId) {
        self.top_lock = None;
        self.top_once = Some(id);
        self.top_anchor = None;
    }

    /// The user browsed: drop the lock and any one-shot, keep the anchor.
    pub fn release_top(&mut self, id: ScrollId) {
        if self.top_lock == Some(id) {
            self.top_lock = None;
        }
        if self.top_once == Some(id) {
            self.top_once = None;
        }
    }

    /// Forget every top alignment for `id`.
    pub fn clear_top(&mut self, id: ScrollId) {
        if self.top_lock == Some(id) {
            self.top_lock = None;
        }
        if self.top_once == Some(id) {
            self.top_once = None;
        }
        if matches!(self.top_anchor, Some((anchor_id, _)) if anchor_id == id) {
            self.top_anchor = None;
        }
    }

    pub fn set_offset(&mut self, id: ScrollId, off: usize) {
        self.offsets[id as usize] = off;
    }

    /// Rows visible as of the last draw.
    pub fn visible(&self, id: ScrollId) -> usize {
        self.vis[id as usize]
    }

    /// Compute and store this frame's offset.
    pub fn update(
        &mut self,
        id: ScrollId,
        cursor: usize,
        visible: usize,
        total: usize,
        scrolloff: usize,
    ) -> usize {
        self.vis[id as usize] = visible;
        let off = if self.top_once == Some(id) {
            self.top_once = None;
            let off = top_offset(cursor, visible, total);
            self.top_anchor = Some((id, off));
            off
        } else if self.top_lock == Some(id) {
            let off = top_offset(cursor, visible, total);
            self.top_anchor = Some((id, off));
            off
        } else if self.anchor_holds(id, cursor, visible, scrolloff) {
            self.offsets[id as usize].min(total.saturating_sub(visible))
        } else if self.center_once == Some(id) {
            self.center_once = None;
            center_offset(cursor, visible, total)
        } else {
            compute_offset(cursor, self.offsets[id as usize], visible, total, scrolloff)
        };
        self.offsets[id as usize] = off;
        off
    }

    /// The released pin holds while the cursor is in the pane, short of the bottom margin.
    fn anchor_holds(
        &mut self,
        id: ScrollId,
        cursor: usize,
        visible: usize,
        scrolloff: usize,
    ) -> bool {
        let Some((anchor_id, anchor)) = self.top_anchor else {
            return false;
        };
        if anchor_id != id {
            return false;
        }
        let margin = scrolloff.min((visible.saturating_sub(1)) / 2);
        if visible > 0 && cursor >= anchor && cursor + margin < anchor + visible {
            true
        } else {
            self.top_anchor = None;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_edge() {
        assert_eq!(compute_offset(1, 3, 10, 30, 4), 0);
        assert_eq!(compute_offset(0, 0, 10, 30, 4), 0);
        assert_eq!(compute_offset(5, 10, 10, 30, 4), 1);
    }

    #[test]
    fn bottom_edge() {
        assert_eq!(compute_offset(29, 0, 10, 30, 4), 20);
        assert_eq!(compute_offset(25, 10, 10, 30, 4), 20);
        assert_eq!(compute_offset(15, 10, 10, 30, 4), 10);
    }

    #[test]
    fn short_list_fits_never_scrolls() {
        assert_eq!(compute_offset(3, 0, 20, 8, 4), 0);
        assert_eq!(compute_offset(7, 5, 8, 8, 4), 0);
        assert_eq!(compute_offset(0, 0, 10, 0, 4), 0);
    }

    #[test]
    fn tiny_pane_clamps_margin() {
        // pane of 3 rows: margin clamps to (3-1)/2 = 1
        assert_eq!(compute_offset(5, 0, 3, 30, 4), 4);
        assert_eq!(compute_offset(28, 29, 3, 30, 4), 27);
        assert_eq!(compute_offset(10, 0, 1, 30, 4), 10);
        assert_eq!(compute_offset(0, 0, 0, 30, 4), 0);
    }

    #[test]
    fn page_jump_keeps_relative_row() {
        assert_eq!(page_jump(0, 20, 100, 1), 20);
        assert_eq!(page_jump(50, 20, 100, -1), 30);
        assert_eq!(page_jump(95, 20, 100, 1), 99); // clamps at end
        assert_eq!(page_jump(3, 20, 100, -1), 0); // clamps at start
        assert_eq!(page_jump(0, 20, 5, 1), 4); // page larger than list
    }

    #[test]
    fn center_offset_puts_cursor_mid_pane() {
        assert_eq!(center_offset(50, 20, 100), 40);
        assert_eq!(center_offset(2, 20, 100), 0);
        assert_eq!(center_offset(99, 20, 100), 80);
    }

    #[test]
    fn top_offset_puts_cursor_on_first_row() {
        assert_eq!(top_offset(50, 20, 100), 50);
        assert_eq!(top_offset(2, 20, 100), 2);
        // near the end the list clamps flush to the bottom
        assert_eq!(top_offset(99, 20, 100), 80);
        assert_eq!(top_offset(3, 20, 8), 0);
    }

    #[test]
    fn store_roundtrip_and_center_once() {
        let mut s = ScrollStore::default();
        assert_eq!(s.update(ScrollId::Queue, 10, 10, 100, 4), 5);
        s.center_once = Some(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 45);
        assert!(s.center_once.is_none(), "center flag is consumed");
        // other lists unaffected
        assert_eq!(s.offset(ScrollId::Help), 0);
    }

    #[test]
    fn top_pin_survives_repaints() {
        let mut s = ScrollStore::default();
        s.pin_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        assert_eq!(
            s.update(ScrollId::Queue, 51, 10, 100, 4),
            51,
            "top alignment must survive ordinary repaints"
        );
    }

    #[test]
    fn released_top_lock_keeps_offset() {
        let mut s = ScrollStore::default();
        s.pin_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        s.release_top(ScrollId::Queue);
        assert!(s.top_lock.is_none());
        assert_eq!(
            s.update(ScrollId::Queue, 51, 10, 100, 4),
            50,
            "the first step after `o` must not scroll the pane"
        );
        assert_eq!(
            s.update(ScrollId::Queue, 55, 10, 100, 4),
            50,
            "browsing inside the pane keeps the pinned offset"
        );
        assert_eq!(
            s.update(ScrollId::Queue, 56, 10, 100, 4),
            51,
            "normal scrolling resumes once the cursor reaches the margin"
        );
    }

    #[test]
    fn moving_up_from_pin_restores_top_margin() {
        let mut s = ScrollStore::default();
        s.pin_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        s.release_top(ScrollId::Queue);
        assert_eq!(
            s.update(ScrollId::Queue, 49, 10, 100, 4),
            45,
            "there are no rows above a pinned row, so Up restores the margin"
        );
    }

    #[test]
    fn one_shot_pin_keeps_pane_still() {
        let mut s = ScrollStore::default();
        s.pin_top_once(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        assert!(s.top_once.is_none());
        assert_eq!(
            s.update(ScrollId::Queue, 51, 10, 100, 4),
            50,
            "browsing from a one-shot jump must not scroll the pane"
        );
        assert_eq!(s.update(ScrollId::Queue, 56, 10, 100, 4), 51);
    }

    #[test]
    fn manual_move_cancels_one_shot_top() {
        let mut s = ScrollStore::default();
        s.pin_top_once(ScrollId::Queue);
        s.release_top(ScrollId::Queue);
        assert!(s.top_once.is_none());
        assert_eq!(
            s.update(ScrollId::Queue, 50, 10, 100, 4),
            45,
            "a move before the one-shot draw must not pin the new cursor"
        );
    }

    #[test]
    fn re_pinning_supersedes_a_released_anchor() {
        let mut s = ScrollStore::default();
        s.pin_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        s.release_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 55, 10, 100, 4), 50);
        s.pin_top(ScrollId::Queue);
        assert_eq!(
            s.update(ScrollId::Queue, 80, 10, 100, 4),
            80,
            "re-arming pins the new cursor immediately"
        );
    }

    #[test]
    fn clearing_the_queue_forgets_the_anchor() {
        let mut s = ScrollStore::default();
        s.pin_top(ScrollId::Queue);
        assert_eq!(s.update(ScrollId::Queue, 50, 10, 100, 4), 50);
        s.release_top(ScrollId::Queue);
        s.clear_top(ScrollId::Queue);
        assert_eq!(
            s.update(ScrollId::Queue, 51, 10, 100, 4),
            47,
            "clearing the queue drops the pinned offset too"
        );
    }
}
