use super::SelState;
use crate::ui::filter::ListFilter;
use ratatui::widgets::ListState;

/// A list's rows with its cursor and filter.
pub struct FilteredList<T, S = ListState> {
    pub items: Vec<T>,
    pub state: S,
    pub filter: ListFilter,
}

impl<T, S: Default> Default for FilteredList<T, S> {
    fn default() -> Self {
        Self { items: Vec::new(), state: S::default(), filter: ListFilter::default() }
    }
}

impl<T, S: SelState> FilteredList<T, S> {
    /// Sets the rows, selects nothing and clears the filter.
    pub fn replace(&mut self, items: Vec<T>) {
        self.items = items;
        self.state.select(None);
        self.filter.clear();
    }

    pub fn clear(&mut self) {
        self.replace(Vec::new());
    }
}
