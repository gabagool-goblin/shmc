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
    TopTracks,
    TopAlbums,
    TopArtists,
}

impl LibraryList {
    pub const LIBRARY: [LibraryList; 3] = [Self::RecentlyPlayed, Self::TopRated, Self::NeverPlayed];
    pub const TOP: [LibraryList; 3] = [Self::TopTracks, Self::TopAlbums, Self::TopArtists];

    pub fn title(self) -> &'static str {
        match self {
            Self::RecentlyPlayed => "Recently played",
            Self::TopRated => "Top rated",
            Self::NeverPlayed => "Never played",
            Self::TopTracks => "Top tracks",
            Self::TopAlbums => "Top albums",
            Self::TopArtists => "Top artists",
        }
    }

    pub fn is_top(self) -> bool {
        Self::TOP.contains(&self)
    }

    pub fn paged(self) -> bool {
        matches!(self, Self::RecentlyPlayed | Self::TopRated)
    }

    fn plex_type(self) -> u8 {
        match self {
            Self::NeverPlayed | Self::TopAlbums => 9,
            Self::TopArtists => 8,
            _ => 10,
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
            Self::TopTracks => "type=10&viewCount>=1&sort=viewCount:desc",
            Self::TopAlbums => "type=9&viewCount>=1&sort=viewCount:desc",
            Self::TopArtists => "type=8&viewCount>=1&sort=viewCount:desc",
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

#[derive(Debug)]
pub enum TopRows {
    Tracks(TrackPage),
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
}

/// `Decade` holds the decade's first year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestOf {
    Year(u32),
    Decade(u32),
}

impl BestOf {
    pub fn label(self) -> String {
        match self {
            Self::Year(y) => y.to_string(),
            Self::Decade(d) => format!("{d}s"),
        }
    }

    fn filter(self) -> String {
        match self {
            Self::Year(y) => format!("year={y}"),
            Self::Decade(d) => format!("decade={d}"),
        }
    }
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
    BestOf(Vec<BestOf>),
}

impl ShelfItems {
    pub fn len(&self) -> usize {
        match self {
            Self::Albums(v) => v.len(),
            Self::Artists(v) => v.len(),
            Self::Playlists(v) => v.len(),
            Self::BestOf(v) => v.len(),
        }
    }

    pub fn title(&self, i: usize) -> Option<&str> {
        match self {
            Self::Albums(v) => v.get(i).map(|a| a.title.as_str()),
            Self::Artists(v) => v.get(i).map(|a| a.title.as_str()),
            Self::Playlists(v) => v.get(i).map(|p| p.title.as_str()),
            Self::BestOf(_) => None,
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
        Ok(albums_of(c))
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

impl PlexClient {
    /// `span` is inclusive unix seconds; `None` is all time.
    pub async fn top_list(
        &self,
        section: &str,
        list: LibraryList,
        span: Option<(i64, i64)>,
    ) -> Result<TopRows> {
        let c = match span {
            Some((since, until)) => {
                let type_ = list.plex_type();
                self.get_xml(&format!(
                    "/library/all/top?type={type_}&librarySectionID={section}&viewedAt>={since}&viewedAt<={until}&accountID=1&X-Plex-Container-Start=0&X-Plex-Container-Size={LIBRARY_PAGE}"
                ))
                .await?
            }
            None => self.library_page(section, list, 0).await?,
        };
        Ok(match list {
            LibraryList::TopAlbums => TopRows::Albums(albums_of(c)),
            LibraryList::TopArtists => TopRows::Artists(
                c.directories.into_iter().map(WireDirectory::into_artist).collect(),
            ),
            _ => {
                let total = c.total_size;
                let tracks = c.into_tracks();
                TopRows::Tracks(TrackPage { rows: tracks.len(), tracks, total })
            }
        })
    }

    /// The years and the decades that have albums, newest first.
    pub async fn best_of_choices(&self, section: &str) -> Result<(Vec<u32>, Vec<u32>)> {
        Ok((self.best_of_keys(section, "year").await?, self.best_of_keys(section, "decade").await?))
    }

    async fn best_of_keys(&self, section: &str, kind: &str) -> Result<Vec<u32>> {
        let c = self
            .get_xml(&format!(
                "/library/sections/{section}/{kind}?type=9&X-Plex-Container-Start=0&X-Plex-Container-Size={LIBRARY_PAGE}"
            ))
            .await?;
        Ok(c.directories.iter().filter_map(|d| d.key.parse().ok()).collect())
    }

    pub async fn best_of_albums(&self, section: &str, what: BestOf) -> Result<Vec<Album>> {
        let filter = what.filter();
        let c = self
            .get_xml(&format!(
                "/library/sections/{section}/all?type=9&{filter}&viewCount>=1&sort=viewCount:desc&X-Plex-Container-Start=0&X-Plex-Container-Size={LIBRARY_PAGE}"
            ))
            .await?;
        Ok(albums_of(c))
    }
}

fn albums_of(c: MediaContainer) -> Vec<Album> {
    c.directories.into_iter().map(WireDirectory::into_album).collect()
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

    const TOP: &str = "<MediaContainer><Directory ratingKey=\"9\" key=\"/library/metadata/9\" type=\"album\" title=\"A\" parentTitle=\"B\" year=\"2020\" globalViewCount=\"12\" viewCount=\"30\"/><Directory ratingKey=\"8\" key=\"/library/metadata/8\" type=\"album\" title=\"C\" viewCount=\"4\"/></MediaContainer>";

    async fn top_line_of<F, Fut, T>(call: F) -> String
    where
        F: FnOnce(PlexClient) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let (base, seen, _) = serve_full(|_| (200, TOP.to_string())).await;
        call(PlexClient::new(&base, "faketoken-lists")).await.unwrap();
        let lines = seen.lock().unwrap().clone();
        lines[0].clone()
    }

    async fn top_line(list: LibraryList, span: Option<(i64, i64)>) -> String {
        top_line_of(|client| async move { client.top_list("1", list, span).await }).await
    }

    #[tokio::test]
    async fn a_top_list_for_a_period_asks_for_its_bounds_and_one_page() {
        let span = Some((1_761_955_200, 1_764_547_199));
        for (list, type_) in [
            (LibraryList::TopTracks, 10),
            (LibraryList::TopAlbums, 9),
            (LibraryList::TopArtists, 8),
        ] {
            let line = top_line(list, span).await;
            assert!(line.starts_with("GET /library/all/top?"), "{line}");
            for part in [
                format!("type={type_}").as_str(),
                "librarySectionID=1",
                "viewedAt%3E=1761955200",
                "viewedAt%3C=1764547199",
                "accountID=1",
                "Start=0",
                "Size=100",
            ] {
                assert!(line.contains(part), "{part} in {line}");
            }
        }
    }

    #[tokio::test]
    async fn a_top_list_for_all_time_sorts_the_section_by_play_count() {
        let line = top_line(LibraryList::TopAlbums, None).await;
        assert!(line.starts_with("GET /library/sections/1/all?"), "{line}");
        for part in ["type=9", "viewCount%3E=1", "sort=viewCount:desc", "Start=0", "Size=100"] {
            assert!(line.contains(part), "{part} in {line}");
        }
    }

    #[tokio::test]
    async fn plays_come_from_the_period_count_before_the_total_count() {
        let (base, _, _) = serve_full(|_| (200, TOP.to_string())).await;
        let client = PlexClient::new(&base, "faketoken-lists");
        let TopRows::Albums(albums) =
            client.top_list("1", LibraryList::TopAlbums, Some((0, 1))).await.unwrap()
        else {
            panic!("expected albums")
        };
        assert_eq!(albums.iter().map(|a| a.plays).collect::<Vec<_>>(), [Some(12), Some(4)]);
    }

    #[tokio::test]
    async fn best_of_choices_read_the_years_and_decades_from_their_keys() {
        let (base, seen, _) = serve_full(|line| {
            let body = if line.contains("/year?") {
                "<MediaContainer><Directory title=\"2026\" key=\"2026\"/><Directory title=\"2025\" key=\"2025\"/></MediaContainer>"
            } else {
                "<MediaContainer><Directory title=\"2020s\" key=\"2020\"/><Directory title=\"2010s\" key=\"2010\"/></MediaContainer>"
            };
            (200, body.to_string())
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-lists");
        let (years, decades) = client.best_of_choices("1").await.unwrap();
        assert_eq!((years, decades), (vec![2026, 2025], vec![2020, 2010]));
        let lines = seen.lock().unwrap().clone();
        assert!(lines.iter().any(|l| l.starts_with("GET /library/sections/1/year?type=9")));
        assert!(lines.iter().any(|l| l.starts_with("GET /library/sections/1/decade?type=9")));
    }

    #[tokio::test]
    async fn best_of_albums_filter_by_year_or_decade_and_sort_by_play_count() {
        for (what, filter, label) in [
            (BestOf::Year(2025), "year=2025", "2025"),
            (BestOf::Decade(2020), "decade=2020", "2020s"),
        ] {
            let line =
                top_line_of(|client| async move { client.best_of_albums("1", what).await }).await;
            assert!(line.starts_with("GET /library/sections/1/all?"), "{line}");
            for part in
                ["type=9", filter, "viewCount%3E=1", "sort=viewCount:desc", "Start=0", "Size=100"]
            {
                assert!(line.contains(part), "{part} in {line}");
            }
            assert_eq!(what.label(), label);
        }
    }
}
