//! Esc in the Search tab: steps back through what `m` and Enter replaced there.

use super::{App, SearchDetail, SimilarHits, Tab};
use crate::plex::SearchHit;
use ratatui::widgets::TableState;

/// The most steps kept; past it, the oldest step after the first is dropped.
const MAX_BACK: usize = 20;

/// The results list a similar albums or artists list replaced.
pub struct SavedResults {
    hits: Vec<SearchHit>,
    state: TableState,
    similar: Option<SimilarHits>,
}

/// One step Esc takes back in the Search tab.
pub enum SearchBack {
    /// `m` was pressed in this tab.
    Tab(Tab),
    Results(SavedResults),
    /// An album view or similar tracks list another one replaced.
    Detail(SearchDetail),
    /// A detail opened over the results.
    CloseDetail,
}

impl App {
    fn push_back(&mut self, step: SearchBack) {
        let back = &mut self.search.back;
        if back.len() >= MAX_BACK {
            back.remove(1);
        }
        back.push(step);
    }

    /// Call before a detail list replaces what Search shows, once its reply is applied.
    pub(super) fn push_back_for_detail(&mut self, origin: Tab) {
        if origin != Tab::Search {
            return self.push_back(SearchBack::Tab(origin));
        }
        let step = match self.search.detail.take() {
            Some(d) => SearchBack::Detail(d),
            None => SearchBack::CloseDetail,
        };
        self.push_back(step);
    }

    /// Call before similar hits replace the results, once their reply is applied.
    pub(super) fn push_back_for_results(&mut self, origin: Tab) {
        if origin != Tab::Search {
            return self.push_back(SearchBack::Tab(origin));
        }
        let saved = SavedResults {
            hits: std::mem::take(&mut self.search.hits),
            state: std::mem::take(&mut self.search.state),
            similar: self.search.similar.take(),
        };
        self.push_back(SearchBack::Results(saved));
    }

    /// Esc: cancels pending requests, then takes one step back.
    pub(super) fn search_back(&mut self) {
        self.search.clear_list_request();
        match self.search.back.pop() {
            Some(SearchBack::Tab(t)) => self.goto_tab(t),
            Some(SearchBack::Results(s)) => {
                self.search.hits = s.hits;
                self.search.state = s.state;
                self.search.similar = s.similar;
                self.set_search_detail(None);
            }
            Some(SearchBack::Detail(d)) => self.set_search_detail(Some(d)),
            Some(SearchBack::CloseDetail) | None => self.set_search_detail(None),
        }
    }
}
