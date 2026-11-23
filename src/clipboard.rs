//! Native system clipboard, OSC 52, log export, and `pipe-pane` path helpers.
//!
//! Zero external dependencies: OS clipboard helpers are invoked directly via
//! [`std::process::Command`] with stdin piping (never through a shell), with
//! automatic fallback to tmux's paste buffer when running headless (e.g. SSH).

use std::{
    env, fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::tmux::Tmux;

/// Optional environment variable overriding the default directory for exported
/// pane logs and `pipe-pane` streams.
pub const LOG_DIR_ENV: &str = "JUSTMUX_LOG_DIR";

/// Where a clipboard copy operation landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardOutcome {
    /// Copied to the OS clipboard using the named native helper (e.g. `pbcopy`,
    /// `wl-copy`, `xclip`, `xsel`).
    SystemClipboard(&'static str),
    /// Fell back to tmux's internal paste buffer (e.g. headless SSH or no OS
    /// clipboard helper available).
    TmuxBuffer,
}

impl ClipboardOutcome {
    /// Human-readable short status label (English or Simplified Chinese).
    pub fn label(&self, zh: bool) -> String {
        match self {
            Self::SystemClipboard(tool) => {
                if zh {
                    format!("已复制到系统剪贴板 ({tool})")
                } else {
                    format!("Copied to system clipboard ({tool})")
                }
            }
            Self::TmuxBuffer => {
                if zh {
                    "已复制到 tmux 粘贴缓冲区".to_owned()
                } else {
                    "Copied to tmux paste buffer".to_owned()
                }
            }
        }
    }
}

impl fmt::Display for ClipboardOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label(false))
    }
}

/// Copy `text` directly to the native OS clipboard (`pbcopy` on macOS;
/// `wl-copy` / `xclip` / `xsel` on Linux/Unix) without invoking a shell.
pub fn copy_to_system_clipboard(text: &str) -> Result<&'static str, String> {
    let candidates = system_clipboard_candidates();
    copy_with_system_commands(text, &candidates)
}

/// Copy `text` to the native OS clipboard, falling back to tmux's paste buffer
/// (`tmux.set_buffer`) when no system clipboard helper succeeds.
pub fn copy_to_clipboard(text: &str, tmux: Option<&dyn Tmux>) -> Result<ClipboardOutcome, String> {
    let candidates = system_clipboard_candidates();
    copy_with_commands(text, &candidates, tmux)
}

