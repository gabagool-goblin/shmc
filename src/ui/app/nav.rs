use super::{App, BrowserCol, ListRef, PendingGoto, PlaylistsCol, PlexCmd, SelState, Tab};
use crate::ui::filter::{self, ListFilter};
use crate::ui::keymap::{Act, KeyCtx};
use crate::ui::scroll::{self, ScrollId};

fn scroll_id(l: ListRef) -> ScrollId {
    match l {
        ListRef::Queue => ScrollId::Queue,
        ListRef::BArtists => ScrollId::BrowserArtists,
        ListRef::BAlbums => ScrollId::BrowserAlbums,
        ListRef::BTracks => ScrollId::BrowserTracks,
        ListRef::PlList => ScrollId::PlaylistList,
        ListRef::PlItems => ScrollId::PlaylistItems,
        ListRef::Search => ScrollId::SearchResults,
        ListRef::SearchDetail => ScrollId::SearchDetail,
        ListRef::DStations => ScrollId::DiscoverStations,
        ListRef::DTracks => ScrollId::DiscoverTracks,
        ListRef::Help => ScrollId::Help,
    }
}

/// The filter counter's label.
fn scope_label(l: ListRef) -> &'static str {
    match l {
        ListRef::Queue => "Queue",
        ListRef::BArtists => "Artists",
        ListRef::BAlbums => "Albums",
        ListRef::BTracks => "Tracks",
        ListRef::PlList => "Playlists",
        ListRef::PlItems => "Playlist tracks",
        ListRef::Search => "Search",
        ListRef::SearchDetail => "Album",
        ListRef::DStations => "Stations",
        ListRef::DTracks => "Station tracks",
        ListRef::Help => "Help",
    }
}

impl App {
    pub fn load_albums(&mut self, artist_key: String) {
        self.browser.albums_for = Some(artist_key.clone());
        self.browser.albums.clear(); // new artist = new album list
        self.browser.tracks.items.clear();
        self.drop_visual(ListRef::BTracks);
        self.browser.tracks.state.select(None);
        self.browser.tracks_for = None;
        let _ = self.plex.tx.send(PlexCmd::LoadAlbums(artist_key));
    }

    pub fn load_tracks(&mut self, album_key: String) {
        self.browser.tracks_for = Some(album_key.clone());
        self.browser.tracks.clear();
        self.drop_visual(ListRef::BTracks);
        let _ = self.plex.tx.send(PlexCmd::LoadTracks(album_key));
    }

    pub(super) fn goto_album(&mut self) {
        let t = self.queue.state.selected().and_then(|i| {
            let real = self.queue.filter.real(i, self.queue.items.len());
            self.queue.items.get(real).cloned()
        });
        let Some(t) = t else { return };
        self.reveal(t.grandparent_rating_key, t.parent_rating_key, Some(t.rating_key));
    }

    /// Search results and the album view's tracks.
    pub(super) fn reveal_search(&mut self) {
        use crate::plex::HitKind;
        if let Some(d) = self.search.detail.as_ref() {
            let Some(t) = d.state.selected().and_then(|i| d.tracks.get(i)) else { return };
            // The track's own album: a similar list's key is the seed track.
            let (artist, album, track) = (
                t.grandparent_rating_key.clone(),
                t.parent_rating_key.clone(),
                Some(t.rating_key.clone()),
            );
            self.reveal(artist, album, track);
            return;
        }
        let Some(hit) = self.search.state.selected().and_then(|i| self.search.hits.get(i)).cloned()
        else {
            return;
        };
        match hit.kind {
            HitKind::Artist => self.reveal(Some(hit.rating_key), None, None),
            HitKind::Album => self.reveal(hit.artist_key, Some(hit.rating_key), None),
            HitKind::Track => self.reveal(hit.artist_key, hit.album_key, Some(hit.rating_key)),
        }
    }

