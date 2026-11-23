#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use justmux::{
    app::{App, AppOptions, Exit},
    commands::{self, CommandKind},
    config::{self, CliOverrides, Config},
    fuzzy::FuzzyMatcher,
    terminal::ScreenMode,
    tmux::{SocketSpec, Target, Tmux, TmuxClient},
    tributes,
    ui::{ComposerMode, EmblemMode, Theme, ThemePreset},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use tempfile::TempDir;

struct PrivateServer {
    socket: String,
    client: TmuxClient,
}

static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

impl PrivateServer {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let socket = format!("jm-tt-{}-{}-{}", std::process::id(), stamp, counter);
        let client =
            TmuxClient::new(SocketSpec::Name(socket.clone())).with_config_file("/dev/null");
        Self { socket, client }
    }

    fn tmux(&self, args: &[&str]) -> std::process::Output {
        Command::new("tmux")
            .arg("-L")
            .arg(&self.socket)
            .arg("-f")
            .arg("/dev/null")
            .args(args)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("tmux executable")
    }
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .arg("-L")
            .arg(&self.socket)
            .arg("-f")
            .arg("/dev/null")
            .arg("kill-server")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output();
    }
}

fn offline_tmux() -> TmuxClient {
    TmuxClient::new(SocketSpec::Name(format!(
        "jm-tt-offline-{}",
        std::process::id()
    )))
    .with_program("/nonexistent-justmux-test-tmux")
}

fn key_mod(app: &mut App<'_>, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

fn key(app: &mut App<'_>, code: KeyCode) {
    key_mod(app, code, KeyModifiers::NONE);
}

fn type_text(app: &mut App<'_>, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

fn render_app(app: &mut App<'_>, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    terminal
}

#[test]
fn inline_popup_at_100x16_keeps_quit_visible_after_10_down_presses() {
    let reg = commands::registry();
    assert_eq!(reg.len(), 19);
    assert_eq!(reg.first().map(|cmd| cmd.kind), Some(CommandKind::Resume));
    assert_eq!(reg.get(9).map(|cmd| cmd.kind), Some(CommandKind::Expose));
    assert_eq!(reg.last().map(|cmd| cmd.kind), Some(CommandKind::Quit));
    // Verify contiguous grouped blocks: Workspace [0..6], Studio [6..14], System [14..19].
    assert!(
        reg[..6]
            .iter()
            .all(|c| c.group == commands::CommandGroup::Workspace)
    );
    assert!(
        reg[6..14]
            .iter()
            .all(|c| c.group == commands::CommandGroup::Studio)
    );
    assert!(
        reg[14..]
            .iter()
            .all(|c| c.group == commands::CommandGroup::System)
    );

    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/");

    // Pressing `Up` once from index 0 wraps immediately to the last command (`/quit`).
    key(&mut app, KeyCode::Up);
    let wrapped_up = render_app(&mut app, 100, 16).backend().to_string();
    assert!(wrapped_up.contains("/theme"), "{wrapped_up}");
    assert!(wrapped_up.contains("/config"), "{wrapped_up}");
    assert!(wrapped_up.contains("/kill-server"), "{wrapped_up}");
    assert!(wrapped_up.contains("/quit"), "{wrapped_up}");

    // One `Down` from `/quit` wraps back to index 0 (`/resume`).
    key(&mut app, KeyCode::Down);
    let wrapped_back = render_app(&mut app, 100, 16).backend().to_string();
    assert!(wrapped_back.contains("/resume"), "{wrapped_back}");

    // 18 `Down` presses from index 0 reach index 18 (`/quit`) with bottom commands visible.
    for _ in 0..(reg.len() - 1) {
        key(&mut app, KeyCode::Down);
    }
    let at_quit = render_app(&mut app, 100, 16).backend().to_string();
    assert!(at_quit.contains("/theme"), "{at_quit}");
    assert!(at_quit.contains("/config"), "{at_quit}");
    assert!(at_quit.contains("/kill-server"), "{at_quit}");
    assert!(at_quit.contains("/quit"), "{at_quit}");

    // One more `Down` (19th) wraps back to index 0 (`/resume`).
    key(&mut app, KeyCode::Down);
    let wrapped_to_resume = render_app(&mut app, 100, 16).backend().to_string();
    assert!(wrapped_to_resume.contains("/resume"), "{wrapped_to_resume}");
}

#[test]
fn zero_learning_home_shortcuts_and_bare_command_forgiveness() {
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());

    // Down on empty composer opens the `/` palette showing top Workspace & Studio commands.
    key(&mut app, KeyCode::Down);
    let output = render_app(&mut app, 80, 16).backend().to_string();
    assert!(output.contains("/resume"), "{output}");
    assert!(output.contains("/snapshot"), "{output}");
    assert!(output.contains("/pane"), "{output}");
    assert!(output.contains("/split"), "{output}");
    assert!(output.contains("/layout"), "{output}");
    assert!(output.contains("/expose"), "{output}");
    assert!(output.contains("/popup"), "{output}");

    // Pressing `Up` once wraps to the bottom, immediately surfacing System commands.
    key(&mut app, KeyCode::Up);
    let wrapped = render_app(&mut app, 80, 16).backend().to_string();
    assert!(wrapped.contains("/clients"), "{wrapped}");
    assert!(wrapped.contains("/exit"), "{wrapped}");
    assert!(wrapped.contains("/kill-server"), "{wrapped}");
    assert!(wrapped.contains("/quit"), "{wrapped}");
    key(&mut app, KeyCode::Esc);

    // `?` on empty composer opens Help immediately (both plain and Shift+? variants).
    key(&mut app, KeyCode::Char('?'));
    let output = render_app(&mut app, 80, 16).backend().to_string();
    assert!(output.contains("JustMux commands"), "{output}");
    key(&mut app, KeyCode::Esc);

    key_mod(&mut app, KeyCode::Char('?'), KeyModifiers::SHIFT);
    let output = render_app(&mut app, 80, 16).backend().to_string();
    assert!(output.contains("JustMux commands"), "{output}");
    key(&mut app, KeyCode::Esc);

    // Bare "resume" executes `/resume` seamlessly.
    type_text(&mut app, "resume");
    key(&mut app, KeyCode::Enter);
    let output = render_app(&mut app, 80, 16).backend().to_string();
    assert!(output.contains("No tmux sessions"), "{output}");
    key(&mut app, KeyCode::Esc);

    // Unknown bare input keeps the friendly "commands start with /" error.
    type_text(&mut app, "hello");
    key(&mut app, KeyCode::Enter);
    assert!(app.status().contains("commands start with /"));
}

#[test]
fn ctrl_b_easter_egg_ctrl_t_cycle_and_theme_picker_preview_revert_commit() {
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    app.set_theme(Theme::default().with_preset(ThemePreset::AntigravityAurora));

    // Ctrl+b triggers prefix-retirement Easter egg and cycles quips.
    key_mod(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let first_quip = app.status().to_owned();
    assert_eq!(first_quip, tributes::ctrl_b_quip(1));
    key_mod(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let second_quip = app.status().to_owned();
    assert_eq!(second_quip, tributes::ctrl_b_quip(2));
    assert_ne!(first_quip, second_quip);

    // Ctrl+t cycles all 5 ThemePresets cleanly and wraps back to the start.
    let initial = app.theme().preset;
    let mut seen = Vec::new();
    for _ in 0..ThemePreset::ALL.len() {
        key_mod(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
        seen.push(app.theme().preset);
        assert!(app.status().contains(app.theme().preset.name()));
    }
    assert_eq!(seen.len(), ThemePreset::ALL.len());
    assert_eq!(app.theme().preset, initial);

    // `/theme` opens live preview; Down previews, Esc reverts to initial.
    type_text(&mut app, "/theme");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    let picker_screen = render_app(&mut app, 80, 16).backend().to_string();
    assert!(picker_screen.contains("Theme palette"), "{picker_screen}");

    key(&mut app, KeyCode::Down);
    assert_eq!(app.theme().preset, ThemePreset::CodexEmerald);
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);

    // `/theme` again + Down twice + Enter commits CyberMatrix (or ClaudeWarmth).
    type_text(&mut app, "/theme");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.theme().preset, ThemePreset::ClaudeWarmth);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::ClaudeWarmth);
    assert!(app.status().contains("Claude Warmth"));

    // Direct `/theme matrix` applies immediately.
    type_text(&mut app, "/theme matrix");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::CyberMatrix);
}

#[test]
fn tribute_modals_tour_remix_apply_themes_and_degrade_in_ascii_no_color() {
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    app.set_theme(Theme::default().with_preset(ThemePreset::Classic));

    for (cmd, expected_badge, expected_preset) in [
        ("/codex", "CODEX CLI", ThemePreset::CodexEmerald),
        ("/claude", "CLAUDE CODE", ThemePreset::ClaudeWarmth),
        (
            "/antigravity",
            "ANTIGRAVITY",
            ThemePreset::AntigravityAurora,
        ),
        ("/agy", "ANTIGRAVITY", ThemePreset::AntigravityAurora),
    ] {
        type_text(&mut app, cmd);
        key(&mut app, KeyCode::Enter);
        let screen = render_app(&mut app, 90, 20).backend().to_string();
        assert!(screen.contains(expected_badge), "{cmd}: {screen}");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.theme().preset, expected_preset);
        assert!(app.status().contains(expected_preset.name()));
    }

    // `/vibe` opens HallOfFame, tours with Right/Left, remixes with Space, and applies with `t`.
    type_text(&mut app, "/vibe");
    key(&mut app, KeyCode::Enter);
    let initial_screen = render_app(&mut app, 90, 20).backend().to_string();
    assert!(initial_screen.contains("TRIBUTE VAULT"), "{initial_screen}");

    key(&mut app, KeyCode::Char(' '));
    let remixed_screen = render_app(&mut app, 90, 20).backend().to_string();
    assert_ne!(initial_screen, remixed_screen);

    key(&mut app, KeyCode::Right);
    let codex_screen = render_app(&mut app, 90, 20).backend().to_string();
    assert!(codex_screen.contains("CODEX CLI"), "{codex_screen}");

    key(&mut app, KeyCode::Right);
    let claude_screen = render_app(&mut app, 90, 20).backend().to_string();
    assert!(claude_screen.contains("CLAUDE CODE"), "{claude_screen}");

    key(&mut app, KeyCode::Left);
    let back_to_codex = render_app(&mut app, 90, 20).backend().to_string();
    assert!(back_to_codex.contains("CODEX CLI"), "{back_to_codex}");

    // Apply Codex signature theme with `t` after resetting theme to Classic first.
    key(&mut app, KeyCode::Esc);
    app.set_theme(Theme {
        ascii: true,
        no_color: true,
        ..Theme::default()
    });
    for (cmd, ascii_art_marker) in [
        ("/codex", "+------------------------+"),
        ("/claude", ".------------------------."),
        ("/antigravity", "+========================+"),
        ("/vibe", "* .--------------------. *"),
    ] {
        type_text(&mut app, cmd);
        key(&mut app, KeyCode::Enter);
        let terminal = render_app(&mut app, 90, 20);
        let screen = terminal.backend().to_string();
        assert!(
            screen.contains('┌'),
            "{cmd} should use Plain block border:\n{screen}"
        );
        assert!(
            !screen.contains('╭'),
            "{cmd} should not use Rounded border in ascii mode:\n{screen}"
        );
        assert!(
            screen.contains(ascii_art_marker),
            "{cmd} missing ascii_art:\n{screen}"
        );
        assert!(
            screen
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "{cmd} content not ASCII:\n{screen}"
        );
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset),
            "{cmd} emitted non-Reset color under no_color"
        );
        key(&mut app, KeyCode::Esc);
    }

    // Also test applying signature theme via `t` key on `/claude`.
    type_text(&mut app, "/claude");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.theme().preset, ThemePreset::ClaudeWarmth);
}

