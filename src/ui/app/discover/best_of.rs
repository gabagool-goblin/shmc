use super::{DiscoverCmd, DiscoverMsg, ItemsPane, Pane, Row};
use crate::plex::discover::{BestOf, ShelfItems};
use crate::plex::PlexClient;
use crate::ui::app::{App, Msg, PlexCmd};
use crate::ui::when::{local, now_unix};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Default)]
enum Fetch {
    #[default]
    Idle,
    Loading,
    Ready {
        years: Vec<u32>,
        decades: Vec<u32>,
    },
    Failed(String),
}

#[derive(Default)]
pub struct Choices {
    generation: u64,
    fetch: Fetch,
}

impl Choices {
    pub fn reset(&mut self) {
        *self = Self { generation: self.generation + 1, fetch: Fetch::Idle };
    }

    pub fn message(&self) -> String {
        match &self.fetch {
            Fetch::Failed(msg) => msg.clone(),
            Fetch::Ready { .. } => "nothing found".into(),
            _ => "loading best of…".into(),
        }
    }

    fn items(&self) -> Option<Vec<BestOf>> {
        let Fetch::Ready { years, decades } = &self.fetch else { return None };
        let this_decade = local(now_unix()).map_or(0, |now| now.year as u32 / 10 * 10);
        let years = years.iter().filter(|y| **y >= this_decade).map(|y| BestOf::Year(*y));
        Some(years.chain(decades.iter().map(|d| BestOf::Decade(*d))).collect())
    }
}

pub(super) async fn load(
    client: &PlexClient,
    section: &str,
    tx: &UnboundedSender<Msg>,
    generation: u64,
) {
    let outcome =
        client.best_of_choices(section).await.map_err(|e| format!("loading best of: {e:#}"));
    let _ = tx.send(Msg::Discover(DiscoverMsg::BestOf { generation, outcome }));
}

impl App {
    pub(super) fn follow_best_of(&mut self) {
        let choices = &mut self.discover.best_of;
        if matches!(choices.fetch, Fetch::Failed(_)) {
            choices.fetch = Fetch::Idle;
        }
        if matches!(choices.fetch, Fetch::Idle) {
            choices.fetch = Fetch::Loading;
            let cmd = DiscoverCmd::LoadBestOf { generation: choices.generation };
            let _ = self.plex.tx.send(PlexCmd::Discover(cmd));
        }
        self.show_best_of();
    }

    fn show_best_of(&mut self) {
        self.discover.pane = match self.discover.best_of.items() {
            Some(items) => {
                let mut pane = ItemsPane {
                    shelf: "Best of".into(),
                    name: String::new(),
                    items: ShelfItems::BestOf(items),
                    state: Default::default(),
                    plays: false,
                };
                pane.state.select(Some(0));
                Pane::Items(pane)
            }
            None => Pane::Empty,
        };
    }

    pub(super) fn best_of_loaded(
        &mut self,
        generation: u64,
        outcome: Result<(Vec<u32>, Vec<u32>), String>,
    ) {
        let choices = &mut self.discover.best_of;
        if generation != choices.generation || !matches!(choices.fetch, Fetch::Loading) {
            return;
        }
        choices.fetch = match outcome {
            Ok((years, decades)) => Fetch::Ready { years, decades },
            Err(msg) => Fetch::Failed(msg),
        };
        if self.discover.followed == Some(Row::BestOf) {
            self.show_best_of();
            self.settle_focus();
        }
    }
}
