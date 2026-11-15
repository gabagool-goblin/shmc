# Changelog

All notable changes to shmc are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## Unreleased

### Added

- `-v` prints the version, like `-V` and `--version`.

### Changed

- Help and the key reference list the `:quit`, `:next`, `:prev` and `:resume` aliases.
- The sleep timer pauses playback instead of stopping it, so playback resumes where it left off.
- On macOS, the media keys act like shmc's own keys: previous restarts the track, next removes the skipped track when consume is on, and stop stops playback instead of quitting mpv.

## 0.1.1 — 2026-11-14

### Fixed

- Now Playing on macOS showed the stream URL, Plex token included, as the title of a track with no title tag. It now shows the track's title from Plex.

## 0.1.0 — 2026-11-14

First public release.
