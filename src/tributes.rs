//! Tribute & Easter Egg engine celebrating OpenAI Codex CLI,
//! Anthropic Claude Code, and Google DeepMind Antigravity.

/// Identifies a showcase card in JustMux's interactive tribute tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TributeId {
    Codex,
    ClaudeCode,
    Antigravity,
    HallOfFame,
}

impl TributeId {
    /// All tribute cards in tour order.
    pub const ALL: &'static [Self] = &[
        Self::Codex,
        Self::ClaudeCode,
        Self::Antigravity,
        Self::HallOfFame,
    ];

    /// Advances to the next tribute card, wrapping around `ALL`.
    pub const fn next(self) -> Self {
        match self {
            Self::Codex => Self::ClaudeCode,
            Self::ClaudeCode => Self::Antigravity,
            Self::Antigravity => Self::HallOfFame,
            Self::HallOfFame => Self::Codex,
        }
    }

    /// Moves to the previous tribute card, wrapping around `ALL`.
    pub const fn prev(self) -> Self {
        match self {
            Self::Codex => Self::HallOfFame,
            Self::ClaudeCode => Self::Codex,
            Self::Antigravity => Self::ClaudeCode,
            Self::HallOfFame => Self::Antigravity,
        }
    }
}

/// Playful slash-command winks borrowed from AI coding CLI culture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MicroTributeKind {
    Compact,
    Cost,
    Boost,
    GrillMe,
    Goal,
    Model,
}

/// Complete showcase payload rendered inside the tribute modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TributeCard {
    pub id: TributeId,
    pub badge: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub telemetry: &'static str,
    pub art: &'static [&'static str],
    pub ascii_art: &'static [&'static str],
    pub highlights: &'static [&'static str],
    pub status_quip: String,
    pub footer_hint: &'static str,
}

const FOOTER_HINT: &str =
    "Enter/t Apply Signature Theme · Space Remix · ←→ Tour Tributes · Esc Back";

const CODEX_ART: &[&str] = &[
    "┏━━━━━━━━━━━━━━━━━━━━━━━━┓",
    "┃ >_ codex · diff & run  ┃",
    "┗━━━━━━━━━━━[on-confirm]━┛",
];

const CODEX_ASCII_ART: &[&str] = &[
    "+------------------------+",
    "| >_ codex . diff & run  |",
    "+-----------[on-confirm]-+",
];

const CODEX_HIGHLIGHTS: &[&str] = &[
    "Bottom-anchored slash composer that respects terminal scrollback",
    "Inline slash completion popup with crisp fuzzy index highlights",
    "Deliberate on-confirm guardrails before destructive kill actions",
];

const CODEX_QUIPS: &[&str] = &[
    ">_ Inspecting tmux tree… zero sandbox escapes detected",
    ">_ Anchoring composer to the bottom edge like a pro…",
    ">_ Staging session diff: +1 window, -0 Ctrl+b regrets",
    ">_ Applying patch to pane layout… confirm? [Enter]",
    ">_ Running /resume fast-path without leaving your terminal",
    ">_ Token budget: 0 tokens spent · 100% native tmux",
];

const CLAUDE_ART: &[&str] = &[
    "╭────────────────────────╮",
    "│ ✻ claude · ultrathink  │",
    "╰─────────[/compact: 0%]─╯",
];

const CLAUDE_ASCII_ART: &[&str] = &[
    ".------------------------.",
    "| * claude . ultrathink  |",
    "`---------[/compact: 0%]-'",
];

const CLAUDE_HIGHLIGHTS: &[&str] = &[
    "Warm terracotta focus cues and iconic ✻ spinner personality",
    "Fluid slash-command discovery so /help replaces cheat sheets",
    "Zero-friction session switching inspired by /resume workflows",
];

const CLAUDE_VERBS: &[&str] = &[
    "✻ Clauding…",
    "✻ Combobulating panes…",
    "✻ Reticulating splines…",
    "✻ Ultrathinking layout…",
    "✻ Herding tmux sessions…",
    "✻ Marinating buffers…",
    "✻ Aligning pane chakras…",
    "✻ Discombobulating Ctrl+b…",
];

