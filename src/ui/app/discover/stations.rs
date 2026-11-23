use super::{DiscoverCmd, DiscoverMsg, Pane, Preview, Row, TracksPane, TracksSource};
use crate::plex::radio::Station;
use crate::plex::sonic::Features;
use crate::plex::PlexClient;
use crate::ui::app::radio::{self, RadioEnv, RadioOutcome, RadioSeed};
use crate::ui::app::{App, ListRef, Msg, PlexCmd};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

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
        RadioOutcome::Tracks(tracks, queue) => Preview::Ready(tracks, queue),
        RadioOutcome::NoStation => Preview::Failed(RadioOutcome::Empty.problem().into()),
        other => Preview::Failed(other.problem().into()),
    };
    let _ = tx.send(Msg::Discover(DiscoverMsg::StationPreview { key, preview }));
}

impl App {
    pub(super) fn highlighted_station(&self) -> Option<Station> {
        match self.highlighted_row()? {
            Row::Station(i) => self.discover.stations.get(i).cloned(),
            _ => None,
        }
    }

    pub(super) fn follow_station(&mut self, i: usize) {
        self.discover.pane = Pane::Empty;
        let Some(station) = self.discover.stations.get(i).cloned() else { return };
        if !self.allow_writes {
            return;
        }
        let failed = matches!(self.discover.previews.get(&station.key), Some(Preview::Failed(_)));
        if self.discover.played.remove(&station.key) || failed {
            self.discover.previews.remove(&station.key);
        }
        if !self.discover.previews.contains_key(&station.key) {
            self.discover.previews.insert(station.key.clone(), Preview::Loading);
            let _ = self.plex.tx.send(PlexCmd::Discover(DiscoverCmd::PreviewStation {
                key: station.key,
                title: station.title,
            }));
        }
        self.show_preview();
    }

    pub(in crate::ui::app) fn station_preview(&mut self, key: String, preview: Preview) {
        self.discover.previews.insert(key.clone(), preview);
        let followed = match self.discover.followed {
            Some(Row::Station(i)) => self.discover.stations.get(i).map(|s| s.key.as_str()),
            _ => None,
        };
        if followed == Some(key.as_str()) {
            self.show_preview();
        }
    }

    fn show_preview(&mut self) {
        self.drop_visual(ListRef::DTracks);
        let Some(Row::Station(i)) = self.discover.followed else { return };
        let Some(station) = self.discover.stations.get(i) else { return };
        self.discover.pane = match self.discover.shown_preview().cloned() {
            Some(Preview::Ready(tracks, queue)) => {
                let mut pane = TracksPane {
                    shelf: String::new(),
                    name: station.title.clone(),
                    total: tracks.len(),
                    tracks,
                    state: Default::default(),
                    source: TracksSource::Station(queue),
                };
                pane.state.select((!pane.tracks.is_empty()).then_some(0));
                Pane::Tracks(pane)
            }
            _ => Pane::Empty,
        };
        self.settle_focus();
    }

    pub(in crate::ui::app) fn start_station(&mut self) {
        let Some(station) = self.highlighted_station() else { return };
        if !self.allow_writes {
            self.transient(
                "a station creates a play queue on the server: turn on plex.allow_writes".into(),
            );
            return;
        }
        match self.discover.previews.get(&station.key) {
            Some(Preview::Ready(tracks, queue)) if !tracks.is_empty() => {
                let (tracks, queue) = (tracks.clone(), queue.clone());
                self.discover.played.insert(station.key);
                self.play_radio(tracks, station.title, queue);
            }
            Some(Preview::Failed(msg)) => self.transient(msg.clone()),
            _ => self.transient(format!("still loading {}…", station.title)),
        }
    }

    pub(super) fn play_station_from(&mut self) {
        let Some(real) = self.real_at(ListRef::DTracks) else { return };
        let Pane::Tracks(pane) = &self.discover.pane else { return };
        let TracksSource::Station(queue) = &pane.source else { return };
        let (name, queue, tracks) =
            (pane.name.clone(), queue.clone(), pane.tracks[real..].to_vec());
        if let Some(station) = self.highlighted_station() {
            self.discover.played.insert(station.key);
        }
        self.play_radio(tracks, name, queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;
    use tokio::sync::mpsc::unbounded_channel;

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

    async fn server(
        features: &'static str,
        n: usize,
    ) -> (String, crate::plex::client::testing::Seen) {
        let (base, seen, _) = serve_full(move |line| {
            let body = if line.starts_with("GET / ") {
                format!("<MediaContainer ownerFeatures=\"{features}\"/>")
            } else if line.starts_with("GET /identity") {
                "<MediaContainer machineIdentifier=\"mid1\"/>".into()
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
        let [Msg::Discover(DiscoverMsg::StationPreview { key, preview: Preview::Ready(tracks, _) })] =
            msgs.as_slice()
        else {
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
            let [Msg::Discover(DiscoverMsg::StationPreview {
                preview: Preview::Failed(msg), ..
            })] = msgs.as_slice()
            else {
                panic!("expected one failed preview");
            };
            assert_eq!(msg, want);
        }
    }
}
