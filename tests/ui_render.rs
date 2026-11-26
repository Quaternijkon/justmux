#![allow(clippy::expect_used, clippy::unwrap_used)]

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use justmux::{
    app::{App, AppOptions},
    tmux::{SocketSpec, TmuxClient},
    ui::{CommandItem, HomeDetails, TextInput, Theme, render_form_scrolled, render_home},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::{Backend, TestBackend},
    layout::Position,
    style::{Color, Modifier},
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
fn welcome_shows_version_directory_context_and_colored_commands() {
    let mut terminal = Terminal::new(TestBackend::new(72, 16)).expect("test terminal");
    terminal
        .draw(|frame| {
            render_home(
                frame,
                frame.area(),
                &TextInput::default(),
                &[],
                0,
                HomeDetails {
                    directory: Some("~/project/justmux"),
                    context: "popup",
                    session: Some("work session"),
                    status: None,
                },
                Theme::default(),
            );
        })
        .expect("draw welcome screen");
    let output = screen(&terminal);
    assert!(output.contains(concat!("◫ JustMux (v", env!("CARGO_PKG_VERSION"), ")")));
    assert!(output.contains("⌂ ~/project/justmux"));
    assert!(output.contains("tmux popup · work session"));
    assert!(output.contains("Create a tmux session"));
    assert!(output.contains("/help"));
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].fg, Color::Cyan);
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "/" && cell.fg == Color::Green)
    );
}

#[test]
fn home_and_command_popup_are_visible_in_test_backend() {
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let input = TextInput::new("/");
    let commands = [CommandItem {
        name: "resume",
        description: "Open a window in an existing tmux session",
        alias: None,
        indices: &[],
    }];
    terminal
        .draw(|frame| {
            render_home(
                frame,
                frame.area(),
                &input,
                &commands,
                0,
                HomeDetails::default(),
                Theme::default(),
            )
        })
        .expect("draw home");
    let output = screen(&terminal);
    assert!(output.contains("JustMux"));
    assert!(output.contains("/resume"));
    assert!(output.contains("Open a window in an existing tmux session"));
}

#[test]
fn resume_grid_has_a_clear_empty_state() {
    let tmux = private_tmux();
    let mut app = App::new(
        &tmux,
        AppOptions {
            initial_command: Some("/resume".into()),
        },
    );
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw grid");
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
    assert!(output.contains("Open a window in an existing tmux session"));
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
    assert!(!screen(&terminal).contains("Search:"));
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
fn alias_prefix_is_highlighted_and_completes_to_the_command_name() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/att");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("/resume /attach"));
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::UNDERLINED))
            .map(|cell| cell.symbol())
            .collect::<String>(),
        "att"
    );
    key(&mut app, KeyCode::Tab);
    terminal.draw(|frame| app.render(frame)).expect("draw");
    let output = screen(&terminal);
    assert!(
        output.contains("› /resume") || output.contains("> /resume"),
        "{output}"
    );
    key(&mut app, KeyCode::Enter);
    terminal.draw(|frame| app.render(frame)).expect("draw");
    assert!(screen(&terminal).contains("No tmux sessions"));
}

