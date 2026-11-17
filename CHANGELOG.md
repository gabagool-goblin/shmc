# Changelog

All notable changes to shmc are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## Unreleased

### Added

- `m` lists tracks that sound like the selected one, from Plex's sonic analysis (needs Plex Pass).
- Track lists show a rated track's rating after its title.

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
