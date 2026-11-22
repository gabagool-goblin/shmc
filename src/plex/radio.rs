//! Plex radio: artist and track stations and the play queues built from them.

use super::client::{read_capped, redirect_error, PlexClient, MAX_XML_BYTES};
use super::models::{MediaContainer, Track, WirePlaylist};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::io::Read;

/// The most reads of a play queue after the one that creates it.
const MAX_READS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioSeed {
    Track(String),
    Artist(String),
    /// A library station's key.
    Station(String),
}

/// A play queue and the `playQueueItemID` of the last track shmc read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioQueue {
    pub id: String,
    pub last: String,
    pub seed: RadioSeed,
}

#[derive(Debug)]
pub enum ReadOn {
    /// New tracks, and the id of the last item read.
    Tracks(Vec<Track>, String),
    /// Nothing follows the last item read.
    Exhausted,
    /// Plex deletes a client's play queue when it makes another.
    Gone,
    /// New tracks from a new play queue for the same seed.
    Restarted(Vec<Track>, RadioQueue),
    Failed(String),
}

#[derive(Debug)]
struct PlayQueueGone;

impl std::fmt::Display for PlayQueueGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Plex returned HTTP 404")
    }
}

impl std::error::Error for PlayQueueGone {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Station {
    pub key: String,
    pub title: String,
}

pub(crate) fn station_of(p: WirePlaylist) -> Option<Station> {
    Some(Station { key: p.key?, title: p.title.unwrap_or_default() })
}

/// The first station an artist lists.
pub fn station_from(c: MediaContainer) -> Option<Station> {
    c.directories.into_iter().find_map(|d| d.stations?.stations.into_iter().find_map(station_of))
}

/// The play queue's id and its tracks in order, each with its `playQueueItemID`.
pub fn play_queue_from(c: MediaContainer) -> (Option<String>, Vec<(Track, String)>) {
    let tracks = c
        .tracks
        .into_iter()
        .filter_map(|t| {
            let item = t.play_queue_item_id.clone()?;
            Some((t.into_track(), item))
        })
        .collect();
    (c.play_queue_id, tracks)
}

/// A random (version 4) UUID.
fn uuid_v4() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

/// A new station for a track; the server makes it on first use.
pub fn track_station_key(track_key: &str) -> Result<String> {
    let uuid = uuid_v4().context("generating a station id")?;
    Ok(format!("/library/metadata/{track_key}/station/{uuid}?type=10"))
}

impl PlexClient {
    /// The artist's radio station, when Plex lists one.
    pub async fn artist_station(&self, key: &str) -> Result<Option<Station>> {
        let c = self.get_xml(&format!("/library/metadata/{key}?includeStations=1")).await?;
        Ok(station_from(c))
    }