#[test]
fn micro_tributes_execute_and_autocomplete_only_at_two_plus_letters() {
    let mut matcher = FuzzyMatcher::new();

    // Hidden on empty `/` and single letter.
    assert_eq!(commands::registry().len(), 19);
    assert_eq!(
        commands::filter_commands("", &mut matcher).len(),
        commands::registry().len()
    );
    assert!(
        commands::filter_commands("c", &mut matcher)
            .iter()
            .all(|m| !matches!(
                m.command.kind,
                CommandKind::Tribute(_) | CommandKind::MicroTribute(_)
            ))
    );

    // Visible at 2+ letters (`/co`, `/cl`, `/ag`, `/gr`), while core commands rank ahead on ties.
    let co_names: Vec<_> = commands::filter_commands("co", &mut matcher)
        .into_iter()
        .map(|m| m.command.name)
        .collect();
    assert_eq!(co_names.first().copied(), Some("config"));
    assert!(co_names.contains(&"codex"), "{co_names:?}");
    assert!(co_names.contains(&"compact"), "{co_names:?}");
    assert!(co_names.contains(&"cost"), "{co_names:?}");

    let cl_names: Vec<_> = commands::filter_commands("cl", &mut matcher)
        .into_iter()
        .map(|m| m.command.name)
        .collect();
    assert!(cl_names.contains(&"claude"), "{cl_names:?}");

    let ag_names: Vec<_> = commands::filter_commands("ag", &mut matcher)
        .into_iter()
        .map(|m| m.command.name)
        .collect();
    assert!(ag_names.contains(&"antigravity"), "{ag_names:?}");

    let gr_names: Vec<_> = commands::filter_commands("gr", &mut matcher)
        .into_iter()
        .map(|m| m.command.name)
        .collect();
    assert!(gr_names.contains(&"grill-me"), "{gr_names:?}");

    // Execute every micro-tribute in App.
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    app.set_theme(Theme::default().with_preset(ThemePreset::Classic));

    for cmd in ["/compact", "/cost", "/grill-me", "/goal", "/model"] {
        type_text(&mut app, cmd);
        key(&mut app, KeyCode::Enter);
        assert!(!app.status().is_empty(), "{cmd} produced empty status");
    }

    // `/boost` toggles AntigravityAurora <-> CyberMatrix.
    type_text(&mut app, "/boost");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    assert!(!app.status().is_empty());

    type_text(&mut app, "/boost");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::CyberMatrix);
}

#[test]
fn split_renders_visual_wireframe_preview_and_rich_picker_shows_badges() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    server
        .client
        .new_session("codex-dev", cwd)
        .expect("session");

    let mut app = App::new(&server.client, AppOptions::default());
    app.set_theme(Theme::default().with_preset(ThemePreset::AntigravityAurora));

    // Home telemetry should reflect the live session without erroring.
    let home = render_app(&mut app, 90, 16).backend().to_string();
    assert!(home.contains("1s"), "{home}");
    assert!(home.contains("1w"), "{home}");

    // `/list` switches Home Matrix to INSPECT mode with live session and detached pill.
    type_text(&mut app, "/list");
    key(&mut app, KeyCode::Enter);
    let list_screen = render_app(&mut app, 90, 16).backend().to_string();
    assert!(list_screen.contains("INSPECT"), "{list_screen}");
    assert!(list_screen.contains("codex-dev"), "{list_screen}");
    assert!(list_screen.contains("detached"), "{list_screen}");
    key(&mut app, KeyCode::Esc);

    // `/pane` opens render_rich_picker with pane row and active/busy badge.
    type_text(&mut app, "/pane");
    key(&mut app, KeyCode::Enter);
    let pane_screen = render_app(&mut app, 90, 16).backend().to_string();
    assert!(pane_screen.contains("codex-dev"), "{pane_screen}");
    assert!(
        pane_screen.contains("active") || pane_screen.contains("busy"),
        "{pane_screen}"
    );
    key(&mut app, KeyCode::Esc);

    // `/split` -> Enter opens interactive Split Studio; `h` splits into 2 panes (#1 & #2).
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    let split_unicode = render_app(&mut app, 100, 26).backend().to_string();
    assert!(split_unicode.contains("#1 ★"), "{split_unicode}");
    assert!(split_unicode.contains("codex-dev"), "{split_unicode}");
    assert!(
        split_unicode.contains("[Enter] Apply & Enter"),
        "{split_unicode}"
    );
    key(&mut app, KeyCode::Char('h'));
    let split_two = render_app(&mut app, 100, 26).backend().to_string();
    assert!(split_two.contains("#1"), "{split_two}");
    assert!(split_two.contains("#2 ★"), "{split_two}");
    assert!(split_two.contains("+ New shell"), "{split_two}");

    // ASCII + NO_COLOR fallback on `/split` stays pure ASCII/box-drawing with zero color.
    app.set_theme(Theme {
        ascii: true,
        no_color: true,
        ..app.theme()
    });
    let split_ascii = render_app(&mut app, 100, 26).backend().to_string();
    assert!(
        split_ascii
            .chars()
            .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
        "{split_ascii}"
    );
    assert!(split_ascii.contains("#1"), "{split_ascii}");
    assert!(split_ascii.contains("#2 *"), "{split_ascii}");
    assert!(split_ascii.contains("+ New shell"), "{split_ascii}");
}

fn wait_for_pane_text(client: &TmuxClient, target: &Target, needle: &str) {
    for _ in 0..200 {
        if client
            .capture_pane(target, 20)
            .ok()
            .is_some_and(|lines| lines.iter().any(|l| l.contains(needle)))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out waiting for {needle:?} in {target}");
}

#[test]
fn home_studio_two_column_card_mascot_and_72x16_compatibility() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    server.client.new_session("claude-core", cwd).expect("s1");
    server.client.new_session("codex-api", cwd).expect("s2");

    let mut app = App::new(&server.client, AppOptions::default());
    app.set_theme(Theme::default().with_preset(ThemePreset::ClaudeWarmth));

    let wide = render_app(&mut app, 100, 16).backend().to_string();
    assert!(wide.contains("╭"), "{wide}");
    assert!(wide.contains("◫ JustMux"), "{wide}");
    assert!(wide.contains("claude-core"), "{wide}");
    assert!(wide.contains("codex-api"), "{wide}");
    assert!(wide.contains("Window Preview"), "{wide}");
    assert!(wide.contains("Search: session or window"), "{wide}");
    assert!(wide.contains("Shift+Tab"), "{wide}");

    let compact_studio = render_app(&mut app, 72, 16).backend().to_string();
    assert!(compact_studio.contains("◫ JustMux"), "{compact_studio}");
    assert!(compact_studio.contains("claude-core"), "{compact_studio}");
    assert!(
        compact_studio.contains("Search: session or window"),
        "{compact_studio}"
    );
    assert!(
        compact_studio.contains("Type / for commands"),
        "{compact_studio}"
    );
}

#[test]
fn shift_tab_modes_live_radar_quick_launch_and_digit_jump() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Empty server: pressing '1' in CommandJump opens New session form.
    let mut empty_app = App::new(&server.client, AppOptions::default());
    key(&mut empty_app, KeyCode::Char('1'));
    let new_form = render_app(&mut empty_app, 80, 16).backend().to_string();
    assert!(new_form.contains("Session name"), "{new_form}");

    // Create two live sessions and emit sentinel output in the first one.
    let s1 = server.client.new_session("alpha", cwd).expect("alpha");
    let _s2 = server.client.new_session("beta", cwd).expect("beta");
    let _ = Command::new("tmux")
        .arg("-L")
        .arg(&server.socket)
        .args([
            "send-keys",
            "-t",
            s1.id.as_str(),
            "echo HELLO_LIVE_PREVIEW",
            "Enter",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("send-keys");
    wait_for_pane_text(
        &server.client,
        &Target::Session(s1.id.clone()),
        "HELLO_LIVE_PREVIEW",
    );

    // 1-9 direct session jump in CommandJump.
    let mut jump_app = App::new(&server.client, AppOptions::default());
    key(&mut jump_app, KeyCode::Char('1'));
    assert!(matches!(
        jump_app.exit(),
        Some(Exit::Attach(Target::Session(_)))
    ));

    // Shift+Tab cycles CommandJump -> LiveRadar -> QuickLaunch -> CommandJump.
    let mut app = App::new(&server.client, AppOptions::default());
    key_mod(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
    let radar = render_app(&mut app, 100, 18).backend().to_string();
    assert!(radar.contains("Live Radar (2)"), "{radar}");
    if !radar.contains("HELLO_LIVE_PREVIEW") {
        key(&mut app, KeyCode::Down);
        let radar2 = render_app(&mut app, 100, 18).backend().to_string();
        assert!(radar2.contains("HELLO_LIVE_PREVIEW"), "{radar}\n{radar2}");
    }
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.exit(), Some(Exit::Attach(Target::Session(_)))));

    // QuickLaunch creates and attaches to a typed workspace or cwd basename on empty Enter.
    let mut ql_app = App::new(&server.client, AppOptions::default());
    key_mod(&mut ql_app, KeyCode::BackTab, KeyModifiers::SHIFT);
    key_mod(&mut ql_app, KeyCode::BackTab, KeyModifiers::SHIFT);
    let ql_screen = render_app(&mut ql_app, 100, 16).backend().to_string();
    assert!(ql_screen.contains("Quick Launch"), "{ql_screen}");
    type_text(&mut ql_app, "gamma");
    key(&mut ql_app, KeyCode::Enter);
    assert!(
        server
            .client
            .list_sessions()
            .expect("sessions")
            .iter()
            .any(|s| s.name == "gamma")
    );
    assert!(matches!(
        ql_app.exit(),
        Some(Exit::Attach(Target::Session(_)))
    ));

    // Empty QuickLaunch Enter derives clean cwd basename and creates/enters it.
    let mut ql_empty = App::new(&server.client, AppOptions::default());
    key_mod(&mut ql_empty, KeyCode::BackTab, KeyModifiers::SHIFT);
    key_mod(&mut ql_empty, KeyCode::BackTab, KeyModifiers::SHIFT);
    key(&mut ql_empty, KeyCode::Enter);
    assert!(matches!(
        ql_empty.exit(),
        Some(Exit::Attach(Target::Session(_)))
    ));
}

