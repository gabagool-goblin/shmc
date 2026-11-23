use super::{DiscoverCmd, ItemsPane, Pane, Row, TracksPane, TracksSource};
use crate::plex::discover::{LibraryList, ShelfItems, TopRows, TrackPage};
use crate::plex::{Album, Artist, PlexClient, Track};
use crate::ui::app::{App, Msg, PlexCmd};
use crate::ui::period::Period;
use crate::ui::when::now_unix;
use std::collections::HashSet;
use tokio::sync::mpsc::UnboundedSender;

const PREFETCH: usize = 20;

#[derive(Debug)]
pub enum LibraryOutcome {
    Tracks(TrackPage),
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
    Failed(String),
}

impl From<TopRows> for LibraryOutcome {
    fn from(rows: TopRows) -> Self {
        match rows {
            TopRows::Tracks(page) => Self::Tracks(page),
            TopRows::Albums(albums) => Self::Albums(albums),
            TopRows::Artists(artists) => Self::Artists(artists),
        }
    }
}

#[derive(Default, PartialEq)]
enum Fetch {
    #[default]
    Idle,
    Loading,
    Failed(String),
}

#[derive(Default)]
struct ListData {
    /// Bumped on every reset, so a reply to an earlier load is dropped.
    generation: u64,
    loaded: bool,
    fetch: Fetch,
    span: Option<(i64, i64)>,
    tracks: Vec<Track>,
    albums: Vec<Album>,
    artists: Vec<Artist>,
    /// Server rows read so far: the next page's start.
    fetched: usize,
    total: usize,
    /// Plex lists a recording on each compilation it is on.
    seen: HashSet<(String, String, Option<u64>)>,
}

#[derive(Default)]
pub struct Library {
    lists: [ListData; 6],
    pub period: Period,
}

impl ListData {
    fn reset(&mut self) {
        *self = Self { generation: self.generation + 1, ..Default::default() };
    }
}

impl Library {
    pub fn reset(&mut self) {
        self.lists.iter_mut().for_each(ListData::reset);
    }

    pub fn reset_list(&mut self, list: LibraryList) {
        self.lists[list.index()].reset();
    }

    pub fn message(&self, list: LibraryList) -> String {
        match &self.lists[list.index()].fetch {
            Fetch::Failed(msg) => msg.clone(),
            _ if self.lists[list.index()].loaded => "nothing found".into(),
            _ => format!("loading {}…", list.title().to_lowercase()),
        }
    }
}

pub(super) async fn load(
    client: &PlexClient,
    section: &str,
    tx: &UnboundedSender<Msg>,
    list: LibraryList,
    generation: u64,
    start: usize,
    span: Option<(i64, i64)>,
) {
    let result = match list {
        LibraryList::NeverPlayed => client.never_played(section).await.map(LibraryOutcome::Albums),
        _ if list.is_top() => client.top_list(section, list, span).await.map(Into::into),
        _ => client.library_tracks(section, list, start).await.map(LibraryOutcome::Tracks),
    };
    let outcome = result.unwrap_or_else(|e| {
        LibraryOutcome::Failed(format!("loading {}: {e:#}", list.title().to_lowercase()))
    });
    let msg = super::DiscoverMsg::Library { list, generation, start, outcome };
    let _ = tx.send(Msg::Discover(msg));
}

impl App {
    pub(super) fn follow_library(&mut self, list: LibraryList, now: i64) {
        let data = &mut self.discover.library.lists[list.index()];
        if matches!(data.fetch, Fetch::Failed(_)) {
            data.fetch = Fetch::Idle;
        }
        self.renew_top(list, now);
        let data = &self.discover.library.lists[list.index()];
        if !data.loaded && data.fetch == Fetch::Idle {
            self.request_library(list, 0, now);
        }
        self.show_library(list);
    }

    /// Fetches a Top list again once its period has rolled over.
    pub(super) fn renew_top(&mut self, list: LibraryList, now: i64) {
        let lib = &self.discover.library;
        let data = &lib.lists[list.index()];
        let asked = data.loaded || data.fetch == Fetch::Loading;
        if !list.is_top() || !asked || lib.period.span(now) == Ok(data.span) {
            return;
        }
        self.discover.library.reset_list(list);
        self.drop_visual(crate::ui::app::ListRef::DTracks);
        self.request_library(list, 0, now);
        self.show_library(list);
        self.settle_focus();
    }

