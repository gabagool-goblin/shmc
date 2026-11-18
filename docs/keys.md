# Keys and commands

Press `?` or `5` to open Help. It lists all current key bindings, including changes from `[keys]`. This page covers the defaults.

## Everywhere

| Key | Action |
|---|---|
| `q`, `Ctrl-C` | Quit (`q` in Help returns to the previous tab) |
| `1`–`5`, `Tab`, `Shift-Tab` | Switch tabs: Queue, Browser, Playlists, Search, Help |
| `?` | Help |
| `Space` | Pause / resume |
| `>`, `<` | Next / previous track (`<` restarts after `ui.prev_restart_secs`) |
| `.`, `,` | Volume up / down by 5 |
| `]`, `[` | Seek forward / back 5 seconds |
| `x` | Toggle repeat |
| `c` | Toggle single-track repeat |
| `z` | Toggle shuffle |
| `Z` | Reshuffle queue |
| `C` | Toggle consume |
| `S` | Sleep timer: 15, 30 or 60 minutes, or off |
| `r` | Rate the playing track |
| `:` | Open command line |

Shuffle reorders the queue, keeping the playing track first. Turning it off restores the original order. Tracks added while shuffle is on go to the end.

Consume removes a track after it plays. Skipping with `>` also removes the track.

The sleep timer pauses playback when it expires. The header shows the remaining time. Use `:sleep 20` for a custom duration or `:sleep off` to cancel it.

The header shows the playing track's rating: five stars, one star or none, set by `ui.rating_style`. The compact layout shows it after the title on the now-playing line, and under the cover when `ui.art_metadata` is on. Track lists show a rated track's rating after its title, `4★` or, with one star, `★`. With `plex.set_ratings = true`, `r` rates it: `1` to `5`, or `0` to clear; with one star, `r` toggles the star. Ratings live in Plex, so other Plex apps see them.

`m` on a track, album or artist lists the ones that sound like it, from Plex's sonic analysis, which needs Plex Pass. The list opens in the Search tab, closest first, with a Match column, and the usual keys work on it. Similar albums include the artist's own. `Esc` goes back a step at a time, ending at the tab where `m` was pressed.

On macOS, the media keys and the Control Center buttons work like shmc's own keys: play/pause like `Space`, next and previous like `>` and `<`, fast-forward and rewind seek 5 seconds, and stop like `:stop`. To leave the media keys alone, add `--input-media-keys=no` to `mpv.extra_args`.

## Lists

These keys work in all tab lists and Help, except where noted.

| Key | Action |
|---|---|
| `j`, `k`, arrows | Move down / up, wrapping at the ends |
| `PgDn`, `PgUp` | Page down / up |
| `gg`, `G`, `Home`, `End` | Jump to top / bottom |
| `g` + letter | Jump to the first row starting with that letter |
| `h`, `l`, `Left`, `Right` | Switch columns (Browser, Playlists) |
| `/` | Filter; starts a new search in Search. Not available in Help |
| `o` | Jump to the playing track (Queue, Browser, Playlists) |
| `v` | Select a range (Queue, Browser, Playlists and Search tracks) |

After `g`, the next letter always triggers a jump, even if it's bound to another action. For example, `g a` jumps rather than adding. Other keys keep their normal bindings, so `g 2` still switches tabs.

Filters match visible columns, ignoring case. While filtering, `Up` and `Down` move through the results. Press `Enter` to keep the filter or `Esc` to clear it. `Backspace` also clears an empty filter.

## Queue

| Key | Action |
|---|---|
| `Enter` | Play selected track |
| `d` | Remove track or selected range |
| `u` | Undo removal (up to 20, newest first) |
| `J`, `K` | Move track down / up |
| `D` | Clear queue (confirmation required) |
| `b` | Open track in Browser |

The playing track stays pinned to the top of the Queue. Moving the cursor or filtering temporarily disables this. Press `o` to restore it.

Set `ui.auto_follow = false` to disable pinning. In that case, `o` jumps to the playing track once.

`J` and `K` are disabled while shuffle is on or the queue is filtered.

Tracks that fail to play are skipped. After three consecutive failures, playback stops and displays the failed track. Press `>` to continue.

## Browser, Playlists and Search

| Key | Action |
|---|---|
| `Enter` | Replace queue and play |
| `a` | Add artist, album, playlist or track to the queue |
| `i` | Insert after the playing track |
| `b` | Open Search result in Browser |
| `Esc`, `Backspace` | Browser: go back one column; from artists, return to Queue |
| `Esc` | Search: go back a step |

`Enter` behaviour depends on the selection:

- **Album or playlist:** Play all tracks.
- **Track:** Play it and the remaining tracks in its album or playlist.
- **Artist:** Open it.
- **Search album:** Open it.

With a range selected using `v`, `a` adds it, `i` inserts it after the playing track, and `Enter` replaces the queue with it.

