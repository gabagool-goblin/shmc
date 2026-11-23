# AGENTS.md — shmc

> A terminal music player for Plex, in Rust + ratatui. It streams through one
> long-lived mpv process over JSON IPC, draws album art with `ratatui-image`
> and can run a cava visualizer. User docs are in README.md and docs/.

---

## Build and test

| Command | What it does |
|---------|--------------|
| `cargo build --release` | Builds `target/release/shmc` |
| `cargo test --release` | The full suite: real `mpv` with `--ao=null`, no Plex server |
| `cargo clippy --release --all-targets -- -D warnings` | Lint; must be clean |
| `cargo fmt` | Format; CI runs `cargo fmt --check` |

- Before a commit, run `cargo fmt`, clippy and the tests for what changed.
  CI runs the full suite on every pull request; run it locally before a
  version bump.
- A fix gets a regression test that fails without it; so does a change to an
  invariant below or to how the token and private files are handled.
- One test per behaviour, asserting the outcome: the queue, the file on disk,
  what mpv was sent. No tests that only pin wording, would pass without the
  change, or test trivial getters. Delete an obsolete test; don't bend it.
- Render tests (`tests/tabs_render/`) check that things are shown, aligned
  and not clipped at several sizes, narrow and tall included.

---

## Secrets

| Path | macOS | Linux |
|------|-------|-------|
| Config | `~/.config/shmc/` | `~/.config/shmc/` |
| Logs | `~/Library/Logs/shmc/` | `~/.local/state/shmc/` |
| Cache | `~/Library/Caches/shmc` | `~/.cache/shmc` |
| State | `~/Library/Application Support/shmc/` | `~/.local/state/shmc/` |

`$XDG_CONFIG_HOME/shmc/` replaces the config path when set.

- The Plex token lives in `config.toml`. Never put it in the repo, tests,
  logs, commit messages or command output; use fake tokens in tests and redact
  `X-Plex-Token=…` in quoted logs.
- The token goes to Plex as a header, and in a query string only in the
  stream URL mpv fetches. Logs are redacted on write (`src/redact.rs`) and
  again by `shmc log`; mpv's log goes through a redacting FIFO.
- `shmc config --list` and `--check` show the token only as `(set)` or
  `(missing)`. The config screen never writes the redacted value back.
- The config and its `.bak` are written atomically 0600. The cache dir is 0700
  and holds only album art, mpv's socket and FIFO, and the leveling script;
  files there are 0600.
  The saved queue (state dir, 0700/0600, atomic) holds rating keys and the
  server address, never a stream URL.
  What cannot be made private fails closed, never falls back to a shared
  location.

---

## Layout

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | Startup, wiring and the UI loop |
| `src/cli.rs` | Command-line validation, before anything has a side effect |
| `src/lifecycle.rs` | How a run ends: exit status, the last message, signals |
| `src/config/` | Schema and defaults (`mod.rs`); paths (`paths.rs`); reading and writing the file (`file.rs`); the settings list (`settings.rs`) |
| `src/inventory.rs` | Every setting described once, for the config screen; `every_setting_is_described` keeps it in step with the schema |
| `src/setup.rs` | Setup wizard and `set_by_key` |
| `src/doctor.rs` | The `shmc config --check` checks |
| `src/cache.rs` | Private dirs and files |
| `src/redact.rs` | Token redaction |
| `src/plex/` | The client (`client.rs`), the XML models (`models.rs`) and sonic analysis (`sonic.rs`) |
| `src/playback/` | `mpv.rs` drives mpv; `cava.rs` runs the visualizer |
| `src/ui/app/` | State, keys, actions |
| `src/ui/queue_seq.rs` | Queue request order |
| `src/ui/playlist_load.rs` | A playlist replace, streamed page by page |
| `src/ui/views.rs` | Rendering |
| `src/ui/keymap.rs` | The one keybinding registry; Help is generated from it |
| `src/ui/config_screen/` | The config screen (`text_input.rs` is its line editor) |
| `src/ui/theme.rs`, `themes.rs` | Colour accessors; built-in themes from `themes/*.toml` |
| `src/ui/scroll.rs`, `scrollbar.rs` | List scrolling (`ScrollStore`) and the custom scrollbar |
| `src/ui/filter.rs`, `icons.rs` | The list filter; the glyph sets for `ui.icons` |
| `src/ui/term.rs` | Terminal setup and the panic-safe restore |
| `build.rs` | Stamps the commit into `--version` |
| `tests/` | `mpv_ipc.rs`, `tabs_render/` (TestBackend), `plex_xml.rs` (on `fixtures/`), `cli.rs` (runs the binary) |

---

## Invariants

### mpv

- The app queue mirrors mpv's playlist. Append with `MpvCmd::Append`, start
  with `playlist-play-index`, stop with `playlist-play-index none`; never
  `stop`, which wipes the playlist.
- The socket is read on its own task: mpv answers every command, and a long
  `loadfile` batch would otherwise deadlock.
