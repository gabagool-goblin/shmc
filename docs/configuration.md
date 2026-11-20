# Configuration

shmc reads `~/.config/shmc/config.toml`, or `$XDG_CONFIG_HOME/shmc/config.toml` when set. Use `--config FILE` to specify another file.

On first launch, shmc asks for your Plex server URL and token, checks the connection and saves the config. Run `shmc setup` to repeat this process. Only `plex.server` and `plex.token` are required.

To find your Plex token, open a track in Plex Web, select **Get Info**, then **View XML**. The URL ends with `X-Plex-Token=...`.

The config contains your Plex token. shmc writes it and its `config.toml.bak` backup atomically with permissions set to `600`. With an `http://` server, the token is sent unencrypted. `shmc config --check` warns about this; use HTTPS or a VPN when connecting outside your local network.

## Config editor

Run `shmc config` to check your setup and open the editor. It has three tabs:

- **Config:** All settings, grouped by section. `*` marks values that differ from their defaults.
- **Theme:** Theme, colours, spacing and symbols, with a live preview. See [themes.md](themes.md).
- **Diagnostics:** Output from `shmc config --check`. Press `d` to run the checks again.

| Key | Action |
|---|---|
| `Enter` | Edit a setting |
| `←` / `→` or `h` / `l` | Change a number, toggle a value or cycle a choice |
| `/` | Filter settings |
| `[` / `]` | Switch groups |
| `1` / `2` / `3` / `Tab` | Switch tabs |
| `r` | Reset the selected setting |
| `R` | Reset the current tab |
| `s` | Show unsaved changes; press again to save |
| `q` | Quit, with confirmation if there are unsaved changes |

`plex.server`, `plex.token` and `plex.music_section` cannot be reset. The token is shown as `(changed)` in the unsaved changes list.

Changes are saved in place. Existing comments, key order and unknown settings are preserved. A running instance picks up changes on its next start.

`plex.music_section` opens a list of music libraries on your server. Press `t` to enter a library ID manually.

`mpv.extra_args` has its own editor: `a` adds an argument, `d` deletes one, `J` and `K` reorder them, and `Esc` finishes editing.

Settings that don't apply to the current configuration are dimmed, with an explanation in the details pane.

## Checking and listing

```sh
shmc config --check   # Check config, server, token, libraries, mpv, cava and terminal
shmc config --list    # List settings, values and defaults
```

`--check` exits with `0` if there are no failures, including when warnings are present. It exits with `1` otherwise.

`--list` doesn't require a terminal or write to disk. The token is shown as `(set)` or `(missing)`, so the output is safe to include in a bug report.

## All settings

These are the defaults shown by `shmc config --list`. Only include settings you want to change.

```toml
[plex]
server = "http://192.168.1.100:32400"
token = "your-plex-token"
music_section = ""            # Library ID; empty uses the first music library
allow_writes = false          # Let shmc change the server: ratings and radio
report_playback = false       # Send plays to Plex: play counts, last played

[mpv]
path = "mpv"                  # Executable path or name on $PATH
volume = 100                  # Startup volume, 0-100
extra_args = []               # e.g. ["--ao=pulse", "--replaygain=album"]

[album_art]
backend = "auto"              # auto, kitty, iterm2, sixel, halfblocks or none
cache_mb = 200                # Cover cache limit in MB; 0 = no cache
max_size_px = [0, 0]          # Maximum encoded cover size [width, height]; 0 = unlimited

[cava]
enabled = true                # Enable the visualizer; requires cava
path = "cava"
framerate = 60                # Frames per second, 1-120
source = ""                   # Audio source; empty uses the platform default
pause_grace_secs = 30         # Keep audio capture after pause; 0 releases immediately
style = "balanced"            # balanced, smooth, snappy or custom
# Only used with style = "custom":
noise_reduction = 77          # 0-100; higher values are smoother
autosens = true               # Automatically adjust gain
sensitivity = 100             # Gain, 1-300
lower_cutoff_hz = 50          # 10-1000
higher_cutoff_hz = 10000      # 1000-20000; must exceed lower_cutoff_hz

[ui]
header = "full"               # full or compact
volume_display = "both"       # meter, percent or both
rating_style = "five"         # five, one or off
mode_flags = "icons"          # icons or words
icons = "nerd"                # nerd or ascii
art_pct = 40                  # Queue art pane width, %; 0 = hidden
art_square = true             # Keep album art square
art_metadata = false          # Show title, artist and album under the cover
art_pad_px = 1                # Padding around album art, in pixels
visualizer_pct = 50           # Visualizer height, % of queue column; 0 = hidden
visualizer_mode = "adaptive"  # adaptive or spread
visualizer_bars = 20          # Maximum bars in spread mode
visualizer_anchor = "gap"     # bar or gap
visualizer_bar_width = 3      # Minimum bar width, in cells
visualizer_bar_gap = 1        # Minimum gap between bars, in cells
queue_columns = ["title", "artist", "album", "len"]  # Also track, year and codec
column_weights = [30, 24, 46] # Title, Artist and Album widths; 0 = hidden
playlist_numbering = "position"  # position or none
album_sort = "year"           # year, year_desc or title
scrolloff = 4                 # Rows kept visible beyond the cursor
auto_follow = true            # Keep the playing track at the top of the Queue
center_cursor = true          # Centre cursor when jumping to the playing track
prev_restart_secs = 3         # `<` restarts a track played longer than this; 0 always goes back
preview_tracks = 500          # Tracks loaded for playlist previews; 0 = disabled
preview_debounce_ms = 150     # Delay before loading a preview
confirm_playlist_over = 1000  # Confirm before queueing larger playlists; 0 = disabled

[theme]
name = "default"              # See themes.md for available themes
```

