//! Where the config, cache and logs live.

use std::path::{Path, PathBuf};

/// No relative fallback: a path under the working directory could be planted.
fn xdg_dir(var: &str, default: &str) -> Option<PathBuf> {
    resolve_dir(std::env::var(var).ok(), dirs::home_dir(), default)
}

fn resolve_dir(xdg: Option<String>, home: Option<PathBuf>, default: &str) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(default)))
}

pub const NO_HOME: &str = "cannot find a home directory: set HOME";

/// The documented location, the same on every platform.
pub fn config_path() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|dir| dir.join("shmc").join("config.toml"))
}

/// `themes/` beside the config file, so `--config FILE` carries its own themes.
pub fn themes_dir_for(path: &Path) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new("")).join("themes")
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("shmc"))
}

pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library").join("Logs").join("shmc"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        xdg_dir("XDG_STATE_HOME", ".local/state").map(|dir| dir.join("shmc"))
    }
}

/// The log directory, created 0700, or the cache dir if it cannot be made.
pub fn usable_log_dir() -> Option<PathBuf> {
    if let Some(dir) = log_dir() {
        if crate::cache::ensure_private_dir(&dir).is_ok() {
            return Some(dir);
        }
    }
    crate::cache::ensure_cache_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_resolve_without_a_relative_fallback() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(resolve_dir(None, None, ".config"), None);
        assert_eq!(resolve_dir(Some("rel/dir".into()), None, ".config"), None);
        assert_eq!(resolve_dir(Some("/xdg".into()), None, ".config"), Some(PathBuf::from("/xdg")));
        assert_eq!(
            resolve_dir(Some("rel".into()), home, ".config"),
            Some(PathBuf::from("/home/me/.config"))
        );
    }
}
