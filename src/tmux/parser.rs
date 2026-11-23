//! Parsers for explicit formats emitted by tmux.
use super::{
    ClientInfo, Pane, PaneId, PasteBuffer, Session, SessionId, TmuxError, TmuxResult, Window,
    WindowId,
};

fn fields(line: &str, count: usize) -> TmuxResult<Vec<&str>> {
    let p: Vec<_> = line.split('\t').collect();
    if p.len() < count {
        Err(TmuxError::Parse(format!(
            "expected {count} fields, got {}",
            p.len()
        )))
    } else {
        Ok(p)
    }
}

fn fields_n(line: &str, count: usize) -> TmuxResult<Vec<&str>> {
    let p: Vec<_> = line.splitn(count, '\t').collect();
    if p.len() < count {
        Err(TmuxError::Parse(format!(
            "expected {count} fields, got {}",
            p.len()
        )))
    } else {
        Ok(p)
    }
}

fn u64f(s: &str) -> TmuxResult<u64> {
    s.parse()
        .map_err(|_| TmuxError::Parse(format!("invalid number: {s}")))
}

fn opt_u64f(s: &str) -> TmuxResult<u64> {
    if s.is_empty() { Ok(0) } else { u64f(s) }
}

fn u32f(s: &str) -> TmuxResult<u32> {
    s.parse()
        .map_err(|_| TmuxError::Parse(format!("invalid number: {s}")))
}

fn opt_u32f(s: &str) -> TmuxResult<u32> {
    if s.is_empty() { Ok(0) } else { u32f(s) }
}

fn sid<T>(s: &str, m: impl FnOnce(String) -> Option<T>) -> TmuxResult<T> {
    m(s.to_string()).ok_or_else(|| TmuxError::Parse(format!("invalid tmux id: {s}")))
}

fn unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

pub fn parse_version(raw: &str) -> TmuxResult<super::TmuxVersion> {
    let value = raw
        .trim()
        .strip_prefix("tmux ")
        .unwrap_or(raw.trim())
        .to_string();
    let mut it = value.split('.');
    let number = it.next().and_then(|m| {
        let major = m.parse::<u32>().ok()?;
        let minor = it
            .next()
            .and_then(|n| {
                n.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .unwrap_or(0);
        Some((major, minor))
    });
    Ok(super::TmuxVersion { raw: value, number })
}

pub fn parse_sessions(output: &str) -> TmuxResult<Vec<Session>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields(line, 8)?;
            Ok(Session {
                id: sid(p[0], SessionId::new)?,
                name: unquote(p[1]),
                windows: p[2]
                    .parse()
                    .map_err(|_| TmuxError::Parse("invalid window count".into()))?,
                attached: p[3]
                    .parse()
                    .map_err(|_| TmuxError::Parse("invalid attached count".into()))?,
                created: u64f(p[4])?,
                last_attached: if p[5].is_empty() {
                    None
                } else {
                    Some(u64f(p[5])?)
                },
                activity: u64f(p[6])?,
                path: unquote(p[7]),
            })
        })
        .collect()
}

pub fn parse_windows(output: &str) -> TmuxResult<Vec<Window>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields_n(line, 15)?;
            Ok(Window {
                id: sid(p[0], WindowId::new)?,
                session_id: sid(p[1], SessionId::new)?,
                session_name: unquote(p[2]),
                index: u32f(p[3])?,
                name: unquote(p[4]),
                active: p[5] == "1",
                panes: p[6]
                    .parse()
                    .map_err(|_| TmuxError::Parse("invalid pane count".into()))?,
                zoomed: p[7] == "1",
                synchronized: p[8] == "1",
                activity_flag: p[9] == "1",
                bell_flag: p[10] == "1",
                silence_flag: p[11] == "1",
                monitor_activity: matches!(p[12], "1" | "on" | "true"),
                monitor_silence: opt_u64f(p[13])?,
                layout: p[14].to_string(),
            })
        })
        .collect()
}

