//! Album art and per-run files; audio is never written to disk.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn dir() -> Option<PathBuf> {
    crate::config::cache_dir()
}

fn no_home() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, crate::config::NO_HOME)
}

/// The cache root, created 0700 (`~/.cache` is often world-readable).
pub fn ensure_cache_dir() -> Option<PathBuf> {
    match try_ensure_cache_dir() {
        Ok(dir) => Some(dir),
        Err(e) => {
            tracing::warn!("cannot make the cache dir private: {e}");
            self::dir()
        }
    }
}

/// [`ensure_cache_dir`], but a directory that cannot be made 0700 is an error.
pub fn try_ensure_cache_dir() -> std::io::Result<PathBuf> {
    let dir = self::dir().ok_or_else(no_home)?;
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// The chmod tightens an existing directory, which keeps its old mode.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// For mpv's socket: mpv uses its own umask, so the 0700 parent keeps others out.
pub fn private_run_dir() -> std::io::Result<PathBuf> {
    let dir = self::dir().ok_or_else(no_home)?.join("run");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// Open a cache file 0600; the chmod tightens one that already existed.
pub fn open_private(path: &Path, append: bool) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    let _ = set_private(path);
    Ok(file)
}

/// Tighten a cache file to 0600, for writers that create it themselves.
pub fn set_private(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Readers, even after a kill mid-write, see the old file or the whole new one.
pub fn write_private_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let name = dest.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = dest.with_file_name(format!(
        ".{name}.{}-{nanos}-{}{TEMP_SUFFIX}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, dest));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A recent temp file is a write in progress; an old one's writer was killed.
const TEMP_SUFFIX: &str = ".tmp";
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(600);

pub fn art_dir() -> Option<PathBuf> {
    dir().map(|dir| dir.join("art"))
}

/// The thumb path is the cache key.
pub fn art_path(thumb_path: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    thumb_path.hash(&mut hasher);
    art_dir().map(|dir| dir.join(format!("{:016x}.img", hasher.finish())))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub files: usize,
    pub bytes: u64,
}

pub fn dir_stats(dir: &Path) -> Stats {
    let mut stats = Stats::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return stats;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            let sub = dir_stats(&entry.path());
            stats.files += sub.files;
            stats.bytes += sub.bytes;
        } else if meta.is_file() {
            stats.files += 1;
            stats.bytes += meta.len();
        }
    }
    stats
}

/// Mark `path` most recently used for eviction.
pub fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Remove all cached art; returns the bytes freed.
pub fn clear_art() -> std::io::Result<u64> {
    clear_dir(&art_dir().ok_or_else(no_home)?)
}

fn clear_dir(path: &Path) -> std::io::Result<u64> {
    let freed = dir_stats(path).bytes;
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(freed),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Delete least recently used art until `dir` fits in `cap_mb` MiB (0 clears it).
pub fn prune_art_at(dir: &Path, cap_mb: u64) {
    // Saturating: a wrapped product would be a tiny cap and empty the cache.
    let cap = cap_mb.saturating_mul(1024 * 1024);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if entry.file_name().to_string_lossy().ends_with(TEMP_SUFFIX) {
            let age = meta.modified().ok().and_then(|m| m.elapsed().ok());
            if age.is_some_and(|a| a > STALE_TEMP_AGE) {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        total += meta.len();
        files.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), entry.path()));
    }
    if total <= cap {
        return;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shmc-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn human_bytes_uses_sensible_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn prune_drops_files_until_under_the_cap() {
        let dir = scratch("prune");
        for name in ["old.img", "mid.img", "new.img"] {
            std::fs::write(dir.join(name), vec![0u8; 1024 * 1024]).unwrap();
        }
        prune_art_at(&dir, 2);
        let stats = dir_stats(&dir);
        assert!(stats.bytes <= 2 * 1024 * 1024, "bytes={}", stats.bytes);
        assert_eq!(stats.files, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_noop_under_cap() {
        let dir = scratch("noop");
        std::fs::write(dir.join("only.img"), vec![0u8; 1024]).unwrap();
        prune_art_at(&dir, 1);
        assert_eq!(dir_stats(&dir).files, 1);
        // A cap too big to express in bytes is no cap, not a wrapped tiny one.
        prune_art_at(&dir, 1 << 44);
        assert_eq!(dir_stats(&dir).files, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_is_private_leaves_no_temp() {
        let dir = scratch("atomic");
        let dest = dir.join("cover.img");
        std::fs::write(&dest, b"old").unwrap();

        // The reader must only ever see a complete payload.
        let payloads: Vec<Vec<u8>> = (0..4u8).map(|i| vec![i; 512 * 1024]).collect();
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let got = std::fs::read(&dest).unwrap();
                    assert!(
                        got == b"old"
                            || (got.len() == 512 * 1024 && got.iter().all(|b| *b == got[0])),
                        "partial file seen: {} bytes",
                        got.len()
                    );
                }
            });
            let writers: Vec<_> = payloads
                .iter()
                .map(|p| {
                    let dest = &dest;
                    s.spawn(move || {
                        for _ in 0..10 {
                            write_private_atomic(dest, p).unwrap();
                        }
                    })
                })
                .collect();
            for w in writers {
                w.join().unwrap();
            }
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            reader.join().unwrap();
        });

        assert!(payloads.contains(&std::fs::read(&dest).unwrap()));
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["cover.img"], "temp file left behind");
        #[cfg(unix)]
        assert_eq!(mode(&dest), 0o600);

        // A failing rename (the destination is a directory) cleans up too.
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_private_atomic(&blocked, b"x").is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_ignores_a_temp_file_being_written() {
        let dir = scratch("prune-tmp");
        std::fs::write(dir.join(".a.img.1-2-3.tmp"), vec![0u8; 1024 * 1024]).unwrap();
        std::fs::write(dir.join("b.img"), vec![0u8; 1024]).unwrap();
        prune_art_at(&dir, 0);
        assert!(dir.join(".a.img.1-2-3.tmp").exists(), "in-progress temp evicted");
        assert!(!dir.join("b.img").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn clear_dir_reports_a_failed_removal() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let art = tmp.path().join("art");
        let locked = art.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("a.img"), b"x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = clear_dir(&art);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(clear_dir(&tmp.path().join("missing")).unwrap(), 0);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn cache_dir_private_and_old_one_tightened() {
        let tmp = tempfile::tempdir().unwrap();

        let fresh = tmp.path().join("fresh").join("shmc");
        ensure_private_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700, "new cache dir must be private");

        // made 0755 by something else
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700, "existing cache dir must be tightened");
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_inside_cache_and_private() {
        let run = private_run_dir().expect("create the run dir");
        assert_eq!(Some(run.clone()), dir().map(|dir| dir.join("run")));
        assert_eq!(mode(&run), 0o700, "the mpv socket's directory must be private");
    }

    #[cfg(unix)]
    #[test]
    fn existing_cache_files_made_private() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("shmc.log");

        let mut fresh = open_private(&path, true).unwrap();
        std::io::Write::write_all(&mut fresh, b"new\n").unwrap();
        assert_eq!(mode(&path), 0o600, "new cache file must be private");
        drop(fresh);

        // a log left at 0644
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut old = open_private(&path, true).unwrap();
        std::io::Write::write_all(&mut old, b"appended\n").unwrap();
        assert_eq!(mode(&path), 0o600, "existing cache file must be tightened");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "new\nappended\n", "append must not truncate");

        set_private(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
    }
}
