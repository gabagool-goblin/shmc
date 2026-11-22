//! The Discover tab: the library's radio stations.

mod stations;

use super::{App, Confirm, ListRef, PlexCmd, QueueAction};
use crate::plex::radio::{RadioQueue, Station};
use crate::plex::Track;
use ratatui::widgets::{ListState, TableState};
pub(in crate::ui::app) use stations::{load_stations, preview_station};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverCol {
    #[default]
    Stations,
    Tracks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StationsLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    NoRadio,
    Failed,
}

#[derive(Debug)]
pub enum StationsOutcome {
    Stations(Vec<Station>),
    NoRadio,
    Failed,
}

#[derive(Debug, Clone)]
pub enum Preview {
    Loading,
    Ready(Vec<Track>, RadioQueue),
    Failed(String),
}

#[derive(Default)]
pub struct DiscoverView {
    pub stations: Vec<Station>,
    pub stations_state: ListState,
    pub load: StationsLoad,
    pub focus: DiscoverCol,
    /// The highlighted station's title, once its tracks are in.
    pub label: Option<String>,
    pub tracks: Vec<Track>,
    /// The play queue behind `tracks`.
    radio: Option<RadioQueue>,
    pub tracks_state: TableState,
    /// Each station's tracks, fetched once; every fetch makes a play queue.
    previews: HashMap<String, Preview>,
    /// Stations played since their preview was fetched.
    played: HashSet<String>,
    /// The station the right side follows.
    shown: Option<String>,
}

impl DiscoverView {
    pub fn shown_preview(&self) -> Option<&Preview> {
        self.previews.get(self.shown.as_ref()?)
    }
}

impl App {
    pub(super) fn discover_list(&self) -> ListRef {
        match self.discover.focus {
            DiscoverCol::Stations => ListRef::DStations,
            DiscoverCol::Tracks => ListRef::DTracks,
        }
    }

    pub(super) fn open_discover(&mut self) {
        if !matches!(self.discover.load, StationsLoad::NotLoaded | StationsLoad::Failed) {
            return;
        }
        self.discover.load = StationsLoad::Loading;
        let _ = self.plex.tx.send(PlexCmd::LoadStations);
    }

    pub(super) fn stations_loaded(&mut self, outcome: StationsOutcome) {
        match outcome {
            StationsOutcome::Stations(stations) => {
                self.discover.stations_state.select((!stations.is_empty()).then_some(0));
                self.discover.stations = stations;
                self.discover.load = StationsLoad::Loaded;
            }
            StationsOutcome::NoRadio => self.discover.load = StationsLoad::NoRadio,
            StationsOutcome::Failed => self.discover.load = StationsLoad::Failed,
        }
    }

    pub(super) fn discover_column_left(&mut self) {
        self.discover.focus = DiscoverCol::Stations;
    }

    pub(super) fn discover_column_right(&mut self) {
        if !self.discover.tracks.is_empty() {
            self.discover.focus = DiscoverCol::Tracks;
        }
    }

    pub(super) fn add_discover_track(&mut self, action: QueueAction) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Some(t) = self.discover.tracks.get(real).cloned() else { return };
        let confirm = Confirm::Track { title: t.title.clone() };
        self.queue_local(vec![t], confirm, action);
    }

    pub(super) fn reveal_discover(&mut self) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Some(t) = self.discover.tracks.get(real) else { return };
        let (artist, album, track) = (
            t.grandparent_rating_key.clone(),
            t.parent_rating_key.clone(),
            Some(t.rating_key.clone()),
        );
        self.reveal(artist, album, track);
    }
}
