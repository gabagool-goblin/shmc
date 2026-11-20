//! Plex radio: `s` on a track or an artist, and the station the queue plays from.

use super::plex_worker::cached_features;
use super::{App, Confirm, ListKind, ListRef, Msg, PlexCmd, SearchDetail, Tab};
use crate::plex::radio::{RadioQueue, ReadOn};
use crate::plex::sonic::Features;
use crate::plex::{HitKind, PlexClient, Track};
use crate::ui::queue_seq::{QueueOutcome, QueueReplyGuard};
use ratatui::widgets::TableState;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// How many tracks to read when a station starts.
const WANT: usize = 30;

/// How many tracks to read each time the queue nears its end.
const MORE: usize = 20;

/// Read more once this many tracks or fewer follow the playing one.
const NEAR_END: usize = 10;

/// The station the queue plays from.
pub struct ActiveRadio {
    pub name: String,
    queue: RadioQueue,
    /// Seq of the last read for more tracks.
    asked: Option<u64>,
    /// The playing track when a read failed; no new read until it changes.
    failed_at: Option<(usize, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioSeed {
    Track(String),
    Artist(String),
    /// A library station's key.
    Station(String),
}

#[derive(Debug)]
pub enum RadioOutcome {
    Tracks(Vec<Track>, RadioQueue),
    /// The server has no Plex Pass radio.
    NoRadio,
    /// The artist lists no station.
    NoStation,
    NoClientId,
    /// The play queue failed or came back empty.
    Empty,
}

impl RadioOutcome {
    pub(super) fn problem(&self) -> &'static str {
        match self {
            RadioOutcome::NoRadio => "radio needs Plex Pass",
            RadioOutcome::NoStation => "no radio for this artist",
            RadioOutcome::NoClientId => "radio is off: cannot store the client id",
            RadioOutcome::Tracks(..) | RadioOutcome::Empty => "the station returned no tracks",
        }
    }
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

pub(super) async fn start(
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
        RadioSeed::Station(key) => (key, label),
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
        Ok((t, queue)) if !t.is_empty() => RadioOutcome::Tracks(t, queue),
        Ok(_) => RadioOutcome::Empty,
        Err(e) => {
            tracing::warn!("radio: {e:#}");
            RadioOutcome::Empty
        }
    };
    (label, outcome)
}

/// Worker side: replies once with `QueueOutcome::RadioMore`.
pub(super) async fn more(
    client: &PlexClient,
    env: &RadioEnv,
    tx: UnboundedSender<Msg>,
    seq: u64,
    queue: RadioQueue,
) {
    let reply = QueueReplyGuard::new(tx, seq);
    let result = match env.client_id.as_deref() {
        Some(client_id) => client.radio_read_on(client_id, &queue, MORE).await,
        None => ReadOn::Failed("no client id".into()),
    };
    reply.send(QueueOutcome::RadioMore { id: queue.id, result });
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
            ListRef::PlItems => self.playlists.tracks.items.get(real).map(Seed::track),
            ListRef::SearchDetail => self.search.detail.as_ref()?.tracks.get(real).map(Seed::track),
            ListRef::DTracks => self.discover.tracks.get(real).map(Seed::track),
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
        self.search.request_list(seed.key.clone(), ListKind::Radio, self.tab);
        let _ = self.plex.tx.send(PlexCmd::StartRadio {
            key: seed.key,
            seed: seed.seed,
            label: seed.label,
        });
        self.transient("starting radio…".into());
    }

    pub(super) fn radio_loaded(&mut self, key: String, label: String, outcome: RadioOutcome) {
        let Some(req) = self.search.take_list_request(&key, |k| k == ListKind::Radio) else {
            return;
        };
        match outcome {
            RadioOutcome::Tracks(tracks, queue) => {
                self.push_back_for_detail(req.origin);
                let mut state = TableState::default();
                state.select(Some(0));
                self.set_search_detail(Some(SearchDetail {
                    key,
                    label,
                    tracks,
                    state,
                    matches: None,
                    radio: Some(queue),
                }));
                self.search.focused = false;
                self.goto_tab(Tab::Search);
            }
            other => self.transient(other.problem().into()),
        }
    }

    /// Replaces the queue with `tracks`, shuffle off, and makes `queue` the
    /// active radio once the replace applies.
    pub(super) fn play_radio(&mut self, tracks: Vec<Track>, name: String, queue: RadioQueue) {
        self.random = false;
        let confirm = Confirm::FromList { name: name.clone() };
        self.queue_outcome_local(true, QueueOutcome::Radio { tracks, confirm, name, queue });
    }

    pub(super) fn begin_radio(&mut self, name: String, queue: RadioQueue) {
        self.radio = Some(ActiveRadio { name, queue, asked: None, failed_at: None });
    }

    /// Radio plays in its own order.
    pub(super) fn refuse_shuffle_in_radio(&mut self) -> bool {
        if self.radio.is_none() {
            return false;
        }
        self.transient("shuffle is off while radio plays".into());
        true
    }

    fn playing_track(&self) -> Option<(usize, String)> {
        let c = self.current?;
        Some((c, self.queue.items.get(c)?.rating_key.clone()))
    }

    /// Runs after every message; reads more tracks when the queue nears its end.
    pub(super) fn follow_radio(&mut self) {
        let Some(radio) = self.radio.as_ref() else { return };
        let Some(playing) = self.playing_track() else { return };
        let ahead = self.queue.items.len().saturating_sub(playing.0 + 1);
        let in_flight = radio.asked.is_some_and(|s| !self.queue_seq.settled(s));
        if self.stopped
            || self.expect_pos.is_some()
            || ahead > NEAR_END
            || in_flight
            || radio.failed_at.as_ref() == Some(&playing)
        {
            return;
        }
        let queue = radio.queue.clone();
        let seq = self.issue_seq(false);
        if let Some(radio) = self.radio.as_mut() {
            radio.asked = Some(seq);
        }
        let _ = self.plex.tx.send(PlexCmd::RadioMore { seq, queue });
    }

    pub(super) fn apply_radio_more(&mut self, id: &str, result: ReadOn) {
        let playing = self.playing_track();
        let Some(radio) = self.radio.as_mut().filter(|r| r.queue.id == id) else { return };
        radio.asked = None;
        match result {
            ReadOn::Tracks(tracks, last) => {
                radio.queue.last = last;
                self.append_playable(tracks);
            }
            ReadOn::Exhausted => {
                let name = std::mem::take(&mut radio.name);
                self.radio = None;
                self.transient(format!("{name} has no more tracks"));
            }
            ReadOn::Gone => {
                self.radio = None;
                self.transient_err("radio ended: the station no longer exists".into());
            }
            ReadOn::Failed(e) => {
                tracing::warn!("radio: reading more tracks: {e}");
                radio.failed_at = playing;
            }
        }
    }
}