#[test]
fn misspelled_command_shows_guidance_without_opening_a_view() {
    let tmux = private_tmux();
    let mut app = App::new(&tmux, AppOptions::default());
    type_text(&mut app, "/reusme");
    key(&mut app, KeyCode::Enter);
    let mut terminal = Terminal::new(TestBackend::new(72, 18)).expect("test terminal");
    terminal.draw(|frame| app.render(frame)).expect("draw");
    let output = screen(&terminal);
    assert!(output.contains("Did you mean /resume?"));
    assert!(output.contains("/reusme"));
    assert!(app.exit().is_none());
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
    let mut terminal = Terminal::new(TestBackend::new(20, 6)).expect("test terminal");
    terminal
        .draw(|frame| {
            render_home(
                frame,
                frame.area(),
                &TextInput::new("abcdefghijklmnopqrstuvwxyz"),
                &[],
                0,
                HomeDetails {
                    directory: Some("~/a/very/long/folder/path/project"),
                    context: "pane",
                    session: Some("work"),
                    status: None,
                },
                Theme {
                    no_color: true,
                    ascii: true,
                    ..Theme::default()
                },
            )
        })
        .expect("draw");
    let output = screen(&terminal);
    assert!(output.contains("[] JustMux"));
    assert!(output.contains("dir ..."));
    assert!(output.contains("/project"));
    assert!(output.contains("z"));
    assert!(output.is_ascii());
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset)
    );
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
    for _ in 1..justmux::commands::registry().len() {
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
    for _ in 1..justmux::commands::registry().len() {
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

#[test]
fn controller_layout_studio_renders_presets_resize_hud_en_zh_ascii_no_color() {
    use justmux::ui::{
        ControllerAction, Language, LayoutStudioView, SubviewContent, WorkspaceSubviewSpec,
        render_workspace_subview,
    };

    let view = LayoutStudioView {
        target_label: "work:1 editor",
        selected_preset: 3,
        pane_size: (112, 34),
        pane_count: 3,
        resize_step: 5,
        zoomed: true,
        synchronized: true,
        status_note: Some("Applied Main Vertical"),
        ..LayoutStudioView::default()
    };

    // EN wide render
    let mut term = Terminal::new(TestBackend::new(112, 30)).expect("en term");
    let mut layout = Default::default();
    term.draw(|frame| {
        layout = render_workspace_subview(
            frame,
            frame.area(),
            &WorkspaceSubviewSpec {
                content: SubviewContent::LayoutStudio(view.clone()),
                ..WorkspaceSubviewSpec::default()
            },
            Theme::default(),
        );
    })
    .expect("draw en layout studio");
    let en_out = screen(&term);
    assert!(
        en_out.contains("Even Horizontal") || en_out.contains("Even H"),
        "{en_out}"
    );
    assert!(
        en_out.contains("Main Vertical") || en_out.contains("Main V"),
        "{en_out}"
    );
    assert!(en_out.contains("112"), "{en_out}");
    assert!(en_out.contains("34"), "{en_out}");
    assert!(
        layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::SelectLayoutPreset(0)),
        "expected clickable layout preset hit: {:?}",
        layout.controller_hits
    );
    assert!(
        layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::ToggleZoom)
    );
    assert!(
        layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::ToggleSync)
    );

    // ZH render
    let mut zh_term = Terminal::new(TestBackend::new(112, 30)).expect("zh term");
    let zh_theme = Theme {
        language: Language::Zh,
        ..Theme::default()
    };
    zh_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::LayoutStudio(view.clone()),
                    ..WorkspaceSubviewSpec::default()
                },
                zh_theme,
            );
        })
        .expect("draw zh layout studio");
    let zh_out = screen(&zh_term);
    assert!(
        zh_out.contains("水平均分") && zh_out.contains("网格平铺"),
        "{zh_out}"
    );

    // ASCII + NO_COLOR render
    let mut ascii_term = Terminal::new(TestBackend::new(100, 28)).expect("ascii term");
    let ascii_nc = Theme {
        ascii: true,
        no_color: true,
        ..Theme::default()
    };
    ascii_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::LayoutStudio(view),
                    ..WorkspaceSubviewSpec::default()
                },
                ascii_nc,
            );
        })
        .expect("draw ascii layout studio");
    let ascii_out = screen(&ascii_term);
    assert!(
        ascii_out
            .chars()
            .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
        "expected ASCII content:\n{ascii_out}"
    );
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn controller_clients_view_renders_size_mismatch_warning_and_actions() {
    use justmux::ui::{
        ClientRowView, ClientsView, ControllerAction, Language, SubviewContent,
        WorkspaceSubviewSpec, render_workspace_subview,
    };

    let clients = vec![
        ClientRowView {
            name: "/dev/ttys001".into(),
            session_name: "work".into(),
            width: 160,
            height: 48,
            tty: "/dev/ttys001".into(),
            readonly: false,
            is_current: true,
        },
        ClientRowView {
            name: "/dev/ttys009".into(),
            session_name: "work".into(),
            width: 80,
            height: 24,
            tty: "/dev/ttys009".into(),
            readonly: true,
            is_current: false,
        },
    ];
    let view = ClientsView {
        clients: &clients,
        selected: 1,
        size_mismatch_warning: true,
        status_note: None,
    };

    let mut term = Terminal::new(TestBackend::new(108, 26)).expect("clients en");
    let mut layout = Default::default();
    term.draw(|frame| {
        layout = render_workspace_subview(
            frame,
            frame.area(),
            &WorkspaceSubviewSpec {
                content: SubviewContent::Clients(view.clone()),
                ..WorkspaceSubviewSpec::default()
            },
            Theme::default(),
        );
    })
    .expect("draw clients en");
    let en_out = screen(&term);
    assert!(
        en_out.contains("/dev/ttys001") && en_out.contains("/dev/ttys009"),
        "{en_out}"
    );
    assert!(en_out.contains("160") && en_out.contains("80"), "{en_out}");
    assert!(layout.list_rect.is_some());
    assert!(
        layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::DetachOtherClients),
        "expected DetachOtherClients hit: {:?}",
        layout.controller_hits
    );

    let mut zh_term = Terminal::new(TestBackend::new(108, 26)).expect("clients zh");
    zh_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Clients(view.clone()),
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw clients zh");
    let zh_out = screen(&zh_term);
    assert!(
        zh_out.contains("客户端") || zh_out.contains("踢出"),
        "{zh_out}"
    );

    let mut ascii_term = Terminal::new(TestBackend::new(100, 24)).expect("clients ascii");
    ascii_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Clients(view),
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
            );
        })
        .expect("draw clients ascii");
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn controller_buffers_and_snapshots_views_render_split_previews_and_hits() {
    use justmux::ui::{
        BufferRowView, BuffersView, ControllerAction, Language, SnapshotRowView, SnapshotsView,
        SubviewContent, WorkspaceSubviewSpec, render_workspace_subview,
    };

    let buffers = vec![BufferRowView {
        name: "buffer0".into(),
        size: 42,
        sample: "cargo test --all".into(),
    }];
    let buf_preview = vec![
        "cargo test --all".to_string(),
        "cargo build --release".to_string(),
    ];
    let mut term = Terminal::new(TestBackend::new(108, 26)).expect("buffers term");
    let mut buf_layout = Default::default();
    term.draw(|frame| {
        buf_layout = render_workspace_subview(
            frame,
            frame.area(),
            &WorkspaceSubviewSpec {
                content: SubviewContent::Buffers(BuffersView {
                    buffers: &buffers,
                    selected: 0,
                    preview_lines: &buf_preview,
                    preview_scroll: 0,
                    status_note: None,
                }),
                ..WorkspaceSubviewSpec::default()
            },
            Theme::default(),
        );
    })
    .expect("draw buffers");
    let buf_out = screen(&term);
    assert!(
        buf_out.contains("buffer0") && buf_out.contains("cargo build --release"),
        "{buf_out}"
    );
    assert!(buf_layout.list_rect.is_some());
    assert!(buf_layout.preview_area.is_some());
    assert!(
        buf_layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::PasteBuffer)
    );

    let snapshots = vec![SnapshotRowView {
        name: "backend-dev".into(),
        created_label: "2026-11-23 15:00".into(),
        session_count: 2,
        window_count: 4,
        pane_count: 6,
    }];
    let tree = vec![
        "Session: api (/srv/api)".to_string(),
        "  Window: server [2 panes]".to_string(),
    ];
    let mut snap_term = Terminal::new(TestBackend::new(108, 26)).expect("snap term");
    let mut snap_layout = Default::default();
    snap_term
        .draw(|frame| {
            snap_layout = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Snapshots(SnapshotsView {
                        snapshots: &snapshots,
                        selected: 0,
                        tree_preview: &tree,
                        preview_scroll: 0,
                        status_note: None,
                    }),
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw snapshots zh");
    let snap_out = screen(&snap_term);
    assert!(
        snap_out.contains("backend-dev") && snap_out.contains("Session: api"),
        "{snap_out}"
    );
    assert!(snap_layout.list_rect.is_some());
    assert!(snap_layout.preview_area.is_some());
    assert!(
        snap_layout
            .controller_hits
            .iter()
            .any(|(_, a)| *a == ControllerAction::RestoreSnapshot)
    );
}

#[test]
fn controller_target_picker_and_dual_form_render_cleanly_bilingual() {
    use justmux::ui::{
        Language, PanePreview, RichPickerRow, SubviewContent, WorkspaceSubviewSpec,
        render_workspace_subview,
    };

    let rows = vec![RichPickerRow {
        primary: "ops".into(),
        primary_indices: vec![0, 1],
        secondary: "3 windows".into(),
        badge: Some(("● attached".into(), Color::Green)),
        is_action: false,
    }];
    let query = TextInput::new("op");
    let preview_lines = vec!["tail -f /var/log/app.log".to_string()];

    let mut tp_term = Terminal::new(TestBackend::new(100, 24)).expect("tp term");
    let mut tp_layout = Default::default();
    tp_term
        .draw(|frame| {
            tp_layout = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::TargetPicker {
                        title: "Move Window to Session",
                        action_banner: "Choose destination session for window work:1",
                        query: &query,
                        rows: &rows,
                        selected: 0,
                        empty: "No other sessions",
                        preview: Some(PanePreview {
                            title: "ops:0",
                            subtitle: "/srv/ops",
                            lines: &preview_lines,
                            ..PanePreview::default()
                        }),
                    },
                    ..WorkspaceSubviewSpec::default()
                },
                Theme::default(),
            );
        })
        .expect("draw target picker");
    let tp_out = screen(&tp_term);
    assert!(tp_out.contains("Choose destination session"), "{tp_out}");
    assert!(tp_out.contains("ops"), "{tp_out}");
    assert!(tp_layout.list_rect.is_some());

    let name_in = TextInput::new("worker");
    let cwd_in = TextInput::new("/tmp/project");
    let mut df_term = Terminal::new(TestBackend::new(88, 22)).expect("df term");
    df_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::DualForm {
                        title: "新建窗口",
                        first_label: "窗口名称",
                        first_input: &name_in,
                        second_label: "工作目录（可选）",
                        second_input: &cwd_in,
                        active_field: 1,
                        dir_exists: true,
                        auto_name: false,
                        completions: &[],
                        completion_selected: 0,
                        completion_open: false,
                    },
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw dual form zh");
    let df_out = screen(&df_term);
    assert!(
        df_out.contains("worker") && df_out.contains("/tmp/project"),
        "{df_out}"
    );
    assert!(
        df_out.contains("窗口名称") && df_out.contains("工作目录"),
        "{df_out}"
    );
}

