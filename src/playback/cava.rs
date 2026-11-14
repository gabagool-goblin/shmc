//! Runs `cava` in raw 8-bit mode and streams bar heights to the UI. The audio
//! source depends on the platform, see [`input_modes`].

use crate::config::CavaConfig;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, warn};

/// Bars per frame requested from cava (1 byte each); the UI downsamples to fit
/// the pane.
pub const BARS: usize = 96;

pub struct CavaHandle {
    child: tokio::sync::Mutex<Child>,
}

impl CavaHandle {
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
    }
}

pub enum CavaCmd {
    /// Make sure cava is running.
    Play,
    /// Release the tap after `cava.pause_grace_secs`.
    Pause,
    /// Release the tap now.
    Stop,
    Shutdown,
}

pub enum CavaEvent {
    Frame(Vec<u8>),
    Started,
    /// cava could not start or exited on its own; carries a UI hint.
    Stopped(String),
}

#[derive(Clone)]
pub struct CavaControl {
    tx: UnboundedSender<CavaCmd>,
}

impl CavaControl {
    pub fn play(&self) {
        let _ = self.tx.send(CavaCmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(CavaCmd::Pause);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(CavaCmd::Stop);
    }
    pub fn shutdown(&self) {
        let _ = self.tx.send(CavaCmd::Shutdown);
    }
}

/// Owns the cava child. Capturing audio can keep the machine awake, so cava
/// runs only while there is something to draw.
pub fn controller(cfg: CavaConfig) -> (CavaControl, UnboundedReceiver<CavaEvent>) {
    let (cmd_tx, mut cmd_rx) = unbounded_channel::<CavaCmd>();
    let (ev_tx, ev_rx) = unbounded_channel::<CavaEvent>();
    let grace = std::time::Duration::from_secs(cfg.pause_grace_secs);

    tokio::spawn(async move {
        let mut child: Option<CavaHandle> = None;
        let mut frames: Option<UnboundedReceiver<Vec<u8>>> = None;
        let mut stop_at: Option<tokio::time::Instant> = None;

        loop {
            if frames.is_none() {
                match cmd_rx.recv().await {
                    Some(CavaCmd::Play) => {
                        stop_at = None;
                        match spawn(&cfg).await {
                            Ok((c, rx)) => {
                                child = Some(c);
                                frames = Some(rx);
                                let _ = ev_tx.send(CavaEvent::Started);
                            }
                            Err(e) => {
                                let _ = ev_tx.send(CavaEvent::Stopped(one_line(&e)));
                            }
                        }
                    }
                    Some(CavaCmd::Pause | CavaCmd::Stop) => {}
                    Some(CavaCmd::Shutdown) | None => break,
                }
                continue;
            }

            let mut rx = frames.take().expect("checked above");
            let mut keep = true;
            let mut quit = false;
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(CavaCmd::Play) => stop_at = None,
                    Some(CavaCmd::Pause) => {
                        stop_at = Some(tokio::time::Instant::now() + grace);
                    }
                    Some(CavaCmd::Stop) => {
                        keep = false;
                        stop_at = None;
                        if let Some(c) = child.take() {
                            c.shutdown().await;
                        }
                    }
                    Some(CavaCmd::Shutdown) | None => {
                        keep = false;
                        quit = true;
                        stop_at = None;
                        if let Some(c) = child.take() {
                            c.shutdown().await;
                        }
                    }
                },
                frame = rx.recv() => match frame {
                    Some(f) => {
                        let _ = ev_tx.send(CavaEvent::Frame(f));
                    }
                    None => {
                        // cava exited on its own
                        keep = false;
                        stop_at = None;
                        if let Some(c) = child.take() {
                            c.shutdown().await;
                        }
                        let _ = ev_tx.send(CavaEvent::Stopped("visualizer stopped".into()));
                    }
                },
                _ = sleep_until(stop_at), if stop_at.is_some() => {
                    keep = false;
                    stop_at = None;
                    if let Some(c) = child.take() {
                        c.shutdown().await;
                    }
                }
            }
            if keep {
                frames = Some(rx);
            }
            if quit {
                break;
            }
        }
    });

    (CavaControl { tx: cmd_tx }, ev_rx)
}

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

/// A spawn error on one line, for the visualizer pane.
fn one_line(e: &anyhow::Error) -> String {
    let reason: String = e.to_string().split_whitespace().collect::<Vec<_>>().join(" ");
    reason.chars().take(150).collect()
}

