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
   pane, and open `jm` through `display-popup -E` with the recommended
   `JUSTMUX_CLIENT=$(tmux display-message -p "#{client_name}")` and
   `JUSTMUX_PANE=$(tmux display-message -p "#{pane_id}")` environment assignments. Resume a second session and
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
