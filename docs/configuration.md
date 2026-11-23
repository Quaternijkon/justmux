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
- `Language:` active UI language (`en` or `zh`)
- `Popup keybind:` whether JustMux's opt-in `Ctrl+b j` popup block is installed in your `tmux.conf`
- `Tmux comfort:` how many of the 5 opt-in comfort settings are enabled in your `tmux.conf` (or `disabled`)

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
2. **Environment variables**: `JUSTMUX_SCREEN_MODE`, `JUSTMUX_THEME`, `JUSTMUX_LIVE_STREAM`, `JUSTMUX_EMBLEM_MODE`, `JUSTMUX_EMBLEM_SPIN`, `JUSTMUX_LANGUAGE`, `JUSTMUX_LANG`, `NO_COLOR`, `TERM=dumb`, and locale (`LC_ALL`, `LC_CTYPE`, `LANG`) when non-UTF-8
3. **`config.toml`**: settings parsed from the resolved TOML file
4. **Built-in defaults**

> [!NOTE]
> Setting `NO_COLOR` (any non-empty value) or `TERM=dumb` forces `no_color = true` and downgrades the default `aurora` theme to `classic` ANSI unless you explicitly select a theme via `config.toml`, `JUSTMUX_THEME`, or `--theme`.

### Environment variables for storage & tmux.conf paths

In addition to `JUSTMUX_CONFIG`, JustMux recognizes these optional path overrides:

| Environment Variable | Default Resolution | Used By |
| --- | --- | --- |
| `JUSTMUX_TMUX_CONF` | Existing `~/.tmux.conf` → `$XDG_CONFIG_HOME/tmux/tmux.conf` → `~/.config/tmux/tmux.conf` → `~/.tmux.conf` (`/dev/null` disables) | Opt-in `tmux_popup`, `[tmux]` comfort block sync, and `[Reload ~/.tmux.conf]` |
| `JUSTMUX_SNAPSHOT_DIR` | `$XDG_CONFIG_HOME/justmux/snapshots` (if absolute) → `~/.config/justmux/snapshots` | Workspace templates saved and restored via `/snapshot` |
| `JUSTMUX_LOG_DIR` | `$XDG_DATA_HOME/justmux/logs` (if absolute) → `~/justmux-logs` | Plain-text log exports (`[Log]`) and live `pipe-pane` streams (`[Pipe]`) |

## Configuration Reference

Section headers are optional (`[ui]`, `[general]`, `[display]`, `[terminal]`, and `[tmux]` are all accepted). Comments start with `#`.

