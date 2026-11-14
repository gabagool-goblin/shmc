//! `shmc config --check`: the checks build a [`Report`] that both the text
//! output and the config screen's Diagnostics tab render.

use crate::config::{plain_http_warning, token_label, Config};
use crate::playback::cava;
use crate::plex::PlexClient;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub severity: Severity,
    /// May contain newlines; continuation lines carry their own indentation.
    pub text: String,
}

impl Entry {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { severity: Severity::Ok, text: text.into() }
    }

    pub fn warn(text: impl Into<String>) -> Self {
        Self { severity: Severity::Warn, text: text.into() }
    }

    pub fn fail(text: impl Into<String>) -> Self {
        Self { severity: Severity::Fail, text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The `config` block; only a warning carries a symbol.
    pub settings: Vec<Entry>,
    pub sections: Vec<Section>,
    pub warnings: u32,
    pub failures: u32,
}

impl Report {
    /// The text form of `shmc config --check`; `title` is its first line.
    pub fn render(&self, title: &str) -> String {
        let mut out = String::new();
        out.push_str(title);
        out.push_str("\n\nconfig\n");
        for entry in &self.settings {
            out.push_str(&line(entry, false));
        }
        for section in &self.sections {
            out.push('\n');
            out.push_str(section.title);
            out.push('\n');
            for entry in &section.entries {
                out.push_str(&line(entry, true));
            }
        }
        out.push('\n');
        out.push_str(&summary(self.failures, self.warnings));
        out.push('\n');
        out
    }

    /// The closing line, for the config screen's footer.
    pub fn summary(&self) -> String {
        summary(self.failures, self.warnings)
    }
}

/// The `config` block prints an `Ok` entry without a symbol; a check prints `✔`.
fn line(entry: &Entry, check: bool) -> String {
    let symbol = match entry.severity {
        Severity::Ok if check => "✔ ",
        Severity::Ok => "",
        Severity::Warn => "! ",
        Severity::Fail => "✘ ",
    };
    format!("  {symbol}{}\n", entry.text)
}

pub async fn run(cfg: &Config, config_path: &Path) -> i32 {
    let report = collect(cfg, config_path, true).await;
    print!("{}", report.render("shmc config --check"));
    i32::from(report.failures > 0)
}

pub async fn collect(cfg: &Config, config_path: &Path, with_terminal: bool) -> Report {
    let mut b = Builder::default();

    b.setting(format!("path:   {}", config_path.display()));
    b.setting(format!("plex.server: {}", crate::redact::redact_url(&cfg.plex.server)));
    if let Some(warning) = plain_http_warning(&cfg.plex.server) {
        b.setting_warn(warning);
    }
    // Pasted into bug reports: never any of the token.
    b.setting(format!("plex.token:  {}", token_label(&cfg.plex.token)));
    b.setting(format!("album_art.backend: {}", cfg.album_art.backend));
    b.setting(format!("ui.icons: {} (ascii = {})", cfg.ui.icons, cfg.ui.ascii_icons()));
    b.setting(format!("ui.auto_follow: {}", cfg.ui.auto_follow));
    b.setting(format!("ui.visualizer_pct: {} (of the queue column)", cfg.ui.visualizer_pct));
    b.setting(format!("ui.playlist_numbering: {}", cfg.ui.playlist_numbering));
    b.setting(format!("mpv.path: {}", cfg.mpv.path));
    let themes = crate::ui::themes::available(cfg.theme.dir.as_deref());
    match themes.entries.iter().find(|e| e.id == cfg.theme.name) {
        Some(entry) => b.setting(format!(
            "theme.name: {} ({})",
            entry.id,
            match entry.source {
                crate::ui::themes::Source::Preset => "built in",
                crate::ui::themes::Source::File => "file",
            }
        )),
        None => b.setting(format!("theme.name: {}", cfg.theme.name)),
    }
    // Both lists can carry the chosen theme's problems; say each once.
    let mut problems = crate::ui::theme::appearance_from_config(&cfg.theme).1;
    for problem in themes.problems {
        if !problems.contains(&problem) {
            problems.push(problem);
        }
    }
    for problem in problems {
        b.setting_warn(problem);
    }
    if let Some(problem) = crate::config::column_weights_problem(&cfg.ui.column_weights) {
        b.setting_warn(format!("{problem}; using the default"));
    }
    if let Some(problem) = crate::config::queue_columns_problem(&cfg.ui.queue_columns) {
        b.setting_warn(format!("{problem}; using the default"));
    } else if cfg.ui.queue_columns_all_hidden() {
        b.setting_warn(
            "ui.queue_columns and ui.column_weights leave the Queue without a Title, Artist or \
             Album column; using the default columns"
                .to_string(),
        );
    }
    for problem in cfg.ui.enum_problems() {
        b.setting_warn(problem);
    }
    let (keymap, key_problems) = crate::ui::keymap::Keymap::from_table(&cfg.keys);
    for problem in key_problems {
        b.setting_warn(format!("{problem}; using the default keys"));
    }
    for shadow in keymap.shadows() {
        b.setting(format!(
            "keys.{}.{}: {} shadows global {}",
            shadow.ctx.config_name(),
            shadow.act.name().unwrap_or("?"),
            shadow.key,
            shadow.global.name().unwrap_or("?")
        ));
    }
    for problem in cfg.cava.problems() {
        b.setting_warn(problem);
    }
    let modes: Vec<String> =
        cava::input_modes(&cfg.cava).into_iter().map(|(m, s)| format!("{m}/{s}")).collect();
    b.setting(format!("cava.path: {}", cfg.cava.path));
    b.setting(format!("cava: enabled = {}, will try {}", cfg.cava.enabled, modes.join(" → ")));
    let art = crate::cache::art_dir().map(|dir| crate::cache::dir_stats(&dir)).unwrap_or_default();
    b.setting(format!(
        "art cache: {} files, {} (cap {} MB; 'shmc cache clear' empties it)",
        art.files,
        crate::cache::human_bytes(art.bytes),
        cfg.album_art.cache_mb
    ));

    b.section("plex server");
    let client = PlexClient::new(&cfg.plex.server, &cfg.plex.token);
    match client.identity().await {
        Ok((machine, version)) => {
            b.ok(format!("reachable: Plex Media Server {} ({})", version, short_id(&machine)))
        }
        Err(e) => b.fail(format!("server unreachable or token rejected: {e:#}")),
    }
    match client.music_sections().await {
        Ok(sections) if sections.is_empty() => b.fail("no music library on the server"),
        Ok(sections) => {
            for s in &sections {
                b.ok(format!("music library: \"{}\" (section {})", s.title, s.key));
            }
        }
        Err(e) => b.fail(format!("could not list libraries: {e:#}")),
    }

    b.section("playback");
    match tokio::process::Command::new(&cfg.mpv.path).arg("--version").output().await {
        Ok(out) => {
            let v = String::from_utf8_lossy(&out.stdout);
            b.ok(format!("mpv found: {}", v.lines().next().unwrap_or("?")));
        }
        Err(e) => b.fail(format!("mpv cannot run at {}: {e}", cfg.mpv.path)),
    }
    if cfg.cava.enabled {
        match cava::find_on_path(&cfg.cava.path) {
            None => b.warn(format!(
                "cava not found at {}; the visualizer stays off ({})",
                cfg.cava.path,
                cava::install_hint()
            )),
            Some(p) => {
                b.ok(format!("cava found: {}", p.display()));
                // Start it, so a bad source or missing permission shows up.
                match cava::spawn(&cfg.cava).await {
                    Ok((handle, _frames)) => {
                        handle.shutdown().await;
                        b.ok("cava starts and captures audio");
                    }
                    Err(e) => {
                        let details = format!("{e:#}");
                        let mut text =
                            format!("cava did not start; the visualizer is off:\n      {details}");
                        if details.contains("AudioTap permissions") {
                            text.push_str(
                                "\n      → grant \"System Audio Recording\" to your terminal",
                            );
                            text.push_str(
                                "\n        (System Settings → Privacy & Security → Screen & System Audio Recording),",
                            );
                            text.push_str("\n        then quit and reopen the terminal");
                        }
                        b.warn(text);
                    }
                }
            }
        }
    } else {
        b.warn("visualizer disabled (cava.enabled = false)");
    }

    if with_terminal {
        b.section("terminal");
        match ratatui_image::picker::Picker::from_query_stdio() {
            Ok(p) => b.ok(format!("image protocol: {:?}", p.protocol_type())),
            Err(e) => b.warn(format!("protocol query failed ({e}); album art may be unavailable")),
        }
    }

    b.finish()
}

/// Keeps the tallies in step with the entries, so a check cannot be added
/// without its count moving.
#[derive(Default)]
struct Builder {
    settings: Vec<Entry>,
    sections: Vec<Section>,
    warnings: u32,
    failures: u32,
}

impl Builder {
    fn setting(&mut self, text: impl Into<String>) {
        self.settings.push(Entry::ok(text));
    }

    fn setting_warn(&mut self, text: impl Into<String>) {
        self.warnings += 1;
        self.settings.push(Entry::warn(text));
    }

    fn section(&mut self, title: &'static str) {
        self.sections.push(Section { title, entries: Vec::new() });
    }

    fn ok(&mut self, text: impl Into<String>) {
        self.entries_mut().push(Entry::ok(text));
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.warnings += 1;
        self.entries_mut().push(Entry::warn(text));
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.failures += 1;
        self.entries_mut().push(Entry::fail(text));
    }

    fn entries_mut(&mut self) -> &mut Vec<Entry> {
        &mut self.sections.last_mut().expect("a check must open a section first").entries
    }

    fn finish(self) -> Report {
        Report {
            settings: self.settings,
            sections: self.sections,
            warnings: self.warnings,
            failures: self.failures,
        }
    }
}

/// The first 8 characters of the server's machine id. Not a byte slice: the
/// id comes from the server and may not be ASCII.
pub fn short_id(machine: &str) -> String {
    machine.chars().take(8).collect()
}

/// Warnings are not failures, but "all good" above them would read as
/// "nothing to look at".
fn summary(failures: u32, warnings: u32) -> String {
    if failures > 0 {
        format!("{failures} problem{} found", if failures == 1 { "" } else { "s" })
    } else if warnings > 0 {
        format!("no problems found, {warnings} warning{}", if warnings == 1 { "" } else { "s" })
    } else {
        "no problems found".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_ascii_machine_id_shortens_safely() {
        assert_eq!(short_id("éééééééééé"), "éééééééé");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn token_never_partly_printed() {
        assert_eq!(token_label("abcdef"), "(set)");
        assert_eq!(token_label(""), "(missing)");
        assert_eq!(token_label("éééé"), "(set)");
    }

    /// Pinned: this is what people paste into bug reports.
    #[test]
    fn report_renders_text_output() {
        let report = Report {
            settings: vec![
                Entry::ok("path:   /tmp/shmc.toml"),
                Entry::ok("plex.server: https://plex.example:32400"),
                Entry::warn("server is not https://"),
                Entry::ok("plex.token:  (set)"),
            ],
            sections: vec![
                Section {
                    title: "plex server",
                    entries: vec![
                        Entry::ok("reachable: Plex Media Server 1.0.0 (abcdef12)"),
                        Entry::fail("no music library on the server"),
                    ],
                },
                Section {
                    title: "playback",
                    entries: vec![Entry::warn(
                        "cava did not start; the visualizer is off:\n      boom\n      → do the thing",
                    )],
                },
            ],
            warnings: 2,
            failures: 1,
        };

        assert_eq!(
            report.render("shmc checks"),
            "shmc checks\n\
             \n\
             config\n\
             \x20 path:   /tmp/shmc.toml\n\
             \x20 plex.server: https://plex.example:32400\n\
             \x20 ! server is not https://\n\
             \x20 plex.token:  (set)\n\
             \n\
             plex server\n\
             \x20 ✔ reachable: Plex Media Server 1.0.0 (abcdef12)\n\
             \x20 ✘ no music library on the server\n\
             \n\
             playback\n\
             \x20 ! cava did not start; the visualizer is off:\n\
             \x20     boom\n\
             \x20     → do the thing\n\
             \n\
             1 problem found\n"
        );
    }

    #[test]
    fn builder_counts_recorded_checks() {
        let mut b = Builder::default();
        b.setting("path:   /tmp/x");
        b.setting_warn("something");
        b.section("playback");
        b.ok("mpv found");
        b.warn("visualizer disabled");
        b.fail("cava not found");
        let report = b.finish();
        assert_eq!(report.warnings, 2);
        assert_eq!(report.failures, 1);
        assert_eq!(report.summary(), "1 problem found");
        assert_eq!(summary(0, 0), "no problems found");
        assert_eq!(summary(0, 1), "no problems found, 1 warning");
        assert_eq!(summary(0, 2), "no problems found, 2 warnings");
        assert_eq!(report.sections.len(), 1);
        assert_eq!(report.sections[0].entries.len(), 3);
    }

    /// A theme file that does not load is a warning, not a failure: the player
    /// starts on the default theme.
    #[test]
    fn bad_theme_file_warns() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("good.toml"), "accent = \"#102030\"\n").unwrap();
        std::fs::write(
            tmp.path().join("bad.toml"),
            "secondary = \"nope\"\n[styles]\ncurrent = { bold = \"SECRET\" }\n",
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report = |name: &str| {
            let mut cfg = Config::default();
            cfg.theme.name = name.into();
            cfg.theme.dir = Some(tmp.path().to_path_buf());
            rt.block_on(collect(&cfg, Path::new("/nonexistent/shmc.toml"), false))
        };
        let good = report("good");
        let bad = report("bad");
        let line = |r: &Report| {
            r.settings.iter().find(|e| e.text.starts_with("theme.name")).unwrap().text.clone()
        };
        assert_eq!(line(&good), "theme.name: good (file)");
        assert_eq!(line(&report("nord")), "theme.name: nord (built in)");
        assert_eq!(line(&bad), "theme.name: bad (file)");
        assert_eq!(bad.failures, good.failures, "not a failure");
        // the broken file is said once, also when it is the chosen theme
        let clean =
            rt.block_on(collect(&Config::default(), Path::new("/nonexistent/shmc.toml"), false));
        assert_eq!(good.warnings, clean.warnings + 2, "one per problem");
        assert_eq!(bad.warnings, clean.warnings + 2);
        let text = bad.render("shmc checks");
        assert!(text.contains("bad: secondary") && !text.contains("nope"), "{text}");
        assert!(text.contains("bad: styles.current") && !text.contains("SECRET"), "{text}");
    }

    /// A bad value is a warning naming the key; shmc starts with the default.
    #[test]
    fn bad_visualizer_or_column_value_warns() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report =
            |cfg: &Config| rt.block_on(collect(cfg, Path::new("/nonexistent/shmc.toml"), false));
        let clean = report(&Config::default());
        let mut cfg = Config::default();
        cfg.cava.style = "custom".into();
        cfg.cava.noise_reduction = 500;
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 1);
        let text = bad.render("shmc checks");
        assert!(text.contains("cava.noise_reduction") && !text.contains("500"), "{text}");

        let mut cfg = Config::default();
        cfg.ui.column_weights = vec![0, 0, 0];
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 1);
        assert!(bad.render("shmc checks").contains("ui.column_weights"));

        let mut cfg = Config::default();
        cfg.ui.queue_columns = vec!["title".into(), "title".into()];
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 1);
        assert!(bad.render("shmc checks").contains("ui.queue_columns"));

        let mut cfg = Config::default();
        cfg.ui.visualizer_anchor = "both".into();
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 1);
        assert!(bad.render("shmc checks").contains("ui.visualizer_anchor"));

        let mut cfg = Config::default();
        cfg.ui.header = "SECRETbogus".into();
        let bad = report(&cfg);
        assert_eq!(bad.warnings, clean.warnings + 1);
        let text = bad.render("shmc checks");
        assert!(text.contains("ui.header") && !text.contains("SECRET"), "{text}");
    }

    /// A bad `[keys]` table is a warning naming the key path; the defaults run.
    #[test]
    fn bad_keys_table_warns() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report =
            |cfg: &Config| rt.block_on(collect(cfg, Path::new("/nonexistent/shmc.toml"), false));
        let clean = report(&Config::default());
        let cfg = Config::parse("[keys.queue]\nundo = [\"d\"]\n").unwrap();
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 1);
        let text = bad.render("shmc checks");
        assert!(
            text.contains("keys.queue.undo") && text.contains("using the default keys"),
            "{text}"
        );
    }

    /// A warning naming the key, not the value; shmc uses the theme's own.
    #[test]
    fn bad_spacing_override_warns_naming_key() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let report =
            |cfg: &Config| rt.block_on(collect(cfg, Path::new("/nonexistent/shmc.toml"), false));
        let clean = report(&Config::default());
        let mut cfg = Config::default();
        cfg.theme.block_gap = Some(11);
        cfg.theme.volume_meter = Some("SECRETabc".into());
        cfg.theme.borders = Some("SECRETframes".into());
        let bad = report(&cfg);
        assert_eq!(bad.failures, clean.failures);
        assert_eq!(bad.warnings, clean.warnings + 3);
        let text = bad.render("shmc checks");
        assert!(text.contains("theme.block_gap") && text.contains("theme.volume_meter"), "{text}");
        assert!(text.contains("theme.borders"), "{text}");
        assert!(!text.contains("= 11") && !text.contains("SECRET"), "{text}");
    }
}
