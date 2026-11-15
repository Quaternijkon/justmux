# JustMux

**Stop memorizing tmux commands. Just type `/`.**

JustMux (`jm`) is a small, native terminal front end for the tmux server you
already use. It provides a focused Ratatui interface, searchable slash
commands, and hands control back to tmux as soon as you enter a session.

## Features

- A Codex-style composer with command completion and a searchable command
  popup.
- Session create, list, resume, rename, and confirmed kill operations.
- Window and pane management through the native tmux CLI.
- Correct attach-from-a-shell and switch-client-from-tmux behavior.
- No daemon, database, plugin, telemetry, or tmux configuration changes.

## Requirements

- Rust 1.88 or newer for source builds.
- tmux 3.2 or newer for the optional popup integration (ordinary `jm` use
  works with older tmux versions where the server supports the requested
  operation).
- macOS or Linux with a UTF-8 capable terminal.

## Install

Install from this checkout:

```sh
cargo install --path .
```

This installs both `jm` and the long-form `justmux` binary. To build a local
release artifact without installing it:

```sh
cargo build --release
./target/release/jm
```

## Quick start

Run `jm` from a normal shell. Type `/` to open the command menu, then choose
`/resume` and press Enter to attach to a session. The TUI is torn down before
tmux receives the terminal, so tmux owns all keyboard input after that point.

When `jm` is launched in a tmux pane (including a `display-popup`), choosing a
session uses `switch-client`; closing the popup leaves the program in the pane
below it untouched.

Recommended popup binding (tmux 3.2+):

```tmux
bind-key j display-popup -E -w 80% -h 70% \
  'JUSTMUX_CLIENT=$(tmux display-message -p "#{client_name}") JUSTMUX_PANE=$(tmux display-message -p "#{pane_id}") JUSTMUX_POPUP=1 jm'
```

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
