//! Real tmux CLI adapter.
use super::error::TmuxError;
use super::parser;
use super::{
    ClientInfo, Context, InsideContext, LayoutPreset, Pane, PaneId, PaneTargetSize, PasteBuffer,
    ResizeDirection, Session, SessionId, SocketSpec, SplitDirection, Target, Tmux, TmuxResult,
    TmuxVersion, Window, WindowId,
};
use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    fn run_with_stdin(&self, args: &[&str], stdin_data: &[u8]) -> TmuxResult<Output> {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TmuxError::NotInstalled
                } else {
                    e.into()
                }
            })?;
        if let Some(mut stdin) = child.stdin.take()
            && let Err(e) = stdin.write_all(stdin_data)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(e.into());
        }
        Ok(child.wait_with_output()?)
    }
    fn checked(&self, args: &[&str]) -> TmuxResult<Output> {
        let o = self.run(args)?;
        if o.status.success() {
            Ok(o)
        } else {
            Err(self.map_error(args.first().copied().unwrap_or("command"), &o))
        }
    }
    fn checked_with_stdin(&self, args: &[&str], stdin_data: &[u8]) -> TmuxResult<Output> {
        let o = self.run_with_stdin(args, stdin_data)?;
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
        } else if low.contains("no current client")
            || low.contains("no client")
            || (low.contains("client") && low.contains("can't find"))
        {
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
    fn attach_command_impl(&self, target: &Target, readonly: bool) -> Command {
        let target = target.to_string();
        let args: &[&str] = if readonly {
            &["attach-session", "-r", "-t", &target]
        } else {
            &["attach-session", "-t", &target]
        };
        let mut cmd = self.command(args);
        if let SocketSpec::Default = &self.socket
            && let Some(val) = env::var_os("TMUX")
        {
            let text = val.to_string_lossy();
            if let Some(path) = text.split(',').next() {
                cmd = self.command(&[]);
                cmd.arg("-S").arg(path).args(args);
            }
        }
        cmd.env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("JUSTMUX_CLIENT")
            .env_remove("JUSTMUX_PANE")
            .env_remove("JUSTMUX_POPUP")
            .env_remove("JUSTMUX_INVOKED_AS")
            .env_remove("TMUX_POPUP");
        cmd
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
const WINDOW_FORMAT: &str = "#{window_id}\t#{session_id}\t#{q:session_name}\t#{window_index}\t#{q:window_name}\t#{window_active}\t#{window_panes}\t#{window_zoomed_flag}\t#{pane_synchronized}\t#{window_activity_flag}\t#{window_bell_flag}\t#{window_silence_flag}\t#{?monitor-activity,1,0}\t#{monitor-silence}\t#{window_layout}";
const PANE_FORMAT: &str = "#{pane_id}\t#{window_id}\t#{window_index}\t#{q:window_name}\t#{session_id}\t#{q:session_name}\t#{pane_index}\t#{pane_active}\t#{q:pane_current_command}\t#{q:pane_current_path}\t#{q:pane_title}\t#{pane_pid}\t#{pane_dead}\t#{pane_in_mode}\t#{pane_width}\t#{pane_height}\t#{pane_left}\t#{pane_top}\t#{window_zoomed_flag}\t#{pane_synchronized}";
const CLIENT_FORMAT: &str = "#{q:client_name}\t#{q:client_session}\t#{session_id}\t#{client_width}\t#{client_height}\t#{q:client_tty}\t#{client_readonly}\t#{client_control_mode}\t#{client_activity}";
const BUFFER_FORMAT: &str = "#{q:buffer_name}\t#{buffer_size}\t#{buffer_sample}";
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
            if !p.is_dir() {
                return Err(TmuxError::DirectoryNotFound(p.to_path_buf()));
            }
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
    fn move_window(&self, source: &WindowId, target_session: &SessionId) -> TmuxResult<()> {
        let target = format!("{target_session}:");
        self.checked(&["move-window", "-d", "-s", source.as_str(), "-t", &target])
            .map(|_| ())
    }
    fn swap_window(&self, source: &WindowId, target: &WindowId) -> TmuxResult<()> {
        self.checked(&[
            "swap-window",
            "-d",
            "-s",
            source.as_str(),
            "-t",
            target.as_str(),
        ])
        .map(|_| ())
    }
    fn link_window(&self, source: &WindowId, target_session: &SessionId) -> TmuxResult<()> {
        let target = format!("{target_session}:");
        self.checked(&["link-window", "-d", "-s", source.as_str(), "-t", &target])
            .map(|_| ())
    }
    fn select_layout(&self, window: &WindowId, layout: LayoutPreset) -> TmuxResult<()> {
        self.select_layout_raw(window, layout.as_tmux_str())
    }
    fn select_layout_raw(&self, window: &WindowId, layout: &str) -> TmuxResult<()> {
        self.checked(&["select-layout", "-t", window.as_str(), layout])
            .map(|_| ())
    }
    fn set_synchronize_panes(&self, window: &WindowId, enabled: bool) -> TmuxResult<()> {
        let val = if enabled { "on" } else { "off" };
        self.checked(&[
            "set-window-option",
            "-t",
            window.as_str(),
            "synchronize-panes",
            val,
        ])
        .map(|_| ())
    }
    fn set_monitor_activity(&self, window: &WindowId, enabled: bool) -> TmuxResult<()> {
        let val = if enabled { "on" } else { "off" };
        self.checked(&[
            "set-window-option",
            "-t",
            window.as_str(),
            "monitor-activity",
            val,
        ])
        .map(|_| ())
    }
    fn set_monitor_silence(&self, window: &WindowId, seconds: u64) -> TmuxResult<()> {
        let val = seconds.to_string();
        self.checked(&[
            "set-window-option",
            "-t",
            window.as_str(),
            "monitor-silence",
            &val,
        ])
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
        self.split_pane_in(target, direction, None)
    }
    fn split_pane_in(
        &self,
        target: &PaneId,
        direction: SplitDirection,
        cwd: Option<&Path>,
    ) -> TmuxResult<Pane> {
        let flag = match direction {
            SplitDirection::SideBySide => "-h",
            SplitDirection::Stacked => "-v",
        };
        let cwd_arg = match cwd {
            Some(dir) => {
                if !dir.is_dir() {
                    return Err(TmuxError::DirectoryNotFound(dir.to_path_buf()));
                }
                dir.to_str().ok_or_else(|| {
                    TmuxError::InvalidName("working directory is not valid UTF-8".into())
                })?
            }
            // This is already one argv entry. Shell escaping would become a
            // literal part of the directory and tmux would fall back to HOME.
            None => "#{pane_current_path}",
        };
        let a = vec![
            "split-window",
            "-d",
            flag,
            "-t",
            target.as_str(),
            "-c",
            cwd_arg,
            "-P",
            "-F",
            PANE_FORMAT,
        ];
        let out = self.checked(&a)?;
        let mut p = parser::parse_panes(&String::from_utf8_lossy(&out.stdout))?;
        p.pop().ok_or(TmuxError::PaneNotFound)
    }
    fn join_pane(
        &self,
        source: &PaneId,
        target: &PaneId,
        direction: SplitDirection,
    ) -> TmuxResult<()> {
        self.join_pane_ordered(source, target, direction, false)
    }
    fn join_pane_ordered(
        &self,
        source: &PaneId,
        target: &PaneId,
        direction: SplitDirection,
        before: bool,
    ) -> TmuxResult<()> {
        let flag = match direction {
            SplitDirection::SideBySide => "-h",
            SplitDirection::Stacked => "-v",
        };
        let mut a = vec!["join-pane", "-d"];
        if before {
            a.push("-b");
        }
        a.extend([flag, "-s", source.as_str(), "-t", target.as_str()]);
        self.checked(&a).map(|_| ())
    }
    fn kill_pane(&self, id: &PaneId) -> TmuxResult<()> {
        self.checked(&["kill-pane", "-t", id.as_str()]).map(|_| ())
    }
    fn toggle_zoom_pane(&self, id: &PaneId) -> TmuxResult<()> {
        self.checked(&["resize-pane", "-Z", "-t", id.as_str()])
            .map(|_| ())
    }
    fn break_pane(&self, id: &PaneId, name: Option<&str>) -> TmuxResult<Window> {
        let mut a = vec![
            "break-pane",
            "-d",
            "-P",
            "-F",
            WINDOW_FORMAT,
            "-s",
            id.as_str(),
        ];
        if let Some(n) = name {
            a.extend(["-n", n]);
        }
        let text = self.text(&a)?;
        parser::parse_windows(&text)?
            .into_iter()
            .next()
            .ok_or(TmuxError::WindowNotFound)
    }
    fn swap_pane(&self, source: &PaneId, target: &PaneId) -> TmuxResult<()> {
        self.checked(&[
            "swap-pane",
            "-d",
            "-s",
            source.as_str(),
            "-t",
            target.as_str(),
        ])
        .map(|_| ())
    }
    fn resize_pane(&self, id: &PaneId, direction: ResizeDirection, amount: u16) -> TmuxResult<()> {
        if amount == 0 {
            return Ok(());
        }
        let amt = amount.to_string();
        self.checked(&["resize-pane", "-t", id.as_str(), direction.flag(), &amt])
            .map(|_| ())
    }
    fn resize_pane_percent(
        &self,
        id: &PaneId,
        width_pct: Option<u8>,
        height_pct: Option<u8>,
    ) -> TmuxResult<()> {
        self.resize_pane_to(id, PaneTargetSize::percent(width_pct, height_pct))
    }
    fn resize_pane_to(&self, id: &PaneId, size: PaneTargetSize) -> TmuxResult<()> {
        if size.is_empty() {
            return Ok(());
        }
        let width_arg = size.width.map(super::PaneDimension::to_tmux_arg);
        let height_arg = size.height.map(super::PaneDimension::to_tmux_arg);
        let mut a = vec!["resize-pane", "-t", id.as_str()];
        if let Some(ref w) = width_arg {
            a.extend(["-x", w.as_str()]);
        }
        if let Some(ref h) = height_arg {
            a.extend(["-y", h.as_str()]);
        }
        self.checked(&a).map(|_| ())
    }
    fn respawn_pane(&self, id: &PaneId, kill_existing: bool) -> TmuxResult<()> {
        if kill_existing {
            self.checked(&["respawn-pane", "-k", "-t", id.as_str()])
                .map(|_| ())
        } else {
            self.checked(&["respawn-pane", "-t", id.as_str()])
                .map(|_| ())
        }
    }
    fn clear_pane_history(&self, id: &PaneId) -> TmuxResult<()> {
        self.checked(&["clear-history", "-t", id.as_str()])
            .map(|_| ())
    }
    fn send_keys(&self, id: &PaneId, keys: &[&str], literal: bool) -> TmuxResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut a = Vec::with_capacity(keys.len() + 5);
        a.extend(["send-keys", "-t", id.as_str()]);
        if literal {
            a.push("-l");
        }
        a.push("--");
        a.extend_from_slice(keys);
        self.checked(&a).map(|_| ())
    }
    fn send_paste(&self, id: &PaneId, text: &str) -> TmuxResult<()> {
        if text.is_empty() {
            return Ok(());
        }
        static PIP_PASTE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = PIP_PASTE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let buf_name = format!("justmux-pip-{}-{seq}", std::process::id());
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.checked_with_stdin(
            &["load-buffer", "-b", &buf_name, "-"],
            normalized.as_bytes(),
        )?;
        match self.checked(&[
            "paste-buffer",
            "-d",
            "-p",
            "-b",
            &buf_name,
            "-t",
            id.as_str(),
        ]) {
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = self.delete_buffer(&buf_name);
                Err(err)
            }
        }
    }
    fn rename_pane_title(&self, id: &PaneId, title: &str) -> TmuxResult<()> {
        self.checked(&["select-pane", "-t", id.as_str(), "-T", title])
            .map(|_| ())
    }
    fn toggle_pipe_pane(&self, id: &PaneId, log_path: Option<&Path>) -> TmuxResult<()> {
        match log_path {
            Some(path) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.is_dir()
                {
                    return Err(TmuxError::DirectoryNotFound(parent.to_path_buf()));
                }
                let raw = path.to_str().ok_or_else(|| {
                    TmuxError::InvalidName("log file path is not valid UTF-8".into())
                })?;
                if raw.is_empty() || raw.contains(['\0', '\n', '\r']) {
                    return Err(TmuxError::InvalidName(
                        "log file path cannot be empty or contain line breaks".into(),
                    ));
                }
                let cmd = format!("cat >> {}", shell_single_quote(raw));
                self.checked(&["pipe-pane", "-o", "-t", id.as_str(), &cmd])
                    .map(|_| ())
            }
            None => self.checked(&["pipe-pane", "-t", id.as_str()]).map(|_| ()),
        }
    }
    fn capture_pane(&self, target: &Target, max_lines: usize) -> TmuxResult<Vec<String>> {
        if max_lines == 0 {
            return Ok(Vec::new());
        }
        let target = target.to_string();
        let start = format!("-{}", max_lines.min(2_000));
        let stdout = self.text(&["capture-pane", "-p", "-e", "-S", &start, "-t", &target])?;
        Ok(sanitize_captured_sgr_lines(&stdout, max_lines))
    }
    fn capture_pane_plain(&self, target: &Target, max_lines: usize) -> TmuxResult<Vec<String>> {
        if max_lines == 0 {
            return Ok(Vec::new());
        }
        let target = target.to_string();
        let start = format!("-{}", max_lines.min(2_000));
        let stdout = self.text(&["capture-pane", "-p", "-S", &start, "-t", &target])?;
        Ok(sanitize_captured_lines(&stdout, max_lines))
    }
    fn list_buffers(&self) -> TmuxResult<Vec<PasteBuffer>> {
        parser::parse_buffers(&self.list_with(&["list-buffers", "-F", BUFFER_FORMAT])?)
    }
    fn show_buffer(&self, name: &str) -> TmuxResult<String> {
        self.text(&["show-buffer", "-b", name])
    }
    fn set_buffer(&self, content: &str) -> TmuxResult<()> {
        self.checked_with_stdin(&["load-buffer", "-"], content.as_bytes())
            .map(|_| ())
    }
    fn paste_buffer(&self, name: &str, target: &PaneId) -> TmuxResult<()> {
        self.checked(&["paste-buffer", "-b", name, "-t", target.as_str()])
            .map(|_| ())
    }
    fn delete_buffer(&self, name: &str) -> TmuxResult<()> {
        self.checked(&["delete-buffer", "-b", name]).map(|_| ())
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
    fn list_clients(&self) -> TmuxResult<Vec<ClientInfo>> {
        parser::parse_clients(&self.list_with(&["list-clients", "-F", CLIENT_FORMAT])?)
    }
    fn detach_other_clients(
        &self,
        session: Option<&SessionId>,
        keep_client: Option<&str>,
    ) -> TmuxResult<usize> {
        let keep_owned = match keep_client {
            Some(client) => Some(client.to_owned()),
            None => match self.context() {
                Context::Inside(InsideContext {
                    client_name: Some(current),
                    ..
                }) => Some(current),
                _ => None,
            },
        };
        let clients = self.list_clients()?;
        let mut detached = 0;
        for client in clients {
            if let Some(sid) = session
                && client.session_id.as_ref() != Some(sid)
            {
                continue;
            }
            if keep_owned.as_deref() == Some(client.name.as_str()) {
                continue;
            }
            match self.detach_client(&client.name) {
                Ok(()) => detached += 1,
                Err(TmuxError::NoCurrentClient) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(detached)
    }
    fn attach_command(&self, target: &Target) -> Command {
        self.attach_command_impl(target, false)
    }
    fn attach_command_readonly(&self, target: &Target) -> Command {
        self.attach_command_impl(target, true)
    }
    fn source_file(&self, path: &Path) -> TmuxResult<()> {
        let raw = path
            .to_str()
            .ok_or_else(|| TmuxError::InvalidName("config file path is not valid UTF-8".into()))?;
        self.checked(&["source-file", raw]).map(|_| ())
    }
    fn kill_server(&self) -> TmuxResult<()> {
        self.checked(&["kill-server"]).map(|_| ())
    }
    fn display_popup_shell(&self, cwd: Option<&Path>) -> TmuxResult<()> {
        let Context::Inside(InsideContext {
            client_name: Some(client),
            ..
        }) = self.context()
        else {
            return Err(TmuxError::NoCurrentClient);
        };
        if !self.version()?.supports_popup() {
            return Err(TmuxError::CommandFailed {
                command: "display-popup".into(),
                message: "tmux 3.2 or newer is required for display-popup".into(),
            });
        }
        let mut a = vec![
            "display-popup",
            "-E",
            "-c",
            client.as_str(),
            "-w",
            "80%",
            "-h",
            "80%",
        ];
        if let Some(dir) = cwd {
            if !dir.is_dir() {
                return Err(TmuxError::DirectoryNotFound(dir.to_path_buf()));
            }
            let raw = dir.to_str().ok_or_else(|| {
                TmuxError::InvalidName("working directory is not valid UTF-8".into())
            })?;
            a.extend(["-d", raw]);
        }
        self.checked(&a).map(|_| ())
    }
    fn set_global_option(&self, option: &str, value: &str) -> TmuxResult<()> {
        self.checked(&["set-option", "-g", option, value])
            .map(|_| ())
    }
    fn set_global_window_option(&self, option: &str, value: &str) -> TmuxResult<()> {
        self.checked(&["set-window-option", "-g", option, value])
            .map(|_| ())
    }
    fn set_server_option(&self, option: &str, value: &str) -> TmuxResult<()> {
        self.checked(&["set-option", "-sg", option, value])
            .map(|_| ())
    }
    fn bind_popup_key(&self, run_shell_command: &str) -> TmuxResult<bool> {
        if !self.server_running()? {
            return Ok(false);
        }
        self.checked(&[
            "bind-key",
            "-T",
            "prefix",
            crate::tmux_conf::POPUP_KEY,
            "run-shell",
            run_shell_command,
        ])?;
        Ok(true)
    }
    fn unbind_popup_key(&self) -> TmuxResult<bool> {
        if !self.server_running()? {
            return Ok(false);
        }
        let out = self.run(&["list-keys", "-T", "prefix", crate::tmux_conf::POPUP_KEY])?;
        if !out.status.success() {
            return Ok(false);
        }
        let keys = String::from_utf8_lossy(&out.stdout);
        if !keys.contains("JUSTMUX_POPUP=1") {
            return Ok(false);
        }
        self.checked(&["unbind-key", "-T", "prefix", crate::tmux_conf::POPUP_KEY])?;
        Ok(true)
    }
}

fn same_socket(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// POSIX single-quote escaping for the command string tmux `pipe-pane` passes
/// to `/bin/sh -c` internally.
fn shell_single_quote(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('\'');
    for ch in raw.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Strip ANSI escape sequences and control characters from captured terminal
/// output, drop trailing completely empty lines, and return up to the last
/// `max_lines` lines.
pub fn sanitize_captured_lines(raw: &str, max_lines: usize) -> Vec<String> {
    sanitize_lines_with(raw, max_lines, sanitize_captured_line)
}

/// Sanitize captured terminal output while preserving valid SGR color and
/// attribute escapes (`\x1b[...m`), stripping unsafe CSI/OSC/ESC and control
/// sequences, collapsing SGR-only/blank lines to `""`, dropping trailing empty
/// lines, and returning up to the last `max_lines` lines.
pub fn sanitize_captured_sgr_lines(raw: &str, max_lines: usize) -> Vec<String> {
    sanitize_lines_with(raw, max_lines, sanitize_captured_sgr_line)
}

fn sanitize_lines_with(
    raw: &str,
    max_lines: usize,
    sanitize_line: fn(&str) -> String,
) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }

    let mut lines: Vec<String> = raw.lines().map(sanitize_line).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.len() > max_lines {
        let start = lines.len() - max_lines;
        lines.drain(..start);
    }
    lines
}

fn sanitize_captured_line(line: &str) -> String {
    sanitize_line_impl(line, false)
}

fn sanitize_captured_sgr_line(line: &str) -> String {
    sanitize_line_impl(line, true)
}

fn sanitize_line_impl(line: &str, keep_sgr: bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut has_visible_non_ws = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                // CSI: ESC [ <params/intermediates 0x20..=0x3f>* <final 0x40..=0x7e>
                Some('[') => {
                    chars.next();
                    let mut params = String::new();
                    let mut final_ch = None;
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii() && (0x20..=0x3f).contains(&(next as u8)) {
                            if keep_sgr {
                                params.push(next);
                            }
                            chars.next();
                        } else if next.is_ascii() && (0x40..=0x7e).contains(&(next as u8)) {
                            final_ch = Some(next);
                            chars.next();
                            break;
                        } else {
                            break;
                        }
                    }
                    if keep_sgr
                        && final_ch == Some('m')
                        && params
                            .bytes()
                            .all(|b| matches!(b, b'0'..=b'9' | b';' | b':'))
                    {
                        out.push_str("\x1b[");
                        out.push_str(&params);
                        out.push('m');
                    }
                }
                // OSC: ESC ] ... (BEL | ESC \ | C1 ST)
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' || next == '\u{009c}' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // 3-byte charset / line-attribute ESC sequences (e.g. ESC ( B, ESC ) 0, ESC # 8)
                Some('(' | ')' | '*' | '+' | '-' | '.' | '/' | '#') => {
                    chars.next();
                    if let Some(&final_ch) = chars.peek()
                        && final_ch.is_ascii()
                        && (0x30..=0x7e).contains(&(final_ch as u8))
                    {
                        chars.next();
                    }
                }
                // Simple 2-byte ESC sequence: ESC <0x20..=0x7e>
                Some(next) if next.is_ascii() && (0x20..=0x7e).contains(&(next as u8)) => {
                    chars.next();
                }
                _ => {}
            }
        } else if ch == '\t' {
            out.push_str("  ");
        } else if !ch.is_control() {
            if !ch.is_whitespace() {
                has_visible_non_ws = true;
            }
            out.push(ch);
        }
    }
    if keep_sgr {
        if !has_visible_non_ws {
            return String::new();
        }
        out.trim_end_matches(' ').to_owned()
    } else {
        let trimmed_len = out.trim_end().len();
        out.truncate(trimmed_len);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    static TEST_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct PrivateTmuxServer {
        socket: String,
        client: TmuxClient,
    }

    impl PrivateTmuxServer {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let seq = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
            let socket = format!("jm-tmux-cap-{}-{}-{}", std::process::id(), stamp, seq);
            let client =
                TmuxClient::new(SocketSpec::Name(socket.clone())).with_config_file("/dev/null");
            Self { socket, client }
        }

        fn tmux(&self, args: &[&str]) -> Output {
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

        fn wait_for_capture(&self, target: &Target, expected: &str) -> Vec<String> {
            self.wait_for_capture_lines(target, 20, expected)
        }

        fn wait_for_capture_lines(
            &self,
            target: &Target,
            max_lines: usize,
            expected: &str,
        ) -> Vec<String> {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let lines = self
                    .client
                    .capture_pane(target, max_lines)
                    .expect("capture_pane succeeded");
                if lines.iter().any(|line| line.contains(expected)) {
                    return lines;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {expected:?} in capture of {target:?}: {lines:?}"
                );
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    impl Drop for PrivateTmuxServer {
        fn drop(&mut self) {
            let _ = self.tmux(&["kill-server"]);
        }
    }

    #[test]
    fn sanitize_strips_csi_sgr_and_cursor_sequences() {
        let raw = "\x1b[2J\x1b[1;1H\x1b[?2004h\x1b[38;2;120;180;255;1m$ cargo test\x1b[0m\x1b[?2004l\x1b[K  \n\x1b[32mok\x1b[0m\n";
        assert_eq!(
            sanitize_captured_lines(raw, 10),
            vec!["$ cargo test".to_string(), "ok".to_string()]
        );
    }

    #[test]
    fn sanitize_strips_osc_and_simple_esc_sequences() {
        let raw = concat!(
            "\x1b]0;zsh - justmux\x07\x1bM\x1b7prompt>\x1b8 \n",
            "\x1b]8;;https://example.com\x1b\\open docs\x1b]8;;\x1b\\\n",
            "\x1b(Bplain\x1b)0 text\x1b=\x1b>\x1b\n",
            "\x1b]2;unterminated title\n",
            "next line survives\n",
        );
        assert_eq!(
            sanitize_captured_lines(raw, 10),
            vec![
                "prompt>".to_string(),
                "open docs".to_string(),
                "plain text".to_string(),
                "".to_string(),
                "next line survives".to_string(),
            ]
        );
    }

    #[test]
    fn sanitize_handles_tabs_controls_whitespace_and_utf8_cjk() {
        let raw = "  \x1b[36m项目进度:\x1b[0m\t🚀 100% │ 完成\x07\x08\x00\x7f\u{0085}   \r\n\tsecond\tline\t \n";
        assert_eq!(
            sanitize_captured_lines(raw, 10),
            vec![
                "  项目进度:  🚀 100% │ 完成".to_string(),
                "  second  line".to_string(),
            ]
        );
    }

    #[test]
    fn sanitize_drops_trailing_blanks_keeps_context_and_truncates() {
        let raw = "line-1\n\nline-3\nline-4\n\n   \n\t\n";
        assert_eq!(
            sanitize_captured_lines(raw, 10),
            vec![
                "line-1".to_string(),
                "".to_string(),
                "line-3".to_string(),
                "line-4".to_string(),
            ]
        );
        assert_eq!(
            sanitize_captured_lines(raw, 2),
            vec!["line-3".to_string(), "line-4".to_string()]
        );
        assert_eq!(
            sanitize_captured_lines(raw, 3),
            vec!["".to_string(), "line-3".to_string(), "line-4".to_string()]
        );
        assert!(sanitize_captured_lines(raw, 0).is_empty());
        assert!(sanitize_captured_lines("\n   \n\t\r\n", 10).is_empty());
    }

    #[test]
    fn capture_pane_reads_session_window_and_pane_on_private_socket() {
        let server = PrivateTmuxServer::new();

        let missing_server_target = Target::Session(SessionId::new("$0").expect("session id"));
        assert!(matches!(
            server.client.capture_pane(&missing_server_target, 5),
            Err(TmuxError::NoServer)
        ));

        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "alpha",
            "-x",
            "80",
            "-y",
            "12",
            "sh -c \"printf 'alpha-one\\n\\nalpha-two   \\n'; sleep 30\"",
        ]);
        assert!(
            out.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let sessions = server.client.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        let session_id = sessions[0].id.clone();

        let windows = server
            .client
            .list_windows(Some(&session_id))
            .expect("list windows");
        let win0_id = windows[0].id.clone();
        let pane0_id = server
            .client
            .list_panes(Some(&win0_id))
            .expect("list panes")[0]
            .id
            .clone();

        let out = server.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            session_id.as_str(),
            "-n",
            "beta",
            "sh -c \"printf 'beta-one\\nbeta-two\\n'; sleep 30\"",
        ]);
        assert!(
            out.status.success(),
            "new-window failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let win1_id = WindowId::new(String::from_utf8_lossy(&out.stdout).trim().to_string())
            .expect("window id");

        let out = server.tmux(&[
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            pane0_id.as_str(),
            "sh -c \"printf 'split-one\\nsplit-two\\n'; sleep 30\"",
        ]);
        assert!(
            out.status.success(),
            "split-window failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let pane1_id =
            PaneId::new(String::from_utf8_lossy(&out.stdout).trim().to_string()).expect("pane id");

        let session_target = Target::Session(session_id.clone());
        let window_target = Target::Window(session_id.clone(), win1_id.clone());
        let pane_target = Target::Pane(pane1_id);

        let session_lines = server.wait_for_capture(&session_target, "alpha-two");
        assert_eq!(session_lines, vec!["alpha-one", "", "alpha-two"]);

        let window_lines = server.wait_for_capture(&window_target, "beta-two");
        assert_eq!(window_lines, vec!["beta-one", "beta-two"]);

        let pane_lines = server.wait_for_capture(&pane_target, "split-two");
        assert_eq!(pane_lines, vec!["split-one", "split-two"]);

        assert_eq!(
            server
                .client
                .capture_pane(&window_target, 1)
                .expect("max_lines=1"),
            vec!["beta-two"]
        );
        assert!(
            server
                .client
                .capture_pane(&window_target, 0)
                .expect("max_lines=0")
                .is_empty()
        );

        assert!(matches!(
            server
                .client
                .capture_pane(&Target::Session(SessionId::new("$9999").expect("id")), 5),
            Err(TmuxError::SessionNotFound)
        ));
        assert!(matches!(
            server.client.capture_pane(
                &Target::Window(session_id, WindowId::new("@9999").expect("id")),
                5
            ),
            Err(TmuxError::WindowNotFound)
        ));
        assert!(matches!(
            server
                .client
                .capture_pane(&Target::Pane(PaneId::new("%9999").expect("id")), 5),
            Err(TmuxError::PaneNotFound)
        ));
    }

    #[test]
    fn attach_command_clears_justmux_and_tmux_env_vars() {
        let client = TmuxClient::new(SocketSpec::Name("jm-env-test".into()));
        let cmd = client.attach_command(&Target::Session(SessionId::new("$1").expect("id")));
        let removed: Vec<&std::ffi::OsStr> = cmd
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then_some(key))
            .collect();
        for var in [
            "TMUX",
            "TMUX_PANE",
            "JUSTMUX_CLIENT",
            "JUSTMUX_PANE",
            "JUSTMUX_POPUP",
            "JUSTMUX_INVOKED_AS",
            "TMUX_POPUP",
        ] {
            assert!(
                removed.contains(&std::ffi::OsStr::new(var)),
                "attach_command should remove {var}"
            );
        }
    }

    #[test]
    fn capture_pane_includes_scrollback_history_on_private_socket() {
        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "scroll",
            "-x",
            "80",
            "-y",
            "10",
            "sh -c 'i=1; while [ \"$i\" -le 40 ]; do printf \"LINE_%02d\\n\" \"$i\"; i=$((i+1)); done; sleep 30'",
        ]);
        assert!(
            out.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let sessions = server.client.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        let target = Target::Session(sessions[0].id.clone());

        let expected_all: Vec<String> = (1..=40).map(|i| format!("LINE_{i:02}")).collect();
        let captured_all = server.wait_for_capture_lines(&target, 200, "LINE_40");
        assert_eq!(captured_all, expected_all);

        let expected_tail: Vec<String> = (36..=40).map(|i| format!("LINE_{i:02}")).collect();
        let captured_tail = server
            .client
            .capture_pane(&target, 5)
            .expect("capture_pane tail");
        assert_eq!(captured_tail, expected_tail);
    }

    #[test]
    fn sanitize_sgr_preserves_colors_and_strips_unsafe_sequences() {
        let raw = concat!(
            "\x1b[2J\x1b[1;1H\x1b[?2004h\x1b]0;title\x07\x1bM\x1b(B",
            "\x1b[1;32mGREEN\x1b[0m\t\x1b[38;5;208m256\x1b[m  \n",
            "\x1b[38;2;120;180;255m\x1b[K   \x1b[0m\t\x07\x08\n",
            "\x1b[38;2;120;180;255mRGB\x1b[0m \x1b[38:2::255:0:128mCOLON\x1b[0m\x1b[>4;2m\r\n",
            "\x1b[31m   \x1b[0m   \n",
        );
        assert_eq!(
            sanitize_captured_sgr_lines(raw, 10),
            vec![
                "\x1b[1;32mGREEN\x1b[0m  \x1b[38;5;208m256\x1b[m".to_string(),
                String::new(),
                "\x1b[38;2;120;180;255mRGB\x1b[0m \x1b[38:2::255:0:128mCOLON\x1b[0m".to_string(),
            ]
        );
        assert_eq!(
            sanitize_captured_sgr_lines(raw, 1),
            vec!["\x1b[38;2;120;180;255mRGB\x1b[0m \x1b[38:2::255:0:128mCOLON\x1b[0m".to_string()]
        );
        assert!(sanitize_captured_sgr_lines(raw, 0).is_empty());
        assert!(sanitize_captured_sgr_lines("\x1b[32m   \x1b[0m\n", 5).is_empty());
    }

    #[test]
    fn capture_pane_preserves_sgr_colors_on_private_socket() {
        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "colors",
            "-x",
            "80",
            "-y",
            "8",
            "sh -c 'printf \"\\033[1;32mGREEN_OK\\033[0m \\033[38;2;120;180;255mRGB_OK\\033[0m\\n\"; sleep 30'",
        ]);
        assert!(
            out.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let sessions = server.client.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        let target = Target::Session(sessions[0].id.clone());

        let lines = server.wait_for_capture_lines(&target, 20, "RGB_OK");
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.contains("\x1b["), "expected SGR escape in {line:?}");
        assert!(line.contains("32m"), "expected green SGR in {line:?}");
        assert!(
            line.contains("38;2;120;180;255m"),
            "expected RGB SGR in {line:?}"
        );
        assert!(line.contains("GREEN_OK"), "missing GREEN_OK in {line:?}");
        assert!(line.contains("RGB_OK"), "missing RGB_OK in {line:?}");
    }

    #[test]
    fn shell_single_quote_escapes_metacharacters_safely() {
        assert_eq!(shell_single_quote("/tmp/simple.log"), "'/tmp/simple.log'");
        assert_eq!(
            shell_single_quote("/tmp/a'b $HOME `id`; rm -rf /"),
            "'/tmp/a'\\''b $HOME `id`; rm -rf /'"
        );
    }

    #[test]
    fn attach_command_readonly_sets_readonly_flag_and_clears_env() {
        let client = TmuxClient::new(SocketSpec::Name("jm-ro-test".into()));
        let cmd =
            client.attach_command_readonly(&Target::Session(SessionId::new("$2").expect("id")));
        let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
        assert!(
            args.windows(4)
                .any(|w| w == ["attach-session", "-r", "-t", "$2"]),
            "expected attach-session -r -t $2 in {args:?}"
        );
        let removed: Vec<&std::ffi::OsStr> = cmd
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then_some(key))
            .collect();
        for var in [
            "TMUX",
            "TMUX_PANE",
            "JUSTMUX_CLIENT",
            "JUSTMUX_PANE",
            "JUSTMUX_POPUP",
            "JUSTMUX_INVOKED_AS",
            "TMUX_POPUP",
        ] {
            assert!(
                removed.contains(&std::ffi::OsStr::new(var)),
                "attach_command_readonly should remove {var}"
            );
        }
    }

    #[test]
    fn zoom_break_swap_resize_and_layouts_on_private_socket() {
        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "layout-demo",
            "-n",
            "main",
            "-x",
            "100",
            "-y",
            "30",
            "sh -c 'sleep 30'",
        ]);
        assert!(
            out.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let sessions = server.client.list_sessions().expect("sessions");
        let sid = sessions[0].id.clone();
        let windows = server.client.list_windows(Some(&sid)).expect("windows");
        assert_eq!(windows.len(), 1);
        let wid = windows[0].id.clone();
        assert!(!windows[0].zoomed);
        assert!(!windows[0].synchronized);
        assert!(!windows[0].layout.is_empty());

        let panes = server.client.list_panes(Some(&wid)).expect("panes");
        assert_eq!(panes.len(), 1);
        assert_eq!(
            (panes[0].width, panes[0].height, panes[0].left, panes[0].top),
            (100, 30, 0, 0)
        );
        let p0 = panes[0].id.clone();

        // Split with explicit cwd via split_pane_in.
        let p1 = server
            .client
            .split_pane_in(&p0, SplitDirection::SideBySide, Some(Path::new("/tmp")))
            .expect("split_pane_in");
        assert!(p1.left > 0, "right split should have left > 0: {p1:?}");

        // Rename pane titles and swap panes.
        server
            .client
            .rename_pane_title(&p0, "left-pane")
            .expect("title p0");
        server
            .client
            .rename_pane_title(&p1.id, "right-pane")
            .expect("title p1");
        let before_swap = server.client.list_panes(Some(&wid)).expect("before swap");
        let p0_before = before_swap.iter().find(|p| p.id == p0).expect("p0 before");
        assert_eq!(p0_before.title, "left-pane");
        let p0_left_before = p0_before.left;

        server.client.swap_pane(&p0, &p1.id).expect("swap_pane");
        let after_swap = server.client.list_panes(Some(&wid)).expect("after swap");
        let p0_after = after_swap.iter().find(|p| p.id == p0).expect("p0 after");
        assert_ne!(p0_after.left, p0_left_before, "swap_pane should move p0");

        // Zoom toggle on window and pane flags.
        server.client.toggle_zoom_pane(&p0).expect("zoom on");
        let w_zoomed = server.client.list_windows(Some(&sid)).expect("w zoomed");
        assert!(w_zoomed[0].zoomed);
        let p_zoomed = server.client.list_panes(Some(&wid)).expect("p zoomed");
        assert!(p_zoomed.iter().all(|p| p.zoomed));
        server.client.toggle_zoom_pane(&p0).expect("zoom off");
        let w_unzoomed = server.client.list_windows(Some(&sid)).expect("w unzoomed");
        assert!(!w_unzoomed[0].zoomed);

        // Save layout, switch to EvenVertical, then restore raw layout.
        let saved_layout = w_unzoomed[0].layout.clone();
        server
            .client
            .select_layout(&wid, LayoutPreset::EvenVertical)
            .expect("even-vertical");
        let stacked = server.client.list_panes(Some(&wid)).expect("stacked");
        assert!(
            stacked.iter().any(|p| p.top > 0),
            "even-vertical should stack panes vertically: {stacked:?}"
        );
        server
            .client
            .select_layout_raw(&wid, &saved_layout)
            .expect("restore raw layout");
        let restored = server.client.list_panes(Some(&wid)).expect("restored");
        assert!(
            restored.iter().all(|p| p.top == 0),
            "restored horizontal layout should put panes on top=0: {restored:?}"
        );

        // Percentage resize and incremental resize.
        server
            .client
            .resize_pane_percent(&p0, Some(70), None)
            .expect("resize 70%");
        let after_pct = server.client.list_panes(Some(&wid)).expect("after pct");
        let w_pct = after_pct.iter().find(|p| p.id == p0).expect("p0").width;
        assert!(
            (65..=75).contains(&w_pct),
            "expected ~70% width of 100 cols, got {w_pct}"
        );
        server
            .client
            .resize_pane(&p0, ResizeDirection::Left, 5)
            .expect("resize step");
        let after_step = server.client.list_panes(Some(&wid)).expect("after step");
        let w_step = after_step.iter().find(|p| p.id == p0).expect("p0").width;
        assert_ne!(w_step, w_pct, "incremental resize should adjust pane width");

        // Break pane out into a new named window.
        let broken = server
            .client
            .break_pane(&p1.id, Some("broken-out"))
            .expect("break_pane");
        assert_eq!(broken.name, "broken-out");
        assert_eq!(broken.panes, 1);
        assert_eq!(
            server
                .client
                .list_panes(Some(&wid))
                .expect("remaining panes")
                .len(),
            1
        );
    }

    #[test]
    fn window_move_swap_link_sync_and_monitoring_on_private_socket() {
        let server = PrivateTmuxServer::new();
        let s1 = server
            .client
            .new_session("alpha", Path::new("/tmp"))
            .expect("s1");
        let s2 = server
            .client
            .new_session("beta", Path::new("/tmp"))
            .expect("s2");
        let w_first = server.client.list_windows(Some(&s1.id)).expect("w1")[0].clone();
        let w_second = server
            .client
            .new_window(&s1.id, Some("second"), None)
            .expect("w2");

        // Synchronize-panes + activity/silence monitoring flags.
        server
            .client
            .set_synchronize_panes(&w_second.id, true)
            .expect("sync on");
        server
            .client
            .set_monitor_activity(&w_second.id, true)
            .expect("monitor-activity on");
        server
            .client
            .set_monitor_silence(&w_second.id, 45)
            .expect("monitor-silence 45");
        let wins = server.client.list_windows(Some(&s1.id)).expect("wins");
        let updated = wins.iter().find(|w| w.id == w_second.id).expect("updated");
        assert!(updated.synchronized);
        assert!(updated.monitor_activity);
        assert_eq!(updated.monitor_silence, 45);
        let panes = server.client.list_panes(Some(&w_second.id)).expect("panes");
        assert!(panes[0].synchronized);

        server
            .client
            .set_synchronize_panes(&w_second.id, false)
            .expect("sync off");
        server
            .client
            .set_monitor_activity(&w_second.id, false)
            .expect("monitor-activity off");
        server
            .client
            .set_monitor_silence(&w_second.id, 0)
            .expect("monitor-silence 0");
        let wins = server.client.list_windows(Some(&s1.id)).expect("wins off");
        let updated = wins.iter().find(|w| w.id == w_second.id).expect("updated");
        assert!(!updated.synchronized);
        assert!(!updated.monitor_activity);
        assert_eq!(updated.monitor_silence, 0);

        // Swap window positions inside s1.
        let idx_first_before = w_first.index;
        let idx_second_before = w_second.index;
        server
            .client
            .swap_window(&w_first.id, &w_second.id)
            .expect("swap_window");
        let wins = server
            .client
            .list_windows(Some(&s1.id))
            .expect("after swap");
        assert_eq!(
            wins.iter().find(|w| w.id == w_first.id).expect("w1").index,
            idx_second_before
        );
        assert_eq!(
            wins.iter().find(|w| w.id == w_second.id).expect("w2").index,
            idx_first_before
        );

        // Move w_second to s2, then link it back into s1.
        server
            .client
            .move_window(&w_second.id, &s2.id)
            .expect("move_window");
        assert!(
            server
                .client
                .list_windows(Some(&s2.id))
                .expect("s2 wins")
                .iter()
                .any(|w| w.id == w_second.id)
        );
        assert!(
            !server
                .client
                .list_windows(Some(&s1.id))
                .expect("s1 wins")
                .iter()
                .any(|w| w.id == w_second.id)
        );

        server
            .client
            .link_window(&w_second.id, &s1.id)
            .expect("link_window");
        assert!(
            server
                .client
                .list_windows(Some(&s1.id))
                .expect("s1 after link")
                .iter()
                .any(|w| w.id == w_second.id)
        );
        assert!(
            server
                .client
                .list_windows(Some(&s2.id))
                .expect("s2 after link")
                .iter()
                .any(|w| w.id == w_second.id)
        );
    }

    #[test]
    fn pane_plain_capture_send_keys_history_respawn_and_pipe_pane() {
        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "pane-ops",
            "-x",
            "80",
            "-y",
            "8",
            "sh",
        ]);
        assert!(
            out.status.success(),
            "new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let pane = server.client.list_panes(None).expect("panes")[0].clone();
        let target = Target::Pane(pane.id.clone());

        // Emit colored output and verify capture_pane_plain strips SGR codes.
        server
            .client
            .send_keys(
                &pane.id,
                &[r#"printf '\033[1;31mPLAIN_MARKER\033[0m\n'"#],
                true,
            )
            .expect("send literal");
        server
            .client
            .send_keys(&pane.id, &["Enter"], false)
            .expect("send enter");
        let _ = server.wait_for_capture(&target, "PLAIN_MARKER");
        let plain = server
            .client
            .capture_pane_plain(&target, 50)
            .expect("plain capture");
        let marker_line = plain
            .iter()
            .find(|l| l.contains("PLAIN_MARKER"))
            .expect("plain marker");
        assert!(
            !marker_line.contains("\x1b["),
            "plain capture must not include ANSI escapes: {marker_line:?}"
        );

        // Pipe-pane to a filename with spaces and single quote inside a temp dir.
        let dir = std::env::temp_dir().join(format!("jm-pipe-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("pane's log.txt");
        let _ = std::fs::remove_file(&log_path);

        assert!(matches!(
            server
                .client
                .toggle_pipe_pane(&pane.id, Some(Path::new("/nonexistent-jm-dir-xyz/a.log"))),
            Err(TmuxError::DirectoryNotFound(_))
        ));
        assert!(matches!(
            server
                .client
                .toggle_pipe_pane(&pane.id, Some(&dir.join("bad\nname.log"))),
            Err(TmuxError::InvalidName(_))
        ));

        server
            .client
            .toggle_pipe_pane(&pane.id, Some(&log_path))
            .expect("pipe start");
        server
            .client
            .send_keys(&pane.id, &["echo PIPE_CAPTURED_987"], true)
            .expect("send pipe marker");
        server
            .client
            .send_keys(&pane.id, &["Enter"], false)
            .expect("send enter");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(content) = std::fs::read_to_string(&log_path)
                && content.contains("PIPE_CAPTURED_987")
            {
                break;
            }
            assert!(Instant::now() < deadline, "timed out waiting for pipe log");
            thread::sleep(Duration::from_millis(25));
        }
        server
            .client
            .toggle_pipe_pane(&pane.id, None)
            .expect("pipe stop");
        let _ = std::fs::remove_file(&log_path);
        let _ = std::fs::remove_dir(&dir);

        // Scrollback clear-history.
        server
            .client
            .send_keys(
                &pane.id,
                &["i=1; while [ $i -le 30 ]; do echo HIST_ROW_$i; i=$((i+1)); done"],
                true,
            )
            .expect("emit scrollback");
        server
            .client
            .send_keys(&pane.id, &["Enter"], false)
            .expect("enter");
        let _ = server.wait_for_capture_lines(&target, 100, "HIST_ROW_30");
        assert!(
            server
                .client
                .capture_pane_plain(&target, 100)
                .expect("before clear")
                .iter()
                .any(|l| l.contains("HIST_ROW_1"))
        );
        server
            .client
            .clear_pane_history(&pane.id)
            .expect("clear_pane_history");
        assert!(
            !server
                .client
                .capture_pane_plain(&target, 100)
                .expect("after clear")
                .iter()
                .any(|l| l.contains("HIST_ROW_1")),
            "HIST_ROW_1 should be cleared from scrollback"
        );

        // Respawn pane with kill_existing=true changes PID.
        let old_pid = pane.pid;
        server
            .client
            .respawn_pane(&pane.id, true)
            .expect("respawn_pane");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let refreshed = server.client.list_panes(None).expect("panes after respawn");
            if refreshed[0].pid != old_pid {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "pane pid did not change after respawn"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn paste_buffers_crud_and_paste_on_private_socket() {
        let server = PrivateTmuxServer::new();
        assert!(
            server
                .client
                .list_buffers()
                .expect("empty on no server")
                .is_empty()
        );

        let out = server.tmux(&["new-session", "-d", "-s", "bufs", "sh"]);
        assert!(out.status.success());
        let pane_id = server.client.list_panes(None).expect("panes")[0].id.clone();

        let payload = "-n hello\t世界\nsecond line";
        server.client.set_buffer(payload).expect("set_buffer");
        let buffers = server.client.list_buffers().expect("list_buffers");
        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].size, payload.len());
        let buf_name = buffers[0].name.clone();
        assert_eq!(
            server.client.show_buffer(&buf_name).expect("show_buffer"),
            payload
        );

        // Push a simple command buffer and paste into pane without deleting buffer.
        server
            .client
            .set_buffer("echo PASTED_BUFFER_OK\n")
            .expect("set cmd buffer");
        let buffers = server.client.list_buffers().expect("list 2 buffers");
        assert_eq!(buffers.len(), 2);
        let cmd_buf = buffers[0].name.clone();
        server
            .client
            .paste_buffer(&cmd_buf, &pane_id)
            .expect("paste_buffer");
        let _ = server.wait_for_capture(&Target::Pane(pane_id), "PASTED_BUFFER_OK");
        assert_eq!(
            server.client.list_buffers().expect("preserved").len(),
            2,
            "paste_buffer must preserve the buffer"
        );

        server
            .client
            .delete_buffer(&cmd_buf)
            .expect("delete_buffer");
        assert_eq!(server.client.list_buffers().expect("after delete").len(), 1);
    }

    #[test]
    fn clients_options_source_file_popup_and_kill_server_on_private_socket() {
        struct ControlClient(std::process::Child);
        impl Drop for ControlClient {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let server = PrivateTmuxServer::new();
        assert!(
            server
                .client
                .list_clients()
                .expect("clients on no server")
                .is_empty()
        );
        assert!(matches!(
            server.client.display_popup_shell(None),
            Err(TmuxError::NoCurrentClient)
        ));

        let s1 = server
            .client
            .new_session("one", Path::new("/tmp"))
            .expect("s1");
        let s2 = server
            .client
            .new_session("two", Path::new("/tmp"))
            .expect("s2");

        let spawn_client = |target: &SessionId, readonly: bool| -> ControlClient {
            let mut cmd = Command::new("tmux");
            cmd.arg("-L")
                .arg(&server.socket)
                .args(["-f", "/dev/null", "-C", "attach-session"]);
            if readonly {
                cmd.arg("-r");
            }
            cmd.args(["-t", target.as_str()])
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            ControlClient(cmd.spawn().expect("spawn control client"))
        };

        let _c1 = spawn_client(&s1.id, false);
        let _c2 = spawn_client(&s1.id, true);
        let _c3 = spawn_client(&s2.id, false);

        let deadline = Instant::now() + Duration::from_secs(3);
        let clients = loop {
            let list = server.client.list_clients().expect("list_clients");
            if list.len() == 3 {
                break list;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for 3 clients: {list:?}"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert!(
            clients
                .iter()
                .any(|c| c.readonly && c.session_id.as_ref() == Some(&s1.id))
        );
        assert!(clients.iter().all(|c| c.control_mode));

        let keep_name = clients
            .iter()
            .find(|c| !c.readonly && c.session_id.as_ref() == Some(&s1.id))
            .expect("rw client on s1")
            .name
            .clone();
        let detached = server
            .client
            .detach_other_clients(Some(&s1.id), Some(&keep_name))
            .expect("detach_other_clients");
        assert_eq!(detached, 1);

        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let remaining = server.client.list_clients().expect("remaining clients");
            if remaining.len() == 2 {
                assert!(remaining.iter().any(|c| c.name == keep_name));
                assert!(
                    remaining
                        .iter()
                        .any(|c| c.session_id.as_ref() == Some(&s2.id))
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected 2 clients left: {remaining:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }

        // Global, window, and server options + source_file.
        server
            .client
            .set_global_option("mouse", "on")
            .expect("set -g mouse");
        server
            .client
            .set_global_window_option("pane-base-index", "1")
            .expect("set -gw pane-base-index");
        server
            .client
            .set_server_option("escape-time", "0")
            .expect("set -sg escape-time");
        assert_eq!(
            String::from_utf8_lossy(&server.tmux(&["show-options", "-gv", "mouse"]).stdout).trim(),
            "on"
        );
        assert_eq!(
            String::from_utf8_lossy(
                &server
                    .tmux(&["show-window-options", "-gv", "pane-base-index"])
                    .stdout
            )
            .trim(),
            "1"
        );
        assert_eq!(
            String::from_utf8_lossy(&server.tmux(&["show-options", "-sgv", "escape-time"]).stdout)
                .trim(),
            "0"
        );

        let conf_path =
            std::env::temp_dir().join(format!("jm-source-test-{}.conf", std::process::id()));
        std::fs::write(&conf_path, "set -g history-limit 12345\n").expect("write temp conf");
        server.client.source_file(&conf_path).expect("source_file");
        let _ = std::fs::remove_file(&conf_path);
        assert_eq!(
            String::from_utf8_lossy(
                &server
                    .tmux(&["show-options", "-gv", "history-limit"])
                    .stdout
            )
            .trim(),
            "12345"
        );

        // kill_server shuts down the private server cleanly.
        server.client.kill_server().expect("kill_server");
        assert!(!server.client.server_running().expect("server_running"));
        assert!(matches!(
            server.client.kill_server(),
            Err(TmuxError::NoServer)
        ));
    }

    #[test]
    fn join_pane_ordered_places_before_left_top_and_after_right_bottom_on_private_socket() {
        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "dock-test",
            "-n",
            "anchor",
            "-x",
            "120",
            "-y",
            "40",
            "sh -c 'sleep 30'",
        ]);
        assert!(out.status.success());
        let sid = server.client.list_sessions().expect("s")[0].id.clone();
        let anchor_win = server.client.list_windows(Some(&sid)).expect("w")[0]
            .id
            .clone();
        let anchor_pane = server.client.list_panes(Some(&anchor_win)).expect("p")[0]
            .id
            .clone();

        let new_detached_pane = |name: &str| -> PaneId {
            let win = server
                .client
                .new_window(&sid, Some(name), None)
                .expect("new win");
            server.client.list_panes(Some(&win.id)).expect("pane")[0]
                .id
                .clone()
        };

        let left_pane = new_detached_pane("left");
        let right_pane = new_detached_pane("right");
        let top_pane = new_detached_pane("top");
        let bottom_pane = new_detached_pane("bottom");

        // Dock `left_pane` BEFORE anchor horizontally -> left_pane.left < anchor.left.
        server
            .client
            .join_pane_ordered(&left_pane, &anchor_pane, SplitDirection::SideBySide, true)
            .expect("join left");
        let panes = server
            .client
            .list_panes(Some(&anchor_win))
            .expect("after left");
        let p_left = panes.iter().find(|p| p.id == left_pane).expect("left");
        let p_anchor = panes.iter().find(|p| p.id == anchor_pane).expect("anchor");
        assert!(
            p_left.left < p_anchor.left,
            "before=true horizontal should dock left: left={p_left:?} anchor={p_anchor:?}"
        );

        // Dock `right_pane` AFTER anchor horizontally -> right_pane.left > anchor.left.
        server
            .client
            .join_pane_ordered(&right_pane, &anchor_pane, SplitDirection::SideBySide, false)
            .expect("join right");
        let panes = server
            .client
            .list_panes(Some(&anchor_win))
            .expect("after right");
        let p_right = panes.iter().find(|p| p.id == right_pane).expect("right");
        let p_anchor = panes.iter().find(|p| p.id == anchor_pane).expect("anchor");
        assert!(
            p_right.left > p_anchor.left,
            "before=false horizontal should dock right: right={p_right:?} anchor={p_anchor:?}"
        );

        // Dock `top_pane` BEFORE anchor vertically -> top_pane.top < anchor.top.
        server
            .client
            .join_pane_ordered(&top_pane, &anchor_pane, SplitDirection::Stacked, true)
            .expect("join top");
        let panes = server
            .client
            .list_panes(Some(&anchor_win))
            .expect("after top");
        let p_top = panes.iter().find(|p| p.id == top_pane).expect("top");
        let p_anchor = panes.iter().find(|p| p.id == anchor_pane).expect("anchor");
        assert!(
            p_top.top < p_anchor.top,
            "before=true vertical should dock above: top={p_top:?} anchor={p_anchor:?}"
        );

        // Dock `bottom_pane` AFTER anchor vertically -> bottom_pane.top > anchor.top.
        server
            .client
            .join_pane_ordered(&bottom_pane, &anchor_pane, SplitDirection::Stacked, false)
            .expect("join bottom");
        let panes = server
            .client
            .list_panes(Some(&anchor_win))
            .expect("after bottom");
        let p_bottom = panes.iter().find(|p| p.id == bottom_pane).expect("bottom");
        let p_anchor = panes.iter().find(|p| p.id == anchor_pane).expect("anchor");
        assert!(
            p_bottom.top > p_anchor.top,
            "before=false vertical should dock below: bottom={p_bottom:?} anchor={p_anchor:?}"
        );
    }

    #[test]
    fn send_key_event_and_send_paste_execute_live_pip_input_without_polluting_buffers() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let server = PrivateTmuxServer::new();
        let out = server.tmux(&[
            "new-session",
            "-d",
            "-s",
            "pip-live",
            "-x",
            "80",
            "-y",
            "16",
            "sh",
        ]);
        assert!(out.status.success());
        let pane_id = server.client.list_panes(None).expect("pane")[0].id.clone();
        let target = Target::Pane(pane_id.clone());

        // Seed a user paste buffer and verify `send_paste` leaves it untouched.
        server
            .client
            .set_buffer("KEEP_USER_BUFFER")
            .expect("seed user buffer");

        // Type a command containing `;` and UTF-8 via `send_key_event` char-by-char + Enter.
        for ch in "echo PIP_KEY_OK; echo 中文_OK".chars() {
            server
                .client
                .send_key_event(
                    &pane_id,
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                )
                .expect("send char");
        }
        server
            .client
            .send_key_event(&pane_id, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("send enter");
        let lines = server.wait_for_capture(&target, "中文_OK");
        assert!(lines.iter().any(|l| l.contains("PIP_KEY_OK")));

        // Verify Ctrl+C interrupts a running command via `send_key_event`.
        server
            .client
            .send_paste(&pane_id, "sleep 30\n")
            .expect("paste sleep");
        thread::sleep(Duration::from_millis(80));
        server
            .client
            .send_key_event(
                &pane_id,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            )
            .expect("send ctrl-c");

        // Multi-line `send_paste` executes both lines and preserves user's paste buffers.
        server
            .client
            .send_paste(&pane_id, "echo PASTE_ONE\r\necho PASTE_TWO\n")
            .expect("send multi-line paste");
        let lines = server.wait_for_capture(&target, "PASTE_TWO");
        assert!(lines.iter().any(|l| l.contains("PASTE_ONE")));

        let bufs = server.client.list_buffers().expect("buffers intact");
        assert_eq!(bufs.len(), 1);
        assert_eq!(
            server.client.show_buffer(&bufs[0].name).expect("show"),
            "KEEP_USER_BUFFER"
        );

        // Sending paste to a missing pane cleans up its ephemeral buffer on error.
        let missing = PaneId::new("%99999").expect("id");
        assert!(matches!(
            server.client.send_paste(&missing, "hello"),
            Err(TmuxError::PaneNotFound)
        ));
        assert_eq!(
            server
                .client
                .list_buffers()
                .expect("no leaked temp buffer")
                .len(),
            1
        );
    }
}
