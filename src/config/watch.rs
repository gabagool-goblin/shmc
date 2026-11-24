//! Notices a saved config while the player runs.

use super::Config;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// An atomic save gives the file a new inode.
#[derive(PartialEq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    inode: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp { len: meta.len(), modified: meta.modified().ok(), inode: meta.ino() })
}

/// Calls `changed` with the file read again after each change, until it returns false.
pub fn watch(
    path: PathBuf,
    every: Duration,
    mut changed: impl FnMut(anyhow::Result<Config>) -> bool + Send + 'static,
) {
    let mut last = stamp(&path);
    std::thread::spawn(move || loop {
        std::thread::sleep(every);
        let Some(now) = stamp(&path) else { continue };
        if last.as_ref() == Some(&now) {
            continue;
        }
        last = Some(now);
        if !changed(Config::read(&path)) {
            break;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn a_saved_file_is_read_once_and_an_untouched_one_never() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ui]\nscrolloff = 3\n").unwrap();
        let (tx, rx) = channel();
        watch(path.clone(), Duration::from_millis(20), move |read| {
            tx.send(read.map(|c| c.ui.scrolloff).map_err(|e| e.to_string())).is_ok()
        });
        let quiet = Duration::from_millis(200);
        assert!(rx.recv_timeout(quiet).is_err(), "untouched");

        crate::cache::write_private_atomic(&path, b"[ui]\nscrolloff = 9\n").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(9));
        assert!(rx.recv_timeout(quiet).is_err(), "read once");

        std::fs::write(&path, "[ui]\nscrolloff = \"many\"\n").unwrap();
        let err = rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }
}
