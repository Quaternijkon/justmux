//! Slash command registry and parser.
use std::fmt;

use crate::{
    fuzzy::{FuzzyMatcher, MatchKind},
    tributes::{MicroTributeKind, TributeId},
};

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
    Theme,
    Config,
    Help,
    Exit,
    Quit,
    Tribute(TributeId),
    MicroTribute(MicroTributeKind),
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
            Self::Theme => "theme",
            Self::Config => "config",
            Self::Help => "help",
            Self::Exit => "exit",
            Self::Quit => "quit",
            Self::Tribute(id) => match id {
                TributeId::Codex => "codex",
                TributeId::ClaudeCode => "claude",
                TributeId::Antigravity => "antigravity",
                TributeId::HallOfFame => "vibe",
            },
            Self::MicroTribute(kind) => match kind {
                MicroTributeKind::Compact => "compact",
                MicroTributeKind::Cost => "cost",
                MicroTributeKind::Boost => "boost",
                MicroTributeKind::GrillMe => "grill-me",
                MicroTributeKind::Goal => "goal",
                MicroTributeKind::Model => "model",
            },
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
        aliases: &["r", "attach", "switch"],
        description: "Open a window in an existing tmux session",
        usage: "/resume [@target]",
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
        usage: "/rename [@target new-name]",
    },
    SlashCommand {
        kind: CommandKind::Kill,
        name: "kill",
        aliases: &["k"],
        description: "Kill a tmux session",
        usage: "/kill [@target]",
    },
    SlashCommand {
        kind: CommandKind::Window,
        name: "window",
        aliases: &["w", "win", "windows"],
        description: "Open, create, rename, or close windows",
        usage: "/window [@target]",
    },
    SlashCommand {
        kind: CommandKind::Pane,
        name: "pane",
        aliases: &["p"],
        description: "Manage panes",
        usage: "/pane [@target]",
    },
    SlashCommand {
        kind: CommandKind::Split,
        name: "split",
        aliases: &["s"],
        description: "Split the current pane",
        usage: "/split [horizontal|vertical|@target]",
    },
    SlashCommand {
        kind: CommandKind::Theme,
        name: "theme",
        aliases: &["palette", "colors"],
        description: "Switch visual color theme",
        usage: "/theme [aurora|codex|claude|matrix|classic]",
    },
    SlashCommand {
        kind: CommandKind::Help,
        name: "help",
        aliases: &["?"],
        description: "Show command help",
        usage: "/help [command]",
    },
    SlashCommand {
        kind: CommandKind::Exit,
        name: "exit",
        aliases: &[],
        description: "Detach the current tmux client",
        usage: "/exit",
    },
    SlashCommand {
        kind: CommandKind::Quit,
        name: "quit",
        aliases: &["q"],
        description: "Quit JustMux",
        usage: "/quit",
    },
];

pub const SECRET_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        kind: CommandKind::Config,
        name: "config",
        aliases: &["settings", "prefs"],
        description: "Inspect or save JustMux configuration",
        usage: "/config",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::Codex),
        name: "codex",
        aliases: &["openai", "approval"],
        description: "Celebrate OpenAI Codex CLI craft",
        usage: "/codex",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::ClaudeCode),
        name: "claude",
        aliases: &["cc", "clauding", "ultrathink"],
        description: "Celebrate Anthropic Claude Code flow",
        usage: "/claude",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::Antigravity),
        name: "antigravity",
        aliases: &["agy", "liftoff", "orbit"],
        description: "Celebrate Google DeepMind Antigravity",
        usage: "/antigravity",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::HallOfFame),
        name: "vibe",
        aliases: &["easter", "credits", "tributes"],
        description: "Open the JustMux Tribute Vault",
        usage: "/vibe",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Compact),
        name: "compact",
        aliases: &[],
        description: "Zero-token tmux scrollback check",
        usage: "/compact",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Cost),
        name: "cost",
        aliases: &[],
        description: "Inspect native tmux token budget",
        usage: "/cost",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Boost),
        name: "boost",
        aliases: &[],
        description: "Engage turbo aurora mode",
        usage: "/boost",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::GrillMe),
        name: "grill-me",
        aliases: &["grill"],
        description: "Interrogate your tmux architecture",
        usage: "/grill-me",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Goal),
        name: "goal",
        aliases: &[],
        description: "Lock in zero-memorization tmux",
        usage: "/goal",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Model),
        name: "model",
        aliases: &[],
        description: "Check active native tmux engine",
        usage: "/model",
    },
];