const ANTIGRAVITY_ART: &[&str] = &[
    "╔════════════════════════╗",
    "║ ✦ antigravity · 0.00g  ║",
    "╚═════[plan -> swarm]════╝",
];

const ANTIGRAVITY_ASCII_ART: &[&str] = &[
    "+========================+",
    "| * antigravity . 0.00g  |",
    "+=====[plan -> swarm]====+",
];

const ANTIGRAVITY_HIGHLIGHTS: &[&str] = &[
    "Luminous cyan-to-violet aurora palette for TrueColor terminals",
    "Visual pane wireframes and session grids instead of guesswork",
    "Multi-pane /resume navigation built for fast agent swarms",
];

const ANTIGRAVITY_QUIPS: &[&str] = &[
    "✦ Zero-G engaged… tmux prefix chords drifting into orbit",
    "✦ Plotting /plan -> /split trajectory across active panes…",
    "✦ Aurora telemetry nominal · 0.00g terminal resistance",
    "✦ Dispatching panes in parallel without touching ~/.tmux.conf",
    "✦ Gravity disabled: panes float cleanly into side-by-side splits",
    "✦ Mission control reports 0 memorized keybinds required",
];

const HALL_ART: &[&str] = &[
    "◈ ╭────────────────────╮ ◈",
    "  │ >_  +  ✻  +  ✦     │  ",
    "◈ ╰───[justmux core]───╯ ◈",
];

const HALL_ASCII_ART: &[&str] = &[
    "* .--------------------. *",
    "  | >_  +  *  +  +     |  ",
    "* `---[justmux core]---' *",
];

const HALL_HIGHLIGHTS: &[&str] = &[
    "Zero daemon, zero lock-in: every command drives native tmux",
    "5 switchable palettes with automatic NO_COLOR & ASCII fallback",
    "Hidden slash winks (/compact, /cost, /boost, /grill-me, /goal, /model)",
];

const HALL_QUIPS: &[&str] = &[
    "◈ Three AI TUI legends walked into a tmux socket…",
    "◈ Press Enter or t to wear this card's signature palette",
    "◈ Tip: type /compact, /cost, /boost, /grill-me, /goal, or /model",
    "◈ Press Ctrl+b anytime if your muscle memory misses 1995",
    "◈ Spacebar remixes the quip; ← → tours all three titans",
    "◈ Just tmux. Just simpler. (And slightly more charismatic.)",
];

const COMPACT_MESSAGES: &[&str] = &[
    "✻ Context compacted to 0 tokens — tmux scrollback left untouched; use /pane or /window to tidy layouts.",
    "✻ /compact complete: 100% of your tmux sessions survived with zero summarization loss.",
    "✻ Buffers stay lossless in tmux; try /window or /kill if you actually want to trim clutter.",
];

const COST_MESSAGES: &[&str] = &[
    "◈ Session cost: $0.0000 · 0 tokens burned · 100% local tmux muscle. (Press /vibe to admire the craft.)",
    "◈ Billing report: $0.00 spent on API credits, ∞ pinky-strain saved by retiring Ctrl+b chords.",
    "◈ Token telemetry: 0 input · 0 output · 1,000% pure Rust + native tmux.",
];

const BOOST_MESSAGES: &[&str] = &[
    "🔥 Turbo engaged: 0ms LLM round-trip — JustMux fires raw tmux commands at native speed.",
    "🔥 Boost mode is always on here: fuzzy /resume and instant /split run straight against tmux.",
    "🔥 Overclocked! (Okay, it was already instant — try /split or Ctrl+T to boost your theme too.)",
];

const GRILL_ME_MESSAGES: &[&str] = &[
    "✦ /grill-me: Ready for interrogation — try /list, /resume, or /pane to inspect every live tmux corner.",
    "✦ Architecture check passed: no daemon, no ~/.tmux.conf edits, just honest tmux calls.",
    "✦ Grilling your layout? Use /resume to audit sessions or /vibe to inspect our design inspirations.",
];

