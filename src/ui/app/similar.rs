//! `m`: tracks that sound like the selected one, from Plex's sonic analysis.

use super::{App, ListRef, PlexCmd, SearchDetail, Tab};
use crate::plex::{HitKind, Track};
use ratatui::widgets::TableState;

/// What the worker found for one request.
#[derive(Debug)]
pub enum SimilarOutcome {
    /// Plex's order, each with its match percentage when it gave a distance.
    Tracks(Vec<(Track, Option<u8>)>),
    NoSonic,
    NotAnalysed,
    NoneFound,
}

/// The track `m` was pressed on.
struct Seed {
    key: String,
    exclude: Option<String>,
    label: String,
}

impl Seed {
    fn of(t: &Track) -> Self {
        Self {
            key: t.rating_key.clone(),
            exclude: t.grandparent_rating_key.clone(),
            label: format!("Sonically similar to {} — {}", t.title, t.artist),
        }
    }
}

impl App {
    fn similar_seed(&self) -> Option<Seed> {
        let list = self.active_list();
        let real = self.real_at(list)?;
        match list {
            ListRef::Queue => self.queue.get(real).map(Seed::of),
            ListRef::BTracks => self.browser.tracks.get(real).map(Seed::of),
            ListRef::PlItems => self.playlists.items.get(real).map(Seed::of),
            ListRef::SearchDetail => self.search.detail.as_ref()?.tracks.get(real).map(Seed::of),
            ListRef::Search => {
                let hit = self.search.hits.get(real).filter(|h| h.kind == HitKind::Track)?;
                Some(Seed {
                    key: hit.rating_key.clone(),
                    exclude: hit.artist_key.clone(),
                    label: format!("Sonically similar to {} — {}", hit.title, hit.artist),
                })
            }
            _ => None,
        }
    }

    pub(super) fn find_similar(&mut self) {
        let Some(seed) = self.similar_seed() else {
            self.transient("m works on a track".into());
            return;
        };
        // One request at a time may open the Search tab's list.
        self.search.detail_request = None;
        self.search.similar_request = Some(seed.key.clone());
        let _ = self.plex.tx.send(PlexCmd::LoadSimilar {
            key: seed.key,
            exclude: seed.exclude,
            label: seed.label,
        });
        self.transient("finding similar tracks…".into());
    }

    /// Replaces or closes the Search tab's detail list, ending any selection in it.
    pub(super) fn set_search_detail(&mut self, d: Option<SearchDetail>) {
        self.search.detail = d;
        self.drop_visual(ListRef::SearchDetail);
    }

    pub(super) fn similar_loaded(&mut self, key: String, label: String, outcome: SimilarOutcome) {
        if self.search.similar_request.as_deref() != Some(key.as_str()) {
            return;
        }
        self.search.similar_request = None;
        match outcome {
            SimilarOutcome::Tracks(rows) => {
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
                }));
                self.search.focused = false;
                self.goto_tab(Tab::Search);
            }
            SimilarOutcome::NoSonic => self.transient("sonic analysis needs Plex Pass".into()),
            SimilarOutcome::NotAnalysed => {
                self.transient("Plex hasn't analysed this track yet".into());
            }
            SimilarOutcome::NoneFound => self.transient("Plex found no similar tracks".into()),
        }
    }
}
