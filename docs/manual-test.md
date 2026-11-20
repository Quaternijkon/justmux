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
   is unchanged. Run `/exit` in a pane or in the popup and confirm it detaches
   only your client. Reattach to verify the original pane is still running.
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
The pane `/exit` check links the same window into two sessions and verifies
that neither client detaches while they share the window. Once only one
client is viewing it, `/exit` detaches that client and restores the pane's
terminal modes. Popup checks use two clients and a foreground `sleep`.
Cancellation, switching, and `/exit` preserve the process PID and leave the
second client on its original session. All child waits and command calls have
deadlines, and cleanup runs on failure.

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

## npm startup update prompt

The updater runs before the Rust interface and only for interactive global npm
launches. Run its automated tests together with the launcher and release tests,
then exercise the actual npm entry point in offline PTYs:

```sh
node --test npm/justmux/test/*.test.cjs tests/*.test.mjs
python3 scripts/test-npm-update-pty.py
```

Verified on 2026-11-18 on macOS ARM64 with Node.js 24.22.2 and tmux 3.8:
all 58 JavaScript tests and all 57 Rust tests passed, along with Rustfmt,
Clippy with warnings denied, and the native release build. All 16 offline
update-picker PTY scenarios passed. The actual npm entry point used a cached
newer version, a native execution fixture, and an npm recording fixture.
Enter selected Update, arrows and j/k navigated, 1/2/3 selected directly, and
Esc/Ctrl+D selected Skip. Ctrl+C and SIGTERM/SIGHUP/SIGQUIT restored the terminal
before exiting with their expected status. A narrow resize kept navigation
usable, a failed update continued to the native executable, and yes/no letters
did not select anything. Every path preserved complete terminal attributes
and restored the cursor without an alternate screen, whole-screen clear, or
mouse capture. npm received the exact version and existing global prefix with
canonical input and echo restored; the native fixture inherited the original
launcher PID. These fixtures never perform a real npm update or network request.

Unit tests also check that Skip only affects one launch, while Skip this version
survives daily metadata refreshes and a subsequent release prompts again. A
bare Esc regression uses Node's actual key decoder, including its `meta` flag.
The real public-registry check took about 2.6 seconds on this Mac. The default
deadline is therefore three seconds; a regression test permits a valid response
taking longer than one second while retaining the bounded-timeout assertions.

The installation check was rerun with the native binary from a temporary build
directory:

```sh
node scripts/test-npm-install.mjs --binary <temporary-build>/release/jm --pty
```

This also runs the 16 updater scenarios using a copy of the package that npm
actually installed. The native attach/detach and two-client popup checks still
use the real Rust executable, with update discovery disabled for those checks.

For named-terminal acceptance after publishing the updater, install it into a
disposable global npm prefix, then launch that version when a newer release
exists. Verify the initial Update highlight, Up/Down and Enter, Esc to skip, and
Ctrl+C returning to the shell. Select Skip and restart to see the prompt again;
select Skip this version and restart to confirm the dismissal. Confirm Update
upgrades that prefix and asks for a fresh launch. Repeat offline and confirm
startup continues. Named-terminal visual acceptance remains manual; the
automated public-registry upgrade check is recorded below.

## Public npm upgrade acceptance

Verified on 2026-11-18 on macOS ARM64: `justmux@0.1.1`,
`justmux-darwin-arm64@0.1.1`, and `justmux-linux-x64@0.1.1` became publicly
downloadable, with each tarball's integrity matching the local release.

A disposable global npm prefix installed the local preview `0.1.0` tarball
containing the new updater, with its native dependency fetched from public npm.
This preview differs from the original public `0.1.0`, which has no updater.
With an empty update cache, the real installed entry discovered public `0.1.1`
and displayed the three-choice picker in a PTY. Pressing Enter invoked real
`npm install`, successfully upgraded that private prefix, and exited with status
0. Both `jm` and `justmux` subsequently reported `jm 0.1.1`; the native optional
dependency was also exactly `0.1.1`. Complete termios attributes and the cursor
were restored. The user's installation remained unchanged, and the disposable
prefix and caches were removed. The observed picker latency was about 1.9
seconds, followed by about 4.6 seconds for the upgrade.

The actual user installation was separately checked in a PTY: it remained the
local preview `0.1.0`, discovered public `0.1.1`, and showed all three choices.
Ctrl+C restored the terminal and exited with status 130. Its real update cache
now records `0.1.1` without a dismissal, leaving the next interactive launch
ready for the user to test Update, Skip, or Skip this version.

Local generated evidence is retained under `dist/update-test/`:
`public-upgrade-report.json`, `public-upgrade.cast`,
`installed-update-menu-report.json`, and `installed-update-menu.cast`.
These are actual PTY captures, rather than named-terminal visual acceptance.
