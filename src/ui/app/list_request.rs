//! The one request whose reply may still open the Search tab's list.

use super::{SearchState, SimilarKind, Tab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Album,
    Similar(SimilarKind),
    Radio,
    Adventure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRequest {
    pub key: String,
    pub kind: ListKind,
    /// The tab the request was made in.
    pub origin: Tab,
}

impl SearchState {
    pub(super) fn request_list(&mut self, key: String, kind: ListKind, origin: Tab) {
        self.list_request = Some(ListRequest { key, kind, origin });
    }

    pub(super) fn clear_list_request(&mut self) {
        self.list_request = None;
    }

    /// Takes the pending request if it is for `key` and of a kind `wanted` accepts.
    pub(super) fn take_list_request(
        &mut self,
        key: &str,
        wanted: impl Fn(ListKind) -> bool,
    ) -> Option<ListRequest> {
        self.list_request.take_if(|r| r.key == key && wanted(r.kind))
    }
}
