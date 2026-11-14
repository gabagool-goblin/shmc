# shmc

A terminal music player for Plex, written in Rust and built with [Ratatui](https://ratatui.rs/). Inspired by [rmpc](https://github.com/mierak/rmpc). Not affiliated with or endorsed by Plex Inc.

- Browse artists, albums, tracks and playlists; search your music library.
- Gapless playback, shuffle, repeat, single and consume modes.
- Queue undo, range selection and saving queues as Plex playlists with `:save`.
- Album art via Kitty, iTerm2, Sixel or half-block graphics.
- Custom themes, key bindings and an interactive config editor.

## Preview

[![Queue](docs/images/queue.webp)](docs/images/queue.webp)

[![Browser](docs/images/browser.webp)](docs/images/browser.webp)

Compact layout with the `gruvbox-dark` theme and track details below the cover:

[![Compact layout](docs/images/compact.webp)](docs/images/compact.webp)

## Requirements

- Plex Media Server with a music library.
- [mpv](https://mpv.io) 0.36 or newer. Some older distributions ship an earlier version.
- [cava](https://github.com/karlstav/cava) (optional, for the visualizer).
- A terminal supporting at least 256 colours.
- A [Nerd Font](https://www.nerdfonts.com) for the default icons. Set `ui.icons = "ascii"` to use ASCII icons instead.
- [Rust](https://rustup.rs), git and a C compiler (for building from source).

## Install

Install mpv and cava:

```sh
# macOS
brew install mpv cava

# Debian / Ubuntu
sudo apt install mpv cava

# Fedora
sudo dnf install mpv cava
```

Run the installer:

```sh
curl -fsSL https://raw.githubusercontent.com/gabagool-goblin/shmc/main/scripts/install.sh | sh
```

The installer downloads a prebuilt binary for macOS on Apple Silicon or Linux on x86_64 and arm64. Downloads are verified against the release's `SHA256SUMS` file.

On other platforms, or when using `--source`, the installer builds from source. This requires Rust, git and a C compiler. On macOS, install the compiler tools with `xcode-select --install`. On Debian or Ubuntu, install `build-essential`; on Fedora, install `gcc`.

The default install location is `~/.local/bin`. Set a different location with `install.sh DIR` or `PREFIX=DIR`. Use `--version x.x.x` to install a specific release. Pass options through the pipe with `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/gabagool-goblin/shmc/main/scripts/install.sh | sh -s -- --version 0.1.0
```

From an existing checkout, the installer builds the local source:

```sh
git clone https://github.com/gabagool-goblin/shmc
cd shmc
./scripts/install.sh
```

Alternatively, install with Cargo:

```sh
cargo install --git https://github.com/gabagool-goblin/shmc
```

## Getting started

Run `shmc`. On first launch, enter your Plex server URL and token. Settings are saved to `~/.config/shmc/config.toml`.

See [Configuration](docs/configuration.md) for help finding your token.

| Key | Action |
|---|---|
| `2` | Browser |
| `h` `j` `k` `l` / arrows | Navigate |
| `Enter` | Play selection |
| `a` / `i` | Add to queue / play next |
| `Space` | Pause / resume |
| `>` / `<` | Next / previous |
| `/` | Filter |
| `?` | All key bindings |
| `q` | Quit |

Run `shmc config` to edit settings and check your setup. Use `shmc config --check` to run checks without opening the editor.

## Documentation

- [Configuration](docs/configuration.md)
- [Keys and commands](docs/keys.md)
- [Themes](docs/themes.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)

## Credits

shmc is an independent implementation inspired by [rmpc](https://github.com/mierak/rmpc), particularly its layout. Theme credits are listed in [themes.md](docs/themes.md#credits).

## License

BSD 3-Clause. See [LICENSE](LICENSE).

## AI disclosure

AI tools were used during development for code, documentation and debugging. Changes are reviewed and tested before merging. The project comes with no warranty; use it at your own risk.
