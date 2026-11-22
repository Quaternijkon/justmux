# JustMux

**Stop memorizing tmux commands. Just type `/`.**

JustMux (`jm`) is a small, native terminal front end for the tmux server you
already use. It provides a focused Ratatui interface, searchable slash
commands, and hands control back to tmux as soon as you enter a session.

## Features

- A Codex-style framed composer with command completion, `Shift+Tab` mode
  switching (`Command & @Jump`, `Live Radar`, `Quick Launch`), `@target` jump
  menus, `1–9` direct session keys, `Ctrl+P`/`Ctrl+K` universal palette,
  natural phrases, and zero-learning shortcuts (`↓`, `?`, `Ctrl+T`).
- A spacious Fullscreen studio by default with live pane previews
  (`tmux capture-pane`), 8 quick commands, 6 numbered workspaces, optional
  mouse scroll/click support (`DisableMouseCapture` before tmux handoff), and
  optional `--inline` mode that preserves native shell scrollback.
- Unified zero-dependency TOML configuration (`~/.config/justmux/config.toml`,
  `--config`, `--print-config`, interactive `/config`, and `/theme` `s` save)
  plus five switchable TrueColor theme presets (`/theme` or `Ctrl+T`, with
  `JUSTMUX_THEME`, `NO_COLOR`, and ASCII fallbacks).
- Session create, list, resume, rename, and Codex-style confirmed kill cards.
- Window and pane management with a visual `/split` wireframe preview through
  the native tmux CLI.
- Correct attach-from-a-shell and switch-client-from-tmux behavior.
- No daemon, database, plugin, telemetry, or tmux configuration changes.

## Requirements

- Rust 1.88 or newer for source builds.
- Node.js 22.15+ with `process.execve` support for npm installations (Node.js 24
  LTS is recommended). npm installs precompiled binaries; Rust is not needed.
- tmux 3.2 or newer for the optional popup integration (ordinary `jm` use
  works with older tmux versions where the server supports the requested
  operation).
- macOS ARM64 (Apple Silicon) or Linux x64 with a UTF-8 capable terminal.

## Install

