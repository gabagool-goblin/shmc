//! mpv driven over its Unix-socket JSON IPC. mpv's playlist is the play queue;
//! it only sees URLs, all metadata lives in the UI's queue model.

use crate::config::MpvConfig;
use crate::redact::redact_secrets;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

/// A queue entry for `loadfile`. `title` becomes mpv's `media-title`; without
/// it an untagged file shows its name, the URL with the token.
#[derive(Debug, Clone)]
pub struct LoadItem {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub enum MpvCmd {
    /// Replace the playlist and start the first URL (`playlist-clear` would
    /// leave the playing entry behind, so the first is `loadfile replace`).
    Play(Vec<LoadItem>),
    Append(Vec<LoadItem>),
    /// mpv 0.36 has no `playlist-insert`: append, then move into place
    /// (`append_at` is the playlist length before the append).
    Insert {
        index: usize,
        append_at: usize,
        items: Vec<LoadItem>,
    },
    PlayIndex(usize),
    Remove(usize),
    Clear,
    /// Stop but keep the playlist (`stop` would wipe it).
    Stop,
    TogglePause,
    /// Sets `pause` explicitly: `cycle` depends on mpv's state, which can be
    /// stale (e.g. the sleep timer firing while paused).
    SetPause {
        paused: bool,
    },
    Next,
    Prev,
    SeekRel(f64),
    SeekAbs(f64),
    SetVolume(f64),
    /// "inf" or "no". Not `cycle`: mpv treats these as numbers and increments.
    SetLoop {
        property: &'static str,
        on: bool,
    },
    /// Move an entry so it ends at index `to`. mpv's target is an entry, one
    /// less than the final index when moving forward; `cmd_to_lines` compensates.
    Move {
        from: usize,
        to: usize,
    },
    /// The reply echoes `request_id`: mpv's answers have no other identity.
    GetProperty {
        name: String,
        request_id: u64,
    },
}

/// A media key or Control Center button, rebound inside mpv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    Toggle,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    Forward,
    Rewind,
}

/// mpv key name, message word, key.
const MEDIA_KEYS: [(&str, &str, MediaKey); 10] = [
    ("PLAY", "toggle", MediaKey::Toggle),
    ("PAUSE", "toggle", MediaKey::Toggle),
    ("PLAYPAUSE", "toggle", MediaKey::Toggle),
    ("PLAYONLY", "play", MediaKey::Play),
    ("PAUSEONLY", "pause", MediaKey::Pause),
    ("STOP", "stop", MediaKey::Stop),
    ("NEXT", "next", MediaKey::Next),
    ("PREV", "prev", MediaKey::Prev),
    ("FORWARD", "forward", MediaKey::Forward),
    ("REWIND", "rewind", MediaKey::Rewind),
];

#[derive(Debug, Clone)]
pub enum MpvEvent {
    /// An observed property: time-pos, pause, playlist-pos, volume,
    /// playlist-count, loop-playlist, loop-file, shuffle, audio-bitrate.
    Property {
        name: String,
        value: Option<Value>,
    },
    EndFile {
        reason: String,
    },
    Ready,
    /// A media key pressed, reported through a `script-message`.
    MediaKey(MediaKey),
    Shutdown,
}

/// Unlinks the socket when the last handle clone is dropped, so a path that
/// never reaches `shutdown` leaves nothing behind.
struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Clone)]
pub struct MpvHandle {
    pub tx: UnboundedSender<MpvCmd>,
    inner: std::sync::Arc<tokio::sync::Mutex<Child>>,
    sock: PathBuf,
    /// A user setting shmc ignored at startup (e.g. `--log-file`).
    pub ignored_setting: Option<String>,
    /// Callers waiting on a `get_property` reply, by `request_id`.
    pending: std::sync::Arc<Pending>,
    next_request: std::sync::Arc<AtomicU64>,
    _socket: std::sync::Arc<SocketCleanup>,
}

impl MpvHandle {
    pub async fn shutdown(&self) {
        let mut child = self.inner.lock().await;
        let _ = child.start_kill();
        // A killed mpv cannot unlink its own socket.
        let _ = std::fs::remove_file(&self.sock);
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.sock
    }

    /// Ask mpv for a property's current value: an authoritative answer, where a
    /// late `property-change` event looks like a stalled player.
    pub async fn get_property(&self, name: &str) -> Result<Value> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let sent =
            self.tx.send(MpvCmd::GetProperty { name: name.to_string(), request_id: id }).is_ok();
        let reply = if !sent {
            Err(anyhow::anyhow!("mpv is not running"))
        } else {
            match tokio::time::timeout(GET_PROPERTY_TIMEOUT, rx).await {
                Ok(Ok(reply)) => reply,
                Ok(Err(_)) => Err(anyhow::anyhow!("mpv stopped answering")),
                Err(_) => Err(anyhow::anyhow!("mpv did not answer '{name}'")),
            }
        };
        // The reader removes the slot on success.
        if reply.is_err() {
            self.pending.lock().await.remove(&id);
        }
        reply
    }
}

