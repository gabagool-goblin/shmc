# Troubleshooting

Start with `shmc config --check`. It checks your configuration, Plex connection and token, music libraries, mpv, cava and terminal image support. It also suggests fixes where possible.

When reporting a bug, include the output of all three commands. None of them prints your Plex token.

```sh
shmc config --check
shmc config --list
shmc log
```

## macOS refuses to open shmc

A binary downloaded with a browser is quarantined, and macOS blocks it because it is not notarized. The installer downloads with curl, which does not quarantine. For a manual download, clear the flag with `xattr -d com.apple.quarantine shmc` or allow it in System Settings > Privacy & Security.

To check that a release tarball was built by this repository's release workflow, run `gh attestation verify shmc-*.tar.gz --repo gabagool-goblin/shmc`.

## Icons show as boxes

The default icons require a [Nerd Font](https://www.nerdfonts.com). shmc cannot detect which font your terminal uses, so install one or set `ui.icons = "ascii"`. ASCII icons are also used automatically with non-UTF-8 locales.

## No album art

`shmc config --check` reports the image protocol detected in your terminal. If album art is missing or garbled with `album_art.backend = "auto"`, set it to a supported protocol: `kitty`, `iterm2` or `sixel`.

You can also use `halfblocks`, which works without image protocol support but produces lower-resolution art.

## The visualizer doesn't work

On macOS, the terminal needs System Audio Recording permission. Restart it after granting access.

On Linux, cava uses the PipeWire monitor of the default output, falling back to PulseAudio. See [Visualizer audio source](configuration.md#visualizer-audio-source).

The visualizer only runs while audio is playing and the Queue tab is visible. The pane is empty otherwise.

## A track does not play

shmc skips tracks mpv cannot play and stops after three consecutive failures. Check the second half of `shmc log` for mpv's error. If the server is unreachable, `shmc config --check` should report it.

## Keys do not match the config

If any entry in `[keys]` is invalid, shmc ignores the entire table and uses the default bindings. `shmc config --check` reports each problem.

Key changes take effect the next time shmc starts.
