//! Stamps the commit into `shmc --version`, the only build identity a stripped
//! release binary has. Builds without `.git` too.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // `rerun-if-changed=build.rs` alone would disable cargo's default and freeze
    // the stamp at the first build's commit, so watch the checkout.
    if let Some(git_dir) = git(["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) {
        watch(&git_dir.join("HEAD"));
        if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
            if let Some(reference) = head.trim().strip_prefix("ref: ") {
                // In a worktree the branch ref is in the main repo's `.git`.
                if let Some(path) = git(["rev-parse", "--git-path", reference.trim()]) {
                    watch(&resolve(&path));
                }
            }
        }
    }
    // Packed refs have no loose file.
    if let Some(common) = git(["rev-parse", "--git-common-dir"]) {
        watch(&resolve(&common).join("packed-refs"));
    }
    println!("cargo:rerun-if-env-changed=SHMC_GIT_SHA");
    let sha = std::env::var("SHMC_GIT_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(git_sha)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SHMC_GIT_SHA={sha}");
}

/// A missing path would make cargo re-run the script on every build.
fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// git may print a path relative to the package root.
fn resolve(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map(|cwd| cwd.join(&path)).unwrap_or(path)
    }
}

/// Trimmed stdout, or `None` outside a checkout.
fn git<const N: usize>(args: [&str; N]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn git_sha() -> Option<String> {
    git(["rev-parse", "--short", "HEAD"])
}