    pub(super) fn reveal(
        &mut self,
        artist_key: Option<String>,
        album_key: Option<String>,
        track_key: Option<String>,
    ) {
        let Some(artist_key) = artist_key else {
            self.transient_err("Album not found in library".into());
            return;
        };
        if track_key.is_some() && album_key.is_none() {
            self.transient_err("Album not found in library".into());
            return;
        }
        let from = self.tab;
        self.goto_tab(Tab::Browser);
        self.pending_goto =
            Some(PendingGoto { artist_key: artist_key.clone(), album_key, track_key, from });
        self.browser.focus = BrowserCol::Artists;
        let _ = self.plex.tx.send(PlexCmd::ResolveArtist(artist_key));
    }

    pub(super) fn continue_goto_artist(&mut self, artist_key: &str) {
        let wanted = matches!(&self.pending_goto, Some(p) if p.artist_key == artist_key);
        if !wanted {
            return;
        }
        let pg = self.pending_goto.take().expect("wanted checked above");
        let Some(album_key) = pg.album_key.clone() else { return };
        if let Some(pos) = self.browser.albums.items.iter().position(|a| a.rating_key == album_key)
        {
            self.browser.albums.state.select(Some(pos));
            self.browser.focus = BrowserCol::Tracks;
            self.load_tracks(album_key);
            self.pending_goto = Some(pg); // the tracks select the row
        } else {
            self.goto_tab(pg.from);
            self.transient_err("Album not found in library".into());
        }
    }

    pub(super) fn continue_goto_album(&mut self, album_key: &str) {
        let wanted =
            matches!(&self.pending_goto, Some(p) if p.album_key.as_deref() == Some(album_key));
        if !wanted {
            return;
        }
        let pg = self.pending_goto.take().expect("wanted checked above");
        let Some(track_key) = pg.track_key else { return };
        if let Some(pos) = self.browser.tracks.items.iter().position(|t| t.rating_key == track_key)
        {
            self.browser.tracks.state.select(Some(pos));
            self.scroll.center_once = Some(ScrollId::BrowserTracks);
        }
    }

    pub(super) fn active_list(&self) -> ListRef {
        match self.tab {
            Tab::Queue => ListRef::Queue,
            Tab::Browser => match self.browser.focus {
                BrowserCol::Artists => ListRef::BArtists,
                BrowserCol::Albums => ListRef::BAlbums,
                BrowserCol::Tracks => ListRef::BTracks,
            },
            Tab::Discover => self.discover_list(),
            Tab::Playlists => match self.playlists.focus {
                PlaylistsCol::List => ListRef::PlList,
                PlaylistsCol::Items => ListRef::PlItems,
            },
            Tab::Search => {
                if self.search.detail.is_some() {
                    ListRef::SearchDetail
                } else {
                    ListRef::Search
                }
            }
            Tab::Help => ListRef::Help,
        }
    }

    pub(super) fn filter(&self, l: ListRef) -> Option<&ListFilter> {
        match l {
            ListRef::Queue => Some(&self.queue.filter),
            ListRef::BArtists => Some(&self.browser.artists.filter),
            ListRef::BAlbums => Some(&self.browser.albums.filter),
            ListRef::BTracks => Some(&self.browser.tracks.filter),
            ListRef::PlList => Some(&self.playlists.list.filter),
            ListRef::PlItems => Some(&self.playlists.tracks.filter),
            ListRef::Search
            | ListRef::SearchDetail
            | ListRef::DStations
            | ListRef::DTracks
            | ListRef::Help => None,
        }
    }

    pub(super) fn filter_mut(&mut self, l: ListRef) -> Option<&mut ListFilter> {
        match l {
            ListRef::Queue => Some(&mut self.queue.filter),
            ListRef::BArtists => Some(&mut self.browser.artists.filter),
            ListRef::BAlbums => Some(&mut self.browser.albums.filter),
            ListRef::BTracks => Some(&mut self.browser.tracks.filter),
            ListRef::PlList => Some(&mut self.playlists.list.filter),
            ListRef::PlItems => Some(&mut self.playlists.tracks.filter),
            ListRef::Search
            | ListRef::SearchDetail
            | ListRef::DStations
            | ListRef::DTracks
            | ListRef::Help => None,
        }
    }

