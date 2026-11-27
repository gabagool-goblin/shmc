//! The Discover tab: Plex's shelves, the library's radio stations and its lists.

mod actions;
mod best_of;
mod hubs;
mod library;
mod open;
mod pane;
mod sections;
mod stations;
mod top;

use super::radio::RadioEnv;
use super::{App, ListRef, Msg, PlexCmd};
use crate::plex::discover::{BestOf, DiscoverHubs, LibraryList, Shelf, ShelfItems};
use crate::plex::radio::Station;
use crate::plex::sonic::Features;
use crate::plex::{Album, PlexClient, Track};
use crate::ui::keymap::Act;
pub use best_of::Choices;
pub use library::{Library, LibraryOutcome};
pub use pane::{trail, ItemsPane, Pane, TracksPane, TracksSource};
use ratatui::widgets::ListState;
pub use sections::Row;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverCol {
    #[default]
    Sections,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoverLoad {
    #[default]
    NotLoaded,
    Loading,
    Loaded,
    Failed,
}

#[derive(Debug, Clone)]
pub enum Preview {
    Loading,
    Ready(Vec<Track>, crate::plex::radio::RadioQueue),
    Failed(String),
}

#[derive(Debug, Clone, Copy)]
pub enum OpenKind {
    Album,
    Artist,
    Playlist { size: usize },
    BestOf(BestOf),
}

#[derive(Debug)]
pub enum Opened {
    Tracks { tracks: Vec<Track>, total: Option<usize> },
    Albums(Vec<Album>),
    Failed(String),
}

#[derive(Debug)]
pub enum DiscoverMsg {
    Loaded(DiscoverHubs),
    LoadFailed,
    StationPreview { key: String, preview: Preview },
    Opened { key: String, outcome: Opened },
    Library { list: LibraryList, generation: u64, start: usize, outcome: LibraryOutcome },
    BestOf { generation: u64, outcome: Result<(Vec<u32>, Vec<u32>), String> },
}

/// Each command replies once with a `Msg::Discover`, never `Msg::Error`.
#[derive(Debug)]
pub enum DiscoverCmd {
    Load,
    PreviewStation { key: String, title: String },
    Open { key: String, what: OpenKind },
    LoadLibrary { list: LibraryList, generation: u64, start: usize, span: Option<(i64, i64)> },
    LoadBestOf { generation: u64 },
}

struct PendingOpen {
    key: String,
    shelf: String,
    name: String,
    what: OpenKind,
}

#[derive(Default)]
pub struct DiscoverView {
    pub load: DiscoverLoad,
    pub shelves: Vec<Shelf>,
    pub stations: Vec<Station>,
    pub rows: Vec<Row>,
    pub sections_state: ListState,
    pub focus: DiscoverCol,
    pub pane: Pane,
    pub library: Library,
    pub best_of: Choices,
    back: Vec<Pane>,
    pending: Option<PendingOpen>,
    followed: Option<Row>,
    /// Each station's tracks, fetched once; every fetch makes a play queue.
    previews: HashMap<String, Preview>,
    /// Stations played since their preview was fetched.
    played: HashSet<String>,
}

static NO_TRACKS: Vec<Track> = Vec::new();

impl DiscoverView {
    pub fn tracks(&self) -> &Vec<Track> {
        match &self.pane {
            Pane::Tracks(p) => &p.tracks,
            _ => &NO_TRACKS,
        }
    }

    pub fn items(&self) -> Option<&ShelfItems> {
        match &self.pane {
            Pane::Items(p) => Some(&p.items),
            _ => None,
        }
    }

    pub fn shown_preview(&self) -> Option<&Preview> {
        match self.followed? {
            Row::Station(i) => self.previews.get(&self.stations.get(i)?.key),
            _ => None,
        }
    }
}

pub(super) async fn run(
    client: &PlexClient,
    section: &str,
    features: &Mutex<Option<Features>>,
    env: &RadioEnv,
    tx: &UnboundedSender<Msg>,
    cmd: DiscoverCmd,
) {
    match cmd {
        DiscoverCmd::Load => hubs::load(client, section, features, tx).await,
        DiscoverCmd::PreviewStation { key, title } => {
            stations::preview_station(client, features, env, tx, key, title).await;
        }
        DiscoverCmd::Open { key, what } => open::open(client, section, tx, key, what).await,
        DiscoverCmd::LoadLibrary { list, generation, start, span } => {
            library::load(client, section, tx, list, generation, start, span).await;
        }
        DiscoverCmd::LoadBestOf { generation } => {
            best_of::load(client, section, tx, generation).await;
        }
    }
}

impl App {
    pub(super) fn discover_msg(&mut self, msg: DiscoverMsg) {
        match msg {
            DiscoverMsg::Loaded(hubs) => self.discover_loaded(hubs),
            DiscoverMsg::LoadFailed => {
                if self.discover.rows.is_empty() {
                    self.discover_loaded(DiscoverHubs::default());
                }
                self.discover.load = DiscoverLoad::Failed;
                self.transient_err("could not load Discover".into());
            }
            DiscoverMsg::StationPreview { key, preview } => self.station_preview(key, preview),
            DiscoverMsg::Opened { key, outcome } => self.discover_opened(key, outcome),
            DiscoverMsg::Library { list, generation, start, outcome } => {
                self.library_loaded(list, generation, start, outcome);
            }
            DiscoverMsg::BestOf { generation, outcome } => self.best_of_loaded(generation, outcome),
        }
    }

    pub(super) fn discover_list(&self) -> ListRef {
        match (self.discover.focus, &self.discover.pane) {
            (DiscoverCol::Right, Pane::Tracks(_)) => ListRef::DTracks,
            (DiscoverCol::Right, Pane::Items(_)) => ListRef::DItems,
            _ => ListRef::DSections,
        }
    }

    pub(super) fn open_discover(&mut self) {
        if matches!(self.discover.load, DiscoverLoad::NotLoaded | DiscoverLoad::Failed) {
            self.request_discover();
        }
    }

    /// Cached station previews stay.
    pub(super) fn refresh_discover(&mut self) {
        if self.discover.load != DiscoverLoad::Loading {
            self.request_discover();
        }
        self.transient("reloading Discover…".into());
    }

    fn request_discover(&mut self) {
        self.discover.load = DiscoverLoad::Loading;
        let _ = self.plex.tx.send(PlexCmd::Discover(DiscoverCmd::Load));
    }

    pub(super) fn discover_column_left(&mut self) {
        self.discover.focus = DiscoverCol::Sections;
    }

    pub(super) fn discover_column_right(&mut self) {
        if !self.discover.pane.is_empty() {
            self.discover.focus = DiscoverCol::Right;
        }
    }

    pub(super) fn reveal_discover(&mut self) {
        let list = self.discover_list();
        let target = self.real_at(list).and_then(|real| match list {
            ListRef::DTracks => self.discover.tracks().get(real).map(|t| {
                (
                    t.grandparent_rating_key.clone(),
                    t.parent_rating_key.clone(),
                    Some(t.rating_key.clone()),
                )
            }),
            ListRef::DItems => match self.discover.items()? {
                ShelfItems::Albums(v) => {
                    v.get(real).map(|a| (a.artist_key.clone(), Some(a.rating_key.clone()), None))
                }
                ShelfItems::Artists(v) => {
                    v.get(real).map(|a| (Some(a.rating_key.clone()), None, None))
                }
                ShelfItems::Playlists(_) | ShelfItems::BestOf(_) => None,
            },
            _ => None,
        });
        match target {
            Some((artist, album, track)) => self.reveal(artist, album, track),
            None => {
                if let Some(key) = self.keymap.first_key(self.key_ctx(), &Act::GotoAlbum) {
                    self.transient(format!("{key} works on a track, album or artist"));
                }
            }
        }
    }
}
