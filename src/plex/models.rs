//! Plex XML wire models (quick-xml serde) and domain models derived from them.

use serde::{Deserialize, Deserializer};

/// A numeric attribute that is `None` when it does not parse (`duration="-1"`,
/// `year=""`) instead of failing the whole listing over one odd item.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
{
    Ok(Option::<String>::deserialize(d)?.and_then(|s| s.trim().parse().ok()))
}

// Wire models: the Plex XML as served (see tests/fixtures)

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaContainer {
    /// Size of the whole result, reported beside a page.
    #[serde(rename = "@totalSize", default, deserialize_with = "lenient")]
    pub total_size: Option<usize>,
    #[serde(rename = "Directory", default)]
    pub directories: Vec<WireDirectory>,
    #[serde(rename = "Track", default)]
    pub tracks: Vec<WireTrack>,
    #[serde(rename = "Playlist", default)]
    pub playlists: Vec<WirePlaylist>,
    #[serde(rename = "Hub", default)]
    pub hubs: Vec<WireHub>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireDirectory {
    /// Absent on library sections, which use `key`.
    #[serde(rename = "@ratingKey", default)]
    pub rating_key: Option<String>,
    #[serde(rename = "@key")]
    pub key: String,
    #[serde(rename = "@type", default)]
    pub type_: Option<String>,
    #[serde(rename = "@title", default)]
    pub title: Option<String>,
    #[serde(rename = "@parentTitle", default)]
    pub parent_title: Option<String>,
    #[serde(rename = "@grandparentTitle", default)]
    pub grandparent_title: Option<String>,
    #[serde(rename = "@thumb", default)]
    pub thumb: Option<String>,
    #[serde(rename = "@parentRatingKey", default)]
    pub parent_rating_key: Option<String>,
    #[serde(rename = "@year", default, deserialize_with = "lenient")]
    pub year: Option<u32>,
    #[serde(rename = "@duration", default, deserialize_with = "lenient")]
    pub duration: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireTrack {
    #[serde(rename = "@ratingKey")]
    pub rating_key: String,
    #[serde(rename = "@key")]
    pub key: String,
    #[serde(rename = "@parentRatingKey", default)]
    pub parent_rating_key: Option<String>,
    #[serde(rename = "@grandparentRatingKey", default)]
    pub grandparent_rating_key: Option<String>,
    #[serde(rename = "@title", default)]
    pub title: Option<String>,
    #[serde(rename = "@parentTitle", default)]
    pub parent_title: Option<String>,
    #[serde(rename = "@grandparentTitle", default)]
    pub grandparent_title: Option<String>,
    /// Track artist credit; `grandparentTitle` is the album artist.
    #[serde(rename = "@originalTitle", default)]
    pub original_title: Option<String>,
    #[serde(rename = "@index", default, deserialize_with = "lenient")]
    pub index: Option<u32>,
    /// Disc number.
    #[serde(rename = "@parentIndex", default, deserialize_with = "lenient")]
    pub parent_index: Option<u32>,
    #[serde(rename = "@duration", default, deserialize_with = "lenient")]
    pub duration: Option<u64>,
    #[serde(rename = "@parentYear", default, deserialize_with = "lenient")]
    pub parent_year: Option<u32>,
    #[serde(rename = "@thumb", default)]
    pub thumb: Option<String>,
    #[serde(rename = "@parentThumb", default)]
    pub parent_thumb: Option<String>,
    #[serde(rename = "@grandparentThumb", default)]
    pub grandparent_thumb: Option<String>,
    /// Playlist entries only; unlike `ratingKey` it is unique per entry.
    #[serde(rename = "@playlistItemID", default)]
    pub playlist_item_id: Option<String>,
    /// A float such as "6.0"; absent when unrated.
    #[serde(rename = "@userRating", default, deserialize_with = "lenient")]
    pub user_rating: Option<f64>,
    #[serde(rename = "Media", default)]
    pub media: Vec<WireMedia>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WirePlaylist {
    #[serde(rename = "@ratingKey")]
    pub rating_key: String,
    #[serde(rename = "@title", default)]
    pub title: Option<String>,
    /// "1" for smart playlists.
    #[serde(rename = "@smart", default)]
    pub smart: Option<String>,
    /// "audio", "video" or "photo".
    #[serde(rename = "@playlistType", default)]
    pub playlist_type: Option<String>,
    #[serde(rename = "@leafCount", default, deserialize_with = "lenient")]
    pub leaf_count: Option<u32>,
    #[serde(rename = "@duration", default, deserialize_with = "lenient")]
    pub duration: Option<u64>,
    #[serde(rename = "@thumb", default)]
    pub thumb: Option<String>,
    /// Present even when `thumb` is absent (smart lists).
    #[serde(rename = "@composite", default)]
    pub composite: Option<String>,
    #[serde(rename = "@summary", default)]
    pub summary: Option<String>,
}

/// A `<Hub>` of `/hubs/search` results, grouped by `type`; kinds other than
/// track, album and artist are ignored.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WireHub {
    #[serde(rename = "@type", default)]
    pub type_: Option<String>,
    #[serde(rename = "@title", default)]
    pub title: Option<String>,
    #[serde(rename = "Directory", default)]
    pub directories: Vec<WireDirectory>,
    #[serde(rename = "Track", default)]
    pub tracks: Vec<WireTrack>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireMedia {
    #[serde(rename = "@container", default)]
    pub container: Option<String>,
    #[serde(rename = "@bitrate", default, deserialize_with = "lenient")]
    pub bitrate: Option<u32>,
    #[serde(rename = "@audioCodec", default)]
    pub audio_codec: Option<String>,
    #[serde(rename = "@audioChannels", default, deserialize_with = "lenient")]
    pub audio_channels: Option<u32>,
    #[serde(rename = "Part", default)]
    pub parts: Vec<WirePart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WirePart {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "@key")]
    pub key: String,
    #[serde(rename = "@duration", default, deserialize_with = "lenient")]
    pub duration: Option<u64>,
    #[serde(rename = "@container", default)]
    pub container: Option<String>,
    #[serde(rename = "Stream", default)]
    pub streams: Vec<WireStream>,
}

/// A `<Stream>`; the audio one (`streamType="2"`) has the sample rate and bit depth.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WireStream {
    #[serde(rename = "@streamType", default)]
    pub stream_type: Option<String>,
    #[serde(rename = "@codec", default)]
    pub codec: Option<String>,
    #[serde(rename = "@bitrate", default, deserialize_with = "lenient")]
    pub bitrate: Option<u32>,
    #[serde(rename = "@samplingRate", default, deserialize_with = "lenient")]
    pub sampling_rate: Option<u32>,
    #[serde(rename = "@bitDepth", default, deserialize_with = "lenient")]
    pub bit_depth: Option<u32>,
    #[serde(rename = "@channels", default, deserialize_with = "lenient")]
    pub channels: Option<u32>,
    #[serde(rename = "@selected", default)]
    pub selected: Option<String>,
}

// Domain models

#[derive(Debug, Clone)]
pub struct Section {
    pub key: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub rating_key: String,
    pub title: String,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub rating_key: String,
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub thumb: Option<String>,
}

/// Bulk XML has codec, bitrate and channels from `<Media>`; the sample rate and
/// bit depth need the `<Stream>` of the full track metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioInfo {
    pub codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub bit_depth: Option<u32>,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub rating_key: String,
    pub title: String,
    /// `originalTitle`, falling back to the album artist.
    pub artist: String,
    pub album: String,
    pub track_no: Option<u32>,
    /// The album's year.
    pub year: Option<u32>,
    pub duration_ms: Option<u64>,
    pub parent_rating_key: Option<String>,
    pub grandparent_rating_key: Option<String>,
    /// The album's thumb.
    pub thumb: Option<String>,
    /// Server-relative, e.g. "/library/parts/18181/…/file.mp3".
    pub part_key: Option<String>,
    pub container: Option<String>,
    pub audio: AudioInfo,
    pub playlist_item_id: Option<String>,
    /// Plex's 0 to 10, a star is 2; None when unrated.
    pub user_rating: Option<u8>,
}

