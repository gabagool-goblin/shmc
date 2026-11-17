//! Star ratings: the playing track's rating for the header, and `r`.

use super::plex_worker::PlexCmd;
use super::App;
use crate::config::RatingStyle;
use crate::ui::views::{rating_glyphs, rating_short};
use std::time::{Duration, Instant};

/// A "rate the track?" question. The key is fixed when it opens, so a track
/// change while it waits cannot redirect the answer.
pub(super) struct PendingRating {
    key: String,
    armed_at: Instant,
}

impl App {
    /// The rating of a track: a known value for its key, else what its own
    /// metadata carried.
    pub(crate) fn rating_of(&self, t: &crate::plex::Track) -> Option<u8> {
        self.rating_for(&t.rating_key, t.user_rating)
    }

    /// `own` is the rating a list row carried; a cached None is a rating cleared
    /// with r, so the row's own value must not come back.
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

    /// The rating of the playing track.
    pub(crate) fn playing_rating(&self) -> Option<u8> {
        self.rating_of(self.current_track()?)
    }

    /// The header's rating glyphs for the playing track.
    pub(crate) fn rating_label(&self) -> Option<String> {
        rating_glyphs(self.rating_style, self.playing_rating()?, self.icons_ascii)
    }

    pub(super) fn rate_playing(&mut self) {
        if !self.set_ratings {
            self.transient("ratings are read-only: turn on plex.set_ratings".into());
            return;
        }
        if self.rating_style == RatingStyle::Off {
            self.transient("ratings are hidden: set ui.rating_style".into());
            return;
        }
        let Some(track) = self.current_track().filter(|_| !self.stopped) else {
            self.transient("nothing playing".into());
            return;
        };
        let (key, title) = (track.rating_key.clone(), track.title.clone());
        let current = self.playing_rating();
        if self.rating_style == RatingStyle::One {
            self.send_rating(key, if current.is_some() { None } else { Some(10) });
            return;
        }
        let shown = self.rating_label().unwrap_or_else(|| "not rated".into());
        self.pending_playlist = None;
        self.pending_rating = Some(PendingRating { key, armed_at: Instant::now() });
        self.transient_secs(
            format!("rate {title} ({shown}): 1 to 5 · 0 clears · any other key cancels"),
            self.confirm_timeout_secs,
            false,
        );
    }

    /// Whether the question took the key: `0` to `5` answer it, anything else
    /// cancels. A lapsed question lets the key through.
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
                self.send_rating(pending.key, (stars > 0).then_some(stars * 2));
            }
            _ => self.transient("cancelled".into()),
        }
        true
    }

    pub(super) fn has_pending_rating(&self) -> bool {
        self.pending_rating.is_some()
    }

    fn send_rating(&mut self, key: String, rating: Option<u8>) {
        let _ = self.plex.tx.send(PlexCmd::Rate { key, rating });
    }

    /// The server confirmed a write.
    pub(super) fn rating_saved(&mut self, rating_key: String, rating: Option<u8>) {
        // The track may have left the queue before the reply.
        let title = self.queue.iter().find(|t| t.rating_key == rating_key).map(|t| t.title.clone());
        self.ratings.insert(rating_key, rating);
        let what = match (self.rating_style, rating) {
            (RatingStyle::One, Some(_)) => "starred",
            (RatingStyle::One, None) => "unstarred",
            (_, None) => "rating cleared",
            (style, Some(r)) => {
                let glyphs = rating_glyphs(style, r, self.icons_ascii).unwrap_or_default();
                self.transient(match title {
                    Some(t) => format!("rated {t}: {glyphs}"),
                    None => format!("rated: {glyphs}"),
                });
                return;
            }
        };
        self.transient(match title {
            Some(t) => format!("{what}: {t}"),
            None => what.to_string(),
        });
    }
}