const CONFIG_TEMPLATE: &str = "\
[general]
framerate = %FRAMERATE%
autosens = %AUTOSENS%
sensitivity = %SENSITIVITY%
bars = %BARS%
bar_width = 1
bar_spacing = 1
lower_cutoff_freq = %LOWER%
higher_cutoff_freq = %HIGHER%
sleep_timer = 5

[input]
method = %METHOD%
source = \"%SOURCE%\"

[output]
method = raw
orientation = bottom
channels = mono
raw_target = /dev/stdout
data_format = binary
bit_format = 8bit

[smoothing]
noise_reduction = %NOISE%
";

/// (method, source) pairs to try, best first, unless `cava.source` is set.
/// `auto` comes last: cava refuses it without a loopback device.
pub fn input_modes(cfg: &CavaConfig) -> Vec<(&'static str, String)> {
    let override_source = cfg.source.trim();
    if !override_source.is_empty() {
        return if cfg!(target_os = "macos") {
            vec![("coreaudio", override_source.to_string())]
        } else {
            vec![("pipewire", override_source.to_string()), ("pulse", override_source.to_string())]
        };
    }
    if cfg!(target_os = "macos") {
        vec![
            ("coreaudio", "tap".to_string()),
            ("coreaudio", "tap_mono".to_string()),
            ("coreaudio", "auto".to_string()),
        ]
    } else {
        vec![("pipewire", "auto".to_string()), ("pulse", "auto".to_string())]
    }
}

pub fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew install cava"
    } else {
        "sudo dnf install cava (or apt install cava)"
    }
}

/// Resolve a command name against `$PATH`, or check a path, without running it.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|c| c.is_file())
}

pub async fn spawn(cfg: &CavaConfig) -> Result<(CavaHandle, UnboundedReceiver<Vec<u8>>)> {
    if find_on_path(&cfg.path).is_none() {
        anyhow::bail!("cava not found at {}; install it ({})", cfg.path, install_hint());
    }

    let modes = input_modes(cfg);
    let mut problems = Vec::new();
    for (method, source) in &modes {
        match spawn_with(cfg, method, source).await {
            Ok(pair) => {
                info!(method, source, "cava started");
                return Ok(pair);
            }
            Err(e) => {
                warn!("cava {method}/{source} failed: {e:#}");
                problems.push(format!("{e}"));
            }
        }
    }
    anyhow::bail!("cava could not start: {}", problems.join("; "))
}

/// Escape for a TOML basic string: `cava.source` is user input inside quotes,
/// and must not be able to inject keys into the generated config.
fn escape_toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // TOML forbids other raw control characters.
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// `noise_reduction` of a style preset; an unknown style is `balanced`.
pub fn preset_noise_reduction(style: &str) -> i64 {
    match style {
        "smooth" => 90,
        "snappy" => 40,
        _ => 77,
    }
}

/// The cava config for one attempt. Numbers come range-checked from
/// `CavaConfig::settings`; `source` is the only free text, and is escaped.
fn build_config(cfg: &CavaConfig, method: &str, source: &str) -> String {
    let s = cfg.settings();
    CONFIG_TEMPLATE
        .replace("%FRAMERATE%", &cfg.framerate().to_string())
        .replace("%AUTOSENS%", if s.autosens { "1" } else { "0" })
        .replace("%SENSITIVITY%", &s.sensitivity.to_string())
        .replace("%LOWER%", &s.lower_cutoff_hz.to_string())
        .replace("%HIGHER%", &s.higher_cutoff_hz.to_string())
        .replace("%NOISE%", &s.noise_reduction.to_string())
        .replace("%BARS%", &BARS.to_string())
        .replace("%METHOD%", method)
        // Last, so a placeholder inside the source stays literal.
        .replace("%SOURCE%", &escape_toml_string(source))
}

