//! Argument parsing, diagnostics, and the blocking TUI event loop.
use crate::{
    app::{App, AppOptions, Exit},
    config::{CliOverrides, LoadedConfig},
    terminal::{ScreenMode, TerminalGuard, TerminalSettings, exec_tmux, install_restore_hooks},
    tmux::{SocketSpec, Tmux, TmuxClient},
};
use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind, MouseButton, MouseEventKind};
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
    /// Use the alternate fullscreen display (default).
    #[arg(long)]
    fullscreen: bool,
    /// Use the native inline display instead of fullscreen.
    #[arg(long, conflicts_with = "fullscreen")]
    inline: bool,
    /// Override color theme preset (aurora, codex, claude, matrix, classic).
    #[arg(long, value_name = "THEME")]
    theme: Option<String>,
    /// Path to configuration TOML file.
    #[arg(short = 'c', long = "config", value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print starter config.toml and exit.
    #[arg(long)]
    print_config: bool,
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

fn cli_screen_override(cli: &Cli) -> Option<ScreenMode> {
    if cli.fullscreen {
        Some(ScreenMode::Fullscreen)
    } else if cli.inline {
        Some(ScreenMode::Inline)
    } else {
        None
    }
}

fn cli_overrides(cli: &Cli) -> CliOverrides {
    CliOverrides {
        config_path: cli.config.clone(),
        screen_mode: cli_screen_override(cli),
        theme: cli.theme.clone(),
    }
}

fn run(cli: Cli) -> Result<()> {
    if cli.print_config {
        print!("{}", crate::config::starter_config_toml());
        return Ok(());
    }
    if cli.print_keybind {
        println!("# tmux 3.2+; add this to your tmux configuration:\n{KEYBIND}");
        return Ok(());
    }
    let tmux = TmuxClient::new(cli.socket());
    if cli.doctor || cli.command.first().map(String::as_str) == Some("doctor") {
        let loaded = crate::config::load(&cli_overrides(&cli))?;
        return doctor(&tmux, &loaded);
    }
    tmux.version().context("cannot start JustMux")?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!("JustMux needs an interactive terminal; try jm doctor or jm --print-keybind");
    }
    let loaded = crate::config::load(&cli_overrides(&cli))?;
    let initial_command = cli.command.first().map(|name| {
        let mut text = String::from("/");
        text.push_str(name.trim_start_matches('/'));
        for arg in cli.command.iter().skip(1) {
            text.push(' ');
            text.push_str(&quote_argument(arg));
        }
        text
    });
    run_tui(&tmux, AppOptions { initial_command }, loaded)
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

fn doctor(tmux: &TmuxClient, loaded: &LoadedConfig) -> Result<()> {
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
    let config_label = match &loaded.path {
        Some(path) if loaded.loaded_from_file => format!("{} (loaded)", path.display()),
        Some(path) => format!("{} (default; file not created)", path.display()),
        None => "default (no config path)".to_owned(),
    };
    println!("Config: {config_label}");
    println!("Screen mode: {}", loaded.config.screen_mode.as_str());
    println!("Theme: {}", theme_summary(loaded.config.effective_theme()));
    Ok(())
}

fn theme_summary(theme: crate::ui::Theme) -> String {
    let color_mode = if theme.no_color {
        "NO_COLOR"
    } else if theme.truecolor {
        "truecolor"
    } else {
        "ANSI"
    };
    format!("{} ({color_mode})", theme.preset.name())
}

fn run_tui(tmux: &TmuxClient, options: AppOptions, loaded: LoadedConfig) -> Result<()> {
    install_restore_hooks()?;
    let settings = TerminalSettings {
        mode: loaded.config.screen_mode,
        mouse: loaded.config.mouse && loaded.config.screen_mode == ScreenMode::Fullscreen,
        inline_height: loaded.config.inline_height,
    };
    let mut guard = TerminalGuard::enter_with(settings)?;
    let viewport = match settings.mode {
        ScreenMode::Inline => Viewport::Fixed(guard.inline_area(None)?),
        ScreenMode::Fullscreen => Viewport::Fullscreen,
    };
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions { viewport },
    )
    .context("could not initialise terminal")?;
    let mut app = App::with_config(tmux, options, loaded.config, loaded.path);
    loop {
        // Hand control back before drawing another frame.
        if let Some(exit) = app.exit().cloned() {
            drop(terminal);
            return match exit {
                Exit::Quit | Exit::Switched => guard.restore(),
                Exit::Attach(target) => exec_tmux(guard, tmux.attach_command(&target)),
                Exit::Detach(client) => {
                    guard.restore()?;
                    tmux.detach_client(&client)
                        .context("cannot detach from tmux")
                }
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
        if resized && settings.mode == ScreenMode::Inline && app.exit().is_none() {
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
        Event::Mouse(mouse) => matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ),
        Event::FocusGained | Event::FocusLost => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fullscreen_is_default_and_inline_or_fullscreen_are_explicit() {
        let default_cli = Cli::try_parse_from(["jm"]).expect("default CLI");
        assert!(!default_cli.fullscreen);
        assert!(!default_cli.inline);
        assert_eq!(cli_screen_override(&default_cli), None);

        let inline_cli = Cli::try_parse_from(["jm", "--inline", "resume"]).expect("inline CLI");
        assert!(inline_cli.inline);
        assert_eq!(cli_screen_override(&inline_cli), Some(ScreenMode::Inline));
        assert_eq!(inline_cli.command, vec!["resume"]);

        let full_cli = Cli::try_parse_from(["jm", "--fullscreen", "resume"]).expect("full CLI");
        assert!(full_cli.fullscreen);
        assert_eq!(cli_screen_override(&full_cli), Some(ScreenMode::Fullscreen));
        assert_eq!(full_cli.command, vec!["resume"]);

        assert!(Cli::try_parse_from(["jm", "--inline", "--fullscreen"]).is_err());
    }

    #[test]
    fn parses_config_theme_and_print_config_flags() {
        let cli = Cli::try_parse_from([
            "jm",
            "-c",
            "/tmp/custom.toml",
            "--theme",
            "codex",
            "--print-config",
        ])
        .expect("config CLI");
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/custom.toml")));
        assert_eq!(cli.theme.as_deref(), Some("codex"));
        assert!(cli.print_config);
    }

    #[test]
    fn focus_and_key_release_events_do_not_trigger_redraws() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
        assert!(!redraw_event(&Event::FocusGained));
        assert!(!redraw_event(&Event::FocusLost));
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert!(redraw_event(&Event::Key(key)));
        assert!(!redraw_event(&Event::Key(KeyEvent {
            kind: KeyEventKind::Release,
            ..key
        })));
        assert!(redraw_event(&Event::Resize(80, 24)));
        assert!(redraw_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(redraw_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(redraw_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(!redraw_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })));
        assert!(!redraw_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })));
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

    #[test]
    fn doctor_theme_summary_reports_preset_and_color_mode() {
        use crate::ui::{Theme, ThemePreset};
        assert_eq!(
            theme_summary(Theme::default().with_preset(ThemePreset::AntigravityAurora)),
            "Antigravity Aurora (truecolor)"
        );
        assert_eq!(theme_summary(Theme::default()), "Classic (ANSI)");
        assert_eq!(
            theme_summary(Theme {
                no_color: true,
                ..Theme::default()
            }),
            "Classic (NO_COLOR)"
        );
    }
}