`[keys]` is covered in [keys.md](keys.md), and the remaining `[theme]` settings are in [themes.md](themes.md).

Invalid or out-of-range values are reported by `shmc config --check` and replaced with their defaults.

### Plex

With `report_playback = true`, shmc reports what it plays to Plex, so play counts, last played and smart playlists include it. A track counts once it has played for 30 seconds, or 90% of a shorter track.

With `allow_writes = true`, shmc can rate tracks and start radio, which creates play queues on the server. `:save` works either way.

### mpv

shmc starts mpv with `--no-config`, so your `mpv.conf` and scripts aren't loaded. Pass options through `mpv.extra_args` instead.

`--log-file` is ignored in `extra_args` because mpv logs stream URLs containing your Plex token.

### Album art and visualizer

With `album_art.backend = "auto"`, shmc tries Kitty, iTerm2, Sixel and half-block graphics, in that order.

Set `backend = "none"` or `art_pct = 0` to hide album art and give the Queue more space. Set `cava.enabled = false` or `visualizer_pct = 0` to hide the visualizer.

On macOS, the cover also appears in Now Playing in Control Center. This needs album art to be on and `album_art.cache_mb` above 0.

The visualizer's bar width and gap are minimums. Narrow panes show fewer bars rather than shrinking them.

- **adaptive:** Shows as many bars as fit, up to cava's limit of 96. Bars widen once all fit.
- **spread:** Shows up to `visualizer_bars`, then widens the bars or gaps according to `visualizer_anchor`.

`cava.style` controls bar movement. `smooth` is calmer, `snappy` is faster, and `custom` uses the individual settings below it. Changes take effect the next time cava starts.

### Queue columns

`queue_columns` controls which columns appear in the Queue and their order.

`column_weights` sets the relative widths of Title, Artist and Album in the Queue, Playlists and Search tables. The values don't need to add up to 100, but at least one must be above 0. Other columns use the space they need.

When the terminal is too narrow, columns are removed in this order: `codec`, `year`, then `track`.

The `/` filter still searches title, artist and album, even when those columns are hidden.

## Visualizer audio source

When `cava.source` is empty, cava uses the platform's default audio source:

- **Linux:** PipeWire monitor of the default output, falling back to PulseAudio.
- **macOS:** Core Audio tap. Requires macOS 14.2+ and cava 1.0+.

On macOS, the terminal running shmc needs **System Audio Recording** permission:

1. Open System Settings → Privacy & Security → Screen & System Audio Recording.
2. Enable your terminal, or add it with `+` if it isn't listed.
3. Quit and reopen the terminal.

`shmc config --check` reports when permission is missing.

If the Core Audio tap isn't available, use a loopback device instead:

```sh
brew install --cask blackhole-2ch
```

In Audio MIDI Setup, create a Multi-Output Device with your speakers first and BlackHole 2ch second. Set it as your system output, then configure:

```toml
[cava]
source = "BlackHole 2ch"
```

To inspect cava's configuration or errors, run:

```sh
# macOS
cava -p ~/Library/Caches/shmc/cava/config

# Linux
cava -p ~/.cache/shmc/cava/config
```

## Files

| | macOS | Linux |
|---|---|---|
| Config | `~/.config/shmc/` | `~/.config/shmc/` |
| Cache (album art, mpv socket) | `~/Library/Caches/shmc/` | `~/.cache/shmc/` |
| Logs (`shmc.log`, `mpv.log`) | `~/Library/Logs/shmc/` | `~/.local/state/shmc/` |

With `report_playback` or `allow_writes` on, `client-id` beside the config file identifies shmc to Plex. Audio is never written to disk. Album art is cached up to `album_art.cache_mb`, with least recently used covers removed first.

`shmc.log` rotates at 2 MB. `mpv.log` is replaced on each start. The Plex token is stripped from both logs before they're written and when displayed by `shmc log`.

Cache and log directories use permissions `700`, and files use `600`. If shmc can't create a private directory for mpv's socket, it refuses to start mpv rather than use a shared one.

## Command line

```sh
shmc                  # Start the player
shmc config           # Check and edit settings
shmc config --check   # Run checks only
shmc config --list    # List settings and defaults
shmc setup            # Re-enter server URL and token
shmc cache            # Show the cache size and log location
shmc cache clear      # Clear cached album art
shmc log [N]          # Show the last N log lines (60 by default)
shmc --version        # Show version and build commit
shmc --config FILE    # Use another config file
```

Unknown commands and options are rejected before execution, with suggestions for likely typos (e.g. `shmc confg`).

Exit codes:

- `0`: Success.
- `1`: Error, failed `--check` or unexpected mpv exit.
- `2`: Invalid command or arguments.
- `128 + n`: Process stopped by signal `n`.