pub fn registry() -> &'static [SlashCommand] {
    COMMANDS
}

/// A canonical command row, with matching positions in its name or alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMatch {
    pub command: &'static SlashCommand,
    pub alias: Option<&'static str>,
    pub indices: Vec<usize>,
}

/// Return the name being completed, stopping the popup when arguments begin.
pub fn popup_query(input: &str) -> Option<&str> {
    let query = input.trim_start().strip_prefix('/')?;
    (!query.contains(char::is_whitespace)).then_some(query)
}

/// Match names and aliases once per command, keeping registry order on ties.
pub fn filter_commands(query: &str, matcher: &mut FuzzyMatcher) -> Vec<CommandMatch> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return registry()
            .iter()
            .map(|command| CommandMatch {
                command,
                alias: None,
                indices: Vec::new(),
            })
            .collect();
    }

    let include_secrets = query.chars().count() >= 2;
    let sources = registry()
        .iter()
        .enumerate()
        .map(|(order, command)| (0_u8, order, command))
        .chain(
            include_secrets
                .then(|| {
                    SECRET_COMMANDS
                        .iter()
                        .enumerate()
                        .map(|(order, command)| (1_u8, order, command))
                })
                .into_iter()
                .flatten(),
        );

    let mut rows = Vec::new();
    for (source_priority, order, command) in sources {
        let candidates = std::iter::once((command.name, None))
            .chain(command.aliases.iter().map(|alias| (*alias, Some(*alias))));
        let mut best = None;
        for (name, alias) in candidates {
            let Some(matched) = matcher.match_one(&query, name) else {
                continue;
            };
            let tier = match matched.kind {
                MatchKind::Exact => 0_u8,
                MatchKind::Prefix => 1_u8,
                MatchKind::Substring => 2_u8,
                MatchKind::Fuzzy => 3_u8,
            };
            let key = (
                tier,
                source_priority,
                u8::from(alias.is_some()),
                std::cmp::Reverse(matched.score),
                order,
            );
            if best.as_ref().is_none_or(|(previous, _)| key < *previous) {
                best = Some((
                    key,
                    CommandMatch {
                        command,
                        alias,
                        indices: matched.indices,
                    },
                ));
            }
        }
        rows.extend(best);
    }
    if rows.iter().any(|((tier, ..), _)| *tier <= 1) {
        rows.retain(|((tier, ..), _)| *tier <= 2);
    }
    rows.sort_by_key(|(key, _)| *key);
    rows.into_iter().map(|(_, row)| row).collect()
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
            Self::UnknownCommand(n) => {
                write!(f, "unknown command: /{n}")?;
                if let Some(command) = suggest(n) {
                    write!(f, ". Did you mean /{}?", command.name)?;
                }
                Ok(())
            }
        }
    }
}

/// Suggest a close spelling without changing the command the parser accepts.
fn suggest(name: &str) -> Option<&'static SlashCommand> {
    let name = name.to_ascii_lowercase();
    let length = name.chars().count();
    if !(3..=32).contains(&length) {
        return None;
    }
    let budget = (length / 3).clamp(1, 2);
    registry()
        .iter()
        .flat_map(|command| {
            let name = name.as_str();
            std::iter::once(command.name)
                .chain(command.aliases.iter().copied())
                .map(move |candidate| (edit_distance(name, candidate), command))
        })
        .filter(|(distance, _)| *distance <= budget)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, command)| command)
}

