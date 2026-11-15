use super::{Editor, Screen};
use crate::plex::models::Section;
use crate::ui::text_input::TextInput;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

impl Screen {
    /// Looks up with the server and token as the screen holds them (unsaved
    /// edits count), on its own thread so a dead server cannot freeze it.
    pub(super) fn begin_library(&mut self, current: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let lookup = self.library_lookup.clone();
        let (server, token) = (self.cfg.plex.server.clone(), self.cfg.plex.token.clone());
        std::thread::spawn(move || {
            let _ = tx.send(lookup(server, token));
        });
        self.library_rx = Some(rx);
        let mut pick = LibraryPick { libraries: Libraries::Loading, current, index: 0 };
        pick.index = pick.index_of_current();
        self.editor = Some(Editor::Library(pick));
    }

    pub(super) fn poll_library(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.library_rx else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            // The thread panicked.
            Err(TryRecvError::Disconnected) => Err("the lookup stopped unexpectedly".into()),
        };
        self.library_rx = None;
        if let Some(Editor::Library(pick)) = &mut self.editor {
            // Keep the cursor on the same id: rows come and go.
            let was = pick.choices().get(pick.index).cloned();
            pick.libraries = match result {
                Ok(sections) => Libraries::Loaded(sections),
                Err(reason) => Libraries::Failed(reason),
            };
            let at = was.and_then(|was| {
                pick.choices().iter().position(|c| match (c, &was) {
                    (Choice::Id(a, _), Choice::Id(b, _)) => a == b,
                    (Choice::Type, Choice::Type) => true,
                    _ => false,
                })
            });
            pick.index = at.unwrap_or(0);
        }
    }

    pub(super) fn library_key(&mut self, mut pick: LibraryPick, key: KeyEvent) {
        let choices = pick.choices();
        match key.code {
            KeyCode::Esc => {
                self.library_rx = None;
                self.status.clear();
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.library_rx = None;
                self.status.clear();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                pick.index = (pick.index + 1).min(choices.len().saturating_sub(1));
                self.editor = Some(Editor::Library(pick));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                pick.index = pick.index.saturating_sub(1);
                self.editor = Some(Editor::Library(pick));
            }
            KeyCode::Char('t') => self.type_library_id(pick.current),
            KeyCode::Enter => match choices.get(pick.index) {
                Some(Choice::Id(id, _)) => {
                    self.library_rx = None;
                    if let Some(setting) = self.selected() {
                        self.set(setting, id);
                    }
                }
                _ => self.type_library_id(pick.current),
            },
            _ => self.editor = Some(Editor::Library(pick)),
        }
    }

    /// For a server that cannot be asked, or an id the list lacks.
    fn type_library_id(&mut self, current: String) {
        self.library_rx = None;
        self.editor = Some(Editor::Field {
            initial: current.clone(),
            text: TextInput::new(current),
            secret: false,
        });
    }
}

pub(super) enum Libraries {
    Loading,
    Loaded(Vec<Section>),
    /// Short, and free of the token and the URL.
    Failed(String),
}

/// The empty id (the first library found) is always the first choice and
/// "type an id" the last, so an unreachable server never locks the setting.
pub(super) struct LibraryPick {
    pub(super) libraries: Libraries,
    /// What `plex.music_section` holds now.
    current: String,
    /// An index into `choices()`.
    pub(super) index: usize,
}

impl LibraryPick {
    pub(super) fn choices(&self) -> Vec<Choice> {
        let mut out = vec![Choice::Id(String::new(), "first one found".into())];
        let mut known = false;
        if let Libraries::Loaded(sections) = &self.libraries {
            for s in sections {
                known |= s.key == self.current;
                out.push(Choice::Id(s.key.clone(), format!("{}  · id {}", s.title, s.key)));
            }
        }
        if !self.current.is_empty() && !known {
            let note = match self.libraries {
                Libraries::Loaded(_) => "not found",
                _ => "current",
            };
            out.push(Choice::Id(self.current.clone(), format!("{} ({note})", self.current)));
        }
        out.push(Choice::Type);
        out
    }

    fn index_of_current(&self) -> usize {
        let at = self
            .choices()
            .iter()
            .position(|c| matches!(c, Choice::Id(id, _) if *id == self.current));
        at.unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Choice {
    /// The id to store (empty: the first library found), and its label.
    Id(String, String),
    /// Open the text field.
    Type,
}

/// Looks the music libraries up for a server and token; `Err` is a short
/// reason. A field so tests can stand in for the network.
pub(super) type LibraryLookup =
    std::sync::Arc<dyn Fn(String, String) -> Result<Vec<Section>, String> + Send + Sync>;

pub(super) const LIBRARY_TIMEOUT: Duration = Duration::from_secs(5);

/// The failure reason names only the kind: the error carries the request URL,
/// which carries the token.
pub(super) fn fetch_libraries(
    server: &str,
    token: &str,
    timeout: Duration,
) -> Result<Vec<Section>, String> {
    if server.trim().is_empty() {
        return Err("no server set yet".into());
    }
    let rt =
        tokio::runtime::Runtime::new().map_err(|_| "could not start the lookup".to_string())?;
    rt.block_on(async {
        let client = crate::plex::client::PlexClient::new(server, token);
        match tokio::time::timeout(timeout, client.music_sections()).await {
            Err(_) => Err("the server did not answer in time".to_string()),
            Ok(Err(_)) => {
                Err("could not ask the server (unreachable, or the token is refused)".to_string())
            }
            Ok(Ok(sections)) => Ok(sections),
        }
    })
}
