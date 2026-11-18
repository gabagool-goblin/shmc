use anyhow::{bail, Context, Result};
use shmc::lifecycle::{self, Outcome, UiEnd};
use shmc::{cache, config, doctor, playback, plex, redact, setup, ui};
use std::path::PathBuf;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tracing_subscriber::EnvFilter;
use ui::app::{spawn_plex_worker, PlexHandle, Tab};
use ui::{App, Msg};

mod cli;

#[derive(Debug, PartialEq, Eq)]
enum ConfigMode {
    Edit,
    Check,
    List,
}

/// What a bare argument asks for.
#[derive(Debug, PartialEq, Eq)]
enum BareCommand {
    Player,
    Setup,
}

fn bare_command(args: &[String]) -> BareCommand {
    if args.iter().any(|a| a == "setup") {
        BareCommand::Setup
    } else {
        BareCommand::Player
    }
}

/// `--config FILE` or `--config=FILE`; the last one wins.
fn config_arg(args: &[String]) -> Option<PathBuf> {
    let i = args.iter().rposition(|a| a == "--config" || a.starts_with("--config="))?;
    let arg = &args[i];
    if let Some(value) = arg.strip_prefix("--config=") {
        return (!value.is_empty()).then(|| PathBuf::from(value));
    }
    args.get(i + 1).map(PathBuf::from)
}

/// `args` without `--config FILE` / `--config=FILE`, so a file named `setup`
/// or `log` is not read as the command of that name. Mirrors `cli::validate`.
fn command_args(args: &[String]) -> Vec<String> {
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--config" {
            it.next();
        } else if !arg.starts_with("--config=") {
            rest.push(arg.clone());
        }
    }
    rest
}

fn config_mode(args: &[String]) -> Option<ConfigMode> {
    if !args.iter().any(|a| a == "config") {
        return None;
    }
    Some(if args.iter().any(|a| a == "--check") {
        ConfigMode::Check
    } else if args.iter().any(|a| a == "--list") {
        ConfigMode::List
    } else {
        ConfigMode::Edit
    })
}

