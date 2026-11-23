use super::{DiscoverView, Row};
use crate::plex::discover::LibraryList;
use crate::ui::app::App;
use crate::ui::period::{Period, PeriodKind};
use crate::ui::when::now_unix;

impl DiscoverView {
    /// `None` unless the pane is a Top list itself.
    pub fn period_line(&self) -> Option<PeriodKind> {
        let Some(Row::Library(list)) = self.followed else { return None };
        (list.is_top() && self.back.is_empty()).then_some(self.library.period.kind)
    }
}

impl App {
    pub(in crate::ui::app) fn cycle_period(&mut self) {
        self.set_period(self.discover.library.period.next_kind());
    }

    pub(in crate::ui::app) fn period_earlier(&mut self) {
        self.set_period(self.discover.library.period.earlier());
    }

    pub(in crate::ui::app) fn period_later(&mut self) {
        if let Some(period) = self.discover.library.period.later() {
            self.set_period(period);
        }
    }

    fn set_period(&mut self, period: Period) {
        let Some(Row::Library(followed)) = self.discover.followed else { return };
        if self.discover.period_line().is_none() {
            return;
        }
        let lib = &mut self.discover.library;
        if period == lib.period {
            return;
        }
        lib.period = period;
        LibraryList::TOP.into_iter().for_each(|list| lib.reset_list(list));
        self.discover.back.clear();
        self.discover.pending = None;
        self.drop_visual(crate::ui::app::ListRef::DTracks);
        self.follow_library(followed, now_unix());
        self.settle_focus();
    }
}
