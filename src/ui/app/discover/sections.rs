use super::{DiscoverCol, DiscoverLoad, ItemsPane, Pane};
use crate::plex::discover::DiscoverHubs;
use crate::ui::app::{App, ListRef, Tab};
use crate::ui::scroll;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Heading(&'static str),
    Blank,
    Shelf(usize),
    Station(usize),
}

impl Row {
    pub fn selectable(self) -> bool {
        matches!(self, Row::Shelf(_) | Row::Station(_))
    }
}

pub(super) fn build_rows(shelves: usize, stations: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    if shelves > 0 {
        rows.push(Row::Heading("Suggested"));
        rows.extend((0..shelves).map(Row::Shelf));
    }
    if stations > 0 {
        if shelves > 0 {
            rows.push(Row::Blank);
        }
        rows.push(Row::Heading("Stations"));
        rows.extend((0..stations).map(Row::Station));
    }
    rows
}

impl App {
    pub(super) fn discover_loaded(&mut self, hubs: DiscoverHubs, radio: bool) {
        self.drop_visual(ListRef::DTracks);
        let d = &mut self.discover;
        d.rows = build_rows(hubs.shelves.len(), hubs.stations.len());
        d.shelves = hubs.shelves;
        d.stations = hubs.stations;
        d.radio = radio;
        d.sections_state.select(d.rows.iter().position(|r| r.selectable()));
        d.load = DiscoverLoad::Loaded;
        d.followed = None;
        d.pane = Pane::Empty;
        d.back.clear();
        d.pending = None;
        d.focus = DiscoverCol::Sections;
    }

    pub(super) fn highlighted_row(&self) -> Option<Row> {
        let real = self.real_at(ListRef::DSections)?;
        self.discover.rows.get(real).copied()
    }

    fn settle(&self, from: i64, dir: i32) -> Option<usize> {
        let rows = &self.discover.rows;
        let mut i = from;
        while i >= 0 && (i as usize) < rows.len() {
            if rows[i as usize].selectable() {
                return Some(i as usize);
            }
            i += dir as i64;
        }
        None
    }

    fn select_row(&mut self, row: Option<usize>) {
        if row.is_some() {
            self.discover.sections_state.select(row);
        }
    }

    pub(in crate::ui::app) fn discover_step(&mut self, delta: i32) {
        let Some(cur) = self.discover.sections_state.selected() else { return };
        let row = self.settle(cur as i64 + delta as i64, delta);
        if row.is_some() {
            return self.select_row(row);
        }
        self.discover_edge(delta > 0);
    }

    pub(in crate::ui::app) fn discover_page(&mut self, dir: i32, visible: usize) {
        let len = self.discover.rows.len();
        let Some(cur) = self.discover.sections_state.selected() else { return };
        let target = scroll::page_jump(cur, visible, len, dir) as i64;
        let row = self.settle(target, dir).or_else(|| self.settle(target, -dir));
        self.select_row(row);
    }

    pub(in crate::ui::app) fn discover_edge(&mut self, top: bool) {
        let row = if top {
            self.settle(0, 1)
        } else {
            self.settle(self.discover.rows.len() as i64 - 1, -1)
        };
        self.select_row(row);
    }

    pub(in crate::ui::app) fn follow_discover(&mut self) {
        if self.tab != Tab::Discover {
            return;
        }
        let Some(row) = self.highlighted_row() else { return };
        if self.discover.followed == Some(row) {
            return;
        }
        self.discover.followed = Some(row);
        self.discover.back.clear();
        self.discover.pending = None;
        self.drop_visual(ListRef::DTracks);
        match row {
            Row::Shelf(i) => self.show_shelf(i),
            Row::Station(i) => self.follow_station(i),
            Row::Heading(_) | Row::Blank => self.discover.pane = Pane::Empty,
        }
        self.settle_focus();
    }

    fn show_shelf(&mut self, i: usize) {
        let Some(shelf) = self.discover.shelves.get(i) else { return };
        let mut pane = ItemsPane {
            shelf: shelf.title.clone(),
            name: String::new(),
            items: shelf.items.clone(),
            state: Default::default(),
        };
        pane.state.select(Some(0));
        self.discover.pane = Pane::Items(pane);
    }

    pub(super) fn settle_focus(&mut self) {
        if self.discover.pane.is_empty() {
            self.discover.focus = DiscoverCol::Sections;
        }
    }
}
