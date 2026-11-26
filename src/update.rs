//! `shmc update`: replaces the running binary with the latest GitHub release.

use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

const REPO: &str = "gabagool-goblin/shmc";

/// The targets of the release workflow's build matrix.
const TARGETS: &[&str] =
    &["aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

const MAX_API_BYTES: u64 = 1024 * 1024;
const MAX_SUMS_BYTES: u64 = 64 * 1024;
const MAX_TARBALL_BYTES: u64 = 64 * 1024 * 1024;

fn release_target() -> Option<&'static str> {
    option_env!("SHMC_RELEASE_TARGET").filter(|t| TARGETS.contains(t))
}

pub fn run(check_only: bool) -> Result<()> {
    let target = release_target();
    if !check_only && target.is_none() {
        bail!(
            "this shmc was not built by the release workflow, so 'shmc update' cannot replace \
             it; update it the way it was installed"
        );
    }
    let current = env!("CARGO_PKG_VERSION");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .user_agent(format!("shmc/{current}"))
            .build()
            .context("building the HTTP client")?;
        let tag = latest_tag(&client).await?;
        if !is_newer(&tag, current) {
            println!("shmc {current} is the latest release");
            return Ok(());
        }
        let Some(target) = target else {
            println!(
                "shmc {tag} is available (this is {current}); update it the way it was installed"
            );
            return Ok(());
        };
        if check_only {
            println!("shmc {tag} is available (this is {current}); run 'shmc update'");
            return Ok(());
        }

        let exe = std::env::current_exe().context("finding the running binary")?;
        let name = format!("shmc-{target}.tar.gz");
        println!("downloading {name} ({tag})");
        let base = format!("https://github.com/{REPO}/releases/download/{tag}");
        let sums =
            get(&client, &format!("{base}/SHA256SUMS"), MAX_SUMS_BYTES, "text/plain").await?;
        let tarball =
            get(&client, &format!("{base}/{name}"), MAX_TARBALL_BYTES, "application/octet-stream")
                .await?;
        let sums = String::from_utf8(sums).context("SHA256SUMS is not text")?;
        install(&tarball, &sums, target, &tag, &exe)?;
        println!("updated {} from {current} to {tag}", exe.display());
        println!("note: a running player keeps the old version until it restarts");
        Ok(())
    })
}

async fn latest_tag(client: &reqwest::Client) -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = get(client, &url, MAX_API_BYTES, "application/vnd.github+json").await?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).context("reading the latest release from GitHub")?;
    let tag = json.get("tag_name").and_then(|t| t.as_str()).unwrap_or_default();
    if parse_version(tag).is_none() {
        bail!("the latest release has an unexpected tag");
    }
    Ok(tag.to_string())
}

async fn get(client: &reqwest::Client, url: &str, max_bytes: u64, accept: &str) -> Result<Vec<u8>> {
    let mut resp = client
        .get(url)
        .header(reqwest::header::ACCEPT, accept)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    if !resp.status().is_success() {
        bail!("{url}: HTTP {}", resp.status());
    }
    let too_big = || anyhow!("{url}: response is larger than {} KiB", max_bytes / 1024);
    if resp.content_length().is_some_and(|n| n > max_bytes) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.with_context(|| format!("reading {url}"))? {
        if body.len() as u64 + chunk.len() as u64 > max_bytes {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Only `X.Y.Z` in digits: the tag goes into URLs.
fn parse_version(tag: &str) -> Option<[u64; 3]> {
    let mut fields = tag.split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        let field = fields.next()?;
        if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = field.parse().ok()?;
    }
    fields.next().is_none().then_some(out)
}

fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

fn expected_hash<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let (hash, file) = (fields.next()?, fields.next()?);
        let file = file.strip_prefix('*').unwrap_or(file);
        (fields.next().is_none() && file == name).then_some(hash)
    })
}

struct TempDir(PathBuf);

impl TempDir {
    fn create_in(parent: &Path) -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = parent.join(format!(".shmc-update-{}-{nanos}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow!("{} is not writable", parent.display())
            } else {
                anyhow!(e).context(format!("creating a folder in {}", parent.display()))
            }
        })?;
        Ok(Self(path))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Replaces the file behind `exe`, which is left untouched on any error.