#[test]
fn at_mentions_ctrl_p_ctrl_k_and_live_previews_in_picker_resume_and_confirm() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dev = server
        .client
        .new_session("codex-api", cwd)
        .expect("codex-api");
    let _ = Command::new("tmux")
        .arg("-L")
        .arg(&server.socket)
        .args([
            "send-keys",
            "-t",
            dev.id.as_str(),
            "echo PICKER_PREVIEW_OK",
            "Enter",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("send-keys");
    wait_for_pane_text(
        &server.client,
        &Target::Session(dev.id.clone()),
        "PICKER_PREVIEW_OK",
    );

    // Live preview in `/list` and `/resume`.
    let mut app = App::new(&server.client, AppOptions::default());
    type_text(&mut app, "/list");
    key(&mut app, KeyCode::Enter);
    let list_screen = render_app(&mut app, 100, 18).backend().to_string();
    assert!(list_screen.contains("PICKER_PREVIEW_OK"), "{list_screen}");

    // Ctrl+P from Picker clears stack and opens `/` command palette.
    key_mod(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    let palette_screen = render_app(&mut app, 80, 16).backend().to_string();
    assert!(palette_screen.contains("/resume"), "{palette_screen}");
    key(&mut app, KeyCode::Esc);

    // `/resume` shows Window Preview with captured lines.
    type_text(&mut app, "/resume");
    key(&mut app, KeyCode::Enter);
    let resume_screen = render_app(&mut app, 90, 18).backend().to_string();
    assert!(resume_screen.contains("Window Preview"), "{resume_screen}");
    assert!(
        resume_screen.contains("PICKER_PREVIEW_OK"),
        "{resume_screen}"
    );
    key(&mut app, KeyCode::Esc);

    // Ctrl+K inside Help opens `/`, while Ctrl+K in non-empty composer kills to end of line.
    key(&mut app, KeyCode::Char('?'));
    assert!(
        render_app(&mut app, 80, 16)
            .backend()
            .to_string()
            .contains("JustMux commands")
    );
    key_mod(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(
        render_app(&mut app, 80, 16)
            .backend()
            .to_string()
            .contains("/resume")
    );
    key(&mut app, KeyCode::Esc);

    type_text(&mut app, "abcd");
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);
    key_mod(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL);
    type_text(&mut app, "Z");
    let composer_screen = render_app(&mut app, 80, 16).backend().to_string();
    assert!(composer_screen.contains("abZ"), "{composer_screen}");
    key(&mut app, KeyCode::Esc);

    // Ctrl+P reverts live ThemePicker preview.
    app.set_theme(Theme::default().with_preset(ThemePreset::AntigravityAurora));
    type_text(&mut app, "/theme");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.theme().preset, ThemePreset::CodexEmerald);
    key_mod(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    key(&mut app, KeyCode::Esc);

    // `@` opens `@ Targets` drawer and Tab completes, `@codex-api` attaches on Enter.
    type_text(&mut app, "@cod");
    let mention_screen = render_app(&mut app, 90, 16).backend().to_string();
    assert!(mention_screen.contains("@ Targets"), "{mention_screen}");
    assert!(mention_screen.contains("@codex-api"), "{mention_screen}");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.exit(),
        Some(&Exit::Attach(Target::Session(dev.id.clone())))
    );

    // `/kill @codex-api` opens Codex Approval Card and defaults to cancel.
    let mut kill_app = App::new(&server.client, AppOptions::default());
    type_text(&mut kill_app, "/kill @codex-api");
    key(&mut kill_app, KeyCode::Enter);
    let confirm_screen = render_app(&mut kill_app, 90, 18).backend().to_string();
    assert!(
        confirm_screen.contains("Approval Required"),
        "{confirm_screen}"
    );
    assert!(
        confirm_screen.contains("[sandbox: native-tmux]"),
        "{confirm_screen}"
    );
    assert!(
        confirm_screen.contains("- kill codex-api"),
        "{confirm_screen}"
    );
    key(&mut kill_app, KeyCode::Enter);
    assert_eq!(kill_app.status(), "Cancelled. Nothing was killed.");

    // `/rename @codex-api api-renamed` renames directly.
    type_text(&mut kill_app, "/rename @codex-api api-renamed");
    key(&mut kill_app, KeyCode::Enter);
    assert!(
        server
            .client
            .list_sessions()
            .expect("renamed")
            .iter()
            .any(|s| s.id == dev.id && s.name == "api-renamed")
    );

    // `/split @api-renamed` opens interactive Split Studio seeded with api-renamed.
    type_text(&mut kill_app, "/split @api-renamed");
    key(&mut kill_app, KeyCode::Enter);
    let split_screen = render_app(&mut kill_app, 100, 26).backend().to_string();
    assert!(split_screen.contains("#1 ★"), "{split_screen}");
    assert!(split_screen.contains("api-renamed"), "{split_screen}");
    assert!(
        split_screen.contains("[Enter] Apply & Enter"),
        "{split_screen}"
    );
}

#[test]
fn natural_language_zero_syntax_intent_and_fallback() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let s = server.client.new_session("my-app", cwd).expect("my-app");

    let mut app = App::new(&server.client, AppOptions::default());

    // `"new demo"` & `"create alpha"` open directory prompt for the session name.
    type_text(&mut app, "new demo");
    key(&mut app, KeyCode::Enter);
    assert!(
        render_app(&mut app, 80, 16)
            .backend()
            .to_string()
            .contains("Working directory")
    );
    key(&mut app, KeyCode::Esc);

    type_text(&mut app, "create alpha");
    key(&mut app, KeyCode::Enter);
    assert!(
        render_app(&mut app, 80, 16)
            .backend()
            .to_string()
            .contains("Working directory")
    );
    key(&mut app, KeyCode::Esc);

    // `"split right"`, `"split down"`, `"hsplit"`, `"vsplit"` split panes directly.
    type_text(&mut app, "split right");
    key(&mut app, KeyCode::Enter);
    assert_eq!(server.client.list_panes(None).expect("panes").len(), 2);

    type_text(&mut app, "split down");
    key(&mut app, KeyCode::Enter);
    assert_eq!(server.client.list_panes(None).expect("panes").len(), 3);

    type_text(&mut app, "hsplit");
    key(&mut app, KeyCode::Enter);
    assert_eq!(server.client.list_panes(None).expect("panes").len(), 4);

    // `"kill my-app"` and `"close my-app"` open Codex Approval Card.
    type_text(&mut app, "kill my-app");
    key(&mut app, KeyCode::Enter);
    assert!(
        render_app(&mut app, 90, 18)
            .backend()
            .to_string()
            .contains("Approval Required")
    );
    key(&mut app, KeyCode::Esc);

    type_text(&mut app, "close my-app");
    key(&mut app, KeyCode::Enter);
    assert!(
        render_app(&mut app, 90, 18)
            .backend()
            .to_string()
            .contains("Approval Required")
    );
    key(&mut app, KeyCode::Esc);

    // Unknown `"hello"` keeps the `"commands start with /"` guidance.
    type_text(&mut app, "hello");
    key(&mut app, KeyCode::Enter);
    assert!(app.status().contains("commands start with /"));

    // Case-insensitive exact session name filters the Home Matrix and attaches directly.
    let mut jump_app = App::new(&server.client, AppOptions::default());
    type_text(&mut jump_app, "MY-APP");
    key(&mut jump_app, KeyCode::Enter);
    assert!(matches!(
        jump_app.exit(),
        Some(Exit::Attach(Target::Window(sid, _))) if sid == &s.id
    ));
}

fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn find_text_cell_in_cols(
    term: &Terminal<TestBackend>,
    needle: &str,
    max_col: usize,
) -> Option<(u16, u16)> {
    let rendered = term.backend().to_string();
    for (row_idx, line) in rendered.lines().enumerate() {
        let prefix: String = line.chars().take(max_col).collect();
        if let Some(byte_col) = prefix.find(needle) {
            let col = prefix[..byte_col].chars().count();
            return Some((col as u16, row_idx as u16));
        }
    }
    None
}

#[test]
fn config_precedence_and_defaults_are_applied_to_app() {
    let dir = TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    std::fs::write(
        &cfg_path,
        r#"
[ui]
theme = "codex"
composer_mode = "live-radar"
live_preview = false
ascii = true

[terminal]
screen_mode = "inline"
mouse = false
inline_height = 22
"#,
    )
    .expect("write config");

    assert_eq!(Config::default().screen_mode, ScreenMode::Fullscreen);
    assert!(Config::default().mouse);
    assert!(Config::default().live_preview);

    let loaded_file = config::load(&CliOverrides {
        config_path: Some(cfg_path.clone()),
        screen_mode: None,
        theme: None,
    })
    .expect("load config file");
    assert!(loaded_file.loaded_from_file);
    assert_eq!(loaded_file.config.screen_mode, ScreenMode::Inline);
    assert_eq!(loaded_file.config.inline_height, 22);
    assert!(!loaded_file.config.mouse);
    assert!(!loaded_file.config.live_preview);
    assert_eq!(loaded_file.config.composer_mode, ComposerMode::LiveRadar);

    let loaded_cli = config::load(&CliOverrides {
        config_path: Some(cfg_path.clone()),
        screen_mode: Some(ScreenMode::Fullscreen),
        theme: Some("claude".into()),
    })
    .expect("load with CLI overrides");
    assert_eq!(loaded_cli.config.screen_mode, ScreenMode::Fullscreen);
    assert_eq!(loaded_cli.config.theme, ThemePreset::ClaudeWarmth);
    assert_eq!(loaded_cli.config.inline_height, 22);
    assert!(!loaded_cli.config.mouse);

    let tmux = offline_tmux();
    let app = App::with_config(
        &tmux,
        AppOptions::default(),
        loaded_cli.config.clone(),
        loaded_cli.path,
    );
    assert_eq!(app.theme().preset, ThemePreset::ClaudeWarmth);
    assert!(app.theme().ascii);
    assert_eq!(app.config().inline_height, 22);
}

#[test]
fn config_command_and_theme_save_persist_to_tempfile() {
    let dir = TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("nested").join("config.toml");
    let tmux = offline_tmux();

    assert_eq!(
        commands::parse("/config").unwrap().command,
        CommandKind::Config
    );
    assert_eq!(
        commands::parse("/settings").unwrap().command,
        CommandKind::Config
    );
    assert_eq!(
        commands::parse("/prefs").unwrap().command,
        CommandKind::Config
    );
    assert!(commands::registry().iter().any(|c| c.name == "config"));

    let mut app = App::with_config(
        &tmux,
        AppOptions::default(),
        Config::default(),
        Some(cfg_path.clone()),
    );
    type_text(&mut app, "/config");
    key(&mut app, KeyCode::Enter);
    let modal = render_app(&mut app, 100, 26).backend().to_string();
    assert!(modal.contains("JustMux Configuration"));
    assert!(modal.contains("screen_mode"));
    assert!(modal.contains("no_color"));

    // Row 0: screen_mode -> inline triggers next-launch note.
    key(&mut app, KeyCode::Right);
    assert_eq!(app.config().screen_mode, ScreenMode::Inline);
    let with_note = render_app(&mut app, 100, 26).backend().to_string();
    assert!(with_note.contains("Takes effect on next jm launch (press s to save)"));

    // Row 1: theme -> CodexEmerald live.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.config().theme, ThemePreset::CodexEmerald);
    assert_eq!(app.theme().preset, ThemePreset::CodexEmerald);

    // Row 3: live_preview -> false; Row 4: mouse -> false.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(!app.config().live_preview);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.config().mouse);

    // Save config to disk with 's'.
    key(&mut app, KeyCode::Char('s'));
    assert!(app.status().contains("Saved config to"));
    let saved = config::load(&CliOverrides {
        config_path: Some(cfg_path.clone()),
        screen_mode: None,
        theme: None,
    })
    .expect("reload saved config");
    assert_eq!(saved.config.screen_mode, ScreenMode::Inline);
    assert_eq!(saved.config.theme, ThemePreset::CodexEmerald);
    assert!(!saved.config.live_preview);
    assert!(!saved.config.mouse);
    assert_eq!(saved.config.language, justmux::ui::Language::En);

    // Move to Row 9 (language), toggle en -> zh live, verify Chinese copy & persistence.
    for _ in 0..5 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.config().language, justmux::ui::Language::Zh);
    assert_eq!(app.theme().language, justmux::ui::Language::Zh);
    let zh_modal = render_app(&mut app, 110, 32).backend().to_string();
    assert!(zh_modal.contains("JustMux 设置与参数"), "{zh_modal}");
    assert!(zh_modal.contains("简体中文"), "{zh_modal}");
    key(&mut app, KeyCode::Char('s'));
    let saved_zh = config::load(&CliOverrides {
        config_path: Some(cfg_path.clone()),
        screen_mode: None,
        theme: None,
    })
    .expect("reload saved zh config");
    assert_eq!(saved_zh.config.language, justmux::ui::Language::Zh);
    // Toggle back to English before exiting /config.
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.config().language, justmux::ui::Language::En);
    key(&mut app, KeyCode::Char('s'));

    // Close /config and save a different theme from /theme via 's'.
    key(&mut app, KeyCode::Esc);
    type_text(&mut app, "/theme");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('s'));
    assert!(app.status().contains("Saved theme 'claude'"));
    let reloaded_theme = config::load(&CliOverrides {
        config_path: Some(cfg_path),
        screen_mode: None,
        theme: None,
    })
    .expect("reload saved theme");
    assert_eq!(reloaded_theme.config.theme, ThemePreset::ClaudeWarmth);
}