    pub(super) fn total_len(&self, l: ListRef) -> usize {
        match l {
            ListRef::Queue => self.queue.items.len(),
            ListRef::BArtists => self.browser.artists.items.len(),
            ListRef::BAlbums => self.browser.albums.items.len(),
            ListRef::BTracks => self.browser.tracks.items.len(),
            ListRef::PlList => self.playlists.list.items.len(),
            ListRef::PlItems => self.playlists.tracks.items.len(),
            ListRef::Search => self.search.hits.len(),
            ListRef::SearchDetail => {
                self.search.detail.as_ref().map(|d| d.tracks.len()).unwrap_or(0)
            }
            ListRef::DStations => self.discover.stations.len(),
            ListRef::DTracks => self.discover.tracks.len(),
            ListRef::Help => self.keymap.help_lines().len(),
        }
    }

    pub(super) fn vis_len(&self, l: ListRef) -> usize {
        match self.filter(l) {
            Some(f) => f.len(self.total_len(l)),
            None => self.total_len(l),
        }
    }

    /// An index into the filtered view.
    pub(super) fn cursor_of(&self, l: ListRef) -> Option<usize> {
        match l {
            ListRef::Queue => self.queue.state.selected(),
            ListRef::BArtists => self.browser.artists.state.selected(),
            ListRef::BAlbums => self.browser.albums.state.selected(),
            ListRef::BTracks => self.browser.tracks.state.selected(),
            ListRef::PlList => self.playlists.list.state.selected(),
            ListRef::PlItems => self.playlists.tracks.state.selected(),
            ListRef::Search => self.search.state.selected(),
            ListRef::SearchDetail => self.search.detail.as_ref().and_then(|d| d.state.selected()),
            ListRef::DStations => self.discover.stations_state.selected(),
            ListRef::DTracks => self.discover.tracks_state.selected(),
            ListRef::Help => self.help_state.selected(),
        }
    }

    /// The cursor as an index into the underlying list; None when nothing is
    /// visible, so actions no-op under a filter with no matches.
    pub(super) fn real_at(&self, l: ListRef) -> Option<usize> {
        if self.vis_len(l) == 0 {
            return None;
        }
        let cursor = self.cursor_of(l)?;
        let total = self.total_len(l);
        if total == 0 {
            return None;
        }
        Some(match self.filter(l) {
            Some(f) => f.real(cursor, total),
            None => cursor.min(total - 1),
        })
    }

    pub(super) fn with_sel(&mut self, f: impl FnOnce(&mut dyn SelState, usize)) {
        let l = self.active_list();
        let vis = self.vis_len(l);
        match l {
            ListRef::Queue => f(&mut self.queue.state, vis),
            ListRef::BArtists => f(&mut self.browser.artists.state, vis),
            ListRef::BAlbums => f(&mut self.browser.albums.state, vis),
            ListRef::BTracks => f(&mut self.browser.tracks.state, vis),
            ListRef::PlList => f(&mut self.playlists.list.state, vis),
            ListRef::PlItems => f(&mut self.playlists.tracks.state, vis),
            ListRef::Search => f(&mut self.search.state, vis),
            ListRef::SearchDetail => {
                if let Some(d) = self.search.detail.as_mut() {
                    f(&mut d.state, vis);
                }
            }
            ListRef::DStations => f(&mut self.discover.stations_state, vis),
            ListRef::DTracks => f(&mut self.discover.tracks_state, vis),
            ListRef::Help => f(&mut self.help_state, vis),
        }
    }

