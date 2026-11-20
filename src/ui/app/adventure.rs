//! Sonic Adventure: `p` picks stops and `P` asks Plex for a path through them.

use super::plex_worker::cached_features;
use super::{App, ListKind, Msg, PlexCmd, SearchDetail, Tab};
use crate::plex::sonic::Features;
use crate::plex::{PlexClient, Track};
use crate::ui::keymap::Act;
use ratatui::widgets::TableState;
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

const MAX_STOPS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub key: String,
    pub title: String,
}

#[derive(Default)]
pub struct Adventure {
    pub stops: Vec<Stop>,
}

#[derive(Debug)]
pub enum AdventureOutcome {
    Path(Vec<Track>),
    NoSonic,
    NoPath { from: String, to: String },
    Failed,
}

/// Worker side: a `Msg::AdventureProgress` before each leg, then one `Msg::AdventureBuilt`.
pub(super) async fn run(
    client: &PlexClient,
    section: &str,
    features: &Mutex<Option<Features>>,
    tx: &UnboundedSender<Msg>,
    key: String,
    stops: Vec<Stop>,
) {
    let outcome = build(client, section, features, tx, &key, &stops).await;
    let _ = tx.send(Msg::AdventureBuilt { key, outcome });
}

async fn build(
    client: &PlexClient,
    section: &str,
    features: &Mutex<Option<Features>>,
    tx: &UnboundedSender<Msg>,
    key: &str,
    stops: &[Stop],
) -> AdventureOutcome {
    if cached_features(client, features).await.is_some_and(|f| !f.sonic_analysis) {
        return AdventureOutcome::NoSonic;
    }
    let legs = stops.len().saturating_sub(1);
    let mut path = Vec::new();
    for (i, pair) in stops.windows(2).enumerate() {
        let _ = tx.send(Msg::AdventureProgress { key: key.to_string(), leg: i + 1, legs });
        match client.compute_path(section, &pair[0].key, &pair[1].key).await {
            Ok(leg) if !leg.is_empty() => stitch(&mut path, leg),
            Ok(_) => {
                return AdventureOutcome::NoPath {
                    from: pair[0].title.clone(),
                    to: pair[1].title.clone(),
                };
            }
            Err(e) => {
                tracing::warn!("sonic adventure: {e:#}");
                return AdventureOutcome::Failed;
            }
        }
    }
    AdventureOutcome::Path(path)
}

/// Appends `leg` minus its first track (the path's last) and any track the path holds.
fn stitch(path: &mut Vec<Track>, leg: Vec<Track>) {
    let junction = usize::from(!path.is_empty());
    for t in leg.into_iter().skip(junction) {
        if !path.iter().any(|p| p.rating_key == t.rating_key) {
            path.push(t);
        }
    }
}

impl App {
    pub(super) fn add_stop(&mut self) {
        let Some((key, title, _)) = self.highlighted_track() else {
            if let Some(add) = self.keymap.first_key(self.key_ctx(), &Act::AddStop) {
                self.transient(format!("{add} adds tracks"));
            }
            return;
        };
        if self.adventure.stops.last().is_some_and(|s| s.key == key) {
            self.transient(format!("{title} is already the last stop"));
        } else if self.adventure.stops.len() >= MAX_STOPS {
            self.transient(format!("Sonic Adventure holds {MAX_STOPS} tracks"));
        } else {
            self.adventure.stops.push(Stop { key, title: title.clone() });
            self.transient(format!("stop {}: {title}", self.adventure.stops.len()));
        }
    }

    pub(super) fn build_adventure(&mut self) {
        if self.adventure.stops.len() < 2 {
            let hint = match self.keymap.first_key(self.key_ctx(), &Act::AddStop) {
                Some(add) => format!("add at least 2 tracks with {add}"),
                None => "add at least 2 tracks".into(),
            };
            self.transient(hint);
            return;
        }
        let stops = self.adventure.stops.clone();
        let keys: Vec<&str> = stops.iter().map(|s| s.key.as_str()).collect();
        let key = keys.join(",");
        self.search.request_list(key.clone(), ListKind::Adventure, self.tab);
        let _ = self.plex.tx.send(PlexCmd::BuildAdventure { key, stops });
    }

    /// Also drops a pending build. False when there were no stops.
    pub(super) fn clear_adventure(&mut self) -> bool {
        if self.adventure.stops.is_empty() {
            return false;
        }
        self.adventure.stops.clear();
        if self.search.list_request.as_ref().is_some_and(|r| r.kind == ListKind::Adventure) {
            self.search.clear_list_request();
        }
        self.transient("Sonic Adventure cleared".into());
        true
    }

    pub(super) fn adventure_progress(&mut self, key: &str, leg: usize, legs: usize) {
        let pending = self.search.list_request.as_ref();
        if pending.is_some_and(|r| r.kind == ListKind::Adventure && r.key == key) {
            self.transient(format!("building path {leg}/{legs}…"));
        }
    }

