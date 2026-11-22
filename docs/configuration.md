# Configuring JustMux

JustMux works out of the box without creating or editing any configuration file. When you want to customize startup defaults—display mode, palette, composer mode, live previews, emblem animation, or mouse support—you can use a TOML configuration file, environment variables, CLI flags, or the interactive `/config` modal.

## Quickstart

Generate a documented starter file cleanly:

```sh
mkdir -p ~/.config/justmux
jm --print-config > ~/.config/justmux/config.toml
```

Verify what JustMux loaded without launching the TUI:

```sh
jm doctor
```

`jm doctor` prints:

- `Config:` the resolved file path followed by `(loaded)` or `(default; file not created)`
- `Screen mode:` `fullscreen` or `inline`
- `Theme:` active preset and detected color capability (`truecolor`, `ANSI`, or `NO_COLOR`)

## Lookup Order & Precedence

### Config file resolution

JustMux locates its config file in this order:

1. Explicit `-c <PATH>` / `--config <PATH>` CLI flag (must exist and be readable)
2. `JUSTMUX_CONFIG` environment variable if set and non-empty
   - Special case: `JUSTMUX_CONFIG=/dev/null` skips reading any user config file and loads built-in defaults cleanly (useful for scripts and tests). Any other missing `JUSTMUX_CONFIG` path returns an error.
3. `$XDG_CONFIG_HOME/justmux/config.toml` (when `$XDG_CONFIG_HOME` is an absolute path)
4. `$HOME/.config/justmux/config.toml`

If no `--config` / `JUSTMUX_CONFIG` is provided and the default path does not exist yet, JustMux runs silently with built-in defaults without auto-creating `~/.config/justmux/config.toml`.

### Setting precedence (highest to lowest)

1. **CLI flags**: `--fullscreen`, `--inline`, `--theme <THEME>`
2. **Environment variables**: `JUSTMUX_SCREEN_MODE`, `JUSTMUX_THEME`, `JUSTMUX_EMBLEM_MODE`, `JUSTMUX_EMBLEM_SPIN`, `JUSTMUX_LANGUAGE`, `JUSTMUX_LANG`, `NO_COLOR`, `TERM=dumb`, and locale (`LC_ALL`, `LC_CTYPE`, `LANG`) when non-UTF-8
3. **`config.toml`**: settings parsed from the resolved TOML file
4. **Built-in defaults**

> [!NOTE]
> Setting `NO_COLOR` (any non-empty value) or `TERM=dumb` forces `no_color = true` and downgrades the default `aurora` theme to `classic` ANSI unless you explicitly select a theme via `config.toml`, `JUSTMUX_THEME`, or `--theme`.

## Configuration Reference

Section headers are optional (`[ui]`, `[general]`, `[display]`, and `[terminal]` are all accepted). Comments start with `#`.

| Key | Type | Default | Accepted Values / Aliases | Effect Timing |
| --- | --- | --- | --- | --- |
| `screen_mode` | string | `"fullscreen"` | `"fullscreen"` (`"full"`, `"alt"`), `"inline"` (`"native"`, `"compact"`) | Startup / next launch |
| `theme` | string | `"aurora"` | `"aurora"`, `"codex"`, `"claude"`, `"matrix"`, `"classic"` (plus full names like `"Antigravity Aurora"`, `"Codex Emerald"`, `"Claude Warmth"`, `"Cyber Matrix"`) | Immediate / live |
| `composer_mode` | string | `"command-jump"` | `"command-jump"` (`"command"`, `"jump"`, `"default"`), `"live-radar"` (`"radar"`), `"quick-launch"` (`"quick"`, `"launch"`) | Immediate / live |
| `live_preview` | bool | `true` | `true`, `false` | Immediate / live |
| `emblem_mode` | string / bool | `"spin"` | `"static"` (`"off"`), `"spin"` (rotating Braille cutout), `"inverted-spin"` (`"inverted"`, `"reverse-spin"` — rotating Braille inverted core); legacy booleans `true`/`false` and aliases `"hydra_mode"`, `"emblem_spin"`, `"hydra_spin"`, `"rotate_emblem"`, `"animate_emblem"` accepted; env `JUSTMUX_EMBLEM_MODE` / `JUSTMUX_EMBLEM_SPIN` | Immediate / live |
| `mouse` | bool | `true` | `true`, `false` (mouse capture is only active when `screen_mode = "fullscreen"`) | Startup / next launch |
| `inline_height` | integer | `16` | `8` through `40` rows (out-of-range values inside `config.toml` return a validation error) | Startup / next launch |
| `ascii` | bool | `false` | `true`, `false` | Immediate / live |
| `no_color` | bool | `false` | `true`, `false` | Immediate / live |
| `language` | string | `"en"` | `"en"` (`"english"`), `"zh"` (`"zh-cn"`, `"chinese"`, `"中文"`); alias `"lang"`; env `JUSTMUX_LANGUAGE` / `JUSTMUX_LANG` | Immediate / live |