#[test]
fn fullscreen_mouse_scroll_and_clicks_navigate_and_respect_mouse_flag() {
    let server = PrivateServer::new();
    let s1 = server
        .client
        .new_session("alpha", Path::new("/tmp"))
        .expect("alpha session");
    let _s2 = server
        .client
        .new_session("beta", Path::new("/tmp"))
        .expect("beta session");

    // When mouse = false, mouse scroll/clicks are ignored.
    let no_mouse_cfg = Config {
        mouse: false,
        ..Config::default()
    };
    let mut disabled_app =
        App::with_config(&server.client, AppOptions::default(), no_mouse_cfg, None);
    disabled_app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    let disabled_term = render_app(&mut disabled_app, 100, 24);
    if let Some((col, row)) = find_text_cell_in_cols(&disabled_term, "alpha", 45) {
        disabled_app.handle_event(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            col,
            row,
        ));
    }
    assert!(disabled_app.exit().is_none());

    // When mouse = true, wheel scrolls and left-click selects/activates in LiveRadar & Picker.
    let mut app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    let radar_term = render_app(&mut app, 100, 24);
    let (alpha_col, alpha_row) =
        find_text_cell_in_cols(&radar_term, "alpha", 45).expect("alpha row in LiveRadar");
    let (beta_col, beta_row) =
        find_text_cell_in_cols(&radar_term, "beta", 45).expect("beta row in LiveRadar");
    assert_ne!(alpha_row, beta_row);

    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 5));
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 5));
    // Click whichever row is second (currently unselected): 1st click selects, 2nd click attaches.
    let (second_col, second_row, expected_id) = if beta_row > alpha_row {
        (beta_col, beta_row, _s2.id.clone())
    } else {
        (alpha_col, alpha_row, s1.id.clone())
    };
    app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        second_col,
        second_row,
    ));
    assert!(app.exit().is_none());
    app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        second_col,
        second_row,
    ));
    assert_eq!(
        app.exit(),
        Some(&Exit::Attach(Target::Session(expected_id)))
    );

    // Unified Home Matrix (CommandJump): wheel inside Window Preview scrolls scrollback without
    // changing selection; wheel in the lower matrix scrolls windows; clicks select/attach.
    let s1_w0 = server
        .client
        .list_windows(Some(&s1.id))
        .expect("alpha windows")
        .into_iter()
        .next()
        .expect("alpha window 0");
    let sent = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "send-keys",
            "-t",
            &s1_w0.id.to_string(),
            "seq -f 'MOUSE_PREV_%02g' 1 30",
            "Enter",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("send-keys to alpha");
    assert!(sent.status.success());
    wait_for_pane_text(
        &server.client,
        &Target::Window(s1.id.clone(), s1_w0.id),
        "MOUSE_PREV_30",
    );
    let _s1_win2 = server
        .client
        .new_window(&s1.id, Some("logs"), None)
        .expect("logs window");
    let mut matrix_app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );
    let mut matrix_term = render_app(&mut matrix_app, 100, 24);
    // Session columns are ordered by 1-second tmux activity timestamps, so
    // ensure `alpha:0` is selected before testing its 30-line preview scroll.
    if !matrix_term
        .backend()
        .to_string()
        .contains("Window Preview · alpha")
    {
        key(&mut matrix_app, KeyCode::Right);
        matrix_term = render_app(&mut matrix_app, 100, 24);
    }
    if matrix_term
        .backend()
        .to_string()
        .contains("Window Preview · alpha · 1:")
    {
        key(&mut matrix_app, KeyCode::Up);
        matrix_term = render_app(&mut matrix_app, 100, 24);
    }
    assert!(
        matrix_term
            .backend()
            .to_string()
            .contains("Window Preview · alpha"),
        "{}",
        matrix_term.backend()
    );
    // Wheel inside Window Preview (top band) scrolls preview without leaving alpha:0.
    matrix_app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 4));
    let preview_scrolled = render_app(&mut matrix_app, 100, 24).backend().to_string();
    assert!(preview_scrolled.contains("↑ +3/"), "{preview_scrolled}");
    assert!(
        preview_scrolled.contains("Window Preview · alpha"),
        "{preview_scrolled}"
    );
    matrix_app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 4));
    let preview_bottom = render_app(&mut matrix_app, 100, 24).backend().to_string();
    assert!(preview_bottom.contains("↕ "), "{preview_bottom}");

    let (beta_col_m, beta_row_m) =
        find_text_cell_in_cols(&matrix_term, "beta", 95).expect("beta column on Home Matrix");
    // Wheel inside the lower matrix navigates alpha windows without exiting.
    matrix_app.handle_event(mouse_event(
        MouseEventKind::ScrollDown,
        beta_col_m,
        beta_row_m + 1,
    ));
    let scrolled_win = render_app(&mut matrix_app, 100, 24).backend().to_string();
    assert!(scrolled_win.contains("1: logs"), "{scrolled_win}");
    matrix_app.handle_event(mouse_event(
        MouseEventKind::ScrollUp,
        beta_col_m,
        beta_row_m + 1,
    ));
    // Clicking Action Tab Bar entries switches matrix action mode in place.
    let (rename_tab_col, rename_tab_row) =
        find_buffer_text(matrix_term.backend().buffer(), "Rename").expect("Rename tab");
    matrix_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        rename_tab_col,
        rename_tab_row,
    ));
    let rename_tab_screen = render_app(&mut matrix_app, 100, 24).backend().to_string();
    assert!(rename_tab_screen.contains("RENAME"), "{rename_tab_screen}");
    let (resume_tab_col, resume_tab_row) =
        find_buffer_text(matrix_term.backend().buffer(), "Resume").expect("Resume tab");
    matrix_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        resume_tab_col,
        resume_tab_row,
    ));
    assert!(
        render_app(&mut matrix_app, 100, 24)
            .backend()
            .to_string()
            .contains("▸ Resume")
    );

    // Clicking session header in Open mode selects that column without exiting.
    matrix_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        beta_col_m,
        beta_row_m,
    ));
    assert!(matrix_app.exit().is_none());
    // 1st click on beta's window row focuses it; 2nd click attaches to that window.
    matrix_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        beta_col_m,
        beta_row_m + 2,
    ));
    assert!(matrix_app.exit().is_none());
    matrix_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        beta_col_m,
        beta_row_m + 2,
    ));
    assert!(matches!(
        matrix_app.exit(),
        Some(Exit::Attach(Target::Window(..)))
    ));

    // Picker click + wheel test on `/pane`.
    let mut picker_app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );
    type_text(&mut picker_app, "/pane");
    key(&mut picker_app, KeyCode::Enter);
    let picker_term = render_app(&mut picker_app, 100, 24);
    let (p_col, p_row) =
        find_text_cell_in_cols(&picker_term, "beta", 60).expect("beta row in /pane");
    picker_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        p_col,
        p_row,
    ));
    let after_first = render_app(&mut picker_app, 100, 24).backend().to_string();
    if !after_first.contains("Pane actions") {
        picker_app.handle_event(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            p_col,
            p_row,
        ));
    }
    let actions = render_app(&mut picker_app, 100, 24).backend().to_string();
    assert!(actions.contains("Pane actions"));
}

fn hydra_top_ring_row(max_width: u16, max_height: u16, theme: &Theme) -> String {
    let (_, lines) =
        justmux::ui::hydra_emblem_frame(max_width, max_height, 0, *theme).expect("hydra frame");
    lines
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .find(|row| !row.is_empty())
        .expect("non-empty hydra outer ring row")
}

#[test]
fn spacious_fullscreen_home_shows_active_preview_and_respects_live_preview_flag() {
    let server = PrivateServer::new();
    // Create background sessions first with quiet `sleep 30` panes so delayed shell prompts cannot
    // bump their recency ahead of `a-studio-one` across a 1-second boundary.
    for name in ["s2", "s3", "s4", "s5", "s6"] {
        let extra = Command::new("tmux")
            .args([
                "-L",
                &server.socket,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                name,
                "sleep 30",
            ])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("spawn extra session");
        assert!(extra.status.success());
    }

    let out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "a-studio-one",
            "sh -c 'printf \"JM_V3_SPACIOUS_PREVIEW\\n\"; sleep 30'",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("spawn a-studio-one");
    assert!(out.status.success());

    let studio_id = server
        .client
        .list_sessions()
        .expect("list sessions")
        .into_iter()
        .find(|s| s.name == "a-studio-one")
        .expect("a-studio-one session")
        .id;

    // Wait briefly for printf inside the pane so capture_pane sees it reliably.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(lines) = server
            .client
            .capture_pane(&Target::Session(studio_id.clone()), 12)
            && lines.iter().any(|l| l.contains("JM_V3_SPACIOUS_PREVIEW"))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pane never printed preview sentinel"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Add a second window inside a-studio-one after background sessions so its recency >= s2..s6.
    server
        .client
        .new_window(&studio_id, Some("logs"), None)
        .expect("second window");

    // With live_preview = true on 120x32: Window Preview renders on top, and Unified Home Matrix
    // (side-by-side sessions + window rows) renders in the lower band beside the 16x8 Hydra.
    let mut app_on = App::with_config(
        &server.client,
        AppOptions::default(),
        Config {
            live_preview: true,
            ..Config::default()
        },
        None,
    );
    let ring_row_on = hydra_top_ring_row(32, 10, &app_on.theme());
    let wide = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(wide.contains("a-studio-one"), "{wide}");
    assert!(
        ["s2", "s3", "s4", "s5", "s6"]
            .iter()
            .any(|s| wide.contains(s)),
        "{wide}"
    );
    assert!(wide.contains("logs"), "{wide}");
    assert!(wide.contains("Window Preview"), "{wide}");
    assert!(wide.contains("JM_V3_SPACIOUS_PREVIEW"), "{wide}");
    // Preview sits above the matrix, and Hydra renders alongside the matrix in the lower band.
    let wide_lines: Vec<&str> = wide.lines().collect();
    let preview_line_idx = wide_lines
        .iter()
        .position(|line| line.contains("Window Preview"))
        .expect("Window Preview line");
    let matrix_line_idx = wide_lines
        .iter()
        .position(|line| {
            line.contains("a-studio-one")
                && ["s2", "s3", "s4", "s5", "s6"]
                    .iter()
                    .any(|s| line.contains(s))
        })
        .expect("session matrix header row");
    let ring_line_idx = wide_lines
        .iter()
        .position(|line| line.contains(&ring_row_on))
        .expect("lower-band hydra outer ring row");
    assert!(
        preview_line_idx < matrix_line_idx,
        "preview should render above matrix: {wide}"
    );
    assert!(
        ring_line_idx >= matrix_line_idx,
        "hydra should render alongside matrix in lower band: {wide}"
    );

    // Navigating windows and sessions updates Window Preview dynamically.
    key(&mut app_on, KeyCode::Down);
    let win2_view = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(win2_view.contains("Window Preview"), "{win2_view}");
    assert!(!win2_view.contains("JM_V3_SPACIOUS_PREVIEW"), "{win2_view}");
    key(&mut app_on, KeyCode::Up);
    assert!(
        render_app(&mut app_on, 120, 32)
            .backend()
            .to_string()
            .contains("JM_V3_SPACIOUS_PREVIEW")
    );
    key(&mut app_on, KeyCode::Right);
    let session2_view = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(session2_view.contains("Window Preview"), "{session2_view}");
    assert!(
        !session2_view.contains("JM_V3_SPACIOUS_PREVIEW"),
        "{session2_view}"
    );
    key(&mut app_on, KeyCode::Left);
    assert!(
        render_app(&mut app_on, 120, 32)
            .backend()
            .to_string()
            .contains("JM_V3_SPACIOUS_PREVIEW")
    );

    // Action Tab Bar renders all 4 tabs; Tab cycles Resume -> Rename -> Kill -> Split -> Resume.
    assert!(wide.contains("Resume"), "{wide}");
    assert!(wide.contains("Rename"), "{wide}");
    assert!(wide.contains("Kill"), "{wide}");
    assert!(wide.contains("Split"), "{wide}");

    key(&mut app_on, KeyCode::Tab);
    let tab_rename = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(tab_rename.contains("RENAME WINDOW"), "{tab_rename}");
    key(&mut app_on, KeyCode::Up);
    let rename_hdr = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(rename_hdr.contains("RENAME SESSION"), "{rename_hdr}");
    key(&mut app_on, KeyCode::Down);
    let rename_win = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(rename_win.contains("RENAME WINDOW"), "{rename_win}");

    key(&mut app_on, KeyCode::Tab);
    let tab_kill = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(tab_kill.contains("KILL WINDOW"), "{tab_kill}");
    key(&mut app_on, KeyCode::Up);
    let kill_hdr = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(kill_hdr.contains("KILL SESSION"), "{kill_hdr}");
    key(&mut app_on, KeyCode::Down);
    let kill_win = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(kill_win.contains("KILL WINDOW"), "{kill_win}");

    key(&mut app_on, KeyCode::Tab);
    let tab_split = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(tab_split.contains("SPLIT"), "{tab_split}");

    key(&mut app_on, KeyCode::Tab);
    assert!(
        render_app(&mut app_on, 120, 32)
            .backend()
            .to_string()
            .contains("▸ Resume")
    );

    // Command switching stays on Home Matrix and resets on Esc.
    type_text(&mut app_on, "/rename");
    key(&mut app_on, KeyCode::Enter);
    let rename_cmd = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(rename_cmd.contains("RENAME SESSION"), "{rename_cmd}");
    key(&mut app_on, KeyCode::Down);
    let rename_cmd_w = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(rename_cmd_w.contains("RENAME WINDOW"), "{rename_cmd_w}");
    key(&mut app_on, KeyCode::Esc);
    assert!(
        render_app(&mut app_on, 120, 32)
            .backend()
            .to_string()
            .contains("▸ Resume")
    );

    for (cmd, expected_pill) in [
        ("/kill", "KILL WINDOW"),
        ("/split", "SPLIT"),
        ("/window", "WINDOW"),
        ("/list", "INSPECT"),
    ] {
        type_text(&mut app_on, cmd);
        key(&mut app_on, KeyCode::Enter);
        let screen = render_app(&mut app_on, 120, 32).backend().to_string();
        assert!(screen.contains(expected_pill), "{cmd}: {screen}");
        key(&mut app_on, KeyCode::Esc);
        assert!(
            render_app(&mut app_on, 120, 32)
                .backend()
                .to_string()
                .contains("▸ Resume")
        );
    }

    type_text(&mut app_on, "/resume");
    key(&mut app_on, KeyCode::Enter);
    assert!(
        render_app(&mut app_on, 120, 32)
            .backend()
            .to_string()
            .contains("▸ Resume")
    );

    // Navigating to `+ New window` and `+ New session` matrix cells opens creation forms.
    key(&mut app_on, KeyCode::Down);
    key(&mut app_on, KeyCode::Down);
    key(&mut app_on, KeyCode::Enter);
    let new_win_form = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(new_win_form.contains("New window"), "{new_win_form}");
    key(&mut app_on, KeyCode::Esc);

    for _ in 0..6 {
        key(&mut app_on, KeyCode::Right);
    }
    let new_sess_preview = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(
        new_sess_preview.contains("New Session"),
        "{new_sess_preview}"
    );
    key(&mut app_on, KeyCode::Enter);
    let new_sess_form = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(new_sess_form.contains("Session name"), "{new_sess_form}");
    key(&mut app_on, KeyCode::Esc);
    for _ in 0..6 {
        key(&mut app_on, KeyCode::Left);
    }

    // Unified subview chrome wraps overlay commands inside `◫ JustMux` with lower-right Hydra.
    let ring_row_subview = hydra_top_ring_row(32, 13, &app_on.theme());
    let ring_row_compact = hydra_top_ring_row(32, 8, &app_on.theme());
    for (cmd, pill, needle) in [
        ("/new", "NEW SESSION", "Session name"),
        ("/theme", "THEME", "Aurora"),
        ("/config", "CONFIG", "screen_mode"),
        ("/help", "HELP", "/resume"),
        ("/pane", "PANES", "Filter:"),
        ("/vibe", "TRIBUTE VAULT", "TRIBUTE VAULT"),
    ] {
        type_text(&mut app_on, cmd);
        key(&mut app_on, KeyCode::Enter);
        let screen = render_app(&mut app_on, 120, 32).backend().to_string();
        assert!(screen.contains("◫ JustMux"), "{cmd}: {screen}");
        assert!(screen.contains(pill), "{cmd}: {screen}");
        assert!(screen.contains(needle), "{cmd}: {screen}");
        if cmd != "/vibe" {
            assert!(
                screen.contains(&ring_row_subview) || screen.contains(&ring_row_compact),
                "{cmd}: {screen}"
            );
        }
        key(&mut app_on, KeyCode::Esc);
    }

    // With live_preview = false: omits Window Preview on Home and inside /pane.
    let mut app_off = App::with_config(
        &server.client,
        AppOptions::default(),
        Config {
            live_preview: false,
            ..Config::default()
        },
        None,
    );
    let ring_row_off = hydra_top_ring_row(32, 21, &app_off.theme());
    let wide_off = render_app(&mut app_off, 120, 32).backend().to_string();
    assert!(wide_off.contains(&ring_row_off), "{wide_off}");
    assert!(!wide_off.contains("Window Preview"), "{wide_off}");
    assert!(!wide_off.contains("JM_V3_SPACIOUS_PREVIEW"), "{wide_off}");

    type_text(&mut app_off, "/pane");
    key(&mut app_off, KeyCode::Enter);
    let pane_off = render_app(&mut app_off, 100, 24).backend().to_string();
    assert!(!pane_off.contains("JM_V3_SPACIOUS_PREVIEW"), "{pane_off}");
}