Install or update from [npm](https://www.npmjs.com/package/justmux):

```sh
npm install -g justmux@latest
jm        # or: justmux
```

Update anytime with `jm update` (or `justmux update`), or check first with
`jm update --check`; you can also re-run `npm install -g justmux@latest`. Both
`jm` and `justmux` start the same program. npm selects the binary for
macOS ARM64 or Linux x64. The Linux package is a static musl build and works on
both glibc and musl systems. Keep optional dependencies enabled.
tmux must be installed separately; for example, `brew install tmux` on macOS
or `sudo apt install tmux` on Debian/Ubuntu. The npm launcher replaces its
process with the Rust executable, so it does not stay in the background.

Global npm installs check for updates when `jm` or `justmux` starts interactively
without a command (also with `--fullscreen` or `--inline`). At most once a day, a check waits
up to three seconds for npm. A newer version opens an inline picker:

```text
JustMux update available
0.1.1 -> 0.1.2

> 1. Update
  2. Skip
  3. Skip this version
```

Use Up/Down and Enter, or press 1, 2, or 3. `Update` is initially selected;
it installs that exact version and exits, so run `jm` again afterward. `Skip`
(also Esc) continues this launch; `Skip this version` remembers the dismissal
until a different version is available. The picker uses a compact region in
the current terminal and restores input and the cursor before continuing.
Ctrl+C cancels startup. Offline checks and failed updates let you keep using
JustMux. Set `JUSTMUX_NO_UPDATE_CHECK=1` to disable checks. Help, version,
direct commands, CI, and noninteractive runs skip them.

Release preparation and publication run locally; GitHub builds are optional.
See [npm release instructions](docs/npm-release.md). The npm source template is
private to prevent accidental publication; only generated packages are released.

Install from this checkout:

```sh
cargo install --path . --locked
```

The package installs two executable names into Cargo's binary directory:
`jm` and `justmux`. They are the same JustMux CLI, so either command starts
the same TUI and accepts the same options. With the standard Rust setup this
directory is `$HOME/.cargo/bin`; add it to your shell `PATH` once if it is not
already present:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
jm --version
justmux --version
```

To build a local release artifact without installing it:

```sh
cargo build --release
./target/release/jm
```

The equivalent long-form invocation is `./target/release/justmux`.

## Quick start

Run `jm` from a normal shell and open `/resume`. Sessions run across the grid,
with their windows listed beneath them. Use Left/Right to choose a session
and Up/Down to select a window, or type a session or window name to filter.
Search ranks exact names before prefixes, substrings, and fuzzy matches. Words
can match in any order, and matching parts of names are underlined. A session
name match keeps that session's windows visible. Enter opens the selected
window. Each session defaults to its active window until a search finds a
better match. Narrower terminals show fewer columns at a time. The TUI is torn
down before tmux takes over the terminal and keyboard.

By default, JustMux launches in a spacious Fullscreen alternate-screen studio
with live workspace previews and mouse wheel/click support (cleanly restored
before handing control to tmux or exiting). The UI waits for input, ignores
mouse hover/move events, updates only changed cells, and combines queued input
events before drawing. This keeps SSH output low, although menu navigation
still depends on network latency.

Use `jm --inline` (or `justmux --inline`, or set `screen_mode = "inline"` in
`config.toml`) when you prefer the compact inline region that keeps existing
shell output above it and leaves native terminal scrollback untouched without
enabling mouse capture. Inside tmux, scrollback remains subject to tmux's own
scroll/copy-mode settings.

See [Configuring JustMux](docs/configuration.md) for `--config`,
`--print-config`, `--theme`, environment precedence, and interactive `/config`.

When `jm` is launched in a tmux pane (including a `display-popup`), choosing a
session uses `switch-client`; closing the popup leaves the program in the pane
below it untouched.

Run `/exit` inside tmux to detach and return to the terminal that opened the
tmux client. You can reconnect to the session with `/resume`. If several
clients share the same window, open JustMux with the popup binding below so
it can identify your client. Use `/quit` to close JustMux while staying in tmux.

Recommended popup binding (tmux 3.2+):

```tmux
bind-key j run-shell 'tmux display-popup -E -w 80% -h 70% -c #{q:client_name} "JUSTMUX_CLIENT=#{q:client_name} JUSTMUX_PANE=#{q:pane_id} JUSTMUX_POPUP=1 jm"'
```

`run-shell` expands the launching client's identity before opening the popup.
This keeps switching and detaching scoped to that client even when several
clients share a session.

## Slash commands

| Command | Action |
| --- | --- |
| `/resume` | Search sessions and windows, then open a window from the grid |
| `/new` | Create a detached session, optionally with a working directory |
| `/list` | Browse current sessions |
| `/rename` | Rename a session |
| `/kill` | Confirm and remove a session |
| `/window` | Inspect and manage windows |
| `/pane` | Inspect and manage panes |
| `/split` | Split the current pane horizontally or vertically with a live wireframe preview |
| `/theme` | Switch visual color theme (`aurora`, `codex`, `claude`, `matrix`, `classic`) |
| `/help` | Show command and keyboard help |
| `/exit` | Detach the current tmux client, or quit JustMux from outside tmux |
| `/quit` | Exit JustMux and return to the shell |

In `/window`, choose **New window** to select a session and enter an optional
name. To manage an existing window, filter by session or window name, select
it, and press Enter for **Open window**, **Rename window**, or **Close window**.
Closing ends the programs running in that window and requires typing `yes`
and pressing Enter. In `/list` and `/pane`, Enter on a row opens its actions.
Choose **Refresh list** in an unfiltered list to reload it.

The command popup filters as you type. For example, `/att` matches the
`/attach` alias for `/resume`, and `/reusme` prompts you to check the
spelling. On the home studio screen:

- Press `Shift+Tab` to cycle between **Command & @Jump** (default), **Live
  Radar** (browse sessions alongside live captured terminal previews), and
  **Quick Launch** (press Enter on an empty bar to create/enter a workspace
  named after your current directory, or type any workspace name).
- Press `1`–`9` on an empty Command & @Jump bar to jump directly to the
  numbered live workspace.
- Type `@` (standalone or after `/kill`, `/rename`, `/split`, `/window`,
  `/pane`, `/resume`) to autocomplete `@session`, `@session:window`, and
  `@session:window.pane` targets.
- Press `Ctrl+P` anywhere (or `Ctrl+K` outside text inputs / on an empty home
  bar) to jump straight to the `/` command palette.
- Press `↓` on an empty prompt to open the command menu without typing `/`, `?`
  to jump straight to `/help`, or `Ctrl+T` to cycle through the five built-in
  palettes (also configurable via `JUSTMUX_THEME`).
- Bare command names (`resume`, `theme codex`, `config`), natural phrases (`new demo`,
  `create demo`, `split right`, `split down`, `kill demo`), and exact live
  session names work without a leading `/`.
- Open `/config` (or `/settings`) to toggle any setting live and press `s` to
  save directly to `~/.config/justmux/config.toml` (or your `--config` file);
  inside `/theme`, press `s` to save the highlighted preset. See
  [`docs/configuration.md`](docs/configuration.md).

Use Up/Down to move, Tab to complete, Enter to run, Esc to return, and Ctrl+C
to quit safely. Pasting edits the focused input. Line breaks and tabs become
spaces, and Enter submits the result. (There are also a few hidden tribute
Easter eggs tucked behind `/codex`, `/claude`, `/antigravity`, `/vibe`, and
friends—plus a wink if you reach for `Ctrl+b` out of habit.)

JustMux passes session and resource names to tmux as individual arguments.

## Relationship with tmux

JustMux is an on-demand client of an existing tmux server. It does not emulate
a terminal, create a private server, maintain session state, or modify
`~/.tmux.conf`. Outside tmux it replaces itself with `tmux attach-session`; in
tmux it uses `switch-client` to enter a session and `detach-client` for `/exit`.
Destructive actions always require an explicit confirmation.

Use `jm -L NAME` or `jm -S PATH` when you intentionally manage a custom tmux
socket. `jm doctor` reports the resolved config path, screen mode, detected
theme, and local tmux capabilities without changing configuration.
`jm --print-config` outputs starter TOML defaults, and `jm --print-keybind`
prints the popup snippet above.

## Development

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

To test the npm packaging and real installation/update workflow without
publishing anything:

```sh
npm --prefix npm/justmux test
node --test tests/npm-packages.test.mjs
node --test tests/release-workflow.test.mjs
node scripts/test-npm-install.mjs --pty
```

The last command uses packed files and a temporary loopback registry and global
prefix. It also checks native tmux handoff and two-client popup switching through
the installed npm launcher. On Linux, pass `--binary` pointing to the static
musl release build.

Integration tests use private tmux sockets and clean them up automatically.
Never run destructive tmux commands against your default server while
developing. Manual interaction is best checked in Ghostty, iTerm2, or a
standard Linux terminal; see [`docs/manual-test.md`](docs/manual-test.md).

After a release build, the bounded PTY smoke check exercises external
attach/detach, session creation, and a real popup switch on an isolated tmux
socket:

```sh
python3 scripts/pty_smoke.py
```

To release from this Mac with one command, after committing your changes:

```sh
./scripts/release 0.1.2
```

This updates the version, runs checks, builds both supported platforms, pushes the
release source and tag to GitHub, and publishes the npm packages. Use `--dry-run`
to preview, `--commit` to include all nonignored working-tree changes, or
`--resume` to continue a failed release at the same version. npm may ask for
browser-based 2FA. See [the local release workflow](docs/local-release.md).

## FAQ and limitations

**Does JustMux require a configuration file?** No. The popup binding is an
optional snippet that you copy into your own tmux configuration.

**What happens if a session disappears while it is selected?** The operation
reports a friendly error and refreshes the list.

**Can it attach from inside a different tmux server?** It can inspect and
manage the selected socket, but refuses to nest a client inside another server.

**Is there a recording or screenshot?** [`assets/demo.cast`](assets/demo.cast)
is a real asciinema v2 recording generated from the bounded PTY smoke check;
it is not a static UI mock.

## Contributing

Keep tmux process spawning in `src/tmux`, keep widgets independent from tmux,
and add regression tests for parser, command, and state-machine changes. Pull
requests should include the four checks in the Development section.
