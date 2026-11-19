//! The Discover tab: the library's radio stations.

use super::plex_worker::cached_features;
use super::radio::{self, RadioEnv, RadioOutcome, RadioSeed};
use super::{App, Confirm, ListRef, Msg, PlexCmd, QueueAction, Tab};
use crate::plex::radio::Station;
use crate::plex::sonic::Features;
use crate::plex::{PlexClient, Track};
use ratatui::widgets::{ListState, TableState};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

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
    Ready(Vec<Track>),
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

/// Worker side: replies once with `Msg::StationsLoaded`.
pub(super) async fn load_stations(
    client: &PlexClient,
    section: &str,
    features: &Mutex<Option<Features>>,
    tx: &UnboundedSender<Msg>,
) {
    let outcome = if cached_features(client, features).await.is_some_and(|f| !f.radio) {
        StationsOutcome::NoRadio
    } else {
        match client.library_stations(section).await {
            Ok(stations) => StationsOutcome::Stations(stations),
            Err(e) => {
                tracing::warn!("loading stations: {e:#}");
                StationsOutcome::Failed
            }
        }
    };
    let _ = tx.send(Msg::StationsLoaded(outcome));
}

/// Worker side: replies once with `Msg::StationPreview`.
pub(super) async fn preview_station(
    client: &PlexClient,
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    tx: &UnboundedSender<Msg>,
    key: String,
    title: String,
) {
    let seed = RadioSeed::Station(key.clone());
    let preview = match radio::start(client, features, env, seed, title).await.1 {
        RadioOutcome::Tracks(tracks) => Preview::Ready(tracks),
        RadioOutcome::NoStation => Preview::Failed(RadioOutcome::Empty.problem().into()),
        other => Preview::Failed(other.problem().into()),
    };
    let _ = tx.send(Msg::StationPreview { key, preview });
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

    fn highlighted_station(&self) -> Option<Station> {
        let real = self.real_at(ListRef::DStations)?;
        self.discover.stations.get(real).cloned()
    }

    /// Runs after every message; acts only when the highlighted station changed.
    pub(super) fn follow_station(&mut self) {
        if self.tab != Tab::Discover
            || self.discover.load != StationsLoad::Loaded
            || !self.allow_writes
        {
            return;
        }
        let Some(station) = self.highlighted_station() else { return };
        if self.discover.shown.as_deref() == Some(station.key.as_str()) {
            return;
        }
        if self.discover.played.remove(&station.key) {
            self.discover.previews.remove(&station.key);
        }
        self.discover.shown = Some(station.key.clone());
        if !self.discover.previews.contains_key(&station.key) {
            self.discover.previews.insert(station.key.clone(), Preview::Loading);
            let _ = self
                .plex
                .tx
                .send(PlexCmd::PreviewStation { key: station.key, title: station.title });
        }
        self.show_preview();
    }

    pub(super) fn station_preview(&mut self, key: String, preview: Preview) {
        self.discover.previews.insert(key.clone(), preview);
        if self.discover.shown.as_deref() == Some(key.as_str()) {
            self.show_preview();
        }
    }

    fn show_preview(&mut self) {
        self.drop_visual(ListRef::DTracks);
        let title = self.discover.shown.as_ref().and_then(|key| {
            self.discover.stations.iter().find(|s| &s.key == key).map(|s| s.title.clone())
        });
        match self.discover.shown_preview().cloned() {
            Some(Preview::Ready(tracks)) => {
                self.discover.tracks_state.select((!tracks.is_empty()).then_some(0));
                self.discover.tracks = tracks;
                self.discover.label = title;
            }
            _ => {
                self.discover.tracks.clear();
                self.discover.tracks_state.select(None);
                self.discover.label = None;
                self.discover.focus = DiscoverCol::Stations;
            }
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

    pub(super) fn start_station(&mut self) {
        let Some(station) = self.highlighted_station() else { return };
        if !self.allow_writes {
            self.transient(
                "a station creates a play queue on the server: turn on plex.allow_writes".into(),
            );
            return;
        }
        match self.discover.previews.get(&station.key) {
            Some(Preview::Ready(tracks)) if !tracks.is_empty() => {
                let tracks = tracks.clone();
                self.discover.played.insert(station.key);
                self.play_station_tracks(tracks, station.title);
            }
            Some(Preview::Failed(msg)) => self.transient(msg.clone()),
            _ => self.transient(format!("still loading {}…", station.title)),
        }
    }

    pub(super) fn play_discover_from(&mut self) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Some(name) = self.discover.label.clone() else { return };
        let tracks = self.discover.tracks[real..].to_vec();
        if let Some(key) = self.discover.shown.clone() {
            self.discover.played.insert(key);
        }
        self.play_station_tracks(tracks, name);
    }

    /// Radio plays in its own order, so shuffle goes off.
    fn play_station_tracks(&mut self, tracks: Vec<Track>, name: String) {
        self.random = false;
        self.queue_local(tracks, Confirm::FromList { name }, QueueAction::Replace);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;
    use tokio::sync::mpsc::unbounded_channel;

    const STATIONS: &str = include_str!("../../../tests/fixtures/hubs_stations.xml");

    fn queue_xml(n: usize) -> String {
        let tracks: String = (1..=n)
            .map(|i| {
                format!(
                    "<Track ratingKey=\"{i}\" key=\"/library/metadata/{i}\" title=\"T{i}\" playQueueItemID=\"{i}\"><Media><Part id=\"{i}\" key=\"/library/parts/{i}/f.mp3\"/></Media></Track>"
                )
            })
            .collect();
        format!("<MediaContainer playQueueID=\"9\">{tracks}</MediaContainer>")
    }

    /// A server with radio, one station hub and a play queue of `n` tracks.
    async fn server(
        features: &'static str,
        n: usize,
    ) -> (String, crate::plex::client::testing::Seen) {
        let (base, seen, _) = serve_full(move |line| {
            let body = if line.starts_with("GET / ") {
                format!("<MediaContainer ownerFeatures=\"{features}\"/>")
            } else if line.starts_with("GET /identity") {
                "<MediaContainer machineIdentifier=\"mid1\"/>".into()
            } else if line.starts_with("GET /hubs/sections/1?includeStations=1") {
                STATIONS.into()
            } else if line.starts_with("POST /playQueues") {
                queue_xml(n)
            } else {
                return (404, String::new());
            };
            (200, body)
        })
        .await;
        (base, seen)
    }

    async fn listed(features: &'static str) -> Option<Vec<Station>> {
        let (base, _) = server(features, 0).await;
        let client = PlexClient::new(&base, "faketoken-stations");
        let (tx, mut rx) = unbounded_channel();
        load_stations(&client, "1", &Mutex::new(None), &tx).await;
        match rx.try_recv().unwrap() {
            Msg::StationsLoaded(StationsOutcome::Stations(s)) => Some(s),
            Msg::StationsLoaded(_) => None,
            _ => panic!("expected StationsLoaded"),
        }
    }

    #[tokio::test]
    async fn stations_load_from_the_section_hub_only_with_radio() {
        let stations = listed("radio").await.expect("stations");
        assert_eq!(stations.len(), 4);
        assert_eq!(stations[1].title, "Deep Cuts Radio");
        assert!(listed("music-analysis").await.is_none(), "no radio, no stations");
    }

    async fn previewed(client_id: Option<&str>, n: usize) -> (Vec<Msg>, Vec<String>) {
        let (base, seen) = server("radio", n).await;
        let client = PlexClient::new(&base, "faketoken-stations");
        let (tx, mut rx) = unbounded_channel();
        let env = RadioEnv::new(client_id.map(Into::into));
        let key = "/library/sections/1/stations/8".to_string();
        preview_station(&client, &Mutex::new(None), &env, &tx, key, "Deep Cuts Radio".into()).await;
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        let lines = seen.lock().unwrap().clone();
        (msgs, lines)
    }

    #[tokio::test]
    async fn a_preview_reads_the_station_key_as_is_and_replies_once() {
        let (msgs, seen) = previewed(Some("cid1"), 30).await;
        let [Msg::StationPreview { key, preview: Preview::Ready(tracks) }] = msgs.as_slice() else {
            panic!("expected one ready preview");
        };
        assert_eq!((key.as_str(), tracks.len()), ("/library/sections/1/stations/8", 30));
        let create = seen.iter().find(|l| l.starts_with("POST ")).unwrap();
        assert!(create.contains("stations%2F8"), "{create}");
    }

    #[tokio::test]
    async fn a_preview_that_fails_replies_once_with_the_reason() {
        for (client_id, want) in [
            (Some("cid1"), "the station returned no tracks"),
            (None, "radio is off: cannot store the client id"),
        ] {
            let (msgs, _) = previewed(client_id, 0).await;
            let [Msg::StationPreview { preview: Preview::Failed(msg), .. }] = msgs.as_slice()
            else {
                panic!("expected one failed preview");
            };
            assert_eq!(msg, want);
        }
    }
}