| Key | Type | Default | Accepted Values / Aliases | Effect Timing |
| --- | --- | --- | --- | --- |
| `screen_mode` | string | `"fullscreen"` | `"fullscreen"` (`"full"`, `"alt"`), `"inline"` (`"native"`, `"compact"`) | Startup / next launch |
| `theme` | string | `"aurora"` | `"aurora"`, `"codex"`, `"claude"`, `"matrix"`, `"classic"` (plus full names like `"Antigravity Aurora"`, `"Codex Emerald"`, `"Claude Warmth"`, `"Cyber Matrix"`) | Immediate / live |
| `composer_mode` | string | `"command-jump"` | `"command-jump"` (`"command"`, `"jump"`, `"default"`), `"live-radar"` (`"radar"`), `"quick-launch"` (`"quick"`, `"launch"`) | Immediate / live |
| `live_preview` | bool | `true` | `true`, `false` (alias `preview`) | Immediate / live |
| `live_stream` | bool | `true` | `true`, `false` (section `[ui]`, aliases `auto_refresh`, `stream_preview`; env `JUSTMUX_LIVE_STREAM` accepting `true`/`false`/`1`/`0`/`on`/`off`/`yes`/`no`). Adaptively refreshes Fullscreen previews (`400ms`), Mission Control `/expose` (`250ms`), and Live PIP (`80ms`) using content-hash dirty checking so unchanged output skips redraws; **strictly disabled in `--inline` mode** so inline terminals emit 0 idle bytes | Immediate / live |
| `emblem_mode` | string / bool | `"spin"` | `"static"` (`"off"`), `"spin"` (rotating Braille cutout), `"inverted-spin"` (`"inverted"`, `"reverse-spin"` — rotating Braille inverted core); legacy booleans `true`/`false` and aliases `"hydra_mode"`, `"emblem_spin"`, `"hydra_spin"`, `"rotate_emblem"`, `"animate_emblem"` accepted; env `JUSTMUX_EMBLEM_MODE` / `JUSTMUX_EMBLEM_SPIN` | Immediate / live |
| `mouse` | bool | `true` | `true`, `false` (mouse capture is only active when `screen_mode = "fullscreen"`) | Startup / next launch |
| `inline_height` | integer | `16` | `8` through `40` rows (out-of-range values inside `config.toml` return a validation error) | Startup / next launch |
| `ascii` | bool | `false` | `true`, `false` | Immediate / live |
| `no_color` | bool | `false` | `true`, `false` | Immediate / live |
| `language` | string | `"en"` | `"en"` (`"english"`), `"zh"` (`"zh-cn"`, `"chinese"`, `"中文"`); alias `"lang"`; env `JUSTMUX_LANGUAGE` / `JUSTMUX_LANG` | Immediate / live |
| `tmux_popup` | bool | `false` | `true`, `false` (section `[tmux]`, aliases `tmux_keybind`, `popup_keybind`; path override `JUSTMUX_TMUX_CONF`) | Immediate (`.tmux.conf` + live tmux bind when toggled in `/config`) |
| `tmux_mouse` | bool | `false` | `true`, `false` — writes `set -g mouse on` into the managed comfort block for native tmux mouse scrolling, click focus, and pane border dragging | Immediate (`.tmux.conf` + live `set-option -g` when toggled in `/config`) |
| `tmux_history_limit` | integer | `0` | `0` (off/unmanaged) or any positive `u32` (in `/config`, cycles `off` → `10000` → `50000` → `100000` → `off`; TOML underscores like `50_000` accepted) — writes `set -g history-limit <N>` | Immediate (`.tmux.conf` + live `set-option -g` when changed in `/config`) |
| `tmux_base_index_one` | bool | `false` | `true`, `false` — writes `set -g base-index 1` and `set -gw pane-base-index 1` so windows and panes number from 1 instead of 0 | Immediate (`.tmux.conf` + live `set-option -g` & `set-window-option -g` when toggled in `/config`) |
| `tmux_renumber_windows` | bool | `false` | `true`, `false` — writes `set -g renumber-windows on` so closing a window closes numbering gaps automatically | Immediate (`.tmux.conf` + live `set-option -g` when toggled in `/config`) |
| `tmux_zero_escape_time` | bool | `false` | `true`, `false` — writes `set -sg escape-time 0` to eliminate `Esc` key delay in Vim, Neovim, and fullscreen TUIs | Immediate (`.tmux.conf` + live `set-option -sg` when toggled in `/config`) |

### Opt-in `~/.tmux.conf` Managed Blocks & `[Reload ~/.tmux.conf]`

JustMux **never** edits or creates `~/.tmux.conf` unless you explicitly enable `tmux_popup` or one of the 5 comfort settings inside `/config`:

- **Popup keybind block** (`# >>> justmux managed popup keybind >>>` .. `# <<< justmux managed popup keybind <<<`) holds only the `Ctrl+b j` binding.
- **Comfort settings block** (`# >>> justmux managed tmux comfort >>>` .. `# <<< justmux managed tmux comfort <<<`) holds only the enabled `tmux_*` ergonomics lines. When all 5 comfort options are set back to `false` / `0`, JustMux removes the comfort block cleanly without leaving blank-line clutter (and never creates `~/.tmux.conf` if it does not exist).
- **`tmux_reload` (`[Reload ~/.tmux.conf]`)** — the `tmux_reload` action row in `/config` runs native `tmux source-file <path>` against the active tmux server so any manual edits to your `~/.tmux.conf` take effect immediately without leaving JustMux (you can also type `reload tmux` on the Home composer).

## Interactive Editing & Saving (`/config` and `/theme`)

You do not need to edit TOML by hand (in the Home workspace, press `Tab` or click `[ ▸ Resume ] [ ✎ Rename ] [ ✕ Kill ] [ ◫ Split ]` to cycle workspace actions across the matrix, click `[ ⏏ Detach ]` when inside tmux or `[ Panes ]` / `[ Layout ]` for quick controller views, and `↑`/`↓` onto `+ New window` or `+ New session` to create resources; interactive subviews render inside the same unified studio chrome):

