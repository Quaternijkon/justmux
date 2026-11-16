//! Argument parsing, diagnostics, and the blocking TUI event loop.
use crate::{
    app::{App, AppOptions, Exit},
    terminal::{TerminalGuard, exec_tmux, install_restore_hooks},
    tmux::{SocketSpec, Tmux, TmuxClient},
};
use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
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
    run_tui(&tmux, AppOptions { initial_command })
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

fn run_tui(tmux: &TmuxClient, options: AppOptions) -> Result<()> {
    install_restore_hooks()?;
    let mut guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .context("could not initialise terminal")?;
    let mut app = App::new(tmux, options);
    loop {
        terminal
            .draw(|frame| app.render(frame))
            .context("could not draw JustMux")?;
        if let Some(exit) = app.exit().cloned() {
            return match exit {
                Exit::Quit | Exit::Switched => guard.restore(),
                Exit::Attach(target) => exec_tmux(guard, tmux.attach_command(&target)),
            };
        }
        app.handle_event(event::read().context("could not read terminal input")?);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
