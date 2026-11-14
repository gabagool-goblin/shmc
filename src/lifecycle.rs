//! How a run ends: why the UI loop stopped, what the process says and exits
//! with, and what a signal does.

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;

/// Why the UI loop returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEnd {
    Quit,
    MpvExited,
}

impl UiEnd {
    /// A quit wins over an mpv that was already gone.
    pub fn from_state(should_quit: bool, mpv_exited: bool) -> Self {
        if !should_quit && mpv_exited {
            UiEnd::MpvExited
        } else {
            UiEnd::Quit
        }
    }
}

/// What the process does once the terminal is restored and mpv is shut down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Clean,
    MpvDied,
    /// Same shutdown as a quit, then exit 128 + the signal number.
    Signal(i32),
}

impl Outcome {
    pub fn of(end: UiEnd, signal: Option<i32>) -> Self {
        match (signal, end) {
            (Some(n), _) => Outcome::Signal(n),
            (None, UiEnd::Quit) => Outcome::Clean,
            (None, UiEnd::MpvExited) => Outcome::MpvDied,
        }
    }

    /// For stderr, once the terminal is back.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Outcome::Clean | Outcome::Signal(_) => None,
            Outcome::MpvDied => Some("shmc: mpv exited unexpectedly (see 'shmc log')"),
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Outcome::Clean => 0,
            Outcome::MpvDied => 1,
            Outcome::Signal(n) => 128 + n,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// Nothing to shut down yet (setup, config screen, checks).
    ExitNow,
    /// Same road out as `q`, so mpv and cava are shut down.
    Quit,
}

pub fn signal_action(orderly: bool) -> SignalAction {
    if orderly {
        SignalAction::Quit
    } else {
        SignalAction::ExitNow
    }
}

/// Set once `main` is about to start mpv.
static ORDERLY: AtomicBool = AtomicBool::new(false);

pub fn begin_orderly() {
    ORDERLY.store(true, Ordering::SeqCst);
}

/// Without a handler mpv outlives shmc. Raw-mode Ctrl-C is a key, not SIGINT.
/// Call inside the runtime.
pub fn install_signal_handlers() -> std::io::Result<watch::Receiver<Option<i32>>> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut term = signal(SignalKind::terminate())?;
    let mut hup = signal(SignalKind::hangup())?;
    let mut int = signal(SignalKind::interrupt())?;
    let (tx, rx) = watch::channel(None);
    tokio::spawn(async move {
        loop {
            let n = tokio::select! {
                _ = term.recv() => libc::SIGTERM,
                _ = hup.recv() => libc::SIGHUP,
                _ = int.recv() => libc::SIGINT,
            };
            match signal_action(ORDERLY.load(Ordering::SeqCst)) {
                SignalAction::ExitNow => {
                    crate::ui::term::restore_if_active();
                    std::process::exit(Outcome::Signal(n).exit_code());
                }
                SignalAction::Quit => {
                    let _ = tx.send(Some(n));
                }
            }
        }
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dying_mpv_exits_non_zero() {
        let died = Outcome::of(UiEnd::from_state(false, true), None);
        assert_ne!(died.exit_code(), 0);
        assert!(died.message().is_some());

        for (quit, mpv_gone) in [(true, false), (true, true)] {
            let outcome = Outcome::of(UiEnd::from_state(quit, mpv_gone), None);
            assert_eq!(outcome.exit_code(), 0);
            assert!(outcome.message().is_none());
        }
    }

    #[test]
    fn signal_before_mpv_exits_at_once() {
        assert_eq!(signal_action(false), SignalAction::ExitNow);
        assert_eq!(signal_action(true), SignalAction::Quit);

        let outcome = Outcome::of(UiEnd::Quit, Some(libc::SIGTERM));
        assert_eq!(outcome.exit_code(), 128 + libc::SIGTERM);
        assert!(outcome.message().is_none());
    }
}