fn main() {
    if let Err(e) = run() {
        eprintln!("shmc: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let explicit = config_arg(&args);
    let config_path = explicit.or_else(config::config_path);

    if args.iter().any(|a| matches!(a.as_str(), "--version" | "-V" | "-v")) {
        println!("shmc {} ({})", env!("CARGO_PKG_VERSION"), env!("SHMC_GIT_SHA"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help(config_path.as_deref());
        return Ok(());
    }

    // Refuse stray words and typos like `shmc confg` before any side effect.
    if let Err(msg) = cli::validate(&args) {
        eprintln!("{msg}");
        std::process::exit(2);
    }
    let args = command_args(&args);

    if let Some(pos) = args.iter().position(|a| a == "cache") {
        return cache_command(&args[pos + 1..]);
    }
    if let Some(pos) = args.iter().position(|a| a == "log") {
        return log_command(&args[pos + 1..]);
    }

    let Some(config_path) = config_path else {
        bail!(config::NO_HOME);
    };
    if config::cache_dir().is_none() || config::usable_log_dir().is_none() {
        bail!(config::NO_HOME);
    }

    init_logging();
    let rt = tokio::runtime::Runtime::new().context("tokio runtime")?;
    // Installed before anything needs cleaning up; see `lifecycle`.
    let mut signals = {
        let _enter = rt.enter();
        lifecycle::install_signal_handlers().context("installing signal handlers")?
    };

    // Without a TTY there is nothing to edit, so `shmc config` acts like `--check`.
    if let Some(mode) = config_mode(&args) {
        let mut cfg = read_config_or_default(&config_path)?;
        cfg.set_themes_dir(&config_path);
        match mode {
            ConfigMode::Check => {
                let code = rt.block_on(doctor::run(&cfg, &config_path));
                std::process::exit(code);
            }
            ConfigMode::List => {
                // For bug reports and scripts: no TTY needed, never writes.
                return to_stdout(|out| out.write_all(cfg.settings_list().as_bytes()));
            }
            ConfigMode::Edit => {
                if !setup::can_prompt() {
                    let code = rt.block_on(doctor::run(&cfg, &config_path));
                    std::process::exit(code);
                }
                let (appearance, problems) = ui::theme::appearance_from_config(&cfg.theme);
                ui::theme::set_appearance(appearance, cfg.ui.ascii_icons());
                for problem in problems {
                    eprintln!("note: {problem}");
                }
                ui::term::install_panic_hook();
                let mut terminal = ui::term::setup()?;
                let mut screen = ui::config_screen::Screen::new(config_path.clone(), cfg);
                let result = screen.run(&mut terminal);
                ui::term::restore(&mut terminal)?;
                result?;
                return Ok(());
            }
        }
    }

    let mut cfg = load_or_setup(&rt, &config_path, &args)?;
    cfg.set_themes_dir(&config_path);
    let (appearance, problems) = ui::theme::appearance_from_config(&cfg.theme);
    ui::theme::set_appearance(appearance, cfg.ui.ascii_icons());
    for problem in problems {
        eprintln!("note: {problem}");
    }
    for problem in ui::keymap::Keymap::from_table(&cfg.keys).1 {
        eprintln!("note: {problem}; using the default keys");
    }

    if bare_command(&args) == BareCommand::Setup {
        println!();
        println!("configuration saved; run 'shmc' to start, or 'shmc config --check' to check it");
        return Ok(());
    }

    // A signal during startup abandons it; dropping kills an mpv already
    // spawned (`kill_on_drop`).
    lifecycle::begin_orderly();
    let mut client_id_failed = false;
    let client_id = if cfg.plex.report_playback {
        let dir = config_path.parent().unwrap_or_else(|| std::path::Path::new("."));
        match plex::client_id::load_or_create(dir) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!("playback reports are off: cannot store the client id: {e}");
                client_id_failed = true;
                None
            }
        }
    } else {
        None
    };
    let started = rt.block_on(async {
        tokio::select! {
            started = start(cfg, client_id) => Some(started),
            _ = signals.wait_for(|sig| sig.is_some()) => None,
        }
    });
    let Some(started) = started else {
        let n = (*signals.borrow()).unwrap_or(0);
        std::process::exit(Outcome::Signal(n).exit_code());
    };
    let mut started = started?;
    if client_id_failed {
        let _ = started
            .msg_tx
            .send(Msg::Error("playback reports are off: cannot store the client id".into()));
    }

    // A signal from here on is the same as pressing `q`.
    {
        let tx = started.msg_tx.clone();
        let mut signals = signals.clone();
        rt.spawn(async move {
            if signals.wait_for(|sig| sig.is_some()).await.is_ok() {
                let _ = tx.send(Msg::Quit);
            }
        });
    }

    if let Some(arg) = &started.mpv.ignored_setting {
        eprintln!(
            "note: ignoring mpv.extra_args entry {arg:?}: a custom mpv log file would \
             receive the unredacted Plex token"
        );
    }

    // Whatever happens to the terminal, mpv and cava are shut down below.
    let result = (|| {
        ui::term::install_panic_hook();
        let mut terminal = ui::term::setup()?;
        let result = ui_loop(&mut terminal, &mut started);
        ui::term::restore(&mut terminal)?;
        result
    })();

    rt.block_on(started.mpv.shutdown());
    if let Some(reporter) = started.reporter.take() {
        rt.block_on(reporter.finish(std::time::Duration::from_secs(2)));
    }
    if let Some(cava) = started.cava.take() {
        cava.shutdown();
    }
    // Let the controller reap cava before the runtime drops.
    rt.block_on(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });

    let outcome = Outcome::of(result?, *signals.borrow());
    if let Some(message) = outcome.message() {
        eprintln!("{message}");
    }
    if outcome != Outcome::Clean {
        std::process::exit(outcome.exit_code());
    }
    Ok(())
}

fn read_config_or_default(path: &PathBuf) -> Result<config::Config> {
    if path.exists() {
        config::Config::read(path)
    } else {
        Ok(config::Config::default())
    }
}

/// Runs interactive setup when the config is missing, incomplete or `shmc setup` was asked for.
fn load_or_setup(
    rt: &tokio::runtime::Runtime,
    config_path: &PathBuf,
    args: &[String],
) -> Result<config::Config> {
    let wants_setup = bare_command(args) == BareCommand::Setup;

    let existing =
        if config_path.exists() { Some(config::Config::read(config_path)?) } else { None };
    let incomplete = existing.as_ref().is_none_or(|c| c.needs_setup());

    if wants_setup || incomplete {
        if !setup::can_prompt() {
            if wants_setup {
                bail!("'shmc setup' needs an interactive terminal");
            }
            bail!("{}", setup::hint(config_path));
        }
        return rt.block_on(setup::interactive(config_path, existing));
    }

    Ok(existing.expect("config exists"))
}

fn print_help(config_path: Option<&std::path::Path>) {
    println!(
        "shmc {} — terminal music player for Plex\n\
         \n\
         USAGE:\n\
         \x20 shmc                 start the player\n\
         \x20 shmc config          edit settings (runs checks first)\n\
         \x20 shmc config --check  check config, server, mpv, cava and terminal\n\
         \x20 shmc config --list   print every setting and its default\n\
         \x20 shmc setup           (re)write the config interactively\n\
         \x20 shmc cache           show the cache size and log location\n\
         \x20 shmc cache clear     delete cached album art\n\
         \x20 shmc log [N]         print the last N lines of the logs, 60 by default\n\
         \x20 shmc --version       print version\n\
         \x20 shmc --config FILE   use a different config file\n\
         \n\
         Config: {}",
        env!("CARGO_PKG_VERSION"),
        config_path.map_or_else(|| config::NO_HOME.into(), |p| p.display().to_string())
    );
}

fn cache_command(rest: &[String]) -> Result<()> {
    let (Some(root), Some(logs)) = (config::cache_dir(), config::usable_log_dir()) else {
        bail!(config::NO_HOME);
    };
    println!("shmc cache");
    println!("  path: {}", root.display());

    if rest.iter().any(|a| a == "clear") {
        let freed = cache::clear_art().context("clearing the album art cache")?;
        println!("  cleared album art, freed {}", cache::human_bytes(freed));
        return Ok(());
    }

    let art = cache::dir_stats(&root.join("art"));
    let log = std::fs::metadata(logs.join("shmc.log")).map(|m| m.len()).unwrap_or(0);
    let mpv_log = std::fs::metadata(logs.join("mpv.log")).map(|m| m.len()).unwrap_or(0);
    let total = cache::dir_stats(&root);
    println!("  album art: {} files, {}", art.files, cache::human_bytes(art.bytes));
    println!("  total:     {} files, {}", total.files, cache::human_bytes(total.bytes));
    println!("  logs:      {}", logs.display());
    println!(
        "             {} (shmc.log {} + mpv.log {})",
        cache::human_bytes(log + mpv_log),
        cache::human_bytes(log),
        cache::human_bytes(mpv_log)
    );
    println!();
    println!("  'shmc cache clear' empties it");
    Ok(())
}

fn log_command(rest: &[String]) -> Result<()> {
    let lines: usize = rest.iter().filter_map(|a| a.parse().ok()).next().unwrap_or(60);
    let Some(root) = config::usable_log_dir() else {
        bail!(config::NO_HOME);
    };
    to_stdout(|out| {
        for name in ["shmc.log", "mpv.log"] {
            let path = root.join(name);
            writeln!(out, "== {} ({}) ==", path.display(), name)?;
            match log_tail(&path, lines) {
                Ok(tail) => {
                    for line in &tail {
                        writeln!(out, "{line}")?;
                    }
                }
                Err(e) => writeln!(out, "  (cannot read: {e})")?,
            }
            writeln!(out)?;
        }
        Ok(())
    })
}

/// A reader that stops early (`shmc log | head`) ends the output; it is not an error.
fn to_stdout(write: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<()> {
    match write(&mut std::io::stdout().lock()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        result => result.map_err(Into::into),
    }
}

/// Read lossily: mpv can log a non-UTF-8 file name.
fn log_tail(path: &std::path::Path, lines: usize) -> std::io::Result<Vec<String>> {
    let text = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
    // Redacted on write too; this covers a line that slipped past.
    let text = redact::redact_secrets(&text);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].iter().map(|l| (*l).to_string()).collect())
}

