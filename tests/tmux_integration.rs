#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use justmux::app::{App, AppOptions, Exit};
use justmux::tmux::{
    Context, SessionId, SocketSpec, SplitDirection, Target, Tmux, TmuxClient, TmuxError,
};

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
        let socket = format!("jm-it-{}-{}-{}", std::process::id(), stamp, counter);
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
        let _ = self.tmux(&["kill-server"]);
    }
}

fn cwd() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn keys(app: &mut App<'_>, codes: &[KeyCode]) {
    for &code in codes {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
}

fn text(app: &mut App<'_>, value: &str) {
    for character in value.chars() {
        keys(app, &[KeyCode::Char(character)]);
    }
}

fn key_mod(app: &mut App<'_>, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_event(Event::Key(KeyEvent::new(code, modifiers)));
}

#[test]
fn session_management_works_through_keyboard_events() {
    use KeyCode::{Down, Enter, Tab};

    let server = PrivateServer::new();
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/new");
    keys(&mut app, &[Enter]);
    text(&mut app, "项目 workspace");
    keys(&mut app, &[Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, env!("CARGO_MANIFEST_DIR"));
    key_mod(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app.exit().is_none(), "Alt+Enter creates & stays on Home");
    let sessions = server.client.list_sessions().expect("created sessions");
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.name, "项目 workspace");

    // Open session actions menu via `/list` + Enter + Enter:
    // 0 Resume, 1 Attach read-only, 2 New window,
    // 3 Rename session, 4 Detach other clients, 5 Save snapshot, 6 Close session.
    text(&mut app, "/list");
    keys(&mut app, &[Enter, Enter, Down, Down, Down, Enter]);
    text(&mut app, "renamed project");
    keys(&mut app, &[Enter]);
    let renamed = server.client.list_sessions().expect("renamed sessions");
    assert_eq!(renamed[0].id, session.id);
    assert_eq!(renamed[0].name, "renamed project");

    // A lone Enter at the destructive confirmation must cancel.
    keys(
        &mut app,
        &[Enter, Down, Down, Down, Down, Down, Down, Enter, Enter],
    );
    assert!(server.client.has_session(&session.id).expect("not killed"));
    keys(
        &mut app,
        &[Enter, Down, Down, Down, Down, Down, Down, Enter],
    );
    text(&mut app, "yes");
    keys(&mut app, &[Enter]);
    assert!(
        !server
            .client
            .has_session(&session.id)
            .expect("confirmed kill")
    );
}

#[test]
fn window_and_pane_management_work_through_keyboard_events() {
    use KeyCode::{Char, Down, Enter, Right};

    let server = PrivateServer::new();
    let session = server.client.new_session("layout", cwd()).expect("session");
    let mut app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/window".into()),
        },
    );
    // In Unified Home Matrix `/window` mode, `+` opens the New Window form.
    keys(&mut app, &[Char('+')]);
    text(&mut app, "build");
    key_mod(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app.exit().is_none(), "Alt+Enter creates & stays on Home");
    let windows = server
        .client
        .list_windows(Some(&session.id))
        .expect("new window");
    assert_eq!(windows.len(), 2);
    let added = windows
        .iter()
        .find(|w| w.name == "build")
        .expect("build window");

    // Filtering `"build"` in `/rename` mode focuses the window directly (no Tab toggle needed).
    text(&mut app, "/rename");
    keys(&mut app, &[Enter]);
    text(&mut app, "build");
    keys(&mut app, &[Enter]);
    text(&mut app, "tests");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("renamed window")
            .iter()
            .any(|w| w.id == added.id && w.name == "tests")
    );

    // `/kill` defaults to Window scope; empty Enter cancels, `"yes"` confirms kill.
    text(&mut app, "/kill");
    keys(&mut app, &[Enter]);
    text(&mut app, "tests");
    keys(&mut app, &[Enter, Enter]);
    assert_eq!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("cancelled")
            .len(),
        2
    );
    text(&mut app, "/kill");
    keys(&mut app, &[Enter]);
    text(&mut app, "tests");
    keys(&mut app, &[Enter]);
    text(&mut app, "yes");
    keys(&mut app, &[Enter]);
    assert_eq!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("killed window")
            .len(),
        1
    );

    let mut app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/split".into()),
        },
    );
    keys(&mut app, &[Enter, KeyCode::Char('h'), Enter]);
    assert!(matches!(app.exit(), Some(Exit::Attach(Target::Window(sid, _))) if sid == &session.id));
    assert_eq!(
        server.client.list_panes(None).expect("split panes").len(),
        2
    );
    let mut app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/pane".into()),
        },
    );
    // Pane actions menu has 17 entries (0 Jump .. 16 Close pane).
    keys(&mut app, &[Down, Enter]);
    for _ in 0..16 {
        keys(&mut app, &[Down]);
    }
    keys(&mut app, &[Enter, Enter]);
    assert_eq!(server.client.list_panes(None).expect("cancelled").len(), 2);
    keys(&mut app, &[Enter]);
    for _ in 0..16 {
        keys(&mut app, &[Down]);
    }
    keys(&mut app, &[Enter]);
    text(&mut app, "yes");
    keys(&mut app, &[Enter]);
    assert_eq!(
        server
            .client
            .list_panes(None)
            .expect("confirmed pane kill")
            .len(),
        1
    );

    // Exercise `+ New window` and `+ New session` matrix creation cells directly from Home.
    let mut matrix_app = App::new(&server.client, AppOptions::default());
    keys(&mut matrix_app, &[Down, Enter]);
    text(&mut matrix_app, "from-matrix");
    key_mod(&mut matrix_app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(matrix_app.exit().is_none());
    assert!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("matrix new window")
            .iter()
            .any(|w| w.name == "from-matrix")
    );
    keys(&mut matrix_app, &[Right, Enter]);
    text(&mut matrix_app, "matrix-workspace");
    key_mod(&mut matrix_app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(matrix_app.exit().is_none());
    assert!(
        server
            .client
            .list_sessions()
            .expect("matrix new session")
            .iter()
            .any(|s| s.name == "matrix-workspace")
    );
}

#[test]
fn no_server_is_reported_without_touching_default_socket() {
    let server = PrivateServer::new();
    assert!(!server.client.server_running().expect("server check"));
    assert!(
        server
            .client
            .list_sessions()
            .expect("list sessions")
            .is_empty()
    );
}

#[test]
fn session_lifecycle_uses_ids_and_preserves_names() {
    let server = PrivateServer::new();
    let created = server
        .client
        .new_session("alpha project", cwd())
        .expect("create session");
    assert_eq!(created.name, "alpha project");
    assert!(created.id.as_str().starts_with('$'));
    assert!(server.client.has_session(&created.id).expect("has session"));

    let sessions = server.client.list_sessions().expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, created.id);
    assert_eq!(sessions[0].name, "alpha project");

    server
        .client
        .rename_session(&created.id, "项目 beta")
        .expect("rename session");
    let renamed = server.client.list_sessions().expect("list renamed");
    assert_eq!(renamed[0].name, "项目 beta");

    server
        .client
        .kill_session(&created.id)
        .expect("kill session");
    assert!(
        !server
            .client
            .has_session(&created.id)
            .expect("has after kill")
    );
    assert!(
        server
            .client
            .list_sessions()
            .expect("list after kill")
            .is_empty()
    );
}

#[test]
fn similar_and_unicode_session_names_are_distinct() {
    let server = PrivateServer::new();
    let first = server.client.new_session("app", cwd()).expect("first");
    let second = server.client.new_session("app two", cwd()).expect("second");
    let third = server
        .client
        .new_session("中文 空间", cwd())
        .expect("third");
    let sessions = server.client.list_sessions().expect("list");
    assert_eq!(sessions.len(), 3);
    assert!(sessions.iter().any(|s| s.id == first.id && s.name == "app"));
    assert!(
        sessions
            .iter()
            .any(|s| s.id == second.id && s.name == "app two")
    );
    assert!(
        sessions
            .iter()
            .any(|s| s.id == third.id && s.name == "中文 空间")
    );
    assert!(matches!(
        server.client.new_session("app", cwd()),
        Err(TmuxError::DuplicateSession(_))
    ));
}

#[test]
fn windows_and_panes_are_real_tmux_objects() {
    let server = PrivateServer::new();
    let session = server.client.new_session("layout", cwd()).expect("session");
    let windows = server
        .client
        .list_windows(Some(&session.id))
        .expect("windows");
    assert_eq!(windows.len(), 1);
    let original = &windows[0];

    let added = server
        .client
        .new_window(&session.id, Some("build"), Some(cwd()))
        .expect("new window");
    assert_eq!(added.session_id, session.id);
    server
        .client
        .rename_window(&added.id, "tests")
        .expect("rename window");
    let all_windows = server
        .client
        .list_windows(Some(&session.id))
        .expect("all windows");
    assert_eq!(all_windows.len(), 2);

    let panes = server.client.list_panes(Some(&original.id)).expect("panes");
    assert_eq!(panes.len(), 1);
    let split = server
        .client
        .split_pane(&panes[0].id, SplitDirection::SideBySide)
        .expect("split");
    assert_eq!(split.window_id, original.id);
    assert_eq!(
        server
            .client
            .list_panes(Some(&original.id))
            .expect("split panes")
            .len(),
        2
    );
    server.client.kill_pane(&split.id).expect("kill pane");
    server.client.kill_window(&added.id).expect("kill window");
    assert_eq!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("remaining")
            .len(),
        1
    );
}

#[test]
fn split_preserves_a_directory_with_spaces_and_unicode() {
    let directory = tempfile::Builder::new()
        .prefix("jm workspace 空格 ")
        .tempdir()
        .expect("workspace");
    let expected = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("workspace", directory.path())
        .expect("session");
    let original = server
        .client
        .list_panes(None)
        .expect("panes")
        .into_iter()
        .find(|pane| pane.session_id == session.id)
        .expect("original pane");
    let split = server
        .client
        .split_pane(&original.id, SplitDirection::SideBySide)
        .expect("split");
    // The first -P format may precede tmux observing the new shell's cwd.
    let mut actual = None;
    for _ in 0..100 {
        actual = server
            .client
            .list_panes(Some(&original.window_id))
            .expect("split panes")
            .into_iter()
            .find(|pane| pane.id == split.id)
            .and_then(|pane| Path::new(&pane.current_path).canonicalize().ok());
        if actual.as_ref() == Some(&expected) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        actual,
        Some(expected),
        "split must start in the target pane's directory"
    );
}

#[test]
fn missing_targets_and_invalid_directories_are_friendly_errors() {
    let server = PrivateServer::new();
    let _existing = server
        .client
        .new_session("existing", cwd())
        .expect("session");
    let missing = SessionId::new("$999999").expect("id");
    assert!(matches!(
        server.client.rename_session(&missing, "x"),
        Err(TmuxError::SessionNotFound)
    ));
    assert!(matches!(
        server.client.kill_session(&missing),
        Err(TmuxError::SessionNotFound)
    ));
    assert!(matches!(
        server
            .client
            .new_session("bad cwd", Path::new("/definitely/not/a/real/path")),
        Err(TmuxError::DirectoryNotFound(_))
    ));
}

#[test]
fn attach_command_uses_private_socket_and_id_target() {
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("attach target", cwd())
        .expect("session");
    let command = server
        .client
        .attach_command(&Target::Session(session.id.clone()));
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-L" && w[1] == server.socket)
    );
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-t" && w[1] == session.id.as_str())
    );
    assert!(args.iter().any(|arg| arg == "attach-session"));
}

#[test]
fn context_is_safe_when_not_inside_target_server() {
    let server = PrivateServer::new();
    assert!(matches!(
        server.client.context(),
        Context::Outside | Context::ForeignServer
    ));
}