pub fn parse_panes(output: &str) -> TmuxResult<Vec<Pane>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields(line, 20)?;
            Ok(Pane {
                id: sid(p[0], PaneId::new)?,
                window_id: sid(p[1], WindowId::new)?,
                window_index: u32f(p[2])?,
                window_name: unquote(p[3]),
                session_id: sid(p[4], SessionId::new)?,
                session_name: unquote(p[5]),
                index: u32f(p[6])?,
                active: p[7] == "1",
                current_command: unquote(p[8]),
                current_path: unquote(p[9]),
                title: unquote(p[10]),
                pid: if p[11].is_empty() {
                    None
                } else {
                    Some(
                        p[11]
                            .parse()
                            .map_err(|_| TmuxError::Parse("invalid pane pid".into()))?,
                    )
                },
                dead: p[12] == "1",
                busy: !matches!(
                    p[8],
                    "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "nu" | "tcsh" | "csh"
                ) && p[12] != "1",
                width: u32f(p[14])?,
                height: u32f(p[15])?,
                left: u32f(p[16])?,
                top: u32f(p[17])?,
                zoomed: p[18] == "1",
                synchronized: p[19] == "1",
            })
        })
        .collect()
}

pub fn parse_clients(output: &str) -> TmuxResult<Vec<ClientInfo>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields(line, 9)?;
            Ok(ClientInfo {
                name: unquote(p[0]),
                session_name: unquote(p[1]),
                session_id: if p[2].is_empty() {
                    None
                } else {
                    Some(sid(p[2], SessionId::new)?)
                },
                width: opt_u32f(p[3])?,
                height: opt_u32f(p[4])?,
                tty: unquote(p[5]),
                readonly: p[6] == "1",
                control_mode: p[7] == "1",
                activity: opt_u64f(p[8])?,
            })
        })
        .collect()
}