    pub(super) fn goto_tab(&mut self, t: Tab) {
        if self.tab == Tab::Search && t != Tab::Search {
            self.search.focused = false;
        }
        // Leaving Search drops the way back, except for a visit to Help.
        if (self.tab == Tab::Search && !matches!(t, Tab::Search | Tab::Help))
            || (self.tab == Tab::Help && t != Tab::Search)
        {
            self.search.back.clear();
        }
        if t == Tab::Help && self.tab != Tab::Help {
            self.help_prev = self.tab;
        }
        if self.visual.is_some_and(|v| {
            !matches!(
                (v.list, t),
                (ListRef::Queue, Tab::Queue)
                    | (ListRef::DTracks, Tab::Discover)
                    | (ListRef::BTracks, Tab::Browser)
                    | (ListRef::PlItems, Tab::Playlists)
                    | (ListRef::SearchDetail, Tab::Search)
            )
        }) {
            self.visual = None;
        }
        self.tab = t;
        self.pending_goto = None;
        if t == Tab::Discover {
            self.open_discover();
        }
        if t == Tab::Queue {
            self.follow_queue();
        }
    }

    /// Uses the pane height recorded by the last draw.
    pub(super) fn page(&mut self, dir: i32) {
        let l = self.active_list();
        let visible = self.scroll.visible(scroll_id(l)).max(1);
        let vis = self.vis_len(l);
        self.with_sel(|st, _| {
            if vis == 0 {
                st.select(None);
                return;
            }
            let cur = st.selected().unwrap_or(0);
            let next = scroll::page_jump(cur, visible, vis, dir);
            st.select(Some(next));
        });
    }

    pub(super) fn column_left(&mut self) {
        match self.tab {
            Tab::Browser => {
                self.browser.focus = match self.browser.focus {
                    BrowserCol::Tracks => BrowserCol::Albums,
                    BrowserCol::Albums => BrowserCol::Artists,
                    BrowserCol::Artists => BrowserCol::Artists,
                };
            }
            Tab::Discover => self.discover_column_left(),
            Tab::Playlists if self.playlists.focus == PlaylistsCol::Items => {
                // going back to the playlist list clears its track filter
                self.playlists.tracks.filter.clear();
                self.playlists.focus = PlaylistsCol::List;
            }
            _ => {}
        }
    }

    pub(super) fn column_right(&mut self) {
        match self.tab {
            Tab::Browser => match self.browser.focus {
                BrowserCol::Artists => {
                    // like Enter, but no refetch when the albums shown are this
                    // artist's
                    let Some(real) = self.real_at(ListRef::BArtists) else { return };
                    let Some(a) = self.browser.artists.items.get(real) else { return };
                    if self.browser.albums_for.as_deref() == Some(a.rating_key.as_str()) {
                        self.browser.focus = BrowserCol::Albums;
                    } else {
                        self.activate();
                    }
                }
                BrowserCol::Albums => {
                    if let Some(real) = self.real_at(ListRef::BAlbums) {
                        if let Some(album) = self.browser.albums.items.get(real) {
                            let key = album.rating_key.clone();
                            self.browser.focus = BrowserCol::Tracks;
                            self.load_tracks(key);
                        }
                    }
                }
                _ => {}
            },
            Tab::Discover => self.discover_column_right(),
            Tab::Playlists if !self.playlists.tracks.items.is_empty() => {
                self.playlists.focus = PlaylistsCol::Items;
            }
            _ => {}
        }
    }

    /// Not while filtered: the playing row may be hidden, and filtering is
    /// browsing.
    pub(super) fn follow_queue(&mut self) {
        if !self.auto_follow
            || !self.queue_follow
            || self.tab != Tab::Queue
            || self.queue.filter.is_active()
        {
            return;
        }
        let Some(cur) = self.current else { return };
        if cur >= self.queue.items.len() {
            return;
        }
        self.queue.state.select(Some(cur));
        self.scroll.pin_top(ScrollId::Queue);
    }