/// Optimal string alignment distance includes adjacent transpositions.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<_> = left.chars().collect();
    let right: Vec<_> = right.chars().collect();
    let mut before_previous = vec![0; right.len() + 1];
    let mut previous: Vec<_> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for i in 1..=left.len() {
        current[0] = i;
        for j in 1..=right.len() {
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + usize::from(left[i - 1] != right[j - 1]));
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                current[j] = current[j].min(before_previous[j - 2] + 1);
            }
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
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
        .chain(SECRET_COMMANDS.iter())
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
        assert_eq!(parse("/exit").unwrap().command, CommandKind::Exit);
        assert_eq!(parse("/q").unwrap().command, CommandKind::Quit);
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

    #[test]
    fn aliases_are_discoverable_without_changing_exit_semantics() {
        for alias in ["r", "attach", "switch"] {
            assert_eq!(
                parse(&format!("/{alias}")).unwrap().command,
                CommandKind::Resume
            );
        }
        for alias in ["w", "win", "windows"] {
            assert_eq!(
                parse(&format!("/{alias}")).unwrap().command,
                CommandKind::Window
            );
        }
        assert_eq!(parse("/exit").unwrap().command, CommandKind::Exit);
        assert_eq!(parse("/quit").unwrap().command, CommandKind::Quit);
    }

    #[test]
    fn popup_only_completes_the_command_name() {
        assert_eq!(popup_query("/"), Some(""));
        assert_eq!(popup_query("  /res"), Some("res"));
        assert_eq!(popup_query("/new "), None);
        assert_eq!(popup_query("/new\tname"), None);
        assert_eq!(popup_query("hello"), None);
    }

    #[test]
    fn popup_ranks_exact_and_prefix_matches_and_identifies_aliases() {
        let mut matcher = FuzzyMatcher::new();
        let matches = filter_commands("r", &mut matcher);
        assert_eq!(matches[0].command.kind, CommandKind::Resume);
        assert_eq!(matches[0].alias, Some("r"));
        assert_eq!(matches[1].command.kind, CommandKind::Rename);
        assert_eq!(matches[1].alias, None);

        let matches = filter_commands("att", &mut matcher);
        assert_eq!(matches[0].command.kind, CommandKind::Resume);
        assert_eq!(matches[0].alias, Some("attach"));
        assert_eq!(matches[0].indices, vec![0, 1, 2]);

        let matches = filter_commands("win", &mut matcher);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].command.kind, CommandKind::Window);
        assert_eq!(matches[0].alias, Some("win"));
        assert_eq!(
            filter_commands("RES", &mut matcher)[0].command.kind,
            CommandKind::Resume
        );
    }

    #[test]
    fn popup_keeps_registry_order_for_ties_and_omits_duplicates() {
        let mut matcher = FuzzyMatcher::new();
        let matches = filter_commands("", &mut matcher);
        assert_eq!(matches.len(), registry().len());
        assert!(matches.iter().zip(registry()).all(|(row, command)| {
            row.command == command && row.alias.is_none() && row.indices.is_empty()
        }));
        let matches = filter_commands("re", &mut matcher);
        assert_eq!(matches[0].command.kind, CommandKind::Resume);
        assert_eq!(matches[1].command.kind, CommandKind::Rename);
        let matches = filter_commands("wi", &mut matcher);
        assert_eq!(
            matches
                .iter()
                .filter(|row| row.command.kind == CommandKind::Window)
                .count(),
            1
        );
        assert!(filter_commands("zzzz", &mut matcher).is_empty());
    }

    #[test]
    fn typo_guidance_does_not_execute_an_unknown_command() {
        let error = parse("/reusme").unwrap_err();
        assert!(matches!(error, ParseError::UnknownCommand(_)));
        assert_eq!(
            error.to_string(),
            "unknown command: /reusme. Did you mean /resume?"
        );
        assert!(parse("/res").is_err());
        assert!(
            parse("/neww project")
                .unwrap_err()
                .to_string()
                .contains("/new?")
        );
        assert!(
            !parse("/zzzzzz")
                .unwrap_err()
                .to_string()
                .contains("Did you mean")
        );
        assert!(
            !ParseError::UnknownCommand("a".repeat(100))
                .to_string()
                .contains("Did you mean")
        );
    }

    #[test]
    fn every_command_and_alias_is_unique() {
        let mut names = std::collections::HashSet::new();
        for command in registry().iter().chain(SECRET_COMMANDS.iter()) {
            assert!(names.insert(command.name));
            for alias in command.aliases {
                assert!(names.insert(*alias));
            }
        }
    }

    #[test]
    fn registry_has_twelve_commands_with_theme_before_help_and_quit_last() {
        assert_eq!(registry().len(), 12);
        assert_eq!(registry()[8].kind, CommandKind::Theme);
        assert_eq!(registry()[9].kind, CommandKind::Help);
        assert_eq!(registry()[11].kind, CommandKind::Quit);
        assert_eq!(
            parse("/theme aurora").unwrap(),
            ParsedCommand {
                command: CommandKind::Theme,
                args: vec!["aurora".into()],
            }
        );
        assert_eq!(parse("/palette").unwrap().command, CommandKind::Theme);
        assert_eq!(parse("/colors").unwrap().command, CommandKind::Theme);
    }

    #[test]
    fn secret_commands_parse_and_filter_only_from_two_characters() {
        for (cmd, expected) in [
            ("/config", CommandKind::Config),
            ("/settings", CommandKind::Config),
            ("/prefs", CommandKind::Config),
            ("/codex", CommandKind::Tribute(TributeId::Codex)),
            ("/openai", CommandKind::Tribute(TributeId::Codex)),
            ("/approval", CommandKind::Tribute(TributeId::Codex)),
            ("/claude", CommandKind::Tribute(TributeId::ClaudeCode)),
            ("/cc", CommandKind::Tribute(TributeId::ClaudeCode)),
            ("/clauding", CommandKind::Tribute(TributeId::ClaudeCode)),
            ("/ultrathink", CommandKind::Tribute(TributeId::ClaudeCode)),
            ("/antigravity", CommandKind::Tribute(TributeId::Antigravity)),
            ("/agy", CommandKind::Tribute(TributeId::Antigravity)),
            ("/liftoff", CommandKind::Tribute(TributeId::Antigravity)),
            ("/orbit", CommandKind::Tribute(TributeId::Antigravity)),
            ("/vibe", CommandKind::Tribute(TributeId::HallOfFame)),
            ("/easter", CommandKind::Tribute(TributeId::HallOfFame)),
            ("/credits", CommandKind::Tribute(TributeId::HallOfFame)),
            ("/tributes", CommandKind::Tribute(TributeId::HallOfFame)),
            (
                "/compact",
                CommandKind::MicroTribute(MicroTributeKind::Compact),
            ),
            ("/cost", CommandKind::MicroTribute(MicroTributeKind::Cost)),
            ("/boost", CommandKind::MicroTribute(MicroTributeKind::Boost)),
            (
                "/grill-me",
                CommandKind::MicroTribute(MicroTributeKind::GrillMe),
            ),
            (
                "/grill",
                CommandKind::MicroTribute(MicroTributeKind::GrillMe),
            ),
            ("/goal", CommandKind::MicroTribute(MicroTributeKind::Goal)),
            ("/model", CommandKind::MicroTribute(MicroTributeKind::Model)),
        ] {
            assert_eq!(parse(cmd).unwrap().command, expected, "{cmd}");
            assert_eq!(parse(cmd).unwrap().command.name(), expected.name());
        }

        let mut matcher = FuzzyMatcher::new();
        assert_eq!(filter_commands("", &mut matcher).len(), 12);
        assert!(
            filter_commands("c", &mut matcher)
                .iter()
                .all(|row| !SECRET_COMMANDS.contains(row.command))
        );
        let co = filter_commands("co", &mut matcher);
        assert_eq!(co[0].command.kind, CommandKind::Theme);
        assert!(co.iter().any(|row| row.command.name == "config"));
        assert!(co.iter().any(|row| row.command.name == "codex"));
        assert!(co.iter().any(|row| row.command.name == "compact"));
        assert!(co.iter().any(|row| row.command.name == "cost"));

        let att = filter_commands("att", &mut matcher);
        assert_eq!(att.len(), 1);
        assert_eq!(att[0].command.kind, CommandKind::Resume);

        let cl = filter_commands("cl", &mut matcher);
        assert_eq!(cl.first().map(|r| r.command.name), Some("claude"));

        assert_eq!(
            filter_commands("atg", &mut matcher)
                .first()
                .map(|r| r.command.name),
            Some("antigravity")
        );
        assert_eq!(
            filter_commands("rsm", &mut matcher)
                .first()
                .map(|r| r.command.name),
            Some("resume")
        );
    }
}
