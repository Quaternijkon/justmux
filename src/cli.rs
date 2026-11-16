//! Argument parsing, diagnostics, and the blocking TUI event loop.
use crate::{
    app::{App, AppOptions, Exit},
    terminal::{ScreenMode, TerminalGuard, exec_tmux, install_restore_hooks},
    tmux::{SocketSpec, Tmux, TmuxClient},
};
use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
    time::Duration,
};

const KEYBIND: &str = r#"bind-key j run-shell 'tmux display-popup -E -w 80% -h 70% -c #{q:client_name} "JUSTMUX_CLIENT=#{q:client_name} JUSTMUX_PANE=#{q:pane_id} JUSTMUX_POPUP=1 jm"'"#;

#[derive(Debug, Parser)]
#[command(
    name = "jm",
    version,
    about = "A modern slash-command interface for tmux",
    long_about = "Just tmux. Just simpler.\nStart the TUI, or use `jm resume`, `jm doctor`, or `jm --print-keybind`."
)]
struct Cli {
    /// Use a named tmux socket (same as tmux -L).
    #[arg(short = 'L', long = "socket-name", conflicts_with = "socket_path")]
    socket_name: Option<String>,
    /// Use a tmux socket path (same as tmux -S).
    #[arg(short = 'S', long = "socket-path", conflicts_with = "socket_name")]
    socket_path: Option<PathBuf>,
    /// Use an alternate fullscreen display instead of the native inline interface.
    #[arg(long)]
    fullscreen: bool,
    /// Print a recommended Popup binding without editing any file.
    #[arg(long)]
    print_keybind: bool,
    /// Report tmux and terminal capabilities.
    #[arg(long)]
    doctor: bool,
    /// Open a slash command directly, followed by optional command arguments.
    /// Shell quoting is preserved by rebuilding a safely quoted slash input.
    #[arg(value_name = "COMMAND", trailing_var_arg = true)]
    command: Vec<String>,
}

impl Cli {
    fn socket(&self) -> SocketSpec {
        match (&self.socket_name, &self.socket_path) {
            (Some(name), _) => SocketSpec::Name(name.clone()),
            (_, Some(path)) => SocketSpec::Path(path.clone()),
            _ => SocketSpec::Default,
        }
    }
}

/// Entry point shared by both executable names.
pub fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jm: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let tmux = TmuxClient::new(cli.socket());
    if cli.print_keybind {
        println!("# tmux 3.2+; add this to your tmux configuration:\n{KEYBIND}");
        return Ok(());
    }
    if cli.doctor || cli.command.first().map(String::as_str) == Some("doctor") {
        return doctor(&tmux);
    }
    tmux.version().context("cannot start JustMux")?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("JustMux needs an interactive terminal; try jm doctor or jm --print-keybind");
    }
    let initial_command = cli.command.first().map(|name| {
        let mut text = String::from("/");
        text.push_str(name.trim_start_matches('/'));
        for arg in cli.command.iter().skip(1) {
            text.push(' ');
            text.push_str(&quote_argument(arg));
        }
        text
    });
    let mode = if cli.fullscreen {
        ScreenMode::Fullscreen
    } else {
        ScreenMode::Inline
    };
    run_tui(&tmux, AppOptions { initial_command }, mode)
}

fn quote_argument(argument: &str) -> String {
    if argument
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./:-".contains(c))
    {
        argument.to_owned()
    } else {
        format!(
            "'{}'",
            argument.replace(char::from(39), concat!("'", "\\", "''"))
        )
    }
}

fn doctor(tmux: &TmuxClient) -> Result<()> {
    let version = tmux.version().context("cannot query tmux")?;
    println!("JustMux {}", env!("CARGO_PKG_VERSION"));
    println!("tmux: {version}");
    println!(
        "Popup: {}",
        if version.supports_popup() {
            "supported"
        } else {
            "requires tmux 3.2+ (ordinary attach still works)"
        }
    );
    println!("Socket: {:?}", tmux.socket());
    println!("Context: {:?}", tmux.context());
    println!(
        "Server: {}",
        if tmux.server_running()? {
            "running"
        } else {
            "not running; /new can create a session"
        }
    );
    println!(
        "Interactive input/output: {}/{}",
        io::stdin().is_terminal(),
        io::stdout().is_terminal()
    );
    Ok(())
}

