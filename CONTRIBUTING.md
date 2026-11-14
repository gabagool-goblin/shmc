# Contributing

Run the following before submitting changes:

```sh
cargo build --release
cargo test --release
cargo clippy --release --all-targets -- -D warnings
cargo fmt
```

Tests start a real mpv instance with `--ao=null`, so mpv must be installed. They do not connect to a Plex server. Clippy must pass without warnings.

Add user-visible changes to the `## Unreleased` section of [CHANGELOG.md](CHANGELOG.md). Update the README or `docs/` as well when behaviour or usage changes.

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) with a short imperative subject, for example `fix: handle empty playlist responses`. Pull requests are merged with rebase and merge, so each commit should stand on its own.

For anything larger than a small fix, open an issue first to agree on the approach.

## Architecture

```text
shmc
 ├─ Plex HTTP API: browsing, playlists, search, artwork and stream URLs
 ├─ mpv JSON IPC: playback and queue management
 └─ cava: visualizer bar heights (raw 8-bit output), from PipeWire or
          PulseAudio on Linux or a Core Audio tap on macOS
```

mpv runs for the entire session, and shmc keeps its queue in sync with mpv's playlist. mpv handles gapless playback and advancing to the next track.

Stream URLs contain the Plex token, so shmc sends them to mpv over its IPC socket rather than passing them on the command line. mpv's log is piped through a filter that removes the token before writing to disk. shmc also redacts the token from its own logs and output.