/// Rotate the log so it cannot grow without bound (keeps one previous file).
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// Redacts every write: `shmc.log` is pasted into bug reports.
struct RedactedLog(std::fs::File);

impl std::io::Write for RedactedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let text = redact::redact_secrets(&text);
        // One event, one line; only the formatter's trailing newline stays.
        let (body, tail) = match text.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (text.as_str(), ""),
        };
        let escaped = redact::escape_field(body);
        std::io::Write::write_all(&mut self.0, escaped.as_bytes())?;
        std::io::Write::write_all(&mut self.0, tail.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.0)
    }
}

fn init_logging() {
    let Some(log_path) = config::usable_log_dir().map(|dir| dir.join("shmc.log")) else {
        return;
    };
    if std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let rotated = log_path.with_extension("log.1");
        let _ = std::fs::rename(&log_path, &rotated);
        let _ = cache::set_private(&rotated);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(concat!(env!("CARGO_CRATE_NAME"), "=info"))),
        )
        .with_writer(move || {
            let file = cache::open_private(&log_path, true)
                .unwrap_or_else(|_| std::fs::File::create("/dev/null").expect("/dev/null"));
            RedactedLog(file)
        })
        .with_ansi(false)
        .init();
}

struct Started {
    cfg: config::Config,
    msg_rx: tokio::sync::mpsc::UnboundedReceiver<Msg>,
    /// back-channel for non-worker senders (transient status timers)
    msg_tx: UnboundedSender<Msg>,
    plex: PlexHandle,
    mpv: playback::MpvHandle,
    cava: Option<playback::cava::CavaControl>,
    reporter: Option<plex::report::Reporter>,
    picker: Option<ratatui_image::picker::Picker>,
    art_enabled: bool,
}

