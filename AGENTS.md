# JustMux project notes

This checkout builds a local CLI/TUI only. It has no web service, listening
port, launch agent, or production deployment target. The supported entry points
are the `jm` and `justmux` binaries.

Use the existing tmux server selected by tmux itself, or an explicit `-L`/`-S`
socket supplied by the user. Development and integration tests must always use
a private socket and must remove inherited `TMUX` and `TMUX_PANE` variables.
Do not edit `~/.tmux.conf`, create a remote repository, push code, or publish a
release without an explicit request.

Before delivery, run:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

Manual terminal checks are recorded in `docs/manual-test.md`; only behavior
actually exercised there should be reported as manually verified.