pub fn parse_buffers(output: &str) -> TmuxResult<Vec<PasteBuffer>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields_n(line, 3)?;
            Ok(PasteBuffer {
                name: unquote(p[0]),
                size: p[1]
                    .parse()
                    .map_err(|_| TmuxError::Parse("invalid buffer size".into()))?,
                sample: p[2].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_line() {
        let x = parse_sessions("$1\talpha\t2\t0\t10\t\t20\t/tmp\n").expect("session");
        assert_eq!(x[0].name, "alpha");
        assert_eq!(x[0].last_attached, None);
    }

    #[test]
    fn parse_version_text() {
        assert_eq!(
            parse_version("tmux 3.8").expect("version").number,
            Some((3, 8))
        );
        assert_eq!(
            parse_version("tmux 3.3a").expect("version").number,
            Some((3, 3))
        );
    }

    #[test]
    fn parse_window_extended_fields() {
        let layout = r#"{"V":2,"L":{"t":"p","w":80,"h":24,"x":0,"y":0,"a":true,"i":0,"I":"%0"}}"#;
        let line = format!(
            "@2\t$1\twork\\ space\t1\teditor\\ win\t1\t3\t1\t1\t1\t0\t1\t1\t30\t{layout}\n"
        );
        let wins = parse_windows(&line).expect("windows");
        assert_eq!(wins.len(), 1);
        let w = &wins[0];
        assert_eq!(w.id.as_str(), "@2");
        assert_eq!(w.session_id.as_str(), "$1");
        assert_eq!(w.session_name, "work space");
        assert_eq!(w.index, 1);
        assert_eq!(w.name, "editor win");
        assert!(w.active);
        assert_eq!(w.panes, 3);
        assert!(w.zoomed);
        assert!(w.synchronized);
        assert!(w.activity_flag);
        assert!(!w.bell_flag);
        assert!(w.silence_flag);
        assert!(w.monitor_activity);
        assert_eq!(w.monitor_silence, 30);
        assert_eq!(w.layout, layout);
    }

    #[test]
    fn parse_window_empty_silence_defaults_to_zero() {
        let line = "@0\t$0\tmain\t0\tshell\t0\t1\t0\t0\t0\t1\t0\t0\t\tbb62,80x24,0,0,0\n";
        let wins = parse_windows(line).expect("windows");
        assert!(!wins[0].zoomed);
        assert!(!wins[0].synchronized);
        assert!(wins[0].bell_flag);
        assert!(!wins[0].monitor_activity);
        assert_eq!(wins[0].monitor_silence, 0);
        assert_eq!(wins[0].layout, "bb62,80x24,0,0,0");
    }

    #[test]
    fn parse_pane_extended_geometry_and_flags() {
        let line = concat!(
            "%5\t@2\t1\teditor\t$1\twork\t2\t1\tcargo\t/tmp/my\\ dir\tbuild\\ pane\t1234\t0\t0\t",
            "100\t32\t51\t0\t1\t1\n",
            "%6\t@2\t1\teditor\t$1\twork\t3\t0\tzsh\t/tmp\t\t\t1\t0\t",
            "50\t32\t0\t0\t0\t0\n",
        );
        let panes = parse_panes(line).expect("panes");
        assert_eq!(panes.len(), 2);
        let p0 = &panes[0];
        assert_eq!(p0.id.as_str(), "%5");
        assert_eq!(p0.current_path, "/tmp/my dir");
        assert_eq!(p0.title, "build pane");
        assert_eq!(p0.pid, Some(1234));
        assert!(p0.busy);
        assert!(!p0.dead);
        assert_eq!((p0.width, p0.height, p0.left, p0.top), (100, 32, 51, 0));
        assert!(p0.zoomed);
        assert!(p0.synchronized);

        let p1 = &panes[1];
        assert_eq!(p1.pid, None);
        assert!(p1.dead);
        assert!(!p1.busy);
        assert_eq!((p1.width, p1.height, p1.left, p1.top), (50, 32, 0, 0));
        assert!(!p1.zoomed);
        assert!(!p1.synchronized);
    }

    #[test]
    fn parse_clients_handles_tty_and_control_mode() {
        let raw = concat!(
            "/dev/ttys001\tmy\\ sess\t$0\t120\t40\t/dev/ttys001\t0\t0\t1700000100\n",
            "client-42\tmy\\ sess\t$0\t80\t\t\t1\t1\t1700000200\n",
            "detached\t\t\t0\t0\t\t0\t0\t\n",
        );
        let clients = parse_clients(raw).expect("clients");
        assert_eq!(clients.len(), 3);
        assert_eq!(clients[0].name, "/dev/ttys001");
        assert_eq!(clients[0].session_name, "my sess");
        assert_eq!(
            clients[0].session_id.as_ref().map(SessionId::as_str),
            Some("$0")
        );
        assert_eq!((clients[0].width, clients[0].height), (120, 40));
        assert_eq!(clients[0].tty, "/dev/ttys001");
        assert!(!clients[0].readonly);
        assert!(!clients[0].control_mode);
        assert_eq!(clients[0].activity, 1700000100);

        assert_eq!(clients[1].name, "client-42");
        assert_eq!((clients[1].width, clients[1].height), (80, 0));
        assert_eq!(clients[1].tty, "");
        assert!(clients[1].readonly);
        assert!(clients[1].control_mode);

        assert_eq!(clients[2].session_id, None);
        assert_eq!(clients[2].activity, 0);
    }

    #[test]
    fn parse_buffers_handles_samples_and_rejects_bad_size() {
        let raw = "buffer0\t14\thello\\tworld\\n\nbuf\\ 1\t5\ta\tb\n";
        let bufs = parse_buffers(raw).expect("buffers");
        assert_eq!(bufs.len(), 2);
        assert_eq!(bufs[0].name, "buffer0");
        assert_eq!(bufs[0].size, 14);
        assert_eq!(bufs[0].sample, r"hello\tworld\n");
        assert_eq!(bufs[1].name, "buf 1");
        assert_eq!(bufs[1].size, 5);
        assert_eq!(bufs[1].sample, "a\tb");

        assert!(matches!(
            parse_buffers("buffer0\tnotanumber\tsample\n"),
            Err(TmuxError::Parse(_))
        ));
    }
}
