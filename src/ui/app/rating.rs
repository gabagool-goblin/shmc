//! Star ratings: the playing track's rating for the header, `r` and `R`.

use super::plex_worker::PlexCmd;
use super::{App, ListRef};
use crate::config::RatingStyle;
use crate::plex::models::HitKind;
use crate::ui::views::{rating_glyphs, rating_short};
use std::time::{Duration, Instant};

/// A "rate the track?" question. The key is fixed when it opens, so a track
/// change while it waits cannot redirect the answer.
pub(super) struct PendingRating {
    key: String,
    title: String,
    armed_at: Instant,
}

impl App {
    /// A known value for its key, else what its own metadata carried.
    pub(crate) fn rating_of(&self, t: &crate::plex::Track) -> Option<u8> {
        self.rating_for(&t.rating_key, t.user_rating)
    }

    /// A cached None is a rating cleared with `r`, so the row's `own` must not come back.
    pub(crate) fn rating_for(&self, key: &str, own: Option<u8>) -> Option<u8> {
        match self.ratings.get(key) {
            Some(known) => *known,
            None => own,
        }
    }

    /// The rating shown after a list row's title.
    pub(crate) fn rating_text(&self, key: &str, own: Option<u8>) -> Option<String> {
        rating_short(self.rating_style, self.rating_for(key, own)?, self.icons_ascii)
    }

    pub(crate) fn playing_rating(&self) -> Option<u8> {
        self.rating_of(self.current_track()?)
    }

    /// The header's rating glyphs for the playing track.
    pub(crate) fn rating_label(&self) -> Option<String> {
        rating_glyphs(self.rating_style, self.playing_rating()?, self.icons_ascii)
    }

    /// Key, title and own rating of the highlighted track.
    pub(super) fn highlighted_track(&self) -> Option<(String, String, Option<u8>)> {
        let list = self.active_list();
        let real = self.real_at(list)?;
        let track = match list {
            ListRef::Queue => self.queue.items.get(real),
            ListRef::BTracks => self.browser.tracks.items.get(real),
            ListRef::PlItems => self.playlists.tracks.items.get(real),
            ListRef::SearchDetail => self.search.detail.as_ref()?.tracks.get(real),
            ListRef::DTracks => self.discover.tracks.get(real),
            ListRef::Search => {
                let hit = self.search.hits.get(real).filter(|h| h.kind == HitKind::Track)?;
                return Some((hit.rating_key.clone(), hit.title.clone(), hit.user_rating));
            }
            _ => None,
        }?;
        Some((track.rating_key.clone(), track.title.clone(), track.user_rating))
    }

    pub(super) fn rate_highlighted(&mut self) {
        if !self.can_rate() {
            return;
        }
        let Some((key, title, own)) = self.highlighted_track() else {
            self.transient("r rates tracks · R rates the playing track".into());
            return;
        };
        let current = self.rating_for(&key, own);
        self.ask_rating(key, title, current);
    }

    pub(super) fn rate_playing(&mut self) {
        if !self.can_rate() {
            return;
        }
        let Some(track) = self.current_track().filter(|_| !self.stopped) else {
            self.transient("nothing playing".into());
            return;
        };
        let (key, title) = (track.rating_key.clone(), track.title.clone());
        let current = self.playing_rating();
        self.ask_rating(key, title, current);
    }

    fn can_rate(&mut self) -> bool {
        if !self.allow_writes {
            self.transient("ratings are read-only: turn on plex.allow_writes".into());
            return false;
        }
        if self.rating_style == RatingStyle::Off {
            self.transient("ratings are hidden: set ui.rating_style".into());
            return false;
        }
        true
    }

    fn ask_rating(&mut self, key: String, title: String, current: Option<u8>) {
        if self.rating_style == RatingStyle::One {
            self.send_rating(key, title, if current.is_some() { None } else { Some(10) });
            return;
        }
        let shown = current
            .and_then(|r| rating_glyphs(self.rating_style, r, self.icons_ascii))
            .unwrap_or_else(|| "not rated".into());
        let question = format!("rate {title} ({shown}): 1 to 5 · 0 clears · any other key cancels");
        self.pending_playlist = None;
        self.pending_rating = Some(PendingRating { key, title, armed_at: Instant::now() });
        self.transient_secs(question, self.confirm_timeout_secs, false);
    }

    /// `0` to `5` answer, anything else cancels; a lapsed question lets the key through.
    pub(super) fn answer_rating(&mut self, key: crossterm::event::KeyCode) -> bool {
        let Some(pending) = self.pending_rating.take() else {
            return false;
        };
        if pending.armed_at.elapsed() >= Duration::from_secs(self.confirm_timeout_secs) {
            return false;
        }
        match key {
            crossterm::event::KeyCode::Char(c @ '0'..='5') => {
                let stars = c.to_digit(10).unwrap_or(0) as u8;
                self.send_rating(pending.key, pending.title, (stars > 0).then_some(stars * 2));
            }
            _ => self.transient("cancelled".into()),
        }
        true
    }

    pub(super) fn has_pending_rating(&self) -> bool {
        self.pending_rating.is_some()
    }

    fn send_rating(&mut self, key: String, title: String, rating: Option<u8>) {
        let _ = self.plex.tx.send(PlexCmd::Rate { key, rating, title });
    }

    /// The server confirmed a write.
    pub(super) fn rating_saved(&mut self, rating_key: String, rating: Option<u8>, title: String) {
        self.ratings.insert(rating_key, rating);
        let msg = match (self.rating_style, rating) {
            (RatingStyle::One, Some(_)) => format!("starred: {title}"),
            (RatingStyle::One, None) => format!("unstarred: {title}"),
            (_, None) => format!("rating cleared: {title}"),
            (style, Some(r)) => {
                let glyphs = rating_glyphs(style, r, self.icons_ascii).unwrap_or_default();
                format!("rated {title}: {glyphs}")
            }
        };
        self.transient(msg);
    }
}