async fn spawn_with(
    cfg: &CavaConfig,
    method: &str,
    source: &str,
) -> Result<(CavaHandle, UnboundedReceiver<Vec<u8>>)> {
    let conf_dir =
        crate::cache::try_ensure_cache_dir().context("making the cache dir private")?.join("cava");
    std::fs::create_dir_all(&conf_dir).context("creating cava config dir")?;
    let conf_path = conf_dir.join("config");
    let conf = build_config(cfg, method, source);
    std::fs::write(&conf_path, conf).context("writing cava config")?;
    let _ = crate::cache::set_private(&conf_path);

    // cava's own stderr is the real reason when it refuses to start.
    let err_path = conf_dir.join("stderr.log");
    let err_file =
        crate::cache::open_private(&err_path, false).context("creating cava stderr log")?;

    let mut child = Command::new(&cfg.path)
        .arg("-p")
        .arg(&conf_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::from(err_file))
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("running {}", cfg.path))?;

    let stdout = child.stdout.take().expect("piped stdout");

    // A bad source makes cava exit at once; catch that before handing it over.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    if let Some(status) = child.try_wait().context("checking cava")? {
        let detail = stderr_excerpt(&err_path);
        let why = if detail.is_empty() {
            format!("cava exited immediately ({status})")
        } else {
            format!("cava exited immediately ({status}): {detail}")
        };
        anyhow::bail!("{why} [method={method} source={source}]");
    }

    let (tx, rx) = unbounded_channel::<Vec<u8>>();
    tokio::spawn(read_frames(stdout, tx));

    Ok((CavaHandle { child: tokio::sync::Mutex::new(child) }, rx))
}

