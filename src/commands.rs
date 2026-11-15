//! Slash command registry and parser.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    Resume,
    New,
    List,
    Rename,
    Kill,
    Window,
    Pane,
    Split,
    Help,
    Quit,
}
impl CommandKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::New => "new",
            Self::List => "list",
            Self::Rename => "rename",
            Self::Kill => "kill",
            Self::Window => "window",
            Self::Pane => "pane",
            Self::Split => "split",
            Self::Help => "help",
            Self::Quit => "quit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub kind: CommandKind,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub usage: &'static str,
}
impl SlashCommand {
    pub fn matches(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }
}

pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        kind: CommandKind::Resume,
        name: "resume",
        aliases: &["r"],
        description: "Resume an existing tmux session",
        usage: "/resume",
    },
    SlashCommand {
        kind: CommandKind::New,
        name: "new",
        aliases: &["n"],
        description: "Create a tmux session",
        usage: "/new [name] [directory]",
    },
    SlashCommand {
        kind: CommandKind::List,
        name: "list",
        aliases: &["ls"],
        description: "List tmux sessions",
        usage: "/list",
    },
    SlashCommand {
        kind: CommandKind::Rename,
        name: "rename",
        aliases: &["mv"],
        description: "Rename a tmux session",
        usage: "/rename",
    },
    SlashCommand {
        kind: CommandKind::Kill,
        name: "kill",
        aliases: &["k"],
        description: "Kill a tmux session",
        usage: "/kill",
    },
    SlashCommand {
        kind: CommandKind::Window,
        name: "window",
        aliases: &["w"],
        description: "Manage windows",
        usage: "/window",
    },
    SlashCommand {
        kind: CommandKind::Pane,
        name: "pane",
        aliases: &["p"],
        description: "Manage panes",
        usage: "/pane",
    },
    SlashCommand {
        kind: CommandKind::Split,
        name: "split",
        aliases: &["s"],
        description: "Split the current pane",
        usage: "/split [horizontal|vertical]",
    },
    SlashCommand {
        kind: CommandKind::Help,
        name: "help",
        aliases: &["?"],
        description: "Show command help",
        usage: "/help [command]",
    },
    SlashCommand {
        kind: CommandKind::Quit,
        name: "quit",
        aliases: &["q", "exit"],
        description: "Quit JustMux",
        usage: "/quit",
    },
];
pub fn registry() -> &'static [SlashCommand] {
    COMMANDS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub command: CommandKind,
    pub args: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    NotACommand,
    MissingName,
    UnknownCommand(String),
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotACommand => f.write_str("commands start with /"),
            Self::MissingName => f.write_str("type / followed by a command"),
            Self::UnknownCommand(n) => write!(f, "unknown command: /{n}"),
        }
    }
}
impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<ParsedCommand, ParseError> {
    let input = input.trim();
    if !input.starts_with('/') {
        return Err(ParseError::NotACommand);
    }
    let body = input[1..].trim();
    if body.is_empty() {
        return Err(ParseError::MissingName);
    }
    let words = tokenize(body);
    let name = words
        .first()
        .map(String::as_str)
        .ok_or(ParseError::MissingName)?;
    let command = registry()
        .iter()
        .find(|c| c.matches(name))
        .map(|c| c.kind)
        .ok_or_else(|| ParseError::UnknownCommand(name.to_owned()))?;
    Ok(ParsedCommand {
        command,
        args: words.into_iter().skip(1).collect(),
    })
}
fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q { quote = None } else { cur.push(ch) }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch)
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch)
        }
    }
    if !cur.is_empty() {
        out.push(cur)
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        assert_eq!(parse("/resume").unwrap().command, CommandKind::Resume);
        assert_eq!(parse("/r").unwrap().command, CommandKind::Resume);
    }
    #[test]
    fn quoted() {
        assert_eq!(
            parse("/new 'my project' /tmp").unwrap().args,
            vec!["my project", "/tmp"]
        );
    }
    #[test]
    fn rejects() {
        assert!(matches!(parse("hello"), Err(ParseError::NotACommand)));
        assert!(matches!(parse("/wat"), Err(ParseError::UnknownCommand(_))));
    }
}
