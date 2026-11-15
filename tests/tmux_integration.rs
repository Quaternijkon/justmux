#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