fn system_clipboard_candidates() -> Vec<(&'static str, &'static [&'static str])> {
    #[cfg(target_os = "macos")]
    {
        vec![("pbcopy", &[])]
    }
    #[cfg(not(target_os = "macos"))]
    {
        let wayland = env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if wayland {
            vec![
                ("wl-copy", &[]),
                ("xclip", &["-selection", "clipboard"]),
                ("xsel", &["--clipboard", "--input"]),
            ]
        } else {
            vec![
                ("xclip", &["-selection", "clipboard"]),
                ("xsel", &["--clipboard", "--input"]),
                ("wl-copy", &[]),
            ]
        }
    }
}

fn copy_with_system_commands(
    text: &str,
    candidates: &[(&'static str, &[&str])],
) -> Result<&'static str, String> {
    let mut errors = Vec::new();
    for &(program, args) in candidates {
        match run_clipboard_command(program, args, text) {
            Ok(()) => return Ok(program),
            Err(err) => errors.push(err),
        }
    }
    if errors.is_empty() {
        Err("no system clipboard command configured".to_owned())
    } else {
        Err(errors.join("; "))
    }
}

fn copy_with_commands(
    text: &str,
    candidates: &[(&'static str, &[&str])],
    tmux: Option<&dyn Tmux>,
) -> Result<ClipboardOutcome, String> {
    match copy_with_system_commands(text, candidates) {
        Ok(tool) => Ok(ClipboardOutcome::SystemClipboard(tool)),
        Err(os_err) => {
            if let Some(client) = tmux {
                client.set_buffer(text).map_err(|tmux_err| {
                    format!(
                        "system clipboard unavailable ({os_err}); tmux buffer fallback failed: {tmux_err}"
                    )
                })?;
                Ok(ClipboardOutcome::TmuxBuffer)
            } else {
                Err(format!("system clipboard unavailable ({os_err})"))
            }
        }
    }
}

fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    // Redirect stdout/stderr to null so backgrounded clipboard daemons like
    // `xclip` or `wl-copy` do not hold open pipes and block `wait()`.
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("{program}: {err}"))?;

    if let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(text.as_bytes())
    {
        let _ = child.wait();
        return Err(format!("{program} write failed: {err}"));
    }

    let status = child
        .wait()
        .map_err(|err| format!("{program} wait failed: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

/// Pure-std RFC 4648 base64 encoder (zero external crates).
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    let (chunks, remainder) = bytes.as_chunks::<3>();
    for &[a, b, c] in chunks {
        let n = ((a as u32) << 16) | ((b as u32) << 8) | (c as u32);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(n & 0x3f) as usize] as char);
    }

    match remainder {
        [a] => {
            let n = (u32::from(*a)) << 16;
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        [a, b] => {
            let n = (u32::from(*a) << 16) | (u32::from(*b) << 8);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }

    out
}

/// Build a standard OSC 52 terminal clipboard escape sequence (`\x1b]52;c;<b64>\x07`).
pub fn osc52_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()))
}

/// Sanitize a session, window, or pane label into a safe ASCII filename component
/// free of path separators, leading dots, quotes, spaces, or control characters.
pub fn sanitize_filename_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(48));
    let mut prev_underscore = false;

    for ch in raw.chars() {
        if out.len() >= 48 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            prev_underscore = false;
        } else if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    let trimmed = out.trim_matches(|c| c == '_' || c == '-');
    if trimmed.is_empty() {
        "unnamed".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Strip ANSI/VT escape sequences and unsafe control characters from a log line
/// while keeping printable UTF-8 text and tabs intact.
pub fn strip_ansi_escapes(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI: ESC [ <params 0x30..=0x3f>* <interm 0x20..=0x2f>* <final 0x40..=0x7e>
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                b']' | b'P' | b'X' | b'^' | b'_' => {
                    // OSC / DCS / SOS / PM / APC: terminated by BEL (0x07) or ST (ESC \)
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                0x20..=0x2f => {
                    // nF escape sequence: intermediate bytes followed by a final byte.
                    i += 1;
                    while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                _ => {
                    // Single-character ESC sequence.
                    i += 1;
                }
            }
            continue;
        }

        if let Some(ch) = raw[i..].chars().next() {
            i += ch.len_utf8();
            if ch == '\t' || !ch.is_control() {
                out.push(ch);
            }
        } else {
            break;
        }
    }

    out
}

/// Resolve the directory used for exported logs and `pipe-pane` recordings:
/// 1) `dir_override` if provided
/// 2) `JUSTMUX_LOG_DIR` if set and non-empty
/// 3) `$XDG_DATA_HOME/justmux/logs` if set and absolute
/// 4) `~/justmux-logs`
pub fn resolve_log_dir(dir_override: Option<&Path>) -> Result<PathBuf, String> {
    let env_override = env::var_os(LOG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let xdg_data_home = env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let home_dir = env::home_dir();
    resolve_log_dir_with(
        dir_override,
        env_override.as_deref(),
        xdg_data_home.as_deref(),
        home_dir.as_deref(),
    )
}

fn resolve_log_dir_with(
    dir_override: Option<&Path>,
    env_override: Option<&Path>,
    xdg_data_home: Option<&Path>,
    home_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(dir) = dir_override {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = env_override.filter(|p| !p.as_os_str().is_empty()) {
        return Ok(dir.to_path_buf());
    }
    if let Some(xdg) = xdg_data_home.filter(|p| p.is_absolute()) {
        return Ok(xdg.join("justmux").join("logs"));
    }
    if let Some(home) = home_dir {
        return Ok(home.join("justmux-logs"));
    }
    Err("could not determine log directory (HOME is not set)".to_owned())
}

fn current_utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc_timestamp(secs)
}

fn format_utc_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let sod = secs % 86_400;
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;

    // Howard Hinnant's civil_from_days algorithm (1970-01-01 = day 0).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Write sanitized plain-text `lines` to a safe timestamped `.log` file inside
/// `dir_override` (or the resolved JustMux log directory).
pub fn export_log_to_file(
    session: &str,
    window: &str,
    lines: &[String],
    dir_override: Option<&Path>,
) -> Result<PathBuf, String> {
    let dir = resolve_log_dir(dir_override)?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("could not create log directory `{}`: {err}", dir.display()))?;

    let safe_session = sanitize_filename_component(session);
    let safe_window = sanitize_filename_component(window);
    let stamp = current_utc_timestamp();

    let mut content = String::new();
    for line in lines {
        content.push_str(&strip_ansi_escapes(line));
        content.push('\n');
    }

    for suffix in 0u32..1_000 {
        let filename = if suffix == 0 {
            format!("{safe_session}-{safe_window}-{stamp}.log")
        } else {
            format!("{safe_session}-{safe_window}-{stamp}-{suffix}.log")
        };
        let path = dir.join(filename);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes()).map_err(|err| {
                    format!("could not write log file `{}`: {err}", path.display())
                })?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "could not create log file `{}`: {err}",
                    path.display()
                ));
            }
        }
    }

    Err(format!(
        "could not allocate unique log filename in `{}`",
        dir.display()
    ))
}

