#![allow(clippy::expect_used, clippy::unwrap_used)]

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use justmux::{
    app::{App, AppOptions},
    tmux::{SocketSpec, TmuxClient},
    ui::{CommandItem, TextInput, Theme, render_home, render_picker},
};
use ratatui::{Terminal, backend::TestBackend};

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