const GOAL_MESSAGES: &[&str] = &[
    "🎯 Goal locked: zero memorized tmux chords. Start with /new, /resume, /split, or /help.",
    "🎯 Mission objective: navigate every session, window, and pane cleanly — try /resume to jump right in.",
    "🎯 Overnight goal? tmux keeps your panes alive even after you /exit or /quit JustMux.",
];

const MODEL_MESSAGES: &[&str] = &[
    "✦ Active model: native-tmux (0B params, 0ms latency). Switch visual flavor anytime with /theme or Ctrl+T!",
    "✻ Model roster: Codex Emerald · Claude Warmth · Antigravity Aurora — pick a vibe with /theme.",
    "◈ No weights loaded — only pure tmux primitives. Try /codex, /claude, or /antigravity for a tour.",
];

const CTRL_B_QUIPS: &[&str] = &[
    "✦ Ctrl+b detected! Your pinky can rest now — just type / for commands.",
    "✻ Muscle memory spotted! No prefix chord needed here; try /split, /window, or /resume.",
    "◈ Reflexive Ctrl+b intercepted (0 tmux panes harmed). Type / or ↓ to browse actions.",
    "✦ Nice try, tmux veteran! Prefix chords are retired in JustMux — hit /help for the modern way.",
    "✻ Ctrl+b? In this economy? Keep both hands relaxed and type /new, /pane, or /theme.",
    "◈ Prefix key archived in the Tribute Vault — inside attached tmux Ctrl+b still works; here just type /!",
];

fn pick(options: &'static [&'static str], step: usize) -> &'static str {
    if options.is_empty() {
        return "";
    }
    options.get(step % options.len()).copied().unwrap_or("")
}

/// Builds the full showcase [`TributeCard`] for `id` at rotation `step`.
pub fn card(id: TributeId, step: usize) -> TributeCard {
    match id {
        TributeId::Codex => TributeCard {
            id,
            badge: ">_ CODEX CLI",
            title: "OpenAI Codex CLI — Terminal Craft",
            subtitle: "Bottom-anchored composer, inline diffs, and fearless sandbox discipline",
            telemetry: "sandbox: native-tmux · approval: on-confirm · composer: bottom-anchored",
            art: CODEX_ART,
            ascii_art: CODEX_ASCII_ART,
            highlights: CODEX_HIGHLIGHTS,
            status_quip: pick(CODEX_QUIPS, step).to_owned(),
            footer_hint: FOOTER_HINT,
        },
        TributeId::ClaudeCode => TributeCard {
            id,
            badge: "✻ CLAUDE CODE",
            title: "Anthropic Claude Code — Flow & Joy",
            subtitle: "Signature asterisk pulse, slash verbs, and effortless ergonomics",
            telemetry: "context: ∞% until /compact · effort: ultrathink · pinky-strain: 0",
            art: CLAUDE_ART,
            ascii_art: CLAUDE_ASCII_ART,
            highlights: CLAUDE_HIGHLIGHTS,
            status_quip: claude_verb(step).to_owned(),
            footer_hint: FOOTER_HINT,
        },
        TributeId::Antigravity => TributeCard {
            id,
            badge: "✦ ANTIGRAVITY",
            title: "Google DeepMind Antigravity — Zero-G",
            subtitle: "Aurora gradients, parallel swarm clarity, and weightless tmux flight",
            telemetry: "gravity: 0.00g · mode: plan->swarm · prefix-chords: retired",
            art: ANTIGRAVITY_ART,
            ascii_art: ANTIGRAVITY_ASCII_ART,
            highlights: ANTIGRAVITY_HIGHLIGHTS,
            status_quip: pick(ANTIGRAVITY_QUIPS, step).to_owned(),
            footer_hint: FOOTER_HINT,
        },
        TributeId::HallOfFame => TributeCard {
            id,
            badge: "◈ TRIBUTE VAULT",
            title: "JustMux Tribute Vault — Best of All Worlds",
            subtitle: "Codex precision + Claude warmth + Antigravity polish over native tmux",
            telemetry: "titans: codex + claude + antigravity · palettes: 5 · secrets: active",
            art: HALL_ART,
            ascii_art: HALL_ASCII_ART,
            highlights: HALL_HIGHLIGHTS,
            status_quip: pick(HALL_QUIPS, step).to_owned(),
            footer_hint: FOOTER_HINT,
        },
    }
}