- mpv runs with `--no-config --load-scripts=no --ytdl=no`, and shmc's
  `--log-file` is always its last argument (a user one in `mpv.extra_args` is
  dropped), so nothing writes the token-bearing URL unredacted. The only
  script is shmc's own leveling one (`--script`, written 0600 to the private
  run dir); it reads the URL for the part id and never logs it.
- `loop-playlist`/`loop-file`: `set_property` with `"inf"`/`"no"`, never
  `cycle`.
- Consume removes only the finished entry; never send `playlist-play-index`
  after it, or the next track restarts.
- `playlist-pos` can be stale. `PosGuard` covers consume and reorder; any
  app-initiated playback change clears it, and it expires on a deadline. A
  stale position must not fetch art or move the cursor, and `current` never
  points past the queue.
- The shuffle restore's `Move` sequence must match the plain O(n²) version
  (`reference_moves` in the tests).
- A panic in the main or keyboard thread restores the terminal before the
  message prints. SIGTERM, SIGHUP and SIGINT (`src/lifecycle.rs`) take the same
  way out as `q`, so mpv and cava shut down. shmc does not capture the mouse.

### Cava and power

- cava runs only while playing with the Queue tab visible. It stops at once
  when playback stops and after a short grace when paused or hidden: its Core
  Audio tap holds a sleep assertion.
- No focus-based throttling: the player is meant to run on a second monitor.

### Rendering

- Messages are coalesced and painting is rate-bounded; repaint for cava frames
  only when the visualizer shows.
- Album art is aspect-fit and centred in pixels on a canvas the size of the
  art area, never in a centred cell rect. The painted rect (`shown_rect`) is
  cached and skipped on later frames, never re-sent; it resets on a resize or
  on leaving the Queue, then redraws after `hold_until`. The same `art.thumb`
  is never reloaded.
- Album-art encoding uses the tty's pixel size (`TIOCGWINSZ`): terminals can
  report cell size in logical points on Retina.
- A queue top pin (`ScrollStore`'s anchor) is sticky until the cursor reaches
  the bottom scrolloff margin; release it with `clear_top`, never by dropping
  `top_lock`.

### Plex

- The player never calls `PlexClient`: it sends a `PlexCmd` and applies the
  `Msg` reply. Plex HTTP lives in `src/plex/` and runs on the worker.
- Never rely on `limit` (`/playlists/<key>/items` ignores it): page with
  `X-Plex-Container-Start`/`-Size` through `get_all_paged`.
- Bulk endpoints have no `<Stream>`; sample rate and bit depth come from
  `/library/metadata/<key>`, cached by rating key.
- Display artist is `originalTitle`, falling back to `grandparentTitle`; album
  and artist rows keep the album artist.
- An artist's tracks: `/library/sections/<s>/all?type=10&artist.id=<key>`
  (`allLeaves` does not work).
- Read bodies in chunks under `MAX_XML_BYTES`/`MAX_IMAGE_BYTES`; never
  `resp.bytes()`.
- Requests go through `url_for` (same origin only) and redirects stay within
  the origin, so the token cannot reach another host.
- Plex `duration_ms` drives the progress bar; mpv's `duration` is not observed.
- Queue changes, local ones too, go through `QueueSeq`: replies apply in
  request order, and a replace or a clear drops everything older. Every request
  replies exactly once (`QueueReplyGuard` sends a failure on drop) or later ones
  stall. Never apply a reply without its seq, or send a failure as `Msg::Error`.
- A playlist replace streams: the request replies with its first page
  (`QueueOutcome::Streaming`) and the rest arrive as `Msg::PlaylistPage`
  tagged with its seq. Pages of a load that is not current are dropped, and
  anything that supersedes it through `QueueSeq` sets its cancel flag. While it
  loads, `exec` refuses adding, play-next, undo, shuffle and J/K. Totals come
  from `totalSize`, not `leafCount`.

### UI

- Colours come only from `[theme]` accessors (`theme::accent()`, …); no module
  names a colour.
- The Queue has no `i`. `J`/`K` are refused while the queue is filtered;
  shuffle works on the filtered queue by real index. `g <letter>` jumps even
  when the letter is bound; any other key after `g` keeps its binding.
- The `:` buffer never holds the colon. Keep `keymap::COMMANDS` and
  `parse_command` in sync.
- The config screen is not part of `App` and holds no Plex or mpv state; its
  theme preview copies the player's header, so change both together. Saves edit
  the file in place (`Config::save_changes`), never rewrite it; messages name
  keys, never values. It never prints to stderr: use `Config::parse`, not
  `read`.

---

## Style

- Match the surrounding naming. Keep changes focused; no unrelated
  reformatting or renames. Prefer existing helpers; avoid new dependencies.
- User-visible changes update README.md or docs/, Help (from the keymap) and
  the `## Unreleased` section of CHANGELOG.md.

---

## Commits and pull requests

- Conventional commits with a short imperative subject that says what changed:
  `fix: handle empty playlist responses`. Commit only intended files.
- Work on a branch; merge through a pull request with **rebase and merge**.

---

If `AGENTS.local.md` exists, read it too.