#[test]
fn startup_tip_renders_only_on_clean_home_screen() {
    let tmux = offline_tmux();
    let tip = "✦ Tip: You can also launch JustMux anytime with 'jm' · Type / for commands";

    let mut home_app = App::new(&tmux, AppOptions::default());
    home_app.set_startup_tip(tip);
    assert_eq!(home_app.status(), tip);
    let rendered = render_app(&mut home_app, 100, 24).backend().to_string();
    assert!(
        rendered.contains("launch JustMux anytime with 'jm'"),
        "{rendered}"
    );

    // Existing status is not overwritten.
    home_app.set_startup_tip("other tip");
    assert_eq!(home_app.status(), tip);

    // Subviews launched via initial_command do not show a startup home tip.
    let mut subview_app = App::new(
        &tmux,
        AppOptions {
            initial_command: Some("help".into()),
        },
    );
    subview_app.set_startup_tip(tip);
    assert!(subview_app.status().is_empty());
}

#[test]
fn update_command_explains_terminal_update_flow() {
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());

    type_text(&mut app, "/update");
    key(&mut app, KeyCode::Enter);
    assert!(app.status().contains("jm update"), "{}", app.status());
    assert!(app.status().contains("justmux update"), "{}", app.status());

    let mut bare_app = App::new(&tmux, AppOptions::default());
    type_text(&mut bare_app, "update");
    key(&mut bare_app, KeyCode::Enter);
    assert!(
        bare_app.status().contains("jm update"),
        "{}",
        bare_app.status()
    );
}

#[test]
fn emblem_spin_rotates_hydra_and_toggles_via_config() {
    let dir = TempDir::new().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    let tmux = offline_tmux();
    let cfg = Config::default();
    assert_eq!(cfg.emblem_mode, EmblemMode::Spin);

    let mut app = App::with_config(&tmux, AppOptions::default(), cfg, Some(cfg_path.clone()));
    let ring_row_on = hydra_top_ring_row(32, 10, &app.theme());
    let frame0 = render_app(&mut app, 120, 32).backend().to_string();
    assert!(app.should_animate());
    assert!(frame0.contains(&ring_row_on), "{frame0}");

    // Advancing ticks rotates only the inner Hydra pattern while the outer ring stays motionless.
    for _ in 0..9 {
        app.tick_animation();
    }
    assert_eq!(app.emblem_step(), 9);
    let rotated = render_app(&mut app, 120, 32).backend().to_string();
    assert_ne!(rotated, frame0);
    assert!(rotated.contains(&ring_row_on), "{rotated}");

    // Verify full 48x24 scale preserves the canonical outer ring row verbatim across all animated modes.
    let render_48 = |step, mode| {
        let (_, lines) =
            justmux::ui::hydra_emblem_mode_frame(48, 24, step, mode, app.theme()).expect("48x24");
        lines
            .into_iter()
            .map(|l| l.spans.into_iter().map(|s| s.content).collect::<String>())
            .collect::<Vec<String>>()
    };
    let text_48_0 = render_48(0, EmblemMode::Spin);
    let text_48_9 = render_48(9, EmblemMode::Spin);
    let inv_48_0 = render_48(0, EmblemMode::InvertedSpin);
    let inv_48_9 = render_48(9, EmblemMode::InvertedSpin);
    assert!(text_48_0[0].contains("⣀⣠⡤⠶⣖⣚⣛⣛⣿⣿⣛⣛⣓⣲⠶⢤⣄⣀"));
    assert_eq!(text_48_9[0], text_48_0[0]);
    assert_eq!(inv_48_0[0], text_48_0[0]);
    assert_eq!(inv_48_9[0], text_48_0[0]);
    assert_ne!(text_48_0, text_48_9);
    assert_ne!(inv_48_0, text_48_0);
    assert_ne!(inv_48_0, inv_48_9);

    for _ in 9..justmux::ui::HYDRA_ROTATION_STEPS {
        app.tick_animation();
    }
    let wrapped = render_app(&mut app, 120, 32).backend().to_string();
    assert!(wrapped.contains(&ring_row_on), "{wrapped}");

    // Advance away from frame 0, then verify compact/LiveRadar disable animation cleanly.
    app.tick_animation();
    assert_ne!(render_app(&mut app, 120, 32).backend().to_string(), wrapped);
    let _ = render_app(&mut app, 100, 16);
    assert!(!app.should_animate());

    let _ = render_app(&mut app, 120, 32);
    assert!(app.should_animate());
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    let _ = render_app(&mut app, 120, 32);
    assert!(!app.should_animate());

    // Switch back from LiveRadar -> QuickLaunch where Hydra renders again.
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    let _ = render_app(&mut app, 120, 32);
    assert!(app.should_animate());

    // Open /config and walk all 11 settings to verify every upper character-art + guide inspector.
    type_text(&mut app, "/config");
    key(&mut app, KeyCode::Enter);
    let config_keys = [
        "screen_mode",
        "theme",
        "composer_mode",
        "live_preview",
        "mouse",
        "emblem_mode",
        "inline_height",
        "ascii",
        "no_color",
        "language",
        "tmux_popup",
    ];
    for (idx, cfg_key) in config_keys.iter().enumerate() {
        let cfg_screen = render_app(&mut app, 120, 32).backend().to_string();
        assert!(
            cfg_screen.contains(&format!("JustMux Configuration · {cfg_key}")),
            "missing inspector header on row {idx} ({cfg_key}):\n{cfg_screen}"
        );
        if idx + 1 < config_keys.len() {
            key(&mut app, KeyCode::Down);
        }
    }

    // Move up 5 rows from Row 10 (tmux_popup) to Row 5 (emblem_mode) and cycle through all 3 modes:
    // Spin -> InvertedSpin -> Static.
    for _ in 0..5 {
        key(&mut app, KeyCode::Up);
    }
    let row5_spin = render_app(&mut app, 120, 32).backend().to_string();
    assert!(row5_spin.contains("static"), "{row5_spin}");
    assert!(row5_spin.contains("spin"), "{row5_spin}");
    assert!(row5_spin.contains("inverted-spin"), "{row5_spin}");
    assert!(row5_spin.contains("(2/3)"), "{row5_spin}");

    let ring_row_subview = hydra_top_ring_row(32, 13, &app.theme());
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.config().emblem_mode, EmblemMode::InvertedSpin);
    assert!(app.should_animate());
    let inv_subview_a = render_app(&mut app, 120, 32).backend().to_string();
    assert!(inv_subview_a.contains(&ring_row_subview), "{inv_subview_a}");
    assert!(inv_subview_a.contains("(3/3)"), "{inv_subview_a}");
    for _ in 0..9 {
        app.tick_animation();
    }
    let inv_subview_b = render_app(&mut app, 120, 32).backend().to_string();
    assert!(inv_subview_b.contains(&ring_row_subview), "{inv_subview_b}");
    assert_ne!(inv_subview_a, inv_subview_b);

    // Cycle from InvertedSpin -> Static: stops animation and resets emblem_step to 0, then save.
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.config().emblem_mode, EmblemMode::Static);
    assert!(!app.should_animate());
    assert_eq!(app.emblem_step(), 0);
    key(&mut app, KeyCode::Char('s'));
    assert!(app.status().contains("Saved config to"));
    let saved = config::load(&CliOverrides {
        config_path: Some(cfg_path),
        screen_mode: None,
        theme: None,
    })
    .expect("reload saved config");
    assert_eq!(saved.config.emblem_mode, EmblemMode::Static);

    key(&mut app, KeyCode::Esc);
    let static_home = render_app(&mut app, 120, 32).backend().to_string();
    assert!(static_home.contains(&ring_row_on), "{static_home}");
    assert!(!app.should_animate());
}