/// Returns a rotating Claude Code–inspired spinner verb.
pub fn claude_verb(step: usize) -> &'static str {
    pick(CLAUDE_VERBS, step)
}

/// Returns a rotating zero-memorization tmux wink for a hidden tribute slash command.
pub fn micro_message(kind: MicroTributeKind, step: usize) -> &'static str {
    match kind {
        MicroTributeKind::Compact => pick(COMPACT_MESSAGES, step),
        MicroTributeKind::Cost => pick(COST_MESSAGES, step),
        MicroTributeKind::Boost => pick(BOOST_MESSAGES, step),
        MicroTributeKind::GrillMe => pick(GRILL_ME_MESSAGES, step),
        MicroTributeKind::Goal => pick(GOAL_MESSAGES, step),
        MicroTributeKind::Model => pick(MODEL_MESSAGES, step),
    }
}

/// Returns a rotating playful Easter egg when a user reflexively presses `Ctrl+b`.
pub fn ctrl_b_quip(step: usize) -> &'static str {
    pick(CTRL_B_QUIPS, step)
}

/// Returns a subtle icon badge for sessions named after Codex, Claude, or Antigravity.
pub fn session_badge(name: &str, ascii: bool) -> Option<&'static str> {
    if ascii {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    if lower.contains("codex") {
        Some(">_")
    } else if lower.contains("claude") {
        Some("✻")
    } else if lower.contains("agy") || lower.contains("antigravity") {
        Some("✦")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn tribute_id_next_and_prev_cycle_cleanly() {
        assert_eq!(TributeId::ALL.len(), 4);
        assert_eq!(TributeId::Codex.next(), TributeId::ClaudeCode);
        assert_eq!(TributeId::ClaudeCode.next(), TributeId::Antigravity);
        assert_eq!(TributeId::Antigravity.next(), TributeId::HallOfFame);
        assert_eq!(TributeId::HallOfFame.next(), TributeId::Codex);

        assert_eq!(TributeId::Codex.prev(), TributeId::HallOfFame);
        assert_eq!(TributeId::HallOfFame.prev(), TributeId::Antigravity);
        assert_eq!(TributeId::Antigravity.prev(), TributeId::ClaudeCode);
        assert_eq!(TributeId::ClaudeCode.prev(), TributeId::Codex);

        for &id in TributeId::ALL {
            assert_eq!(id.next().prev(), id);
            assert_eq!(id.prev().next(), id);
        }
    }

    #[test]
    fn all_cards_have_non_empty_strings_and_pure_ascii_art() {
        let expected_badges = [
            (TributeId::Codex, ">_ CODEX CLI"),
            (TributeId::ClaudeCode, "✻ CLAUDE CODE"),
            (TributeId::Antigravity, "✦ ANTIGRAVITY"),
            (TributeId::HallOfFame, "◈ TRIBUTE VAULT"),
        ];
        let expected_telemetry = [
            (
                TributeId::Codex,
                "sandbox: native-tmux · approval: on-confirm · composer: bottom-anchored",
            ),
            (
                TributeId::ClaudeCode,
                "context: ∞% until /compact · effort: ultrathink · pinky-strain: 0",
            ),
            (
                TributeId::Antigravity,
                "gravity: 0.00g · mode: plan->swarm · prefix-chords: retired",
            ),
            (
                TributeId::HallOfFame,
                "titans: codex + claude + antigravity · palettes: 5 · secrets: active",
            ),
        ];

        for &id in TributeId::ALL {
            for step in 0..10 {
                let c = card(id, step);
                assert_eq!(c.id, id);
                assert!(!c.badge.trim().is_empty());
                assert!(!c.title.trim().is_empty());
                assert!(!c.subtitle.trim().is_empty());
                assert!(!c.telemetry.trim().is_empty());
                assert!(!c.status_quip.trim().is_empty());
                assert_eq!(
                    c.footer_hint,
                    "Enter/t Apply Signature Theme · Space Remix · ←→ Tour Tributes · Esc Back"
                );
                assert_eq!(c.art.len(), 3);
                assert_eq!(c.ascii_art.len(), 3);
                assert_eq!(c.highlights.len(), 3);

                for line in c.art {
                    assert!(!line.trim().is_empty());
                }
                for line in c.ascii_art {
                    assert!(!line.trim().is_empty());
                    assert!(
                        line.is_ascii(),
                        "ascii_art line must be pure ASCII for {id:?}: {line:?}"
                    );
                }
                for line in c.highlights {
                    assert!(!line.trim().is_empty());
                }
            }

            let c = card(id, 0);
            assert!(expected_badges.contains(&(id, c.badge)));
            assert!(expected_telemetry.contains(&(id, c.telemetry)));
        }
    }

    #[test]
    fn step_cycling_rotates_status_quips_claude_verbs_and_ctrl_b_quips() {
        for &id in TributeId::ALL {
            let first = card(id, 0).status_quip;
            let second = card(id, 1).status_quip;
            assert_ne!(first, second, "status_quip did not change for {id:?}");
            let unique: HashSet<String> = (0..12).map(|s| card(id, s).status_quip).collect();
            assert!(unique.len() >= 4, "expected rotating quips for {id:?}");
        }

        assert_eq!(claude_verb(0), "✻ Clauding…");
        assert_ne!(claude_verb(0), claude_verb(1));
        let verbs: HashSet<&str> = (0..CLAUDE_VERBS.len()).map(claude_verb).collect();
        assert_eq!(verbs.len(), CLAUDE_VERBS.len());
        assert!(verbs.iter().all(|v| !v.trim().is_empty()));
        assert_eq!(claude_verb(CLAUDE_VERBS.len()), claude_verb(0));

        assert_ne!(ctrl_b_quip(0), ctrl_b_quip(1));
        let quips: HashSet<&str> = (0..CTRL_B_QUIPS.len()).map(ctrl_b_quip).collect();
        assert_eq!(quips.len(), CTRL_B_QUIPS.len());
        assert!(quips.iter().all(|q| !q.trim().is_empty()));
        assert_eq!(ctrl_b_quip(CTRL_B_QUIPS.len()), ctrl_b_quip(0));
    }

    #[test]
    fn micro_messages_are_non_empty_and_rotate_per_kind() {
        let kinds = [
            MicroTributeKind::Compact,
            MicroTributeKind::Cost,
            MicroTributeKind::Boost,
            MicroTributeKind::GrillMe,
            MicroTributeKind::Goal,
            MicroTributeKind::Model,
        ];

        for kind in kinds {
            let first = micro_message(kind, 0);
            let second = micro_message(kind, 1);
            assert!(!first.trim().is_empty());
            assert!(!second.trim().is_empty());
            assert_ne!(first, second, "micro_message should rotate for {kind:?}");
            let third = micro_message(kind, 2);
            assert!(!third.trim().is_empty());
            assert_ne!(second, third);
        }
    }

    #[test]
    fn session_badge_detects_agent_names_and_respects_ascii_flag() {
        assert_eq!(session_badge("codex", false), Some(">_"));
        assert_eq!(session_badge("OpenAI-CODEX-cli", false), Some(">_"));
        assert_eq!(session_badge("claude", false), Some("✻"));
        assert_eq!(session_badge("my-Claude-Code-pane", false), Some("✻"));
        assert_eq!(session_badge("agy", false), Some("✦"));
        assert_eq!(session_badge("AGY-swarm-1", false), Some("✦"));
        assert_eq!(session_badge("antigravity", false), Some("✦"));
        assert_eq!(session_badge("Antigravity-UI", false), Some("✦"));

        assert_eq!(session_badge("workspace", false), None);
        assert_eq!(session_badge("", false), None);

        assert_eq!(session_badge("codex", true), None);
        assert_eq!(session_badge("claude", true), None);
        assert_eq!(session_badge("agy", true), None);
        assert_eq!(session_badge("antigravity", true), None);
    }
}
