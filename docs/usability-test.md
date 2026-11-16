# Build and usability verification

Verified on 2026-11-16 on macOS ARM64 (Darwin), Rust/Cargo 1.100.0 and tmux 3.8.
These results describe automated tests using real tmux processes and PTYs;
they do not represent visual acceptance in a desktop terminal application.

## Build and automated checks

All commands below passed after the fixes from this verification:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
python3 scripts/pty_smoke.py --cast /tmp/justmux-usability-20261116.cast
git diff --check
```

The Rust suite passed 50 tests: 26 library tests, 15 isolated tmux integration
tests, and 9 UI rendering/event tests. Integration coverage includes Session
creation/rename/confirmed deletion and Window/Pane management driven through
the real application's keyboard event handler, as well as native CLI operations.

The five PTY checks passed:

1. `/resume`: the original `jm` PID becomes the native tmux client. A command
   sent through that terminal changes a value checked independently on the
   private server. Native detach exits successfully and restores termios.
2. `/new`: the native session and requested working directory exist, and the
   session survives `/quit`.
3. Ctrl+C restores the complete termios state, cursor, and alternate screen.
4. SIGTERM restores the same terminal state and exits with the expected status.
5. With two attached clients, the actual printed key binding opens a Popup via
   Prefix+j. Cancel preserves both clients; resume switches only its originating
   client. The source foreground `sleep` retains its exact PID and remains alive.

The capture path above contains actual timed PTY output. The checked-in demo
recording was left unchanged. All tmux test servers use private sockets and
are cleaned up; the user's tmux configuration was not edited.

CLI help, both binary version flags, key binding output, `doctor` against an
absent private server, and the friendly error for non-interactive input/output
also passed. Read-only diagnostics did not create a tmux server.

## Problems found and fixed

- Splitting a pane in a path containing spaces could start the new shell in
  the home directory. The working-directory argument now uses the unescaped
  tmux format; each argument is already passed directly to tmux.
- The former Popup binding could resolve a different attached client. The
  binding now expands the originating client and pane in `run-shell` before
  opening the Popup, and explicitly targets that client.
- Enter on a command alias could execute a different command from the
  highlighted candidate. Enter now executes the visible selection.
- Long input in a narrow terminal could hide the caret. Composer, search,
  and form fields now scroll within their actual rendered widths.
- A short command menu could hide its selected item. It now scrolls to the
  selection.
- Destructive-operation warnings could be truncated. They now wrap and
  support Up/Down and PageUp/PageDown review while the input stays visible.

Regression tests cover these cases, including cancellation without a tmux kill.

## Try the build

```sh
./target/release/jm
```

The long-form alias is `./target/release/justmux`. To install both commands:

```sh
cargo install --path . --locked
```

Linux and Intel macOS were not exercised in this local run. Ghostty, iTerm2,
Terminal.app, light/dark appearance, and foreground Codex/Vim interaction still
require the steps in [manual-test.md](manual-test.md). No release was published.