#[test]
fn home_detach_pill_window_status_badges_and_preview_search_highlighting() {
    use justmux::ui::{
        HomeQuickAction, HomeTelemetry, Language, PanePreview, PreviewAction, WindowStatusFlags,
        WorkspaceHomeSpec, render_workspace_home, window_status_badge_spans,
    };

    // Verify status badges across EN, ZH, and ASCII+NO_COLOR.
    let all_flags = WindowStatusFlags {
        zoomed: true,
        synchronized: true,
        activity: true,
        bell: false,
        silence: false,
        dead: true,
        multi_clients: 3,
    };
    let en_badges: String = window_status_badge_spans(all_flags, Theme::default())
        .into_iter()
        .map(|s| s.content.into_owned())
        .collect();
    assert!(en_badges.contains("⛶") && en_badges.contains("⚡SYNC") && en_badges.contains("👥 3"));

    let zh_badges: String = window_status_badge_spans(
        all_flags,
        Theme {
            language: Language::Zh,
            ..Theme::default()
        },
    )
    .into_iter()
    .map(|s| s.content.into_owned())
    .collect();
    assert!(zh_badges.contains("⚡同步"), "{zh_badges}");

    let ascii_badges: String = window_status_badge_spans(
        all_flags,
        Theme {
            ascii: true,
            no_color: true,
            ..Theme::default()
        },
    )
    .into_iter()
    .map(|s| s.content.into_owned())
    .collect();
    assert!(ascii_badges.is_ascii());
    assert!(
        ascii_badges.contains("[Z]")
            && ascii_badges.contains("[!SYNC]")
            && ascii_badges.contains("3c")
    );

    // Verify Home inside_tmux Detach pill + multi-pane preview search highlighting & clickable hits.
    let preview_lines = vec![
        "starting worker...".to_string(),
        "ERROR: socket timeout on worker".to_string(),
        "retrying worker".to_string(),
    ];
    let spec = WorkspaceHomeSpec {
        details: HomeDetails {
            directory: Some("~/project/justmux"),
            context: "pane",
            session: Some("work"),
            status: None,
        },
        telemetry: HomeTelemetry {
            sessions: Some(2),
            windows: Some(4),
            attached: Some(1),
            clients: Some(2),
            hint_badge: None,
        },
        inside_tmux: true,
        live_preview: true,
        preview: Some(PanePreview {
            title: "work:0 · logs",
            subtitle: "tail",
            lines: &preview_lines,
            scroll: 0,
            search_query: Some("ERROR"),
            search_active: true,
            match_count: 1,
            current_match: 1,
            pane_index: Some((1, 3)),
            zoomed: true,
            synchronized: true,
            piping: true,
            show_actions: true,
            ..PanePreview::default()
        }),
        ..WorkspaceHomeSpec::default()
    };

    let mut term = Terminal::new(TestBackend::new(120, 32)).expect("home term");
    let mut home_layout = Default::default();
    term.draw(|frame| {
        home_layout = render_workspace_home(
            frame,
            frame.area(),
            &TextInput::default(),
            spec,
            Theme::default(),
            |_, _| {},
        );
    })
    .expect("draw home with detach & search");
    let out = screen(&term);
    assert!(out.contains("Detach"), "expected Detach pill:\n{out}");
    assert!(out.contains("1/3"), "expected 1/3 pane counter:\n{out}");
    assert!(out.contains("ERROR"), "expected matched line:\n{out}");
    assert!(
        home_layout
            .quick_action_hits
            .iter()
            .any(|(_, a)| *a == HomeQuickAction::Detach),
        "expected clickable Detach hit: {:?}",
        home_layout.quick_action_hits
    );
    assert!(
        home_layout
            .preview_action_hits
            .iter()
            .any(|(_, a)| *a == PreviewAction::NextPane),
        "expected clickable NextPane hit: {:?}",
        home_layout.preview_action_hits
    );
    assert!(
        home_layout
            .preview_action_hits
            .iter()
            .any(|(_, a)| *a == PreviewAction::Search)
    );

    // ASCII + NO_COLOR invariant on the same upgraded Home view.
    let mut ascii_term = Terminal::new(TestBackend::new(112, 30)).expect("ascii home");
    ascii_term
        .draw(|frame| {
            render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                spec,
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
                |_, _| {},
            );
        })
        .expect("draw ascii home");
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset),
        "expected zero colors emitted under no_color"
    );
}

