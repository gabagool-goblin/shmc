//! `s`: Plex's radio for a track or an artist, opened in the Search tab's list.

use super::plex_worker::cached_features;
use super::{App, ListRef, Msg, PlexCmd, SearchDetail, Tab};
use crate::plex::sonic::Features;
use crate::plex::{HitKind, PlexClient, Track};
use ratatui::widgets::TableState;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// How many tracks to read from a station.
const WANT: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioSeed {
    Track(String),
    Artist(String),
}

/// The `s` request whose reply may still open the Search tab's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioRequest {
    pub key: String,
    /// The tab `s` was pressed in.
    pub origin: Tab,
}

#[derive(Debug)]
pub enum RadioOutcome {
    Tracks(Vec<Track>),
    /// The server has no Plex Pass radio.
    NoRadio,
    /// The artist lists no station.
    NoStation,
    NoClientId,
    /// The play queue failed or came back empty.
    Empty,
}

/// What the worker keeps between radio requests.
#[derive(Clone, Default)]
pub struct RadioEnv {
    machine_id: Arc<Mutex<Option<String>>>,
    client_id: Option<String>,
}

impl RadioEnv {
    pub fn new(client_id: Option<String>) -> Self {
        Self { client_id, ..Self::default() }
    }

    /// Read once per session.
    async fn machine_id(&self, client: &PlexClient) -> Option<String> {
        let known = self.machine_id.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if known.is_some() {
            return known;
        }
        let id = client.machine_id().await.inspect_err(|e| tracing::warn!("machine id: {e:#}"));
        let id = id.ok()?;
        *self.machine_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
        Some(id)
    }
}

/// Worker side: replies exactly once with `Msg::RadioLoaded`.
pub(super) async fn run(
    client: &PlexClient,
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    tx: &UnboundedSender<Msg>,
    key: String,
    seed: RadioSeed,
    label: String,
) {
    let (label, outcome) = start(client, features, env, seed, label).await;
    let _ = tx.send(Msg::RadioLoaded { key, label, outcome });
}

async fn start(
    client: &PlexClient,
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    seed: RadioSeed,
    label: String,
) -> (String, RadioOutcome) {
    let Some(client_id) = env.client_id.as_deref() else {
        return (label, RadioOutcome::NoClientId);
    };
    if cached_features(client, features).await.is_some_and(|f| !f.radio) {
        return (label, RadioOutcome::NoRadio);
    }
    let (station_key, label) = match seed {
        RadioSeed::Track(key) => match crate::plex::radio::track_station_key(&key) {
            Ok(k) => (k, label),
            Err(e) => {
                tracing::warn!("radio: {e:#}");
                return (label, RadioOutcome::Empty);
            }
        },
        RadioSeed::Artist(key) => match client.artist_station(&key).await {
            Ok(Some(s)) => (s.key, if s.title.is_empty() { label } else { s.title }),
            Ok(None) => return (label, RadioOutcome::NoStation),
            Err(e) => {
                tracing::warn!("radio station: {e:#}");
                return (label, RadioOutcome::Empty);
            }
        },
    };
    let Some(machine_id) = env.machine_id(client).await else {
        return (label, RadioOutcome::Empty);
    };
    let outcome = match client.radio_tracks(client_id, &machine_id, &station_key, WANT).await {
        Ok(t) if !t.is_empty() => RadioOutcome::Tracks(t),
        Ok(_) => RadioOutcome::Empty,
        Err(e) => {
            tracing::warn!("radio: {e:#}");
            RadioOutcome::Empty
        }
    };
    (label, outcome)
}

struct Seed {
    key: String,
    seed: RadioSeed,
    label: String,
}

impl Seed {
    fn track(t: &Track) -> Self {
        Self::track_key(&t.rating_key, &t.title)
    }

    fn track_key(key: &str, title: &str) -> Self {
        Self {
            key: key.to_string(),
            seed: RadioSeed::Track(key.to_string()),
            label: format!("{title} Radio"),
        }
    }

    fn artist(key: &str, name: &str) -> Self {
        Self {
            key: key.to_string(),
            seed: RadioSeed::Artist(key.to_string()),
            label: format!("{name} Radio"),
        }
    }
}

impl App {
    fn radio_seed(&self) -> Option<Seed> {
        let list = self.active_list();
        let real = self.real_at(list)?;
        match list {
            ListRef::Queue => self.queue.items.get(real).map(Seed::track),
            ListRef::BTracks => self.browser.tracks.items.get(real).map(Seed::track),
            ListRef::PlItems => self.playlists.items.get(real).map(Seed::track),
            ListRef::SearchDetail => self.search.detail.as_ref()?.tracks.get(real).map(Seed::track),
            ListRef::BArtists => {
                self.browser.artists.items.get(real).map(|a| Seed::artist(&a.rating_key, &a.title))
            }
            ListRef::Search => {
                let hit = self.search.hits.get(real)?;
                match hit.kind {
                    HitKind::Track => Some(Seed::track_key(&hit.rating_key, &hit.title)),
                    HitKind::Artist => Some(Seed::artist(&hit.rating_key, &hit.artist)),
                    HitKind::Album => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn start_radio(&mut self) {
        let Some(seed) = self.radio_seed() else {
            self.transient("s works on a track or artist".into());
            return;
        };
        if !self.allow_writes {
            self.transient(
                "radio creates a play queue on the server: turn on plex.allow_writes".into(),
            );
            return;
        }
        self.clear_list_requests();
        self.search.radio_request = Some(RadioRequest { key: seed.key.clone(), origin: self.tab });
        let _ = self.plex.tx.send(PlexCmd::StartRadio {
            key: seed.key,
            seed: seed.seed,
            label: seed.label,
        });
        self.transient("starting radio…".into());
    }

    pub(super) fn radio_loaded(&mut self, key: String, label: String, outcome: RadioOutcome) {
        let Some(req) = self.search.radio_request.take_if(|r| r.key == key) else {
            return;
        };
        match outcome {
            RadioOutcome::Tracks(tracks) => {
                self.push_back_for_detail(req.origin);
                let mut state = TableState::default();
                state.select(Some(0));
                self.set_search_detail(Some(SearchDetail {
                    key,
                    label,
                    tracks,
                    state,
                    matches: None,
                }));
                self.search.focused = false;
                self.goto_tab(Tab::Search);
            }
            RadioOutcome::NoRadio => self.transient("radio needs Plex Pass".into()),
            RadioOutcome::NoStation => self.transient("Plex lists no radio for this artist".into()),
            RadioOutcome::NoClientId => {
                self.transient("radio is off: cannot store the client id".into());
            }
            RadioOutcome::Empty => self.transient("the station returned no tracks".into()),
        }
    }
}