/// mpv answers local properties at once; slower means a dead socket.
const GET_PROPERTY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// mpv's arguments, and the `extra_args` entry dropped, if any. A user
/// `--log-file` would get the token-bearing URLs unredacted; shmc's goes last.
fn build_args(cfg: &MpvConfig, sock: &Path, fifo: Option<&Path>) -> (Vec<String>, Option<String>) {
    let mut args: Vec<String> = vec![
        "--no-terminal".into(),
        // A user's mpv.conf (`save-position-on-quit` writes the URL to
        // watch_later) or scripts could put the token-bearing URL on disk.
        "--no-config".into(),
        "--load-scripts=no".into(),
        "--no-video".into(),
        "--audio-display=no".into(),
        "--idle=yes".into(),
        "--gapless-audio=yes".into(),
        "--keep-open=no".into(),
        // A stream mpv cannot open would go to yt-dlp, whose command line
        // carries the token. Before `extra_args`, so a user can turn it back on.
        "--ytdl=no".into(),
        "--volume-max=100".into(),
        "--msg-level=all=warn".into(),
        format!("--input-ipc-server={}", sock.display()),
    ];
    let mut skip_value = false;
    let mut ignored: Option<String> = None;
    for arg in &cfg.extra_args {
        if skip_value {
            // The value of a bare `--log-file`; left alone it would become a
            // file mpv tries to open.
            skip_value = false;
            continue;
        }
        if arg == "--log-file" {
            warn!(
                "ignoring `mpv.extra_args` entry {arg:?} and the value after it: a custom \
                 mpv log file would receive the unredacted Plex token"
            );
            ignored = Some(arg.clone());
            skip_value = true;
            continue;
        }
        if arg.starts_with("--log-file=") {
            warn!(
                "ignoring `mpv.extra_args` entry {arg:?}: a custom mpv log file would \
                 receive the unredacted Plex token"
            );
            ignored = Some(arg.clone());
            continue;
        }
        args.push(arg.clone());
    }
    if let Some(fifo) = fifo {
        // Last wins in mpv, so this must stay after `extra_args`.
        args.push(format!("--log-file={}", fifo.display()));
    }
    (args, ignored)
}

pub async fn spawn(cfg: &MpvConfig) -> Result<(MpvHandle, UnboundedReceiver<MpvEvent>)> {
    let sock = socket_path_from(crate::cache::private_run_dir())?;
    let _ = std::fs::remove_file(&sock);

    // Must exist before mpv starts: `--log-file` blocks until a reader is there.
    let log_pipe = mpv_log_pipe();
    let fifo_path = log_pipe.as_ref().map(|pipe| pipe.fifo.clone());

    let (args, ignored_setting) =
        build_args(cfg, &sock, log_pipe.as_ref().map(|pipe| pipe.fifo.as_path()));

    let child = match Command::new(&cfg.path)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("cannot start mpv at {}; is it installed?", cfg.path))
    {
        Ok(child) => child,
        Err(e) => {
            if let Some(pipe) = log_pipe {
                let _ = std::fs::remove_file(&pipe.fifo);
            }
            return Err(e);
        }
    };
    info!(pid = child.id(), sock = %sock.display(), "mpv spawned");

    let log_done = log_pipe.map(spawn_log_reader);

    let volume = cfg.volume;
    let inner = std::sync::Arc::new(tokio::sync::Mutex::new(child));

    let stream = match connect_with_retry(&sock, std::time::Duration::from_secs(5), &inner).await {
        Ok(stream) => stream,
        Err(e) => {
            let _ = inner.lock().await.start_kill();
            let _ = std::fs::remove_file(&sock);
            if let (Some(fifo), Some(done)) = (&fifo_path, &log_done) {
                abandon_log_pipe(fifo, done).await;
            }
            return Err(e.context("connecting to mpv IPC socket"));
        }
    };

    // mpv creates the node with its own umask.
    let _ = crate::cache::set_private(&sock);

    let (tx, mut rx) = unbounded_channel::<MpvCmd>();
    let (etx, erx) = unbounded_channel::<MpvEvent>();
    let pending: std::sync::Arc<Pending> =
        std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let next_request = std::sync::Arc::new(AtomicU64::new(1));
    let reader_pending = pending.clone();
    let writer_pending = pending.clone();

    tokio::spawn(async move {
        let (read_half, mut write_half) = stream.into_split();

        for (id, name) in [
            (1, "time-pos"),
            (2, "pause"),
            (3, "playlist-pos"),
            (5, "volume"),
            (6, "playlist-count"),
            (7, "loop-playlist"),
            (8, "loop-file"),
            (9, "shuffle"),
            // fires once per file: no polling
            (10, "audio-bitrate"),
        ] {
            let _ = write_half
                .write_all(
                    format!("{}\n", json!({"command": ["observe_property", id, name]})).as_bytes(),
                )
                .await;
        }
        let _ = write_half
            .write_all(
                format!("{}\n", json!({"command": ["set_property", "volume", volume]})).as_bytes(),
            )
            .await;
        // mpv's own media key bindings would act behind shmc's back, and STOP quits mpv.
        let section: String = MEDIA_KEYS
            .iter()
            .map(|(key, word, _)| format!("{key} script-message shmc-media {word}\n"))
            .collect();
        for command in [
            json!(["define-section", "shmc-media", section, "force"]),
            json!(["enable-section", "shmc-media"]),
        ] {
            let _ = write_half
                .write_all(format!("{}\n", json!({ "command": command })).as_bytes())
                .await;
        }
        let _ = etx.send(MpvEvent::Ready);

        // Reads run on their own task: mpv answers every command, and a long
        // `loadfile` burst would otherwise deadlock the writer.
        let reader = tokio::spawn(async move {
            read_events(read_half, &reader_pending, &etx).await;
            // Nobody will answer the questions still open.
            reader_pending.lock().await.clear();
        });

        while let Some(cmd) = rx.recv().await {
            let cmds = cmd_to_lines(cmd);
            let mut closed = false;
            for line in cmds {
                // Never log the payload: `loadfile` carries the token.
                debug!(cmd = %command_name(&line), len = line.len(), "mpv send");
                if write_half.write_all(line.as_bytes()).await.is_err()
                    || write_half.write_all(b"\n").await.is_err()
                {
                    closed = true;
                    break;
                }
            }
            if closed {
                warn!("mpv IPC socket closed");
                break;
            }
        }
        reader.abort();
        rx.close();
        writer_pending.lock().await.clear();
    });

    Ok((
        MpvHandle {
            tx,
            inner,
            _socket: std::sync::Arc::new(SocketCleanup(sock.clone())),
            sock,
            ignored_setting,
            pending,
            next_request,
        },
        erx,
    ))
}