    pub(super) fn adventure_built(&mut self, key: String, outcome: AdventureOutcome) {
        let Some(req) = self.search.take_list_request(&key, |k| k == ListKind::Adventure) else {
            return;
        };
        match outcome {
            AdventureOutcome::Path(tracks) => {
                let (Some(first), Some(last)) = (tracks.first(), tracks.last()) else {
                    self.transient_err("Sonic Adventure failed".into());
                    return;
                };
                let label = format!("Sonic Adventure: {} to {}", first.title, last.title);
                self.push_back_for_detail(req.origin);
                let mut state = TableState::default();
                state.select(Some(0));
                self.set_search_detail(Some(SearchDetail {
                    key,
                    label,
                    tracks,
                    state,
                    matches: None,
                    radio: None,
                }));
                self.search.focused = false;
                self.adventure.stops.clear();
                self.goto_tab(Tab::Search);
            }
            AdventureOutcome::NoSonic => self.transient("sonic analysis needs Plex Pass".into()),
            AdventureOutcome::NoPath { from, to } => {
                self.transient(format!("no path from {from} to {to}"));
            }
            AdventureOutcome::Failed => self.transient_err("Sonic Adventure failed".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::client::testing::serve_full;
    use tokio::sync::mpsc::unbounded_channel;

    fn track_xml(key: &str) -> String {
        format!(
            "<Track ratingKey=\"{key}\" key=\"/library/metadata/{key}\" title=\"T{key}\"><Media><Part id=\"{key}\" key=\"/library/parts/{key}/f.mp3\"/></Media></Track>"
        )
    }

    fn leg_xml(keys: &[&str]) -> String {
        let tracks: String = keys.iter().map(|k| track_xml(k)).collect();
        format!("<MediaContainer>{tracks}</MediaContainer>")
    }

    fn stop(key: &str) -> Stop {
        Stop { key: key.into(), title: format!("T{key}") }
    }

    fn param(line: &str, name: &str) -> String {
        let rest = line.split(&format!("{name}=")).nth(1).unwrap_or_default();
        rest.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or_default().into()
    }

    /// The outcome, progress messages and requests of a build against a fake server.
    async fn built(
        stops: &[&str],
        features: &'static str,
        leg: impl Fn(&str, &str) -> (u16, String) + Send + 'static,
    ) -> (AdventureOutcome, Vec<(usize, usize)>, Vec<String>) {
        let (base, seen, _) = serve_full(move |line| {
            if line.starts_with("GET / ") {
                return (200, format!("<MediaContainer ownerFeatures=\"{features}\"/>"));
            }
            leg(&param(line, "startID"), &param(line, "endID"))
        })
        .await;
        let client = PlexClient::new(&base, "faketoken-adventure");
        let (tx, mut rx) = unbounded_channel();
        let stops: Vec<Stop> = stops.iter().map(|k| stop(k)).collect();
        run(&client, "3", &Mutex::new(None), &tx, "k".into(), stops).await;
        let mut progress = Vec::new();
        let mut outcome = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::AdventureProgress { leg, legs, .. } => progress.push((leg, legs)),
                Msg::AdventureBuilt { outcome: o, .. } => outcome = Some(o),
                _ => panic!("unexpected message"),
            }
        }
        let seen = seen.lock().unwrap();
        let lines = seen.iter().filter(|l| l.contains("computePath")).cloned().collect();
        (outcome.expect("one reply"), progress, lines)
    }

    fn keys(outcome: AdventureOutcome) -> Vec<String> {
        match outcome {
            AdventureOutcome::Path(t) => t.into_iter().map(|t| t.rating_key).collect(),
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn legs_are_asked_in_order_and_stitched_without_repeats() {
        let (outcome, progress, lines) = built(&["1", "2", "3"], "music-analysis", |a, b| {
            match (a, b) {
                ("1", "2") => (200, leg_xml(&["1", "7", "2"])),
                // The junction, then a track the first leg already has, then new ones.
                _ => (200, leg_xml(&["2", "7", "8", "3"])),
            }
        })
        .await;
        assert_eq!(keys(outcome), ["1", "7", "2", "8", "3"]);
        assert_eq!(progress, [(1, 2), (2, 2)]);
        assert!(lines[0].contains("startID=1&endID=2"), "{lines:?}");
        assert!(lines[1].contains("startID=2&endID=3"), "{lines:?}");
        assert_eq!(lines.len(), 2);
    }

    #[tokio::test]
    async fn a_missing_or_empty_leg_is_no_path_and_ends_the_build() {
        for second in [(404, String::new()), (200, "<MediaContainer size=\"0\"/>".into())] {
            let (outcome, progress, lines) =
                built(&["1", "2", "3", "4"], "music-analysis", move |a, _| match a {
                    "1" => (200, leg_xml(&["1", "2"])),
                    _ => second.clone(),
                })
                .await;
            let AdventureOutcome::NoPath { from, to } = outcome else { panic!("{outcome:?}") };
            assert_eq!((from.as_str(), to.as_str()), ("T2", "T3"));
            assert_eq!(progress, [(1, 3), (2, 3)]);
            assert_eq!(lines.len(), 2, "no leg after the failed one: {lines:?}");
        }
    }

    #[tokio::test]
    async fn a_server_error_fails_the_build_and_ends_it() {
        let (outcome, _, lines) =
            built(&["1", "2", "3"], "music-analysis", |_, _| (500, String::new())).await;
        assert!(matches!(outcome, AdventureOutcome::Failed), "{outcome:?}");
        assert_eq!(lines.len(), 1);
    }

    #[tokio::test]
    async fn a_server_without_sonic_analysis_asks_for_no_leg() {
        let (outcome, progress, lines) =
            built(&["1", "2"], "radio", |_, _| (200, leg_xml(&["1", "2"]))).await;
        assert!(matches!(outcome, AdventureOutcome::NoSonic), "{outcome:?}");
        assert!(progress.is_empty() && lines.is_empty());
    }
}
