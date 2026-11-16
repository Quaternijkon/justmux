# Manual terminal checks

These checks require a real interactive terminal and a private tmux socket.
They complement the automated test suite; they are not run by CI.

The repeatable PTY version is `python3 scripts/pty_smoke.py`; it uses a fresh
socket, a 100x30 xterm PTY, deadline based reads, and cleanup on every path.

1. Build with `cargo build --release` and run `./target/release/jm` from a
   normal shell.
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
7. Interrupt with Ctrl+C and verify the cursor, alternate screen, and shell
   prompt are restored.

Do not run these destructive steps against a user's default server. Replace
the tmux socket with a unique `-L` name or a temporary `-S` path and clean it
up after the check.

## Automated PTY assertions

Run `cargo build --release`, then
`python3 scripts/pty_smoke.py --cast /tmp/justmux-pty-regression.cast`.
The script uses a fresh named socket and real 100x30 PTYs. It verifies the
original `jm` PID becomes the native tmux client, sends a pane command whose
result is checked via tmux, detaches, and compares complete termios attributes.
It also checks `/new` creates the named native session with the requested cwd,
that the session survives `/quit`, and that Ctrl+C and SIGTERM restore termios,
the cursor and the alternate screen. Popup checks use two attached clients and
a foreground `sleep`; cancellation and switching must preserve its exact PID
and leave the second client on its original session. All child waits and command
calls have deadlines, and cleanup runs on failure.

These automated PTY checks do not constitute Ghostty, iTerm2, Terminal.app,
Vim or Codex visual/manual acceptance. Cursor and alternate-screen restoration
is asserted from their actual output sequences; visual appearance still needs
an interactive terminal check.

Verified on macOS ARM64 with tmux 3.8: the repeatable PTY assertions above all
passed, including two clients attached to the same source session. The popup
was opened by actual Prefix+j input; cancellation and `/resume` preserved the
source sleep PID and did not switch the other client. A real capture was written
to `/tmp/justmux-pty-regression.cast`, leaving the checked-in demo unchanged.