#[test]
fn running_server_without_sessions_has_an_empty_list() {
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("temporary", cwd())
        .expect("session");
    assert!(
        server
            .tmux(&["set-option", "-g", "exit-empty", "off"])
            .status
            .success()
    );
    server
        .client
        .kill_session(&session.id)
        .expect("kill last session");
    assert!(
        server
            .client
            .list_sessions()
            .expect("empty server")
            .is_empty()
    );
}

#[test]
fn selection_deleted_by_another_client_is_detected() {
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("deleted", cwd())
        .expect("session");
    let keeper = server.client.new_session("keeper", cwd()).expect("keeper");
    assert!(
        server
            .tmux(&["kill-session", "-t", session.id.as_str()])
            .status
            .success()
    );
    assert!(
        !server
            .client
            .has_session(&session.id)
            .expect("stale selection")
    );
    assert!(
        server
            .client
            .has_session(&keeper.id)
            .expect("other session")
    );
}

#[test]
fn resume_refreshes_when_a_selected_window_is_unlinked() {
    use KeyCode::{Down, Enter};

    let server = PrivateServer::new();
    let origin = server
        .client
        .new_session("origin", cwd())
        .expect("source session");
    let target = server
        .client
        .new_session("target", cwd())
        .expect("target session");
    let shared = server
        .client
        .new_window(&origin.id, Some("shared"), None)
        .expect("shared window");
    let link = format!("{}:7", target.id);
    assert!(
        server
            .tmux(&["link-window", "-d", "-s", shared.id.as_str(), "-t", &link])
            .status
            .success()
    );
    let mut app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/resume".into()),
        },
    );
    text(&mut app, "target");
    keys(&mut app, &[Down]);
    assert!(
        server
            .tmux(&["unlink-window", "-t", &link])
            .status
            .success()
    );
    keys(&mut app, &[Enter]);
    assert!(
        app.exit().is_none(),
        "a shared window must not redirect the client to the other session"
    );
    assert!(
        server
            .client
            .list_windows(Some(&origin.id))
            .expect("origin windows")
            .iter()
            .any(|window| window.id == shared.id)
    );

    let remaining = server
        .client
        .list_windows(Some(&target.id))
        .expect("target windows");
    assert_eq!(remaining.len(), 1);
    // Vertical stops for filtered single-window session are [SessionHeader, Window(0)]:
    // two `Down` presses wrap Window(0) -> SessionHeader -> Window(0), then Enter attaches.
    keys(&mut app, &[Down, Down, Enter]);
    assert_eq!(
        app.exit(),
        Some(&Exit::Attach(Target::Window(
            target.id,
            remaining[0].id.clone()
        )))
    );
}

#[test]
fn path_socket_is_supported() {
    let directory = tempfile::tempdir().expect("tempdir");
    let socket = directory.path().join("tmux.sock");
    let client = TmuxClient::new(SocketSpec::Path(socket.clone())).with_config_file("/dev/null");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .arg("kill-server")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output();
        }
    }
    let _cleanup = Cleanup(socket);
    let session = client
        .new_session("custom path", cwd())
        .expect("create on path");
    assert!(client.has_session(&session.id).expect("path target"));
    assert_eq!(client.list_sessions().expect("path sessions").len(), 1);
}

#[test]
fn missing_executable_and_failed_commands_are_reported() {
    let server = PrivateServer::new();
    let missing = server.client.clone().with_program("/definitely/no/tmux");
    assert!(matches!(missing.version(), Err(TmuxError::NotInstalled)));
    let failed = server.client.clone().with_program("/usr/bin/false");
    assert!(matches!(
        failed.version(),
        Err(TmuxError::CommandFailed { .. })
    ));
}

#[test]
fn attached_clients_do_not_block_management_or_get_detached() {
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("attached", cwd())
        .expect("session");
    let child = Command::new("tmux")
        .arg("-L")
        .arg(&server.socket)
        .args([
            "-f",
            "/dev/null",
            "-C",
            "attach-session",
            "-t",
            session.id.as_str(),
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("control client");
    struct Client(std::process::Child);
    impl Drop for Client {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _client = Client(child);
    let mut attached = false;
    for _ in 0..100 {
        if server
            .client
            .list_sessions()
            .expect("attached list")
            .iter()
            .any(|s| s.id == session.id && s.attached > 0)
        {
            attached = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(attached, "control-mode client should attach");
    server
        .client
        .rename_session(&session.id, "still attached")
        .expect("rename attached");
    assert!(
        server
            .client
            .list_sessions()
            .expect("after rename")
            .iter()
            .any(|s| s.id == session.id && s.attached > 0)
    );
}

#[test]
fn join_pane_combines_windows_into_split_layout() {
    let server = PrivateServer::new();
    let session = server
        .client
        .new_session("studio-join", cwd())
        .expect("session");
    let first_win = server
        .client
        .list_windows(Some(&session.id))
        .expect("first win")
        .into_iter()
        .next()
        .expect("window 0");
    let second_win = server
        .client
        .new_window(&session.id, Some("right"), Some(cwd()))
        .expect("second win");
    let first_pane = server
        .client
        .list_panes(Some(&first_win.id))
        .expect("first pane")
        .into_iter()
        .next()
        .expect("pane 0");
    let second_pane = server
        .client
        .list_panes(Some(&second_win.id))
        .expect("second pane")
        .into_iter()
        .next()
        .expect("pane 1");

    server
        .client
        .join_pane(&second_pane.id, &first_pane.id, SplitDirection::SideBySide)
        .expect("join_pane should move second pane beside first pane");

    let combined = server
        .client
        .list_panes(Some(&first_win.id))
        .expect("combined panes");
    assert_eq!(combined.len(), 2);
    assert!(combined.iter().any(|p| p.id == first_pane.id));
    assert!(combined.iter().any(|p| p.id == second_pane.id));
}

fn wait_for_pane_text(client: &TmuxClient, target: &Target, expected: &str) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let lines = client
            .capture_pane_plain(target, 120)
            .expect("capture_pane_plain");
        if lines.iter().any(|line| line.contains(expected)) {
            return lines;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {expected:?} in {target:?}: {lines:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[test]
fn controller_zoom_break_swap_window_ops_and_pane_actions_e2e() {
    use KeyCode::{Down, Enter, Esc};

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "work",
        "-n",
        "editor",
        "-x",
        "100",
        "-y",
        "30",
        "sh",
    ]);
    assert!(out.status.success());
    let s_work = server.client.list_sessions().expect("sessions")[0].clone();
    let w_editor = server.client.list_windows(Some(&s_work.id)).expect("w")[0].clone();
    let w_logs = server
        .client
        .new_window(&s_work.id, Some("logs"), Some(cwd()))
        .expect("logs win");
    let p0 = server.client.list_panes(Some(&w_editor.id)).expect("p0")[0].clone();
    let p1 = server
        .client
        .split_pane(&p0.id, SplitDirection::SideBySide)
        .expect("p1");

    // 1. Natural phrases: zoom / unzoom / sync panes drive real tmux flags on `work:editor`.
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "zoom");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&s_work.id))
            .expect("wins")
            .iter()
            .any(|w| w.id == w_editor.id && w.zoomed),
        "expected zoomed flag after 'zoom' phrase"
    );
    text(&mut app, "unzoom");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&s_work.id))
            .expect("wins")
            .iter()
            .any(|w| w.id == w_editor.id && !w.zoomed)
    );

    text(&mut app, "sync panes");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&s_work.id))
            .expect("wins")
            .iter()
            .any(|w| w.id == w_editor.id && w.synchronized)
    );
    text(&mut app, "sync panes");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&s_work.id))
            .expect("wins")
            .iter()
            .any(|w| w.id == w_editor.id && !w.synchronized)
    );

    // 2. Pane actions via `/pane`: filter to `work:0.0`, open Pane actions (index 15 = Rename pane title).
    let mut app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/pane".into()),
        },
    );
    text(&mut app, "work:0.0");
    keys(&mut app, &[Enter]);
    for _ in 0..15 {
        keys(&mut app, &[Down]);
    }
    keys(&mut app, &[Enter]);
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    text(&mut app, "build-slot");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_panes(Some(&w_editor.id))
            .expect("renamed pane")
            .iter()
            .any(|p| p.title == "build-slot")
    );

    // Send command (index 12) + clear scrollback (index 13) + respawn (index 14) on real tmux.
    server
        .client
        .send_keys(&p0.id, &["echo INTEGRATED_SEND_OK"], true)
        .expect("send literal");
    server
        .client
        .send_keys(&p0.id, &["Enter"], false)
        .expect("send enter");
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(p0.id.clone()),
        "INTEGRATED_SEND_OK",
    );
    server
        .client
        .clear_pane_history(&p0.id)
        .expect("clear history");
    let old_pid = server
        .client
        .list_panes(Some(&w_editor.id))
        .expect("panes")
        .into_iter()
        .find(|p| p.id == p0.id)
        .expect("p0")
        .pid;
    server.client.respawn_pane(&p0.id, true).expect("respawn");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let cur_pid = server
            .client
            .list_panes(Some(&w_editor.id))
            .expect("after respawn")
            .into_iter()
            .find(|p| p.id == p0.id)
            .expect("p0")
            .pid;
        if cur_pid != old_pid {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "pid did not change");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    // Swap panes and break pane out into a new window.
    let p0_left_before = server
        .client
        .list_panes(Some(&w_editor.id))
        .expect("before swap")
        .into_iter()
        .find(|p| p.id == p0.id)
        .expect("p0")
        .left;
    server.client.swap_pane(&p0.id, &p1.id).expect("swap_pane");
    let p0_left_after = server
        .client
        .list_panes(Some(&w_editor.id))
        .expect("after swap")
        .into_iter()
        .find(|p| p.id == p0.id)
        .expect("p0")
        .left;
    assert_ne!(p0_left_before, p0_left_after);

    let broken = server
        .client
        .break_pane(&p1.id, Some("broken-win"))
        .expect("break_pane");
    assert_eq!(broken.name, "broken-win");
    assert_eq!(
        server
            .client
            .list_panes(Some(&w_editor.id))
            .expect("remaining")
            .len(),
        1
    );

    // 3. Window monitoring, swap-window, move-window, and link-window.
    let s_ops = server.client.new_session("ops", cwd()).expect("ops");
    server
        .client
        .set_monitor_activity(&w_logs.id, true)
        .expect("monitor activity");
    server
        .client
        .set_monitor_silence(&w_logs.id, 30)
        .expect("monitor silence");
    let w_mon = server
        .client
        .list_windows(Some(&s_work.id))
        .expect("monitored")
        .into_iter()
        .find(|w| w.id == w_logs.id)
        .expect("w_logs");
    assert!(w_mon.monitor_activity);
    assert_eq!(w_mon.monitor_silence, 30);

    let idx_before = w_mon.index;
    server
        .client
        .swap_window(&w_editor.id, &w_logs.id)
        .expect("swap_window");
    let idx_after = server
        .client
        .list_windows(Some(&s_work.id))
        .expect("swapped")
        .into_iter()
        .find(|w| w.id == w_logs.id)
        .expect("w_logs")
        .index;
    assert_ne!(idx_before, idx_after);

    server
        .client
        .move_window(&w_logs.id, &s_ops.id)
        .expect("move_window");
    assert!(
        server
            .client
            .list_windows(Some(&s_ops.id))
            .expect("ops wins")
            .iter()
            .any(|w| w.id == w_logs.id)
    );
    assert!(
        !server
            .client
            .list_windows(Some(&s_work.id))
            .expect("work wins")
            .iter()
            .any(|w| w.id == w_logs.id)
    );

    server
        .client
        .link_window(&w_logs.id, &s_work.id)
        .expect("link_window");
    assert!(
        server
            .client
            .list_windows(Some(&s_work.id))
            .expect("work wins after link")
            .iter()
            .any(|w| w.id == w_logs.id)
    );
    assert!(
        server
            .client
            .list_windows(Some(&s_ops.id))
            .expect("ops wins after link")
            .iter()
            .any(|w| w.id == w_logs.id)
    );

    // Dismiss `/pane` cleanly without side effects.
    keys(&mut app, &[Esc]);
}