async fn start(cfg: config::Config, client_id: Option<String>) -> Result<Started> {
    let client = plex::PlexClient::new(&cfg.plex.server, &cfg.plex.token);

    let section = if !cfg.plex.music_section.is_empty() {
        cfg.plex.music_section.clone()
    } else {
        let sections = client
            .music_sections()
            .await
            .context("cannot reach Plex; check plex.server and plex.token in the config, or run 'shmc config --check'")?;
        match sections.first() {
            Some(s) => {
                tracing::info!(
                    section = %redact::escape_field(&s.title),
                    key = %s.key,
                    "using music library"
                );
                s.key.clone()
            }
            None => bail!("no music library on {}; add one in Plex first", cfg.plex.server),
        }
    };

    let mut picker = ui::image_picker();
    let mut art_enabled = cfg.album_art.backend != "none";
    if let Some(p) = picker.as_mut() {
        use ratatui_image::picker::ProtocolType;
        match cfg.album_art.backend.as_str() {
            "kitty" => p.set_protocol_type(ProtocolType::Kitty),
            "iterm2" => p.set_protocol_type(ProtocolType::Iterm2),
            "sixel" => p.set_protocol_type(ProtocolType::Sixel),
            "halfblocks" => p.set_protocol_type(ProtocolType::Halfblocks),
            _ => {}
        }
        tracing::info!(
            protocol = ?p.protocol_type(),
            cell = ?p.font_size(),
            "terminal image protocol"
        );
    } else if art_enabled {
        tracing::warn!("terminal reports no image protocol; album art disabled");
        art_enabled = false;
    }

    let (mpv, mut mpv_events) = playback::mpv::spawn(&cfg.mpv).await.context("starting mpv")?;

    let (msg_tx, msg_rx) = unbounded_channel::<Msg>();

    {
        let fwd_tx = msg_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = mpv_events.recv().await {
                if fwd_tx.send(Msg::Mpv(ev)).is_err() {
                    break;
                }
            }
        });
    }

    let reporter = client_id.map(|id| {
        let tx = msg_tx.clone();
        plex::report::Reporter::spawn(client.clone(), id, move |note| {
            let _ = tx.send(Msg::Error(note));
        })
    });

    let plex_tx = spawn_plex_worker(
        client,
        section,
        msg_tx.clone(),
        cfg.album_art.cache_mb,
        cfg.album_art.max_size_px,
    );
    let _ = plex_tx.send(ui::app::PlexCmd::LoadArtists);
    let _ = plex_tx.send(ui::app::PlexCmd::LoadPlaylists);

    // Started on demand so capturing audio doesn't keep the machine awake.
    let cava = if cfg.cava.enabled && cfg.ui.visualizer_pct > 0 {
        let (control, mut events) = playback::cava::controller(cfg.cava.clone());
        // Frames wait for playback; keep the pane from reading "off".
        let _ = msg_tx.send(Msg::CavaReady);
        let fwd_tx = msg_tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                let msg = match ev {
                    playback::cava::CavaEvent::Frame(frame) => Msg::Cava(frame),
                    playback::cava::CavaEvent::Started => Msg::CavaReady,
                    playback::cava::CavaEvent::Stopped(hint) => Msg::CavaDead(hint),
                };
                if fwd_tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Some(control)
    } else {
        None
    };

    spawn_keyboard_thread(msg_tx.clone());

    Ok(Started {
        cfg,
        msg_rx,
        msg_tx,
        plex: PlexHandle { tx: plex_tx },
        mpv,
        cava,
        reporter,
        picker,
        art_enabled,
    })
}