    pub(super) fn pause_queue_follow(&mut self) {
        if self.active_list() != ListRef::Queue {
            return;
        }
        // also cancels a pending one-shot pin, or `o` then a move would pin
        // the new cursor (messages are coalesced, so that window is real)
        self.scroll.release_top(ScrollId::Queue);
        if self.queue_follow {
            self.queue_follow = false;
            let key = self
                .keymap
                .first_key(KeyCtx::Queue, &Act::JumpToPlaying)
                .unwrap_or_else(|| ":playing".into());
            self.transient(format!("queue follow: paused ({key} to resume)"));
        }
    }

    /// `g <letter>`: first visible row whose title starts with it.
    pub(super) fn jump_to_letter(&mut self, needle: char) {
        let l = self.active_list();
        if matches!(l, ListRef::Help) {
            return;
        }
        let total = self.total_len(l);
        let visible = self.vis_len(l);
        if total == 0 || visible == 0 {
            return;
        }
        let Some(needle) = needle.to_lowercase().next() else { return };
        let filter = self.filter(l);
        let mut target = None;
        for vi in 0..visible {
            let real = match filter {
                Some(f) => f.real(vi, total),
                None => vi,
            };
            let Some(title) = self.row_title(l, real) else { continue };
            let first = title.trim_start().chars().next().and_then(|c| c.to_lowercase().next());
            if first == Some(needle) {
                target = Some(vi);
                break;
            }
        }
        match target {
            Some(vi) => {
                self.with_sel(|st, _| st.select(Some(vi)));
                self.transient(format!("jump: {}", needle.to_uppercase()));
            }
            None => self.transient(format!("no row starts with {}", needle.to_uppercase())),
        }
    }

    fn row_title(&self, l: ListRef, real: usize) -> Option<&str> {
        match l {
            ListRef::Queue => self.queue.items.get(real).map(|t| t.title.as_str()),
            ListRef::BArtists => self.browser.artists.items.get(real).map(|a| a.title.as_str()),
            ListRef::BAlbums => self.browser.albums.items.get(real).map(|a| a.title.as_str()),
            ListRef::BTracks => self.browser.tracks.items.get(real).map(|t| t.title.as_str()),
            ListRef::PlList => self.playlists.list.items.get(real).map(|p| p.title.as_str()),
            ListRef::PlItems => self.playlists.tracks.items.get(real).map(|t| t.title.as_str()),
            ListRef::Search => self.search.hits.get(real).map(|h| h.title.as_str()),
            ListRef::SearchDetail => self
                .search
                .detail
                .as_ref()
                .and_then(|d| d.tracks.get(real))
                .map(|t| t.title.as_str()),
            ListRef::DStations => self.discover.stations.get(real).map(|s| s.title.as_str()),
            ListRef::DTracks => self.discover.tracks.get(real).map(|t| t.title.as_str()),
            ListRef::Help => None,
        }
    }

    /// `o`. In the Queue this also resumes auto-follow; other lists center as
    /// configured.
    pub(super) fn jump_to_playing(&mut self) {
        let Some(cur) = self.current else {
            self.transient("nothing playing".into());
            return;
        };
        let Some(key) = self.queue.items.get(cur).map(|t| t.rating_key.clone()) else { return };
        let center = self.center_cursor;
        let in_queue = self.active_list() == ListRef::Queue;
        if in_queue {
            self.queue_follow = self.auto_follow;
        }
        match self.active_list() {
            ListRef::Queue => {
                let Some(p) = self.queue.filter.cursor_of(cur) else {
                    self.transient("playing track is filtered out".into());
                    return;
                };
                self.queue.state.select(Some(p));
                // with auto-follow off `o` still jumps, once
                if self.auto_follow {
                    self.scroll.pin_top(ScrollId::Queue);
                    self.transient("queue follow: on".into());
                } else {
                    self.scroll.pin_top_once(ScrollId::Queue);
                    self.transient("queue follow: off".into());
                }
            }
            ListRef::BTracks => {
                let pos = self.browser.tracks.items.iter().position(|t| t.rating_key == key);
                if let Some(p) = pos.and_then(|r| self.browser.tracks.filter.cursor_of(r)) {
                    self.browser.tracks.state.select(Some(p));
                    if center {
                        self.scroll.center_once = Some(ScrollId::BrowserTracks);
                    }
                }
            }
            ListRef::PlItems => {
                let pos = self.playlists.tracks.items.iter().position(|t| t.rating_key == key);
                if let Some(p) = pos.and_then(|r| self.playlists.tracks.filter.cursor_of(r)) {
                    self.playlists.tracks.state.select(Some(p));
                    if center {
                        self.scroll.center_once = Some(ScrollId::PlaylistItems);
                    }
                }
            }
            _ => {}
        }
    }