## Interactive Editing & Saving (`/config` and `/theme`)

You do not need to edit TOML by hand (in the Home workspace, press `Tab` or click `[ ▸ Resume ] [ ✎ Rename ] [ ✕ Kill ] [ ◫ Split ]` to cycle workspace actions across the matrix, and `↑`/`↓` onto `+ New window` or `+ New session` to create resources; interactive subviews render inside the same unified studio chrome):

- Type `/config` (or aliases `/settings`, `/prefs`) in the composer to open the **JustMux Configuration** visual inspector (the upper card places clear explanations in the left half and rich, live-updating TUI graphical diagrams in the right half—including dual fullscreen/inline terminal monitors, 24-bit TrueColor spectrum swatches & mascot showcase, interactive mouse hit-zone blueprint, a framed 3-card Braille medallion gallery across `static` / `spin` / `inverted-spin`, and a proportional `8..=40` viewport gauge—alongside the interactive setting list):
  - `↑` / `↓`: select a setting
  - `←` / `→` / `Enter` / `Space`: change, cycle (`emblem_mode`, `theme`, `composer_mode`), or toggle the setting immediately
  - `s`: save the active configuration to your resolved `config.toml` path (creating parent directories if needed)
  - `Esc`: close the modal (terminal-level settings like `screen_mode`, `mouse`, and `inline_height` display `"Takes effect on next jm launch (press s to save)"`)
- Inside `/theme`:
  - `↑` / `↓`: live-preview themes
  - `Enter`: apply the highlighted preset for the current session
  - `s`: apply the highlighted preset **and** persist it to `config.toml`
  - `Esc`: revert to the previous theme

## Mouse Behavior & Safety

When `screen_mode = "fullscreen"` and `mouse = true` (default):

- Scroll wheel inside the `Window Preview` or picker preview cards scrolls through captured tmux scrollback history without changing the selected session, window, or pane (use `PageUp` / `PageDown`, or `Ctrl+U` / `Ctrl+D` / `Home` / `End` on an empty bar, for keyboard paging).
- Scroll wheel outside preview cards navigates the `/` palette drawer, `@` target menu, `Live Radar` sessions, `Session×Window` matrix rows, `/pane` / `/split` / `/theme` / `/config` lists, and `/kill` confirmation card.
- Left-clicking a session header or window row in the Home matrix selects it (and activates according to the active command mode); left-clicking a row in `Live Radar` or `/pane` highlights that row, and clicking an already selected row activates it.
- Mouse movement/hover events are ignored so moving your cursor causes zero terminal redraw traffic.
- JustMux always disables mouse capture (`DisableMouseCapture`) before leaving the alternate screen or calling `exec` to hand control to native `tmux`, and never enables mouse capture in `--inline` mode.

## Copy-Paste Presets

### Default Fullscreen Studio (spacious previews + mouse)

```toml
[ui]
theme = "aurora"
composer_mode = "command-jump"
live_preview = true
emblem_mode = "spin"

[terminal]
screen_mode = "fullscreen"
mouse = true
```

### Native Inline & Shell Scrollback

```toml
[ui]
theme = "codex"
composer_mode = "command-jump"
live_preview = true
emblem_mode = "spin"

[terminal]
screen_mode = "inline"
mouse = false
inline_height = 18
```

### Low-Bandwidth SSH / Minimal Output

```toml
[ui]
theme = "classic"
live_preview = false
emblem_mode = "static"
ascii = true

[terminal]
screen_mode = "inline"
mouse = false
inline_height = 14
```

### Live Radar Workspace Launcher

```toml
[ui]
theme = "claude"
composer_mode = "live-radar"
live_preview = true
emblem_mode = "inverted-spin"

[terminal]
screen_mode = "fullscreen"
mouse = true
```
