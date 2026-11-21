#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
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
    ui::{ComposerMode, Theme, ThemePreset},
};
use ratatui::{Terminal, backend::TestBackend, style::Color};
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
    assert_eq!(commands::registry().len(), 12);
    assert_eq!(
        commands::registry().last().map(|cmd| cmd.kind),
        Some(CommandKind::Quit)
    );

    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/");
    for _ in 0..10 {
        key(&mut app, KeyCode::Down);
    }

    let terminal = render_app(&mut app, 100, 16);
    let output = terminal.backend().to_string();
    assert!(output.contains("/theme"), "{output}");
    assert!(output.contains("/quit"), "{output}");
}

#[test]
fn zero_learning_home_shortcuts_and_bare_command_forgiveness() {
    let tmux = offline_tmux();
    let mut app = App::new(&tmux, AppOptions::default());

    // Down on empty composer opens the `/` palette immediately.
    key(&mut app, KeyCode::Down);
    let output = render_app(&mut app, 80, 16).backend().to_string();
    assert!(output.contains("/resume"), "{output}");
    assert!(output.contains("/theme"), "{output}");
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
    assert_eq!(commands::filter_commands("", &mut matcher).len(), 12);
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
    assert_eq!(co_names.first().copied(), Some("theme"));
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

    // `/list` uses render_rich_picker with session badge and detached pill.
    type_text(&mut app, "/list");
    key(&mut app, KeyCode::Enter);
    let list_screen = render_app(&mut app, 90, 16).backend().to_string();
    assert!(list_screen.contains("codex-dev"), "{list_screen}");
    assert!(list_screen.contains("detached"), "{list_screen}");
    assert!(list_screen.contains(">_"), "{list_screen}");
    key(&mut app, KeyCode::Esc);

    // `/split` -> Enter opens View::Split with live visual wireframe preview.
    type_text(&mut app, "/split");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    let split_unicode = render_app(&mut app, 80, 16).backend().to_string();
    assert!(split_unicode.contains("╭──────┬──────╮"), "{split_unicode}");
    assert!(split_unicode.contains("╭─────────────╮"), "{split_unicode}");

    // ASCII fallback on `/split` stays pure ASCII.
    app.set_theme(Theme {
        ascii: true,
        no_color: true,
        ..app.theme()
    });
    let split_ascii = render_app(&mut app, 80, 16).backend().to_string();
    assert!(split_ascii.is_ascii(), "{split_ascii}");
    assert!(split_ascii.contains("+------+------+"), "{split_ascii}");
    assert!(split_ascii.contains("+-------------+"), "{split_ascii}");
}

fn wait_for_pane_text(client: &TmuxClient, target: &Target, needle: &str) {
    for _ in 0..80 {
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
    assert!(wide.contains("▗▛▀▜▖"), "{wide}");
    assert!(wide.contains("Live Workspaces (1-5)"), "{wide}");
    assert!(wide.contains("[1]"), "{wide}");
    assert!(wide.contains("Pro Tip"), "{wide}");
    assert!(wide.contains("Shift+Tab Mode"), "{wide}");

    let compact_studio = render_app(&mut app, 72, 16).backend().to_string();
    assert!(compact_studio.contains("◫ JustMux"), "{compact_studio}");
    assert!(compact_studio.contains("Try a command"), "{compact_studio}");
    assert!(compact_studio.contains("/help"), "{compact_studio}");
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

    // `/split @api-renamed` opens visual Split selector.
    type_text(&mut kill_app, "/split @api-renamed");
    key(&mut kill_app, KeyCode::Enter);
    let split_screen = render_app(&mut kill_app, 80, 16).backend().to_string();
    assert!(split_screen.contains("╭──────┬──────╮"), "{split_screen}");
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

    // Case-insensitive exact session name attaches directly.
    let mut jump_app = App::new(&server.client, AppOptions::default());
    type_text(&mut jump_app, "MY-APP");
    key(&mut jump_app, KeyCode::Enter);
    assert_eq!(jump_app.exit(), Some(&Exit::Attach(Target::Session(s.id))));
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
    assert!(!commands::registry().iter().any(|c| c.name == "config"));

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
        (beta_col, beta_row, _s2.id)
    } else {
        (alpha_col, alpha_row, s1.id)
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

    // Picker click + wheel test.
    let mut picker_app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        None,
    );
    type_text(&mut picker_app, "/list");
    key(&mut picker_app, KeyCode::Enter);
    let picker_term = render_app(&mut picker_app, 100, 24);
    let (p_col, p_row) =
        find_text_cell_in_cols(&picker_term, "beta", 45).expect("beta row in /list");
    picker_app.handle_event(mouse_event(
        MouseEventKind::Down(MouseButton::Left),
        p_col,
        p_row,
    ));
    let after_first = render_app(&mut picker_app, 100, 24).backend().to_string();
    if !after_first.contains("Session actions") {
        picker_app.handle_event(mouse_event(
            MouseEventKind::Down(MouseButton::Left),
            p_col,
            p_row,
        ));
    }
    let actions = render_app(&mut picker_app, 100, 24).backend().to_string();
    assert!(actions.contains("Session actions"));
}

#[test]
fn spacious_fullscreen_home_shows_active_preview_and_respects_live_preview_flag() {
    let server = PrivateServer::new();
    for name in ["s2", "s3", "s4", "s5", "s6"] {
        server
            .client
            .new_session(name, Path::new("/tmp"))
            .expect("extra session");
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

    // Wait briefly for printf inside the pane so capture_pane sees it reliably.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(lines) = server.client.capture_pane(
            &Target::Session(
                server
                    .client
                    .list_sessions()
                    .unwrap()
                    .into_iter()
                    .find(|s| s.name == "a-studio-one")
                    .unwrap()
                    .id,
            ),
            12,
        ) && lines.iter().any(|l| l.contains("JM_V3_SPACIOUS_PREVIEW"))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pane never printed preview sentinel"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // With live_preview = true on 120x32: shows 8 commands, 6 Live Workspaces, and embedded preview.
    let mut app_on = App::with_config(
        &server.client,
        AppOptions::default(),
        Config {
            live_preview: true,
            ..Config::default()
        },
        None,
    );
    let wide = render_app(&mut app_on, 120, 32).backend().to_string();
    assert!(wide.contains("/theme"));
    assert!(wide.contains("/help"));
    assert!(wide.contains("[6]"));
    assert!(wide.contains("Active Workspace Preview"));
    assert!(wide.contains("JM_V3_SPACIOUS_PREVIEW"));

    // With live_preview = false: skips preview both on Home and inside /list & /resume.
    let mut app_off = App::with_config(
        &server.client,
        AppOptions::default(),
        Config {
            live_preview: false,
            ..Config::default()
        },
        None,
    );
    let wide_off = render_app(&mut app_off, 120, 32).backend().to_string();
    assert!(!wide_off.contains("Active Workspace Preview"));
    assert!(!wide_off.contains("JM_V3_SPACIOUS_PREVIEW"));

    type_text(&mut app_off, "/list");
    key(&mut app_off, KeyCode::Enter);
    let list_off = render_app(&mut app_off, 100, 24).backend().to_string();
    assert!(!list_off.contains("JM_V3_SPACIOUS_PREVIEW"));
}