#[test]
fn window_preview_scrolls_and_pages_through_tmux_scrollback_and_resets_on_switch() {
    let server = PrivateServer::new();
    let out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            "80",
            "-y",
            "12",
            "-s",
            "scroll-test",
            "sh -c 'i=1; while [ $i -le 60 ]; do printf \"HIST_%02d\\n\" \"$i\"; i=$((i+1)); done; sleep 30'",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("spawn scroll-test session");
    assert!(out.status.success());

    let sid = server
        .client
        .list_sessions()
        .expect("list sessions")
        .into_iter()
        .find(|s| s.name == "scroll-test")
        .expect("scroll-test session")
        .id;
    wait_for_pane_text(&server.client, &Target::Session(sid.clone()), "HIST_60");

    let _other_win = server
        .client
        .new_window(&sid, Some("other"), None)
        .expect("second window");

    let mut app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );

    // Initial Home render shows bottom of scrollback (HIST_60) and full line count badge (↕ 60L).
    let init = render_app(&mut app, 120, 32).backend().to_string();
    assert!(init.contains("HIST_60"), "{init}");
    assert!(init.contains("↕ 60L"), "{init}");
    assert!(!init.contains("HIST_01"), "{init}");

    // PageUp pages backward into scrollback history; PageDown returns toward the bottom.
    key(&mut app, KeyCode::PageUp);
    let after_pgup = render_app(&mut app, 120, 32).backend().to_string();
    assert!(after_pgup.contains("↑ +"), "{after_pgup}");
    assert!(!after_pgup.contains("HIST_60"), "{after_pgup}");

    key(&mut app, KeyCode::PageDown);
    let after_pgdn = render_app(&mut app, 120, 32).backend().to_string();
    assert!(after_pgdn.contains("HIST_60"), "{after_pgdn}");
    assert!(after_pgdn.contains("↕ 60L"), "{after_pgdn}");

    // Empty-bar Home/End jump to the top (HIST_01) and back to the bottom (HIST_60).
    key(&mut app, KeyCode::Home);
    let at_top = render_app(&mut app, 120, 32).backend().to_string();
    assert!(at_top.contains("HIST_01"), "{at_top}");
    assert!(!at_top.contains("HIST_60"), "{at_top}");

    key(&mut app, KeyCode::End);
    let at_bottom = render_app(&mut app, 120, 32).backend().to_string();
    assert!(at_bottom.contains("HIST_60"), "{at_bottom}");
    assert!(at_bottom.contains("↕ 60L"), "{at_bottom}");

    // Empty-bar Ctrl+U / Ctrl+D half-page up and down through scrollback.
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    let after_ctrl_u = render_app(&mut app, 120, 32).backend().to_string();
    assert!(after_ctrl_u.contains("↑ +"), "{after_ctrl_u}");
    assert!(!after_ctrl_u.contains("HIST_60"), "{after_ctrl_u}");

    key_mod(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let after_ctrl_d = render_app(&mut app, 120, 32).backend().to_string();
    assert!(after_ctrl_d.contains("HIST_60"), "{after_ctrl_d}");
    assert!(after_ctrl_d.contains("↕ 60L"), "{after_ctrl_d}");

    // Non-empty Ctrl+U clears the composer instead of scrolling Window Preview.
    type_text(&mut app, "scrollquery");
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    let after_clear = render_app(&mut app, 120, 32).backend().to_string();
    assert!(!after_clear.contains("scrollquery"), "{after_clear}");
    assert!(after_clear.contains("HIST_60"), "{after_clear}");
    assert!(after_clear.contains("↕ 60L"), "{after_clear}");

    // Mouse wheel inside Window Preview scrolls scrollback in 3-line steps.
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 4));
    let wheel_up = render_app(&mut app, 120, 32).backend().to_string();
    assert!(wheel_up.contains("↑ +3/"), "{wheel_up}");
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 4));
    let wheel_down = render_app(&mut app, 120, 32).backend().to_string();
    assert!(wheel_down.contains("↕ 60L"), "{wheel_down}");

    // Switching windows resets preview scroll to 0 (bottom of the newly selected window).
    key(&mut app, KeyCode::Home);
    assert!(
        render_app(&mut app, 120, 32)
            .backend()
            .to_string()
            .contains("HIST_01")
    );
    key(&mut app, KeyCode::Down);
    let on_other = render_app(&mut app, 120, 32).backend().to_string();
    assert!(on_other.contains("1: other"), "{on_other}");
    assert!(!on_other.contains("HIST_01"), "{on_other}");
    assert!(!on_other.contains("↑ +"), "{on_other}");

    key(&mut app, KeyCode::Up);
    let back_to_hist = render_app(&mut app, 120, 32).backend().to_string();
    assert!(back_to_hist.contains("HIST_60"), "{back_to_hist}");
    assert!(back_to_hist.contains("↕ 60L"), "{back_to_hist}");
    assert!(!back_to_hist.contains("HIST_01"), "{back_to_hist}");
}

fn find_buffer_text(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    let chars: Vec<String> = needle.chars().map(|ch| ch.to_string()).collect();
    if chars.is_empty() || buf.area.width < chars.len() as u16 {
        return None;
    }
    for y in 0..buf.area.height {
        let max_x = buf.area.width - chars.len() as u16;
        for x in 0..=max_x {
            if chars
                .iter()
                .enumerate()
                .all(|(i, expected)| buf[(x + i as u16, y)].symbol() == expected)
            {
                return Some((x, y));
            }
        }
    }
    None
}

#[test]
fn window_preview_renders_ansi_256_and_truecolor_pane_output() {
    let server = PrivateServer::new();
    let out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            "100",
            "-y",
            "12",
            "-s",
            "sgr-preview",
            "sh -c 'printf \"\\033[1;32mANSI_GREEN\\033[0m \\033[38;5;208mIDX_ORANGE\\033[0m \\033[38;2;120;180;255mRGB_SKY\\033[0m\\n\"; sleep 30'",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("spawn sgr-preview session");
    assert!(out.status.success());

    let sid = server
        .client
        .list_sessions()
        .expect("list sessions")
        .into_iter()
        .find(|s| s.name == "sgr-preview")
        .expect("sgr-preview session")
        .id;
    wait_for_pane_text(&server.client, &Target::Session(sid), "RGB_SKY");

    let mut app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );
    let term = render_app(&mut app, 120, 32);
    let buf = term.backend().buffer();

    let (green_col, green_row) = find_buffer_text(buf, "ANSI_GREEN").expect("ANSI_GREEN cell");
    for offset in 0.."ANSI_GREEN".len() {
        let cell = &buf[(green_col + offset as u16, green_row)];
        assert_eq!(cell.fg, Color::Green, "green offset {offset}");
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "bold offset {offset}"
        );
    }

    let (orange_col, orange_row) = find_buffer_text(buf, "IDX_ORANGE").expect("IDX_ORANGE cell");
    for offset in 0.."IDX_ORANGE".len() {
        let cell = &buf[(orange_col + offset as u16, orange_row)];
        assert_eq!(cell.fg, Color::Indexed(208), "orange offset {offset}");
    }

    let (rgb_col, rgb_row) = find_buffer_text(buf, "RGB_SKY").expect("RGB_SKY cell");
    for offset in 0.."RGB_SKY".len() {
        let cell = &buf[(rgb_col + offset as u16, rgb_row)];
        assert_eq!(cell.fg, Color::Rgb(120, 180, 255), "rgb offset {offset}");
    }

    let mut no_color_app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config {
            no_color: true,
            ..Config::default()
        },
        None,
    );
    let no_color_term = render_app(&mut no_color_app, 120, 32);
    let rendered = no_color_term.backend().to_string();
    assert!(rendered.contains("RGB_SKY"), "{rendered}");
    for cell in no_color_term.backend().buffer().content() {
        assert_eq!(cell.fg, Color::Reset, "fg reset on {cell:?}");
        assert_eq!(cell.bg, Color::Reset, "bg reset on {cell:?}");
    }
}