- Type `/config` (or aliases `/settings`, `/prefs`) in the composer to open the **JustMux Configuration** visual inspector (the upper card places clear explanations in the left half and rich, live-updating TUI graphical diagrams in the right half—including dual fullscreen/inline terminal monitors, 24-bit TrueColor spectrum swatches & mascot showcase, interactive mouse hit-zone blueprint, a framed 3-card Braille medallion gallery across `static` / `spin` / `inverted-spin`, a proportional `8..=40` viewport gauge, and dedicated diagrams for `live_stream`, `tmux_popup`, all 5 `tmux_*` comfort switches, and `[Reload ~/.tmux.conf]`—alongside the interactive setting list):
  - `↑` / `↓`: select a setting or action row
  - `←` / `→` / `Enter` / `Space`: change, cycle (`emblem_mode`, `theme`, `composer_mode`, `tmux_history_limit`), toggle (`live_stream`, `tmux_popup`, `tmux_mouse`, `tmux_base_index_one`, `tmux_renumber_windows`, `tmux_zero_escape_time`), or trigger `[Reload]` (`tmux_reload`) immediately
  - `s`: save the active configuration to your resolved `config.toml` path (creating parent directories if needed)
  - `Esc`: close the modal (terminal-level settings like `screen_mode`, `mouse`, and `inline_height` display `"Takes effect on next jm launch (press s to save)"`)
- Inside `/theme`:
  - `↑` / `↓`: live-preview themes
  - `Enter`: apply the highlighted preset for the current session
  - `s`: apply the highlighted preset **and** persist it to `config.toml`
  - `Esc`: revert to the previous theme

## Mouse Behavior & Safety

When `screen_mode = "fullscreen"` and `mouse = true` (default):

- **Top IDE Menubar & dirty-checked hover:** Click `[⌂ Studio]`, `[⊞ Expose]`, `[◫ Split]`, `[▦ Layout]`, `[≡ Menu ▾]`, `[⌨ PIP]`, `[🎨]`, `[中/EN]`, `[⚙]`, or `[⏏]` directly; hovering controls updates hover highlights and micro-tooltips only when the hovered hit-target ID actually changes—moving the cursor over empty space emits **zero redraws and zero bytes**.
- **Click, double-click, right-click & middle-click:**
  - Single left-click focuses a session, window, pane, or Exposé tile and refreshes its live preview without accidentally jumping into tmux; a true **`<= 350ms` double-click** enters tmux immediately.
  - **Right-click** on any session, window, pane, preview card, Split Studio tile, Exposé tile, or background opens a clamped 2D drop-shadow context menu (dismiss on `Esc` or clicking outside).
  - **Middle-click** opens the `yes` kill confirmation for the clicked session, window, pane, or Exposé tile (or removes the clicked slot in Split Studio).
- **VS Code Drag & Drop & draggable sashes (`>= 2` cell drag threshold):**
  - Drag a window across sessions (`move-window`), onto another window in the same session (`swap-window`), or onto the **Left / Right / Top / Bottom / Center edge dock zones** of a Preview Card, Split Studio tile, or Exposé tile (`join-pane [-b] -h|-v`) while a floating ghost card tracks the cursor.
  - Drag the **Split Sash** divider in Home (`25..=75` preview/matrix ratio), `/split`, or `/layout` to resize panes live with a floating percentage badge.
- **VS Code Scrollbar & drag-to-select copy:**
  - Click the track or drag the proportional scrollbar thumb on preview cards (marked with amber search ticks and red `ERROR`/`FATAL`/`panic`/`FAIL` ticks), or use the mouse wheel / `PageUp` / `PageDown` / `Ctrl+U` / `Ctrl+D` / `Home` / `End` to scroll scrollback history.
  - Click-drag inside preview text content to select a text range and copy plain text to the clipboard on release.
- **Circular mouse-wheel list navigation:** Scroll wheel outside preview cards cycles with wrap-around across the `/` palette drawer, `@` target menu, `Live Radar` sessions, `Session×Window` matrix rows, `/pane`, `/split`, `/layout`, `/clients`, `/buffers`, `/snapshot`, `/theme`, and `/config` lists.
- **Terminal restoration & inline safety:** JustMux always disables mouse capture (`DisableMouseCapture`) before leaving the alternate screen or calling `exec` to hand control to native `tmux`, never enables mouse capture in `--inline` mode, and strictly disables `live_stream` background polling in `--inline` mode so native scrollback stays clean with 0 idle bytes.

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
live_stream = false
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

### Beginner-Friendly `~/.tmux.conf` Comfort Defaults

```toml
[tmux]
tmux_popup = true
tmux_mouse = true
tmux_history_limit = 50000
tmux_base_index_one = true
tmux_renumber_windows = true
tmux_zero_escape_time = true
```