#[test]
fn controller_layout_studio_presets_resize_zoom_and_sync_e2e() {
    use KeyCode::{Char, Enter, Esc, Left, Right};

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "studio",
        "-n",
        "grid",
        "-x",
        "100",
        "-y",
        "30",
        "sh -c 'sleep 30'",
    ]);
    assert!(out.status.success());
    let win = server.client.list_windows(None).expect("wins")[0].clone();
    let p0 = server.client.list_panes(Some(&win.id)).expect("p0")[0].clone();
    let _p1 = server
        .client
        .split_pane(&p0.id, SplitDirection::SideBySide)
        .expect("p1");

    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/layout");
    keys(&mut app, &[Enter]);

    // Preset `2` -> even-vertical (panes stack vertically, top > 0).
    keys(&mut app, &[Char('2')]);
    let stacked = server.client.list_panes(Some(&win.id)).expect("stacked");
    assert!(stacked.iter().any(|p| p.top > 0), "{stacked:?}");

    // Preset `1` -> even-horizontal (side-by-side, all top == 0).
    keys(&mut app, &[Char('1')]);
    let side_by_side = server.client.list_panes(Some(&win.id)).expect("side");
    assert!(side_by_side.iter().all(|p| p.top == 0), "{side_by_side:?}");
    let width_before = side_by_side
        .iter()
        .find(|p| p.id == p0.id)
        .expect("p0")
        .width;

    // Step resize with `+` and `Left`/`Right` arrow keys changes real pane width.
    keys(&mut app, &[Char('+'), Right, Right]);
    let width_after = server
        .client
        .list_panes(Some(&win.id))
        .expect("resized")
        .into_iter()
        .find(|p| p.id == p0.id)
        .expect("p0")
        .width;
    assert_ne!(width_before, width_after);
    keys(&mut app, &[Left]);

    // `z` toggles zoom and `s` toggles synchronize-panes directly in Layout Studio.
    keys(&mut app, &[Char('z')]);
    assert!(server.client.list_windows(None).expect("w")[0].zoomed);
    keys(&mut app, &[Char('z')]);
    assert!(!server.client.list_windows(None).expect("w")[0].zoomed);

    keys(&mut app, &[Char('s')]);
    assert!(server.client.list_windows(None).expect("w")[0].synchronized);
    keys(&mut app, &[Char('s'), Esc]);
    assert!(!server.client.list_windows(None).expect("w")[0].synchronized);
}

#[test]
fn controller_buffers_export_clients_readonly_and_kill_server_e2e() {
    use KeyCode::{Char, Down, Enter, Esc};

    struct ControlClient(std::process::Child);
    impl Drop for ControlClient {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let server = PrivateServer::new();
    let out = server.tmux(&["new-session", "-d", "-s", "alpha", "sh"]);
    assert!(out.status.success());
    let s_alpha = server.client.list_sessions().expect("alpha")[0].clone();
    let pane_id = server.client.list_panes(None).expect("pane")[0].id.clone();

    // 1. `/buffers`: create new buffer via `n`, paste via `Enter` -> TargetPick, delete via `x`.
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/buffers");
    keys(&mut app, &[Enter, Char('n')]);
    text(&mut app, "echo BUF_PASTE_E2E_OK");
    keys(&mut app, &[Enter]);
    let bufs = server.client.list_buffers().expect("bufs");
    assert_eq!(bufs.len(), 1);
    assert_eq!(
        server
            .client
            .show_buffer(&bufs[0].name)
            .expect("show buffer"),
        "echo BUF_PASTE_E2E_OK"
    );

    // Paste selected buffer into pane via TargetPick (`Enter` on buffers -> `Enter` on first pane).
    keys(&mut app, &[Enter, Enter]);
    server
        .client
        .send_keys(&pane_id, &["Enter"], false)
        .expect("run pasted line");
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(pane_id.clone()),
        "BUF_PASTE_E2E_OK",
    );

    // Re-open `/buffers` after paste returns to Home and delete buffer with `x`.
    text(&mut app, "/buffers");
    keys(&mut app, &[Enter, Char('x'), Esc]);
    assert!(server.client.list_buffers().expect("deleted").is_empty());

    // 2. Log export & pipe-pane into an isolated tempdir (never touches real HOME).
    let temp = tempfile::tempdir().expect("tempdir");
    let lines = server
        .client
        .capture_pane_plain(&Target::Pane(pane_id.clone()), 40)
        .expect("plain");
    let exported =
        justmux::clipboard::export_log_to_file("alpha", "win0", &lines, Some(temp.path()))
            .expect("export log");
    assert!(exported.is_file());
    assert!(
        std::fs::read_to_string(&exported)
            .expect("read log")
            .contains("BUF_PASTE_E2E_OK")
    );
    let pipe_file =
        justmux::clipboard::pipe_log_path_in(temp.path(), "alpha", "win0", pane_id.as_str());
    server
        .client
        .toggle_pipe_pane(&pane_id, Some(&pipe_file))
        .expect("start pipe");
    server
        .client
        .send_keys(&pane_id, &["echo PIPE_E2E_777"], true)
        .expect("send pipe");
    server
        .client
        .send_keys(&pane_id, &["Enter"], false)
        .expect("enter");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Ok(body) = std::fs::read_to_string(&pipe_file)
            && body.contains("PIPE_E2E_777")
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "pipe log timed out");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    server
        .client
        .toggle_pipe_pane(&pane_id, None)
        .expect("stop pipe");

    // 3. `/clients`: attach 2 control-mode clients (1 rw, 1 readonly), detach other via `o`.
    let spawn = |readonly: bool| -> ControlClient {
        let mut cmd = Command::new("tmux");
        cmd.arg("-L")
            .arg(&server.socket)
            .args(["-f", "/dev/null", "-C", "attach-session"]);
        if readonly {
            cmd.arg("-r");
        }
        cmd.args(["-t", s_alpha.id.as_str()])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ControlClient(cmd.spawn().expect("client"))
    };
    let _c1 = spawn(false);
    let _c2 = spawn(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if server.client.list_clients().expect("clients").len() == 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "clients did not attach"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    // In `/clients`, `d` (or `o`) detaches other clients, and `x` detaches the selected client.
    text(&mut app, "/clients");
    keys(&mut app, &[Enter, Char('d')]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if server
            .client
            .list_clients()
            .expect("after detach others")
            .len()
            == 1
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected 1 client remaining after 'd'"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    keys(&mut app, &[Char('x')]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if server
            .client
            .list_clients()
            .expect("after detach selected")
            .is_empty()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "expected 0 clients remaining after 'x'"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    keys(&mut app, &[Esc]);

    // 4. Readonly attach command (`attach -r`) via Session action menu (index 1).
    let mut ro_app = App::new(&server.client, AppOptions::default());
    // Home starts focused on the session's window; Up moves focus to the Session header,
    // or `/list` opens session picker -> Enter opens Session actions -> Down selects Attach read-only.
    text(&mut ro_app, "/list");
    keys(&mut ro_app, &[Enter, Enter, Down, Enter]);
    assert!(
        matches!(ro_app.exit(), Some(Exit::AttachReadOnly(Target::Session(id))) if id == &s_alpha.id),
        "expected Exit::AttachReadOnly for alpha, got {:?}",
        ro_app.exit()
    );
    let ro_cmd = server
        .client
        .attach_command_readonly(&Target::Session(s_alpha.id.clone()));
    let ro_args: Vec<_> = ro_cmd.get_args().collect();
    assert!(
        ro_args
            .windows(4)
            .any(|w| w == ["attach-session", "-r", "-t", s_alpha.id.as_str()]),
        "{ro_args:?}"
    );

    // 5. `/kill-server` requires strict `yes` confirmation.
    text(&mut app, "/kill-server");
    keys(&mut app, &[Enter, Enter]);
    assert!(
        server.client.server_running().expect("still running"),
        "empty Enter must cancel /kill-server"
    );
    text(&mut app, "/kill-server");
    keys(&mut app, &[Enter]);
    text(&mut app, "yes");
    keys(&mut app, &[Enter]);
    assert!(
        !server.client.server_running().expect("server stopped"),
        "confirmed /kill-server must stop the private tmux server"
    );
}

#[test]
fn controller_comfort_tmux_conf_and_snapshot_save_restore_e2e() {
    use KeyCode::{Char, Down, Enter, Esc};
    use justmux::{
        config::Config,
        snapshot,
        tmux_conf::{COMFORT_BLOCK_END, COMFORT_BLOCK_START},
    };

    let server = PrivateServer::new();
    let s1 = server.client.new_session("snap-s1", cwd()).expect("s1");
    let w0 = server.client.list_windows(Some(&s1.id)).expect("w0")[0].clone();
    let p0 = server.client.list_panes(Some(&w0.id)).expect("p0")[0].clone();
    let _p1 = server
        .client
        .split_pane(&p0.id, SplitDirection::SideBySide)
        .expect("split");
    server
        .client
        .set_synchronize_panes(&w0.id, true)
        .expect("sync on");
    server
        .client
        .rename_pane_title(&p0.id, "api-pane")
        .expect("title");

    // 1. Opt-in `~/.tmux.conf` comfort settings (rows 11..=15) + live reload (row 16) in tempdir.
    let temp = tempfile::tempdir().expect("tempdir");
    let conf_path = temp.path().join("tmux.conf");
    let user_prefix = "# User personal tmux config\nset -g status-position top\n";
    std::fs::write(&conf_path, user_prefix).expect("write conf");

    let mut app = App::with_config(
        &server.client,
        AppOptions::default(),
        Config::default(),
        Some(temp.path().join("config.toml")),
    )
    .with_tmux_conf(Some(conf_path.clone()), None);

    text(&mut app, "/config");
    keys(&mut app, &[Enter]);
    // Move from row 0 down to row 11 (`tmux_mouse`), then enable rows 11..=15.
    for _ in 0..11 {
        keys(&mut app, &[Down]);
    }
    for row_idx in 11..=15 {
        keys(&mut app, &[Char(' ')]);
        if row_idx < 15 {
            keys(&mut app, &[Down]);
        }
    }
    assert!(app.config().tmux_mouse);
    assert!(app.config().tmux_history_limit > 0);
    assert!(app.config().tmux_base_index_one);
    assert!(app.config().tmux_renumber_windows);
    assert!(app.config().tmux_zero_escape_time);

    let conf_body = std::fs::read_to_string(&conf_path).expect("read conf");
    assert!(conf_body.starts_with(user_prefix), "{conf_body}");
    assert!(conf_body.contains(COMFORT_BLOCK_START), "{conf_body}");
    assert!(conf_body.contains("set -g mouse on"), "{conf_body}");
    assert!(conf_body.contains("set -g base-index 1"), "{conf_body}");
    assert!(
        conf_body.contains("set -gw pane-base-index 1"),
        "{conf_body}"
    );
    assert!(
        conf_body.contains("set -g renumber-windows on"),
        "{conf_body}"
    );
    assert!(conf_body.contains("set -sg escape-time 0"), "{conf_body}");
    assert!(conf_body.contains(COMFORT_BLOCK_END), "{conf_body}");

    // Row 16: `[Reload ~/.tmux.conf]` runs `source-file` on the private server cleanly.
    keys(&mut app, &[Down, Enter]);
    assert_eq!(
        String::from_utf8_lossy(&server.tmux(&["show-options", "-gv", "mouse"]).stdout).trim(),
        "on"
    );
    assert_eq!(
        String::from_utf8_lossy(&server.tmux(&["show-options", "-sgv", "escape-time"]).stdout)
            .trim(),
        "0"
    );
    keys(&mut app, &[Esc]);

    // 2. Workspace snapshot capture, TOML save/list/load, destroy server, and full restore!
    let snap_dir = temp.path().join("snapshots");
    let captured =
        snapshot::capture_snapshot(&server.client, "dev-stack", None).expect("capture_snapshot");
    assert_eq!(captured.session_count(), 1);
    assert_eq!(captured.pane_count(), 2);
    snapshot::save_snapshot(&snap_dir, &captured).expect("save_snapshot");
    let listed = snapshot::list_snapshots(&snap_dir).expect("list_snapshots");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "dev-stack");

    // Kill the server completely, then restore the snapshot onto a fresh server on the same socket.
    server.client.kill_server().expect("kill_server");
    assert!(!server.client.server_running().expect("stopped"));
    let loaded = snapshot::load_snapshot(&snap_dir, "dev-stack").expect("load_snapshot");
    let report = snapshot::restore_snapshot(&server.client, &loaded).expect("restore_snapshot");
    assert_eq!(report.sessions_created, 1);
    assert_eq!(report.windows_created, 1);
    assert_eq!(report.panes_created, 2);

    let restored_sessions = server.client.list_sessions().expect("restored sessions");
    assert_eq!(restored_sessions.len(), 1);
    assert_eq!(restored_sessions[0].name, "snap-s1");
    let restored_wins = server
        .client
        .list_windows(Some(&restored_sessions[0].id))
        .expect("restored wins");
    assert_eq!(restored_wins.len(), 1);
    assert!(restored_wins[0].synchronized);
    let restored_panes = server
        .client
        .list_panes(Some(&restored_wins[0].id))
        .expect("restored panes");
    assert_eq!(restored_panes.len(), 2);
    assert!(restored_panes.iter().any(|p| p.title == "api-pane"));
}