/// Only keys, pastes and resizes are input; anything else would be
/// misclassified as a key.
fn ui_msg(ev: crossterm::event::Event) -> Option<Msg> {
    use crossterm::event::Event;

    match ev {
        Event::Key(key) => Some(Msg::Key(Event::Key(key))),
        Event::Paste(text) => Some(Msg::Paste(text)),
        // The loop paints only on a message.
        Event::Resize(..) => Some(Msg::Resize),
        _ => None,
    }
}

fn spawn_keyboard_thread(tx: UnboundedSender<Msg>) {
    // Without input the player cannot be used or quit, so a panic here ends it.
    struct QuitIfPanicking(UnboundedSender<Msg>);
    impl Drop for QuitIfPanicking {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let _ = self.0.send(Msg::Quit);
            }
        }
    }

    std::thread::Builder::new()
        .name(ui::term::KEYBOARD_THREAD.into())
        .spawn(move || {
            let guard = QuitIfPanicking(tx.clone());
            loop {
                let ev = match crossterm::event::read() {
                    Ok(ev) => ev,
                    Err(e) => {
                        tracing::error!("reading the keyboard failed: {e}");
                        let _ = tx.send(Msg::Quit);
                        break;
                    }
                };
                let Some(msg) = ui_msg(ev) else { continue };
                if tx.send(msg).is_err() {
                    break;
                }
            }
            drop(guard);
        })
        .expect("spawning the keyboard thread");
}

/// Redraw cap with the visualizer off; with it on, the cava framerate sets it.
const DEFAULT_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

/// With cava off the pane would be an empty box, so it gets no height.
fn ui_settings(cfg: &config::Config) -> config::UiConfig {
    let mut ui = cfg.ui.clone();
    if !cfg.cava.enabled {
        ui.visualizer_pct = 0;
    }
    ui
}

fn frame_interval(cava_enabled: bool, visualizer_pct: u16, framerate: u32) -> std::time::Duration {
    if !cava_enabled || visualizer_pct == 0 {
        return DEFAULT_FRAME_INTERVAL;
    }
    let fps = u64::from(framerate.max(1));
    std::time::Duration::from_micros(1_000_000 / fps)
}
/// The bar has one cell per ~3 s of a song and the text is whole seconds.
const POS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
/// A draw longer than this is logged (a cover payload can hold the frame).
const SLOW_DRAW: std::time::Duration = std::time::Duration::from_millis(150);

/// Slow, and the last such line is over a second old.
fn slow_draw(
    elapsed: std::time::Duration,
    last_logged: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    elapsed > SLOW_DRAW
        && last_logged.is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_secs(1))
}

