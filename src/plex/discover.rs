//! Discover: the music hubs of a library section.

use super::models::{Album, Artist, MediaContainer, Playlist, Track, WireDirectory, WirePlaylist};
use super::radio::{station_of, Station};
use super::PlexClient;
use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SKIPPED: [&str; 3] = ["music.recent.played", "music.touring", "music.videos"];

pub const LIBRARY_PAGE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryList {
    RecentlyPlayed,
    TopRated,
    NeverPlayed,
}

impl LibraryList {
    pub const ALL: [LibraryList; 3] = [Self::RecentlyPlayed, Self::TopRated, Self::NeverPlayed];

    pub fn title(self) -> &'static str {
        match self {
            Self::RecentlyPlayed => "Recently played",
            Self::TopRated => "Top rated",
            Self::NeverPlayed => "Never played",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    fn query(self) -> &'static str {
        match self {
            Self::RecentlyPlayed => "type=10&lastViewedAt>>=1&sort=lastViewedAt:desc",
            Self::TopRated => "type=10&userRating>=8&sort=lastRatedAt:desc",
            Self::NeverPlayed => "type=9&lastViewedAt<<=1&sort=random",
        }
    }
}

/// One page of a track list; `rows` is how many rows the server sent.
#[derive(Debug)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub rows: usize,
    pub total: Option<usize>,
}

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

impl PlexClient {
    pub async fn library_tracks(
        &self,
        section: &str,
        list: LibraryList,
        start: usize,
    ) -> Result<TrackPage> {
        let c = self.library_page(section, list, start).await?;
        let total = c.total_size;
        let tracks = c.into_tracks();
        Ok(TrackPage { rows: tracks.len(), tracks, total })
    }

    /// Random per request, so one page and never more.
    pub async fn never_played(&self, section: &str) -> Result<Vec<Album>> {
        let c = self.library_page(section, LibraryList::NeverPlayed, 0).await?;
        Ok(c.directories.into_iter().map(WireDirectory::into_album).collect())
    }

    async fn library_page(
        &self,
        section: &str,
        list: LibraryList,
        start: usize,
    ) -> Result<MediaContainer> {
        let query = list.query();
        self.get_xml(&format!(
            "/library/sections/{section}/all?{query}&X-Plex-Container-Start={start}&X-Plex-Container-Size={LIBRARY_PAGE}"
        ))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;

    const TRACKS: &str = "<MediaContainer totalSize=\"9299\"><Track ratingKey=\"1\" key=\"/library/metadata/1\" title=\"T\" lastViewedAt=\"1763800000\"/></MediaContainer>";

    async fn requested(list: LibraryList, start: usize) -> (String, Vec<String>) {
        let (base, seen, _) = serve_full(|_| (200, TRACKS.to_string())).await;
        let client = PlexClient::new(&base, "faketoken-lists");
        match list {
            LibraryList::NeverPlayed => {
                client.never_played("1").await.unwrap();
            }
            _ => {
                let page = client.library_tracks("1", list, start).await.unwrap();
                assert_eq!((page.total, page.rows), (Some(9299), 1));
                assert_eq!(page.tracks[0].last_viewed_at, Some(1_763_800_000));
            }
        }
        let lines = seen.lock().unwrap().clone();
        (lines[0].clone(), lines)
    }

    #[tokio::test]
    async fn each_list_asks_with_its_own_filter_sort_and_page() {
        let size = "X-Plex-Container-Size=100";
        for (list, start, want) in [
            (
                LibraryList::RecentlyPlayed,
                200,
                ["type=10", "lastViewedAt%3E%3E=1", "sort=lastViewedAt:desc", "Start=200"],
            ),
            (
                LibraryList::TopRated,
                0,
                ["type=10", "userRating%3E=8", "sort=lastRatedAt:desc", "Start=0"],
            ),
            (
                LibraryList::NeverPlayed,
                0,
                ["type=9", "lastViewedAt%3C%3C=1", "sort=random", "Start=0"],
            ),
        ] {
            let (line, _) = requested(list, start).await;
            assert!(line.starts_with("GET /library/sections/1/all?"), "{line}");
            assert!(line.contains(size), "{line}");
            for part in want {
                assert!(line.contains(part), "{part} in {line}");
            }
        }
    }
}
