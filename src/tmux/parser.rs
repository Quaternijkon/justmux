//! Parsers for explicit formats emitted by tmux.
use super::{Pane, PaneId, Session, SessionId, TmuxError, TmuxResult, Window, WindowId};
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
fn u64f(s: &str) -> TmuxResult<u64> {
    s.parse()
        .map_err(|_| TmuxError::Parse(format!("invalid number: {s}")))
}
fn u32f(s: &str) -> TmuxResult<u32> {
    s.parse()
        .map_err(|_| TmuxError::Parse(format!("invalid number: {s}")))
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
            let p = fields(line, 7)?;
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
            })
        })
        .collect()
}
pub fn parse_panes(output: &str) -> TmuxResult<Vec<Pane>> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let p = fields(line, 14)?;
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
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_session_line() {
        let x = parse_sessions("$1\talpha\t2\t0\t10\t\t20\t/tmp\n").unwrap();
        assert_eq!(x[0].name, "alpha");
        assert_eq!(x[0].last_attached, None);
    }
    #[test]
    fn parse_version_text() {
        assert_eq!(parse_version("tmux 3.8").unwrap().number, Some((3, 8)));
    }
}