impl Track {
    /// The stream URL for mpv, token included. Built with `Url`: concatenating
    /// `@evil.example/x` would send the token to another host.
    pub fn stream_url(&self, base: &str, token: &str) -> Option<String> {
        let key = self.part_key.as_ref()?;
        let mut url = url::Url::parse(base).ok()?;
        let base_path = url.path().trim_end_matches('/').to_string();
        url.set_path(&format!("{base_path}/{}", key.trim_start_matches('/')));
        url.query_pairs_mut().append_pair("X-Plex-Token", token);
        Some(url.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub rating_key: String,
    pub title: String,
    pub smart: bool,
    pub item_count: Option<u32>,
    pub duration_ms: Option<u64>,
    pub thumb: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Artist,
    Album,
    Track,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: HitKind,
    pub rating_key: String,
    /// Track title or album name; empty for artist hits (the name is `artist`).
    pub title: String,
    pub artist: String,
    /// Album name for tracks, year for albums.
    pub subtitle: String,
    pub duration_ms: Option<u64>,
    /// Tracks only; an album hit's own key is its album.
    pub album_key: Option<String>,
    pub artist_key: Option<String>,
}

// Wire to domain

impl WireDirectory {
    pub fn into_artist(self) -> Artist {
        Artist {
            rating_key: self.rating_key.unwrap_or(self.key),
            title: self.title.unwrap_or_default(),
            thumb: self.thumb,
        }
    }

    pub fn into_album(self) -> Album {
        Album {
            rating_key: self.rating_key.unwrap_or(self.key),
            title: self.title.unwrap_or_default(),
            artist: self.grandparent_title.or(self.parent_title).unwrap_or_default(),
            year: self.year,
            thumb: self.thumb,
        }
    }
}

impl WirePlaylist {
    pub fn into_playlist(self) -> Playlist {
        Playlist {
            rating_key: self.rating_key,
            title: self.title.unwrap_or_default(),
            smart: self.smart.as_deref() == Some("1"),
            item_count: self.leaf_count,
            duration_ms: self.duration,
            thumb: self.thumb.or(self.composite),
            summary: self.summary,
        }
    }
}

impl WireTrack {
    pub fn into_track(self) -> Track {
        let media = self.media.first();
        let part = media.and_then(|m| m.parts.first());
        let audio_stream = part.and_then(|p| {
            p.streams
                .iter()
                .find(|s| {
                    s.stream_type.as_deref() == Some("2") && s.selected.as_deref() == Some("1")
                })
                .or_else(|| p.streams.iter().find(|s| s.stream_type.as_deref() == Some("2")))
        });
        let audio = AudioInfo {
            codec: audio_stream
                .and_then(|s| s.codec.clone())
                .or_else(|| media.and_then(|m| m.audio_codec.clone())),
            bitrate_kbps: audio_stream
                .and_then(|s| s.bitrate)
                .or_else(|| media.and_then(|m| m.bitrate)),
            sample_rate_hz: audio_stream.and_then(|s| s.sampling_rate),
            bit_depth: audio_stream.and_then(|s| s.bit_depth),
            channels: audio_stream
                .and_then(|s| s.channels)
                .or_else(|| media.and_then(|m| m.audio_channels)),
        };
        let artist = self
            .original_title
            .filter(|s| !s.trim().is_empty())
            .or(self.grandparent_title)
            .unwrap_or_default();
        Track {
            rating_key: self.rating_key,
            title: self.title.unwrap_or_default(),
            artist,
            album: self.parent_title.unwrap_or_default(),
            track_no: self.index,
            year: self.parent_year,
            duration_ms: self.duration,
            parent_rating_key: self.parent_rating_key,
            grandparent_rating_key: self.grandparent_rating_key,
            thumb: self.thumb.or(self.parent_thumb).or(self.grandparent_thumb),
            part_key: part.map(|p| p.key.clone()),
            container: part
                .and_then(|p| p.container.clone())
                .or_else(|| media.and_then(|m| m.container.clone())),
            audio,
            playlist_item_id: self.playlist_item_id,
            user_rating: self
                .user_rating
                .map(|r| r.round().clamp(0.0, 10.0) as u8)
                .filter(|&r| r > 0),
        }
    }
}

impl MediaContainer {
    pub fn into_tracks(self) -> Vec<Track> {
        self.tracks.into_iter().map(|t| t.into_track()).collect()
    }

    /// Search hits: tracks, then albums, then artists. Flat (non-hub) results
    /// go in the same buckets, so the plain `/search` shape parses too.
    pub fn into_search_hits(self) -> Vec<SearchHit> {
        let mut tracks = Vec::new();
        let mut albums = Vec::new();
        let mut artists = Vec::new();
        for hub in self.hubs {
            let WireHub { type_, tracks: hub_tracks, directories: hub_dirs, .. } = hub;
            match type_.as_deref() {
                Some("track") => tracks.extend(hub_tracks),
                Some("album") => albums.extend(hub_dirs),
                Some("artist") => artists.extend(hub_dirs),
                _ => {}
            }
        }
        tracks.extend(self.tracks);
        for d in self.directories {
            match d.type_.as_deref() {
                Some("album") => albums.push(d),
                Some("artist") => artists.push(d),
                _ => {}
            }
        }

        let mut hits = Vec::new();
        for t in tracks {
            let t = t.into_track();
            hits.push(SearchHit {
                kind: HitKind::Track,
                rating_key: t.rating_key,
                title: t.title,
                artist: t.artist,
                subtitle: t.album,
                duration_ms: t.duration_ms,
                album_key: t.parent_rating_key,
                artist_key: t.grandparent_rating_key,
            });
        }
        for d in albums {
            hits.push(SearchHit {
                kind: HitKind::Album,
                rating_key: d.rating_key.unwrap_or(d.key),
                title: d.title.unwrap_or_default(),
                artist: d.grandparent_title.or(d.parent_title).unwrap_or_default(),
                subtitle: d.year.map(|y| y.to_string()).unwrap_or_default(),
                duration_ms: d.duration,
                album_key: None,
                artist_key: d.parent_rating_key,
            });
        }
        for d in artists {
            hits.push(SearchHit {
                kind: HitKind::Artist,
                rating_key: d.rating_key.unwrap_or(d.key),
                title: String::new(),
                artist: d.title.unwrap_or_default(),
                subtitle: String::new(),
                duration_ms: None,
                album_key: None,
                artist_key: None,
            });
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_with_key(key: &str) -> Track {
        Track {
            rating_key: "1".into(),
            title: "t".into(),
            artist: String::new(),
            album: String::new(),
            track_no: None,
            year: None,
            duration_ms: None,
            parent_rating_key: None,
            grandparent_rating_key: None,
            thumb: None,
            part_key: Some(key.into()),
            container: None,
            audio: AudioInfo::default(),
            playlist_item_id: None,
            user_rating: None,
        }
    }

    #[test]
    fn plain_key_appended_to_root() {
        let t = track_with_key("/library/parts/1/file.flac");
        let url = t.stream_url("http://plex:32400", "tok").unwrap();
        assert_eq!(url, "http://plex:32400/library/parts/1/file.flac?X-Plex-Token=tok");
    }

    #[test]
    fn proxied_base_path_survives() {
        let t = track_with_key("/library/parts/1/file.flac");
        let url = t.stream_url("http://host/plex", "tok").unwrap();
        assert_eq!(url, "http://host/plex/library/parts/1/file.flac?X-Plex-Token=tok");
    }

    /// Concatenating `@evil.example/x` made userinfo of the base.
    #[test]
    fn hostile_part_key_cannot_change_host() {
        let t = track_with_key("@evil.example/x");
        let url = t.stream_url("http://plex:32400", "tok").unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("plex"), "{url}");
        assert_eq!(parsed.port(), Some(32400), "{url}");
        let token =
            parsed.query_pairs().find(|(k, _)| k == "X-Plex-Token").map(|(_, v)| v.into_owned());
        assert_eq!(token.as_deref(), Some("tok"), "{url}");
    }
}
