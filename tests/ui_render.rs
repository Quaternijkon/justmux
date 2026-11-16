#![allow(clippy::expect_used, clippy::unwrap_used)]

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use justmux::{
    app::{App, AppOptions},
    tmux::{SocketSpec, TmuxClient},
    ui::{CommandItem, TextInput, Theme, render_form_scrolled, render_home, render_picker},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::{Backend, TestBackend},
    layout::Position,
    style::Modifier,
};

fn key(app: &mut App<'_>, code: KeyCode) {
    app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn type_text(app: &mut App<'_>, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

fn private_tmux() -> TmuxClient {
    TmuxClient::new(SocketSpec::Name(format!(
        "jm-ui-audit-{}",
        std::process::id()
    )))
    .with_config_file("/dev/null")
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    terminal.backend().to_string()
}

#[test]
fn home_and_command_popup_are_visible_in_test_backend() {
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let input = TextInput::new("/");
    let commands = [CommandItem {
        name: "resume",
        description: "Resume an existing tmux session",
    }];
    terminal
        .draw(|frame| {
            render_home(
                frame,
                frame.area(),
                &input,
                &commands,
                0,
                None,
                Theme::default(),
            )
        })
        .expect("draw home");
    let output = screen(&terminal);
    assert!(output.contains("JustMux"));
    assert!(output.contains("/resume"));
    assert!(output.contains("Resume an existing tmux session"));
}

#[test]
fn empty_picker_has_a_clear_empty_state() {
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| {
            render_picker(
                frame,
                frame.area(),
                "Resume session",
                &TextInput::default(),
                &[],
                0,
                "No tmux sessions",
                "Esc back",
                Theme::default(),
            )
        })
        .expect("draw picker");
    assert!(screen(&terminal).contains("No tmux sessions"));
}

#[test]
fn app_opens_palette_on_slash() {
    let socket = format!("jm-ui-{}", std::process::id());
    let tmux = TmuxClient::new(SocketSpec::Name(socket)).with_config_file("/dev/null");
    let mut app = App::new(&tmux, AppOptions::default());
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('/'),
        KeyModifiers::NONE,
    )));
    let mut terminal = Terminal::new(TestBackend::new(72, 18)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw app");
    let output = screen(&terminal);
    assert!(output.contains("/resume"));
    assert!(output.contains("Resume an existing tmux session"));
}

#[test]
fn enter_executes_highlighted_command_instead_of_valid_alias() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/r");
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    let mut terminal = Terminal::new(TestBackend::new(72, 18)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("Rename session"));
    assert!(!screen(&terminal).contains("Resume session"));
}

#[test]
fn canonical_command_for_an_alias_is_visible_even_without_fuzzy_match() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/mv");
    let mut terminal = Terminal::new(TestBackend::new(72, 18)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("/rename"));
    key(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("Rename session"));
}

#[test]
fn command_arguments_are_parsed_instead_of_replaced_by_completion() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/new 'my session'");
    key(&mut app, KeyCode::Enter);
    let mut terminal = Terminal::new(TestBackend::new(72, 18)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("Working directory"));
}

#[test]
fn narrow_home_scrolls_long_input_and_keeps_caret_visible() {
    let mut terminal = Terminal::new(TestBackend::new(20, 10)).expect("test terminal");
    terminal
        .draw(|frame| {
            render_home(
                frame,
                frame.area(),
                &TextInput::new("abcdefghijklmnopqrstuvwxyz"),
                &[],
                0,
                None,
                Theme::default(),
            )
        })
        .expect("draw");
    assert!(screen(&terminal).contains("z"));
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn short_palette_scrolls_to_selected_last_command() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/");
    for _ in 0..9 {
        key(&mut app, KeyCode::Down);
    }
    let mut terminal = Terminal::new(TestBackend::new(30, 8)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("/quit"));
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn inline_menu_preserves_output_above_a_small_viewport() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/");
    let mut backend = TestBackend::with_lines([
        "Previous shell output           ",
        "native scrollback stays here     ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
        "                                ",
    ]);
    backend
        .set_cursor_position(Position::new(0, 10))
        .expect("initial shell cursor");
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(6),
        },
    )
    .expect("inline terminal");
    terminal
        .draw(|frame| app.render(frame))
        .expect("draw inline");
    assert!(
        screen(&terminal).contains("/resume"),
        "first candidate must remain visible"
    );
    for _ in 0..9 {
        key(&mut app, KeyCode::Down);
    }
    terminal
        .draw(|frame| app.render(frame))
        .expect("draw selection");
    let buffer = terminal.backend().buffer();
    let row = |y| (0..32).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    assert!(row(0).starts_with("Previous shell output"));
    assert!(row(1).starts_with("native scrollback stays here"));
    assert!(!row(8).contains('/'), "menu must stay within its viewport");
    assert!(!row(9).contains('/'), "menu must not overwrite the shell");
    assert!(screen(&terminal).contains("/quit"));
}

#[test]
fn danger_description_scrolls_while_confirmation_input_stays_visible() {
    let message = "Permanently kill session? All programs terminate. Attached clients will be affected. A running task may be lost.";
    let mut terminal = Terminal::new(TestBackend::new(24, 6)).expect("test terminal");
    let mut offset = 0;
    terminal
        .draw(|frame| {
            offset = render_form_scrolled(
                frame,
                frame.area(),
                "Confirm",
                message,
                &TextInput::new("yes"),
                "Up/Down Review | Esc Back",
                Theme::default(),
                0,
            );
        })
        .expect("draw");
    assert_eq!(offset, 0);
    assert!(screen(&terminal).contains("Permanently kill"));
    assert!(screen(&terminal).contains("yes"));
    terminal
        .draw(|frame| {
            offset = render_form_scrolled(
                frame,
                frame.area(),
                "Confirm",
                message,
                &TextInput::new("yes"),
                "Up/Down Review | Esc Back",
                Theme::default(),
                u16::MAX,
            );
        })
        .expect("draw");
    assert!(offset > 0);
    assert!(screen(&terminal).contains("lost."));
    assert!(screen(&terminal).contains("yes"));
}