fn install(tarball: &[u8], sums: &str, target: &str, tag: &str, exe: &Path) -> Result<()> {
    let name = format!("shmc-{target}.tar.gz");
    let want =
        expected_hash(sums, &name).ok_or_else(|| anyhow!("SHA256SUMS has no entry for {name}"))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, tarball);
    let got: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    if !got.eq_ignore_ascii_case(want) {
        bail!("{name} does not match its checksum in SHA256SUMS");
    }

    let real = std::fs::canonicalize(exe)
        .with_context(|| format!("finding the file behind {}", exe.display()))?;
    let dir = real.parent().ok_or_else(|| anyhow!("{} has no folder", real.display()))?;
    // The same folder as the binary, so the final rename stays on one filesystem.
    let tmp = TempDir::create_in(dir)?;
    let archive = tmp.0.join(&name);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&archive)
        .and_then(|mut f| f.write_all(tarball))
        .with_context(|| format!("writing {}", archive.display()))?;

    let status = Command::new("tar")
        .arg("--no-same-owner")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&tmp.0)
        .status()
        .context("running 'tar'")?;
    if !status.success() {
        bail!("'tar' could not unpack {name}");
    }

    let new = tmp.0.join(format!("shmc-{target}")).join("shmc");
    let is_file = std::fs::symlink_metadata(&new).is_ok_and(|m| m.file_type().is_file());
    if !is_file {
        bail!("{name} does not contain shmc-{target}/shmc");
    }
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("setting permissions on {}", new.display()))?;

    let out = Command::new(&new).arg("--version").output().context("running the new shmc")?;
    if !out.status.success() {
        bail!("the downloaded shmc does not run here");
    }
    if !String::from_utf8_lossy(&out.stdout).starts_with(&format!("shmc {tag} ")) {
        bail!("the downloaded shmc does not report version {tag}");
    }

    std::fs::File::open(&new)
        .and_then(|f| f.sync_all())
        .with_context(|| format!("writing {}", new.display()))?;
    std::fs::rename(&new, &real).with_context(|| format!("replacing {}", real.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "x86_64-unknown-linux-gnu";

    fn tarball(dir: &Path, script: &str, inner: &str) -> Vec<u8> {
        let src = dir.join("src");
        std::fs::create_dir_all(src.join(inner)).unwrap();
        std::fs::write(src.join(inner).join("shmc"), script).unwrap();
        let out = dir.join("out.tar.gz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg(inner)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read(out).unwrap()
    }

    fn sums_for(bytes: &[u8]) -> String {
        let d = ring::digest::digest(&ring::digest::SHA256, bytes);
        let hex: String = d.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        format!("{hex}  shmc-{TARGET}.tar.gz\n")
    }

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let work = tempfile::tempdir().unwrap();
        let bin = work.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let exe = bin.join("shmc");
        std::fs::write(&exe, "old").unwrap();
        (work, exe)
    }

    fn only_exe_left(exe: &Path) -> bool {
        std::fs::read_dir(exe.parent().unwrap()).unwrap().count() == 1
    }

    const GOOD: &str = "#!/bin/sh\necho 'shmc 9.9.9 (abc1234)'\n";

    #[test]
    fn good_update_replaces_the_binary() {
        let (work, exe) = setup();
        let tar = tarball(work.path(), GOOD, &format!("shmc-{TARGET}"));
        install(&tar, &sums_for(&tar), TARGET, "9.9.9", &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), GOOD);
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert!(only_exe_left(&exe));
    }

    #[test]
    fn wrong_hash_leaves_the_binary() {
        let (work, exe) = setup();
        let tar = tarball(work.path(), GOOD, &format!("shmc-{TARGET}"));
        let sums = format!("{}  shmc-{TARGET}.tar.gz\n", "0".repeat(64));
        assert!(install(&tar, &sums, TARGET, "9.9.9", &exe).is_err());
        assert!(install(&tar, "", TARGET, "9.9.9", &exe).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(only_exe_left(&exe));
    }

    #[test]
    fn bad_contents_leave_the_binary() {
        let (work, exe) = setup();
        let misplaced = tarball(work.path(), GOOD, "other");
        let r = install(&misplaced, &sums_for(&misplaced), TARGET, "9.9.9", &exe);
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(only_exe_left(&exe));

        let tar = tarball(work.path(), GOOD, &format!("shmc-{TARGET}"));
        assert!(install(&tar, &sums_for(&tar), TARGET, "1.0.0", &exe).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(only_exe_left(&exe));
    }

    #[test]
    fn symlinked_binary_has_its_target_replaced() {
        let (work, exe) = setup();
        let link = work.path().join("link");
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        let tar = tarball(work.path(), GOOD, &format!("shmc-{TARGET}"));
        install(&tar, &sums_for(&tar), TARGET, "9.9.9", &link).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), GOOD);
    }

    #[test]
    fn tags_are_plain_versions_compared_by_field() {
        for bad in ["v1.2.3", "1.2", "1.2.3/../x", "1.2.3.4", "1..3", ""] {
            assert!(parse_version(bad).is_none(), "{bad}");
        }
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.9.9", "0.9.9"));
        assert!(!is_newer("0.3.1", "0.10.0"));
    }

    #[test]
    fn checksum_line_matches_the_whole_file_name() {
        let sums = "aa  other-shmc-x.tar.gz\nbb  shmc-x.tar.gz.sig\ncc  shmc-x.tar.gz\n";
        assert_eq!(expected_hash(sums, "shmc-x.tar.gz"), Some("cc"));
        assert_eq!(expected_hash("aa  other-shmc-x.tar.gz\n", "shmc-x.tar.gz"), None);
    }
}
