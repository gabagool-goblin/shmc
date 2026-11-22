# Changelog

All notable changes to shmc are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## Unreleased

### Added

- The queue is kept across restarts and comes back paused where it was, with its shuffle, repeat, single and consume modes (`ui.restore_queue`).
- When a dropped connection stops playback, shmc plays the failed track again once Plex answers.
- Loudness leveling from Plex's analysis, by track or by album (`mpv.loudness_leveling`, Plex Pass).
- Discover lists Plex's shelves above the stations, and Recently played, Top rated and Never played below them. Albums, artists and playlists open in place, `Esc` goes back, and `:refresh` loads new shelves.

### Changed

- Building from source needs Rust 1.90 or newer.

### Fixed

- A redirect to a deeply nested `blob:` URL can no longer crash shmc.

## 0.3.1 — 2026-11-21

### Fixed

- Radio no longer ends after its first tracks when another station was previewed or started; shmc starts the station again.

## 0.3.0 — 2026-11-20

### Added

- A Discover tab lists the library's radio stations and shows the highlighted one's tracks; `Enter` plays them (needs Plex Pass and `plex.allow_writes`).
- `s` starts track or artist radio from Plex (needs Plex Pass and `plex.allow_writes`).
- Radio keeps going as the queue nears its end, and the Queue tab shows the station's name.
- `m` lists tracks, albums or artists that sound like the selected one, from Plex's sonic analysis (needs Plex Pass).
- `w` picks tracks as stops and `W` builds a Sonic Adventure, a path of sonically close tracks through them (needs Plex Pass).
- `Esc` in the Search tab goes back a step at a time, ending at the tab where `m` was pressed.
- `v` selects a range in the Search tab's album view and similar list.
- Track lists show a rated track's rating after its title.
- `R` rates the playing track from any tab.

### Changed

- Discover is tab 2, so Browser, Playlists, Search and Help move to 3–6.
- The sleep timer moves from `S` to `t`.
- `b` now works on a playlist's tracks, and `o` in Discover and Search.
- `plex.allow_writes` replaces `plex.set_ratings`; `shmc config --check` names the old key.
- `r` rates the highlighted track instead of the playing one.
- Building from source needs Rust 1.88 or newer.

### Fixed

- Log lines written at the same moment no longer run together in `shmc.log`.
- On a narrow window, track tables no longer cut Title to a letter while Album stays wide.
- Crafted XML from a server can no longer stall shmc or exhaust its memory, and distrusted certificate authorities are no longer trusted.

## 0.2.1 — 2026-11-16

### Fixed

- The compact layout showed no rating for the playing track. It now shows it after the title on the now-playing line, and under the cover when `ui.art_metadata` is on.

## 0.2.0 — 2026-11-15

### Added

- `-v` prints the version, like `-V` and `--version`.
- On macOS, Now Playing in Control Center shows the album cover.
- Star ratings in the header; `r` rates the playing track when `plex.set_ratings` is on.
- `plex.report_playback` reports plays to Plex: play counts, last played and Now Playing.

### Changed

- Help and the key reference list the `:quit`, `:next`, `:prev` and `:resume` aliases.
- The sleep timer pauses playback instead of stopping it, so playback resumes where it left off.
- On macOS, the media keys act like shmc's own keys: previous restarts the track, next removes the skipped track when consume is on, and stop stops playback instead of quitting mpv.

## 0.1.1 — 2026-11-14

### Fixed

- Now Playing on macOS showed the stream URL, Plex token included, as the title of a track with no title tag. It now shows the track's title from Plex.

## 0.1.0 — 2026-11-14

First public release.