#[test]
fn split_studio_interactive_2x2_mouse_and_apply_layout() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let session = server
        .client
        .new_session("studio-e2e", cwd)
        .expect("session");
    let w_editor = server
        .client
        .list_windows(Some(&session.id))
        .expect("windows")
        .into_iter()
        .next()
        .expect("editor window");
    server
        .client
        .rename_window(&w_editor.id, "editor")
        .expect("rename editor");
    let w_server = server
        .client
        .new_window(&session.id, Some("server"), Some(cwd))
        .expect("server window");
    let w_logs = server
        .client
        .new_window(&session.id, Some("logs"), Some(cwd))
        .expect("logs window");

    for (wid, a, b) in [
        (&w_editor.id, "__S_", "EDIT__"),
        (&w_server.id, "__S_", "SERV__"),
        (&w_logs.id, "__S_", "LOGS__"),
    ] {
        let cmd = format!("printf '{a}{b}\\n'");
        let out = Command::new("tmux")
            .args([
                "-L",
                &server.socket,
                "send-keys",
                "-t",
                wid.as_str(),
                &cmd,
                "Enter",
            ])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("send-keys");
        assert!(out.status.success());
        let needle = format!("{a}{b}");
        wait_for_pane_text(
            &server.client,
            &Target::Window(session.id.clone(), wid.clone()),
            &needle,
        );
    }

    let p_editor = server
        .client
        .list_panes(Some(&w_editor.id))
        .expect("editor pane")[0]
        .id
        .clone();
    let p_server = server
        .client
        .list_panes(Some(&w_server.id))
        .expect("server pane")[0]
        .id
        .clone();
    let p_logs = server
        .client
        .list_panes(Some(&w_logs.id))
        .expect("logs pane")[0]
        .id
        .clone();

    // Cancelling after staging splits, flip, and reset leaves tmux untouched.
    let mut cancel_app = App::new(&server.client, AppOptions::default());
    type_text(&mut cancel_app, "/split");
    key(&mut cancel_app, KeyCode::Enter);
    key(&mut cancel_app, KeyCode::Enter);
    key(&mut cancel_app, KeyCode::Char('h'));
    key(&mut cancel_app, KeyCode::Char('v'));
    let color_theme = Theme {
        no_color: false,
        ..cancel_app.theme()
    };
    key(&mut cancel_app, KeyCode::Char('t'));
    assert!(
        cancel_app.status().contains("Flipped"),
        "{}",
        cancel_app.status()
    );
    assert_eq!(
        justmux::ui::status_style(cancel_app.status(), color_theme),
        color_theme.style(color_theme.accent)
    );
    assert_ne!(
        justmux::ui::status_style(cancel_app.status(), color_theme),
        color_theme.style(color_theme.error)
    );
    key(&mut cancel_app, KeyCode::Char('r'));
    assert!(
        cancel_app.status().contains("Reset"),
        "{}",
        cancel_app.status()
    );
    assert_eq!(
        justmux::ui::status_style(cancel_app.status(), color_theme),
        color_theme.style(color_theme.accent)
    );
    assert_ne!(
        justmux::ui::status_style(cancel_app.status(), color_theme),
        color_theme.style(color_theme.error)
    );
    let reset_screen = render_app(&mut cancel_app, 120, 32).backend().to_string();
    assert!(reset_screen.contains("#1 ★"), "{reset_screen}");
    assert!(!reset_screen.contains("#2"), "{reset_screen}");
    key(&mut cancel_app, KeyCode::Esc);
    assert!(cancel_app.exit().is_none());
    assert_eq!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("unchanged windows")
            .len(),
        3
    );
    assert_eq!(
        server
            .client
            .list_panes(None)
            .expect("unchanged panes")
            .len(),
        3
    );

    // Interactive 2x2 studio via mouse + keyboard, live previews, ZH localization, and Apply & Enter.
    let mut app = App::new(&server.client, AppOptions::default());
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);

    let t1 = render_app(&mut app, 120, 32);
    let s1 = t1.backend().to_string();
    assert!(s1.contains("#1 ★"), "{s1}");
    assert!(
        s1.contains("__S_EDIT__") || s1.contains("__S_SERV__") || s1.contains("__S_LOGS__"),
        "expected initial live preview in canvas:\n{s1}"
    );

    // Click `[h] Split H` toolbar button.
    let (h_col, h_row) = find_buffer_text(t1.backend().buffer(), "[h]").expect("[h] button");
    app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        h_col,
        h_row,
    ));
    let t2 = render_app(&mut app, 120, 32);
    let s2 = t2.backend().to_string();
    assert!(s2.contains("#1"), "{s2}");
    assert!(s2.contains("#2 ★"), "{s2}");

    // Click `[v] Split V` on right pane -> 3 panes, then focus #1 and press `v` -> 2x2 (4 panes).
    let (v_col, v_row) = find_buffer_text(t2.backend().buffer(), "[v]").expect("[v] button");
    app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        v_col,
        v_row,
    ));
    key(&mut app, KeyCode::Char('1'));
    key(&mut app, KeyCode::Char('v'));
    let s4 = render_app(&mut app, 120, 32).backend().to_string();
    for tag in ["#1", "#2 ★", "#3", "#4"] {
        assert!(s4.contains(tag), "missing {tag} in 2x2:\n{s4}");
    }

    // One-key flip (`t`) transposes split axes and flips back cleanly.
    key(&mut app, KeyCode::Char('t'));
    assert!(app.status().contains("Flipped"), "{}", app.status());
    key(&mut app, KeyCode::Char('t'));
    assert!(app.status().contains("Flipped"), "{}", app.status());

    // Attempting a 5th split is refused with a friendly 2x2 cap status.
    key(&mut app, KeyCode::Char('h'));
    let s5 = render_app(&mut app, 120, 32).backend().to_string();
    assert!(!s5.contains("#5"), "{s5}");
    assert!(app.status().contains("Max 2x2"), "{}", app.status());

    // Fill #2 and #3 with the remaining free windows; keep #4 as `+ New shell`.
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Char('3'));
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);

    // Extra preview scroll keys (Home/End/Ctrl+U/Ctrl+D) operate safely inside Split Studio.
    key(&mut app, KeyCode::Home);
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('d'),
        crossterm::event::KeyModifiers::CONTROL,
    )));
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        crossterm::event::KeyModifiers::CONTROL,
    )));
    key(&mut app, KeyCode::End);

    // Focus #2 explicitly so Apply should leave #2's pane active (distinct from #1).
    key(&mut app, KeyCode::Char('2'));
    let filled = render_app(&mut app, 120, 32).backend().to_string();
    assert!(filled.contains("#2 ★ studio-e2e:2 logs"), "{filled}");
    assert!(filled.contains("Enter Launch"), "{filled}");
    assert!(filled.contains("__S_EDIT__"), "{filled}");
    assert!(filled.contains("__S_SERV__"), "{filled}");
    assert!(filled.contains("__S_LOGS__"), "{filled}");
    assert!(filled.contains("+ New shell"), "{filled}");

    // Switching language to Chinese updates Split Studio chrome & toolbar pills cleanly.
    let saved_theme = app.theme();
    app.set_theme(Theme {
        language: justmux::ui::Language::Zh,
        ..saved_theme
    });
    let zh_screen = render_app(&mut app, 120, 32).backend().to_string();
    assert!(zh_screen.contains("左右分"), "{zh_screen}");
    assert!(zh_screen.contains("上下分"), "{zh_screen}");
    assert!(zh_screen.contains("翻转"), "{zh_screen}");
    assert!(zh_screen.contains("确认进入 tmux"), "{zh_screen}");
    app.set_theme(saved_theme);

    // Click `[Enter] Apply & Enter` pill to materialize the 2x2 layout and enter tmux.
    let final_term = render_app(&mut app, 120, 32);
    let (apply_col, apply_row) =
        find_buffer_text(final_term.backend().buffer(), "[Enter] Apply & Enter")
            .expect("[Enter] Apply & Enter button");
    app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        apply_col,
        apply_row,
    ));

    let target_window = match app.exit() {
        Some(Exit::Attach(Target::Window(sid, wid))) if sid == &session.id => wid.clone(),
        other => panic!("expected Exit::Attach(Target::Window) for studio session, got {other:?}"),
    };
    let merged_panes = server
        .client
        .list_panes(Some(&target_window))
        .expect("merged panes");
    assert_eq!(merged_panes.len(), 4, "expected 4 panes: {merged_panes:?}");

    let active_pane = merged_panes
        .iter()
        .find(|p| p.active)
        .expect("active pane after Apply");
    assert_ne!(
        active_pane.id, p_editor,
        "selected slot #2 should be focused instead of #1 (editor)"
    );
    assert_ne!(
        active_pane.id, p_server,
        "selected slot #2 (logs) should be focused instead of #3 (server)"
    );
    assert_eq!(
        active_pane.id, p_logs,
        "expected selected slot #2 pane ({p_logs:?}) to be active: {merged_panes:?}"
    );

    let mut combined_output = String::new();
    for pane in &merged_panes {
        let lines = server
            .client
            .capture_pane(&Target::Pane(pane.id.clone()), 40)
            .expect("capture merged pane");
        combined_output.push_str(&lines.join("\n"));
        combined_output.push('\n');
    }
    assert!(combined_output.contains("__S_EDIT__"), "{combined_output}");
    assert!(combined_output.contains("__S_SERV__"), "{combined_output}");
    assert!(combined_output.contains("__S_LOGS__"), "{combined_output}");
}

#[test]
fn split_studio_multi_pane_label_and_vanished_window_recovery() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let session = server
        .client
        .new_session("studio-recover", cwd)
        .expect("session");
    let w_main = server
        .client
        .list_windows(Some(&session.id))
        .expect("windows")
        .into_iter()
        .next()
        .expect("main window");
    server
        .client
        .rename_window(&w_main.id, "main")
        .expect("rename");
    let w_vanish = server
        .client
        .new_window(&session.id, Some("worker"), Some(cwd))
        .expect("worker");
    let vanish_pane = server
        .client
        .list_panes(Some(&w_vanish.id))
        .expect("worker panes")
        .into_iter()
        .next()
        .expect("worker pane");
    server
        .client
        .split_pane(&vanish_pane.id, justmux::tmux::SplitDirection::SideBySide)
        .expect("split worker window");

    // Re-focus main so initial slot #1 holds main and worker sits at row 1.
    server
        .client
        .select(&Target::Window(session.id.clone(), w_main.id.clone()))
        .expect("select main");

    let mut app = App::new(&server.client, AppOptions::default());
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);

    let en_screen = render_app(&mut app, 120, 32).backend().to_string();
    assert!(en_screen.contains("2p · active pane"), "{en_screen}");

    let saved = app.theme();
    app.set_theme(Theme {
        language: justmux::ui::Language::Zh,
        ..saved
    });
    let zh_screen = render_app(&mut app, 120, 32).backend().to_string();
    assert!(zh_screen.contains("2p · 仅迁活跃窗格"), "{zh_screen}");
    app.set_theme(saved);

    // Stage #1 = main, #2 = worker, then destroy worker externally before Apply.
    key(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Up);
    let staged = render_app(&mut app, 120, 32).backend().to_string();
    assert!(staged.contains("[ #2 active ]"), "{staged}");

    let kill_out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "kill-window",
            "-t",
            w_vanish.id.as_str(),
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("kill worker window");
    assert!(kill_out.status.success());

    key(&mut app, KeyCode::Enter);
    assert!(
        app.exit().is_none(),
        "should recover cleanly in Split Studio"
    );
    assert!(
        !app.status().is_empty(),
        "expected recovery status explaining vanished window"
    );

    let recovered = render_app(&mut app, 120, 32).backend().to_string();
    assert!(
        !recovered.contains("worker"),
        "vanished window should be pruned from options:\n{recovered}"
    );
    assert!(recovered.contains("#2 ★"), "{recovered}");
    assert!(recovered.contains("+ New shell"), "{recovered}");
}

#[test]
fn split_studio_lazy_preview_captures_beyond_initial_24_windows() {
    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let session = server
        .client
        .new_session("studio-lazy", cwd)
        .expect("session");
    for i in 1..=25 {
        server
            .client
            .new_window(&session.id, Some(&format!("w{i:02}")), Some(cwd))
            .expect("create extra window");
    }
    let windows = server
        .client
        .list_windows(Some(&session.id))
        .expect("all windows");
    assert_eq!(windows.len(), 26);

    // Ensure window at index 0 is active so w25 stays at index 25 (>= 24 eager cap).
    let w0 = &windows[0];
    let w25 = &windows[25];
    server
        .client
        .select(&Target::Window(session.id.clone(), w0.id.clone()))
        .expect("select w0");

    let out = Command::new("tmux")
        .args([
            "-L",
            &server.socket,
            "send-keys",
            "-t",
            w25.id.as_str(),
            "printf '__LAZY_25_' && printf 'DONE__\\n'",
            "Enter",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("send-keys to w25");
    assert!(out.status.success());
    wait_for_pane_text(
        &server.client,
        &Target::Window(session.id.clone(), w25.id.clone()),
        "__LAZY_25_DONE__",
    );

    let mut app = App::new(&server.client, AppOptions::default());
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);

    // Split to #2 (+ New shell, cursor at row 26), then step Up onto row 25 (idx 25 >= 24).
    key(&mut app, KeyCode::Char('h'));
    key(&mut app, KeyCode::Up);

    let screen = render_app(&mut app, 120, 32).backend().to_string();
    assert!(
        screen.contains("__LAZY_25_DONE__"),
        "expected lazy capture for window beyond index 24:\n{screen}"
    );
}

#[test]
fn config_tmux_popup_manages_tmux_conf_and_live_private_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("config.toml");
    let tmux_conf_path = dir.path().join(".tmux.conf");
    let initial_conf = "set -g mouse on\nset -g history-limit 5000\n";
    std::fs::write(&tmux_conf_path, initial_conf).expect("seed temp tmux.conf");

    let server = PrivateServer::new();
    let _session = server
        .client
        .new_session("agy-work", Path::new(env!("CARGO_MANIFEST_DIR")))
        .expect("start private server");

    let mut app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        Some(cfg_path.clone()),
    )
    .with_tmux_conf(
        Some(tmux_conf_path.clone()),
        Some(PathBuf::from("/usr/local/bin/jm")),
    );
    assert!(!app.config().tmux_popup);

    // Navigate to Row 10 (tmux_popup) in /config and toggle ON.
    type_text(&mut app, "/config");
    key(&mut app, KeyCode::Enter);
    for _ in 0..10 {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Char(' '));

    assert!(app.config().tmux_popup);
    assert!(
        app.status().contains("Ctrl+b j"),
        "expected Ctrl+b j feedback in status: {}",
        app.status()
    );
    let installed_conf =
        std::fs::read_to_string(&tmux_conf_path).expect("read installed tmux.conf");
    assert!(installed_conf.starts_with(initial_conf), "{installed_conf}");
    assert!(installed_conf.contains(justmux::tmux_conf::MANAGED_BLOCK_START));
    assert!(installed_conf.contains("bind-key j run-shell"));
    assert!(installed_conf.contains("JUSTMUX_POPUP=1"));
    assert!(installed_conf.contains(justmux::tmux_conf::MANAGED_BLOCK_END));

    // Live private tmux server should now have `prefix j` bound.
    let live_keys = server.tmux(&["list-keys", "-T", "prefix", "j"]);
    assert!(
        live_keys.status.success(),
        "expected prefix+j bound on live server: {}",
        String::from_utf8_lossy(&live_keys.stderr)
    );
    let live_stdout = String::from_utf8_lossy(&live_keys.stdout);
    assert!(live_stdout.contains("display-popup -E"), "{live_stdout}");
    assert!(live_stdout.contains("JUSTMUX_POPUP=1"), "{live_stdout}");

    // A fresh tmux server started with the generated tmux.conf must parse it cleanly.
    let check_socket = format!("jm-conf-check-{}", std::process::id());
    struct CheckServerCleanup(String);
    impl Drop for CheckServerCleanup {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .args(["-L", &self.0, "kill-server"])
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output();
        }
    }
    let _cleanup = CheckServerCleanup(check_socket.clone());
    let booted = Command::new("tmux")
        .arg("-L")
        .arg(&check_socket)
        .arg("-f")
        .arg(&tmux_conf_path)
        .args(["new-session", "-d", "-s", "boot-ok", "sleep 10"])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("boot fresh tmux server with generated conf");
    assert!(
        booted.status.success(),
        "tmux failed to load generated conf: {}",
        String::from_utf8_lossy(&booted.stderr)
    );
    let fresh_keys = Command::new("tmux")
        .args(["-L", &check_socket, "list-keys", "-T", "prefix", "j"])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("list-keys on booted server");
    assert!(fresh_keys.status.success());
    assert!(String::from_utf8_lossy(&fresh_keys.stdout).contains("JUSTMUX_POPUP=1"));

    // Save config.toml while enabled and verify round-trip + tmux_conf auto-detection.
    key(&mut app, KeyCode::Char('s'));
    let saved_on = config::load(&CliOverrides {
        config_path: Some(cfg_path.clone()),
        screen_mode: None,
        theme: None,
    })
    .expect("reload saved config");
    assert!(saved_on.config.tmux_popup);

    let detected_app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    )
    .with_tmux_conf(Some(tmux_conf_path.clone()), None);
    assert!(detected_app.config().tmux_popup);

    // Toggle OFF and verify clean file restoration + live unbind.
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.config().tmux_popup);
    let restored_conf = std::fs::read_to_string(&tmux_conf_path).expect("read restored tmux.conf");
    assert_eq!(restored_conf, initial_conf);
    let live_after_unbind = server.tmux(&["list-keys", "-T", "prefix", "j"]);
    assert!(
        !live_after_unbind.status.success(),
        "expected prefix+j unbound on live server: {}",
        String::from_utf8_lossy(&live_after_unbind.stdout)
    );

    // Re-enable, replace live `j` with a custom non-JustMux binding, then toggle OFF:
    // file block is removed, but user's custom live `j` binding is preserved!
    key(&mut app, KeyCode::Char(' '));
    assert!(app.config().tmux_popup);
    let custom_bind = server.tmux(&[
        "bind-key",
        "-T",
        "prefix",
        "j",
        "display-message",
        "custom-user-bind",
    ]);
    assert!(custom_bind.status.success());
    key(&mut app, KeyCode::Char(' '));
    assert!(!app.config().tmux_popup);
    assert_eq!(
        std::fs::read_to_string(&tmux_conf_path).expect("read after custom"),
        initial_conf
    );
    let custom_keys = server.tmux(&["list-keys", "-T", "prefix", "j"]);
    assert!(custom_keys.status.success());
    assert!(String::from_utf8_lossy(&custom_keys.stdout).contains("custom-user-bind"));
}

