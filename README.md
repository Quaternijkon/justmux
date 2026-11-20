# JustMux

**Stop memorizing tmux commands. Just type `/`.**

JustMux (`jm`) is a small, native terminal front end for the tmux server you
already use. It provides a focused Ratatui interface, searchable slash
commands, and hands control back to tmux as soon as you enter a session.

## Features

- A Codex-style composer with command completion and a searchable command
  popup.
- A compact interface in the terminal's main screen, preserving shell output
  and native scrollback. Fullscreen is optional.
- Session create, list, resume, rename, and confirmed kill operations.
- Window and pane management through the native tmux CLI.
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
```

Both `jm` and `justmux` start the same program. npm selects the binary for
macOS ARM64 or Linux x64. The Linux package is a static musl build and works on
both glibc and musl systems. Keep optional dependencies enabled.
tmux must be installed separately; for example, `brew install tmux` on macOS
or `sudo apt install tmux` on Debian/Ubuntu. The npm launcher replaces its
process with the Rust executable, so it does not stay in the background.

Global npm installs check for updates when `jm` or `justmux` starts interactively
without a command (also with `--fullscreen`). At most once a day, a check waits
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
Enter opens the highlighted window. Each session defaults to its active window,
and narrower terminals show fewer columns at a time. The TUI is torn down
before tmux takes over the terminal and keyboard.

By default, JustMux uses a compact region of the main screen instead of
entering an alternate screen. Existing shell output stays above it, and the
terminal keeps its normal scrollback. It does not capture mouse scrolling.
The UI waits for input, updates only changed cells, and combines queued input
events before drawing. This reduces output over SSH, although menu navigation
still depends on network latency. Inside tmux, scrollback remains subject to
tmux's own scroll/copy-mode settings.

Use `jm --fullscreen` (or `justmux --fullscreen`) for the alternate-screen
layout when you want to use the entire terminal.

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
| `/split` | Split the current pane horizontally or vertically |
| `/help` | Show command and keyboard help |
| `/exit` | Detach the current tmux client, or quit JustMux from outside tmux |
| `/quit` | Exit JustMux and return to the shell |

In `/window`, choose **New window** to select a session and enter an optional
name. To manage an existing window, filter by session or window name, select
it, and press Enter for **Open window**, **Rename window**, or **Close window**.
Closing ends the programs running in that window and requires typing `yes`
and pressing Enter. In `/list` and `/pane`, Enter on a row opens its actions.
Choose **Refresh list** in an unfiltered list to reload it.

The command popup filters as you type. Use Up/Down to move, Tab to complete,
Enter to run, Esc to return, and Ctrl+C to quit safely. Session and resource
names are passed as separate process arguments; shell interpolation is never
used.

## Relationship with tmux

JustMux is an on-demand client of an existing tmux server. It does not emulate
a terminal, create a private server, maintain session state, or modify
`~/.tmux.conf`. Outside tmux it replaces itself with `tmux attach-session`; in
tmux it uses `switch-client` to enter a session and `detach-client` for `/exit`.
Destructive actions always require an explicit confirmation.

Use `jm -L NAME` or `jm -S PATH` when you intentionally manage a custom tmux
socket. `jm doctor` reports the local tmux and terminal capabilities without
changing configuration. `jm --print-keybind` prints the popup snippet above.

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
