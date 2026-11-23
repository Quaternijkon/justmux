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
    Layout,
    Expose,
    Clients,
    Buffers,
    Snapshot,
    Popup,
    KillServer,
    Theme,
    Config,
    Update,
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
            Self::Layout => "layout",
            Self::Expose => "expose",
            Self::Clients => "clients",
            Self::Buffers => "buffers",
            Self::Snapshot => "snapshot",
            Self::Popup => "popup",
            Self::KillServer => "kill-server",
            Self::Theme => "theme",
            Self::Config => "config",
            Self::Update => "update",
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
pub enum CommandGroup {
    Workspace,
    Studio,
    System,
}

impl CommandGroup {
    pub const fn label(self) -> &'static str {
        self.label_for(crate::ui::Language::En)
    }

    pub const fn label_for(self, lang: crate::ui::Language) -> &'static str {
        match (lang, self) {
            (crate::ui::Language::Zh, Self::Workspace) => "工作区",
            (crate::ui::Language::Zh, Self::Studio) => "工作室",
            (crate::ui::Language::Zh, Self::System) => "系统",
            (crate::ui::Language::En, Self::Workspace) => "workspace",
            (crate::ui::Language::En, Self::Studio) => "studio",
            (crate::ui::Language::En, Self::System) => "system",
        }
    }
}

