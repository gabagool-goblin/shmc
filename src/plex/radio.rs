//! Plex radio: an artist's station, a station for a track, and the play queue
//! the server builds from one.

use super::client::{read_capped, redirect_error, PlexClient, MAX_XML_BYTES};
use super::models::{MediaContainer, Track};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::io::Read;

/// The most reads of a play queue after the one that creates it.
const MAX_READS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Station {
    pub key: String,
    pub title: String,
}

/// The first station an artist lists.
pub fn station_from(c: MediaContainer) -> Option<Station> {
    let station =
        c.directories.into_iter().find_map(|d| d.stations?.stations.into_iter().next())?;
    Some(Station { key: station.key, title: station.title.unwrap_or_default() })
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

    /// Creates a play queue from the station and reads it until it holds `want`
    /// tracks, stops growing or has been read `MAX_READS` times. The create is
    /// never retried; a failed read keeps what was read before it.
    pub async fn radio_tracks(
        &self,
        client_id: &str,
        machine_id: &str,
        station_key: &str,
        want: usize,
    ) -> Result<Vec<Track>> {
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
        Ok(items.into_iter().map(|(t, _)| t).collect())
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
        let tracks = client.radio_tracks("cid1", "mid1", key, 30).await.unwrap();
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
        let tracks = client.radio_tracks("cid1", "mid1", "/s?type=10", 30).await.unwrap();
        assert_eq!(tracks.len(), 10);
        assert_eq!(seen.lock().unwrap().len(), 3);
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