fn run_tui(tmux: &TmuxClient, options: AppOptions, mode: ScreenMode) -> Result<()> {
    install_restore_hooks()?;
    let mut guard = TerminalGuard::enter(mode)?;
    let viewport = match mode {
        ScreenMode::Inline => Viewport::Fixed(guard.inline_area(None)?),
        ScreenMode::Fullscreen => Viewport::Fullscreen,
    };
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions { viewport },
    )
    .context("could not initialise terminal")?;
    let mut app = App::new(tmux, options);
    loop {
        // Exit before another draw: attach/switch must hand over immediately.
        if let Some(exit) = app.exit().cloned() {
            drop(terminal);
            return match exit {
                Exit::Quit | Exit::Switched => guard.restore(),
                Exit::Attach(target) => exec_tmux(guard, tmux.attach_command(&target)),
            };
        }
        terminal
            .draw(|frame| app.render(frame))
            .context("could not draw JustMux")?;

        // Render once per input batch, without polling while idle. Bursts of
        // arrow-key repeats or pasted text therefore don't emit a frame each.
        let mut changed = false;
        let mut resized = false;
        while !changed {
            let first = event::read().context("could not read terminal input")?;
            let mut current = first;
            for index in 0..64 {
                resized |= matches!(current, Event::Resize(_, _));
                if redraw_event(&current) {
                    app.handle_event(current);
                    changed = true;
                }
                if index == 63 || app.exit().is_some() || !event::poll(Duration::ZERO)? {
                    break;
                }
                current = event::read().context("could not read queued terminal input")?;
            }
        }
        // A resize burst needs one cursor report and one region reservation.
        // Waiting for a remote terminal once per resize event adds avoidable RTTs.
        if resized && mode == ScreenMode::Inline && app.exit().is_none() {
            let previous = terminal.get_frame().area();
            let area = guard.inline_area(Some(previous))?;
            terminal = Terminal::with_options(
                CrosstermBackend::new(io::stdout()),
                TerminalOptions {
                    viewport: Viewport::Fixed(area),
                },
            )?;
        }
    }
}

fn redraw_event(event: &Event) -> bool {
    match event {
        Event::Key(key) => key.kind != KeyEventKind::Release,
        Event::Paste(_) | Event::Resize(_, _) => true,
        Event::Mouse(_) | Event::FocusGained | Event::FocusLost => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_is_default_and_fullscreen_is_explicit() {
        assert!(!Cli::try_parse_from(["jm"]).expect("default CLI").fullscreen);
        let cli = Cli::try_parse_from(["jm", "--fullscreen", "resume"]).expect("fullscreen CLI");
        assert!(cli.fullscreen);
        assert_eq!(cli.command, vec!["resume"]);
    }

    #[test]
    fn focus_and_key_release_events_do_not_trigger_redraws() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        assert!(!redraw_event(&Event::FocusGained));
        assert!(!redraw_event(&Event::FocusLost));
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert!(redraw_event(&Event::Key(key)));
        assert!(!redraw_event(&Event::Key(KeyEvent {
            kind: KeyEventKind::Release,
            ..key
        })));
        assert!(redraw_event(&Event::Resize(80, 24)));
    }

    #[test]
    fn socket_arguments_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["jm", "-L", "private", "-S", "/tmp/private"]).is_err());
    }
    #[test]
    fn parses_commands_and_custom_sockets() {
        let cli = Cli::try_parse_from(["jm", "-L", "private", "resume"]).expect("valid CLI");
        assert_eq!(cli.command, vec!["resume"]);
        assert_eq!(cli.socket(), SocketSpec::Name("private".into()));
        let cli = Cli::try_parse_from(["justmux", "doctor"]).expect("doctor CLI");
        assert_eq!(cli.command, vec!["doctor"]);
    }
    #[test]
    fn command_arguments_are_quoted_for_app_parser() {
        let cli = Cli::try_parse_from(["jm", "new", "my project", "/tmp/work"]).expect("valid CLI");
        let text = std::iter::once(cli.command[0].clone())
            .chain(cli.command.iter().skip(1).map(|arg| quote_argument(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text, "new 'my project' /tmp/work");
    }
    #[test]
    fn keybind_is_opt_in_and_uses_native_popup() {
        assert!(KEYBIND.contains("display-popup -E"));
        assert!(KEYBIND.contains("run-shell"));
        assert!(KEYBIND.contains("-c #{q:client_name}"));
        assert!(KEYBIND.contains("JUSTMUX_CLIENT=#{q:client_name}"));
        assert!(KEYBIND.contains("JUSTMUX_PANE=#{q:pane_id}"));
        assert!(
            Cli::try_parse_from(["jm", "--print-keybind"])
                .expect("keybind CLI")
                .print_keybind
        );
    }
}
