//! Plex Pass sonic analysis and the server's Plex Pass features.

use super::client::{NotFound, PlexClient};
use super::models::{MediaContainer, SearchHit, Track};
use anyhow::{Context, Result};

/// Plex Pass features shmc uses, from the server root's `ownerFeatures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
    pub sonic_analysis: bool,
    pub radio: bool,
    pub loudness: bool,
}

impl Features {
    pub fn from_container(c: &MediaContainer) -> Self {
        let listed = c.owner_features.as_deref().unwrap_or_default();
        let has = |name: &str| listed.split(',').any(|f| f.trim() == name);
        Self {
            sonic_analysis: has("music-analysis"),
            radio: has("radio"),
            loudness: has("loudness"),
        }
    }
}

/// `ceil(100 × (1 − distance))`, clamped to 0..=100.
pub fn match_percent(distance: f64) -> u8 {
    // Float error must not lift an exact percentage (0.05 is 95) to the next one.
    ((1.0 - distance) * 100.0 - 1e-9).ceil().clamp(0.0, 100.0) as u8
}

/// Tracks in Plex's order (closest first), each with its match when it has a distance.
pub fn similar_from(c: MediaContainer) -> Vec<(Track, Option<u8>)> {
    c.tracks
        .into_iter()
        .map(|t| {
            let percent = t.distance.filter(|d| d.is_finite()).map(match_percent);
            (t.into_track(), percent)
        })
        .collect()
}

/// Albums and artists in Plex's order, each with its match when it has a distance.
pub fn similar_hits_from(c: MediaContainer) -> Vec<(SearchHit, Option<u8>)> {
    c.directories
        .into_iter()
        .filter_map(|d| {
            let percent = d.distance.filter(|x| x.is_finite()).map(match_percent);
            let hit = match d.type_.as_deref() {
                Some("album") => d.into_album_hit(),
                Some("artist") => d.into_artist_hit(),
                _ => return None,
            };
            Some((hit, percent))
        })
        .collect()
}

impl PlexClient {
    /// Plex leaves the seed out of its own list. `exclude` is an artist key
    /// whose tracks are left out too.
    pub async fn similar(
        &self,
        key: &str,
        exclude: Option<&str>,
    ) -> Result<Vec<(Track, Option<u8>)>> {
        let mut path = format!("/library/metadata/{key}/nearest");
        if let Some(artist) = exclude {
            path.push_str(&format!("?excludeGrandparentID={artist}"));
        }
        Ok(similar_from(self.get_xml(&path).await?))
    }

    /// The albums or artists nearest to an album or artist, seed left out.
    pub async fn similar_hits(&self, key: &str) -> Result<Vec<(SearchHit, Option<u8>)>> {
        let c = self.get_xml(&format!("/library/metadata/{key}/nearest")).await?;
        Ok(similar_hits_from(c))
    }

    /// The tracks from `start` to `end`, both included; empty when Plex finds no route.
    pub async fn compute_path(&self, section: &str, start: &str, end: &str) -> Result<Vec<Track>> {
        let path = format!("/library/sections/{section}/computePath?startID={start}&endID={end}");
        match self.get_xml(&path).await {
            Ok(c) => Ok(c.into_tracks()),
            Err(e) if e.is::<NotFound>() => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    pub async fn features(&self) -> Result<Features> {
        Ok(Features::from_container(&self.get_xml("/").await?))
    }

    /// Whether Plex has analysed the track, from its own metadata: bulk listings
    /// may not carry `musicAnalysisVersion`.
    pub async fn is_analysed(&self, key: &str) -> Result<bool> {
        let c = self.get_xml(&format!("/library/metadata/{key}")).await?;
        let t = c.tracks.into_iter().next().context("no track found")?;
        Ok(t.music_analysis_version.is_some_and(|v| !v.trim().is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;

    fn track_xml(key: &str) -> String {
        format!(
            "<Track ratingKey=\"{key}\" key=\"/library/metadata/{key}\" title=\"T{key}\"><Media><Part id=\"{key}\" key=\"/library/parts/{key}/f.mp3\"/></Media></Track>"
        )
    }

    #[tokio::test]
    async fn a_path_lists_the_tracks_and_a_missing_id_or_empty_route_lists_none() {
        let (base, seen, _) = serve_full(|line| {
            if line.contains("endID=9") {
                (404, String::new())
            } else if line.contains("endID=2") {
                (200, "<MediaContainer size=\"0\"/>".into())
            } else {
                (
                    200,
                    format!(
                        "<MediaContainer>{}{}</MediaContainer>",
                        track_xml("1"),
                        track_xml("5")
                    ),
                )
            }
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-path");
        let keys: Vec<_> = client
            .compute_path("3", "1", "5")
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.rating_key)
            .collect();
        assert_eq!(keys, ["1", "5"]);
        assert!(seen.lock().unwrap()[0]
            .starts_with("GET /library/sections/3/computePath?startID=1&endID=5 "));
        assert!(client.compute_path("3", "1", "9").await.unwrap().is_empty());
        assert!(client.compute_path("3", "1", "2").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_server_error_is_an_error_not_an_empty_path() {
        let (base, _, _) = serve_full(|_| (500, String::new())).await;
        let client = PlexClient::new(&base, "faketoken-path");
        assert!(client.compute_path("3", "1", "5").await.is_err());
    }
}
