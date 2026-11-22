//! Discover: the music hubs of a library section.

use super::models::{Album, Artist, MediaContainer, Playlist, WireDirectory, WirePlaylist};
use super::radio::{station_of, Station};
use super::PlexClient;
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SKIPPED: [&str; 3] = ["music.recent.played", "music.touring", "music.videos"];

#[derive(Debug, Clone)]
pub struct Shelf {
    pub title: String,
    pub items: ShelfItems,
}

#[derive(Debug, Clone)]
pub enum ShelfItems {
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
    Playlists(Vec<Playlist>),
}

impl ShelfItems {
    pub fn len(&self) -> usize {
        match self {
            Self::Albums(v) => v.len(),
            Self::Artists(v) => v.len(),
            Self::Playlists(v) => v.len(),
        }
    }

    pub fn title(&self, i: usize) -> Option<&str> {
        match self {
            Self::Albums(v) => v.get(i).map(|a| a.title.as_str()),
            Self::Artists(v) => v.get(i).map(|a| a.title.as_str()),
            Self::Playlists(v) => v.get(i).map(|p| p.title.as_str()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiscoverHubs {
    pub shelves: Vec<Shelf>,
    pub stations: Vec<Station>,
}

fn shelf_items(type_: &str, dirs: Vec<WireDirectory>, lists: Vec<WirePlaylist>) -> ShelfItems {
    match type_ {
        "album" => ShelfItems::Albums(dirs.into_iter().map(WireDirectory::into_album).collect()),
        "artist" => ShelfItems::Artists(dirs.into_iter().map(WireDirectory::into_artist).collect()),
        _ => ShelfItems::Playlists(
            lists.into_iter().filter_map(WirePlaylist::into_playlist).collect(),
        ),
    }
}

pub fn hubs_from(c: MediaContainer) -> DiscoverHubs {
    let mut out = DiscoverHubs::default();
    for hub in c.hubs {
        let identifier = hub.identifier.unwrap_or_default();
        if SKIPPED.iter().any(|s| identifier.starts_with(s)) {
            continue;
        }
        match hub.type_.as_deref() {
            Some("station") => {
                out.stations.extend(hub.playlists.into_iter().filter_map(station_of));
            }
            Some(kind @ ("album" | "artist" | "playlist")) => {
                let items = shelf_items(kind, hub.directories, hub.playlists);
                if !items.is_empty() {
                    out.shelves.push(Shelf { title: hub.title.unwrap_or_default(), items });
                }
            }
            _ => {}
        }
    }
    out
}

/// Plex caches hub replies per URL, so every request needs a value it has not seen.
fn cache_buster() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("{now}-{}", COUNT.fetch_add(1, Ordering::Relaxed))
}

impl PlexClient {
    /// `includeLibraryPlaylists` adds the Recent Playlists hub.
    pub async fn discover_hubs(&self, section: &str) -> Result<DiscoverHubs> {
        let c = self
            .get_xml(&format!(
                "/hubs/sections/{section}?includeStations=1&includeLibraryPlaylists=1&count=50&shmcRefresh={}",
                cache_buster()
            ))
            .await?;
        Ok(hubs_from(c))
    }
}