/// Bounds one coalescing burst so a fast producer cannot starve painting.
const MAX_COALESCE: usize = 1024;

/// What arrived since the last paint.
#[derive(Default)]
struct Redraw {
    /// any message other than a cava frame or a clock update
    state: bool,
    cava: bool,
    pos: bool,
    shutdown: bool,
}

fn note_msg(msg: &Msg, redraw: &mut Redraw) {
    match msg {
        Msg::Cava(_) => redraw.cava = true,
        Msg::Mpv(playback::MpvEvent::Property { name, .. }) if name == "time-pos" => {
            redraw.pos = true;
        }
        _ => redraw.state = true,
    }
}

fn apply_msg(msg: Msg, app: &mut App, redraw: &mut Redraw) {
    note_msg(&msg, redraw);
    redraw.shutdown |= matches!(&msg, Msg::Mpv(playback::MpvEvent::Shutdown));
    app.handle_msg(msg);
    redraw.shutdown |= app.should_quit;
}

fn wants_redraw(redraw: &Redraw, viz_on: bool, pos_due: bool) -> bool {
    redraw.state || (redraw.cava && viz_on) || (redraw.pos && pos_due)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CavaWanted {
    Play,
    Pause,
    Stop,
}

/// Its tap keeps the machine from idle-sleeping, so run it only when visible.
fn cava_wanted(app: &App) -> CavaWanted {
    if app.stopped {
        CavaWanted::Stop
    } else if app.paused || app.tab != Tab::Queue || app.visualizer_pct == 0 {
        CavaWanted::Pause
    } else {
        CavaWanted::Play
    }
}

/// The UI loop runs on the bare main thread (no tokio context) so that
/// `blocking_recv` is legal.
fn ui_loop(terminal: &mut ui::term::Term, started: &mut Started) -> Result<UiEnd> {
    let cfg = &started.cfg;
    let mut app = App::new(
        cfg.plex.server.trim_end_matches('/').to_string(),
        cfg.plex.token.clone(),
        PlexHandle { tx: started.plex.tx.clone() },
        started.mpv.clone(),
        cfg.mpv.volume,
        started.picker.take(),
        started.art_enabled,
        cfg.album_art.max_size_px,
        started.msg_tx.clone(),
        ui_settings(cfg),
        &cfg.keys,
    );
    app.allow_writes = cfg.plex.allow_writes;
    app.reporter = started.reporter.as_ref().map(|r| r.sender());

    let cava = started.cava.clone();
    let mut cava_state = cava_wanted(&app);
    let frame_budget =
        frame_interval(cfg.cava.enabled, cfg.ui.visualizer_pct, cfg.cava.framerate());
    let mut last_draw =
        std::time::Instant::now().checked_sub(frame_budget).unwrap_or_else(std::time::Instant::now);

    let mut last_slow_log: Option<std::time::Instant> = None;

    while let Some(msg) = started.msg_rx.blocking_recv() {
        let mut redraw = Redraw::default();
        apply_msg(msg, &mut app, &mut redraw);
        for _ in 0..MAX_COALESCE {
            match started.msg_rx.try_recv() {
                Ok(msg) => apply_msg(msg, &mut app, &mut redraw),
                Err(_) => break,
            }
        }
        if redraw.shutdown {
            break;
        }

        let want = cava_wanted(&app);
        if want != cava_state {
            cava_state = want;
            if let Some(c) = &cava {
                match want {
                    CavaWanted::Play => c.play(),
                    CavaWanted::Pause => c.pause(),
                    CavaWanted::Stop => c.stop(),
                }
            }
        }

        let viz_on = app.tab == Tab::Queue && app.visualizer_pct > 0;
        let pos_due = last_draw.elapsed() >= POS_INTERVAL;
        if !wants_redraw(&redraw, viz_on, pos_due) {
            continue;
        }

        // Wait out the frame budget, folding in what arrives so a burst paints once.
        let elapsed = last_draw.elapsed();
        if elapsed < frame_budget {
            std::thread::sleep(frame_budget - elapsed);
            for _ in 0..MAX_COALESCE {
                match started.msg_rx.try_recv() {
                    Ok(msg) => apply_msg(msg, &mut app, &mut redraw),
                    Err(_) => break,
                }
            }
            if redraw.shutdown {
                break;
            }
        }

        app.art.emitted = false;
        let drawn = std::time::Instant::now();
        terminal.draw(|f| ui::views::draw(f, &mut app))?;
        last_draw = std::time::Instant::now();
        let took = last_draw.duration_since(drawn);
        if slow_draw(took, last_slow_log, last_draw) {
            last_slow_log = Some(last_draw);
            let px = match (app.art.prepared_rect, app.picker.as_ref()) {
                (Some(r), Some(p)) => {
                    let (cw, ch) = p.font_size();
                    format!(
                        "{}x{}",
                        u32::from(r.width) * u32::from(cw),
                        u32::from(r.height) * u32::from(ch)
                    )
                }
                _ => "none".to_string(),
            };
            tracing::info!(
                "slow draw: {} ms, tab {:?}, cover sent {}, cover area {} px",
                took.as_millis(),
                app.tab,
                app.art.emitted,
                px
            );
        }
    }
    app.finish_reporting();
    Ok(UiEnd::from_state(app.should_quit, app.mpv_exited))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cava_off_gives_visualizer_no_height() {
        let mut cfg = config::Config::default();
        assert!(cfg.cava.enabled && cfg.ui.visualizer_pct > 0);
        assert_eq!(super::ui_settings(&cfg).visualizer_pct, cfg.ui.visualizer_pct);
        cfg.cava.enabled = false;
        assert_eq!(super::ui_settings(&cfg).visualizer_pct, 0);
    }

    use super::*;

    #[test]
    fn slow_draws_are_logged_once_a_second() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let slow = Duration::from_millis(151);
        assert!(!slow_draw(Duration::from_millis(150), None, t0));
        assert!(slow_draw(slow, None, t0));
        assert!(!slow_draw(slow, Some(t0), t0 + Duration::from_millis(900)));
        assert!(slow_draw(slow, Some(t0), t0 + Duration::from_secs(1)));
    }

    #[test]
    fn log_writer_redacts_tokens() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shmc.log");
        let mut log = RedactedLog(std::fs::File::create(&path).expect("create log"));
        writeln!(log, "GET http://plex/x?X-Plex-Token=SECRETTOKEN123").expect("write");
        writeln!(log, "plain line").expect("write");
        log.flush().expect("flush");

        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(!text.contains("SECRETTOKEN123"), "{text}");
        assert!(text.contains("X-Plex-Token=<redacted>"), "{text}");
        assert!(text.contains("plain line"), "{text}");
        assert_eq!(text.lines().count(), 2, "one event per line: {text}");
    }

    #[test]
    fn shmc_log_handles_invalid_utf8_and_redacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("mpv.log");
        let mut bytes = b"opening \xff\xfe.flac\n".to_vec();
        bytes.extend_from_slice(b"GET http://plex/x?X-Plex-Token=SECRETTOKEN123\n");
        std::fs::write(&path, bytes).expect("write");

        let tail = log_tail(&path, 10).expect("an odd byte must not hide the log");
        assert_eq!(tail.len(), 2);
        let all = tail.join("\n");
        assert!(!all.contains("SECRETTOKEN123"), "{all}");
        assert!(all.contains("opening"), "{all}");
    }

    #[test]
    fn newline_in_log_message_cannot_forge_line() {
        use std::io::Write;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shmc.log");
        let mut log = RedactedLog(std::fs::File::create(&path).expect("create log"));
        writeln!(log, "track=Night\nMix level=warn forged").expect("write");
        log.flush().expect("flush");

        let text = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("Night\\nMix"), "{text}");
        assert!(text.ends_with('\n'), "the event's own newline stays: {text:?}");
    }

    #[test]
    fn only_resize_events_reach_ui() {
        use crossterm::event::{Event, KeyCode, KeyEvent, MouseEvent, MouseEventKind};

        assert!(matches!(ui_msg(Event::Resize(80, 24)), Some(Msg::Resize)));
        let mouse = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 1,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(ui_msg(Event::Mouse(mouse)).is_none());
        assert!(ui_msg(Event::FocusGained).is_none());
        assert!(matches!(
            ui_msg(Event::Key(KeyEvent::from(KeyCode::Char('j')))),
            Some(Msg::Key(_))
        ));
        assert!(matches!(ui_msg(Event::Paste("x".into())), Some(Msg::Paste(_))));
    }

    #[test]
    fn resize_wants_redraw() {
        let mut redraw = Redraw::default();
        note_msg(&Msg::Resize, &mut redraw);
        assert!(wants_redraw(&redraw, false, false));
    }

    #[test]
    fn cava_frames_repaint_only_when_visualizer_visible() {
        let cava = Redraw { cava: true, ..Redraw::default() };
        assert!(wants_redraw(&cava, true, false));
        assert!(!wants_redraw(&cava, false, false));
    }

    #[test]
    fn clock_updates_are_throttled() {
        let pos = Redraw { pos: true, ..Redraw::default() };
        assert!(wants_redraw(&pos, false, true));
        assert!(!wants_redraw(&pos, false, false));
    }

    #[test]
    fn state_changes_always_repaint() {
        let state = Redraw { state: true, ..Redraw::default() };
        assert!(wants_redraw(&state, false, false));
        assert!(wants_redraw(&state, true, true));
    }

    #[test]
    fn config_path_accepts_both_spellings_last_wins() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(config_arg(&args(&[])), None);
        assert_eq!(
            config_arg(&args(&["--config", "a.toml", "config"])),
            Some(PathBuf::from("a.toml"))
        );
        assert_eq!(
            config_arg(&args(&["--config=a.toml", "config"])),
            Some(PathBuf::from("a.toml"))
        );
        assert_eq!(
            config_arg(&args(&["--config", "a.toml", "--config", "b.toml", "config"])),
            Some(PathBuf::from("b.toml"))
        );
        // A value that looks like a command is still a value.
        assert_eq!(config_arg(&args(&["--config", "config"])), Some(PathBuf::from("config")));
        assert_eq!(config_arg(&args(&["--config="])), None);
        assert_eq!(config_arg(&args(&["--config"])), None);
    }

    #[test]
    fn config_file_named_like_command_ok() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for name in ["setup", "config", "cache", "log"] {
            let rest = command_args(&args(&["--config", name]));
            assert_eq!(bare_command(&rest), BareCommand::Player, "{name}");
            assert_eq!(config_mode(&rest), None, "{name}");
            assert!(!rest.iter().any(|a| a == "cache" || a == "log"), "{name}");
        }
        assert_eq!(
            command_args(&args(&["config", "--config=x.toml", "--check", "--config", "log"])),
            args(&["config", "--check"])
        );
    }

    #[test]
    fn config_mode_distinguishes_edit_check_and_list() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(config_mode(&args(&["config"])), Some(ConfigMode::Edit));
        assert_eq!(config_mode(&args(&["config", "--check"])), Some(ConfigMode::Check));
        assert_eq!(
            config_mode(&args(&["--config", "x", "config", "--check"])),
            Some(ConfigMode::Check)
        );
        assert_eq!(config_mode(&args(&["config", "--list"])), Some(ConfigMode::List));
        assert_eq!(
            config_mode(&args(&["--config", "x", "config", "--list"])),
            Some(ConfigMode::List)
        );
        assert_eq!(config_mode(&args(&["--check"])), None);
    }

    #[test]
    fn bare_command_detects_setup() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(bare_command(&args(&["setup"])), BareCommand::Setup);
        assert_eq!(bare_command(&args(&["setup", "--config", "x"])), BareCommand::Setup);
    }

    #[test]
    fn frame_budget_follows_the_visualizer_framerate() {
        assert_eq!(frame_interval(true, 50, 60), std::time::Duration::from_micros(16_666));
        assert_eq!(frame_interval(true, 50, 30), std::time::Duration::from_micros(33_333));
        assert_eq!(frame_interval(false, 50, 60), DEFAULT_FRAME_INTERVAL);
        assert_eq!(frame_interval(true, 0, 60), DEFAULT_FRAME_INTERVAL);
    }
}
