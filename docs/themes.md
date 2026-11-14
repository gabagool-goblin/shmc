# Themes

Set `theme.name` to a built-in theme or the ID of a [theme file](#theme-files):

- `default`
- `tokyo-night`
- `catppuccin-mocha`
- `catppuccin-latte`
- `gruvbox-dark`
- `nord`
- `dracula`

All built-in themes except `default` use their own background colour. `default` uses the terminal background.

Settings in `[theme]` override the selected theme:

```toml
[theme]
name = "catppuccin-mocha"
accent = "#f5c2e7"
background = "none"     # Use the terminal background
block_gap = 1
progress_filled = "▰"
```

The Theme tab in `shmc config` previews changes as you edit. Press `Enter` on the theme name to preview and select another theme. Press `x` to export the current theme, including overrides, to a theme file.

Selecting another theme clears existing overrides. Press `s` to review them before they're removed. Export first if you want to keep them. Exports only overwrite files previously created by shmc.

Invalid values are reported by `shmc config --check`, which identifies the affected key without displaying its value. The theme's default value is used instead.

## Colours

All colours use `#rrggbb` format.

| Key | Used for |
|---|---|
| `accent` | Cursor, active tab, progress fill and icons |
| `on_accent` | Text on accent backgrounds |
| `text` | Normal text |
| `dim` | Dimmed text and inactive tabs |
| `secondary` | Help keys, header artist and filter matches |
| `error` | Errors |
| `border` | Pane and box borders |
| `highlight` | Cursor in an unfocused pane |
| `viz_bottom`, `viz_mid`, `viz_top` | Visualizer colour ramp, bottom to top |
| `background` | Background colour; `none` uses the terminal background |

## Spacing and symbols

| Key | Description | Default |
|---|---|---|
| `block_gap` | Empty rows between header, tabs, content and progress bar (0–3) | `0` |
| `inset` | Space between box borders and content (0–3) | `1` |
| `borders` | `box`, `rounded`, `double`, `lines` or `none` | `box` |
| `progress_bar` | `solid`, `line`, `dot`, `plain`, `angled` or `rounded` | `solid` |
| `progress_filled`, `progress_empty`, `progress_head` | One character each for the played, unplayed and last played cell, or `solid` for a plain coloured cell | From `progress_bar` |
| `progress_start`, `progress_end` | One character for each end of the bar, or `solid` for none | From `progress_bar` |
| `ellipsis` | Character used for truncated text | `…` |
| `scroll_up`, `scroll_down`, `scroll_thumb` | Scrollbar characters | `▲` `▼` `█` |
| `volume_meter` | Preset or custom meter characters | `blocks` |
| `visualizer` | Preset or custom visualizer characters | `blocks` |

`borders = "lines"` uses separators between sections without drawing boxes. `none` removes borders.

Progress bar presets:

- `solid`: Plain coloured cells.
- `line`: `━` and `─`.
- `dot`: Line with a `●` head.
- `plain`: `=` and `-` with a `>` head.
- `angled`: Solid bar with a pointed head.
- `rounded`: Solid bar with rounded caps.

`angled` and `rounded` require Powerline glyphs, included in Nerd Fonts.

Volume meter presets:

- `blocks`: `▁▂▃▄▅▆▇█`
- `squares`: `■` and `·`
- `outline`: `■` and `□`
- `plain`: `#` and `-`

Visualizer presets:

- `blocks`: `▁▂▃▄▅▆▇█`
- `ascii`: `_.-=+*#@`

Custom `volume_meter` and `visualizer` values use eight characters, ordered from lowest to highest. The volume meter also accepts two characters for lit and unlit states.

Symbols must be single-column characters. Wide characters (such as CJK and emoji) and combining characters aren't supported.

With `ui.icons = "ascii"` or a non-UTF-8 locale, non-ASCII symbols are replaced with their defaults.

Glyph rendering depends on the terminal font. For example, `●` may sit lower than `━`, while `╸` aligns with it. Some terminals render `■`, `□`, `·` and block characters as double-width, which can distort meters. Use `plain` or `ascii` if that happens.

## Theme files

Theme files are named `<id>.toml` and stored in a `themes/` directory beside the config file:

- Default config: `~/.config/shmc/themes/`
- Custom config: beside the file specified by `--config`

Theme IDs must start with a lowercase letter or digit and contain only lowercase letters, digits, `-` and `_`. Set `theme.name` to the ID to use it.

A theme file with the same ID as a built-in theme replaces it.

The built-in themes are in [`themes/`](../themes) and opt-in ones in [`themes/extra/`](../themes/extra). Copy a file into your own `themes/` directory to use or change it.

Example:

```toml
name = "Catppuccin Mocha"   # Display name in the chooser
author = "…"                # Optional
variant = "dark"            # dark or light; informational
accent = "#89b4fa"
on_accent = "#1e1e2e"
text = "#cdd6f4"
background = "#1e1e2e"

[spacing]
block_gap = 1
borders = "rounded"

[symbols]
progress_bar = "line"
progress_head = "▶"
volume_meter = "squares"

[styles]
current = { fg = "on_accent", bg = "accent", bold = true }
```

All keys are optional; missing ones use the defaults. Invalid values also use the defaults and are reported. Theme files must be valid TOML, no larger than 64 KiB, regular files (not symlinks), and named with a valid ID.

Unknown keys and tables are ignored. Theme files contain configuration only; nothing in them is executed.

### Styles

`[styles]` is available in theme files only. It controls the appearance of individual UI elements:

`accent`, `secondary`, `text`, `dim`, `error`, `warn`, `border`, `current`, `current_unfocused`, `tab_active`, `tab_inactive`, `progress_fill` and `progress_track`.

Each style is independent. For example, changing `current` doesn't affect `tab_active`.

Styles accept:

- `fg`, `bg`: A colour key or `#rrggbb` value.
- `bold`, `italic`, `underline`, `reversed`, `dim`: `true` to enable or `false` to disable.

Only specified properties are changed. For example, `{ bold = false }` removes bold without changing the colours.

```toml
[styles]
tab_active = { italic = true }
error = { fg = "error", underline = true }
dim = { fg = "#7f849c", italic = true }
```

Invalid style entries are skipped and reported. Other entries in the file still apply.

## Credits

Built-in themes use shmc's own colour mappings based on these palettes. They aren't copies of the original theme files and aren't endorsed by their authors.

- [Catppuccin](https://github.com/catppuccin/catppuccin) (MIT)
- [Gruvbox](https://github.com/morhetz/gruvbox) by morhetz (MIT/X11)
- [Nord](https://github.com/nordtheme/nord) (MIT)
- [Dracula](https://github.com/dracula/dracula-theme) (MIT)
- [Tokyo Night](https://github.com/enkia/tokyo-night-vscode-theme) (MIT)

The `default` palette is based on the preview screenshot from [rmpc](https://github.com/mierak/rmpc) (BSD-3-Clause).