    /// The first `want` tracks of a new play queue; a failed read after the create keeps the rest.
    pub async fn radio_tracks(
        &self,
        client_id: &str,
        machine_id: &str,
        station_key: &str,
        seed: RadioSeed,
        want: usize,
    ) -> Result<(Vec<Track>, RadioQueue)> {
        let mut url = self.url_for("/playQueues")?;
        url.query_pairs_mut().append_pair("type", "audio").append_pair("own", "1").append_pair(
            "uri",
            &format!("server://{machine_id}/com.plexapp.plugins.library{station_key}"),
        );
        let c = self.play_queue_xml(reqwest::Method::POST, &url, client_id).await?;
        let (id, first) = play_queue_from(c);
        let id = id.filter(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric()));
        let id = id.context("Plex returned no play queue id")?;
        let mut seen: HashSet<String> = first.iter().map(|(_, i)| i.clone()).collect();
        let mut items = first;
        for _ in 0..MAX_READS {
            let Some((_, last)) = items.last().filter(|_| items.len() < want) else { break };
            let mut url = self.url_for(&format!("/playQueues/{id}"))?;
            url.query_pairs_mut().append_pair("center", last).append_pair("window", "200");
            let c = match self.play_queue_xml(reqwest::Method::GET, &url, client_id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("reading the play queue: {e:#}");
                    break;
                }
            };
            let before = items.len();
            for (t, i) in play_queue_from(c).1 {
                if seen.insert(i.clone()) {
                    items.push((t, i));
                }
            }
            if items.len() == before {
                break;
            }
        }
        items.truncate(want);
        let last = items.last().map(|(_, i)| i.clone()).context("the play queue is empty")?;
        Ok((items.into_iter().map(|(t, _)| t).collect(), RadioQueue { id, last, seed }))
    }

    /// Reads on until `want` tracks not in `skip`; a failure keeps what was read.
    pub async fn radio_read_on(
        &self,
        client_id: &str,
        queue: &RadioQueue,
        want: usize,
        skip: &HashSet<String>,
    ) -> ReadOn {
        let mut last = queue.last.clone();
        let mut found: Vec<(Track, String)> = Vec::new();
        for _ in 0..MAX_READS {
            match self.items_after(client_id, &queue.id, &last).await {
                Ok(next) => {
                    let Some((_, i)) = next.last() else { break };
                    last = i.clone();
                    found.extend(next.into_iter().filter(|(t, _)| !skip.contains(&t.rating_key)));
                }
                Err(e) if last == queue.last => {
                    return if e.is::<PlayQueueGone>() {
                        ReadOn::Gone
                    } else {
                        ReadOn::Failed(format!("{e:#}"))
                    };
                }
                Err(e) => {
                    tracing::warn!("reading the play queue: {e:#}");
                    break;
                }
            }
            if found.len() >= want {
                break;
            }
        }
        if found.len() > want {
            found.truncate(want);
            last = found[want - 1].1.clone();
        }
        if last == queue.last {
            return ReadOn::Exhausted;
        }
        ReadOn::Tracks(found.into_iter().map(|(t, _)| t).collect(), last)
    }

    async fn items_after(
        &self,
        client_id: &str,
        id: &str,
        last: &str,
    ) -> Result<Vec<(Track, String)>> {
        let mut url = self.url_for(&format!("/playQueues/{id}"))?;
        url.query_pairs_mut().append_pair("center", last).append_pair("window", "200");
        let c = self.play_queue_xml(reqwest::Method::GET, &url, client_id).await?;
        let items = play_queue_from(c).1;
        let at = items.iter().position(|(_, i)| i == last);
        let at = at.context("the last track is not in the play queue")?;
        Ok(items.into_iter().skip(at + 1).collect())
    }

    /// Play queues belong to the client that makes them, so each request names it.
    async fn play_queue_xml(
        &self,
        method: reqwest::Method,
        url: &url::Url,
        client_id: &str,
    ) -> Result<MediaContainer> {
        tracing::debug!(url = %crate::redact::escape_field(url.as_str()), %method, "plex play queue");
        let resp = self
            .request(method, url.as_str(), "application/xml")
            .header("X-Plex-Client-Identifier", client_id)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("request failed: {}", e.without_url()))?;
        let status = resp.status();
        if !status.is_success() {
            if let Some(msg) = redirect_error(&resp) {
                bail!(msg);
            }
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(PlayQueueGone.into());
            }
            bail!("Plex returned HTTP {status}");
        }
        let bytes = read_capped(resp, MAX_XML_BYTES, url.as_str()).await?;
        quick_xml::de::from_reader(&bytes[..]).context("parsing the play queue")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::{serve_full, Seen};

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

    fn seed() -> RadioSeed {
        RadioSeed::Station("/s".into())
    }

    fn queue(last: &str) -> RadioQueue {
        RadioQueue { id: "9".into(), last: last.into(), seed: seed() }
    }

    fn query(line: &str, name: &str) -> String {
        let url = url::Url::parse(&format!("http://x{}", line.split(' ').nth(1).unwrap())).unwrap();
        url.query_pairs().find(|(k, _)| k == name).unwrap().1.into_owned()
    }

    /// A server whose queue starts at 5 tracks and grows by 5 per centred read,
    /// up to `cap`.
    async fn server(cap: usize) -> (String, Seen, Seen) {
        serve_full(move |line| {
            if line.starts_with("POST ") {
                return (200, queue_xml(5));
            }
            let center: usize = query(line, "center").parse().unwrap();
            (200, queue_xml((center + 5).min(cap)))
        })
        .await
    }

    #[tokio::test]
    async fn reads_the_play_queue_until_it_holds_thirty_tracks() {
        let (base, seen, full) = server(100).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let key = "/library/metadata/7/station/u-1?type=10";
        let (tracks, queue) = client.radio_tracks("cid1", "mid1", key, seed(), 30).await.unwrap();
        assert_eq!(queue, RadioQueue { id: "9".into(), last: "30".into(), seed: seed() });
        let keys: Vec<_> = tracks.iter().map(|t| t.rating_key.clone()).collect();
        let expected: Vec<_> = (1..=30).map(|i| i.to_string()).collect();
        assert_eq!(keys, expected);
        let lines = seen.lock().unwrap().clone();
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert!(lines[0].starts_with("POST /playQueues?"), "{}", lines[0]);
        assert_eq!(
            query(&lines[0], "uri"),
            format!("server://mid1/com.plexapp.plugins.library{key}")
        );
        for req in full.lock().unwrap().iter() {
            assert!(req.to_lowercase().contains("x-plex-client-identifier: cid1"), "{req}");
        }
    }

    #[tokio::test]
    async fn a_read_that_adds_nothing_ends_the_loop() {
        let (base, seen, _) = server(10).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let (tracks, queue) =
            client.radio_tracks("cid1", "mid1", "/s?type=10", seed(), 30).await.unwrap();
        assert_eq!(queue.last, "10");
        assert_eq!(tracks.len(), 10);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_station_with_more_than_wanted_keeps_only_wanted_and_its_last_item() {
        let (base, _, _) = server(100).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let (tracks, queue) =
            client.radio_tracks("cid1", "mid1", "/s?type=10", seed(), 12).await.unwrap();
        assert_eq!(tracks.len(), 12);
        assert_eq!(queue.last, "12");
    }

    #[tokio::test]
    async fn reading_on_returns_only_items_after_the_last_and_stops_at_want() {
        let (base, seen, full) = server(100).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let queue = RadioQueue { id: "9".into(), last: "30".into(), seed: seed() };
        let ReadOn::Tracks(tracks, last) =
            client.radio_read_on("cid1", &queue, 8, &HashSet::new()).await
        else {
            panic!("expected tracks");
        };
        let keys: Vec<_> = tracks.iter().map(|t| t.rating_key.clone()).collect();
        let expected: Vec<_> = (31..=38).map(|i| i.to_string()).collect();
        assert_eq!(keys, expected);
        assert_eq!(last, "38");
        let lines = seen.lock().unwrap().clone();
        assert!(lines[0].starts_with("GET /playQueues/9?"), "{}", lines[0]);
        assert_eq!(query(&lines[0], "center"), "30");
        for req in full.lock().unwrap().iter() {
            assert!(req.to_lowercase().contains("x-plex-client-identifier: cid1"), "{req}");
        }
    }

    #[tokio::test]
    async fn reading_on_skips_tracks_in_skip_but_reads_past_them() {
        let (base, _, _) = server(100).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let skip: HashSet<String> = (31..=40).map(|i| i.to_string()).collect();
        let ReadOn::Tracks(tracks, last) =
            client.radio_read_on("cid1", &queue("30"), 3, &skip).await
        else {
            panic!("expected tracks");
        };
        let keys: Vec<_> = tracks.iter().map(|t| t.rating_key.clone()).collect();
        assert_eq!(keys, ["41", "42", "43"]);
        assert_eq!(last, "43");

        let (base, _, _) = server(35).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        let ReadOn::Tracks(tracks, last) =
            client.radio_read_on("cid1", &queue("30"), 3, &skip).await
        else {
            panic!("expected an empty read that moves on");
        };
        assert!(tracks.is_empty());
        assert_eq!(last, "35");
    }

    #[tokio::test]
    async fn reading_on_from_the_end_of_the_station_is_exhausted() {
        let (base, _, _) = server(10).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        assert!(matches!(
            client.radio_read_on("cid1", &queue("10"), 20, &HashSet::new()).await,
            ReadOn::Exhausted
        ));
    }

    #[tokio::test]
    async fn reading_on_a_dropped_play_queue_is_gone_and_a_lost_item_is_a_failure() {
        let (base, _, _) = serve_full(|_| (404, String::new())).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        assert!(matches!(
            client.radio_read_on("cid1", &queue("10"), 20, &HashSet::new()).await,
            ReadOn::Gone
        ));
        let (base, _, _) = server(10).await;
        let client = PlexClient::new(&base, "faketoken-radio");
        assert!(matches!(
            client.radio_read_on("cid1", &queue("99"), 20, &HashSet::new()).await,
            ReadOn::Failed(_)
        ));
    }

    #[test]
    fn uuid_is_a_version_4_uuid() {
        let u = uuid_v4().unwrap();
        let parts: Vec<_> = u.split('-').map(str::len).collect();
        assert_eq!(parts, [8, 4, 4, 4, 12]);
        assert_eq!(u.as_bytes()[14], b'4');
        assert!(matches!(u.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "{u}");
        assert!(u.bytes().all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }
}
