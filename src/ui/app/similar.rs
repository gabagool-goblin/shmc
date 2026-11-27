//! `m`: tracks, albums or artists that sound like the selected one.

use super::{App, ListKind, ListRef, PlexCmd, SearchDetail, Tab};
use crate::plex::discover::ShelfItems;
use crate::plex::{Album, Artist, HitKind, SearchHit, Track};
use ratatui::widgets::TableState;

/// What `m` was pressed on, and so what it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimilarKind {
    Tracks,
    Albums,
    Artists,
}

impl SimilarKind {
    /// "1 album", "3 albums".
    pub fn count(self, n: usize) -> String {
        let noun = match (self, n) {
            (Self::Tracks, 1) => "track",
            (Self::Albums, 1) => "album",
            (Self::Artists, 1) => "artist",
            _ => self.plural(),
        };
        format!("{n} {noun}")
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::Albums => "albums",
            Self::Artists => "artists",
        }
    }
}

/// Labels the Search tab's results while they are a similar albums or artists list.
pub struct SimilarHits {
    pub label: String,
    pub kind: SimilarKind,
    /// The match percentage of each hit, in the hits' order.
    pub matches: Vec<Option<u8>>,
}

#[derive(Debug)]
pub enum SimilarOutcome {
    /// Plex's order, each with its match percentage when it gave a distance.
    Tracks(Vec<(Track, Option<u8>)>),
    /// Albums or artists, as search hits, in the same order.
    Hits(Vec<(SearchHit, Option<u8>)>),
    NoSonic,
    NotAnalysed,
    NoneFound,
}

struct Seed {
    key: String,
    kind: SimilarKind,
    exclude: Option<String>,
    label: String,
}

impl Seed {
    fn of(t: &Track) -> Self {
        Self {
            key: t.rating_key.clone(),
            kind: SimilarKind::Tracks,
            exclude: t.grandparent_rating_key.clone(),
            label: format!("Sonically similar to {} — {}", t.title, t.artist),
        }
    }

    fn of_album(a: &Album) -> Self {
        Self {
            key: a.rating_key.clone(),
            kind: SimilarKind::Albums,
            exclude: None,
            label: format!("Sonically similar to {} — {}", a.title, a.artist),
        }
    }

    fn of_artist(a: &Artist) -> Self {
        Self {
            key: a.rating_key.clone(),
            kind: SimilarKind::Artists,
            exclude: None,
            label: format!("Sonically similar to {}", a.title),
        }
    }

    fn of_hit(hit: &SearchHit) -> Self {
        let (kind, exclude, label) = match hit.kind {
            HitKind::Track => (
                SimilarKind::Tracks,
                hit.artist_key.clone(),
                format!("Sonically similar to {} — {}", hit.title, hit.artist),
            ),
            HitKind::Album => (
                SimilarKind::Albums,
                None,
                format!("Sonically similar to {} — {}", hit.title, hit.artist),
            ),
            HitKind::Artist => {
                (SimilarKind::Artists, None, format!("Sonically similar to {}", hit.artist))
            }
        };
        Self { key: hit.rating_key.clone(), kind, exclude, label }
    }
}

impl App {
    fn similar_seed(&self) -> Option<Seed> {
        let list = self.active_list();
        let real = self.real_at(list)?;
        match list {
            ListRef::Queue => self.queue.items.get(real).map(Seed::of),
            ListRef::BTracks => self.browser.tracks.items.get(real).map(Seed::of),
            ListRef::PlItems => self.playlists.tracks.items.get(real).map(Seed::of),
            ListRef::SearchDetail => self.search.detail.as_ref()?.tracks.get(real).map(Seed::of),
            ListRef::DTracks => self.discover.tracks().get(real).map(Seed::of),
            ListRef::DItems => match self.discover.items()? {
                ShelfItems::Albums(v) => v.get(real).map(Seed::of_album),
                ShelfItems::Artists(v) => v.get(real).map(Seed::of_artist),
                ShelfItems::Playlists(_) | ShelfItems::BestOf(_) => None,
            },
            ListRef::BArtists => self.browser.artists.items.get(real).map(Seed::of_artist),
            ListRef::BAlbums => self.browser.albums.items.get(real).map(Seed::of_album),
            ListRef::Search => self.search.hits.get(real).map(Seed::of_hit),
            _ => None,
        }
    }

    pub(super) fn find_similar(&mut self) {
        let Some(seed) = self.similar_seed() else {
            self.transient("m works on a track, album or artist".into());
            return;
        };
        self.search.request_list(seed.key.clone(), ListKind::Similar(seed.kind), self.tab);
        let _ = self.plex.tx.send(PlexCmd::LoadSimilar {
            key: seed.key,
            kind: seed.kind,
            exclude: seed.exclude,
            label: seed.label,
        });
        self.transient(format!("finding similar {}…", seed.kind.plural()));
    }

    /// Replaces or closes the Search tab's detail list, ending any selection in it.
    pub(super) fn set_search_detail(&mut self, d: Option<SearchDetail>) {
        self.search.detail = d;
        self.drop_visual(ListRef::SearchDetail);
    }

    pub(super) fn similar_loaded(&mut self, key: String, label: String, outcome: SimilarOutcome) {
        let Some(req) = self.search.take_list_request(&key, |k| matches!(k, ListKind::Similar(_)))
        else {
            return;
        };
        let ListKind::Similar(kind) = req.kind else { return };
        match outcome {
            SimilarOutcome::Tracks(rows) => {
                self.push_back_for_detail(req.origin);
                let (tracks, matches): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
                let mut state = TableState::default();
                if !tracks.is_empty() {
                    state.select(Some(0));
                }
                self.set_search_detail(Some(SearchDetail {
                    key,
                    label,
                    tracks,
                    state,
                    matches: Some(matches),
                    radio: None,
                }));
                self.search.focused = false;
                self.goto_tab(Tab::Search);
            }
            SimilarOutcome::Hits(rows) => {
                let (hits, matches): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
                self.push_back_for_results(req.origin);
                self.search.state.select((!hits.is_empty()).then_some(0));
                self.search.hits = hits;
                self.search.similar = Some(SimilarHits { label, kind, matches });
                self.set_search_detail(None);
                self.search.focused = false;
                self.goto_tab(Tab::Search);
            }
            SimilarOutcome::NoSonic => self.transient("sonic analysis needs Plex Pass".into()),
            SimilarOutcome::NotAnalysed => {
                self.transient("track not analysed yet".into());
            }
            SimilarOutcome::NoneFound => {
                self.transient(format!("no similar {}", kind.plural()));
            }
        }
    }
}