Playlists larger than `ui.confirm_playlist_over` require confirmation. Large playlists start playing after the first 200 tracks load. While the remaining tracks load, adding, inserting, undoing, shuffling and moving tracks are disabled.

Use `Up` and `Down` in the Search input to recall previous searches.

## Commands

Press `:` to open the command line.

| Command | Action |
|---|---|
| `:q`, `:quit` | Quit |
| `:n`, `:next`, `:p`, `:prev` | Next / previous track |
| `:play`, `:resume`, `:pause`, `:stop` | Play / resume, toggle pause, stop |
| `:seek N` | Seek N seconds; negative values go back |
| `:vol N` | Set volume (0–100); `+N` and `-N` adjust it |
| `:shuffle`, `:repeat`, `:single`, `:consume` | Toggle playback modes |
| `:reshuffle` | Reshuffle queue |
| `:sleep N`, `:sleep off` | Set sleep timer in minutes or cancel |
| `:queue`, `:browser`, `:playlists`, `:search`, `:help` | Switch tabs |
| `:playing` | Jump to playing track |
| `:clear` | Clear queue |
| `:save NAME` | Save queue as a new Plex playlist |

`:save` saves the entire queue in its current order, even when filtered. It only creates playlists; existing playlists are never overwritten or deleted. Duplicate names are rejected. Plex playlists store each track once, so repeated tracks are omitted.

## Changing keys

Use `[keys]` in `config.toml` to rebind actions. Each entry is `action = ["key", ...]`. An empty array unbinds an action; omitted actions keep their defaults.

```toml
[keys.global]
next_track = [">", "n"]
toggle_repeat = []

[keys.queue]
remove = ["d", "Delete"]

[keys.lists]
move_down = ["j", "Down", "C-n"]
```

Available sections: `global`, `lists`, `queue`, `browser`, `playlists`, `search` and `help`.

`lists` accepts movement actions only:

`move_up`, `move_down`, `page_up`, `page_down`, `top`, `bottom`, `column_left`, `column_right`.

These apply to every section that supports them. A section-specific binding takes precedence.

Key names match those shown in Help:

- Printable characters: `q`, `J`, `>`
- Control keys: `C-<char>`
- Special keys: `Space`, `Enter`, `Esc`, `Tab`, `S-Tab`, `BSpace`, `Up`, `Down`, `Left`, `Right`, `PgUp`, `PgDn`, `Home`, `End`, `Delete`
- Function keys: `F1`–`F12`
- Sequence: `gg` (the only supported sequence)

If `[keys]` contains an invalid section, action, key name or duplicate binding within a section, shmc falls back to the default keys. `shmc config --check` reports each problem.

`C-c` cannot be bound. `Ctrl-C` always quits, and command-line commands cannot be rebound. Changes take effect on the next start.

## Default bindings

| Action | Default | Sections |
|---|---|---|
| `quit` | `q`, `C-c` | global |
| `next_tab`, `prev_tab` | `Tab`, `S-Tab` | global |
| `tab_queue`, `tab_browser`, `tab_playlists`, `tab_search` | `1`, `2`, `3`, `4` | global |
| `tab_help` | `5`, `?` | global |
| `command_line` | `:` | global |
| `toggle_pause` | `Space` | global |
| `next_track`, `prev_track` | `>`, `<` | global |
| `volume_up`, `volume_down` | `.`, `,` | global |
| `seek_forward`, `seek_back` | `]`, `[` | global |
| `toggle_shuffle` | `z` | global |
| `reshuffle` | `Z` | global |
| `toggle_repeat` | `x` | global |
| `toggle_single` | `c` | global |
| `toggle_consume` | `C` | global |
| `sleep_timer` | `S` | global |
| `move_up`, `move_down` | `k`, `Up`; `j`, `Down` | queue, browser, playlists, search, help |
| `page_up`, `page_down` | `PgUp`; `PgDn` | queue, browser, playlists, search, help |
| `top`, `bottom` | `gg`, `Home`; `G`, `End` | queue, browser, playlists, search, help |
| `column_left`, `column_right` | `h`, `Left`; `l`, `Right` | browser, playlists |
| `activate` | `Enter` | browser, playlists, search |
| `add` | `a` | browser, playlists, search |
| `play_next` | `i` | browser, playlists, search |
| `visual` | `v` | queue, browser, playlists, search |
| `jump_to_playing` | `o` | queue, browser, playlists |
| `filter` | `/` | queue, browser, playlists |
| `play` | `Enter` | queue |
| `remove` | `d` | queue |
| `undo` | `u` | queue |
| `clear` | `D` | queue |
| `move_track_down`, `move_track_up` | `J`, `K` | queue |
| `show_in_browser` | `b` | queue, search |
| `search` | `/` | search |
| `back` | `Esc`, `BSpace` (browser); `Esc` (search); `Esc`, `q` (help) | browser, search, help |