/// The first non-blank lines of cava's stderr, enough to include the hint that
/// follows some errors (the tap-permission notice).
fn stderr_excerpt(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

async fn read_frames(mut stdout: tokio::process::ChildStdout, tx: UnboundedSender<Vec<u8>>) {
    let mut frame = vec![0u8; BARS];
    loop {
        match stdout.read_exact(&mut frame).await {
            Ok(_) => {
                if tx.send(frame.clone()).is_err() {
                    break; // UI gone
                }
            }
            Err(_) => {
                tracing::info!("cava stream ended");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_source(source: &str) -> CavaConfig {
        CavaConfig { source: source.into(), ..CavaConfig::default() }
    }

    #[test]
    fn quoted_cava_source_cannot_inject_toml() {
        let source = "Black\"Hole\n[input]\nmethod = tap";
        let escaped = escape_toml_string(source);
        assert!(!escaped.contains('\n'), "a raw newline would end the line");

        let line = format!("source = \"{escaped}\"");
        let table: toml::Table = toml::from_str(&line).expect("the generated value is valid TOML");
        assert_eq!(table["source"].as_str(), Some(source), "round-trip lost the value: {line}");

        let sections = |s: &str| s.lines().filter(|l| l.trim_start().starts_with('[')).count();
        let benign = build_config(&CavaConfig::default(), "coreaudio", "tap");
        let hostile = build_config(&CavaConfig::default(), "coreaudio", source);
        assert_eq!(
            sections(&hostile),
            sections(&benign),
            "the source injected a section:\n{hostile}"
        );
    }

    fn custom() -> CavaConfig {
        CavaConfig {
            style: "custom".into(),
            noise_reduction: 20,
            autosens: false,
            sensitivity: 150,
            lower_cutoff_hz: 80,
            higher_cutoff_hz: 8000,
            ..CavaConfig::default()
        }
    }

    fn has(conf: &str, line: &str) -> bool {
        conf.lines().any(|l| l == line)
    }

    #[test]
    fn default_config_unchanged() {
        let before = "\
[general]
framerate = 60
autosens = 1
sensitivity = 100
bars = 96
bar_width = 1
bar_spacing = 1
lower_cutoff_freq = 50
higher_cutoff_freq = 10000
sleep_timer = 5

[input]
method = coreaudio
source = \"tap\"

[output]
method = raw
orientation = bottom
channels = mono
raw_target = /dev/stdout
data_format = binary
bit_format = 8bit

[smoothing]
noise_reduction = 77
";
        assert_eq!(build_config(&CavaConfig::default(), "coreaudio", "tap"), before);
    }

    #[test]
    fn preset_sets_smoothing_ignores_raw_keys() {
        for (style, noise) in [("balanced", 77), ("smooth", 90), ("snappy", 40)] {
            // raw keys are ignored outside `custom`
            let cfg = CavaConfig { style: style.into(), ..custom() };
            let conf = build_config(&cfg, "coreaudio", "tap");
            assert!(has(&conf, &format!("noise_reduction = {noise}")), "{style}\n{conf}");
            for line in [
                "autosens = 1",
                "sensitivity = 100",
                "lower_cutoff_freq = 50",
                "higher_cutoff_freq = 10000",
            ] {
                assert!(has(&conf, line), "{style}: {line}\n{conf}");
            }
        }
    }

    #[test]
    fn custom_style_uses_raw_keys() {
        let conf = build_config(&custom(), "coreaudio", "tap");
        for line in [
            "noise_reduction = 20",
            "autosens = 0",
            "sensitivity = 150",
            "lower_cutoff_freq = 80",
            "higher_cutoff_freq = 8000",
        ] {
            assert!(has(&conf, line), "{line}\n{conf}");
        }
    }

    #[test]
    fn out_of_range_raw_value_reported() {
        let bad = [
            CavaConfig { noise_reduction: 101, ..custom() },
            CavaConfig { noise_reduction: -1, ..custom() },
            CavaConfig { sensitivity: 0, ..custom() },
            CavaConfig { lower_cutoff_hz: 5, ..custom() },
            CavaConfig { higher_cutoff_hz: 99999, ..custom() },
            CavaConfig { lower_cutoff_hz: 1000, higher_cutoff_hz: 1000, ..custom() },
        ];
        for cfg in bad {
            assert_eq!(cfg.problems().len(), 1, "{cfg:?}");
            let s = cfg.settings();
            assert!(s.lower_cutoff_hz < s.higher_cutoff_hz);
            assert!((0..=100).contains(&s.noise_reduction));
            assert!((1..=300).contains(&s.sensitivity));
            let conf = build_config(&cfg, "coreaudio", "tap");
            assert!(!conf.contains("99999") && !conf.contains("-1\n"), "{conf}");
        }
        let preset = CavaConfig { style: "smooth".into(), noise_reduction: 500, ..custom() };
        assert!(preset.problems().is_empty());
        assert!(custom().problems().is_empty());
        let odd = CavaConfig { style: "wild".into(), ..CavaConfig::default() };
        assert_eq!(odd.problems().len(), 1);
        assert!(has(&build_config(&odd, "coreaudio", "tap"), "noise_reduction = 77"));
    }

    #[test]
    fn framerate_capped_at_the_ui_rate() {
        let fast = CavaConfig { framerate: 240, ..CavaConfig::default() };
        assert_eq!(fast.problems().len(), 1);
        assert!(has(&build_config(&fast, "coreaudio", "tap"), "framerate = 120"));
    }

    #[test]
    fn control_char_in_cava_source_stays_parseable() {
        let source = "Black\u{1}Hole";
        let escaped = escape_toml_string(source);
        assert!(!escaped.chars().any(|c| c.is_control()), "{escaped:?}");
        let line = format!("source = \"{escaped}\"");
        let table: toml::Table = toml::from_str(&line).expect("valid TOML");
        assert_eq!(table["source"].as_str(), Some(source));
    }

    #[test]
    fn input_modes_start_with_the_platform_default() {
        let modes = input_modes(&CavaConfig::default());
        assert!(!modes.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(modes[0], ("coreaudio", "tap".to_string()));
        } else {
            assert_eq!(modes[0], ("pipewire", "auto".to_string()));
        }
    }

    #[test]
    fn explicit_source_overrides_every_mode() {
        let modes = input_modes(&cfg_with_source("BlackHole 2ch"));
        assert!(!modes.is_empty());
        assert!(modes.iter().all(|(_, s)| s == "BlackHole 2ch"));
    }

    /// An idle player must not keep capturing audio, which keeps the machine
    /// awake.
    #[cfg(unix)]
    #[tokio::test]
    async fn controller_starts_and_stops_cava() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cava");
        std::fs::write(
            &path,
            format!("#!/bin/sh\nwhile true; do printf '{}'; sleep 0.01; done\n", "x".repeat(BARS)),
        )
        .expect("write fake cava");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake cava");

        let cfg = CavaConfig {
            path: path.to_string_lossy().into_owned(),
            framerate: 30,
            source: "test".into(),
            ..CavaConfig::default()
        };
        let (control, mut events) = controller(cfg);

        control.play();
        let mut started = false;
        let mut framed = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(CavaEvent::Started)) => started = true,
                Ok(Some(CavaEvent::Frame(frame))) => {
                    assert_eq!(frame.len(), BARS);
                    framed = true;
                    break;
                }
                Ok(Some(CavaEvent::Stopped(e))) => panic!("cava stopped: {e}"),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert!(started, "cava never reported Started");
        assert!(framed, "no frames arrived");

        control.stop();
        // Drain frames already in flight.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        while events.try_recv().is_ok() {}
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(events.try_recv().is_err(), "frames continued after Stop");

        control.shutdown();
    }
}