    fn request_library(&mut self, list: LibraryList, start: usize, now: i64) {
        let lib = &mut self.discover.library;
        let span = if list.is_top() { lib.period.span(now) } else { Ok(None) };
        let data = &mut lib.lists[list.index()];
        let span = match span {
            Ok(span) => span,
            Err(why) => {
                let title = list.title().to_lowercase();
                data.fetch = Fetch::Failed(format!("loading {title}: {why}"));
                return;
            }
        };
        if start == 0 {
            data.span = span;
        }
        data.fetch = Fetch::Loading;
        let cmd = DiscoverCmd::LoadLibrary { list, generation: data.generation, start, span };
        let _ = self.plex.tx.send(PlexCmd::Discover(cmd));
    }

    fn show_library(&mut self, list: LibraryList) {
        let lib = &self.discover.library;
        let data = &lib.lists[list.index()];
        if !data.loaded {
            self.discover.pane = Pane::Empty;
            return;
        }
        let title = if list.is_top() {
            format!("{} · {}", list.title(), lib.period.label(now_unix()))
        } else {
            list.title().into()
        };
        let items = match list {
            LibraryList::NeverPlayed | LibraryList::TopAlbums => {
                Some(ShelfItems::Albums(data.albums.clone()))
            }
            LibraryList::TopArtists => Some(ShelfItems::Artists(data.artists.clone())),
            _ => None,
        };
        self.discover.pane = if let Some(items) = items {
            let plays = list.is_top();
            let mut pane = ItemsPane {
                shelf: title,
                name: String::new(),
                items,
                state: Default::default(),
                plays,
                counted: list.is_top(),
            };
            pane.state.select((!pane.items.is_empty()).then_some(0));
            Pane::Items(pane)
        } else {
            let mut pane = TracksPane {
                shelf: String::new(),
                name: title,
                tracks: data.tracks.clone(),
                total: data.total,
                state: Default::default(),
                source: TracksSource::Library(list),
            };
            pane.state.select((!pane.tracks.is_empty()).then_some(0));
            Pane::Tracks(pane)
        };
    }

    pub(super) fn page_library(&mut self) {
        let Some(Row::Library(list)) = self.discover.followed else { return };
        if !list.paged() {
            return;
        }
        let Pane::Tracks(pane) = &self.discover.pane else { return };
        if pane.source != TracksSource::Library(list) {
            return;
        }
        let Some(cursor) = pane.state.selected() else { return };
        let data = &self.discover.library.lists[list.index()];
        if data.fetch != Fetch::Idle
            || data.fetched >= data.total
            || cursor + PREFETCH < pane.tracks.len()
        {
            return;
        }
        let start = data.fetched;
        self.request_library(list, start, now_unix());
    }

    pub(super) fn library_loaded(
        &mut self,
        list: LibraryList,
        generation: u64,
        start: usize,
        outcome: LibraryOutcome,
    ) {
        let data = &mut self.discover.library.lists[list.index()];
        let expected = if list.paged() { data.fetched } else { 0 };
        if generation != data.generation || start != expected || data.fetch != Fetch::Loading {
            return;
        }
        data.fetch = Fetch::Idle;
        let mut kept = Vec::new();
        match outcome {
            LibraryOutcome::Failed(msg) => {
                data.fetch = Fetch::Failed(msg.clone());
                if start > 0 {
                    self.transient_err(msg);
                }
                return;
            }
            LibraryOutcome::Albums(albums) => data.albums = albums,
            LibraryOutcome::Artists(artists) => data.artists = artists,
            LibraryOutcome::Tracks(page) => {
                data.fetched = start + page.rows;
                data.total =
                    if page.rows == 0 { data.fetched } else { page.total.unwrap_or(data.fetched) };
                for t in page.tracks {
                    let key = (t.title.to_lowercase(), t.artist.clone(), t.duration_ms);
                    if data.seen.insert(key) {
                        kept.push(t);
                    }
                }
                data.tracks.extend(kept.iter().cloned());
                if !list.paged() {
                    data.total = data.tracks.len();
                }
            }
        }
        data.loaded = true;
        let total = data.total;
        if self.discover.followed != Some(Row::Library(list)) {
            return;
        }
        match &mut self.discover.pane {
            Pane::Tracks(pane) if pane.source == TracksSource::Library(list) => {
                pane.tracks.extend(kept);
                pane.total = total;
            }
            _ => {
                self.show_library(list);
                self.settle_focus();
            }
        }
    }
}
