# Manual terminal checks

These checks require a real interactive terminal and a private tmux socket.
They complement the automated test suite; they are not run by CI.

The repeatable PTY version is `python3 scripts/pty_smoke.py`; it uses a fresh
socket, a 100x30 xterm PTY, deadline based reads, and cleanup on every path.

1. Build with `cargo build --release`, print a few lines of shell output, and
   run `./target/release/jm` from a normal shell. Confirm it uses a compact
   region below that output and native terminal scrollback remains available.
   Repeat with `--fullscreen` to check the optional alternate-screen layout.
2. Type `/`, then `/res`; verify filtering, Tab completion, Up/Down navigation,
   Esc, and the empty-session message.
3. Run `/new`, create a temporary session, choose its resume action, and detach
   with the native tmux prefix. Confirm the original shell returns.
4. Start a private tmux server, run a foreground editor or shell command in a
   pane, and open `jm` using the README's `run-shell` / `display-popup` binding.
   It expands `JUSTMUX_CLIENT=#{q:client_name}` and `JUSTMUX_PANE=#{q:pane_id}`
   in the launching client's context. Resume a second session and
   confirm `switch-client` occurs while the original process remains alive.
5. Open the popup again and press Esc or `/quit`; confirm the underlying pane
   is unchanged.
6. Resize a narrow popup and test `/list`, `/rename`, `/kill`, `/window`,
   `/pane`, and `/split`. Confirm kill dialogs default to Cancel.
7. Interrupt with Ctrl+C and SIGTERM and verify raw mode and the cursor are
   restored. In the default inline mode, the shell prompt should follow the UI;
   with `--fullscreen`, the original screen should return.

Do not run these destructive steps against a user's default server. Replace
the tmux socket with a unique `-L` name or a temporary `-S` path and clean it
up after the check.

## Automated PTY assertions

Run `cargo build --release`, then
`python3 scripts/pty_smoke.py --cast /tmp/justmux-inline-regression.cast`.
The script uses a fresh named socket and real 100x30 PTYs. It verifies the
original `jm` PID becomes the native tmux client, sends a pane command whose
result is checked via tmux, detaches, and compares complete termios attributes.
It also checks `/new` creates the named native session with the requested cwd,
that the session survives `/quit`, and that Ctrl+C and SIGTERM restore termios
and the cursor. Inline checks answer actual cursor-position reports, start below
existing shell output, resize the PTY, and check that the shell returns below
the interface. The main screen must not receive alternate-screen, whole-screen
clear, scrollback-clear, or mouse-capture sequences. `--fullscreen` separately
checks alternate-screen entry, exit, and restoration after SIGTERM.
Popup checks use two attached clients and
a foreground `sleep`; cancellation and switching must preserve its exact PID
and leave the second client on its original session. All child waits and command
calls have deadlines, and cleanup runs on failure.

These automated PTY checks do not constitute Ghostty, iTerm2, Terminal.app,
Vim or Codex visual/manual acceptance. The test's small ANSI screen model
validates cursor reports and output placement; visual appearance and actual
SSH/network latency still need an interactive terminal check.

Verified on macOS ARM64 with tmux 3.8: the repeatable PTY assertions above all
passed, including two clients attached to the same source session. The popup
was opened by actual Prefix+j input; cancellation and `/resume` preserved the
source sleep PID and did not switch the other client. A real capture was written
to `/tmp/justmux-inline-regression.cast`, leaving the checked-in demo unchanged.

## npm distribution checks

`node scripts/test-npm-install.mjs --pty` packs the real Rust executable and
installs it into a temporary global prefix using a loopback fixture registry.
It checks both command names, an incomplete install's error message, and a
version update using explicit fixture executables. It then runs the external
attach and two-client popup assertions above through the installed npm launcher.
No packages are published and the user's global npm installation is unchanged.

Verified on 2026-11-17 on macOS ARM64 with Node.js 24.22.2, npm 11.19.1, and
tmux 3.8: these npm checks and all nine direct Rust PTY checks passed. The new
PTY capture is `/tmp/justmux-npm-release-validation.cast`. All four native npm
artifacts were built locally with Rust 1.100.0, macOS SDK 27.2, Zig 0.17.1, and
cargo-zigbuild 0.24.1. Intel macOS and both Linux executables passed architecture
inspection; their runtime behavior has not been verified on those platforms.

The published `0.1.0` packages were also checked against the public npm registry:
all five integrity values matched the local tarballs, and an anonymous fresh
`npm install --global --prefix <temporary-directory> justmux@latest` succeeded.
Both command names resolved through that prefix's `PATH`. External attach/detach
and two-client popup cancellation/switching passed through this public npm install;
the temporary installation was removed afterward. This remains automated PTY
coverage rather than named-terminal visual acceptance.
