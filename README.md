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
- macOS or Linux with a UTF-8 capable terminal.

## Install

Install or update from [npm](https://www.npmjs.com/package/justmux):

```sh
npm install -g justmux@latest
```

Both `jm` and `justmux` start the same program. npm selects the binary for
macOS or Linux on Intel/AMD x64 or ARM64. Linux packages are static musl builds
and work on both glibc and musl systems. Keep optional dependencies enabled.
tmux must be installed separately; for example, `brew install tmux` on macOS
or `sudo apt install tmux` on Debian/Ubuntu. The npm launcher replaces its
process with the Rust executable, so it does not stay in the background.

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

Run `jm` from a normal shell. Type `/` to open the command menu, then choose
`/resume` and press Enter to attach to a session. The TUI is torn down before
tmux receives the terminal, so tmux owns all keyboard input after that point.

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

Recommended popup binding (tmux 3.2+):

```tmux
bind-key j run-shell 'tmux display-popup -E -w 80% -h 70% -c #{q:client_name} "JUSTMUX_CLIENT=#{q:client_name} JUSTMUX_PANE=#{q:pane_id} JUSTMUX_POPUP=1 jm"'
```

`run-shell` expands the launching client's identity before opening the popup.
This keeps switching scoped to that client even when several clients share a session.

## Slash commands

| Command | Action |
| --- | --- |
| `/resume` | Search sessions and attach or switch to one |
| `/new` | Create a detached session, optionally with a working directory |
| `/list` | Browse current sessions |
| `/rename` | Rename a session |
| `/kill` | Confirm and remove a session |
| `/window` | Inspect and manage windows |
| `/pane` | Inspect and manage panes |
| `/split` | Split the current pane horizontally or vertically |
| `/help` | Show command and keyboard help |
| `/quit` | Exit JustMux and return to the shell |

The command popup filters as you type. Use Up/Down to move, Tab to complete,
Enter to run, Esc to return, and Ctrl+C to quit safely. Session and resource
names are passed as separate process arguments; shell interpolation is never
used.

## Relationship with tmux

JustMux is an on-demand client of an existing tmux server. It does not emulate
a terminal, create a private server, maintain session state, or modify
`~/.tmux.conf`. Outside tmux it replaces itself with `tmux attach-session`; in
tmux it runs `switch-client` and exits. Existing clients remain attached.
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
./scripts/release 0.1.1
```

This updates the version, runs checks, builds all four platforms, pushes the
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