#[test]
fn desktop_top_menubar_renders_tabs_sparkline_icons_and_dropdown_en_zh_ascii_no_color() {
    use justmux::ui::{
        ActiveDesktopTab, ContextMenuItemView, ContextMenuView, DesktopMenubarState,
        DesktopOverlaySpec, HomeTelemetry, Language, MenubarItem, WorkspaceHomeSpec,
        render_desktop_overlays, render_workspace_home,
    };

    let spark = [0_u8, 2, 5, 9, 4, 8, 6, 3];
    let spec = WorkspaceHomeSpec {
        details: HomeDetails {
            directory: Some("~/project/justmux"),
            context: "pane",
            session: Some("work"),
            status: None,
        },
        telemetry: HomeTelemetry {
            sessions: Some(2),
            windows: Some(4),
            attached: Some(1),
            clients: Some(2),
            hint_badge: None,
        },
        inside_tmux: true,
        live_preview: true,
        menubar: DesktopMenubarState {
            active_tab: ActiveDesktopTab::Studio,
            hovered: Some(MenubarItem::TabExpose),
            menu_open: true,
            pip_active: true,
            inside_tmux: true,
            sparkline: &spark,
        },
        ..WorkspaceHomeSpec::default()
    };

    let dropdown_items = vec![
        ContextMenuItemView {
            label: "Panes".into(),
            icon: "⊞".into(),
            shortcut: "/pane".into(),
            danger: false,
            divider_before: false,
            enabled: true,
        },
        ContextMenuItemView {
            label: "Clients".into(),
            icon: "👥".into(),
            shortcut: "/clients".into(),
            danger: false,
            divider_before: false,
            enabled: true,
        },
        ContextMenuItemView {
            label: "Detach Client".into(),
            icon: "⏏".into(),
            shortcut: "/exit".into(),
            danger: false,
            divider_before: true,
            enabled: true,
        },
    ];
    let overlay_spec = DesktopOverlaySpec {
        context_menu: Some(ContextMenuView {
            title: "Workspace Menu",
            anchor: (2, 1),
            items: &dropdown_items,
            selected: 0,
            dim_backdrop: false,
        }),
        ..DesktopOverlaySpec::default()
    };

    // EN wide render
    let mut term = Terminal::new(TestBackend::new(124, 30)).expect("menubar en");
    let mut home_layout = Default::default();
    let mut overlay_layout = Default::default();
    term.draw(|frame| {
        home_layout = render_workspace_home(
            frame,
            frame.area(),
            &TextInput::default(),
            spec,
            Theme::default(),
            |_, _| {},
        );
        overlay_layout =
            render_desktop_overlays(frame, frame.area(), &overlay_spec, Theme::default());
    })
    .expect("draw menubar + dropdown");
    let en_out = screen(&term);
    assert!(
        en_out.contains("Studio") && en_out.contains("Expose"),
        "{en_out}"
    );
    assert!(
        en_out.contains("Split") && en_out.contains("Layout"),
        "{en_out}"
    );
    assert!(
        en_out.contains("Menu") && en_out.contains("PIP"),
        "{en_out}"
    );
    assert!(
        en_out.contains("Panes") && en_out.contains("Clients"),
        "{en_out}"
    );
    for expected_item in [
        MenubarItem::TabStudio,
        MenubarItem::TabExpose,
        MenubarItem::TabSplit,
        MenubarItem::TabLayout,
        MenubarItem::MenuDropdown,
        MenubarItem::PipToggle,
        MenubarItem::ThemeCycle,
        MenubarItem::LanguageToggle,
        MenubarItem::Config,
        MenubarItem::Detach,
    ] {
        assert!(
            home_layout
                .menubar_hits
                .iter()
                .any(|(_, item)| *item == expected_item),
            "missing menubar hit {expected_item:?} in {:?}",
            home_layout.menubar_hits
        );
    }
    assert_eq!(overlay_layout.menu_item_hits.len(), 3);

    // ZH render
    let mut zh_term = Terminal::new(TestBackend::new(124, 30)).expect("menubar zh");
    zh_term
        .draw(|frame| {
            render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                spec,
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
                |_, _| {},
            );
        })
        .expect("draw menubar zh");
    let zh_out = screen(&zh_term);
    assert!(
        zh_out.contains("工作台") || zh_out.contains("总览") || zh_out.contains("分屏"),
        "{zh_out}"
    );

    // ASCII + NO_COLOR invariant
    let mut ascii_term = Terminal::new(TestBackend::new(124, 30)).expect("menubar ascii");
    ascii_term
        .draw(|frame| {
            render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                spec,
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
                |_, _| {},
            );
        })
        .expect("draw menubar ascii");
    let ascii_out = screen(&ascii_term);
    assert!(
        ascii_out
            .chars()
            .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
        "expected pure ASCII menubar:\n{ascii_out}"
    );
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn desktop_expose_mission_control_renders_grid_sparklines_controls_pip_and_dock() {
    use justmux::ui::{
        ActiveDesktopTab, DesktopMenubarState, DockZone, ExposeTileView, ExposeView, Language,
        PreviewAction, SubviewContent, TileControl, WindowStatusFlags, WorkspaceSubviewSpec,
        render_workspace_subview,
    };

    let l0 = vec!["\x1b[32mserver listening :8080\x1b[0m".to_string()];
    let l1 = vec!["cargo watch --test all".to_string()];
    let l2 = vec!["ERROR: upstream reset".to_string()];
    let l3 = vec!["htop · 24% cpu".to_string()];
    let s0 = [1_u8, 3, 7, 9, 6, 4];
    let s1 = [0_u8, 1, 2, 1, 0, 1];
    let tiles = vec![
        ExposeTileView {
            index: 0,
            session_name: "api",
            window_index: 0,
            window_name: "server",
            command: "cargo",
            preview_lines: &l0,
            sparkline: &s0,
            flags: WindowStatusFlags::default(),
            selected: true,
            hovered: false,
            hovered_control: None,
            pip_active: true,
            dock_highlight: None,
        },
        ExposeTileView {
            index: 1,
            session_name: "api",
            window_index: 1,
            window_name: "tests",
            command: "cargo",
            preview_lines: &l1,
            sparkline: &s1,
            flags: WindowStatusFlags {
                zoomed: true,
                ..WindowStatusFlags::default()
            },
            selected: false,
            hovered: true,
            hovered_control: Some(TileControl::Zoom),
            pip_active: false,
            dock_highlight: Some(DockZone::Left),
        },
        ExposeTileView {
            index: 2,
            session_name: "ops",
            window_index: 0,
            window_name: "logs",
            command: "tail",
            preview_lines: &l2,
            sparkline: &s0,
            flags: WindowStatusFlags {
                synchronized: true,
                ..WindowStatusFlags::default()
            },
            selected: false,
            hovered: false,
            hovered_control: None,
            pip_active: false,
            dock_highlight: None,
        },
        ExposeTileView {
            index: 3,
            session_name: "ops",
            window_index: 1,
            window_name: "top",
            command: "htop",
            preview_lines: &l3,
            sparkline: &s1,
            flags: WindowStatusFlags::default(),
            selected: false,
            hovered: false,
            hovered_control: None,
            pip_active: false,
            dock_highlight: None,
        },
    ];
    let expose = ExposeView {
        tiles: &tiles,
        selected: 0,
        page: 0,
        total_pages: 1,
        total_windows: 4,
        pip_active: true,
        status_note: None,
    };
    let spec = WorkspaceSubviewSpec {
        menubar: DesktopMenubarState {
            active_tab: ActiveDesktopTab::Expose,
            pip_active: true,
            sparkline: &s0,
            ..DesktopMenubarState::default()
        },
        content: SubviewContent::Expose(expose),
        ..WorkspaceSubviewSpec::default()
    };

    let mut term = Terminal::new(TestBackend::new(124, 34)).expect("expose en");
    let mut layout = Default::default();
    term.draw(|frame| {
        layout = render_workspace_subview(frame, frame.area(), &spec, Theme::default());
    })
    .expect("draw expose en");
    let en_out = screen(&term);
    assert!(en_out.contains("api") && en_out.contains("ops"), "{en_out}");
    assert!(en_out.contains("LIVE INPUT"), "{en_out}");
    assert!(en_out.contains("Send Esc"), "{en_out}");
    assert!(en_out.contains("Left 50%"), "{en_out}");
    assert_eq!(layout.expose_tile_hits.len(), 4);
    assert!(
        layout.expose_tile_hits[0]
            .control_hits
            .iter()
            .any(|(_, c)| *c == TileControl::Pip)
    );
    assert!(
        layout
            .preview_action_hits
            .iter()
            .any(|(_, a)| *a == PreviewAction::SendEsc),
        "expected clickable SendEsc hit in Expose PIP mode: {:?}",
        layout.preview_action_hits
    );

    // ZH + ASCII/NO_COLOR renders
    let mut zh_term = Terminal::new(TestBackend::new(124, 34)).expect("expose zh");
    zh_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &spec,
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw expose zh");
    let zh_out = screen(&zh_term);
    assert!(
        zh_out.contains("实时") || zh_out.contains("发送 Esc"),
        "{zh_out}"
    );

    let mut ascii_term = Terminal::new(TestBackend::new(120, 32)).expect("expose ascii");
    ascii_term
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &spec,
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
            );
        })
        .expect("draw expose ascii");
    let ascii_out = screen(&ascii_term);
    assert!(
        ascii_out
            .chars()
            .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
        "expected pure ASCII expose:\n{ascii_out}"
    );
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn desktop_overlays_render_clamped_context_menu_ghost_dock_tooltip_toast_and_rename() {
    use justmux::ui::{
        ContextMenuItemView, ContextMenuView, DesktopOverlaySpec, DockZone, DragGhostView,
        HoverTooltipView, InlineRenameView, Language, ToastKind, ToastView,
        render_desktop_overlays,
    };
    use ratatui::layout::Rect;

    let items = vec![
        ContextMenuItemView {
            label: "Open Window".into(),
            icon: "▸".into(),
            shortcut: "Enter".into(),
            danger: false,
            divider_before: false,
            enabled: true,
        },
        ContextMenuItemView {
            label: "Rename".into(),
            icon: "✎".into(),
            shortcut: "F2".into(),
            danger: false,
            divider_before: false,
            enabled: true,
        },
        ContextMenuItemView {
            label: "Close Window".into(),
            icon: "✕".into(),
            shortcut: "MidClick".into(),
            danger: true,
            divider_before: true,
            enabled: true,
        },
    ];
    let rename_input = TextInput::new("editor-v2");
    let spec = DesktopOverlaySpec {
        context_menu: Some(ContextMenuView {
            title: "Window Menu",
            anchor: (116, 4), // near right edge to verify clamping without covering bottom-right toast
            items: &items,
            selected: 2,
            dim_backdrop: true,
        }),
        drag_ghost: Some(DragGhostView {
            label: "work:1 logs",
            detail: "Drop to dock Right 50%",
            cursor: (48, 12),
            dock_target: Some((Rect::new(6, 4, 60, 14), DockZone::Right)),
        }),
        hover_tooltip: Some(HoverTooltipView {
            text: "Toggle Live PIP (Ctrl+I)",
            anchor: (40, 2),
        }),
        toast: Some(ToastView {
            kind: ToastKind::Success,
            message: "Docked pane to right 50%",
            progress: 0.65,
        }),
        inline_rename: Some(InlineRenameView {
            title: "Rename Window",
            target_label: "work:1",
            input: &rename_input,
            anchor: Some((4, 20)),
        }),
    };

    let mut term = Terminal::new(TestBackend::new(120, 30)).expect("overlay en");
    let mut layout = Default::default();
    term.draw(|frame| {
        layout = render_desktop_overlays(frame, frame.area(), &spec, Theme::default());
    })
    .expect("draw overlays");
    let out = screen(&term);
    assert!(out.contains("Close Window"), "{out}");
    assert!(out.contains("Right 50%"), "{out}");
    assert!(out.contains("editor-v2"), "{out}");
    assert!(out.contains("Docked pane to right 50%"), "{out}");
    let menu_rect = layout.menu_rect.expect("menu_rect");
    assert!(menu_rect.right() <= 120 && menu_rect.bottom() <= 30);
    assert_eq!(layout.menu_item_hits.len(), 3);
    assert!(layout.rename_rect.is_some());
    assert!(layout.toast_rect.is_some());

    // Hover tooltip renders cleanly when no modal menu or drag ghost is active.
    let tip_spec = DesktopOverlaySpec {
        hover_tooltip: Some(HoverTooltipView {
            text: "Toggle Live PIP (Ctrl+I)",
            anchor: (40, 2),
        }),
        ..DesktopOverlaySpec::default()
    };
    let mut tip_term = Terminal::new(TestBackend::new(120, 12)).expect("tooltip term");
    tip_term
        .draw(|frame| {
            render_desktop_overlays(frame, frame.area(), &tip_spec, Theme::default());
        })
        .expect("draw tooltip");
    assert!(screen(&tip_term).contains("Toggle Live PIP"));

    // ZH + ASCII/NO_COLOR renders
    let mut zh_term = Terminal::new(TestBackend::new(120, 30)).expect("overlay zh");
    zh_term
        .draw(|frame| {
            render_desktop_overlays(
                frame,
                frame.area(),
                &spec,
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw overlays zh");
    assert!(screen(&zh_term).contains("右侧分屏 50%"));

    let mut ascii_term = Terminal::new(TestBackend::new(120, 30)).expect("overlay ascii");
    ascii_term
        .draw(|frame| {
            render_desktop_overlays(
                frame,
                frame.area(),
                &spec,
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
            );
        })
        .expect("draw overlays ascii");
    assert!(
        ascii_term
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}

#[test]
fn desktop_preview_scrollbar_selection_and_split_sash_render_in_en_zh_ascii_no_color() {
    use justmux::ui::{
        DockZone, Language, PanePreview, PreviewAction, SashId, SplitCanvasNode,
        SplitCanvasPaneView, SplitStudioView, SplitWindowRowView, SubviewContent,
        WorkspaceHomeSpec, WorkspaceSubviewSpec, render_workspace_home, render_workspace_subview,
    };

    let lines = vec![
        "booting service worker...".to_string(),
        "MATCH_QUERY ok on port 8080".to_string(),
        "ERROR: panic in worker thread".to_string(),
        "restarting...".to_string(),
    ];
    let match_lines = [1_usize];
    let error_lines = [2_usize];

    let home_spec = WorkspaceHomeSpec {
        live_preview: true,
        home_split_ratio: 64,
        home_sash_hovered: true,
        home_sash_dragging: true,
        preview: Some(PanePreview {
            title: "api:0 · server",
            subtitle: "cargo",
            lines: &lines,
            scroll: 0,
            search_query: Some("MATCH_QUERY"),
            search_active: false,
            match_count: 1,
            current_match: 1,
            show_actions: true,
            pip_active: true,
            hovered_action: Some(PreviewAction::TogglePip),
            scrollbar_hovered: true,
            scrollbar_dragging: true,
            search_match_lines: &match_lines,
            error_lines: &error_lines,
            selection_range: Some(((0, 2), (0, 12))),
            dock_highlight: Some(DockZone::Bottom),
            ..PanePreview::default()
        }),
        ..WorkspaceHomeSpec::default()
    };

    let mut term = Terminal::new(TestBackend::new(120, 32)).expect("preview sash en");
    let mut home_layout = Default::default();
    term.draw(|frame| {
        home_layout = render_workspace_home(
            frame,
            frame.area(),
            &TextInput::default(),
            home_spec,
            Theme::default(),
            |_, _| {},
        );
    })
    .expect("draw preview sash");
    let out = screen(&term);
    assert!(
        out.contains("64% │ 36%"),
        "expected sash ratio badge:\n{out}"
    );
    assert!(out.contains("LIVE INPUT"), "{out}");
    assert!(out.contains("Bottom 50%"), "{out}");
    assert!(home_layout.preview_layout.content_rect.is_some());
    assert!(home_layout.preview_layout.scrollbar_track_rect.is_some());
    assert!(home_layout.preview_layout.scrollbar_thumb_rect.is_some());
    assert!(
        home_layout
            .sash_hits
            .iter()
            .any(|(_, id)| *id == SashId::HomePreviewMatrix)
    );
    for expected in [
        PreviewAction::TogglePip,
        PreviewAction::SendEsc,
        PreviewAction::Zoom,
        PreviewAction::Rename,
        PreviewAction::Close,
    ] {
        assert!(
            home_layout
                .preview_action_hits
                .iter()
                .any(|(_, a)| *a == expected),
            "missing preview action {expected:?}: {:?}",
            home_layout.preview_action_hits
        );
    }

    // Split Studio draggable sash + dock overlay across ZH and ASCII+NO_COLOR.
    let tree = SplitCanvasNode::Split {
        horizontal: true,
        ratio: 64,
        first: Box::new(SplitCanvasNode::Pane(SplitCanvasPaneView {
            slot_id: 1,
            ordinal: 1,
            title: "api:0 server".into(),
            subtitle: "cargo".into(),
            selected: true,
            is_new_shell: false,
            preview_lines: &lines,
            preview_scroll: 0,
        })),
        second: Box::new(SplitCanvasNode::Pane(SplitCanvasPaneView {
            slot_id: 2,
            ordinal: 2,
            title: "+ New shell".into(),
            subtitle: "zsh".into(),
            selected: false,
            is_new_shell: true,
            preview_lines: &[],
            preview_scroll: 0,
        })),
    };
    let opts = vec![SplitWindowRowView {
        primary: "api:0 server".into(),
        detail: "cargo".into(),
        active_in_session: true,
        assigned_slot: Some(1),
        is_new_shell: false,
    }];
    let split_view = SplitStudioView {
        root: tree,
        selected_slot: 1,
        selected_ordinal: 1,
        slot_count: 2,
        can_split_h: false,
        can_split_v: true,
        can_flip: true,
        can_remove: true,
        active_ratio: 64,
        can_adjust_ratio: true,
        sync_panes: false,
        can_break_out: false,
        editing_existing: false,
        window_rows: &opts,
        window_cursor: 0,
        hovered_sash: Some(SashId::SplitNode(0)),
        dragging_sash: Some(SashId::SplitNode(0)),
        hovered_dock: Some((2, DockZone::Top)),
    };

    let mut zh_split = Terminal::new(TestBackend::new(116, 30)).expect("split zh");
    let mut split_layout = Default::default();
    zh_split
        .draw(|frame| {
            split_layout = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Split(split_view.clone()),
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    language: Language::Zh,
                    ..Theme::default()
                },
            );
        })
        .expect("draw split sash zh");
    let zh_out = screen(&zh_split);
    assert!(zh_out.contains("64% │ 36%"), "{zh_out}");
    assert!(zh_out.contains("上方分屏 50%"), "{zh_out}");
    assert!(
        split_layout
            .sash_hits
            .iter()
            .any(|(_, id)| *id == SashId::SplitNode(0))
    );

    let mut ascii_split = Terminal::new(TestBackend::new(116, 30)).expect("split ascii");
    ascii_split
        .draw(|frame| {
            render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Split(split_view),
                    ..WorkspaceSubviewSpec::default()
                },
                Theme {
                    ascii: true,
                    no_color: true,
                    ..Theme::default()
                },
            );
        })
        .expect("draw split ascii");
    let ascii_out = screen(&ascii_split);
    assert!(ascii_out.contains("64% | 36%"), "{ascii_out}");
    assert!(
        ascii_split
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}
