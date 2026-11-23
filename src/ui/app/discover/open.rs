use super::{DiscoverCmd, DiscoverCol, ItemsPane, OpenKind, Opened, Pane, PendingOpen};
use super::{TracksPane, TracksSource};
use crate::plex::discover::ShelfItems;
use crate::plex::PlexClient;
use crate::ui::app::{sort_albums, App, ListRef, Msg, PlexCmd};
use tokio::sync::mpsc::UnboundedSender;

pub(super) async fn open(
    client: &PlexClient,
    section: &str,
    tx: &UnboundedSender<Msg>,
    key: String,
    what: OpenKind,
) {
    let result = match what {
        OpenKind::Album => client
            .album_tracks(&key)
            .await
            .map(|tracks| Opened::Tracks { total: Some(tracks.len()), tracks })
            .map_err(|e| format!("loading album: {e:#}")),
        OpenKind::Artist => client
            .albums(section, &key)
            .await
            .map(Opened::Albums)
            .map_err(|e| format!("loading albums: {e:#}")),
        OpenKind::BestOf(what) => client
            .best_of_albums(section, what)
            .await
            .map(Opened::Albums)
            .map_err(|e| format!("loading albums: {e:#}")),
        OpenKind::Playlist { size } => client
            .playlist_page(&key, 0, size)
            .await
            .map(|page| Opened::Tracks { total: page.total, tracks: page.tracks })
            .map_err(|e| format!("loading playlist: {e:#}")),
    };
    let outcome = result.unwrap_or_else(Opened::Failed);
    let _ = tx.send(Msg::Discover(super::DiscoverMsg::Opened { key, outcome }));
}

impl App {
    pub(in crate::ui::app) fn open_item(&mut self) {
        let Some(real) = self.real_at(ListRef::DItems) else { return };
        let Pane::Items(pane) = &self.discover.pane else { return };
        let size = self.preview_tracks.max(1);
        let opened = match &pane.items {
            ShelfItems::Albums(v) => v.get(real).map(|a| {
                (a.rating_key.clone(), a.title.clone(), OpenKind::Album, "loading album…")
            }),
            ShelfItems::Artists(v) => v.get(real).map(|a| {
                (a.rating_key.clone(), a.title.clone(), OpenKind::Artist, "loading albums…")
            }),
            ShelfItems::BestOf(v) => v.get(real).map(|b| {
                let label = b.label();
                (format!("best-of:{label}"), label, OpenKind::BestOf(*b), "loading albums…")
            }),
            ShelfItems::Playlists(v) => v.get(real).map(|p| {
                let what = OpenKind::Playlist { size };
                (p.rating_key.clone(), p.title.clone(), what, "loading playlist…")
            }),
        };
        let Some((key, name, what, status)) = opened else { return };
        let shelf = pane.shelf.clone();
        self.discover.pending = Some(PendingOpen { key: key.clone(), shelf, name, what });
        let _ = self.plex.tx.send(PlexCmd::Discover(DiscoverCmd::Open { key, what }));
        self.transient(status.into());
    }

    pub(super) fn discover_opened(&mut self, key: String, outcome: Opened) {
        if self.discover.pending.as_ref().map(|p| p.key.as_str()) != Some(key.as_str()) {
            return;
        }
        let Some(PendingOpen { shelf, name, what, .. }) = self.discover.pending.take() else {
            return;
        };
        let new = match outcome {
            Opened::Failed(msg) => return self.transient_err(msg),
            Opened::Tracks { tracks, .. } if tracks.is_empty() => {
                return self.transient("no tracks".into());
            }
            Opened::Albums(albums) if albums.is_empty() => {
                let none = match what {
                    OpenKind::BestOf(_) => "no played albums",
                    _ => "no albums",
                };
                return self.transient(none.into());
            }
            Opened::Tracks { tracks, total } => {
                let source = match what {
                    OpenKind::Playlist { .. } => TracksSource::Playlist { key },
                    _ => TracksSource::Album,
                };
                let mut pane = TracksPane {
                    shelf,
                    name,
                    total: total.unwrap_or(tracks.len()),
                    tracks,
                    state: Default::default(),
                    source,
                };
                pane.state.select((!pane.tracks.is_empty()).then_some(0));
                Pane::Tracks(pane)
            }
            Opened::Albums(mut albums) => {
                let best_of = matches!(what, OpenKind::BestOf(_));
                if !best_of {
                    sort_albums(&mut albums, &self.album_sort);
                }
                let mut pane = ItemsPane {
                    shelf,
                    name,
                    items: ShelfItems::Albums(albums),
                    state: Default::default(),
                    plays: best_of,
                    counted: true,
                };
                pane.state.select((!pane.items.is_empty()).then_some(0));
                Pane::Items(pane)
            }
        };
        self.drop_visual(ListRef::DTracks);
        let old = std::mem::replace(&mut self.discover.pane, new);
        self.discover.back.push(old);
        self.discover.focus = DiscoverCol::Right;
        self.settle_focus();
    }

    pub(in crate::ui::app) fn discover_back(&mut self) -> bool {
        if self.discover.pending.take().is_some() {
            return true;
        }
        let Some(pane) = self.discover.back.pop() else { return false };
        self.drop_visual(ListRef::DTracks);
        self.discover.pane = pane;
        self.settle_focus();
        true
    }
}
