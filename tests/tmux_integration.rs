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

#[test]
fn session_management_works_through_keyboard_events() {
    use KeyCode::{Down, Enter, Esc};

    let server = PrivateServer::new();
    let mut app = App::new(&server.client, AppOptions::default());
    text(&mut app, "/new");
    keys(&mut app, &[Enter]);
    text(&mut app, "项目 workspace");
    keys(&mut app, &[Enter]);
    text(&mut app, env!("CARGO_MANIFEST_DIR"));
    keys(&mut app, &[Enter]);
    let sessions = server.client.list_sessions().expect("created sessions");
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.name, "项目 workspace");

    keys(&mut app, &[Esc, Enter, Down, Enter]);
    text(&mut app, "renamed project");
    keys(&mut app, &[Enter]);
    let renamed = server.client.list_sessions().expect("renamed sessions");
    assert_eq!(renamed[0].id, session.id);
    assert_eq!(renamed[0].name, "renamed project");

    // A lone Enter at the destructive confirmation must cancel.
    keys(&mut app, &[Enter, Down, Down, Enter, Enter]);
    assert!(server.client.has_session(&session.id).expect("not killed"));
    keys(&mut app, &[Enter, Down, Down, Enter]);
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
    use KeyCode::{Char, Down, Enter, Esc, Right};

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
    keys(&mut app, &[Enter]);
    let windows = server
        .client
        .list_windows(Some(&session.id))
        .expect("new window");
    assert_eq!(windows.len(), 2);
    let added = windows
        .iter()
        .find(|w| w.name == "build")
        .expect("build window");
    keys(&mut app, &[Esc]);

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
    keys(&mut app, &[Down, Enter, Down, Enter, Enter]);
    assert_eq!(server.client.list_panes(None).expect("cancelled").len(), 2);
    keys(&mut app, &[Enter, Down, Enter]);
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
    keys(&mut matrix_app, &[Enter]);
    assert!(
        server
            .client
            .list_windows(Some(&session.id))
            .expect("matrix new window")
            .iter()
            .any(|w| w.name == "from-matrix")
    );
    keys(&mut matrix_app, &[Esc, Right, Enter]);
    text(&mut matrix_app, "matrix-workspace");
    keys(&mut matrix_app, &[Enter, Enter]);
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
    keys(&mut app, &[Down, Enter]);
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