/// Localized command description for `/` popup hints and `/help`.
pub const fn description_for(kind: CommandKind, lang: crate::ui::Language) -> &'static str {
    if !matches!(lang, crate::ui::Language::Zh) {
        return match kind {
            CommandKind::Resume => "Open a window in an existing tmux session",
            CommandKind::New => "Create a tmux session",
            CommandKind::Window => "Open, create, move, rename, or close windows",
            CommandKind::Split => "Open interactive 2x2 visual Split Studio",
            CommandKind::Layout => "Visual layout presets & interactive pane resize",
            CommandKind::Expose => "Mission Control live multi-window preview wall",
            CommandKind::Rename => "Rename a session or window in workspace",
            CommandKind::Kill => "Kill a session or window with confirmation",
            CommandKind::Pane => "Inspect, zoom, break, swap, or close individual panes",
            CommandKind::Clients => "Manage attached clients & fix screen-size locks",
            CommandKind::Buffers => "Browse, copy & paste tmux paste buffers",
            CommandKind::Snapshot => "Save & restore workspace layouts",
            CommandKind::Popup => "Open a floating scratchpad terminal (tmux 3.2+)",
            CommandKind::Config => "Configure screen mode, theme, preview & tmux.conf",
            CommandKind::Theme => "Live-preview and switch color themes",
            CommandKind::Help => "Keyboard shortcuts & command reference",
            CommandKind::Exit => "Detach the current tmux client",
            CommandKind::KillServer => "Stop the entire tmux server (requires yes)",
            CommandKind::Quit => "Quit JustMux (or /exit to detach client)",
            CommandKind::List => "List tmux sessions",
            CommandKind::Update => "How to update JustMux (jm update)",
            CommandKind::Tribute(TributeId::Codex) => "Celebrate OpenAI Codex CLI craft",
            CommandKind::Tribute(TributeId::ClaudeCode) => "Celebrate Anthropic Claude Code flow",
            CommandKind::Tribute(TributeId::Antigravity) => "Celebrate Google DeepMind Antigravity",
            CommandKind::Tribute(TributeId::HallOfFame) => "Open the JustMux Tribute Vault",
            CommandKind::MicroTribute(MicroTributeKind::Compact) => {
                "Zero-token tmux scrollback check"
            }
            CommandKind::MicroTribute(MicroTributeKind::Cost) => "Inspect native tmux token budget",
            CommandKind::MicroTribute(MicroTributeKind::Boost) => "Engage turbo aurora mode",
            CommandKind::MicroTribute(MicroTributeKind::GrillMe) => {
                "Interrogate your tmux architecture"
            }
            CommandKind::MicroTribute(MicroTributeKind::Goal) => "Lock in zero-memorization tmux",
            CommandKind::MicroTribute(MicroTributeKind::Model) => "Check active native tmux engine",
        };
    }
    match kind {
        CommandKind::Resume => "在已有 tmux 会话中打开窗口",
        CommandKind::New => "创建新的 tmux 会话",
        CommandKind::Window => "打开、新建、移动、重命名或关闭窗口",
        CommandKind::Split => "打开 2x2 可视化分屏工作台",
        CommandKind::Layout => "可视化布局预设与交互式窗格尺寸调整",
        CommandKind::Expose => "Mission Control 多窗口实时预览全景墙",
        CommandKind::Rename => "重命名工作区中的会话或窗口",
        CommandKind::Kill => "二次确认并结束会话或窗口",
        CommandKind::Pane => "查看、全屏缩放、拆出、互换或关闭单个窗格",
        CommandKind::Clients => "管理已附着客户端并解除点阵边框尺寸锁定",
        CommandKind::Buffers => "浏览、复制到剪贴板与粘贴 tmux 缓冲区",
        CommandKind::Snapshot => "保存与一键恢复工作区快照布局",
        CommandKind::Popup => "打开原生悬浮临时终端窗口 (tmux 3.2+)",
        CommandKind::Config => "修改显示模式、主题、预览与 ~/.tmux.conf 快捷配置",
        CommandKind::Theme => "实时预览并切换配色主题",
        CommandKind::Help => "查看快捷键与命令参考",
        CommandKind::Exit => "分离当前所在的 tmux 客户端",
        CommandKind::KillServer => "关闭整个 tmux 服务进程（需输入 yes 确认）",
        CommandKind::Quit => "退出 JustMux（/exit 仅分离当前客户端）",
        CommandKind::List => "浏览所有 tmux 会话列表",
        CommandKind::Update => "查看 JustMux 更新方式 (jm update)",
        CommandKind::Tribute(TributeId::Codex) => "致敬 OpenAI Codex CLI 匠心",
        CommandKind::Tribute(TributeId::ClaudeCode) => "致敬 Anthropic Claude Code 心流",
        CommandKind::Tribute(TributeId::Antigravity) => "致敬 Google DeepMind Antigravity",
        CommandKind::Tribute(TributeId::HallOfFame) => "打开 JustMux 致敬纪念馆",
        CommandKind::MicroTribute(MicroTributeKind::Compact) => "零消耗查看 tmux 滚动历史",
        CommandKind::MicroTribute(MicroTributeKind::Cost) => "检查原生 tmux 零消耗额度",
        CommandKind::MicroTribute(MicroTributeKind::Boost) => "激活极光性能增益模式",
        CommandKind::MicroTribute(MicroTributeKind::GrillMe) => "审视当前 tmux 会话结构",
        CommandKind::MicroTribute(MicroTributeKind::Goal) => "坚持零记忆直觉式 tmux 操控",
        CommandKind::MicroTribute(MicroTributeKind::Model) => "查看当前原生 tmux 引擎状态",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub kind: CommandKind,
    pub group: CommandGroup,
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
    // Workspace (0..=5)
    SlashCommand {
        kind: CommandKind::Resume,
        group: CommandGroup::Workspace,
        name: "resume",
        aliases: &["r", "attach", "switch"],
        description: "Open a window in an existing tmux session",
        usage: "/resume [@target]",
    },
    SlashCommand {
        kind: CommandKind::New,
        group: CommandGroup::Workspace,
        name: "new",
        aliases: &["n"],
        description: "Create a tmux session",
        usage: "/new [name] [directory]",
    },
    SlashCommand {
        kind: CommandKind::Window,
        group: CommandGroup::Workspace,
        name: "window",
        aliases: &["w", "win", "windows"],
        description: "Open, create, move, rename, or close windows",
        usage: "/window [@target]",
    },
    SlashCommand {
        kind: CommandKind::Rename,
        group: CommandGroup::Workspace,
        name: "rename",
        aliases: &["mv"],
        description: "Rename a session or window in workspace",
        usage: "/rename [@target new-name]",
    },
    SlashCommand {
        kind: CommandKind::Kill,
        group: CommandGroup::Workspace,
        name: "kill",
        aliases: &["k", "close", "rm"],
        description: "Kill a session or window with confirmation",
        usage: "/kill [@target]",
    },
    SlashCommand {
        kind: CommandKind::Snapshot,
        group: CommandGroup::Workspace,
        name: "snapshot",
        aliases: &["save", "restore", "template", "workspace"],
        description: "Save & restore workspace layouts",
        usage: "/snapshot [save|restore]",
    },
    // Studio (6..=13)
    SlashCommand {
        kind: CommandKind::Pane,
        group: CommandGroup::Studio,
        name: "pane",
        aliases: &["p"],
        description: "Inspect, zoom, break, swap, or close individual panes",
        usage: "/pane [@target]",
    },
    SlashCommand {
        kind: CommandKind::Split,
        group: CommandGroup::Studio,
        name: "split",
        aliases: &["s"],
        description: "Open interactive 2x2 visual Split Studio",
        usage: "/split [horizontal|vertical|@target]",
    },
    SlashCommand {
        kind: CommandKind::Layout,
        group: CommandGroup::Studio,
        name: "layout",
        aliases: &["resize", "arrange", "l"],
        description: "Visual layout presets & interactive pane resize",
        usage: "/layout [preset|@target]",
    },
    SlashCommand {
        kind: CommandKind::Expose,
        group: CommandGroup::Studio,
        name: "expose",
        aliases: &["mission", "wall", "grid", "overview"],
        description: "Mission Control live multi-window preview wall",
        usage: "/expose",
    },
    SlashCommand {
        kind: CommandKind::Buffers,
        group: CommandGroup::Studio,
        name: "buffers",
        aliases: &["buffer", "clip", "paste", "copy"],
        description: "Browse, copy & paste tmux paste buffers",
        usage: "/buffers",
    },
    SlashCommand {
        kind: CommandKind::Popup,
        group: CommandGroup::Studio,
        name: "popup",
        aliases: &["scratch", "float"],
        description: "Open a floating scratchpad terminal (tmux 3.2+)",
        usage: "/popup",
    },
    SlashCommand {
        kind: CommandKind::Theme,
        group: CommandGroup::Studio,
        name: "theme",
        aliases: &["palette", "colors"],
        description: "Live-preview and switch color themes",
        usage: "/theme [aurora|codex|claude|matrix|classic]",
    },
    SlashCommand {
        kind: CommandKind::Config,
        group: CommandGroup::Studio,
        name: "config",
        aliases: &["settings", "prefs"],
        description: "Configure screen mode, theme, preview & tmux.conf",
        usage: "/config",
    },
    // System (14..=18)
    SlashCommand {
        kind: CommandKind::Clients,
        group: CommandGroup::System,
        name: "clients",
        aliases: &["client", "who"],
        description: "Manage attached clients & fix screen-size locks",
        usage: "/clients",
    },
    SlashCommand {
        kind: CommandKind::Help,
        group: CommandGroup::System,
        name: "help",
        aliases: &["?"],
        description: "Keyboard shortcuts & command reference",
        usage: "/help [command]",
    },
    SlashCommand {
        kind: CommandKind::Exit,
        group: CommandGroup::System,
        name: "exit",
        aliases: &["detach"],
        description: "Detach the current tmux client",
        usage: "/exit",
    },
    SlashCommand {
        kind: CommandKind::KillServer,
        group: CommandGroup::System,
        name: "kill-server",
        aliases: &["shutdown"],
        description: "Stop the entire tmux server (requires yes)",
        usage: "/kill-server",
    },
    SlashCommand {
        kind: CommandKind::Quit,
        group: CommandGroup::System,
        name: "quit",
        aliases: &["q"],
        description: "Quit JustMux (or /exit to detach client)",
        usage: "/quit",
    },
];