type Pending = tokio::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>;

/// Route mpv's IPC lines to their callers and the UI until mpv is gone. Lines
/// are read as bytes: a bad one is skipped, only EOF or an I/O error ends this.
async fn read_events<R: tokio::io::AsyncRead + Unpin>(
    read: R,
    pending: &Pending,
    etx: &UnboundedSender<MpvEvent>,
) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => {
                warn!("mpv exited (IPC closed)");
                let _ = etx.send(MpvEvent::Shutdown);
                break;
            }
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\n', '\r']);
                if let Some((id, reply)) = parse_reply(line) {
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let _ = tx.send(reply);
                    }
                    continue;
                }
                if let Some(ev) = parse_event(line) {
                    let shutdown = matches!(ev, MpvEvent::Shutdown);
                    if etx.send(ev).is_err() || shutdown {
                        break;
                    }
                }
            }
            Err(e) => {
                error!("mpv IPC read error: {e}");
                let _ = etx.send(MpvEvent::Shutdown);
                break;
            }
        }
    }
}

/// mpv logs every `loadfile` URL with the token, so its log goes through a FIFO
/// that redacts each line. Without the FIFO mpv gets no log at all.
struct MpvLogPipe {
    fifo: PathBuf,
    log: PathBuf,
    /// Opened before mpv starts so its write-open cannot block.
    reader: std::fs::File,
}