#[test]
fn circular_wrap_around_navigation_for_home_palette_mentions_radar_theme_config_and_split() {
    let highlighted_mention_line = |term: &Terminal<TestBackend>| -> Option<String> {
        let buf = term.backend().buffer();
        let area = buf.area;
        for y in 0..area.height {
            let line: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
            let trimmed = line.trim_start();
            if trimmed.starts_with('@')
                && (0..area.width).any(|x| {
                    let cell = &buf[(x, y)];
                    cell.symbol() == "@"
                        && (cell.bg != Color::Reset || cell.modifier.contains(Modifier::REVERSED))
                })
            {
                return Some(trimmed.split_whitespace().next().unwrap_or("").to_owned());
            }
        }
        None
    };

    let server = PrivateServer::new();
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        server
            .tmux(&[
                "new-session",
                "-d",
                "-s",
                "alpha",
                "-n",
                "main",
                "-c",
                cwd.to_str().expect("cwd"),
                "sh -c 'sleep 30'",
            ])
            .status
            .success()
    );
    assert!(
        server
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                "alpha",
                "-n",
                "logs",
                "-c",
                cwd.to_str().expect("cwd"),
                "sh -c 'sleep 30'",
            ])
            .status
            .success()
    );
    assert!(
        server
            .tmux(&[
                "new-session",
                "-d",
                "-s",
                "beta",
                "-n",
                "core",
                "-c",
                cwd.to_str().expect("cwd"),
                "sh -c 'sleep 30'",
            ])
            .status
            .success()
    );
    assert!(
        server
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                "beta",
                "-n",
                "api",
                "-c",
                cwd.to_str().expect("cwd"),
                "sh -c 'sleep 30'",
            ])
            .status
            .success()
    );

    let cfg = Config {
        inline_height: 8,
        ..Config::default()
    };
    let mut app = App::with_config(&server.client, AppOptions::default(), cfg, None);
    let _ = render_app(&mut app, 100, 26);

    // 1. Home Matrix column wrap: `Left` from col 0 wraps to `+ New session`, `Right` wraps back.
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Enter);
    let new_sess_screen = render_app(&mut app, 100, 26).backend().to_string();
    assert!(
        new_sess_screen.contains("New session"),
        "Left from col 0 should wrap to + New session:\n{new_sess_screen}"
    );
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Right);
    let back_col0 = render_app(&mut app, 100, 26).backend().to_string();
    assert!(back_col0.contains("alpha"), "{back_col0}");

    // Home Matrix row wrap: from Window(0), `Up` -> SessionHeader, `Up` -> wraps to `+ New window`.
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Enter);
    let new_win_screen = render_app(&mut app, 100, 26).backend().to_string();
    assert!(
        new_win_screen.contains("New window"),
        "Up from SessionHeader should wrap to + New window:\n{new_win_screen}"
    );
    key(&mut app, KeyCode::Esc);
    // Mouse wheel down from `+ New window` wraps to SessionHeader, then back into `+ New window`.
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 18));
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 18));
    key(&mut app, KeyCode::Enter);
    let wheel_wrapped = render_app(&mut app, 100, 26).backend().to_string();
    assert!(
        wheel_wrapped.contains("New window"),
        "Mouse wheel Up from SessionHeader should wrap back to + New window:\n{wheel_wrapped}"
    );
    key(&mut app, KeyCode::Esc);

    // 2. `/` palette wrap with keyboard & mouse wheel.
    type_text(&mut app, "/");
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 10));
    let pal_up = render_app(&mut app, 100, 18).backend().to_string();
    assert!(pal_up.contains("/quit"), "{pal_up}");
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 10));
    let pal_down = render_app(&mut app, 100, 18).backend().to_string();
    assert!(pal_down.contains("/resume"), "{pal_down}");
    key(&mut app, KeyCode::Esc);

    // 3. `@` mentions wrap with keyboard & mouse wheel (checking highlighted target row).
    type_text(&mut app, "@");
    let mentions_0 = highlighted_mention_line(&render_app(&mut app, 100, 24));
    key(&mut app, KeyCode::Up);
    let mentions_last = highlighted_mention_line(&render_app(&mut app, 100, 24));
    assert!(mentions_0.is_some() && mentions_last.is_some());
    assert_ne!(
        mentions_0, mentions_last,
        "Up from index 0 in @ mentions should wrap to last candidate"
    );
    key(&mut app, KeyCode::Down);
    assert_eq!(
        highlighted_mention_line(&render_app(&mut app, 100, 24)),
        mentions_0,
        "Down from last @ mention should wrap back to index 0"
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 10));
    assert_eq!(
        highlighted_mention_line(&render_app(&mut app, 100, 24)),
        mentions_last
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 10));
    assert_eq!(
        highlighted_mention_line(&render_app(&mut app, 100, 24)),
        mentions_0
    );
    key(&mut app, KeyCode::Esc);

    // 4. Live Radar (`Shift+Tab`) wraps across sessions on keyboard & mouse wheel.
    key(&mut app, KeyCode::BackTab);
    let radar_0 = render_app(&mut app, 100, 24).backend().to_string();
    key(&mut app, KeyCode::Up);
    let radar_1 = render_app(&mut app, 100, 24).backend().to_string();
    assert_ne!(
        radar_0, radar_1,
        "Up in Live Radar should wrap to last session"
    );
    key(&mut app, KeyCode::Down);
    assert_eq!(render_app(&mut app, 100, 24).backend().to_string(), radar_0);
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 12));
    assert_eq!(render_app(&mut app, 100, 24).backend().to_string(), radar_1);
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 12));
    assert_eq!(render_app(&mut app, 100, 24).backend().to_string(), radar_0);
    // LiveRadar -> QuickLaunch -> CommandJump
    key(&mut app, KeyCode::BackTab);
    key(&mut app, KeyCode::BackTab);

    // 5. `/theme` picker wraps from first preset <-> last preset via keys & mouse wheel.
    app.set_theme(Theme::default().with_preset(ThemePreset::AntigravityAurora));
    type_text(&mut app, "/theme");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    key(&mut app, KeyCode::Up);
    assert_eq!(
        app.theme().preset,
        *ThemePreset::ALL.last().expect("last theme")
    );
    key(&mut app, KeyCode::Down);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 10));
    assert_eq!(
        app.theme().preset,
        *ThemePreset::ALL.last().expect("last theme")
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 10));
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    key(&mut app, KeyCode::Esc);

    // 6. `/config` list wraps (`Up`/`Down` + wheel) AND `inline_height` wraps `8 <-> 40`.
    type_text(&mut app, "/config");
    key(&mut app, KeyCode::Enter);
    let cfg_top = render_app(&mut app, 108, 28).backend().to_string();
    key(&mut app, KeyCode::Up);
    let cfg_bottom = render_app(&mut app, 108, 28).backend().to_string();
    assert!(
        cfg_bottom.contains("live_stream") || cfg_bottom.contains("tmux_reload"),
        "Up from row 0 in /config should wrap to last item:\n{cfg_bottom}"
    );
    key(&mut app, KeyCode::Down);
    assert_eq!(render_app(&mut app, 108, 28).backend().to_string(), cfg_top);
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 10, 10));
    assert_eq!(
        render_app(&mut app, 108, 28).backend().to_string(),
        cfg_bottom
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 10, 10));
    assert_eq!(render_app(&mut app, 108, 28).backend().to_string(), cfg_top);

    // Move to row 6 (`inline_height`), verify `Left` wraps `8 -> 40` and `Right` wraps `40 -> 8`.
    for _ in 0..6 {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(app.config().inline_height, 8);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.config().inline_height, 40);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.config().inline_height, 8);
    key(&mut app, KeyCode::Esc);

    // 7. `/split` Studio window list wraps on `Up`/`Down` and mouse wheel.
    // Move off `+ New window` onto `0* main` (`Down` -> SessionHeader, `Down` -> `0* main`),
    // open `/split` mode and press `Enter` on `0* main` to enter Split Studio.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('h'));
    let split_bottom = render_app(&mut app, 112, 28).backend().to_string();
    assert!(split_bottom.contains("Fill Pane #2"), "{split_bottom}");
    assert!(split_bottom.contains("› + + New shell"), "{split_bottom}");
    // Cursor starts on last row (`+ New shell`): `Down` wraps to row 0 (`alpha:0 main`), `Up` wraps back.
    key(&mut app, KeyCode::Down);
    let split_top = render_app(&mut app, 112, 28).backend().to_string();
    assert!(
        split_top.contains("› ● alpha:0 main"),
        "Down from last row in Split Studio window list should wrap to row 0:\n{split_top}"
    );
    key(&mut app, KeyCode::Up);
    assert_eq!(
        render_app(&mut app, 112, 28).backend().to_string(),
        split_bottom
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollDown, 5, 18));
    assert_eq!(
        render_app(&mut app, 112, 28).backend().to_string(),
        split_top
    );
    app.handle_event(mouse_event(MouseEventKind::ScrollUp, 5, 18));
    assert_eq!(
        render_app(&mut app, 112, 28).backend().to_string(),
        split_bottom
    );
    key(&mut app, KeyCode::Esc);
}