#[test]
fn controller_dual_form_new_window_multipane_preview_cycle_and_search_e2e() {
    use KeyCode::{Char, Down, Enter, Esc, Tab};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let custom_dir = tempfile::tempdir().expect("custom dir");
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "preview-hub",
        "-n",
        "dual",
        "-x",
        "80",
        "-y",
        "12",
        "sh -c 'echo PANE_ALPHA_TOKEN; sleep 30'",
    ]);
    assert!(out.status.success());
    let session = server.client.list_sessions().expect("session")[0].clone();
    let win = server.client.list_windows(Some(&session.id)).expect("win")[0].clone();
    let p0 = server.client.list_panes(Some(&win.id)).expect("p0")[0].clone();
    let out_split = server.tmux(&[
        "split-window",
        "-d",
        "-h",
        "-t",
        p0.id.as_str(),
        "sh -c 'echo PANE_BETA_TOKEN; sleep 30'",
    ]);
    assert!(out_split.status.success());
    let panes = server.client.list_panes(Some(&win.id)).expect("2 panes");
    assert_eq!(panes.len(), 2);
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(panes[0].id.clone()),
        "PANE_ALPHA_TOKEN",
    );
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(panes[1].id.clone()),
        "PANE_BETA_TOKEN",
    );

    // 1. Multi-pane Home preview cycling (`[`/`]`) and interactive preview search (`f`).
    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(110, 30)).expect("term");
    term.draw(|f| app.render(f)).expect("draw initial");
    let first_screen = term.backend().to_string();
    assert!(
        first_screen.contains("PANE_ALPHA_TOKEN"),
        "expected pane 1 preview:\n{first_screen}"
    );
    assert!(
        first_screen.contains("1/2"),
        "expected 1/2 pane switcher badge:\n{first_screen}"
    );

    keys(&mut app, &[Char(']')]);
    term.draw(|f| app.render(f)).expect("draw pane 2");
    let second_screen = term.backend().to_string();
    assert!(
        second_screen.contains("PANE_BETA_TOKEN"),
        "expected pane 2 preview after `]`:\n{second_screen}"
    );
    assert!(
        second_screen.contains("2/2"),
        "expected 2/2 pane switcher badge:\n{second_screen}"
    );

    // Open preview search (`f`), search for `BETA`, confirm match counter, Esc clears.
    keys(&mut app, &[Char('f')]);
    text(&mut app, "BETA");
    term.draw(|f| app.render(f)).expect("draw search");
    let search_screen = term.backend().to_string();
    assert!(
        search_screen.contains("BETA") && search_screen.contains("1/1"),
        "expected active search match counter:\n{search_screen}"
    );
    keys(&mut app, &[Esc]);

    // 2. DualForm New Window: Tab switches to optional CWD and Alt+Enter creates window in custom_dir!
    // Focus `+ New window` row under `preview-hub` (`Down`, `Enter`).
    keys(&mut app, &[Down, Enter]);
    text(&mut app, "custom-cwd-win");
    keys(&mut app, &[Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, custom_dir.path().to_str().expect("utf8 path"));
    key_mod(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app.exit().is_none(), "Alt+Enter creates & stays on Home");

    let wins = server
        .client
        .list_windows(Some(&session.id))
        .expect("wins after dual form");
    let created = wins
        .iter()
        .find(|w| w.name == "custom-cwd-win")
        .expect("created window");
    let created_pane = server
        .client
        .list_panes(Some(&created.id))
        .expect("created pane")[0]
        .clone();
    let expected_canon = custom_dir
        .path()
        .canonicalize()
        .unwrap_or_else(|_| custom_dir.path().to_path_buf());
    let actual_canon = Path::new(&created_pane.current_path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(&created_pane.current_path));
    assert_eq!(actual_canon, expected_canon);

    // 3. Invalid working directory must be rejected by both `new_window` and DualForm New Window.
    let missing_dir = custom_dir.path().join("does-not-exist-subdir");
    assert!(matches!(
        server
            .client
            .new_window(&session.id, Some("bad-cwd"), Some(&missing_dir)),
        Err(TmuxError::DirectoryNotFound(_))
    ));
    // Open DualForm from Home Matrix (`+ New window`) with invalid dir -> stays in form with error.
    let mut bad_app = App::new(&server.client, AppOptions::default());
    // Move down past the 2 existing windows to `+ New window`.
    keys(&mut bad_app, &[Down, Down, Enter]);
    text(&mut bad_app, "bad-cwd-win");
    keys(&mut bad_app, &[Tab]);
    key_mod(&mut bad_app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut bad_app, missing_dir.to_str().expect("utf8 missing"));
    keys(&mut bad_app, &[Enter]);
    assert!(
        bad_app.status().contains("directory does not exist"),
        "expected missing dir error in status: {}",
        bad_app.status()
    );
    assert!(
        !server
            .client
            .list_windows(Some(&session.id))
            .expect("wins unchanged")
            .iter()
            .any(|w| w.name == "bad-cwd-win")
    );
}

