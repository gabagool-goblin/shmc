use super::{
    App, BrowserCol, Confirm, ListRef, Msg, PendingPlaylist, PlaylistsCol, PlexCmd, QueueAction,
    Tab,
};
use crate::ui::keymap::{Act, KeyCtx};
use crate::ui::playlist_load::PlaylistLoad;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

impl App {
    /// Keeps the items pane on the highlighted playlist. Previews are bounded
    /// and debounced so a huge playlist is not fetched whole.
    pub(super) fn preview_playlist(&mut self) {
        if self.tab != Tab::Playlists
            || self.playlists.focus != PlaylistsCol::List
            || self.playlists.list.filter.typing
        {
            self.preview_blocked = None;
            return;
        }
        let Some(real) = self.real_at(ListRef::PlList) else { return };
        let Some(pl) = self.playlists.list.items.get(real) else { return };
        let key = pl.rating_key.clone();
        if self.preview_blocked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview_blocked = None;
        let total = pl.item_count.map(|n| n as usize);
        if self.playlists.tracks_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        self.playlists.tracks_for = Some(key.clone());
        self.playlists.tracks_total = total;
        self.playlists.tracks.items.clear();
        self.drop_visual(ListRef::PlItems);
        self.playlists.tracks.state.select(Some(0));
        self.playlists.tracks.filter.clear();

        if self.preview_tracks == 0 {
            self.playlists.tracks_for = None;
            return;
        }
        if let Some(cached) = self.playlists.preview_cache.get(&key).cloned() {
            self.playlists.tracks.items = cached;
            self.drop_visual(ListRef::PlItems);
            self.announce_preview_cap();
            return;
        }
        if self.preview_debounce_ms == 0 {
            self.load_playlist_preview(key);
        } else {
            let tx = self.msg_tx.clone();
            let epoch = self.preview_epoch;
            let delay = self.preview_debounce_ms;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                let _ = tx.send(Msg::PreviewPlaylist { key, epoch });
            });
        }
    }

    pub(super) fn selected_playlist_key(&self) -> Option<String> {
        let real = self.real_at(ListRef::PlList)?;
        self.playlists.list.items.get(real).map(|p| p.rating_key.clone())
    }

    pub(super) fn load_playlist_preview(&mut self, key: String) {
        let size = self.preview_tracks.max(1);
        let _ = self.plex.tx.send(PlexCmd::LoadPlaylistPreview { key, size });
    }

    /// The status line is the only place the preview cap shows.
    pub(super) fn announce_preview_cap(&mut self) {
        if self.tab != Tab::Playlists {
            return;
        }
        let Some(total) = self.playlists.tracks_total else { return };
        if self.preview_tracks == 0 || total <= self.preview_tracks {
            return;
        }
        let shown = self.playlists.tracks.items.len();
        let mut msg = format!("preview: {shown} of {total} tracks");
        if let Some(key) = self.keymap.first_key(KeyCtx::Playlists, &Act::Activate) {
            msg.push_str(&format!("; {key} adds all"));
        }
        self.transient(msg);
    }

    /// Asks first when it is very large. `count` is what would actually be
    /// queued (the remainder when starting mid-list).
    fn queue_playlist(
        &mut self,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        count: Option<u32>,
        start: Option<String>,
    ) {
        let n = count.unwrap_or(0) as usize;
        if self.confirm_playlist_over > 0 && n > self.confirm_playlist_over {
            self.pending_clear = None;
            self.pending_playlist = Some(PendingPlaylist {
                key,
                confirm,
                action,
                start,
                count,
                armed_at: Instant::now(),
            });
            let verb = match action {
                QueueAction::Append => "Add",
                QueueAction::Replace => "Replace queue with",
                QueueAction::PlayNext => "Insert",
            };
            self.transient_secs(
                format!("{verb} {n} tracks? (y/n)"),
                self.confirm_timeout_secs,
                false,
            );
            return;
        }
        self.send_resolve_playlist(key, confirm, action, start, count);
    }

    /// A replace streams unless shuffle is on, which needs the whole list.
    pub(super) fn send_resolve_playlist(
        &mut self,
        key: String,
        confirm: Confirm,
        action: QueueAction,
        start: Option<String>,
        count: Option<u32>,
    ) {
        if action != QueueAction::Replace && self.refuse_while_loading() {
            return;
        }
        let seq = self.issue_seq(action == QueueAction::Replace);
        let stream = action == QueueAction::Replace && !self.random;
        let mut cancel = Arc::new(AtomicBool::new(false));
        if stream {
            let load = PlaylistLoad::new(seq);
            cancel = load.cancel.clone();
            self.playlist_load = Some(load);
        }
        let _ = self.plex.tx.send(PlexCmd::ResolvePlaylist {
            seq,
            key,
            confirm,
            action,
            start,
            stream,
            expected: count.unwrap_or(0) as usize,
            cancel,
        });
        self.transient(
            match action {
                QueueAction::Append => "adding playlist…",
                QueueAction::Replace => "loading playlist…",
                QueueAction::PlayNext => "inserting playlist…",
            }
            .into(),
        );
    }

    /// Enter. Albums drill with Right/l so Enter can play at once.
    pub(super) fn activate(&mut self) {
        match self.active_list() {
            ListRef::Queue => {
                if let Some(real) = self.real_at(ListRef::Queue) {
                    self.play_queue_index(real);
                }
            }
            ListRef::BArtists => {
                let Some(real) = self.real_at(ListRef::BArtists) else { return };
                let Some(a) = self.browser.artists.items.get(real) else { return };
                let key = a.rating_key.clone();
                self.browser.artists.filter.clear();
                self.browser.focus = BrowserCol::Albums;
                self.load_albums(key);
            }
            ListRef::BAlbums => {
                let Some(real) = self.real_at(ListRef::BAlbums) else { return };
                let Some(al) = self.browser.albums.items.get(real) else { return };
                let confirm = Confirm::Album { name: al.title.clone() };
                let key = al.rating_key.clone();
                let seq = self.issue_seq(true);
                let _ = self.plex.tx.send(PlexCmd::ResolveAlbum {
                    seq,
                    key,
                    confirm,
                    action: QueueAction::Replace,
                });
                self.transient("loading album…".into());
            }
            ListRef::BTracks => {
                let Some(real) = self.real_at(ListRef::BTracks) else { return };
                let Some(t) = self.browser.tracks.items.get(real).cloned() else { return };
                let tracks = self.browser.tracks.items[real..].to_vec();
                let confirm = Confirm::FromList { name: t.album.clone() };
                self.queue_local(tracks, confirm, QueueAction::Replace);
            }
            ListRef::PlList => {
                let Some(real) = self.real_at(ListRef::PlList) else { return };
                let Some(pl) = self.playlists.list.items.get(real) else { return };
                let confirm = Confirm::Playlist { name: pl.title.clone() };
                let key = pl.rating_key.clone();
                let count = pl.item_count;
                self.queue_playlist(key, confirm, QueueAction::Replace, count, None);
            }
            ListRef::PlItems => {
                let Some(real) = self.real_at(ListRef::PlItems) else { return };
                let Some(t) = self.playlists.tracks.items.get(real).cloned() else { return };
                let Some(key) = self.playlists.tracks_for.clone() else { return };
                let row = self.playlists.list.items.iter().find(|p| p.rating_key == key);
                let name = row.map(|p| p.title.clone()).unwrap_or_else(|| "playlist".into());
                // the preview is the first page, so its index is the playlist's
                // index, and the remainder is what the question must name
                let remainder =
                    row.and_then(|p| p.item_count).map(|total| total.saturating_sub(real as u32));
                let confirm = Confirm::FromList { name };
                self.queue_playlist(
                    key,
                    confirm,
                    QueueAction::Replace,
                    remainder,
                    Some(crate::plex::client::entry_key(&t)),
                );
            }
            ListRef::Search => {
                let Some(i) = self.search.state.selected() else { return };
                let Some(hit) = self.search.hits.get(i).cloned() else { return };
                use crate::plex::HitKind;
                match hit.kind {
                    HitKind::Track => {
                        let seq = self.issue_seq(true);
                        let _ = self.plex.tx.send(PlexCmd::ResolveTrack {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Track { title: hit.title },
                            action: QueueAction::Replace,
                            rest_of_album: true,
                        });
                        self.transient("loading…".into());
                    }
                    HitKind::Album => {
                        self.clear_list_requests();
                        self.search.detail_request = Some(hit.rating_key.clone());
                        let _ = self.plex.tx.send(PlexCmd::LoadAlbumDetail {
                            key: hit.rating_key,
                            label: hit.title,
                        });
                        self.transient("loading album…".into());
                    }
                    HitKind::Artist => self.reveal(Some(hit.rating_key), None, None),
                }
            }
            ListRef::SearchDetail => {
                let Some(d) = self.search.detail.as_ref() else { return };
                let Some(i) = d.state.selected() else { return };
                let tracks = d.tracks[i..].to_vec();
                if let Some(queue) = d.radio.clone() {
                    let name = d.label.clone();
                    self.play_radio(tracks, name, queue);
                    return;
                }
                let confirm = Confirm::FromList { name: d.label.clone() };
                self.queue_local(tracks, confirm, QueueAction::Replace);
            }
            ListRef::DStations => self.start_station(),
            ListRef::DTracks => self.play_discover_from(),
            ListRef::Help => {}
        }
    }

    pub(super) fn add_selected(&mut self, action: QueueAction) {
        match self.active_list() {
            ListRef::BArtists => {
                let Some(real) = self.real_at(ListRef::BArtists) else { return };
                let Some(a) = self.browser.artists.items.get(real) else { return };
                let confirm = Confirm::Artist { name: a.title.clone() };
                let key = a.rating_key.clone();
                let seq = self.issue_seq(action == QueueAction::Replace);
                let _ =
                    self.plex.tx.send(PlexCmd::ResolveArtistTracks { seq, key, confirm, action });
                self.transient(
                    match action {
                        QueueAction::Append => "loading artist…",
                        QueueAction::Replace => "loading artist…",
                        QueueAction::PlayNext => "inserting artist…",
                    }
                    .into(),
                );
            }
            ListRef::BAlbums => {
                let Some(real) = self.real_at(ListRef::BAlbums) else { return };
                let Some(al) = self.browser.albums.items.get(real) else { return };
                let confirm = Confirm::Album { name: al.title.clone() };
                let key = al.rating_key.clone();
                let seq = self.issue_seq(action == QueueAction::Replace);
                let _ = self.plex.tx.send(PlexCmd::ResolveAlbum { seq, key, confirm, action });
                self.transient(
                    match action {
                        QueueAction::Append => "loading album…",
                        QueueAction::Replace => "loading album…",
                        QueueAction::PlayNext => "inserting album…",
                    }
                    .into(),
                );
            }
            ListRef::BTracks => {
                let Some(real) = self.real_at(ListRef::BTracks) else { return };
                let Some(t) = self.browser.tracks.items.get(real).cloned() else { return };
                let confirm = Confirm::Track { title: t.title.clone() };
                self.queue_local(vec![t], confirm, action);
            }
            ListRef::PlList => {
                let Some(real) = self.real_at(ListRef::PlList) else { return };
                let Some(pl) = self.playlists.list.items.get(real) else { return };
                let confirm = Confirm::Playlist { name: pl.title.clone() };
                let key = pl.rating_key.clone();
                let count = pl.item_count;
                self.queue_playlist(key, confirm, action, count, None);
            }
            ListRef::PlItems => {
                let Some(real) = self.real_at(ListRef::PlItems) else { return };
                let Some(t) = self.playlists.tracks.items.get(real).cloned() else { return };
                if action == QueueAction::PlayNext {
                    let confirm = Confirm::Track { title: t.title.clone() };
                    self.queue_local(vec![t], confirm, QueueAction::PlayNext);
                } else {
                    let Some(key) = self.playlists.tracks_for.clone() else { return };
                    let row = self.playlists.list.items.iter().find(|p| p.rating_key == key);
                    let name = row.map(|p| p.title.clone()).unwrap_or_else(|| "playlist".into());
                    // `real` is the position in the playlist (first-page preview)
                    let remainder = row
                        .and_then(|p| p.item_count)
                        .map(|total| total.saturating_sub(real as u32));
                    self.queue_playlist(
                        key,
                        Confirm::FromList { name },
                        action,
                        remainder,
                        Some(crate::plex::client::entry_key(&t)),
                    );
                }
            }
            ListRef::Search => {
                let Some(i) = self.search.state.selected() else { return };
                let Some(hit) = self.search.hits.get(i).cloned() else { return };
                use crate::plex::HitKind;
                match hit.kind {
                    HitKind::Track => {
                        let seq = self.issue_seq(action == QueueAction::Replace);
                        let _ = self.plex.tx.send(PlexCmd::ResolveTrack {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Track { title: hit.title },
                            action,
                            rest_of_album: false,
                        });
                        self.transient(
                            if action == QueueAction::PlayNext {
                                "inserting…"
                            } else {
                                "adding…"
                            }
                            .into(),
                        );
                    }
                    HitKind::Album => {
                        let seq = self.issue_seq(action == QueueAction::Replace);
                        let _ = self.plex.tx.send(PlexCmd::ResolveAlbum {
                            seq,
                            key: hit.rating_key,
                            confirm: Confirm::Album { name: hit.title },
                            action,
                        });
                        self.transient(
                            if action == QueueAction::PlayNext {
                                "inserting album…"
                            } else {
                                "adding album…"
                            }
                            .into(),
                        );
                    }
                    HitKind::Artist => {}
                }
            }
            ListRef::SearchDetail => {
                let Some(d) = self.search.detail.as_ref() else { return };
                let Some(i) = d.state.selected() else { return };
                let Some(t) = d.tracks.get(i).cloned() else { return };
                let confirm = Confirm::Track { title: t.title.clone() };
                self.queue_local(vec![t], confirm, action);
            }
            ListRef::DTracks => self.add_discover_track(action),
            ListRef::Queue | ListRef::DStations | ListRef::Help => {}
        }
    }
}