    /// Esc / Backspace.
    pub(super) fn back(&mut self) {
        if self.clear_adventure() {
            return;
        }
        match self.tab {
            Tab::Browser => match self.browser.focus {
                BrowserCol::Tracks => {
                    self.browser.tracks.filter.clear();
                    self.browser.focus = BrowserCol::Albums;
                }
                BrowserCol::Albums => {
                    self.browser.albums.filter.clear();
                    self.browser.focus = BrowserCol::Artists;
                }
                BrowserCol::Artists => {
                    self.browser.artists.filter.clear();
                    self.goto_tab(Tab::Queue);
                }
            },
            Tab::Search => self.search_back(),
            Tab::Help => {
                let t = self.help_prev;
                self.goto_tab(t);
            }
            _ => {}
        }
    }

    /// `(input, scope, shown, total)` while typing.
    pub fn filter_typing(&self) -> Option<(String, &'static str, usize, usize)> {
        let l = self.active_list();
        let f = self.filter(l)?;
        if !f.typing {
            return None;
        }
        let total = self.total_len(l);
        Some((f.text.clone(), scope_label(l), f.len(total), total))
    }

    /// Re-applies the active filter and puts the cursor on the first match.
    pub(super) fn refilter_active_and_reset(&mut self) {
        let l = self.active_list();
        self.refilter(l);
        self.with_sel(|st, vis| {
            st.select(if vis == 0 { None } else { Some(0) });
        });
    }

    /// Also called whenever the underlying list changes, so a refresh keeps
    /// the filter applied.
    pub(super) fn refilter(&mut self, l: ListRef) {
        let Some(text) = self.filter(l).map(|f| f.text.clone()) else {
            return;
        };
        if text.is_empty() {
            if let Some(f) = self.filter_mut(l) {
                f.indices.clear();
            }
            return;
        }
        let needle = filter::Needle::new(&text);
        match l {
            ListRef::Queue => {
                let items = &self.queue.items;
                self.queue.filter.apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::BArtists => {
                let items = &self.browser.artists.items;
                self.browser
                    .artists
                    .filter
                    .apply(items.len(), |i| needle.found_in(&items[i].title));
            }
            ListRef::BAlbums => {
                let items = &self.browser.albums.items;
                self.browser.albums.filter.apply(items.len(), |i| {
                    let a = &items[i];
                    needle.found_in(&a.title) || needle.found_in(&a.artist)
                });
            }
            ListRef::BTracks => {
                let items = &self.browser.tracks.items;
                self.browser.tracks.filter.apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::PlList => {
                let items = &self.playlists.list.items;
                self.playlists.list.filter.apply(items.len(), |i| needle.found_in(&items[i].title));
            }
            ListRef::PlItems => {
                let items = &self.playlists.tracks.items;
                self.playlists.tracks.filter.apply(items.len(), |i| {
                    let t = &items[i];
                    needle.found_in(&t.title)
                        || needle.found_in(&t.artist)
                        || needle.found_in(&t.album)
                });
            }
            ListRef::Search
            | ListRef::SearchDetail
            | ListRef::DStations
            | ListRef::DTracks
            | ListRef::Help => {}
        }
    }
}