fn mpv_log_pipe() -> Option<MpvLogPipe> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = match crate::cache::try_ensure_cache_dir() {
        Ok(dir) => dir,
        Err(e) => {
            warn!(error = %e, "the cache dir is not private; mpv will not keep a log");
            return None;
        }
    };

    // Truncate on every start so the file cannot grow without bound.
    let log = crate::config::usable_log_dir()?.join("mpv.log");
    crate::cache::open_private(&log, false).ok()?;

    remove_stale_pipes(&dir, STALE_SOCKET_MAX_AGE);

    let fifo = dir.join(format!("mpv-{}-{}.pipe", std::process::id(), nonce()));
    let _ = std::fs::remove_file(&fifo);

    let c_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c_path` is a valid NUL-terminated path.
    if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
        warn!(
            path = %fifo.display(),
            error = %std::io::Error::last_os_error(),
            "cannot create the mpv log FIFO; mpv will not keep a log"
        );
        return None;
    }

    // O_NONBLOCK: a blocking read-open would wait for mpv, which is not spawned yet.
    let reader =
        match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&fifo) {
            Ok(f) => f,
            Err(e) => {
                warn!(path = %fifo.display(), error = %e, "cannot open the mpv log FIFO");
                let _ = std::fs::remove_file(&fifo);
                return None;
            }
        };

    Some(MpvLogPipe { fifo, log, reader })
}

/// Drain the FIFO into `mpv.log`, redacted, until EOF. The flag tells
/// [`abandon_log_pipe`] whether the reader is done.
fn spawn_log_reader(pipe: MpvLogPipe) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    struct DoneOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for DoneOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = done.clone();
    let log = pipe.log.clone();
    std::thread::spawn(move || {
        let _done = DoneOnDrop(flag);
        let mut out = match crate::cache::open_private(&log, true) {
            Ok(f) => f,
            Err(e) => {
                warn!(path = %log.display(), error = %e, "cannot write the mpv log");
                let _ = std::fs::remove_file(&pipe.fifo);
                return;
            }
        };
        // The pre-opened handle reads EOF until mpv writes; keep it until the
        // blocking open returns so mpv's own open never blocks.
        let reader = match std::fs::File::open(&pipe.fifo) {
            Ok(f) => f,
            Err(e) => {
                warn!(path = %pipe.fifo.display(), error = %e, "cannot read the mpv log FIFO");
                let _ = std::fs::remove_file(&pipe.fifo);
                return;
            }
        };
        drop(pipe.reader);
        // Unlink now so a killed shmc leaves no FIFO behind.
        let _ = std::fs::remove_file(&pipe.fifo);
        copy_redacted(std::io::BufReader::new(reader), &mut out);
        let _ = out.flush();
    });
    done
}

/// Copy lines to `out`, redacted. Bytes, since mpv can log non-UTF-8 names;
/// after a write error it keeps reading, or a full pipe would block mpv.
fn copy_redacted(mut reader: impl BufRead, out: &mut impl Write) {
    let mut buf = Vec::new();
    let mut failed = false;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) if failed => {}
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf);
                if let Err(e) = out.write_all(redact_secrets(&text).as_bytes()) {
                    warn!(error = %e, "cannot write the mpv log; discarding the rest");
                    failed = true;
                }
            }
            Err(e) => {
                warn!(error = %e, "reading the mpv log FIFO");
                break;
            }
        }
    }
}

/// mpv never opened its log, so the reader is stuck in `open`: open the FIFO
/// for writing and close it until the thread is done.
async fn abandon_log_pipe(fifo: &Path, done: &std::sync::atomic::AtomicBool) {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..50 {
        if done.load(Ordering::SeqCst) {
            break;
        }
        let _ = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let _ = std::fs::remove_file(fifo);
}

/// Unique per call: the clock can be coarse, so a counter fills the low bits.
fn nonce() -> u128 {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = u128::from(LAST.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    (now << 8) | (seq & 0xFF)
}

/// A socket path in the 0700 run dir, never the system temp dir: mpv's IPC
/// can read the token-bearing queue. Fails rather than fall back.
fn socket_path_from(run_dir: std::io::Result<PathBuf>) -> Result<PathBuf> {
    let dir = run_dir.map_err(|e| {
        anyhow::anyhow!("cannot create a private directory for the mpv socket: {e}")
    })?;
    remove_stale_sockets(&dir, STALE_SOCKET_MAX_AGE);
    Ok(dir.join(format!("shmc-mpv-{}-{}.sock", std::process::id(), nonce())))
}

/// mpv creates the socket node a moment before `listen()`; a second shmc
/// sweeping inside that window must not unlink it.
const STALE_SOCKET_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

/// Remove sockets a crashed shmc left: no one answers and they are older than
/// `max_age` (a young one may not be listening yet).
fn remove_stale_sockets(dir: &Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("shmc-mpv-") || !name.ends_with(".sock") {
            continue;
        }
        let path = entry.path();
        // Probe only old nodes: a young one may not be listening yet.
        if !touched_longer_ago_than(&path, now, max_age) {
            continue;
        }
        if std::os::unix::net::UnixStream::connect(&path).is_err() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remove log FIFOs a killed shmc left behind; a live one exists only until
/// the reader attaches.
fn remove_stale_pipes(dir: &Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("mpv-")
            && name.ends_with(".pipe")
            && touched_longer_ago_than(&entry.path(), now, max_age)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether the mtime is more than `max_age` behind `now`. One that cannot be
/// read, or lies in the future, counts as fresh.
fn touched_longer_ago_than(
    path: &Path,
    now: std::time::SystemTime,
    max_age: std::time::Duration,
) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    match now.duration_since(modified) {
        Ok(age) => age > max_age,
        Err(_) => false,
    }
}

/// Connect to mpv's socket; fails at once if mpv has already exited (a bad
/// path or option) instead of waiting out the timeout.
async fn connect_with_retry(
    sock: &PathBuf,
    timeout: std::time::Duration,
    child: &tokio::sync::Mutex<Child>,
) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match UnixStream::connect(sock).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if let Ok(Some(status)) = child.lock().await.try_wait() {
                    anyhow::bail!(
                        "mpv exited before it was ready ({status}); see 'shmc log' and mpv.extra_args"
                    );
                }
                if std::time::Instant::now() > deadline {
                    return Err(anyhow::anyhow!("timeout waiting for {}: {e}", sock.display()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

/// The verb of an IPC line, for logging without the arguments.
fn command_name(line: &str) -> String {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| {
            let command = v.get("command")?;
            let verb = command.get(0).or_else(|| command.get("name"))?;
            verb.as_str().map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".into())
}

/// Named arguments, as mpv 0.38 put `index` before `options`. A blank title
/// would fall back to the URL, so it is sent as "Untitled".
fn loadfile_line(item: &LoadItem, flags: &str) -> String {
    let title = item.title.trim();
    let title = if title.is_empty() { "Untitled" } else { title };
    json!({"command": {
        "name": "loadfile",
        "url": item.url,
        "flags": flags,
        "options": {"force-media-title": title},
    }})
    .to_string()
}

fn cmd_to_lines(cmd: MpvCmd) -> Vec<String> {
    let v: Value = match cmd {
        MpvCmd::Play(items) => {
            return items
                .iter()
                .enumerate()
                .map(|(i, item)| loadfile_line(item, if i == 0 { "replace" } else { "append" }))
                .collect();
        }
        MpvCmd::Append(items) => {
            return items.iter().map(|item| loadfile_line(item, "append")).collect();
        }
        MpvCmd::Insert { index, append_at, items } => {
            let mut lines: Vec<String> =
                items.iter().map(|item| loadfile_line(item, "append")).collect();
            // Ascending, so each move leaves the next item's index untouched.
            for (i, _) in items.iter().enumerate() {
                let from = append_at + i;
                let to = index + i;
                if from != to {
                    lines.push(json!({"command": ["playlist-move", from, to]}).to_string());
                }
            }
            return lines;
        }
        MpvCmd::PlayIndex(i) => json!({"command": ["playlist-play-index", i]}),
        MpvCmd::Remove(i) => json!({"command": ["playlist-remove", i]}),
        MpvCmd::Clear => json!({"command": ["playlist-clear"]}),
        MpvCmd::Stop => json!({"command": ["playlist-play-index", "none"]}),
        MpvCmd::TogglePause => json!({"command": ["cycle", "pause"]}),
        MpvCmd::SetPause { paused } => json!({"command": ["set_property", "pause", paused]}),
        MpvCmd::Next => json!({"command": ["playlist-next", "force"]}),
        MpvCmd::Prev => json!({"command": ["playlist-prev", "force"]}),
        MpvCmd::SeekRel(secs) => json!({"command": ["seek", secs, "relative+exact"]}),
        MpvCmd::SeekAbs(secs) => json!({"command": ["seek", secs, "absolute+exact"]}),
        MpvCmd::SetVolume(v) => json!({"command": ["set_property", "volume", v]}),
        MpvCmd::SetLoop { property, on } => json!({
            "command": ["set_property", property, if on { "inf" } else { "no" }]
        }),
        MpvCmd::Move { from, to } => {
            // mpv's target is an entry: moving forward lands one slot earlier.
            let target = if from < to { to + 1 } else { to };
            json!({"command": ["playlist-move", from, target]})
        }
        MpvCmd::GetProperty { name, request_id } => {
            json!({"command": ["get_property", name], "request_id": request_id})
        }
    };
    vec![v.to_string()]
}

/// A reply to a command that carried a `request_id`; `None` for everything
/// else (events, replies to commands sent without an id).
fn parse_reply(line: &str) -> Option<(u64, Result<Value>)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let request_id = v.get("request_id")?.as_u64()?;
    match v.get("error").and_then(|e| e.as_str()) {
        Some("success") | None => {
            Some((request_id, Ok(v.get("data").cloned().unwrap_or(Value::Null))))
        }
        Some(other) => Some((request_id, Err(anyhow::anyhow!("mpv: {other}")))),
    }
}

fn parse_event(line: &str) -> Option<MpvEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("event")?.as_str()? {
        "property-change" => Some(MpvEvent::Property {
            name: v.get("name")?.as_str()?.to_string(),
            value: v.get("data").cloned(),
        }),
        "end-file" => Some(MpvEvent::EndFile {
            reason: v.get("reason").and_then(|r| r.as_str()).unwrap_or("unknown").to_string(),
        }),
        "client-message" => {
            let args = v.get("args")?.as_array()?;
            if args.first()?.as_str()? != "shmc-media" {
                return None;
            }
            let word = args.get(1)?.as_str()?;
            MEDIA_KEYS.iter().find(|(_, w, _)| *w == word).map(|(_, _, k)| MpvEvent::MediaKey(*k))
        }
        "shutdown" => Some(MpvEvent::Shutdown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_reader_keeps_draining_after_write_error() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let feed: Vec<u8> = (0..1000).flat_map(|i| format!("line {i}\n").into_bytes()).collect();
        let mut source = std::io::Cursor::new(feed.clone());
        copy_redacted(std::io::BufReader::with_capacity(64, &mut source), &mut Full);
        assert_eq!(source.position(), feed.len() as u64, "the pipe must be read to EOF");
    }

    #[test]
    fn socket_lives_in_private_run_dir() {
        let run = crate::cache::private_run_dir().expect("create the run dir");
        let sock = socket_path_from(Ok(run.clone())).expect("a socket path");
        assert_eq!(
            sock.parent(),
            Some(run.as_path()),
            "the IPC socket must not land in the system temp dir"
        );
        let name = sock.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("shmc-mpv-") && name.ends_with(".sock"),
            "unexpected socket name {name}"
        );
    }

    #[test]
    fn no_private_dir_means_no_socket() {
        let err = socket_path_from(Err(std::io::Error::other("denied")))
            .expect_err("must not fall back to the system temp dir");
        assert!(
            format!("{err:#}").contains("cannot create a private directory for the mpv socket"),
            "{err:#}"
        );
    }

    #[test]
    fn socket_names_are_unique_per_spawn() {
        let tmp = tempfile::tempdir().unwrap();
        let path = || socket_path_from(Ok(tmp.path().to_path_buf())).unwrap();
        assert_ne!(path(), path());
    }

    #[test]
    fn only_stale_sockets_are_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let dead = tmp.path().join("shmc-mpv-1-1.sock");
        std::fs::write(&dead, b"").unwrap();
        age_by(&dead, std::time::Duration::from_secs(3600));
        let live = tmp.path().join("shmc-mpv-2-2.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let other = tmp.path().join("unrelated.sock");
        std::fs::write(&other, b"").unwrap();
        age_by(&other, std::time::Duration::from_secs(3600));

        remove_stale_sockets(tmp.path(), STALE_SOCKET_MAX_AGE);

        assert!(!dead.exists(), "a stale socket must be cleaned up");
        assert!(live.exists(), "a running mpv's socket must not be removed");
        assert!(other.exists(), "only shmc's own sockets may be touched");
    }

    #[test]
    fn fresh_unanswered_socket_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let starting = tmp.path().join("shmc-mpv-1-1.sock");
        std::fs::write(&starting, b"").unwrap();

        remove_stale_sockets(tmp.path(), STALE_SOCKET_MAX_AGE);

        assert!(starting.exists(), "a just-created socket may be an mpv that has not listened yet");
    }

    #[test]
    fn answering_socket_kept_at_any_age() {
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("shmc-mpv-1-1.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();

        // Zero age makes everything old: only the probe can save the socket.
        remove_stale_sockets(tmp.path(), std::time::Duration::ZERO);

        assert!(live.exists(), "a live mpv must survive any sweep");
    }

    /// Plain files only: a socket cannot be opened to backdate it.
    fn age_by(path: &Path, age: std::time::Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        let then = std::time::SystemTime::now().checked_sub(age).unwrap();
        file.set_modified(then).unwrap();
    }

    #[test]
    fn set_pause_sets_property_not_cycle() {
        let paused = cmd_to_lines(MpvCmd::SetPause { paused: true });
        assert!(paused[0].contains("\"set_property\""));
        assert!(paused[0].contains("\"pause\""));
        assert!(paused[0].contains("true"));
        assert!(!paused[0].contains("cycle"), "a toggle depends on mpv's state");
        let resumed = cmd_to_lines(MpvCmd::SetPause { paused: false });
        assert!(resumed[0].contains("false"));
    }

    #[test]
    fn set_loop_sets_inf_and_no() {
        let on = cmd_to_lines(MpvCmd::SetLoop { property: "loop-playlist", on: true });
        assert!(on[0].contains("\"set_property\""));
        assert!(on[0].contains("loop-playlist"));
        assert!(on[0].contains("\"inf\""));

        let off = cmd_to_lines(MpvCmd::SetLoop { property: "loop-file", on: false });
        assert!(off[0].contains("\"no\""));
    }

    fn item(url: &str, title: &str) -> LoadItem {
        LoadItem { url: url.into(), title: title.into() }
    }

    fn loadfile_args(line: &str) -> Value {
        let v: Value = serde_json::from_str(line).expect("json");
        assert_eq!(v["command"]["name"], "loadfile", "{line}");
        v["command"].clone()
    }

    #[test]
    fn every_loadfile_carries_its_title() {
        let items = || vec![item("http://plex/1?X-Plex-Token=T", "One"), item("u2", "Two, b=c")];
        let cmds = [
            MpvCmd::Play(items()),
            MpvCmd::Append(items()),
            MpvCmd::Insert { index: 0, append_at: 5, items: items() },
        ];
        for cmd in cmds {
            let lines: Vec<String> =
                cmd_to_lines(cmd).into_iter().filter(|l| l.contains("loadfile")).collect();
            assert_eq!(lines.len(), 2);
            let a = loadfile_args(&lines[0]);
            assert_eq!(a["options"]["force-media-title"], "One");
            assert_eq!(a["url"], "http://plex/1?X-Plex-Token=T");
            let b = loadfile_args(&lines[1]);
            assert_eq!(b["options"]["force-media-title"], "Two, b=c");
            assert_eq!(b["url"], "u2");
            assert!(!a["options"].to_string().contains("X-Plex-Token"));
        }
    }

    #[test]
    fn a_blank_title_is_sent_as_untitled() {
        let lines = cmd_to_lines(MpvCmd::Append(vec![
            item("u1", ""),
            item("u2", "  \t "),
            item("u3", " X "),
        ]));
        let titles: Vec<String> = lines
            .iter()
            .map(|l| loadfile_args(l)["options"]["force-media-title"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(titles, ["Untitled", "Untitled", "X"]);
    }

    #[test]
    fn insert_appends_then_moves_into_place() {
        let lines = cmd_to_lines(MpvCmd::Insert {
            index: 1,
            append_at: 3,
            items: vec![item("a", "A"), item("b", "B")],
        });
        assert_eq!(lines.len(), 4, "two appends then two moves: {lines:?}");
        assert_eq!(command_name(&lines[0]), "loadfile");
        assert_eq!(command_name(&lines[1]), "loadfile");
        assert!(lines[2].contains("[\"playlist-move\",3,1]"), "{}", lines[2]);
        assert!(lines[3].contains("[\"playlist-move\",4,2]"), "{}", lines[3]);
    }

    #[test]
    fn play_replaces_the_whole_playlist() {
        let lines = cmd_to_lines(MpvCmd::Play(vec![item("a", "A"), item("b", "B")]));
        assert_eq!(lines.len(), 2, "replace then append: {lines:?}");
        let first = loadfile_args(&lines[0]);
        assert_eq!((first["url"].as_str(), first["flags"].as_str()), (Some("a"), Some("replace")));
        let second = loadfile_args(&lines[1]);
        assert_eq!((second["url"].as_str(), second["flags"].as_str()), (Some("b"), Some("append")));
        assert!(
            !lines.iter().any(|l| l.contains("playlist-clear")),
            "playlist-clear keeps the playing entry: {lines:?}"
        );

        let empty = cmd_to_lines(MpvCmd::Play(Vec::new()));
        assert!(empty.is_empty(), "an empty Play sends nothing");
    }

    #[test]
    fn logged_command_metadata_has_no_payload() {
        let lines = cmd_to_lines(MpvCmd::Play(vec![item(
            "http://plex/x?X-Plex-Token=SECRETTOKEN123",
            "T",
        )]));
        assert_eq!(command_name(&lines[0]), "loadfile");
        assert!(
            !command_name(&lines[0]).contains("SECRETTOKEN123"),
            "the debug log must not carry the URL"
        );
    }

    #[test]
    fn get_property_carries_a_request_id() {
        let lines =
            cmd_to_lines(MpvCmd::GetProperty { name: "playlist-pos".into(), request_id: 7 });
        assert_eq!(lines.len(), 1);
        let v: Value = serde_json::from_str(&lines[0]).expect("json");
        assert_eq!(v["command"][0], "get_property");
        assert_eq!(v["command"][1], "playlist-pos");
        assert_eq!(v["request_id"], 7);
        assert_eq!(command_name(&lines[0]), "get_property");
    }

    #[test]
    fn reply_matched_by_request_id() {
        let (id, reply) =
            parse_reply(r#"{"request_id":7,"error":"success","data":3}"#).expect("a reply");
        assert_eq!(id, 7);
        assert_eq!(reply.expect("data"), json!(3));

        let (id, reply) =
            parse_reply(r#"{"request_id":8,"error":"property unavailable"}"#).expect("a reply");
        assert_eq!(id, 8);
        assert!(reply.is_err());

        assert!(parse_reply(r#"{"error":"success"}"#).is_none());
        assert!(
            parse_reply(r#"{"event":"property-change","name":"playlist-pos","data":3}"#).is_none()
        );
    }

    #[test]
    fn move_uses_final_index_semantics() {
        let forward = cmd_to_lines(MpvCmd::Move { from: 0, to: 2 });
        assert!(
            forward[0].contains("[\"playlist-move\",0,3]"),
            "moving forward must target to + 1: {}",
            forward[0]
        );

        let backward = cmd_to_lines(MpvCmd::Move { from: 3, to: 1 });
        assert!(
            backward[0].contains("[\"playlist-move\",3,1]"),
            "moving backward keeps the target index: {}",
            backward[0]
        );
    }

    #[test]
    fn ytdl_off_but_user_can_enable() {
        let cfg =
            MpvConfig { path: "mpv".into(), extra_args: vec!["--ytdl=yes".into()], volume: 50.0 };
        let (args, _) = build_args(&cfg, Path::new("/tmp/shmc.sock"), None);
        let ours = args.iter().position(|a| a == "--ytdl=no").expect("--ytdl=no");
        let theirs = args.iter().position(|a| a == "--ytdl=yes").expect("user arg");
        assert!(ours < theirs, "mpv takes the last occurrence: {args:?}");
    }

    #[test]
    fn user_mpv_config_and_scripts_not_loaded() {
        let cfg = MpvConfig {
            path: "mpv".into(),
            extra_args: vec!["--audio-device=null".into()],
            volume: 50.0,
        };
        let (args, _) = build_args(&cfg, Path::new("/tmp/shmc.sock"), None);
        let user = args.iter().position(|a| a == "--audio-device=null").expect("user arg");
        for flag in ["--no-config", "--load-scripts=no"] {
            let at = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("{flag}"));
            assert!(at < user, "{flag} comes before extra_args: {args:?}");
        }
    }

    #[tokio::test]
    async fn non_utf8_line_does_not_end_reader() {
        let input: &[u8] = b"{\"event\":\"property-change\",\"name\":\"pause\",\"data\":true}\n\
            \xff\xfe not json\n\
            {\"event\":\"end-file\",\"reason\":\"eof\"}\n";
        let pending = Pending::default();
        let (etx, mut erx) = unbounded_channel();
        read_events(input, &pending, &etx).await;
        let mut got = Vec::new();
        while let Ok(ev) = erx.try_recv() {
            got.push(ev);
        }
        assert!(
            matches!(
                got.as_slice(),
                [MpvEvent::Property { .. }, MpvEvent::EndFile { .. }, MpvEvent::Shutdown,]
            ),
            "the bad line is skipped; only EOF ends the reader: {got:?}"
        );
    }

    #[test]
    fn only_old_log_pipes_are_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("mpv-1-2.pipe");
        let fresh = tmp.path().join("mpv-3-4.pipe");
        let other = tmp.path().join("album.pipe");
        for f in [&old, &fresh, &other] {
            std::fs::write(f, b"").unwrap();
        }
        age_by(&old, std::time::Duration::from_secs(3600));
        age_by(&other, std::time::Duration::from_secs(3600));
        remove_stale_pipes(tmp.path(), STALE_SOCKET_MAX_AGE);
        assert!(!old.exists(), "a stale log FIFO must be cleaned up");
        assert!(fresh.exists(), "a fresh one may belong to a starting mpv");
        assert!(other.exists(), "only our own names are swept");
    }

    #[test]
    fn user_log_file_is_dropped() {
        let cfg = MpvConfig {
            path: "mpv".into(),
            extra_args: vec!["--ao=pulse".into(), "--log-file=/tmp/leak.log".into()],
            volume: 50.0,
        };
        let sock = Path::new("/tmp/shmc.sock");
        let fifo = Path::new("/tmp/shmc.pipe");

        let (args, ignored) = build_args(&cfg, sock, Some(fifo));
        assert_eq!(ignored.as_deref(), Some("--log-file=/tmp/leak.log"));
        assert!(args.iter().any(|a| a == "--ao=pulse"), "other args survive");
        assert!(
            !args.iter().any(|a| a.starts_with("--log-file=/tmp/leak.log")),
            "a user log file would receive the unredacted token: {args:?}"
        );
        assert_eq!(
            args.last().map(String::as_str),
            Some("--log-file=/tmp/shmc.pipe"),
            "ours must be last: mpv takes the last occurrence"
        );

        let (args, ignored) = build_args(&cfg, sock, None);
        assert!(ignored.is_some());
        assert!(!args.iter().any(|a| a.starts_with("--log-file")));

        // The two-element form loses its value too.
        let cfg = MpvConfig {
            path: "mpv".into(),
            extra_args: vec!["--log-file".into(), "/tmp/leak.log".into(), "--ao=pulse".into()],
            volume: 50.0,
        };
        let (args, ignored) = build_args(&cfg, sock, Some(fifo));
        assert_eq!(ignored.as_deref(), Some("--log-file"));
        assert!(!args.iter().any(|a| a.contains("leak")), "{args:?}");
        assert!(args.iter().any(|a| a == "--ao=pulse"), "{args:?}");
    }
}
