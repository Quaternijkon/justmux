//! Real tmux CLI adapter.
use super::error::TmuxError;
use super::parser;
use super::{
    Context, InsideContext, Pane, PaneId, Session, SessionId, SocketSpec, SplitDirection, Target,
    Tmux, TmuxResult, TmuxVersion, Window, WindowId,
};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone)]
pub struct TmuxClient {
    program: OsString,
    socket: SocketSpec,
    config_file: Option<PathBuf>,
}
impl TmuxClient {
    pub fn new(socket: SocketSpec) -> Self {
        Self {
            program: "tmux".into(),
            socket,
            config_file: None,
        }
    }
    pub fn with_program(mut self, p: impl Into<OsString>) -> Self {
        self.program = p.into();
        self
    }
    pub fn with_config_file(mut self, p: impl Into<PathBuf>) -> Self {
        self.config_file = Some(p.into());
        self
    }
    pub fn socket(&self) -> &SocketSpec {
        &self.socket
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.program);
        match &self.socket {
            SocketSpec::Default => {}
            SocketSpec::Name(n) => {
                c.arg("-L").arg(n);
            }
            SocketSpec::Path(p) => {
                c.arg("-S").arg(p);
            }
        }
        if let Some(f) = &self.config_file {
            c.arg("-f").arg(f);
        }
        // An explicit socket must win over inherited tmux client variables.
        // This also keeps tests and `jm -L/-S` from accidentally addressing
        // the server of the pane that launched JustMux.
        if !matches!(self.socket, SocketSpec::Default) {
            // Keep TMUX_PANE available for context discovery when an explicit
            // socket names the same server as the pane that launched JustMux.
            // The explicit -L/-S flag still prevents TMUX from selecting a
            // different inherited server.
            c.env_remove("TMUX");
        }
        c.args(args);
        c
    }
    fn run(&self, args: &[&str]) -> TmuxResult<Output> {
        self.command(args).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TmuxError::NotInstalled
            } else {
                e.into()
            }
        })
    }
    fn checked(&self, args: &[&str]) -> TmuxResult<Output> {
        let o = self.run(args)?;
        if o.status.success() {
            Ok(o)
        } else {
            Err(self.map_error(args.first().copied().unwrap_or("command"), &o))
        }
    }
    fn text(&self, args: &[&str]) -> TmuxResult<String> {
        let o = self.checked(args)?;
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    }
    fn map_error(&self, cmd: &str, o: &Output) -> TmuxError {
        let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
        let low = msg.to_ascii_lowercase();
        if low.contains("no server running")
            || low.contains("error connecting")
            || low.contains("can't find socket")
        {
            TmuxError::NoServer
        } else if low.contains("duplicate") || low.contains("already exists") {
            TmuxError::DuplicateSession(msg)
        } else if low.contains("session") && low.contains("can't find") {
            TmuxError::SessionNotFound
        } else if low.contains("window") && low.contains("can't find") {
            TmuxError::WindowNotFound
        } else if low.contains("pane") && low.contains("can't find") {
            TmuxError::PaneNotFound
        } else if low.contains("no current client") || low.contains("no client") {
            TmuxError::NoCurrentClient
        } else {
            TmuxError::CommandFailed {
                command: cmd.to_string(),
                message: if msg.is_empty() {
                    "tmux returned a non-zero status".into()
                } else {
                    msg
                },
            }
        }
    }
    fn list_with(&self, args: &[&str]) -> TmuxResult<String> {
        match self.run(args)? {
            o if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
            o => {
                let e = self.map_error(args.first().copied().unwrap_or("list"), &o);
                if matches!(e, TmuxError::NoServer) {
                    Ok(String::new())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// tmux's implicit current-client selection chooses the most recently
    /// active client, which is unsafe with multiple clients. Use an explicit
    /// popup client token when available, otherwise require exactly one
    /// interactive client showing the launching pane's window, even if that
    /// window is linked into several sessions.
    fn current_client(&self, window: &WindowId) -> Option<(String, String)> {
        let requested = env::var("JUSTMUX_CLIENT").ok();
        let clients = self
            .text(&[
                "list-clients",
                "-F",
                "#{client_name}\t#{window_id}\t#{client_control_mode}\t#{client_session}",
            ])
            .ok()?;
        let mut candidates = clients.lines().filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let name = fields.next()?;
            let active_window = fields.next()?;
            let control_mode = fields.next()?;
            let session = fields.next()?;
            let matched = match requested.as_deref() {
                Some(requested) => requested == name,
                None => active_window == window.as_str(),
            };
            (matched && !name.is_empty() && control_mode != "1").then_some((name, session))
        });
        let (name, session) = candidates.next()?;
        candidates
            .next()
            .is_none()
            .then(|| (name.to_owned(), session.to_owned()))
    }
}
const SESSION_FORMAT: &str = "#{session_id}\t#{q:session_name}\t#{session_windows}\t#{session_attached}\t#{session_created}\t#{session_last_attached}\t#{session_activity}\t#{q:session_path}";
const WINDOW_FORMAT: &str = "#{window_id}\t#{session_id}\t#{q:session_name}\t#{window_index}\t#{q:window_name}\t#{window_active}\t#{window_panes}";
const PANE_FORMAT: &str = "#{pane_id}\t#{window_id}\t#{window_index}\t#{q:window_name}\t#{session_id}\t#{q:session_name}\t#{pane_index}\t#{pane_active}\t#{q:pane_current_command}\t#{q:pane_current_path}\t#{q:pane_title}\t#{pane_pid}\t#{pane_dead}\t#{pane_in_mode}";
impl Tmux for TmuxClient {
    fn version(&self) -> TmuxResult<TmuxVersion> {
        parser::parse_version(&String::from_utf8_lossy(&self.checked(&["-V"])?.stdout))
    }
    fn server_running(&self) -> TmuxResult<bool> {
        let o = self.run(&["has-session"])?;
        if o.status.success() {
            Ok(true)
        } else {
            let e = self.map_error("has-session", &o);
            if matches!(e, TmuxError::NoServer | TmuxError::SessionNotFound) {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
    fn context(&self) -> Context {
        let Some(tmux_env) = env::var_os("TMUX") else {
            return Context::Outside;
        };
        let tmux_env_text = tmux_env.to_string_lossy();
        let inherited_socket = tmux_env_text.split(',').next().unwrap_or("");
        let pane = match env::var("JUSTMUX_PANE").or_else(|_| env::var("TMUX_PANE")) {
            Ok(p) => p,
            Err(_) => return Context::Unavailable,
        };
        let pane_line = match self.text(&[
            "display-message",
            "-p",
            "-t",
            pane.as_str(),
            "#{session_id}\t#{window_id}\t#{pane_id}\t#{pane_tty}\t#{socket_path}",
        ]) {
            Ok(v) => v,
            Err(_) => {
                return if matches!(self.socket, SocketSpec::Default) {
                    Context::Unavailable
                } else {
                    Context::ForeignServer
                };
            }
        };
        let p: Vec<_> = pane_line.trim_end().split('\t').collect();
        if p.len() < 5 {
            return Context::Unavailable;
        }
        let socket = p[4];
        if !same_socket(Path::new(socket), Path::new(inherited_socket)) {
            return Context::ForeignServer;
        }
        if let SocketSpec::Path(path) = &self.socket
            && !same_socket(path, Path::new(socket))
        {
            return Context::ForeignServer;
        }
        if let SocketSpec::Name(_) = &self.socket {
            // -L has no path available without asking tmux; current pane's
            // socket is authoritative, and the explicit -L command already
            // selected the requested server. A mismatched server yielded no
            // matching pane above and therefore resolves as ForeignServer.
        }
        let Some(window_id) = WindowId::new(p[1].to_string()) else {
            return Context::Unavailable;
        };
        let Some((client_name, session_name)) = self.current_client(&window_id) else {
            return Context::Unavailable;
        };
        Context::Inside(InsideContext {
            session_id: SessionId::new(p[0].to_string()),
            session_name: Some(session_name),
            window_id: Some(window_id),
            pane_id: PaneId::new(p[2].to_string()),
            popup: env::var_os("JUSTMUX_POPUP").is_some() || env::var_os("TMUX_POPUP").is_some(),
            client_name: Some(client_name),
        })
    }
    fn list_sessions(&self) -> TmuxResult<Vec<Session>> {
        parser::parse_sessions(&self.list_with(&["list-sessions", "-F", SESSION_FORMAT])?)
    }
    fn has_session(&self, id: &SessionId) -> TmuxResult<bool> {
        let o = self.run(&["has-session", "-t", id.as_str()])?;
        if o.status.success() {
            Ok(true)
        } else {
            let e = self.map_error("has-session", &o);
            if matches!(e, TmuxError::NoServer | TmuxError::SessionNotFound) {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
    fn new_session(&self, name: &str, cwd: &Path) -> TmuxResult<Session> {
        super::validate_session_name(name)?;
        if !cwd.is_dir() {
            return Err(TmuxError::DirectoryNotFound(cwd.to_path_buf()));
        }
        match self.checked(&[
            "new-session",
            "-d",
            "-s",
            name,
            "-c",
            cwd.to_str().ok_or_else(|| {
                TmuxError::InvalidName("working directory is not valid UTF-8".into())
            })?,
        ]) {
            Err(TmuxError::DuplicateSession(_)) => {
                return Err(TmuxError::DuplicateSession(name.to_owned()));
            }
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        self.list_sessions()?
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| TmuxError::Parse("created session was not returned by tmux".into()))
    }
    fn rename_session(&self, id: &SessionId, new_name: &str) -> TmuxResult<()> {
        super::validate_session_name(new_name)?;
        match self.checked(&["rename-session", "-t", id.as_str(), new_name]) {
            Err(TmuxError::DuplicateSession(_)) => {
                Err(TmuxError::DuplicateSession(new_name.to_owned()))
            }
            Err(error) => Err(error),
            Ok(_) => Ok(()),
        }
    }
    fn kill_session(&self, id: &SessionId) -> TmuxResult<()> {
        self.checked(&["kill-session", "-t", id.as_str()])
            .map(|_| ())
    }
    fn list_windows(&self, session: Option<&SessionId>) -> TmuxResult<Vec<Window>> {
        let mut a = vec!["list-windows", "-F", WINDOW_FORMAT];
        if let Some(s) = session {
            a.extend(["-t", s.as_str()]);
        } else {
            a.push("-a");
        }
        parser::parse_windows(&self.list_with(&a)?)
    }
    fn new_window(
        &self,
        session: &SessionId,
        name: Option<&str>,
        cwd: Option<&Path>,
    ) -> TmuxResult<Window> {
        let mut a = vec![
            "new-window",
            "-d",
            "-P",
            "-F",
            WINDOW_FORMAT,
            "-t",
            session.as_str(),
        ];
        if let Some(n) = name {
            a.extend(["-n", n]);
        }
        if let Some(p) = cwd {
            a.extend([
                "-c",
                p.to_str().ok_or_else(|| {
                    TmuxError::InvalidName("working directory is not valid UTF-8".into())
                })?,
            ]);
        }
        let text = self.text(&a)?;
        parser::parse_windows(&text)?
            .into_iter()
            .next()
            .ok_or(TmuxError::WindowNotFound)
    }
    fn rename_window(&self, id: &WindowId, new_name: &str) -> TmuxResult<()> {
        self.checked(&["rename-window", "-t", id.as_str(), new_name])
            .map(|_| ())
    }
    fn kill_window(&self, id: &WindowId) -> TmuxResult<()> {
        self.checked(&["kill-window", "-t", id.as_str()])
            .map(|_| ())
    }
    fn list_panes(&self, window: Option<&WindowId>) -> TmuxResult<Vec<Pane>> {
        let mut a = vec!["list-panes", "-F", PANE_FORMAT];
        if let Some(w) = window {
            a.extend(["-t", w.as_str()]);
        } else {
            a.push("-a");
        }
        parser::parse_panes(&self.list_with(&a)?)
    }
    fn split_pane(&self, target: &PaneId, direction: SplitDirection) -> TmuxResult<Pane> {
        let flag = match direction {
            SplitDirection::SideBySide => "-h",
            SplitDirection::Stacked => "-v",
        };
        let a = vec![
            "split-window",
            "-d",
            flag,
            "-t",
            target.as_str(),
            "-c",
            // This is already one argv entry. Shell escaping would become a
            // literal part of the directory and tmux would fall back to HOME.
            "#{pane_current_path}",
            "-P",
            "-F",
            PANE_FORMAT,
        ];
        let out = self.checked(&a)?;
        let mut p = parser::parse_panes(&String::from_utf8_lossy(&out.stdout))?;
        p.pop().ok_or(TmuxError::PaneNotFound)
    }
    fn kill_pane(&self, id: &PaneId) -> TmuxResult<()> {
        self.checked(&["kill-pane", "-t", id.as_str()]).map(|_| ())
    }
    fn select(&self, target: &Target) -> TmuxResult<()> {
        match target {
            Target::Session(_) => Ok(()),
            Target::Window(..) => {
                let target = target.to_string();
                let resolved = self.text(&[
                    "display-message",
                    "-p",
                    "-t",
                    &target,
                    "#{session_id}:#{window_id}",
                ])?;
                if resolved.trim_end() == target {
                    Ok(())
                } else {
                    Err(TmuxError::WindowNotFound)
                }
            }
            Target::Pane(id) => self
                .checked(&["select-pane", "-t", id.as_str()])
                .map(|_| ()),
        }
    }
    fn switch_client(&self, target: &Target) -> TmuxResult<()> {
        let Context::Inside(InsideContext {
            client_name: Some(client),
            ..
        }) = self.context()
        else {
            return Err(TmuxError::NoCurrentClient);
        };
        self.checked(&[
            "switch-client",
            "-c",
            client.as_str(),
            "-t",
            &target.to_string(),
        ])
        .map(|_| ())
    }
    fn detach_client(&self, client: &str) -> TmuxResult<()> {
        self.checked(&["detach-client", "-t", client]).map(|_| ())
    }
    fn attach_command(&self, target: &Target) -> Command {
        let target = target.to_string();
        let mut cmd = self.command(&["attach-session", "-t", &target]);
        if let SocketSpec::Default = &self.socket
            && let Some(val) = env::var_os("TMUX")
        {
            let text = val.to_string_lossy();
            if let Some(path) = text.split(',').next() {
                cmd = self.command(&[]);
                cmd.arg("-S")
                    .arg(path)
                    .args(["attach-session", "-t", &target]);
            }
        }
        cmd.env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("JUSTMUX_CLIENT")
            .env_remove("JUSTMUX_PANE")
            .env_remove("JUSTMUX_POPUP")
            .env_remove("TMUX_POPUP");
        cmd
    }
}

fn same_socket(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
