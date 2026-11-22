//! Plex radio: `s` on a track or an artist, and the station the queue plays from.

use super::plex_worker::cached_features;
use super::{App, Confirm, ListKind, ListRef, Msg, PlexCmd, SearchDetail, Tab};
pub use crate::plex::radio::RadioSeed;
use crate::plex::radio::{RadioQueue, ReadOn};
use crate::plex::sonic::Features;
use crate::plex::{HitKind, PlexClient, Track};
use crate::ui::queue_seq::{QueueOutcome, QueueReplyGuard};
use ratatui::widgets::TableState;
use std::collections::HashSet;
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
    /// Rating keys of the tracks the radio added.
    added: HashSet<String>,
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
    let (station_key, label) = match &seed {
        RadioSeed::Track(key) => match crate::plex::radio::track_station_key(key) {
            Ok(k) => (k, label),
            Err(e) => {
                tracing::warn!("radio: {e:#}");
                return (label, RadioOutcome::Empty);
            }
        },
        RadioSeed::Station(key) => (key.clone(), label),
        RadioSeed::Artist(key) => match client.artist_station(key).await {
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
    let outcome = match client.radio_tracks(client_id, &machine_id, &station_key, seed, WANT).await
    {
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
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    tx: UnboundedSender<Msg>,
    seq: u64,
    queue: RadioQueue,
    skip: HashSet<String>,
) {
    let reply = QueueReplyGuard::new(tx, seq);
    let result = match env.client_id.as_deref() {
        Some(client_id) => match client.radio_read_on(client_id, &queue, MORE, &skip).await {
            ReadOn::Gone => restart(client, features, env, queue.seed.clone(), &skip).await,
            other => other,
        },
        None => ReadOn::Failed("no client id".into()),
    };
    reply.send(QueueOutcome::RadioMore { id: queue.id, result });
}

async fn restart(
    client: &PlexClient,
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    seed: RadioSeed,
    skip: &HashSet<String>,
) -> ReadOn {
    tracing::info!("radio: the play queue is gone, starting the station again");
    match start(client, features, env, seed, String::new()).await.1 {
        RadioOutcome::Tracks(tracks, queue) => {
            let tracks = tracks.into_iter().filter(|t| !skip.contains(&t.rating_key)).collect();
            ReadOn::Restarted(tracks, queue)
        }
        other => {
            tracing::warn!("radio: starting the station again: {}", other.problem());
            ReadOn::Gone
        }
    }
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
            ListRef::DTracks => self.discover.tracks().get(real).map(Seed::track),
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

    /// Replaces the queue, shuffle off; `queue` becomes the active radio once that applies.
    pub(super) fn play_radio(&mut self, tracks: Vec<Track>, name: String, queue: RadioQueue) {
        self.random = false;
        let confirm = Confirm::FromList { name: name.clone() };
        self.queue_outcome_local(true, QueueOutcome::Radio { tracks, confirm, name, queue });
    }

    pub(super) fn begin_radio(&mut self, name: String, queue: RadioQueue) {
        let added = self.queue.items.iter().map(|t| t.rating_key.clone()).collect();
        self.radio = Some(ActiveRadio { name, queue, asked: None, failed_at: None, added });
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
        let (queue, skip) = (radio.queue.clone(), radio.added.clone());
        let seq = self.issue_seq(false);
        if let Some(radio) = self.radio.as_mut() {
            radio.asked = Some(seq);
        }
        let _ = self.plex.tx.send(PlexCmd::RadioMore { seq, queue, skip });
    }

    pub(super) fn apply_radio_more(&mut self, id: &str, result: ReadOn) {
        let playing = self.playing_track();
        let Some(radio) = self.radio.as_mut().filter(|r| r.queue.id == id) else { return };
        radio.asked = None;
        match result {
            ReadOn::Tracks(tracks, last) => {
                radio.queue.last = last;
                radio.added.extend(tracks.iter().map(|t| t.rating_key.clone()));
                self.append_playable(tracks);
            }
            ReadOn::Restarted(tracks, queue) => {
                radio.queue = queue;
                radio.added.extend(tracks.iter().map(|t| t.rating_key.clone()));
                self.append_playable(tracks);
            }
            ReadOn::Exhausted => {
                let name = std::mem::take(&mut radio.name);
                self.radio = None;
                tracing::info!("radio ended: the station has no more tracks");
                self.transient(format!("{name} has no more tracks"));
            }
            ReadOn::Gone => {
                self.radio = None;
                tracing::info!("radio ended: the station no longer exists");
                self.transient_err("radio ended: the station no longer exists".into());
            }
            ReadOn::Failed(e) => {
                tracing::warn!("radio: reading more tracks: {e}");
                radio.failed_at = playing;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;
    use tokio::sync::mpsc::unbounded_channel;

    const KEY: &str = "/library/sections/1/stations/8";

    fn queue_xml(n: usize) -> String {
        let tracks: String = (1..=n)
            .map(|i| {
                format!(
                    "<Track ratingKey=\"{i}\" key=\"/library/metadata/{i}\" title=\"T{i}\" playQueueItemID=\"{i}\"><Media><Part id=\"{i}\" key=\"/library/parts/{i}/f.mp3\"/></Media></Track>"
                )
            })
            .collect();
        format!("<MediaContainer playQueueID=\"10\">{tracks}</MediaContainer>")
    }

    /// Play queue 9 is gone; `create` is the status of `POST /playQueues`.
    async fn read_more(create: u16) -> (ReadOn, String, Vec<String>) {
        let (base, seen, _) = serve_full(move |line| match line {
            l if l.starts_with("GET / ") => {
                (200, "<MediaContainer ownerFeatures=\"radio\"/>".into())
            }
            l if l.starts_with("GET /identity") => {
                (200, "<MediaContainer machineIdentifier=\"mid1\"/>".into())
            }
            l if l.starts_with("POST /playQueues") => (create, queue_xml(5)),
            l if l.starts_with("GET /playQueues/10") => (200, queue_xml(5)),
            _ => (404, String::new()),
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let env = RadioEnv::new(Some("cid1".into()));
        let (tx, mut rx) = unbounded_channel();
        let seed = RadioSeed::Station(KEY.into());
        let queue = RadioQueue { id: "9".into(), last: "30".into(), seed };
        let skip = ["1", "2"].into_iter().map(String::from).collect();
        more(&client, &Mutex::new(None), &env, tx, 4, queue, skip).await;
        let Ok(Msg::QueueReply { seq: 4, outcome: QueueOutcome::RadioMore { id, result } }) =
            rx.try_recv()
        else {
            panic!("expected one RadioMore reply");
        };
        let lines = seen.lock().unwrap().clone();
        (result, id, lines)
    }

    #[tokio::test]
    async fn a_gone_play_queue_starts_the_station_again_without_repeats() {
        let (result, id, lines) = read_more(200).await;
        assert_eq!(id, "9");
        let ReadOn::Restarted(tracks, queue) = result else { panic!("expected a restart") };
        let keys: Vec<_> = tracks.iter().map(|t| t.rating_key.as_str()).collect();
        assert_eq!(keys, ["3", "4", "5"]);
        let seed = RadioSeed::Station(KEY.into());
        assert_eq!(queue, RadioQueue { id: "10".into(), last: "5".into(), seed });
        assert_eq!(lines.iter().filter(|l| l.starts_with("POST ")).count(), 1);
    }

    #[tokio::test]
    async fn a_station_that_cannot_start_again_is_gone() {
        let (result, _, lines) = read_more(404).await;
        assert!(matches!(result, ReadOn::Gone), "{result:?}");
        assert_eq!(lines.iter().filter(|l| l.starts_with("POST ")).count(), 1);
    }
}
