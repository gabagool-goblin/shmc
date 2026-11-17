//! Plex Pass sonic analysis: the nearest tracks to a seed, and which Plex Pass
//! features the server has.

use super::client::PlexClient;
use super::models::{MediaContainer, Track};
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
