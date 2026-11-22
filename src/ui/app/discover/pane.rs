use crate::plex::discover::{LibraryList, ShelfItems};
use crate::plex::radio::RadioQueue;
use crate::plex::Track;
use ratatui::widgets::TableState;

#[derive(PartialEq, Eq)]
pub enum TracksSource {
    Station(RadioQueue),
    Library(LibraryList),
    Album,
    Playlist { key: String },
}

pub struct TracksPane {
    pub shelf: String,
    pub name: String,
    pub tracks: Vec<Track>,
    /// The server's count; a playlist pane holds only its first page.
    pub total: usize,
    pub state: TableState,
    pub source: TracksSource,
}

pub struct ItemsPane {
    pub shelf: String,
    pub name: String,
    pub items: ShelfItems,
    pub state: TableState,
    pub plays: bool,
}

#[derive(Default)]
pub enum Pane {
    #[default]
    Empty,
    Tracks(TracksPane),
    Items(ItemsPane),
}

pub fn trail(shelf: &str, name: &str) -> String {
    match (shelf.is_empty(), name.is_empty()) {
        (_, true) => shelf.to_string(),
        (true, false) => name.to_string(),
        (false, false) => format!("{shelf} ▸ {name}"),
    }
}

impl TracksPane {
    pub fn label(&self) -> String {
        trail(&self.shelf, &self.name)
    }
}

impl ItemsPane {
    pub fn label(&self) -> String {
        trail(&self.shelf, &self.name)
    }
}

impl Pane {
    pub fn is_empty(&self) -> bool {
        match self {
            Pane::Empty => true,
            Pane::Tracks(p) => p.tracks.is_empty(),
            Pane::Items(p) => p.items.is_empty(),
        }
    }
}
