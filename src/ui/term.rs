//! The terminal both the player and the config screen draw into, with one
//! setup and one panic-safe restore.

use anyhow::{Context, Result};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};

/// True between `setup` and `restore`, so a signal handler does not write
/// escape codes into a pipe.
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

pub fn setup() -> Result<Term> {
    crossterm::terminal::enable_raw_mode().context("enabling raw mode; is this a terminal?")?;
    let mut out = stdout();
    // Bracketed paste, so a paste cannot trigger bindings like `/`.
    if let Err(e) = execute!(out, crossterm::terminal::EnterAlternateScreen, EnableBracketedPaste) {
        restore_raw();
        return Err(e.into());
    }
    ACTIVE.store(true, Ordering::SeqCst);
    match Terminal::new(CrosstermBackend::new(out)) {
        Ok(term) => Ok(term),
        Err(e) => {
            restore_if_active();
            Err(e.into())
        }
    }
}

/// [`restore_raw`] if `setup` has run and `restore` has not.
pub fn restore_if_active() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        restore_raw();
    }
}

/// Without the `Term` handle, which neither the panic hook nor a signal handler can borrow.
fn restore_raw() {
    let _ = crossterm::terminal::disable_raw_mode();
    let mut out = stdout();
    let _ = execute!(
        out,
        DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

/// A panic on the keyboard thread takes the UI down too.
pub const KEYBOARD_THREAD: &str = "shmc-keys";

/// A tokio worker can panic while the app carries on; restoring then breaks a live UI.
fn panic_ends_the_ui(thread_name: Option<&str>) -> bool {
    matches!(thread_name, Some("main" | KEYBOARD_THREAD))
}

/// Only while the UI is up: after `restore` stdout may be a pipe.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if panic_ends_the_ui(std::thread::current().name()) {
            restore_if_active();
        }
        previous(info);
    }));
}

pub fn restore(terminal: &mut Term) -> Result<()> {
    ACTIVE.store(false, Ordering::SeqCst);
    crossterm::terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ui_ending_panic_restores_terminal() {
        assert!(panic_ends_the_ui(Some("main")));
        assert!(panic_ends_the_ui(Some(KEYBOARD_THREAD)));
        assert!(!panic_ends_the_ui(Some("tokio-runtime-worker")));
        assert!(!panic_ends_the_ui(None));
    }
}