#[test]
fn circular_wrap_around_in_pane_actions_layout_clients_buffers_snapshots_and_target_pick() {
    use KeyCode::{BackTab, Down, Enter, Esc, Tab, Up};
    use crossterm::event::{MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Mutex;

    static SNAPSHOT_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }
    impl EnvVarGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: serialized by SNAPSHOT_ENV_LOCK across tests in this binary.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }
    }
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: serialized by SNAPSHOT_ENV_LOCK across tests in this binary.
            unsafe {
                if let Some(prev) = &self.prev {
                    std::env::set_var(self.key, prev);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    struct ControlClient(std::process::Child);
    impl Drop for ControlClient {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let wheel = |up: bool, col: u16, row: u16| {
        Event::Mouse(MouseEvent {
            kind: if up {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            },
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    let draw = |app: &mut App<'_>| -> String {
        let mut term = Terminal::new(TestBackend::new(112, 28)).expect("term");
        term.draw(|f| app.render(f)).expect("draw");
        term.backend().to_string()
    };

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "wrap-s1",
        "-n",
        "main",
        "-x",
        "100",
        "-y",
        "30",
        "sh -c 'sleep 30'",
    ]);
    assert!(out.status.success());
    let s1 = server.client.list_sessions().expect("s1")[0].clone();
    let w0 = server.client.list_windows(Some(&s1.id)).expect("w0")[0].clone();
    let p0 = server.client.list_panes(Some(&w0.id)).expect("p0")[0].clone();
    let _p1 = server
        .client
        .split_pane(&p0.id, SplitDirection::SideBySide)
        .expect("p1");
    let _s2 = server.client.new_session("wrap-s2", cwd()).expect("s2");

    // 1. `/pane` picker + Resource Actions (`View::Actions`) wrap-around.
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/pane");
    keys(&mut app, &[Enter]);
    let pane_top = draw(&mut app);
    keys(&mut app, &[Up]);
    let pane_bottom = draw(&mut app);
    assert_ne!(pane_top, pane_bottom, "Up in /pane picker should wrap");
    keys(&mut app, &[Down]);
    assert_eq!(draw(&mut app), pane_top, "Down from bottom wraps to top");
    app.handle_event(wheel(true, 10, 16));
    assert_eq!(draw(&mut app), pane_bottom);
    app.handle_event(wheel(false, 10, 16));
    assert_eq!(draw(&mut app), pane_top);

    // Open Pane actions -> `Up` once wraps straight to last action (`Close pane`)!
    keys(&mut app, &[Enter, Up, Enter]);
    let confirm_screen = draw(&mut app);
    assert!(
        confirm_screen.contains("yes") || confirm_screen.contains("Confirm"),
        "Up once in Pane Actions should wrap to Close pane confirm:\n{confirm_screen}"
    );
    // Cancel confirmation (returns to `/pane` picker).
    keys(&mut app, &[Esc]);

    // Also test mouse wheel wrap in Resource Actions menu (index 0 -> last -> index 0).
    keys(&mut app, &[Enter]);
    app.handle_event(wheel(true, 10, 16));
    keys(&mut app, &[Enter]);
    let wheel_confirm = draw(&mut app);
    assert!(
        wheel_confirm.contains("yes") || wheel_confirm.contains("Confirm"),
        "Wheel Up in Actions should wrap to Close pane confirm:\n{wheel_confirm}"
    );
    keys(&mut app, &[Esc, Esc]);

    // 2. `/layout` Studio wraps presets on `Tab`/`BackTab` and mouse wheel.
    text(&mut app, "/layout");
    keys(&mut app, &[Enter]);
    let lay_0 = draw(&mut app);
    keys(&mut app, &[BackTab]);
    let lay_last = draw(&mut app);
    assert_ne!(
        lay_0, lay_last,
        "BackTab from preset 0 should wrap to Tiled"
    );
    keys(&mut app, &[Tab]);
    assert_eq!(draw(&mut app), lay_0, "Tab from Tiled wraps to preset 0");
    app.handle_event(wheel(true, 20, 10));
    assert_eq!(
        draw(&mut app),
        lay_last,
        "Wheel Up wraps layout preset 0->4"
    );
    app.handle_event(wheel(false, 20, 10));
    assert_eq!(draw(&mut app), lay_0, "Wheel Down wraps layout preset 4->0");
    keys(&mut app, &[Esc]);

    // 3. `/clients` manager wraps across 2 attached clients (keys + wheel).
    let spawn = |readonly: bool| -> ControlClient {
        let mut cmd = Command::new("tmux");
        cmd.arg("-L")
            .arg(&server.socket)
            .args(["-f", "/dev/null", "-C", "attach-session"]);
        if readonly {
            cmd.arg("-r");
        }
        cmd.args(["-t", s1.id.as_str()])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        ControlClient(cmd.spawn().expect("control client"))
    };
    let _c1 = spawn(false);
    let _c2 = spawn(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if server.client.list_clients().expect("clients").len() == 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "clients did not attach"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    text(&mut app, "/clients");
    keys(&mut app, &[Enter]);
    let cli_0 = draw(&mut app);
    keys(&mut app, &[Up]);
    let cli_1 = draw(&mut app);
    assert_ne!(cli_0, cli_1, "Up in /clients should wrap 0 -> 1");
    keys(&mut app, &[Down]);
    assert_eq!(draw(&mut app), cli_0, "Down in /clients should wrap 1 -> 0");
    app.handle_event(wheel(true, 10, 12));
    assert_eq!(draw(&mut app), cli_1);
    app.handle_event(wheel(false, 10, 12));
    assert_eq!(draw(&mut app), cli_0);
    keys(&mut app, &[Esc]);

    // 4. `/buffers` list + `TargetPick` wrap (keys + wheel).
    server
        .client
        .set_buffer("FIRST_BUF_UNIQUE_ALPHA")
        .expect("buf1");
    server
        .client
        .set_buffer("SECOND_BUF_UNIQUE_BETA")
        .expect("buf2");
    text(&mut app, "/buffers");
    keys(&mut app, &[Enter]);
    let buf_0 = draw(&mut app);
    assert!(buf_0.contains("SECOND_BUF_UNIQUE_BETA"), "{buf_0}");
    keys(&mut app, &[Up]);
    let buf_1 = draw(&mut app);
    assert!(
        buf_1.contains("FIRST_BUF_UNIQUE_ALPHA"),
        "Up in /buffers should wrap to older buffer:\n{buf_1}"
    );
    keys(&mut app, &[Down]);
    assert!(draw(&mut app).contains("SECOND_BUF_UNIQUE_BETA"));
    app.handle_event(wheel(true, 8, 16));
    assert!(draw(&mut app).contains("FIRST_BUF_UNIQUE_ALPHA"));
    app.handle_event(wheel(false, 8, 16));
    assert!(draw(&mut app).contains("SECOND_BUF_UNIQUE_BETA"));

    // Press Enter on `/buffers` -> opens `TargetPick` (choosing target pane); verify wrap!
    keys(&mut app, &[Enter]);
    let selected_row_line = |screen: &str| -> String {
        screen
            .lines()
            .find(|line| line.contains("› wrap-s"))
            .unwrap_or("")
            .to_owned()
    };
    let tp_0 = selected_row_line(&draw(&mut app));
    assert!(!tp_0.is_empty(), "expected selected TargetPick row");
    keys(&mut app, &[Up]);
    let tp_last = selected_row_line(&draw(&mut app));
    assert!(!tp_last.is_empty());
    assert_ne!(tp_0, tp_last, "Up in TargetPick should wrap to last pane");
    keys(&mut app, &[Down]);
    assert_eq!(
        selected_row_line(&draw(&mut app)),
        tp_0,
        "Down in TargetPick wraps back to 0"
    );
    app.handle_event(wheel(true, 10, 16));
    assert_eq!(selected_row_line(&draw(&mut app)), tp_last);
    app.handle_event(wheel(false, 10, 16));
    assert_eq!(selected_row_line(&draw(&mut app)), tp_0);
    keys(&mut app, &[Esc, Esc]);

    // 5. `/snapshot` manager list wrap (keys + wheel) using isolated tempdir.
    let _guard_lock = SNAPSHOT_ENV_LOCK.lock().expect("env lock");
    let snap_dir = tempfile::tempdir().expect("snap dir");
    let _env_guard = EnvVarGuard::set(justmux::snapshot::SNAPSHOT_DIR_ENV, snap_dir.path());
    let snap1 = justmux::snapshot::capture_snapshot(&server.client, "alpha-snap", Some(&s1.id))
        .expect("cap1");
    justmux::snapshot::save_snapshot(snap_dir.path(), &snap1).expect("save1");
    let snap2 =
        justmux::snapshot::capture_snapshot(&server.client, "zeta-snap", None).expect("cap2");
    justmux::snapshot::save_snapshot(snap_dir.path(), &snap2).expect("save2");

    text(&mut app, "/snapshot");
    keys(&mut app, &[Enter]);
    let snap_0 = draw(&mut app);
    keys(&mut app, &[Up]);
    let snap_1 = draw(&mut app);
    assert_ne!(snap_0, snap_1, "Up in /snapshot should wrap 0 -> 1");
    keys(&mut app, &[Down]);
    assert_eq!(draw(&mut app), snap_0, "Down in /snapshot wraps 1 -> 0");
    app.handle_event(wheel(true, 8, 16));
    assert_eq!(draw(&mut app), snap_1);
    app.handle_event(wheel(false, 8, 16));
    assert_eq!(draw(&mut app), snap_0);
    keys(&mut app, &[Esc]);
}

fn find_rendered_text_from_row(
    term: &ratatui::Terminal<ratatui::backend::TestBackend>,
    needle: &str,
    min_y: u16,
) -> Option<(u16, u16)> {
    let buf = term.backend().buffer();
    let area = buf.area;
    for y in min_y..area.height {
        let mut x = 0_u16;
        let mut row_str = String::new();
        let mut col_map: Vec<(usize, u16)> = Vec::new();
        while x < area.width {
            let sym = buf[(x, y)].symbol();
            col_map.push((row_str.len(), x));
            row_str.push_str(sym);
            let step = unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            x = x.saturating_add(step);
        }
        if let Some(byte_pos) = row_str.find(needle)
            && let Some((_, col)) = col_map.iter().rev().find(|(b, _)| *b <= byte_pos)
        {
            return Some((*col, y));
        }
    }
    None
}

fn find_rendered_text(
    term: &ratatui::Terminal<ratatui::backend::TestBackend>,
    needle: &str,
) -> Option<(u16, u16)> {
    find_rendered_text_from_row(term, needle, 0)
}

fn mouse_ev(kind: crossterm::event::MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn desktop_menubar_dirty_hover_double_click_context_menu_middle_click_and_f2_rename_e2e() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "alpha",
        "-n",
        "main",
        "-x",
        "100",
        "-y",
        "30",
        "sh -c 'sleep 30'",
    ]);
    assert!(out.status.success());
    let s_alpha = server.client.list_sessions().expect("alpha")[0].clone();
    let _w_logs = server
        .client
        .new_window(&s_alpha.id, Some("logs"), Some(cwd()))
        .expect("logs");
    let _s_beta = server.client.new_session("beta", cwd()).expect("beta");

    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 32)).expect("term");
    term.draw(|f| app.render(f)).expect("initial draw");

    // 1. Dirty-check `MouseEventKind::Moved`: empty space returns false (0 redraws),
    // entering a menubar control returns true, hovering the same control returns false.
    assert!(
        !app.handle_event(mouse_ev(MouseEventKind::Moved, 0, 0)),
        "Moved over empty corner must not trigger redraw"
    );
    let (lang_x, lang_y) = find_rendered_text(&term, "[中/EN]").expect("find [中/EN] menubar pill");
    assert!(
        app.handle_event(mouse_ev(MouseEventKind::Moved, lang_x + 1, lang_y)),
        "Moved into menubar button must mark dirty"
    );
    assert!(
        !app.handle_event(mouse_ev(MouseEventKind::Moved, lang_x + 1, lang_y)),
        "Repeated Moved on same menubar button must stay clean"
    );

    // 2. Top IDE Menubar clicks: 1-click `[中/EN]` toggles language live, and tabs switch views.
    assert!(!app.theme().language.is_zh());
    assert!(app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        lang_x + 1,
        lang_y
    )));
    assert!(app.theme().language.is_zh());
    term.draw(|f| app.render(f)).expect("zh draw");
    let (lang_x2, lang_y2) = find_rendered_text(&term, "[EN/中]")
        .or_else(|| find_rendered_text(&term, "[中/EN]"))
        .expect("find language toggle pill in ZH");
    assert!(app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        lang_x2 + 1,
        lang_y2
    )));
    assert!(!app.theme().language.is_zh());
    term.draw(|f| app.render(f)).expect("en draw");

    let (expose_x, expose_y) = find_rendered_text(&term, "Expose").expect("Expose tab");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        expose_x,
        expose_y,
    ));
    term.draw(|f| app.render(f)).expect("expose draw");
    let expose_screen = term.backend().to_string();
    assert!(
        expose_screen.contains("EXPOSE") || expose_screen.contains("Mission Control"),
        "{expose_screen}"
    );

    let (layout_x, layout_y) = find_rendered_text(&term, "Layout").expect("Layout tab");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        layout_x,
        layout_y,
    ));
    term.draw(|f| app.render(f)).expect("layout draw");
    assert!(term.backend().to_string().contains("LAYOUT"));

    let (split_x, split_y) = find_rendered_text(&term, "Split").expect("Split tab");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        split_x,
        split_y,
    ));
    term.draw(|f| app.render(f)).expect("split draw");
    assert!(term.backend().to_string().contains("SPLIT"));

    let (studio_x, studio_y) = find_rendered_text(&term, "Studio").expect("Studio tab");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        studio_x,
        studio_y,
    ));
    term.draw(|f| app.render(f)).expect("studio draw");

    // 3. `F2` inline rename pre-fills current window name and renames real tmux window on Enter.
    let matrix_min_y = find_rendered_text(&term, "[f Search]")
        .expect("preview bottom")
        .1
        + 2;
    let (logs_x, logs_y) =
        find_rendered_text_from_row(&term, "logs", matrix_min_y).expect("logs matrix cell");
    // Single-click selects `logs` without entering tmux.
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        logs_x,
        logs_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        logs_x,
        logs_y,
    ));
    assert!(app.exit().is_none(), "single click must only focus");

    keys(&mut app, &[KeyCode::F(2), KeyCode::Right]);
    text(&mut app, "-live");
    keys(&mut app, &[KeyCode::Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&s_alpha.id))
            .expect("renamed win")
            .iter()
            .any(|w| w.name == "logs-live"),
        "F2 inline rename should rename real tmux window to logs-live"
    );

    // 4. Right-click context menu open, wrap, and Esc dismiss; Middle-click opens Kill confirm.
    term.draw(|f| app.render(f)).expect("draw before rclick");
    let (ren_x, ren_y) = find_rendered_text_from_row(&term, "logs-live", matrix_min_y)
        .expect("logs-live matrix cell");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Right),
        ren_x,
        ren_y,
    ));
    term.draw(|f| app.render(f)).expect("context menu draw");
    let ctx_screen = term.backend().to_string();
    assert!(
        ctx_screen.contains("Rename") && ctx_screen.contains("Close"),
        "right-click should open floating context menu:\n{ctx_screen}"
    );
    keys(&mut app, &[KeyCode::Up, KeyCode::Esc]);

    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Middle),
        ren_x,
        ren_y,
    ));
    term.draw(|f| app.render(f)).expect("middle-click confirm");
    assert!(
        term.backend().to_string().contains("CONFIRM KILL"),
        "middle-click on window must open safe yes Kill confirmation"
    );
    keys(&mut app, &[KeyCode::Esc]);

    // 5. True `<= 350ms` Double-Click enters the target window in tmux.
    let mut dbl_app = App::new(&server.client, AppOptions::default());
    term.draw(|f| dbl_app.render(f)).expect("dbl draw");
    let (target_x, target_y) = find_rendered_text_from_row(&term, "logs-live", matrix_min_y)
        .expect("logs-live matrix cell");
    dbl_app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        target_x,
        target_y,
    ));
    dbl_app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        target_x,
        target_y,
    ));
    assert!(dbl_app.exit().is_none(), "first click selects");
    dbl_app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        target_x,
        target_y,
    ));
    assert!(
        matches!(dbl_app.exit(), Some(Exit::Attach(Target::Window(sid, _))) if sid == &s_alpha.id),
        "second click within 350ms must attach window, got {:?}",
        dbl_app.exit()
    );
}

