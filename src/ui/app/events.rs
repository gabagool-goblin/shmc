use super::{
    sort_albums, App, BrowserCol, ListKind, ListRef, Msg, PlaylistsCol, PlexCmd, SearchDetail, Tab,
    FAILED_STREAK_LIMIT,
};
use crate::config::RatingStyle;
use crate::playback::{MpvCmd, MpvEvent};
use crate::plex::AudioInfo;
use crate::ui::keymap::Act;
use ratatui::widgets::TableState;
use std::time::Instant;

/// mpv booleans come as bool, "no"/"inf"/"force" or numbers.
fn prop_on(value: &Option<serde_json::Value>) -> bool {
    use serde_json::Value;
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.as_str(), "no" | "false" | "0" | "off" | "none" | ""),
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        _ => false,
    }
}

impl App {
    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Key(ev) => self.handle_key(ev),
            Msg::Paste(text) => self.handle_paste(text),
            Msg::Quit => self.should_quit = true,
            Msg::Resize => {
                // ratatui clears the screen on a resize; the cover's cells would stay skipped
                self.art.shown_rect = None;
                // a drag-resize sends the image once, after it settles
                self.hold_art();
                // a font zoom or another display changes the cell size
                let new = self.picker.as_ref().and_then(|old| {
                    crate::ui::picker_for_cell(old, crate::ui::physical_cell_size())
                });
                if let Some(picker) = new {
                    self.picker = Some(picker);
                    self.art.protocol = None;
                    self.art.prepared_rect = None;
                }
            }
            Msg::ArtistsLoaded(artists) => {
                self.browser.artists.items = artists;
                self.refilter(ListRef::BArtists);
                self.browser.artists.state.select(if self.browser.artists.items.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            Msg::AlbumsLoaded { artist_key, albums } => {
                if self.browser.albums_for.as_deref() == Some(artist_key.as_str()) {
                    self.browser.albums.items = albums;
                    sort_albums(&mut self.browser.albums.items, &self.album_sort);
                    self.browser.albums.state.select(if self.browser.albums.items.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.browser.tracks.items.clear();
                    self.drop_visual(ListRef::BTracks);
                    self.browser.tracks.state.select(None);
                    self.browser.tracks_for = None;
                    self.refilter(ListRef::BAlbums);
                    self.continue_goto_artist(&artist_key);
                }
            }
            Msg::TracksLoaded { album_key, tracks } => {
                if self.browser.tracks_for.as_deref() == Some(album_key.as_str()) {
                    self.browser.tracks.items = tracks;
                    self.drop_visual(ListRef::BTracks);
                    self.browser.tracks.state.select(if self.browser.tracks.items.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.refilter(ListRef::BTracks);
                    self.continue_goto_album(&album_key);
                }
            }
            Msg::SearchResults { query, hits } => {
                if query == self.search.last_query {
                    self.search.hits = hits;
                    self.search.similar = None;
                    self.search.back.clear();
                    self.search.state.select(if self.search.hits.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.set_search_detail(None);
                    self.search.clear_list_request();
                    self.transient(format!("{} results for {:?}", self.search.hits.len(), query));
                }
            }
            Msg::SearchAlbumLoaded { key, label, tracks } => {
                if self.search.take_list_request(&key, |k| k == ListKind::Album).is_some() {
                    self.push_back_for_detail(Tab::Search);
                    let mut state = TableState::default();
                    if !tracks.is_empty() {
                        state.select(Some(0));
                    }
                    self.set_search_detail(Some(SearchDetail {
                        key,
                        label,
                        tracks,
                        state,
                        matches: None,
                        radio: None,
                    }));
                    self.search.focused = false;
                }
            }
            Msg::SimilarLoaded { key, label, outcome } => self.similar_loaded(key, label, outcome),
            Msg::AdventureProgress { key, leg, legs } => self.adventure_progress(&key, leg, legs),
            Msg::AdventureBuilt { key, outcome } => self.adventure_built(key, outcome),
            Msg::RadioLoaded { key, label, outcome } => self.radio_loaded(key, label, outcome),
            Msg::Discover(msg) => self.discover_msg(msg),
            Msg::PlaylistsLoaded(list) => {
                self.playlists.list.items = list;
                self.playlists.preview_cache.clear();
                self.playlists.tracks_for = None;
                self.playlists.tracks_total = None;
                self.playlists.tracks.items.clear();
                self.drop_visual(ListRef::PlItems);
                self.refilter(ListRef::PlList);
                self.playlists.list.state.select(if self.playlists.list.items.is_empty() {
                    None
                } else {
                    Some(0)
                });
            }
            Msg::PlaylistSaved(outcome) => {
                use crate::plex::SaveOutcome;
                self.saving = false;
                let reload = match outcome {
                    SaveOutcome::Saved { name, count, repeats_skipped } => {
                        tracing::info!(count, repeats_skipped, "saved the queue as a playlist");
                        let mut msg = format!(
                            "saved {count} track{} to \"{name}\"",
                            if count == 1 { "" } else { "s" }
                        );
                        if repeats_skipped > 0 {
                            msg.push_str(&format!(
                                " ({repeats_skipped} repeats skipped: a Plex playlist holds each track once)"
                            ));
                        }
                        self.transient(msg);
                        true
                    }
                    SaveOutcome::Partial { name, saved, total, reason } => {
                        tracing::warn!(saved, total, "saved the queue only in part");
                        self.transient_err(format!(
                            "saved {saved} of {total} tracks to \"{name}\"; the rest were not added: {reason}"
                        ));
                        true
                    }
                    SaveOutcome::NameTaken { name } => {
                        self.transient_err(format!(
                            "a playlist named \"{name}\" already exists; choose another name"
                        ));
                        false
                    }
                    SaveOutcome::Failed { reason } => {
                        tracing::warn!("playlist save failed");
                        self.transient_err(format!("could not save the playlist: {reason}"));
                        false
                    }
                };
                if reload {
                    let _ = self.plex.tx.send(PlexCmd::LoadPlaylists);
                }
            }
            Msg::PlaylistItemsLoaded { rating_key, tracks } => {
                if self.playlists.tracks_for.as_deref() == Some(rating_key.as_str()) {
                    self.playlists.preview_cache.insert(rating_key, tracks.clone());
                    self.playlists.tracks.items = tracks;
                    self.drop_visual(ListRef::PlItems);
                    self.refilter(ListRef::PlItems);
                    self.playlists.tracks.state.select(if self.playlists.tracks.items.is_empty() {
                        None
                    } else {
                        Some(0)
                    });
                    self.announce_preview_cap();
                }
            }
            Msg::PreviewPlaylist { key, epoch } => {
                let still_selected = self.selected_playlist_key().as_deref() == Some(key.as_str());
                if epoch == self.preview_epoch
                    && still_selected
                    && self.tab == Tab::Playlists
                    && self.playlists.focus == PlaylistsCol::List
                    && self.playlists.tracks_for.as_deref() == Some(key.as_str())
                {
                    self.load_playlist_preview(key);
                }
            }
            Msg::QueueReply { seq, outcome } => {
                for due in self.queue_seq.deliver(seq, outcome) {
                    self.apply_queue_outcome(due);
                }
            }
            Msg::PlaylistPage { load_id, tracks, last } => {
                self.apply_playlist_page(load_id, tracks, last);
            }
            Msg::PlaylistLoadStopped { load_id, error } => {
                let Some(l) = self.playlist_load.take_if(|l| l.id == load_id) else { return };
                if l.landed {
                    self.transient_err(format!(
                        "playlist load stopped at {}: {error}",
                        l.progress()
                    ));
                }
            }
            Msg::GotoArtist(artist) => {
                // a late reply must not drag the user back after they left
                if !matches!(&self.pending_goto, Some(p) if p.artist_key == artist.rating_key) {
                    return;
                }
                self.browser.focus = BrowserCol::Artists;
                // the jump selects a real index, which a filter would misplace
                self.browser.artists.filter.clear();
                let key = artist.rating_key.clone();
                if !self.browser.artists.items.iter().any(|a| a.rating_key == key) {
                    self.browser.artists.items.push(artist);
                    self.browser.artists.items.sort_by_key(|a| a.title.to_lowercase());
                }
                let pos = self.browser.artists.items.iter().position(|a| a.rating_key == key);
                self.browser.artists.state.select(pos);
                self.load_albums(key);
            }
            Msg::ArtReady { thumb_path, img, file } => {
                let wanted = self.current_thumb_path();
                if wanted.as_deref() != Some(thumb_path.as_str()) {
                    // Playback moved on; a path left in flight is never asked for again.
                    if self.art.requested.as_deref() == Some(thumb_path.as_str()) {
                        self.art.requested = None;
                    }
                } else {
                    if self.art.thumb.as_deref() == Some(thumb_path.as_str())
                        && self.art.img.is_some()
                    {
                        self.art.requested = None;
                        return;
                    }
                    self.art.thumb = Some(thumb_path);
                    self.art.requested = None;
                    self.art.img = Some(img);
                    self.art.protocol = None; // force re-encode for current size
                    self.art.shown_rect = None;
                    self.art.prepared_rect = None;
                    // A new cover is drawn at once, not after a dwell.
                    self.art.hold_until = None;
                    self.art.file = file;
                    self.attach_cover();
                }
            }
            Msg::ArtDue => {}
            Msg::ConnectionChecked { epoch, ok } => self.connection_checked(epoch, ok),
            Msg::ArtFailed { thumb_path } => {
                if self.art.requested.as_deref() == Some(thumb_path.as_str()) {
                    self.art.requested = None;
                }
            }
            Msg::Rated { rating_key, rating, title } => {
                self.rating_saved(rating_key, rating, title);
            }
            Msg::TrackInfo { rating_key, audio, user_rating } => {
                self.ratings.insert(rating_key.clone(), user_rating);
                self.level_track_info(&rating_key, &audio);
                self.audio_cache.insert(rating_key.clone(), audio.clone());
                let wanted = self.current_track().map(|t| t.rating_key.as_str())
                    == Some(rating_key.as_str());
                if wanted {
                    if let Some(t) = self.current.and_then(|i| self.queue.items.get_mut(i)) {
                        t.audio = audio.clone();
                    }
                    self.audio = audio;
                }
            }
            Msg::Mpv(ev) => self.handle_mpv(ev),
            Msg::Cava(frame) => {
                // cava emits frames before playback starts, so not gated on `stopped`
                self.cava_frame = frame;
                self.cava_alive = true;
                self.cava_note.clear();
            }
            Msg::CavaReady => {
                self.cava_alive = true;
                self.cava_note.clear();
            }
            Msg::CavaDead(hint) => {
                self.cava_alive = false;
                self.cava_note = hint.clone();
                self.transient_err(hint);
            }
            Msg::ClearStatus(epoch) => {
                if epoch == self.status_epoch {
                    self.status.clear();
                    self.status_error = false;
                }
            }
            Msg::SleepTimer { epoch } => {
                if epoch == self.sleep_epoch && self.sleep_until.is_some() {
                    self.sleep_until = None;
                    self.sleep_mins = None;
                    self.sleep_cancel = None;
                    if self.stopped {
                        self.transient("sleep timer: off".into());
                    } else {
                        // Set rather than toggle, which would resume a paused track.
                        self.paused = true;
                        let _ = self.mpv.tx.send(MpvCmd::SetPause { paused: true });
                        self.transient("sleep timer: playback paused".into());
                    }
                }
            }
            Msg::Error(e) => {
                tracing::error!(e);
                self.transient_err(e);
            }
            Msg::LoadFailed { key, error } => {
                tracing::error!(error);
                let serves_jump = matches!(
                    &self.pending_goto,
                    Some(p) if p.artist_key == key || p.album_key.as_deref() == Some(key.as_str())
                );
                if serves_jump {
                    let pg = self.pending_goto.take().expect("serves_jump checked above");
                    self.goto_tab(pg.from);
                    self.transient_err("Album not found in library".into());
                } else {
                    self.transient_err(error);
                }
            }
            Msg::PreviewFailed { key } => {
                // Retry on the next highlight, or a dead server is asked on every message
                if self.playlists.tracks_for.as_deref() == Some(key.as_str()) {
                    self.playlists.tracks_for = None;
                    self.preview_blocked = Some(key);
                }
            }
        }
        self.preview_playlist();
        self.follow_discover();
        self.follow_radio();
    }

    /// The guard ends on its expected value, count or deadline.
    fn stale_position(&mut self, pos: usize) -> bool {
        let Some(guard) = self.expect_pos.as_mut() else {
            return false;
        };
        if pos == guard.expected {
            self.expect_pos = None;
            return false;
        }
        guard.ignores = guard.ignores.saturating_add(1);
        let spent = guard.max_ignores.is_some_and(|max| guard.ignores > max)
            || Instant::now() >= guard.until;
        if spent {
            self.expect_pos = None;
        }
        true
    }

    fn handle_mpv(&mut self, ev: MpvEvent) {
        match ev {
            MpvEvent::Property { name, value } => match name.as_str() {
                "time-pos" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_f64()) {
                        self.position = v;
                        self.report_position(v);
                        // a failing file never gets a playback time: the sign a track really plays
                        if v > 0.0 {
                            self.err_streak = 0;
                            self.failed_at = None;
                        }
                    }
                }

                "pause" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_bool()) {
                        self.paused = v;
                        self.report_pause(v);
                        if v {
                            self.cava_frame = vec![0; crate::playback::cava::BARS];
                        }
                    }
                }
                "volume" => {
                    if let Some(v) = value.as_ref().and_then(|v| v.as_f64()) {
                        self.volume = v;
                    }
                }
                "loop-playlist" => self.loop_playlist = prop_on(&value),
                "loop-file" => self.loop_file = prop_on(&value),
                // shmc owns the shuffled order
                "shuffle" => {}
                crate::playback::mpv::LEVELING_READY => self.leveling.ready = prop_on(&value),
                "audio-bitrate" => {
                    self.bitrate_bps = value.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0);
                }
                "playlist-pos" => {
                    let pos = value.as_ref().and_then(|v| v.as_i64());
                    match pos {
                        Some(p) if p >= 0 => {
                            let p = p as usize;
                            // stale after a consume; `current` must not point past the end
                            if p >= self.queue.items.len() {
                                return;
                            }
                            if self.stale_position(p) {
                                return;
                            }
                            let changed = self.current != Some(p);
                            self.current = Some(p);
                            self.stopped = false;
                            // mpv may be failing through entries: fetch once one loads
                            if self.err_streak == 0 {
                                self.fetch_art_for_current();
                                self.fetch_audio_for_current();
                            }
                            if changed {
                                self.follow_queue();
                            }
                        }
                        _ => {
                            self.expect_pos = None;
                            self.current = None;
                            self.stopped = true;
                            self.position = 0.0;
                            self.audio = AudioInfo::default();
                            self.cava_frame = vec![0; crate::playback::cava::BARS];
                        }
                    }
                }
                _ => {}
            },
            MpvEvent::EndFile { reason } => {
                self.report_end(&reason);
                if reason == "error" {
                    self.err_streak += 1;
                    // mpv may fail a few entries before the Stop lands: keep the first
                    let marker = self.failed_at.or(self.current);
                    self.failed_at = marker;
                    let title = marker
                        .and_then(|i| self.queue.items.get(i))
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| "track".into());
                    // with repeat mpv wraps; the streak limit ends an all-failing queue
                    let more = (self.loop_playlist && self.queue.items.len() > 1)
                        || marker.map_or(!self.queue.items.is_empty(), |f| {
                            f + 1 < self.queue.items.len()
                        });
                    tracing::info!(
                        track = %crate::redact::escape_field(&title),
                        streak = self.err_streak,
                        "mpv failed to play a track"
                    );
                    if !more || self.err_streak >= FAILED_STREAK_LIMIT {
                        if let Some(at) = marker {
                            self.check_connection(at);
                        }
                        let _ = self.mpv.tx.send(MpvCmd::Stop);
                        self.stopped = true;
                        self.position = 0.0;
                        self.duration = 0.0;
                        let hint = if more {
                            let key = self
                                .keymap
                                .first_key(self.key_ctx(), &Act::NextTrack)
                                .unwrap_or_else(|| ":n".into());
                            format!("press {key} for the next track")
                        } else {
                            "pick another song".into()
                        };
                        self.transient_err(format!("{title} failed to play; {hint}"));
                    } else {
                        self.transient_err(format!("{title} failed to play; skipping"));
                    }
                }
                if reason == "eof" && self.consume {
                    if let Some(c) = self.current {
                        self.consume_finished(c);
                    }
                }
            }
            MpvEvent::Shutdown => {
                self.report_stop();
                self.transient_err("mpv exited".into());
                self.stopped = true;
                self.mpv_exited = true;
            }
            MpvEvent::Ready => {}
            MpvEvent::MediaKey(key) => self.media_key(key),
            MpvEvent::FileLoaded => {
                self.cancel_resume();
                self.check_leveling_loaded();
                self.fetch_art_for_current();
                self.fetch_audio_for_current();
                self.seek_restored();
                self.report_open();
                self.cover_due = true;
                self.attach_cover();
            }
        }
    }

    fn current_thumb_path(&self) -> Option<String> {
        self.current_track().and_then(|t| t.thumb.clone()).or_else(|| {
            self.current_track()
                .and_then(|t| t.parent_rating_key.clone())
                .map(|k| format!("/library/metadata/{k}/thumb"))
        })
    }

    /// The cover goes to mpv once per file, when both the file and its art are ready.
    fn attach_cover(&mut self) {
        if !(self.now_playing_cover && self.cover_due && !self.stopped) {
            return;
        }
        let thumb = self.current_thumb_path();
        if thumb.is_none() || thumb != self.art.thumb || self.art.img.is_none() {
            return;
        }
        let Some(file) = self.art.file.clone() else { return };
        let _ = self.mpv.tx.send(MpvCmd::AddCover(file));
        self.cover_due = false;
    }

    pub(super) fn fetch_art_for_current(&mut self) {
        if !self.art_enabled {
            return;
        }
        let Some(path) = self.current_thumb_path() else {
            self.art.clear();
            return;
        };
        if self.art.img.is_some() && self.art.thumb.as_deref() == Some(path.as_str()) {
            return;
        }
        if self.art.requested.as_deref() == Some(path.as_str()) {
            return;
        }
        self.art.requested = Some(path.clone());
        let _ = self.plex.tx.send(PlexCmd::LoadThumb { path });
    }

    /// Shows the bulk audio info at once, then asks for the full metadata.
    pub(super) fn fetch_audio_for_current(&mut self) {
        let Some(t) = self.current_track() else {
            self.audio = AudioInfo::default();
            return;
        };
        let key = t.rating_key.clone();
        let bulk = t.audio.clone();
        let full = t.audio.sample_rate_hz.is_some() || t.audio.bit_depth.is_some();
        let duration = t.duration_ms.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0);
        self.duration = duration;
        let cached = self.audio_cache.get(&key).cloned();
        let need_audio = !full && cached.is_none();
        let need_rating = self.rating_style != RatingStyle::Off && !self.ratings.contains_key(&key);
        self.audio = if full { bulk } else { cached.unwrap_or(bulk) };
        if need_audio || need_rating {
            let _ = self.plex.tx.send(PlexCmd::LoadTrackInfo(key));
        }
        self.level_around_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyTable, UiConfig};
    use crate::plex::Track;
    use crate::ui::app::PlexHandle;
    use image::DynamicImage;
    use std::sync::Arc;

    fn cover_track(i: usize) -> Track {
        Track {
            rating_key: format!("t{i}"),
            title: format!("T{i}"),
            artist: "A".into(),
            album: "B".into(),
            track_no: None,
            year: None,
            duration_ms: None,
            parent_rating_key: Some(format!("al{i}")),
            grandparent_rating_key: None,
            thumb: None,
            part_key: Some(format!("/p/{i}")),
            container: None,
            audio: AudioInfo::default(),
            playlist_item_id: None,
            user_rating: None,
            last_viewed_at: None,
            plays: None,
        }
    }

    fn play_pos(app: &mut App, i: u64) {
        app.handle_msg(Msg::Mpv(MpvEvent::Property {
            name: "playlist-pos".into(),
            value: Some(serde_json::json!(i)),
        }));
    }

    /// A late cover must not leave its path marked in flight.
    #[tokio::test]
    async fn late_cover_can_be_requested_again() {
        let cfg = crate::config::MpvConfig {
            path: "mpv".into(),
            extra_args: vec!["--ao=null".into()],
            volume: 50.0,
            ..crate::config::MpvConfig::default()
        };
        let (mpv, _events) = crate::playback::mpv::spawn(&cfg).await.expect("spawn mpv");
        let (plex_tx, mut plex_rx) = tokio::sync::mpsc::unbounded_channel();
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            "http://127.0.0.1:1".into(),
            "TOK".into(),
            PlexHandle { tx: plex_tx },
            mpv,
            50.0,
            None,
            true,
            [0, 0],
            msg_tx,
            UiConfig::default(),
            &KeyTable::default(),
        );
        app.queue.items = (0..2).map(cover_track).collect();
        let thumb = |i: usize| format!("/library/metadata/al{i}/thumb");
        let loads = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<PlexCmd>| {
            let mut paths = Vec::new();
            while let Ok(cmd) = rx.try_recv() {
                if let PlexCmd::LoadThumb { path } = cmd {
                    paths.push(path);
                }
            }
            paths
        };

        let cover = || Arc::new(DynamicImage::new_rgba8(2, 2));
        play_pos(&mut app, 0);
        assert_eq!(loads(&mut plex_rx), [thumb(0)]);
        app.handle_msg(Msg::ArtReady { thumb_path: thumb(0), img: cover(), file: None });
        play_pos(&mut app, 1);
        assert_eq!(loads(&mut plex_rx), [thumb(1)]);
        // back on the cover that is showing, then the other one lands too late
        play_pos(&mut app, 0);
        app.handle_msg(Msg::ArtReady { thumb_path: thumb(1), img: cover(), file: None });
        play_pos(&mut app, 1);
        assert_eq!(loads(&mut plex_rx), [thumb(1)]);
    }
}