/// Build the default log file path used by tmux `pipe-pane` streaming.
pub fn default_pipe_log_path(session: &str, window: &str, pane_id: &str) -> PathBuf {
    let dir = resolve_log_dir(None).unwrap_or_else(|_| env::temp_dir().join("justmux-logs"));
    pipe_log_path_in(&dir, session, window, pane_id)
}

/// Build a `pipe-pane` log file path inside an explicit `dir`.
pub fn pipe_log_path_in(dir: &Path, session: &str, window: &str, pane_id: &str) -> PathBuf {
    let safe_session = sanitize_filename_component(session);
    let safe_window = sanitize_filename_component(window);
    let pane_trimmed = pane_id.strip_prefix('%').unwrap_or(pane_id);
    let safe_pane = sanitize_filename_component(pane_trimmed);
    dir.join(format!(
        "pipe-{safe_session}-{safe_window}-p{safe_pane}.log"
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::tmux::{
        Context, Pane, PaneId, Session, SessionId, SplitDirection, Target, TmuxError, TmuxResult,
        TmuxVersion, Window, WindowId,
    };

    struct BufferStubTmux {
        saved: RefCell<Option<String>>,
        fail: bool,
    }

    impl Tmux for BufferStubTmux {
        fn version(&self) -> TmuxResult<TmuxVersion> {
            Ok(TmuxVersion {
                raw: "3.4".into(),
                number: Some((3, 4)),
            })
        }
        fn server_running(&self) -> TmuxResult<bool> {
            Ok(!self.fail)
        }
        fn context(&self) -> Context {
            Context::Outside
        }
        fn list_sessions(&self) -> TmuxResult<Vec<Session>> {
            Ok(Vec::new())
        }
        fn has_session(&self, _: &SessionId) -> TmuxResult<bool> {
            Ok(false)
        }
        fn new_session(&self, _: &str, _: &Path) -> TmuxResult<Session> {
            Err(TmuxError::NoServer)
        }
        fn rename_session(&self, _: &SessionId, _: &str) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn kill_session(&self, _: &SessionId) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn list_windows(&self, _: Option<&SessionId>) -> TmuxResult<Vec<Window>> {
            Ok(Vec::new())
        }
        fn new_window(
            &self,
            _: &SessionId,
            _: Option<&str>,
            _: Option<&Path>,
        ) -> TmuxResult<Window> {
            Err(TmuxError::NoServer)
        }
        fn rename_window(&self, _: &WindowId, _: &str) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn kill_window(&self, _: &WindowId) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn list_panes(&self, _: Option<&WindowId>) -> TmuxResult<Vec<Pane>> {
            Ok(Vec::new())
        }
        fn split_pane(&self, _: &PaneId, _: SplitDirection) -> TmuxResult<Pane> {
            Err(TmuxError::NoServer)
        }
        fn join_pane(&self, _: &PaneId, _: &PaneId, _: SplitDirection) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn kill_pane(&self, _: &PaneId) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn select(&self, _: &Target) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn switch_client(&self, _: &Target) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn detach_client(&self, _: &str) -> TmuxResult<()> {
            Err(TmuxError::NoServer)
        }
        fn attach_command(&self, _: &Target) -> Command {
            Command::new("tmux")
        }
        fn set_buffer(&self, content: &str) -> TmuxResult<()> {
            if self.fail {
                Err(TmuxError::NoServer)
            } else {
                *self.saved.borrow_mut() = Some(content.to_owned());
                Ok(())
            }
        }
    }

    #[test]
    fn base64_and_osc52_follow_rfc4648() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode("你好".as_bytes()), "5L2g5aW9");

        assert_eq!(osc52_sequence("hello"), "\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(osc52_sequence(""), "\x1b]52;c;\x07");
    }

    #[test]
    fn sanitize_filename_component_blocks_traversal_and_special_chars() {
        assert_eq!(sanitize_filename_component("dev-api_01"), "dev-api_01");
        assert_eq!(
            sanitize_filename_component("../../etc/passwd"),
            "etc_passwd"
        );
        assert_eq!(
            sanitize_filename_component("my session / window:1"),
            "my_session_window_1"
        );
        assert_eq!(sanitize_filename_component("   ...///   "), "unnamed");
        assert_eq!(sanitize_filename_component("中文名"), "unnamed");
        let long = "a".repeat(100);
        assert_eq!(sanitize_filename_component(&long).len(), 48);
    }

    #[test]
    fn strip_ansi_escapes_removes_sgr_osc_and_controls_but_keeps_utf8() {
        let raw = "\x1b[38;2;120;180;255mhello\x1b[0m \x1b]0;title\x07world\t中文\x00\x07!";
        assert_eq!(strip_ansi_escapes(raw), "hello world\t中文!");
        let st_osc = "prefix\x1b]52;c;abc\x1b\\suffix";
        assert_eq!(strip_ansi_escapes(st_osc), "prefixsuffix");
    }

    #[test]
    fn utc_timestamp_handles_epoch_and_leap_day() {
        assert_eq!(format_utc_timestamp(0), "19700101-000000");
        // 2000-02-29 12:34:56 UTC
        assert_eq!(format_utc_timestamp(951_827_696), "20000229-123456");
    }

    #[test]
    fn export_log_to_file_writes_sanitized_lines_to_tempdir_and_avoids_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lines = vec![
            "\x1b[32mok\x1b[0m build finished".to_owned(),
            "line 2\twith tab".to_owned(),
        ];

        let first = export_log_to_file("../prod", "api:logs", &lines, Some(dir.path()))
            .expect("export first");
        assert_eq!(first.parent(), Some(dir.path()));
        let fname = first
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf8 filename");
        assert!(fname.starts_with("prod-api_logs-"), "{fname}");
        assert!(fname.ends_with(".log"), "{fname}");
        assert!(!fname.contains(".."));

        let body = fs::read_to_string(&first).expect("read exported log");
        assert_eq!(body, "ok build finished\nline 2\twith tab\n");

        // Second export within the same second gets a distinct filename.
        let second = export_log_to_file("../prod", "api:logs", &lines, Some(dir.path()))
            .expect("export second");
        assert_ne!(first, second);
        assert!(second.exists());
    }

    #[test]
    fn pipe_log_path_and_log_dir_precedence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = pipe_log_path_in(dir.path(), "my/sess", "win 1", "%14");
        assert_eq!(p, dir.path().join("pipe-my_sess-win_1-p14.log"));

        let explicit = PathBuf::from("/explicit/logs");
        let env_dir = PathBuf::from("/env/logs");
        let xdg = PathBuf::from("/xdg/data");
        let rel_xdg = PathBuf::from("relative/xdg");
        let home = PathBuf::from("/home/tester");

        assert_eq!(
            resolve_log_dir_with(Some(&explicit), Some(&env_dir), Some(&xdg), Some(&home)),
            Ok(explicit)
        );
        assert_eq!(
            resolve_log_dir_with(None, Some(&env_dir), Some(&xdg), Some(&home)),
            Ok(env_dir)
        );
        assert_eq!(
            resolve_log_dir_with(None, None, Some(&xdg), Some(&home)),
            Ok(xdg.join("justmux/logs"))
        );
        assert_eq!(
            resolve_log_dir_with(None, None, Some(&rel_xdg), Some(&home)),
            Ok(home.join("justmux-logs"))
        );
        assert!(resolve_log_dir_with(None, None, None, None).is_err());
    }

    #[test]
    fn copy_with_commands_pipes_stdin_and_falls_back_to_tmux_buffer() {
        // Test native command success without touching the real user clipboard.
        let dir = tempfile::tempdir().expect("tempdir");
        let out_file = dir.path().join("captured.txt");
        let out_arg = out_file.to_str().expect("utf8 path");
        let tee_args = [out_arg];
        let candidates: Vec<(&'static str, &[&str])> = vec![
            ("justmux-nonexistent-clip-helper-xyz", &[]),
            ("tee", &tee_args),
        ];
        let stub = BufferStubTmux {
            saved: RefCell::new(None),
            fail: false,
        };
        let outcome =
            copy_with_commands("hello clipboard\nline 2", &candidates, Some(&stub)).expect("copy");
        assert_eq!(outcome, ClipboardOutcome::SystemClipboard("tee"));
        assert_eq!(
            fs::read_to_string(&out_file).expect("read tee output"),
            "hello clipboard\nline 2"
        );
        // Native success does not unnecessarily overwrite tmux's buffer.
        assert_eq!(*stub.saved.borrow(), None);

        // When all OS clipboard helpers fail, fall back cleanly to tmux buffer.
        let missing: Vec<(&'static str, &[&str])> =
            vec![("justmux-nonexistent-clip-helper-xyz", &[])];
        let fallback =
            copy_with_commands("headless ssh copy", &missing, Some(&stub)).expect("tmux fallback");
        assert_eq!(fallback, ClipboardOutcome::TmuxBuffer);
        assert_eq!(stub.saved.borrow().as_deref(), Some("headless ssh copy"));

        // When both OS helpers and tmux fail, return a clear error.
        let failing_stub = BufferStubTmux {
            saved: RefCell::new(None),
            fail: true,
        };
        let err = copy_with_commands("fail", &missing, Some(&failing_stub))
            .expect_err("expected failure");
        assert!(err.contains("system clipboard unavailable"), "{err}");
        assert!(err.contains("tmux buffer fallback failed"), "{err}");
    }
}