#[test]
fn desktop_drag_drop_move_swap_edge_dock_sash_scrollbar_and_drag_select_copy_e2e() {
    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "alpha",
        "-n",
        "winA",
        "-x",
        "120",
        "-y",
        "36",
        "sh -c 'i=1; while [ $i -le 30 ]; do printf \"SCROLL_LINE_%02d\\n\" $i; i=$((i+1)); done; echo ERROR: sample failure; sleep 30'",
    ]);
    assert!(out.status.success());
    let s_alpha = server.client.list_sessions().expect("alpha")[0].clone();
    let w_a = server.client.list_windows(Some(&s_alpha.id)).expect("wA")[0].clone();
    let p_a = server.client.list_panes(Some(&w_a.id)).expect("pA")[0].clone();
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(p_a.id.clone()),
        "ERROR: sample failure",
    );

    let w_b = server
        .client
        .new_window(&s_alpha.id, Some("winB"), Some(cwd()))
        .expect("winB");
    let w_dock_left = server
        .client
        .new_window(&s_alpha.id, Some("dockL"), Some(cwd()))
        .expect("dockL");
    let p_dock_left = server.client.list_panes(Some(&w_dock_left.id)).expect("pL")[0].clone();
    let w_dock_bottom = server
        .client
        .new_window(&s_alpha.id, Some("dockB"), Some(cwd()))
        .expect("dockB");
    let p_dock_bottom = server
        .client
        .list_panes(Some(&w_dock_bottom.id))
        .expect("pB")[0]
        .clone();
    let s_beta = server.client.new_session("beta", cwd()).expect("beta");

    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 34)).expect("term");
    term.draw(|f| app.render(f)).expect("initial draw");

    // 1. Drag Home Preview/Matrix Sash up (shrink preview) then down (grow preview).
    let preview_bottom_before = find_rendered_text(&term, "[f Search]")
        .expect("search pill")
        .1;
    let (sash_x, sash_y) =
        find_rendered_text_from_row(&term, "──────────────────────", preview_bottom_before + 1)
            .expect("find horizontal preview/matrix sash");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        sash_x + 2,
        sash_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        sash_x + 2,
        sash_y.saturating_sub(5),
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        sash_x + 2,
        sash_y.saturating_sub(5),
    ));
    term.draw(|f| app.render(f)).expect("after sash drag up");
    let preview_bottom_shrunk = find_rendered_text(&term, "[f Search]")
        .expect("search pill after up")
        .1;
    assert!(
        preview_bottom_shrunk < preview_bottom_before,
        "dragging Home sash up should shrink preview card ({preview_bottom_before} -> {preview_bottom_shrunk})"
    );

    let (sash_x2, sash_y2) =
        find_rendered_text_from_row(&term, "──────────────────────", preview_bottom_shrunk + 1)
            .expect("find sash after shrink");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        sash_x2 + 2,
        sash_y2,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        sash_x2 + 2,
        sash_y2 + 5,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        sash_x2 + 2,
        sash_y2 + 5,
    ));
    term.draw(|f| app.render(f)).expect("after sash drag down");
    let preview_bottom_regrown = find_rendered_text(&term, "[f Search]")
        .expect("search pill after down")
        .1;
    assert!(
        preview_bottom_regrown > preview_bottom_shrunk,
        "dragging Home sash down should grow preview card ({preview_bottom_shrunk} -> {preview_bottom_regrown})"
    );

    // 2. Preview Scrollbar click near top scrolls up into scrollback (`SCROLL_LINE_01`).
    let (prev_top_x, prev_top_y) =
        find_rendered_text(&term, "Window Preview").expect("preview hdr");
    let (pipe_x, prev_bot_y) = find_rendered_text(&term, "[Pipe]").expect("preview footer");
    let scrollbar_col = pipe_x + 5; // right interior border/track column of preview card
    let _ = (prev_top_x, prev_bot_y);
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        scrollbar_col,
        prev_top_y + 2,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        scrollbar_col,
        prev_top_y + 2,
    ));
    // Drag-to-select inside preview text interior copies plain text cleanly.
    term.draw(|f| app.render(f))
        .expect("draw before drag-select");
    if let Some((err_x, err_y)) = find_rendered_text(&term, "SCROLL_LINE_") {
        app.handle_event(mouse_ev(
            MouseEventKind::Down(MouseButton::Left),
            err_x,
            err_y,
        ));
        app.handle_event(mouse_ev(
            MouseEventKind::Drag(MouseButton::Left),
            err_x + 12,
            err_y,
        ));
        app.handle_event(mouse_ev(
            MouseEventKind::Up(MouseButton::Left),
            err_x + 12,
            err_y,
        ));
        assert!(
            !app.status().is_empty(),
            "drag-to-select copy should report status/toast feedback"
        );
    }

    // 3. Drag `winB` onto `winA` in same session -> `swap-window`.
    term.draw(|f| app.render(f)).expect("before swap drag");
    let matrix_y = find_rendered_text(&term, "[f Search]")
        .expect("preview bottom")
        .1
        + 2;
    let wb_idx_before = server
        .client
        .list_windows(Some(&s_alpha.id))
        .expect("wins")
        .into_iter()
        .find(|w| w.id == w_b.id)
        .expect("w_b")
        .index;
    let (wb_x, wb_y) = find_rendered_text_from_row(&term, "winB", matrix_y).expect("winB cell");
    let (wa_x, wa_y) = find_rendered_text_from_row(&term, "winA", matrix_y).expect("winA cell");
    // Move >= 2 cells horizontally+vertically to cross the intentional drag threshold.
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        wb_x,
        wb_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        wa_x + 3,
        wa_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        wa_x + 3,
        wa_y,
    ));
    let wb_idx_after = server
        .client
        .list_windows(Some(&s_alpha.id))
        .expect("wins after swap")
        .into_iter()
        .find(|w| w.id == w_b.id)
        .expect("w_b")
        .index;
    assert_ne!(
        wb_idx_before, wb_idx_after,
        "drag onto sibling window must swap-window"
    );

    // 4. Drag `winB` onto `beta` session header -> `move-window`.
    term.draw(|f| app.render(f)).expect("before move drag");
    let (wb_x, wb_y) = find_rendered_text_from_row(&term, "winB", matrix_y).expect("winB cell");
    let (beta_x, beta_y) =
        find_rendered_text_from_row(&term, "beta", matrix_y).expect("beta header");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        wb_x,
        wb_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        beta_x,
        beta_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        beta_x,
        beta_y,
    ));
    assert!(
        server
            .client
            .list_windows(Some(&s_beta.id))
            .expect("beta wins")
            .iter()
            .any(|w| w.id == w_b.id),
        "dragging winB onto beta session header must move-window"
    );

    // 5. VS Code Edge Dock Splits: drag `dockL` to Preview LEFT edge (`before=true`)
    // and `dockB` to Preview BOTTOM edge (`before=false`), verifying real pane coordinates!
    // Focus `winA` first so Preview shows `winA`.
    term.draw(|f| app.render(f)).expect("before focus winA");
    let (wa_x, wa_y) = find_rendered_text_from_row(&term, "winA", matrix_y).expect("winA cell");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        wa_x,
        wa_y,
    ));
    app.handle_event(mouse_ev(MouseEventKind::Up(MouseButton::Left), wa_x, wa_y));
    term.draw(|f| app.render(f)).expect("before dockL");

    let (dl_x, dl_y) = find_rendered_text_from_row(&term, "dockL", matrix_y).expect("dockL cell");
    let (hdr_x, hdr_y) = find_rendered_text(&term, "Window Preview").expect("preview hdr");
    let (_, bot_y) = find_rendered_text(&term, "[f Search]").expect("preview bot");
    let mid_y = (hdr_y + bot_y) / 2;
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        dl_x,
        dl_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        hdr_x,
        mid_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        hdr_x,
        mid_y,
    ));
    let panes_after_left = server.client.list_panes(Some(&w_a.id)).expect("panes left");
    let pl = panes_after_left
        .iter()
        .find(|p| p.id == p_dock_left.id)
        .expect("dockL joined into winA");
    let pa = panes_after_left
        .iter()
        .find(|p| p.id == p_a.id)
        .expect("pA still in winA");
    assert!(
        pl.left < pa.left,
        "Left edge dock must place dockL to the left of winA: pl={pl:?} pa={pa:?}"
    );

    // Drag `dockB` to Preview BOTTOM edge -> places dockB below active pane (`top > pa.top`).
    term.draw(|f| app.render(f)).expect("before dockB");
    let (db_x, db_y) = find_rendered_text_from_row(&term, "dockB", matrix_y).expect("dockB cell");
    let (hdr_x, _) = find_rendered_text(&term, "Window Preview").expect("preview hdr");
    let (_, bot_y) = find_rendered_text(&term, "[f Search]").expect("preview bot");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        db_x,
        db_y,
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Drag(MouseButton::Left),
        hdr_x + 35,
        bot_y.saturating_sub(1),
    ));
    app.handle_event(mouse_ev(
        MouseEventKind::Up(MouseButton::Left),
        hdr_x + 35,
        bot_y.saturating_sub(1),
    ));
    let panes_after_bottom = server
        .client
        .list_panes(Some(&w_a.id))
        .expect("panes bottom");
    assert!(
        panes_after_bottom
            .iter()
            .any(|p| p.id == p_dock_bottom.id && p.top > 0),
        "Bottom edge dock must place dockB below (top > 0): {panes_after_bottom:?}"
    );
}

#[test]
fn desktop_expose_mission_control_and_live_pip_interactive_type_through_e2e() {
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let out = server.tmux(&[
        "new-session",
        "-d",
        "-s",
        "mission",
        "-n",
        "sh1",
        "-x",
        "80",
        "-y",
        "18",
        "sh",
    ]);
    assert!(out.status.success());
    let sess = server.client.list_sessions().expect("session")[0].clone();
    let w1 = server.client.list_windows(Some(&sess.id)).expect("w1")[0].clone();
    let p1 = server.client.list_panes(Some(&w1.id)).expect("p1")[0].clone();
    let _w2 = server
        .client
        .new_window(&sess.id, Some("sh2"), Some(cwd()))
        .expect("w2");

    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(120, 32)).expect("term");

    // 1. Open Mission Control `/expose` via `Ctrl+G`, verify live tiles & wrap navigation.
    app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
    )));
    term.draw(|f| app.render(f)).expect("expose draw");
    let expose_screen = term.backend().to_string();
    assert!(
        expose_screen.contains("sh1") && expose_screen.contains("sh2"),
        "Expose should render both windows:\n{expose_screen}"
    );
    // Circular navigation across tiles (`Left` wraps 0 -> 1, `Right` wraps 1 -> 0).
    keys(&mut app, &[KeyCode::Left, KeyCode::Right]);

    // 2. Enter Live PIP (`i`) on tile 0 (`sh1`), type a real shell command + Enter!
    keys(&mut app, &[KeyCode::Char('i')]);
    assert!(app.is_live_pip_active(), "expected Live PIP active");
    term.draw(|f| app.render(f)).expect("pip draw");
    assert!(
        term.backend().to_string().contains("LIVE INPUT"),
        "expected LIVE INPUT banner"
    );

    text(&mut app, "echo LIVE_PIP_E2E_OK; echo PIP_ZH_中文");
    keys(&mut app, &[KeyCode::Enter]);
    let captured = wait_for_pane_text(&server.client, &Target::Pane(p1.id.clone()), "PIP_ZH_中文");
    assert!(captured.iter().any(|l| l.contains("LIVE_PIP_E2E_OK")));

    // 3. `Event::Paste` injects `sleep 30\n`, then `Ctrl+C` forwards to pane (NOT quitting JustMux!).
    assert!(app.handle_event(Event::Paste("sleep 30\n".into())));
    std::thread::sleep(std::time::Duration::from_millis(80));
    assert!(app.handle_event(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    ))));
    assert!(
        app.exit().is_none(),
        "Ctrl+C in Live PIP must interrupt remote pane, not quit JustMux"
    );
    assert!(app.is_live_pip_active());

    // Confirm shell recovered after Ctrl+C by pasting another command.
    assert!(app.handle_event(Event::Paste("echo AFTER_CTRL_C_PASTE\n".into())));
    let _ = wait_for_pane_text(
        &server.client,
        &Target::Pane(p1.id.clone()),
        "AFTER_CTRL_C_PASTE",
    );

    // 4. `Esc` exits Live PIP cleanly without exiting JustMux.
    keys(&mut app, &[KeyCode::Esc]);
    assert!(!app.is_live_pip_active());
    assert!(app.exit().is_none());
}