pub const MERGED_COMMANDS: &[SlashCommand] = &[SlashCommand {
    kind: CommandKind::List,
    group: CommandGroup::Workspace,
    name: "list",
    aliases: &["ls"],
    description: "List tmux sessions",
    usage: "/list",
}];

pub const SECRET_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::Codex),
        group: CommandGroup::Studio,
        name: "codex",
        aliases: &["openai", "approval"],
        description: "Celebrate OpenAI Codex CLI craft",
        usage: "/codex",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::ClaudeCode),
        group: CommandGroup::Studio,
        name: "claude",
        aliases: &["cc", "clauding", "ultrathink"],
        description: "Celebrate Anthropic Claude Code flow",
        usage: "/claude",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::Antigravity),
        group: CommandGroup::Studio,
        name: "antigravity",
        aliases: &["agy", "liftoff", "orbit"],
        description: "Celebrate Google DeepMind Antigravity",
        usage: "/antigravity",
    },
    SlashCommand {
        kind: CommandKind::Tribute(TributeId::HallOfFame),
        group: CommandGroup::Studio,
        name: "vibe",
        aliases: &["easter", "credits", "tributes"],
        description: "Open the JustMux Tribute Vault",
        usage: "/vibe",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Compact),
        group: CommandGroup::Studio,
        name: "compact",
        aliases: &[],
        description: "Zero-token tmux scrollback check",
        usage: "/compact",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Cost),
        group: CommandGroup::Studio,
        name: "cost",
        aliases: &[],
        description: "Inspect native tmux token budget",
        usage: "/cost",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Boost),
        group: CommandGroup::Studio,
        name: "boost",
        aliases: &[],
        description: "Engage turbo aurora mode",
        usage: "/boost",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::GrillMe),
        group: CommandGroup::Studio,
        name: "grill-me",
        aliases: &["grill"],
        description: "Interrogate your tmux architecture",
        usage: "/grill-me",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Goal),
        group: CommandGroup::Studio,
        name: "goal",
        aliases: &[],
        description: "Lock in zero-memorization tmux",
        usage: "/goal",
    },
    SlashCommand {
        kind: CommandKind::MicroTribute(MicroTributeKind::Model),
        group: CommandGroup::Studio,
        name: "model",
        aliases: &[],
        description: "Check active native tmux engine",
        usage: "/model",
    },
    SlashCommand {
        kind: CommandKind::Update,
        group: CommandGroup::System,
        name: "update",
        aliases: &["upgrade"],
        description: "How to update JustMux (jm update)",
        usage: "/update",
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
            MERGED_COMMANDS
                .iter()
                .enumerate()
                .map(|(order, command)| (1_u8, order, command)),
        )
        .chain(
            include_secrets
                .then(|| {
                    SECRET_COMMANDS
                        .iter()
                        .enumerate()
                        .map(|(order, command)| (2_u8, order, command))
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
        .chain(MERGED_COMMANDS.iter())
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
        .chain(MERGED_COMMANDS.iter())
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
        for command in registry()
            .iter()
            .chain(MERGED_COMMANDS.iter())
            .chain(SECRET_COMMANDS.iter())
        {
            assert!(names.insert(command.name));
            for alias in command.aliases {
                assert!(names.insert(*alias));
            }
        }
    }

    #[test]
    fn registry_surfaces_zero_memorization_commands_and_merged_list_completes() {
        assert_eq!(registry().len(), 19);
        assert_eq!(
            registry()
                .iter()
                .map(|c| (c.kind, c.group))
                .collect::<Vec<_>>(),
            vec![
                (CommandKind::Resume, CommandGroup::Workspace),
                (CommandKind::New, CommandGroup::Workspace),
                (CommandKind::Window, CommandGroup::Workspace),
                (CommandKind::Rename, CommandGroup::Workspace),
                (CommandKind::Kill, CommandGroup::Workspace),
                (CommandKind::Snapshot, CommandGroup::Workspace),
                (CommandKind::Pane, CommandGroup::Studio),
                (CommandKind::Split, CommandGroup::Studio),
                (CommandKind::Layout, CommandGroup::Studio),
                (CommandKind::Expose, CommandGroup::Studio),
                (CommandKind::Buffers, CommandGroup::Studio),
                (CommandKind::Popup, CommandGroup::Studio),
                (CommandKind::Theme, CommandGroup::Studio),
                (CommandKind::Config, CommandGroup::Studio),
                (CommandKind::Clients, CommandGroup::System),
                (CommandKind::Help, CommandGroup::System),
                (CommandKind::Exit, CommandGroup::System),
                (CommandKind::KillServer, CommandGroup::System),
                (CommandKind::Quit, CommandGroup::System),
            ]
        );
        // Verify groups appear in strictly contiguous blocks (Workspace -> Studio -> System).
        let mut seen_groups = Vec::new();
        for cmd in registry() {
            if seen_groups.last() != Some(&cmd.group) {
                assert!(
                    !seen_groups.contains(&cmd.group),
                    "command group {:?} is not contiguous in COMMANDS",
                    cmd.group
                );
                seen_groups.push(cmd.group);
            }
        }
        assert_eq!(
            seen_groups,
            vec![
                CommandGroup::Workspace,
                CommandGroup::Studio,
                CommandGroup::System,
            ]
        );
        assert_eq!(CommandGroup::Workspace.label(), "workspace");
        assert_eq!(CommandGroup::Studio.label(), "studio");
        assert_eq!(CommandGroup::System.label(), "system");

        assert_eq!(
            parse("/theme aurora").unwrap(),
            ParsedCommand {
                command: CommandKind::Theme,
                args: vec!["aurora".into()],
            }
        );
        assert_eq!(parse("/palette").unwrap().command, CommandKind::Theme);
        assert_eq!(parse("/colors").unwrap().command, CommandKind::Theme);
        assert_eq!(parse("/close").unwrap().command, CommandKind::Kill);
        assert_eq!(parse("/rm").unwrap().command, CommandKind::Kill);

        // New controller commands & aliases parse cleanly.
        for (cmd, expected) in [
            ("/layout", CommandKind::Layout),
            ("/resize", CommandKind::Layout),
            ("/arrange", CommandKind::Layout),
            ("/l", CommandKind::Layout),
            ("/expose", CommandKind::Expose),
            ("/mission", CommandKind::Expose),
            ("/wall", CommandKind::Expose),
            ("/grid", CommandKind::Expose),
            ("/overview", CommandKind::Expose),
            ("/clients", CommandKind::Clients),
            ("/client", CommandKind::Clients),
            ("/who", CommandKind::Clients),
            ("/buffers", CommandKind::Buffers),
            ("/buffer", CommandKind::Buffers),
            ("/clip", CommandKind::Buffers),
            ("/paste", CommandKind::Buffers),
            ("/copy", CommandKind::Buffers),
            ("/snapshot", CommandKind::Snapshot),
            ("/save", CommandKind::Snapshot),
            ("/restore", CommandKind::Snapshot),
            ("/template", CommandKind::Snapshot),
            ("/workspace", CommandKind::Snapshot),
            ("/popup", CommandKind::Popup),
            ("/scratch", CommandKind::Popup),
            ("/float", CommandKind::Popup),
            ("/kill-server", CommandKind::KillServer),
            ("/shutdown", CommandKind::KillServer),
        ] {
            assert_eq!(parse(cmd).unwrap().command, expected, "{cmd}");
            assert!(!description_for(expected, crate::ui::Language::En).is_empty());
            assert!(!description_for(expected, crate::ui::Language::Zh).is_empty());
        }

        // Core commands surfaced on bare `/`; `/list` remains searchable via MERGED_COMMANDS.
        assert_eq!(parse("/new demo").unwrap().command, CommandKind::New);
        assert_eq!(parse("/n").unwrap().command, CommandKind::New);
        assert_eq!(parse("/window").unwrap().command, CommandKind::Window);
        assert_eq!(parse("/list").unwrap().command, CommandKind::List);
        assert_eq!(parse("/ls").unwrap().command, CommandKind::List);
        assert_eq!(parse("/exit").unwrap().command, CommandKind::Exit);
        assert_eq!(parse("/detach").unwrap().command, CommandKind::Exit);
        assert!(parse("/exitt").unwrap_err().to_string().contains("/exit?"));

        let mut matcher = FuzzyMatcher::new();
        let bare = filter_commands("", &mut matcher);
        assert_eq!(bare.len(), registry().len());
        assert!(
            bare.iter()
                .all(|row| !MERGED_COMMANDS.contains(row.command))
        );
        for surfaced in [
            CommandKind::Exit,
            CommandKind::New,
            CommandKind::Window,
            CommandKind::Pane,
            CommandKind::Layout,
            CommandKind::Clients,
            CommandKind::Buffers,
            CommandKind::Snapshot,
            CommandKind::Popup,
        ] {
            assert!(
                bare.iter().any(|row| row.command.kind == surfaced),
                "expected {surfaced:?} on bare / palette"
            );
        }

        assert!(
            filter_commands("n", &mut matcher)
                .iter()
                .any(|row| row.command.kind == CommandKind::New)
        );
        assert!(
            filter_commands("ls", &mut matcher)
                .iter()
                .any(|row| row.command.kind == CommandKind::List)
        );
        assert!(
            filter_commands("det", &mut matcher)
                .iter()
                .any(|row| row.command.kind == CommandKind::Exit)
        );
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
            ("/update", CommandKind::Update),
            ("/upgrade", CommandKind::Update),
        ] {
            assert_eq!(parse(cmd).unwrap().command, expected, "{cmd}");
            assert_eq!(parse(cmd).unwrap().command.name(), expected.name());
        }

        let mut matcher = FuzzyMatcher::new();
        assert_eq!(filter_commands("", &mut matcher).len(), registry().len());
        assert!(
            filter_commands("c", &mut matcher)
                .iter()
                .all(|row| !SECRET_COMMANDS.contains(row.command))
        );
        let co = filter_commands("co", &mut matcher);
        assert_eq!(co[0].command.kind, CommandKind::Config);
        assert!(co.iter().any(|row| row.command.kind == CommandKind::Theme));
        assert!(
            co.iter()
                .any(|row| row.command.kind == CommandKind::Buffers)
        );
        assert!(co.iter().any(|row| row.command.name == "codex"));
        assert!(co.iter().any(|row| row.command.name == "compact"));
        assert!(co.iter().any(|row| row.command.name == "cost"));

        let att = filter_commands("att", &mut matcher);
        assert_eq!(att.len(), 1);
        assert_eq!(att[0].command.kind, CommandKind::Resume);

        let cl = filter_commands("cl", &mut matcher);
        assert_eq!(cl[0].command.kind, CommandKind::Clients);
        assert_eq!(cl[0].alias, None);
        assert!(cl.iter().any(|r| r.command.kind == CommandKind::Kill));
        assert!(cl.iter().any(|r| r.command.kind == CommandKind::Buffers));
        assert!(cl.iter().any(|r| r.command.name == "claude"));

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
