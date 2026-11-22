use super::{Pane, Row, TracksSource};
use crate::plex::discover::ShelfItems;
use crate::ui::app::{App, Confirm, ListRef, QueueAction};

impl App {
    pub(in crate::ui::app) fn activate_section(&mut self) {
        match self.highlighted_row() {
            Some(Row::Station(_)) => self.start_station(),
            Some(Row::Shelf(_)) => self.discover_column_right(),
            _ => {}
        }
    }

    pub(in crate::ui::app) fn activate_discover_track(&mut self) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Pane::Tracks(pane) = &self.discover.pane else { return };
        match &pane.source {
            TracksSource::Station(_) => self.play_station_from(),
            TracksSource::Album | TracksSource::Library(_) => {
                let confirm = Confirm::FromList { name: pane.name.clone() };
                let tracks = pane.tracks[real..].to_vec();
                self.queue_local(tracks, confirm, QueueAction::Replace);
            }
            TracksSource::Playlist { .. } => {
                self.queue_discover_playlist(real, QueueAction::Replace);
            }
        }
    }

    pub(in crate::ui::app) fn add_discover_track(&mut self, action: QueueAction) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Pane::Tracks(pane) = &self.discover.pane else { return };
        if matches!(pane.source, TracksSource::Playlist { .. }) {
            return self.queue_discover_playlist(real, action);
        }
        let Some(t) = pane.tracks.get(real).cloned() else { return };
        let confirm = Confirm::Track { title: t.title.clone() };
        self.queue_local(vec![t], confirm, action);
    }

    fn queue_discover_playlist(&mut self, real: usize, action: QueueAction) {
        let Pane::Tracks(pane) = &self.discover.pane else { return };
        let TracksSource::Playlist { key } = &pane.source else { return };
        let Some(track) = pane.tracks.get(real).cloned() else { return };
        let (key, name) = (key.clone(), pane.name.clone());
        let total = u32::try_from(pane.total).ok();
        self.queue_playlist_track(key, name, total, real, &track, action);
    }

    pub(in crate::ui::app) fn add_discover_item(&mut self, action: QueueAction) {
        let Some(real) = self.real_at(ListRef::DItems) else { return };
        let Pane::Items(pane) = &self.discover.pane else { return };
        match &pane.items {
            ShelfItems::Albums(v) => {
                let Some(a) = v.get(real) else { return };
                let (key, name) = (a.rating_key.clone(), a.title.clone());
                self.resolve_album(key, name, action);
            }
            ShelfItems::Artists(v) => {
                let Some(a) = v.get(real) else { return };
                let (key, name) = (a.rating_key.clone(), a.title.clone());
                self.resolve_artist(key, name, action);
            }
            ShelfItems::Playlists(v) => {
                let Some(p) = v.get(real) else { return };
                let confirm = Confirm::Playlist { name: p.title.clone() };
                let (key, count) = (p.rating_key.clone(), p.item_count);
                self.queue_playlist(key, confirm, action, count, None);
            }
        }
    }
}