#[test]
fn dual_form_inherits_target_folder_name_and_syncs_until_manual_edit() {
    use KeyCode::{BackTab, Down, Enter, Tab};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let alpha_dir = tmp.path().join("alpha-proj");
    let beta_dir = tmp.path().join("beta.svc");
    std::fs::create_dir_all(&alpha_dir).expect("alpha dir");
    std::fs::create_dir_all(&beta_dir).expect("beta dir");

    let hub = server
        .client
        .new_session("hub", &alpha_dir)
        .expect("hub session");
    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 32)).expect("term");

    // Open New Window DualForm from Home Matrix (`+ New window`).
    keys(&mut app, &[Down, Enter]);
    term.draw(|f| app.render(f)).expect("draw initial dualform");
    let init_screen = term.backend().to_string();
    assert!(init_screen.contains("NEW WINDOW"), "{init_screen}");
    assert!(init_screen.contains("alpha-proj"), "{init_screen}");
    assert!(init_screen.contains("auto from folder"), "{init_screen}");
    assert!(init_screen.contains("exists"), "{init_screen}");

    // Changing Directory dynamically updates unedited Name (`beta.svc` kept verbatim for windows).
    keys(&mut app, &[Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, beta_dir.to_str().expect("utf8 beta"));
    term.draw(|f| app.render(f))
        .expect("draw synced window name");
    let synced_screen = term.backend().to_string();
    assert!(synced_screen.contains("beta.svc"), "{synced_screen}");

    key_mod(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app.exit().is_none(), "Alt+Enter stays on Home");
    let win = server
        .client
        .list_windows(Some(&hub.id))
        .expect("hub windows")
        .into_iter()
        .find(|w| w.name == "beta.svc")
        .expect("window named beta.svc");
    let win_pane = server.client.list_panes(Some(&win.id)).expect("win pane")[0].clone();
    assert_eq!(
        Path::new(&win_pane.current_path)
            .canonicalize()
            .expect("canon actual"),
        beta_dir.canonicalize().expect("canon beta")
    );

    // New Session DualForm sanitizes dots in folder names (`beta.svc` -> `beta-svc`)
    // and stops syncing once Name is manually edited.
    text(&mut app, "/new");
    keys(&mut app, &[Enter, Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, beta_dir.to_str().expect("utf8 beta"));
    term.draw(|f| app.render(f))
        .expect("draw sanitized session");
    let sess_synced = term.backend().to_string();
    assert!(sess_synced.contains("beta-svc"), "{sess_synced}");

    // Switch back to Name, replace via replace-on-type, then change Dir to `alpha-proj`:
    // manual name `custom-sess` must stay intact.
    keys(&mut app, &[BackTab]);
    text(&mut app, "custom-sess");
    keys(&mut app, &[Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, alpha_dir.to_str().expect("utf8 alpha"));
    term.draw(|f| app.render(f))
        .expect("draw manual name preserved");
    let manual_screen = term.backend().to_string();
    assert!(manual_screen.contains("custom-sess"), "{manual_screen}");

    key_mod(&mut app, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app.exit().is_none(), "Alt+Enter stays on Home");
    let sess = server
        .client
        .list_sessions()
        .expect("sessions")
        .into_iter()
        .find(|s| s.name == "custom-sess")
        .expect("custom-sess created");
    assert_eq!(
        Path::new(&sess.path)
            .canonicalize()
            .expect("canon sess path"),
        alpha_dir.canonicalize().expect("canon alpha")
    );
}

#[test]
fn dual_form_zsh_style_directory_tab_completion_drill_in_and_esc_restore() {
    use KeyCode::{Char, Down, Enter, Esc, Tab};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon root");
    std::fs::create_dir_all(root.join("app-api/v1")).expect("app-api/v1");
    std::fs::create_dir_all(root.join("app-web")).expect("app-web");
    std::fs::create_dir_all(root.join(".hidden-dir")).expect(".hidden-dir");
    std::fs::write(root.join("app-file.txt"), b"ignore").expect("regular file");

    let sess = server.client.new_session("comp", &root).expect("comp sess");
    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 34)).expect("term");

    // Open New Window DualForm, move to Directory, enter partial prefix `<root>/app`, press Tab.
    keys(&mut app, &[Down, Enter, Tab]);
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, &format!("{}/app", root.display()));
    keys(&mut app, &[Tab]);

    term.draw(|f| app.render(f)).expect("draw completion menu");
    let menu_screen = term.backend().to_string();
    assert!(menu_screen.contains("app-api/"), "{menu_screen}");
    assert!(menu_screen.contains("app-web/"), "{menu_screen}");
    assert!(!menu_screen.contains("app-file.txt"), "{menu_screen}");
    assert!(!menu_screen.contains(".hidden-dir"), "{menu_screen}");

    // Cycle with Tab -> app-api/, Ctrl+N -> app-web/, Ctrl+P -> app-api/, then Esc restores `<root>/app-`.
    keys(&mut app, &[Tab]);
    key_mod(&mut app, Char('n'), KeyModifiers::CONTROL);
    term.draw(|f| app.render(f)).expect("draw app-web preview");
    assert!(term.backend().to_string().contains("app-web"));
    key_mod(&mut app, Char('p'), KeyModifiers::CONTROL);
    term.draw(|f| app.render(f)).expect("draw app-api preview");
    assert!(term.backend().to_string().contains("app-api"));

    keys(&mut app, &[Esc]);
    term.draw(|f| app.render(f))
        .expect("draw after esc restore");
    let restored_screen = term.backend().to_string();
    assert!(
        restored_screen.contains("NEW WINDOW"),
        "Esc closes menu, keeps DualForm open: {restored_screen}"
    );
    assert!(!restored_screen.contains("app-web/"), "{restored_screen}");

    // Re-open with Tab, select `app-api/`, press `/` to drill into `app-api/` -> shows `v1/`.
    keys(&mut app, &[Tab, Tab, Char('/')]);
    term.draw(|f| app.render(f)).expect("draw drilled v1");
    let drilled_screen = term.backend().to_string();
    assert!(drilled_screen.contains("v1/"), "{drilled_screen}");

    // 1st Enter accepts `v1/` without submitting; 2nd Enter creates & enters `v1`.
    keys(&mut app, &[Enter]);
    assert!(
        app.exit().is_none(),
        "Enter on completion menu accepts without submitting"
    );
    assert!(
        !server
            .client
            .list_windows(Some(&sess.id))
            .expect("windows before submit")
            .iter()
            .any(|w| w.name == "v1")
    );
    keys(&mut app, &[Enter]);
    assert!(
        matches!(app.exit(), Some(Exit::Attach(Target::Window(sid, _))) if sid == &sess.id),
        "2nd Enter creates and enters window, got {:?}",
        app.exit()
    );
    let created = server
        .client
        .list_windows(Some(&sess.id))
        .expect("windows after submit")
        .into_iter()
        .find(|w| w.name == "v1")
        .expect("v1 window");
    let v1_pane = server
        .client
        .list_panes(Some(&created.id))
        .expect("v1 pane")[0]
        .clone();
    assert_eq!(
        Path::new(&v1_pane.current_path)
            .canonicalize()
            .expect("canon v1"),
        root.join("app-api/v1")
    );

    // Hidden `.hidden-dir` completion (only when prefix starts with `.`) & Ctrl+W segment rubout.
    let mut app2 = App::new(&server.client, AppOptions::default());
    keys(&mut app2, &[Down, Down, Enter, Tab]);
    key_mod(&mut app2, Char('u'), KeyModifiers::CONTROL);
    text(&mut app2, &format!("{}/.h", root.display()));
    keys(&mut app2, &[Tab]);
    term.draw(|f| app2.render(f)).expect("draw hidden dir");
    let hidden_screen = term.backend().to_string();
    assert!(hidden_screen.contains(".hidden-dir"), "{hidden_screen}");

    key_mod(&mut app2, Char('w'), KeyModifiers::CONTROL);
    text(&mut app2, "app-w");
    keys(&mut app2, &[Tab]);
    term.draw(|f| app2.render(f)).expect("draw after ctrl-w");
    assert!(term.backend().to_string().contains("app-web"));
}

#[test]
fn dual_form_enter_creates_and_enters_while_alt_enter_stays_on_home() {
    use KeyCode::{Down, Enter, Tab};

    let server = PrivateServer::new();
    let base = server
        .client
        .new_session("main", cwd())
        .expect("base session");

    // Plain Enter creates New Window and attaches immediately.
    let mut app_win_enter = App::new(&server.client, AppOptions::default());
    keys(&mut app_win_enter, &[Down, Enter]);
    text(&mut app_win_enter, "enter-win");
    keys(&mut app_win_enter, &[Enter]);
    let enter_win = server
        .client
        .list_windows(Some(&base.id))
        .expect("wins")
        .into_iter()
        .find(|w| w.name == "enter-win")
        .expect("enter-win");
    assert_eq!(
        app_win_enter.exit(),
        Some(&Exit::Attach(Target::Window(base.id.clone(), enter_win.id)))
    );

    // Plain Enter creates New Session and attaches immediately.
    let mut app_sess_enter = App::new(&server.client, AppOptions::default());
    text(&mut app_sess_enter, "/new");
    keys(&mut app_sess_enter, &[Enter]);
    text(&mut app_sess_enter, "enter-sess");
    keys(&mut app_sess_enter, &[Tab]);
    key_mod(
        &mut app_sess_enter,
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    );
    text(&mut app_sess_enter, cwd().to_str().expect("utf8 cwd"));
    keys(&mut app_sess_enter, &[Enter]);
    let enter_sess = server
        .client
        .list_sessions()
        .expect("sessions")
        .into_iter()
        .find(|s| s.name == "enter-sess")
        .expect("enter-sess");
    assert_eq!(
        app_sess_enter.exit(),
        Some(&Exit::Attach(Target::Session(enter_sess.id)))
    );

    // Alt+Enter creates New Window, stays on Home with new window selected in Open mode.
    let stay_server = PrivateServer::new();
    let stay_base = stay_server
        .client
        .new_session("stay-main", cwd())
        .expect("stay-main session");
    let mut app_stay = App::new(&stay_server.client, AppOptions::default());
    keys(&mut app_stay, &[Down, Enter]);
    text(&mut app_stay, "stay-win");
    key_mod(&mut app_stay, KeyCode::Enter, KeyModifiers::ALT);
    assert!(app_stay.exit().is_none(), "Alt+Enter must stay on Home");
    let stay_win = stay_server
        .client
        .list_windows(Some(&stay_base.id))
        .expect("wins stay")
        .into_iter()
        .find(|w| w.name == "stay-win")
        .expect("stay-win");
    keys(&mut app_stay, &[Enter]);
    assert_eq!(
        app_stay.exit(),
        Some(&Exit::Attach(Target::Window(
            stay_base.id.clone(),
            stay_win.id
        )))
    );

    // Alt+Enter creates New Session, stays on Home with new session focused in Open mode.
    let mut app_sess_stay = App::new(&stay_server.client, AppOptions::default());
    text(&mut app_sess_stay, "/new");
    keys(&mut app_sess_stay, &[Enter]);
    text(&mut app_sess_stay, "stay-sess");
    keys(&mut app_sess_stay, &[Tab]);
    key_mod(
        &mut app_sess_stay,
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    );
    text(&mut app_sess_stay, cwd().to_str().expect("utf8 cwd"));
    key_mod(&mut app_sess_stay, KeyCode::Enter, KeyModifiers::ALT);
    assert!(
        app_sess_stay.exit().is_none(),
        "Alt+Enter must stay on Home"
    );
    let stay_sess = stay_server
        .client
        .list_sessions()
        .expect("sessions stay")
        .into_iter()
        .find(|s| s.name == "stay-sess")
        .expect("stay-sess");
    keys(&mut app_sess_stay, &[Enter]);
    assert!(
        matches!(
            app_sess_stay.exit(),
            Some(Exit::Attach(Target::Session(sid) | Target::Window(sid, _))) if sid == &stay_sess.id
        ),
        "Home Enter after Alt+Enter /new must enter newly created session, got {:?}",
        app_sess_stay.exit()
    );
}

#[test]
fn tiered_kill_confirmation_allows_y_for_idle_window_pane_and_requires_yes_for_high_risk() {
    use KeyCode::{Down, Enter};

    let server = PrivateServer::new();
    let sess = server.client.new_session("work", cwd()).expect("work");
    let idle_win = server
        .client
        .new_window(&sess.id, Some("idle-win"), Some(cwd()))
        .expect("idle-win");

    // 1. Idle non-current window accepts `Y ` (case-insensitive trimmed `y`).
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/kill");
    keys(&mut app, &[Enter]);
    text(&mut app, "idle-win");
    keys(&mut app, &[Enter]);
    text(&mut app, "Y ");
    keys(&mut app, &[Enter]);
    assert!(
        !server
            .client
            .list_windows(Some(&sess.id))
            .expect("wins after idle kill")
            .iter()
            .any(|w| w.id == idle_win.id),
        "idle window should be killed by 'Y '"
    );

    // 2. Idle pane in 2-pane window accepts `y`.
    let win0 = server.client.list_windows(Some(&sess.id)).expect("win0")[0].clone();
    let p0 = server.client.list_panes(Some(&win0.id)).expect("p0")[0].clone();
    let idle_p2 = server
        .client
        .split_pane(&p0.id, SplitDirection::SideBySide)
        .expect("idle split");
    let mut pane_app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/pane".into()),
        },
    );
    keys(&mut pane_app, &[Down, Enter]);
    for _ in 0..16 {
        keys(&mut pane_app, &[Down]);
    }
    keys(&mut pane_app, &[Enter]);
    text(&mut pane_app, "y");
    keys(&mut pane_app, &[Enter]);
    assert_eq!(
        server
            .client
            .list_panes(Some(&win0.id))
            .expect("panes after idle pane kill")
            .len(),
        1,
        "idle pane {} should be killed by 'y'",
        idle_p2.id
    );

    // 3. Busy pane (`sleep 30`) rejects `y` and requires `yes`.
    let out_busy = server.tmux(&["split-window", "-d", "-t", win0.id.as_str(), "sleep", "30"]);
    assert!(out_busy.status.success());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if server
            .client
            .list_panes(Some(&win0.id))
            .expect("panes")
            .iter()
            .any(|p| p.busy)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "busy pane did not start sleep"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    let mut busy_app = App::new(
        &server.client,
        AppOptions {
            initial_command: Some("/pane".into()),
        },
    );
    keys(&mut busy_app, &[Down, Enter]);
    for _ in 0..16 {
        keys(&mut busy_app, &[Down]);
    }
    keys(&mut busy_app, &[Enter]);
    text(&mut busy_app, "y");
    keys(&mut busy_app, &[Enter]);
    assert_eq!(
        server
            .client
            .list_panes(Some(&win0.id))
            .expect("busy pane must survive 'y'")
            .len(),
        2
    );
    keys(&mut busy_app, &[Enter]);
    for _ in 0..16 {
        keys(&mut busy_app, &[Down]);
    }
    keys(&mut busy_app, &[Enter]);
    text(&mut busy_app, "yes");
    keys(&mut busy_app, &[Enter]);
    assert_eq!(
        server
            .client
            .list_panes(Some(&win0.id))
            .expect("busy pane killed by 'yes'")
            .len(),
        1
    );

    // 4. Killing a session rejects `y` and accepts `YES `.
    let victim = server
        .client
        .new_session("victim-sess", cwd())
        .expect("victim");
    text(&mut app, "/kill @victim-sess");
    keys(&mut app, &[Enter]);
    text(&mut app, "y");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .has_session(&victim.id)
            .expect("victim survives 'y'")
    );

    text(&mut app, "/kill @victim-sess");
    keys(&mut app, &[Enter]);
    text(&mut app, "YES ");
    keys(&mut app, &[Enter]);
    assert!(
        !server
            .client
            .has_session(&victim.id)
            .expect("victim killed by 'YES '")
    );

    // 5. `/kill-server` rejects `y` and accepts `yes`.
    text(&mut app, "/kill-server");
    keys(&mut app, &[Enter]);
    text(&mut app, "y");
    keys(&mut app, &[Enter]);
    assert!(server.client.server_running().expect("server survives 'y'"));

    text(&mut app, "/kill-server");
    keys(&mut app, &[Enter]);
    text(&mut app, "yes");
    keys(&mut app, &[Enter]);
    let shutdown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while server
        .client
        .server_running()
        .expect("server killed by 'yes'")
    {
        assert!(
            std::time::Instant::now() < shutdown_deadline,
            "private tmux server stayed running after confirmed /kill-server"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn single_click_matrix_create_cells_and_dual_form_mouse_completion() {
    use KeyCode::{Esc, Tab};
    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon root");
    std::fs::create_dir_all(root.join("dir-one")).expect("dir-one");
    std::fs::create_dir_all(root.join("dir-two")).expect("dir-two");

    let _sess = server
        .client
        .new_session("clicker", &root)
        .expect("clicker");
    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 34)).expect("term");

    term.draw(|f| app.render(f)).expect("initial home draw");
    let (nw_x, nw_y) = find_rendered_text(&term, "+ New window").expect("+ New window cell");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        nw_x,
        nw_y,
    ));
    term.draw(|f| app.render(f))
        .expect("dualform new window draw");
    assert!(term.backend().to_string().contains("NEW WINDOW"));
    keys(&mut app, &[Esc]);

    term.draw(|f| app.render(f)).expect("home redraw");
    let (ns_x, ns_y) = find_rendered_text(&term, "+ New session").expect("+ New session cell");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        ns_x,
        ns_y,
    ));
    term.draw(|f| app.render(f))
        .expect("dualform new session draw");
    assert!(term.backend().to_string().contains("NEW SESSION"));

    // Click Working directory field label to focus Dir without Tab.
    let (dir_x, dir_y) =
        find_rendered_text(&term, "Working directory").expect("Working directory field");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        dir_x,
        dir_y,
    ));
    key_mod(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    text(&mut app, &format!("{}/dir", root.display()));
    keys(&mut app, &[Tab]);
    term.draw(|f| app.render(f)).expect("draw dir completion");
    let menu_screen = term.backend().to_string();
    assert!(menu_screen.contains("dir-one/"), "{menu_screen}");
    assert!(menu_screen.contains("dir-two/"), "{menu_screen}");

    // Single-click `dir-two/` selects & previews; immediate second click double-clicks into leaf dir & closes menu.
    let (d2_x, d2_y) = find_rendered_text(&term, "dir-two/").expect("dir-two row");
    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        d2_x,
        d2_y,
    ));
    term.draw(|f| app.render(f))
        .expect("draw after single click");
    let preview_screen = term.backend().to_string();
    assert!(preview_screen.contains("dir-two"), "{preview_screen}");
    assert!(app.exit().is_none());

    app.handle_event(mouse_ev(
        MouseEventKind::Down(MouseButton::Left),
        d2_x,
        d2_y,
    ));
    term.draw(|f| app.render(f))
        .expect("draw after double click");
    let accepted_screen = term.backend().to_string();
    assert!(!accepted_screen.contains("dir-one/"), "{accepted_screen}");
    assert!(accepted_screen.contains("NEW SESSION"), "{accepted_screen}");
    assert!(app.exit().is_none());
}

#[test]
fn rename_selected_all_terminal_nav_and_composer_new_path_tab_completion() {
    use KeyCode::{Char, Enter, Esc, Right, Tab};
    use justmux::ui::ThemePreset;
    use ratatui::{Terminal, backend::TestBackend};

    let server = PrivateServer::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon root");
    std::fs::create_dir_all(root.join("dir-one")).expect("dir-one");
    std::fs::create_dir_all(root.join("dir-two")).expect("dir-two");

    let sess = server.client.new_session("alpha", &root).expect("alpha");
    let _ = server.tmux(&["rename-window", "-t", sess.id.as_str(), "main"]);
    let mut app = App::new(&server.client, AppOptions::default());
    let mut term = Terminal::new(TestBackend::new(124, 32)).expect("term");

    // 1. `/rename` opens FormAction::Rename selected-all: immediate typing replaces; Right cancels & appends.
    text(&mut app, "/rename");
    keys(&mut app, &[Enter]);
    text(&mut app, "main");
    keys(&mut app, &[Enter]);
    text(&mut app, "beta");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&sess.id))
            .expect("wins after rename 1")
            .iter()
            .any(|w| w.name == "beta")
    );

    text(&mut app, "/rename");
    keys(&mut app, &[Enter]);
    text(&mut app, "beta");
    keys(&mut app, &[Enter, Right]);
    text(&mut app, "-2");
    keys(&mut app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&sess.id))
            .expect("wins after rename 2")
            .iter()
            .any(|w| w.name == "beta-2")
    );

    // 2. Terminal navigation: j/k & Ctrl+N/Ctrl+P in ThemePicker, Config (h/l), and Actions.
    app.set_theme(justmux::ui::Theme::default().with_preset(ThemePreset::AntigravityAurora));
    text(&mut app, "/theme");
    keys(&mut app, &[Enter, Char('j')]);
    assert_eq!(app.theme().preset, ThemePreset::CodexEmerald);
    keys(&mut app, &[Char('k')]);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    key_mod(&mut app, Char('n'), KeyModifiers::CONTROL);
    assert_eq!(app.theme().preset, ThemePreset::CodexEmerald);
    key_mod(&mut app, Char('p'), KeyModifiers::CONTROL);
    assert_eq!(app.theme().preset, ThemePreset::AntigravityAurora);
    keys(&mut app, &[Esc]);

    let initial_mode = app.config().screen_mode;
    text(&mut app, "/config");
    keys(&mut app, &[Enter, Char('l')]);
    assert_ne!(app.config().screen_mode, initial_mode);
    keys(&mut app, &[Char('h')]);
    assert_eq!(app.config().screen_mode, initial_mode);
    key_mod(&mut app, Char('n'), KeyModifiers::CONTROL);
    key_mod(&mut app, Char('p'), KeyModifiers::CONTROL);
    keys(&mut app, &[Char('j'), Char('k'), Esc]);

    text(&mut app, "/list");
    keys(&mut app, &[Enter, Enter, Char('j'), Char('k')]);
    key_mod(&mut app, Char('n'), KeyModifiers::CONTROL);
    key_mod(&mut app, Char('p'), KeyModifiers::CONTROL);
    keys(&mut app, &[Esc, Esc]);

    // 3. Home composer `/new demo <path>` Tab directory completion.
    text(&mut app, &format!("/new demo {}/dir", root.display()));
    keys(&mut app, &[Tab]);
    term.draw(|f| app.render(f)).expect("draw composer lcp");
    let lcp_screen = term.backend().to_string();
    assert!(
        lcp_screen.contains("/new demo") && lcp_screen.contains("dir-"),
        "{lcp_screen}"
    );
    text(&mut app, "t");
    keys(&mut app, &[Tab]);
    term.draw(|f| app.render(f))
        .expect("draw composer completed");
    let done_screen = term.backend().to_string();
    assert!(done_screen.contains("dir-two/"), "{done_screen}");
}
