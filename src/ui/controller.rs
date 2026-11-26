//! Zero-memorization tmux controller subviews, interactive preview chrome, status badges,
//! and `[tmux]` comfort configuration diagrams.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    ConfigGuideSpec, EmblemMode, Language, PanePreview, PathCompletionItem, RichPickerRow, SashId,
    TextInput, Theme, WorkspaceSubviewLayout, build_art_box, build_rich_picker_list_items,
    parse_ansi_preview_line, pill_span, render_dock_zone_overlay, render_horizontal_divider_line,
    render_hydra_emblem_if_some, render_rich_picker_with_preview, render_sash_divider, single_line,
    split_lower_band_for_hydra, split_subview_bands, status_style, styled_key_hint_line,
    styled_key_hint_spans, truncate_to_width, wrap_description,
};

/// Clickable action pill rendered on the live terminal preview card chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAction {
    PrevPane,
    NextPane,
    Search,
    Copy,
    ExportLog,
    TogglePipe,
    TogglePip,
    SendEsc,
    Zoom,
    Rename,
    Close,
}

/// Layout geometry and mouse hit-boxes returned by [`super::render_pane_preview_card_interactive`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewCardLayout {
    pub area: Option<Rect>,
    pub content_rect: Option<Rect>,
    pub scrollbar_track_rect: Option<Rect>,
    pub scrollbar_thumb_rect: Option<Rect>,
    pub action_hits: Vec<(Rect, PreviewAction)>,
}

/// Clickable zero-memorization quick-launch pill displayed on the Home workspace action bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeQuickAction {
    Detach,
    Panes,
    Layout,
    Clients,
    Buffers,
    Snapshots,
}

/// Status flags for a tmux window/session cell rendered in Home Matrix or pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowStatusFlags {
    pub zoomed: bool,
    pub synchronized: bool,
    pub activity: bool,
    pub bell: bool,
    pub silence: bool,
    pub dead: bool,
    pub multi_clients: usize,
}

/// Format compact status badge spans (`⛶`, `⚡SYNC`, `●`/`🔔`/`💤`, `✕`, `👥 N`)
/// respecting both `--ascii` and `NO_COLOR`.
pub fn window_status_badge_spans(flags: WindowStatusFlags, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let push_gap = |v: &mut Vec<Span<'static>>| {
        if !v.is_empty() {
            v.push(Span::raw(" "));
        }
    };
    if flags.zoomed {
        push_gap(&mut spans);
        let label = if theme.ascii { "[Z]" } else { "⛶" };
        spans.push(Span::styled(
            label,
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        ));
    }
    if flags.synchronized {
        push_gap(&mut spans);
        let label = if theme.ascii {
            "[!SYNC]"
        } else if theme.language.is_zh() {
            "⚡同步"
        } else {
            "⚡SYNC"
        };
        spans.push(Span::styled(
            label,
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        ));
    }
    if flags.bell {
        push_gap(&mut spans);
        let label = if theme.ascii { "[!]" } else { "🔔" };
        spans.push(Span::styled(
            label,
            theme.style(theme.error).add_modifier(Modifier::BOLD),
        ));
    } else if flags.activity {
        push_gap(&mut spans);
        let label = if theme.ascii { "*" } else { "●" };
        spans.push(Span::styled(
            label,
            theme.style(theme.success).add_modifier(Modifier::BOLD),
        ));
    } else if flags.silence {
        push_gap(&mut spans);
        let label = if theme.ascii { "[z]" } else { "💤" };
        spans.push(Span::styled(label, theme.style(theme.muted)));
    }
    if flags.dead {
        push_gap(&mut spans);
        let label = if theme.ascii { "[X]" } else { "✕" };
        spans.push(Span::styled(
            label,
            theme.style(theme.error).add_modifier(Modifier::BOLD),
        ));
    }
    if flags.multi_clients > 1 {
        push_gap(&mut spans);
        let label = if theme.ascii {
            format!("{}c", flags.multi_clients)
        } else {
            format!("👥 {}", flags.multi_clients)
        };
        spans.push(Span::styled(label, theme.style(theme.secondary)));
    }
    spans
}

/// Clickable card or toolbar action returned in [`WorkspaceSubviewLayout::controller_hits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerAction {
    // LayoutStudio
    SelectLayoutPreset(usize),
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    ToggleZoom,
    ToggleSync,
    // Clients
    DetachOtherClients,
    DetachSelectedClient,
    KillServer,
    // Buffers
    PasteBuffer,
    CopyBufferToSystem,
    NewBuffer,
    DeleteBuffer,
    // Snapshots
    SaveSnapshot,
    RestoreSnapshot,
    DeleteSnapshot,
}

/// Built-in tmux layout preset showcased inside [`LayoutStudioView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPresetKind {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl LayoutPresetKind {
    /// All 5 built-in tmux layout presets in visual display order.
    pub const ALL: &'static [Self] = &[
        Self::EvenHorizontal,
        Self::EvenVertical,
        Self::MainHorizontal,
        Self::MainVertical,
        Self::Tiled,
    ];

    /// Localized preset title.
    pub const fn name(self, lang: Language) -> &'static str {
        match (lang, self) {
            (Language::Zh, Self::EvenHorizontal) => "水平均分",
            (Language::Zh, Self::EvenVertical) => "垂直均分",
            (Language::Zh, Self::MainHorizontal) => "上主下分",
            (Language::Zh, Self::MainVertical) => "左主右分",
            (Language::Zh, Self::Tiled) => "网格平铺",
            (Language::En, Self::EvenHorizontal) => "Even Horizontal",
            (Language::En, Self::EvenVertical) => "Even Vertical",
            (Language::En, Self::MainHorizontal) => "Main Horizontal",
            (Language::En, Self::MainVertical) => "Main Vertical",
            (Language::En, Self::Tiled) => "Tiled",
        }
    }

    /// Compact localized title for tight cards.
    const fn short_name(self, zh: bool) -> &'static str {
        match (zh, self) {
            (true, Self::EvenHorizontal) => "水平均分",
            (true, Self::EvenVertical) => "垂直均分",
            (true, Self::MainHorizontal) => "上主下分",
            (true, Self::MainVertical) => "左主右分",
            (true, Self::Tiled) => "网格平铺",
            (false, Self::EvenHorizontal) => "Even H",
            (false, Self::EvenVertical) => "Even V",
            (false, Self::MainHorizontal) => "Main H",
            (false, Self::MainVertical) => "Main V",
            (false, Self::Tiled) => "Tiled",
        }
    }

    /// Canonical tmux `select-layout` preset identifier.
    pub const fn tmux_name(self) -> &'static str {
        match self {
            Self::EvenHorizontal => "even-horizontal",
            Self::EvenVertical => "even-vertical",
            Self::MainHorizontal => "main-horizontal",
            Self::MainVertical => "main-vertical",
            Self::Tiled => "tiled",
        }
    }
}

/// View model for `SubviewContent::LayoutStudio` (`/layout`).
#[derive(Debug, Clone, Default)]
pub struct LayoutStudioView<'a> {
    pub target_label: &'a str,
    pub selected_preset: usize,
    pub pane_size: (u32, u32),
    pub pane_count: usize,
    pub resize_step: u16,
    pub zoomed: bool,
    pub synchronized: bool,
    pub status_note: Option<&'a str>,
    pub hovered_sash: Option<SashId>,
    pub dragging_sash: Option<SashId>,
}

/// Single attached tmux client row rendered inside [`ClientsView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRowView {
    pub name: String,
    pub session_name: String,
    pub width: u32,
    pub height: u32,
    pub tty: String,
    pub readonly: bool,
    pub is_current: bool,
}

/// View model for `SubviewContent::Clients` (`/clients`).
#[derive(Debug, Clone)]
pub struct ClientsView<'a> {
    pub clients: &'a [ClientRowView],
    pub selected: usize,
    pub size_mismatch_warning: bool,
    pub status_note: Option<&'a str>,
}

/// Single tmux paste buffer entry rendered inside [`BuffersView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferRowView {
    pub name: String,
    pub size: usize,
    pub sample: String,
}

/// View model for `SubviewContent::Buffers` (`/buffers`).
#[derive(Debug, Clone)]
pub struct BuffersView<'a> {
    pub buffers: &'a [BufferRowView],
    pub selected: usize,
    pub preview_lines: &'a [String],
    pub preview_scroll: usize,
    pub status_note: Option<&'a str>,
}

/// Saved workspace snapshot metadata row rendered inside [`SnapshotsView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRowView {
    pub name: String,
    pub created_label: String,
    pub session_count: usize,
    pub window_count: usize,
    pub pane_count: usize,
}

/// View model for `SubviewContent::Snapshots` (`/snapshot`).
#[derive(Debug, Clone)]
pub struct SnapshotsView<'a> {
    pub snapshots: &'a [SnapshotRowView],
    pub selected: usize,
    pub tree_preview: &'a [String],
    pub preview_scroll: usize,
    pub status_note: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// 1. Upgraded Interactive Preview Card Chrome & Search Highlighting
// ---------------------------------------------------------------------------

/// Highlight all case-insensitive occurrences of `query` across a parsed ANSI `Line`.
pub(super) fn highlight_preview_line(
    line: Line<'static>,
    query: &str,
    theme: Theme,
) -> Line<'static> {
    if query.is_empty() || line.spans.is_empty() {
        return line;
    }
    let query_chars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query_chars.is_empty() {
        return line;
    }

    let mut plain = String::new();
    for span in &line.spans {
        plain.push_str(&span.content);
    }
    if plain.is_empty() {
        return line;
    }

    // Build mapping from lowercased chars back to guaranteed UTF-8 byte ranges in `plain`.
    let mut lower_map: Vec<(char, usize, usize)> = Vec::with_capacity(plain.len());
    for (byte_start, ch) in plain.char_indices() {
        let byte_end = byte_start + ch.len_utf8();
        for lc in ch.to_lowercase() {
            lower_map.push((lc, byte_start, byte_end));
        }
    }
    let qlen = query_chars.len();
    if lower_map.len() < qlen {
        return line;
    }

    let mut match_ranges: Vec<(usize, usize)> = Vec::new();
    let mut idx = 0_usize;
    while idx + qlen <= lower_map.len() {
        let matched = query_chars
            .iter()
            .enumerate()
            .all(|(k, &qc)| lower_map[idx + k].0 == qc);
        if matched {
            let start = lower_map[idx].1;
            let end = lower_map[idx + qlen - 1].2;
            match_ranges.push((start, end));
            idx += qlen;
        } else {
            idx += 1;
        }
    }
    if match_ranges.is_empty() {
        return line;
    }

    let match_style = if theme.no_color {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.keycap_fg())
            .bg(theme.warning)
            .add_modifier(Modifier::BOLD)
    };

    let mut out_spans = Vec::with_capacity(line.spans.len() + match_ranges.len() * 2);
    let mut cursor_byte = 0_usize;
    let mut m_idx = 0_usize;

    for span in line.spans {
        let text = span.content.as_ref();
        let span_start = cursor_byte;
        let span_end = span_start + text.len();
        cursor_byte = span_end;

        while m_idx < match_ranges.len() && match_ranges[m_idx].1 <= span_start {
            m_idx += 1;
        }
        if m_idx >= match_ranges.len() || match_ranges[m_idx].0 >= span_end {
            out_spans.push(span);
            continue;
        }

        let mut local_pos = 0_usize;
        let mut scan = m_idx;
        while scan < match_ranges.len() {
            let (ms, me) = match_ranges[scan];
            if ms >= span_end {
                break;
            }
            let rel_s = ms.saturating_sub(span_start).min(text.len());
            let rel_e = me.saturating_sub(span_start).min(text.len());
            if rel_s > local_pos && text.is_char_boundary(local_pos) && text.is_char_boundary(rel_s)
            {
                out_spans.push(Span::styled(text[local_pos..rel_s].to_owned(), span.style));
            }
            if rel_e > rel_s && text.is_char_boundary(rel_s) && text.is_char_boundary(rel_e) {
                out_spans.push(Span::styled(text[rel_s..rel_e].to_owned(), match_style));
            }
            local_pos = local_pos.max(rel_e);
            if me <= span_end {
                scan += 1;
            } else {
                break;
            }
        }
        if local_pos < text.len() && text.is_char_boundary(local_pos) {
            out_spans.push(Span::styled(text[local_pos..].to_owned(), span.style));
        }
        m_idx = scan;
    }

    Line::from(out_spans)
}

/// Apply mouse drag-selection highlighting to a rendered preview `Line` using 0-indexed
/// `(visible_row, display_col)` coordinates relative to `content_rect`.
fn apply_selection_highlight(
    line: Line<'static>,
    visible_row: usize,
    selection: ((usize, usize), (usize, usize)),
    theme: Theme,
) -> Line<'static> {
    let ((r0, c0), (r1, c1)) = if selection.0 <= selection.1 {
        (selection.0, selection.1)
    } else {
        (selection.1, selection.0)
    };
    if visible_row < r0 || visible_row > r1 || line.spans.is_empty() {
        return line;
    }
    let (sel_start, sel_end) = if r0 == r1 {
        (c0, c1)
    } else if visible_row == r0 {
        (c0, usize::MAX)
    } else if visible_row == r1 {
        (0, c1)
    } else {
        (0, usize::MAX)
    };

    let sel_style_for = |base: Style| -> Style {
        if theme.no_color {
            base.add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .fg(theme.keycap_fg())
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        }
    };

    let mut out_spans = Vec::with_capacity(line.spans.len() + 2);
    let mut col = 0_usize;
    for span in line.spans {
        let base_style = span.style;
        let text = span.content.as_ref();
        let mut run = String::new();
        let mut run_selected: Option<bool> = None;

        for ch in text.chars() {
            let cw = ch.width().unwrap_or(0);
            let hit_end = col.saturating_add(cw.saturating_sub(1));
            let is_sel = hit_end >= sel_start && col <= sel_end;
            if run_selected.is_some_and(|prev| prev != is_sel) && !run.is_empty() {
                let st = if run_selected == Some(true) {
                    sel_style_for(base_style)
                } else {
                    base_style
                };
                out_spans.push(Span::styled(std::mem::take(&mut run), st));
            }
            run_selected = Some(is_sel);
            run.push(ch);
            col = col.saturating_add(cw);
        }
        if !run.is_empty() {
            let st = if run_selected == Some(true) {
                sel_style_for(base_style)
            } else {
                base_style
            };
            out_spans.push(Span::styled(run, st));
        }
    }
    Line::from(out_spans)
}

/// Render a VS Code-style 1-column scrollbar on the right edge of a preview card with
/// proportional draggable thumb, amber search-match ticks, and red error ticks.
#[allow(clippy::too_many_arguments)]
fn render_preview_scrollbar(
    frame: &mut Frame<'_>,
    sb_area: Rect,
    total: usize,
    visible_h: usize,
    actual_scroll: usize,
    hovered: bool,
    dragging: bool,
    search_match_lines: &[usize],
    error_lines: &[usize],
    theme: Theme,
) -> (Rect, Rect) {
    let h = usize::from(sb_area.height);
    if sb_area.width == 0 || h == 0 {
        return (sb_area, sb_area);
    }

    let thumb_h = if total <= visible_h {
        h
    } else {
        ((h * visible_h) / total.max(1)).clamp(1, h)
    };
    let max_scroll = total.saturating_sub(visible_h);
    let travel = h.saturating_sub(thumb_h);
    let from_top = max_scroll.saturating_sub(actual_scroll);
    let thumb_top = (from_top * travel + max_scroll / 2)
        .checked_div(max_scroll)
        .unwrap_or(0)
        .min(travel);

    let mut match_ticks = vec![false; h];
    let mut error_ticks = vec![false; h];
    let denom = total.saturating_sub(1);
    for &idx in search_match_lines {
        let row = if denom == 0 || h <= 1 {
            0
        } else {
            (idx.min(denom) * (h - 1)) / denom
        };
        if let Some(slot) = match_ticks.get_mut(row) {
            *slot = true;
        }
    }
    for &idx in error_lines {
        let row = if denom == 0 || h <= 1 {
            0
        } else {
            (idx.min(denom) * (h - 1)) / denom
        };
        if let Some(slot) = error_ticks.get_mut(row) {
            *slot = true;
        }
    }

    let active = hovered || dragging;
    let track_ch = if theme.ascii { "|" } else { "│" };
    let thumb_ch = if theme.ascii { "#" } else { "█" };
    let match_ch = if theme.ascii { "=" } else { "◈" };
    let err_ch = if theme.ascii { "!" } else { "●" };

    let track_style = theme.style(theme.muted);
    let thumb_style = if theme.no_color {
        if active {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        }
    } else if active {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.secondary)
    };
    let match_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.warning).add_modifier(Modifier::BOLD)
    };
    let err_style = if theme.no_color {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        theme.style(theme.error).add_modifier(Modifier::BOLD)
    };

    let mut rows = Vec::with_capacity(h);
    for r in 0..h {
        let in_thumb = r >= thumb_top && r < thumb_top + thumb_h;
        let span = if error_ticks[r] {
            Span::styled(err_ch, err_style)
        } else if match_ticks[r] {
            Span::styled(match_ch, match_style)
        } else if in_thumb {
            Span::styled(thumb_ch, thumb_style)
        } else {
            Span::styled(track_ch, track_style)
        };
        rows.push(Line::from(span));
    }
    frame.render_widget(Paragraph::new(rows), sb_area);

    let thumb_rect = Rect {
        x: sb_area.x,
        y: sb_area
            .y
            .saturating_add(u16::try_from(thumb_top).unwrap_or(0)),
        width: 1,
        height: u16::try_from(thumb_h).unwrap_or(1),
    };
    (sb_area, thumb_rect)
}

pub(super) fn render_pane_preview_card_inner(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: PanePreview<'_>,
    theme: Theme,
) -> PreviewCardLayout {
    if area.width < 6 || area.height < 3 {
        return PreviewCardLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let muted = theme.style(theme.muted);
    let adapted_title = theme.adapt(preview.title);
    let has_leading_icon = preview.title.starts_with('📺')
        || preview.title.starts_with('◈')
        || preview.title.starts_with("[]");
    let title_label = if has_leading_icon {
        format!(" {adapted_title}")
    } else if theme.ascii {
        format!(" [] {adapted_title}")
    } else {
        format!(" ◈ {adapted_title}")
    };
    let mut title_spans = vec![Span::styled(
        title_label,
        theme
            .style(if preview.pip_active {
                theme.success
            } else {
                theme.accent
            })
            .add_modifier(Modifier::BOLD),
    )];

    // Status badges for live PIP / zoomed / synchronized / piping states.
    let mut status_badges: Vec<(String, Color)> = Vec::new();
    if preview.pip_active {
        let lbl = if theme.ascii {
            " [*LIVE INPUT | Esc exit] "
        } else if zh {
            " ● LIVE INPUT · Esc 退出直控 "
        } else {
            " ● LIVE INPUT · Esc exit "
        };
        status_badges.push((lbl.to_owned(), theme.success));
    }
    if preview.zoomed {
        let lbl = if theme.ascii {
            " [Z] "
        } else if zh {
            " ⛶ 缩放 "
        } else {
            " ⛶ ZOOM "
        };
        status_badges.push((lbl.to_owned(), theme.accent));
    }
    if preview.synchronized {
        let lbl = if theme.ascii {
            " [!SYNC] "
        } else if zh {
            " ⚡同步 "
        } else {
            " ⚡SYNC "
        };
        status_badges.push((lbl.to_owned(), theme.warning));
    }
    if preview.piping {
        let lbl = if theme.ascii {
            " [*PIPE] "
        } else if zh {
            " ● 录制 "
        } else {
            " ● PIPE "
        };
        status_badges.push((lbl.to_owned(), theme.success));
    }

    let inner_total_h = usize::from(area.height.saturating_sub(2));
    let show_search_bar = (preview.search_active
        || preview.search_query.is_some_and(|q| !q.is_empty()))
        && inner_total_h >= 2;
    let content_h = inner_total_h.saturating_sub(usize::from(show_search_bar));
    let total = preview.lines.len();
    let max_scroll = total.saturating_sub(content_h);
    let actual_scroll = preview.scroll.min(max_scroll);

    // Top-right window controls `[⌨][⛶][✎][✕]` when actions chrome is active and wide enough.
    let top_ctrls = [
        (
            PreviewAction::TogglePip,
            if theme.ascii { "[I]" } else { "[⌨]" },
            theme.success,
            preview.pip_active,
        ),
        (
            PreviewAction::Zoom,
            if theme.ascii { "[Z]" } else { "[⛶]" },
            theme.accent,
            preview.zoomed,
        ),
        (
            PreviewAction::Rename,
            if theme.ascii { "[R]" } else { "[✎]" },
            theme.warning,
            false,
        ),
        (
            PreviewAction::Close,
            if theme.ascii { "[X]" } else { "[✕]" },
            theme.error,
            false,
        ),
    ];
    let top_ctrl_w: usize = top_ctrls.iter().map(|(_, s, _, _)| s.width()).sum();
    let show_top_controls = preview.show_actions && area.width >= 36;
    let ctrl_reserve = if show_top_controls { top_ctrl_w + 1 } else { 0 };

    // Reserve right-side space for scroll badge when needed.
    let scroll_spec = (content_h > 0 && max_scroll > 0).then(|| {
        if actual_scroll > 0 {
            let style = theme.style(theme.accent).add_modifier(Modifier::BOLD);
            if theme.ascii {
                (
                    format!(" [+{actual_scroll}/{max_scroll} PgUp/PgDn] "),
                    format!(" [+{actual_scroll}] "),
                    style,
                )
            } else {
                (
                    format!(" ↑ +{actual_scroll}/{max_scroll} · PgUp/PgDn "),
                    format!(" ↑ +{actual_scroll} "),
                    style,
                )
            }
        } else {
            let style = theme.style(theme.muted);
            if theme.ascii {
                (
                    format!(" [{total}L PgUp/Wheel] "),
                    format!(" [{total}L] "),
                    style,
                )
            } else {
                let lines_unit = theme.tr("L", "行");
                let wheel = theme.tr("Wheel", "滚轮");
                (
                    format!(" ↕ {total}{lines_unit} · PgUp/{wheel} "),
                    format!(" ↕ {total}{lines_unit} "),
                    style,
                )
            }
        }
    });

    let top_avail = usize::from(area.width.saturating_sub(2)).saturating_sub(ctrl_reserve);
    let badge_reserve: usize = status_badges.iter().map(|(s, _)| s.width() + 1).sum();
    if !preview.subtitle.is_empty() {
        let sep = if theme.ascii { " | " } else { " · " };
        let sub = format!("{sep}{}", theme.adapt(preview.subtitle));
        let used: usize = title_spans.iter().map(Span::width).sum();
        if used + sub.width() + badge_reserve < top_avail {
            title_spans.push(Span::styled(sub, muted));
        }
    }
    let bg = theme.pill_bg();
    for (badge_text, badge_color) in status_badges {
        let used: usize = title_spans.iter().map(Span::width).sum();
        if used + 1 + badge_text.width() < top_avail {
            title_spans.push(Span::raw(" "));
            title_spans.push(pill_span(badge_text, badge_color, bg, theme, true));
        }
    }
    title_spans.push(Span::raw(" "));
    let left_used: usize = title_spans.iter().map(Span::width).sum();

    let border_color = if preview.pip_active {
        theme.success
    } else if preview.synchronized {
        theme.warning
    } else if preview.search_active {
        theme.accent
    } else {
        theme.secondary
    };
    let border_style = if preview.pip_active {
        theme.style(border_color).add_modifier(Modifier::BOLD)
    } else {
        theme.style(border_color)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else if preview.pip_active {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(Line::from(title_spans));

    if !show_top_controls && let Some((full, compact, style)) = &scroll_spec {
        let avail = usize::from(area.width);
        if left_used + full.width() + 2 <= avail {
            block = block.title_top(styled_key_hint_line(full, *style, theme).right_aligned());
        } else if left_used + compact.width() + 2 <= avail {
            block = block.title_top(styled_key_hint_line(compact, *style, theme).right_aligned());
        }
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut action_hits = Vec::new();

    // Paint top-right window controls `[⌨][⛶][✎][✕]` + adjacent scroll badge with exact hit rects.
    if show_top_controls {
        let ctrl_w_u16 = u16::try_from(top_ctrl_w).unwrap_or(0);
        let ctrl_start_x = area.right().saturating_sub(1).saturating_sub(ctrl_w_u16);

        if let Some((full, compact, style)) = &scroll_spec {
            let left_end = area
                .x
                .saturating_add(1)
                .saturating_add(u16::try_from(left_used).unwrap_or(0));
            let space_before_ctrls = usize::from(ctrl_start_x.saturating_sub(left_end));
            let chosen = if full.width() <= space_before_ctrls {
                Some(full.as_str())
            } else if compact.width() <= space_before_ctrls {
                Some(compact.as_str())
            } else {
                None
            };
            if let Some(badge) = chosen {
                let bw = u16::try_from(badge.width()).unwrap_or(0);
                frame.render_widget(
                    Paragraph::new(styled_key_hint_line(badge, *style, theme)),
                    Rect {
                        x: ctrl_start_x.saturating_sub(bw),
                        y: area.y,
                        width: bw,
                        height: 1,
                    },
                );
            }
        }

        let mut cx = ctrl_start_x;
        let mut ctrl_spans = Vec::with_capacity(top_ctrls.len());
        for (act, lbl, color, active) in top_ctrls {
            let hovered = preview.hovered_action == Some(act);
            let w = u16::try_from(lbl.width()).unwrap_or(0);
            let style = if theme.no_color {
                if hovered || active {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::default()
                }
            } else if hovered {
                Style::default()
                    .fg(theme.keycap_fg())
                    .bg(color)
                    .add_modifier(Modifier::BOLD)
            } else if active {
                theme.style(color).add_modifier(Modifier::BOLD)
            } else {
                theme.style(theme.muted)
            };
            ctrl_spans.push(Span::styled(lbl, style));
            action_hits.push((
                Rect {
                    x: cx,
                    y: area.y,
                    width: w,
                    height: 1,
                },
                act,
            ));
            cx = cx.saturating_add(w);
        }
        frame.render_widget(
            Paragraph::new(Line::from(ctrl_spans)),
            Rect {
                x: ctrl_start_x,
                y: area.y,
                width: ctrl_w_u16,
                height: 1,
            },
        );
    }

    // Helper for hover-aware bottom-border action pills.
    let render_pill_span =
        |act: PreviewAction, label: &str, color: Color, bold: bool| -> Span<'static> {
            let hovered = preview.hovered_action == Some(act);
            if theme.no_color {
                if hovered || bold {
                    Span::styled(
                        theme.adapt(label),
                        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                    )
                } else {
                    Span::raw(theme.adapt(label))
                }
            } else if hovered {
                Span::styled(
                    theme.adapt(label),
                    Style::default()
                        .fg(theme.keycap_fg())
                        .bg(color)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                pill_span(label, color, bg, theme, bold)
            }
        };

    // Render interactive bottom-border controls with exact 1-row mouse hit-boxes.
    let bottom_y = area.bottom().saturating_sub(1);
    let border_left = area.x.saturating_add(2);
    let border_right = area.right().saturating_sub(2);
    let mut left_cursor = border_left;

    if let Some((cur, total_panes)) = preview.pane_index
        && total_panes > 1
        && border_right > border_left + 8
    {
        let prev_lbl = if theme.ascii { "[<]" } else { "[◀]" };
        let next_lbl = if theme.ascii { "[>]" } else { "[▶]" };
        let mid_full = if zh {
            format!(" 窗格 {cur}/{total_panes} ")
        } else {
            format!(" Pane {cur}/{total_panes} ")
        };
        let mid_short = format!(" {cur}/{total_panes} ");
        let avail = usize::from(border_right.saturating_sub(left_cursor));
        let mid_lbl = if prev_lbl.width() + mid_full.width() + next_lbl.width() + 2 <= avail {
            mid_full
        } else {
            mid_short
        };
        let total_w = prev_lbl.width() + mid_lbl.width() + next_lbl.width();
        if total_w <= avail {
            let prev_w = u16::try_from(prev_lbl.width()).unwrap_or(0);
            let mid_w = u16::try_from(mid_lbl.width()).unwrap_or(0);
            let next_w = u16::try_from(next_lbl.width()).unwrap_or(0);
            let prev_rect = Rect {
                x: left_cursor,
                y: bottom_y,
                width: prev_w,
                height: 1,
            };
            let next_rect = Rect {
                x: left_cursor.saturating_add(prev_w).saturating_add(mid_w),
                y: bottom_y,
                width: next_w,
                height: 1,
            };
            let line = Line::from(vec![
                render_pill_span(PreviewAction::PrevPane, prev_lbl, theme.accent, true),
                Span::styled(
                    mid_lbl,
                    theme.style(theme.secondary).add_modifier(Modifier::BOLD),
                ),
                render_pill_span(PreviewAction::NextPane, next_lbl, theme.accent, true),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect {
                    x: left_cursor,
                    y: bottom_y,
                    width: u16::try_from(total_w).unwrap_or(0),
                    height: 1,
                },
            );
            action_hits.push((prev_rect, PreviewAction::PrevPane));
            action_hits.push((next_rect, PreviewAction::NextPane));
            left_cursor = next_rect.right().saturating_add(1);
        }
    }

    let send_esc_lbl = if zh { "[发送 Esc]" } else { "[Send Esc]" };
    if preview.show_actions && border_right > left_cursor.saturating_add(10) {
        let right_avail = usize::from(border_right.saturating_sub(left_cursor));
        let search_full = if zh { "[f 搜索]" } else { "[f Search]" };
        let copy_full = if zh { "[y 复制]" } else { "[y Copy]" };
        let log_full = if zh { "[日志]" } else { "[Log]" };
        let pipe_full = if preview.piping {
            if theme.ascii {
                "[*Pipe]"
            } else if zh {
                "[● 录制]"
            } else {
                "[● Pipe]"
            }
        } else if zh {
            "[录制]"
        } else {
            "[Pipe]"
        };
        let mut pills_full = Vec::with_capacity(5);
        let mut pills_compact = Vec::with_capacity(4);
        if preview.pip_active {
            pills_full.push((PreviewAction::SendEsc, send_esc_lbl, theme.warning, true));
            pills_compact.push((PreviewAction::SendEsc, send_esc_lbl, theme.warning, true));
        }
        pills_full.extend_from_slice(&[
            (
                PreviewAction::Search,
                search_full,
                if preview.search_active {
                    theme.accent
                } else {
                    theme.secondary
                },
                preview.search_active,
            ),
            (PreviewAction::Copy, copy_full, theme.secondary, false),
            (PreviewAction::ExportLog, log_full, theme.secondary, false),
            (
                PreviewAction::TogglePipe,
                pipe_full,
                if preview.piping {
                    theme.success
                } else {
                    theme.muted
                },
                preview.piping,
            ),
        ]);
        pills_compact.extend_from_slice(&[
            (
                PreviewAction::Search,
                "[f]",
                if preview.search_active {
                    theme.accent
                } else {
                    theme.secondary
                },
                preview.search_active,
            ),
            (PreviewAction::Copy, "[y]", theme.secondary, false),
            (PreviewAction::ExportLog, log_full, theme.secondary, false),
        ]);

        let full_w: usize = pills_full
            .iter()
            .map(|(_, s, _, _)| s.width())
            .sum::<usize>()
            + pills_full.len().saturating_sub(1);
        let compact_w: usize = pills_compact
            .iter()
            .map(|(_, s, _, _)| s.width())
            .sum::<usize>()
            + pills_compact.len().saturating_sub(1);

        let chosen: Vec<(PreviewAction, &str, Color, bool)> = if full_w <= right_avail {
            pills_full
        } else if compact_w <= right_avail {
            pills_compact
        } else if preview.pip_active && send_esc_lbl.width() <= right_avail {
            vec![(PreviewAction::SendEsc, send_esc_lbl, theme.warning, true)]
        } else {
            Vec::new()
        };

        if !chosen.is_empty() {
            let used_w: usize = chosen.iter().map(|(_, s, _, _)| s.width()).sum::<usize>()
                + chosen.len().saturating_sub(1);
            let start_x = border_right.saturating_sub(u16::try_from(used_w).unwrap_or(0));
            let mut spans = Vec::new();
            let mut x = start_x;
            for (idx, (act, label, color, bold)) in chosen.into_iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::raw(" "));
                    x = x.saturating_add(1);
                }
                let w = u16::try_from(label.width()).unwrap_or(0);
                spans.push(render_pill_span(act, label, color, bold));
                action_hits.push((
                    Rect {
                        x,
                        y: bottom_y,
                        width: w,
                        height: 1,
                    },
                    act,
                ));
                x = x.saturating_add(w);
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    x: start_x,
                    y: bottom_y,
                    width: u16::try_from(used_w).unwrap_or(0),
                    height: 1,
                },
            );
        }
    } else if preview.pip_active
        && usize::from(border_right.saturating_sub(left_cursor)) >= send_esc_lbl.width()
    {
        let w = u16::try_from(send_esc_lbl.width()).unwrap_or(0);
        let start_x = border_right.saturating_sub(w);
        let rect = Rect {
            x: start_x,
            y: bottom_y,
            width: w,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(render_pill_span(
                PreviewAction::SendEsc,
                send_esc_lbl,
                theme.warning,
                true,
            ))),
            rect,
        );
        action_hits.push((rect, PreviewAction::SendEsc));
    }

    if inner.width == 0 || inner.height == 0 {
        return PreviewCardLayout {
            area: Some(area),
            content_rect: None,
            scrollbar_track_rect: None,
            scrollbar_thumb_rect: None,
            action_hits,
        };
    }

    let (lines_area, search_area) = if show_search_bar {
        let [top, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
        (top, Some(bar))
    } else {
        (inner, None)
    };

    let show_scrollbar = lines_area.width >= 12
        && (total > content_h
            || !preview.search_match_lines.is_empty()
            || !preview.error_lines.is_empty());
    let (text_area, sb_area) = if show_scrollbar {
        let [t, s] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).areas(lines_area);
        (t, Some(s))
    } else {
        (lines_area, None)
    };

    if preview.lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(
                    if zh {
                        " (空终端缓冲区)"
                    } else {
                        " (empty terminal buffer)"
                    },
                    usize::from(text_area.width),
                ),
                muted,
            ))),
            text_area,
        );
    } else {
        let max_rows = usize::from(text_area.height);
        let end = total.saturating_sub(actual_scroll);
        let start = end.saturating_sub(max_rows);
        let max_width = usize::from(text_area.width);
        let query = preview.search_query.unwrap_or("");
        let lines: Vec<Line<'static>> = preview.lines[start..end]
            .iter()
            .enumerate()
            .map(|(vis_row, raw)| {
                let parsed = parse_ansi_preview_line(raw, max_width, theme);
                let searched = if query.is_empty() {
                    parsed
                } else {
                    highlight_preview_line(parsed, query, theme)
                };
                if let Some(sel) = preview.selection_range {
                    apply_selection_highlight(searched, vis_row, sel, theme)
                } else {
                    searched
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), text_area);
    }

    let (scrollbar_track_rect, scrollbar_thumb_rect) = if let Some(sb_rect) = sb_area {
        let (track, thumb) = render_preview_scrollbar(
            frame,
            sb_rect,
            total,
            content_h,
            actual_scroll,
            preview.scrollbar_hovered,
            preview.scrollbar_dragging,
            preview.search_match_lines,
            preview.error_lines,
            theme,
        );
        (Some(track), Some(thumb))
    } else {
        (None, None)
    };

    if let Some(bar_rect) = search_area {
        render_preview_search_bar(frame, bar_rect, preview, theme);
    }

    if let Some(zone) = preview.dock_highlight {
        render_dock_zone_overlay(frame, inner, zone, theme);
    }

    PreviewCardLayout {
        area: Some(area),
        content_rect: Some(text_area),
        scrollbar_track_rect,
        scrollbar_thumb_rect,
        action_hits,
    }
}

fn render_preview_search_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    preview: PanePreview<'_>,
    theme: Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let query = preview.search_query.unwrap_or("");
    let prefix = if theme.ascii {
        "/? Search: "
    } else if zh {
        "🔍 搜索: "
    } else {
        "🔍 Search: "
    };
    let mut spans = vec![Span::styled(
        prefix,
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    let match_badge = if query.is_empty() {
        None
    } else if preview.match_count > 0 {
        Some((
            format!(" {}/{} ", preview.current_match.max(1), preview.match_count),
            theme.success,
        ))
    } else {
        Some((
            if zh { " 无匹配 " } else { " 0 matches " }.to_owned(),
            theme.warning,
        ))
    };
    let right_hint = if theme.ascii {
        " n/N next | Esc close"
    } else if zh {
        " n/N 切换 · Esc 关闭"
    } else {
        " n/N next · Esc close"
    };
    let total_w = usize::from(area.width);
    let badge_w = match_badge.as_ref().map_or(0, |(s, _)| s.width() + 1);
    let caret_w = usize::from(preview.search_active);
    let query_budget = total_w.saturating_sub(prefix.width() + badge_w + caret_w + 1);
    let shown_query = truncate_to_width(&single_line(&theme.adapt(query)), query_budget);
    spans.push(Span::styled(
        shown_query,
        theme.style(theme.text).add_modifier(Modifier::BOLD),
    ));
    if preview.search_active {
        spans.push(Span::styled(" ", theme.selected_style()));
    }
    if let Some((badge, color)) = match_badge {
        let used: usize = spans.iter().map(Span::width).sum();
        if used + 1 + badge.width() <= total_w {
            spans.push(Span::raw(" "));
            spans.push(pill_span(badge, color, theme.pill_bg(), theme, true));
        }
    }
    let used: usize = spans.iter().map(Span::width).sum();
    if used + right_hint.width() <= total_w {
        let gap = total_w.saturating_sub(used + right_hint.width());
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
        }
        spans.extend(styled_key_hint_spans(
            right_hint,
            theme.style(theme.muted),
            theme,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// 2. Shared Toolbar & Size Helpers
// ---------------------------------------------------------------------------

struct ToolbarPill<'a> {
    action: ControllerAction,
    full: &'a str,
    short: &'a str,
    color: Color,
    enabled: bool,
    emphasized: bool,
}

fn render_controller_toolbar(
    frame: &mut Frame<'_>,
    area: Rect,
    pills: &[ToolbarPill<'_>],
    theme: Theme,
) -> Vec<(Rect, ControllerAction)> {
    if area.width == 0 || area.height == 0 || pills.is_empty() {
        return Vec::new();
    }
    let total_w = usize::from(area.width);
    let full_w: usize =
        pills.iter().map(|p| p.full.width()).sum::<usize>() + pills.len().saturating_sub(1);
    let use_short = full_w > total_w;
    let bg = theme.pill_bg();
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut col = 0_usize;

    for p in pills {
        let label = if use_short { p.short } else { p.full };
        let w = label.width();
        let gap = usize::from(col > 0);
        if col + gap + w > total_w {
            break;
        }
        if gap > 0 {
            spans.push(Span::raw(" "));
            col += 1;
        }
        let span = if !p.enabled {
            Span::styled(theme.adapt(label), theme.style(theme.muted))
        } else if p.emphasized && theme.no_color {
            Span::styled(
                theme.adapt(label),
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            )
        } else {
            pill_span(label, p.color, bg, theme, p.emphasized)
        };
        spans.push(span);
        if p.enabled {
            hits.push((
                Rect {
                    x: area.x.saturating_add(u16::try_from(col).unwrap_or(0)),
                    y: area.y,
                    width: u16::try_from(w).unwrap_or(0),
                    height: 1,
                },
                p.action,
            ));
        }
        col += w;
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    hits
}

fn format_byte_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn detect_client_dimension_mismatch(clients: &[ClientRowView]) -> bool {
    for (i, a) in clients.iter().enumerate() {
        for b in clients.iter().skip(i + 1) {
            if a.session_name == b.session_name && (a.width != b.width || a.height != b.height) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 3. Layout Studio Subview (`/layout`)
// ---------------------------------------------------------------------------

fn preset_diagram_lines(preset: LayoutPresetKind, ascii: bool) -> [&'static str; 5] {
    if ascii {
        match preset {
            LayoutPresetKind::EvenHorizontal => [
                "+---+---+---+",
                "| 1 | 2 | 3 |",
                "|   |   |   |",
                "|   |   |   |",
                "+---+---+---+",
            ],
            LayoutPresetKind::EvenVertical => [
                "+-----------+",
                "|     1     |",
                "+-----------+",
                "|  2  /  3  |",
                "+-----------+",
            ],
            LayoutPresetKind::MainHorizontal => [
                "+-----------+",
                "|   1 MAIN  |",
                "+-----+-----+",
                "|  2  |  3  |",
                "+-----+-----+",
            ],
            LayoutPresetKind::MainVertical => [
                "+------+----+",
                "|  1   | 2  |",
                "| MAIN +----+",
                "|      | 3  |",
                "+------+----+",
            ],
            LayoutPresetKind::Tiled => [
                "+-----+-----+",
                "|  1  |  2  |",
                "+-----+-----+",
                "|  3  |  4  |",
                "+-----+-----+",
            ],
        }
    } else {
        match preset {
            LayoutPresetKind::EvenHorizontal => [
                "┌───┬───┬───┐",
                "│ 1 │ 2 │ 3 │",
                "│   │   │   │",
                "│   │   │   │",
                "└───┴───┴───┘",
            ],
            LayoutPresetKind::EvenVertical => [
                "┌───────────┐",
                "│     1     │",
                "├───────────┤",
                "│  2  ─  3  │",
                "└───────────┘",
            ],
            LayoutPresetKind::MainHorizontal => [
                "┌───────────┐",
                "│   1 MAIN  │",
                "├─────┬─────┤",
                "│  2  │  3  │",
                "└─────┴─────┘",
            ],
            LayoutPresetKind::MainVertical => [
                "┌──────┬────┐",
                "│  1   │ 2  │",
                "│ MAIN ├────┤",
                "│      │ 3  │",
                "└──────┴────┘",
            ],
            LayoutPresetKind::Tiled => [
                "┌─────┬─────┐",
                "│  1  │  2  │",
                "├─────┼─────┤",
                "│  3  │  4  │",
                "└─────┴─────┘",
            ],
        }
    }
}

fn render_layout_preset_cards(
    frame: &mut Frame<'_>,
    area: Rect,
    selected: usize,
    theme: Theme,
) -> Vec<(Rect, ControllerAction)> {
    let mut hits = Vec::new();
    if area.width < 12 || area.height < 3 {
        return hits;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let clamped = selected.min(LayoutPresetKind::ALL.len().saturating_sub(1));

    // Wide horizontal 5-card strip when width >= 80 and height >= 7.
    if area.width >= 80 && area.height >= 7 {
        let constraints = [Constraint::Ratio(1, 5); 5];
        let cols = Layout::horizontal(constraints).areas::<5>(area);
        for (idx, (&preset, &card_rect)) in
            LayoutPresetKind::ALL.iter().zip(cols.iter()).enumerate()
        {
            let is_sel = idx == clamped;
            let border_style = if is_sel {
                theme.style(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                theme.style(theme.muted)
            };
            let title_label = format!(
                " {} {} ",
                idx + 1,
                if card_rect.width >= 19 {
                    if zh {
                        preset.name(Language::Zh)
                    } else {
                        preset.name(Language::En)
                    }
                } else {
                    preset.short_name(zh)
                }
            );
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(if theme.ascii {
                    BorderType::Plain
                } else if is_sel {
                    BorderType::Thick
                } else {
                    BorderType::Rounded
                })
                .border_style(border_style)
                .title(Line::from(Span::styled(
                    truncate_to_width(&title_label, usize::from(card_rect.width.saturating_sub(2))),
                    if is_sel {
                        theme.style(theme.accent).add_modifier(Modifier::BOLD)
                    } else {
                        theme.style(theme.text)
                    },
                )));
            let inner = block.inner(card_rect);
            frame.render_widget(block, card_rect);

            let diagram = preset_diagram_lines(preset, theme.ascii);
            let art_style = if is_sel {
                theme.style(theme.secondary).add_modifier(Modifier::BOLD)
            } else {
                theme.style(theme.muted)
            };
            let pad_left = usize::from(inner.width).saturating_sub(13) / 2;
            let mut lines: Vec<Line<'static>> = diagram
                .iter()
                .take(usize::from(inner.height))
                .map(|row| {
                    let padded = format!("{}{row}", " ".repeat(pad_left));
                    Line::from(Span::styled(
                        truncate_to_width(&padded, usize::from(inner.width)),
                        art_style,
                    ))
                })
                .collect();
            if usize::from(inner.height) > lines.len() {
                let tag = preset.tmux_name();
                let tpad = usize::from(inner.width).saturating_sub(tag.width()) / 2;
                lines.push(Line::from(Span::styled(
                    truncate_to_width(
                        &format!("{}{tag}", " ".repeat(tpad)),
                        usize::from(inner.width),
                    ),
                    theme.style(theme.muted),
                )));
            }
            frame.render_widget(Paragraph::new(lines), inner);
            hits.push((card_rect, ControllerAction::SelectLayoutPreset(idx)));
        }
        return hits;
    }

    // Medium / compact selectable list of the 5 presets.
    for (idx, &preset) in LayoutPresetKind::ALL.iter().enumerate() {
        if idx >= usize::from(area.height) {
            break;
        }
        let is_sel = idx == clamped;
        let row_rect = Rect {
            x: area.x,
            y: area.y.saturating_add(u16::try_from(idx).unwrap_or(0)),
            width: area.width,
            height: 1,
        };
        let marker = if is_sel { theme.marker() } else { " " };
        let name = if zh {
            preset.name(Language::Zh)
        } else {
            preset.name(Language::En)
        };
        let line_text = format!("{marker} [{}] {name} ({})", idx + 1, preset.tmux_name());
        let style = if is_sel {
            theme.selected_style()
        } else {
            theme.style(theme.text)
        };
        frame.render_widget(
            Paragraph::new(truncate_to_width(&line_text, usize::from(row_rect.width))).style(style),
            row_rect,
        );
        hits.push((row_rect, ControllerAction::SelectLayoutPreset(idx)));
    }
    hits
}

pub(super) fn render_subview_layout_studio(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    view: &LayoutStudioView<'_>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 13 {
        return render_compact_layout_studio(frame, card_inner, view, theme);
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 8, true);

    let [head_area, cards_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(upper_band);
    let heading = if zh {
        format!(
            "▦ 布局与窗格尺寸工作台 · {} ({} 窗格)",
            single_line(view.target_label),
            view.pane_count
        )
    } else if theme.ascii {
        format!(
            "[] Layout & Resize Studio | {} ({} panes)",
            single_line(view.target_label),
            view.pane_count
        )
    } else {
        format!(
            "▦ Layout & Resize Studio · {} ({} panes)",
            single_line(view.target_label),
            view.pane_count
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(&heading, usize::from(head_area.width)),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        ))),
        head_area,
    );

    let mut controller_hits =
        render_layout_preset_cards(frame, cards_area, view.selected_preset, theme);

    let mut sash_hits = Vec::new();
    let total_band_h = upper_band.height.saturating_add(lower_band.height).max(1);
    let upper_ratio = ((u32::from(upper_band.height) * 100 + u32::from(total_band_h) / 2)
        / u32::from(total_band_h)) as u16;
    let h_sash = SashId::LayoutResize(false);
    render_sash_divider(
        frame,
        rule_area,
        false,
        upper_ratio,
        view.hovered_sash == Some(h_sash),
        view.dragging_sash == Some(h_sash),
        theme,
    );
    sash_hits.push((rule_area, h_sash));

    let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
    if let Some(hbox) = hydra_box
        && hbox.x > left_band.right()
    {
        let v_sash_rect = Rect {
            x: left_band.right(),
            y: lower_band.y,
            width: 1,
            height: lower_band.height,
        };
        let v_sash = SashId::LayoutResize(true);
        let total_w = left_band.width.saturating_add(hbox.width).max(1);
        let left_ratio = ((u32::from(left_band.width) * 100 + u32::from(total_w) / 2)
            / u32::from(total_w)) as u16;
        if view.hovered_sash == Some(v_sash) || view.dragging_sash == Some(v_sash) {
            render_sash_divider(
                frame,
                v_sash_rect,
                true,
                left_ratio,
                view.hovered_sash == Some(v_sash),
                view.dragging_sash == Some(v_sash),
                theme,
            );
        }
        sash_hits.push((v_sash_rect, v_sash));
    }

    // Lower-left: Live Pane Resize HUD.
    let [hud_head, dim_area, resize_row, toggle_row, note_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(left_band);

    let hud_title = if theme.ascii {
        "# Live Pane Resize HUD"
    } else if zh {
        "◈ 实时窗格尺寸调节 HUD"
    } else {
        "◈ Live Pane Resize HUD"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(hud_title, usize::from(hud_head.width)),
            theme.style(theme.secondary).add_modifier(Modifier::BOLD),
        ))),
        hud_head,
    );

    let (w, h) = view.pane_size;
    let dim_text = if theme.ascii {
        format!("Current Pane: {w}x{h}  |  Step: {} cells", view.resize_step)
    } else if zh {
        format!("当前窗格尺寸: {w}×{h}  ·  步长: {} 格", view.resize_step)
    } else {
        format!("Current Pane: {w}×{h}  ·  Step: {} cells", view.resize_step)
    };
    let mut dim_spans = vec![Span::styled(
        truncate_to_width(&dim_text, usize::from(dim_area.width)),
        theme.style(theme.text).add_modifier(Modifier::BOLD),
    )];
    if view.zoomed {
        let z = if theme.ascii {
            " [ZOOMED]"
        } else if zh {
            " ⛶ 已放大"
        } else {
            " ⛶ ZOOMED"
        };
        dim_spans.push(Span::styled(
            z,
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        ));
    }
    if view.synchronized {
        let s = if theme.ascii {
            " [!SYNC ON]"
        } else if zh {
            " ⚡同步输入已开启"
        } else {
            " ⚡SYNC ON"
        };
        dim_spans.push(Span::styled(
            s,
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(dim_spans)), dim_area);

    let resize_pills = [
        ToolbarPill {
            action: ControllerAction::ResizeLeft,
            full: if theme.ascii {
                " [<-] Narrow "
            } else if zh {
                " [←] 变窄 "
            } else {
                " [←] Narrow "
            },
            short: if theme.ascii { "[<-]" } else { "[←]" },
            color: theme.accent,
            enabled: true,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::ResizeRight,
            full: if theme.ascii {
                " [->] Widen "
            } else if zh {
                " [→] 变宽 "
            } else {
                " [→] Widen "
            },
            short: if theme.ascii { "[->]" } else { "[→]" },
            color: theme.accent,
            enabled: true,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::ResizeUp,
            full: if theme.ascii {
                " [Up] Taller "
            } else if zh {
                " [↑] 增高 "
            } else {
                " [↑] Taller "
            },
            short: if theme.ascii { "[Up]" } else { "[↑]" },
            color: theme.accent,
            enabled: true,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::ResizeDown,
            full: if theme.ascii {
                " [Down] Shorter "
            } else if zh {
                " [↓] 变矮 "
            } else {
                " [↓] Shorter "
            },
            short: if theme.ascii { "[Dn]" } else { "[↓]" },
            color: theme.accent,
            enabled: true,
            emphasized: false,
        },
    ];
    controller_hits.extend(render_controller_toolbar(
        frame,
        resize_row,
        &resize_pills,
        theme,
    ));

    let zoom_full = if view.zoomed {
        if theme.ascii {
            " [z] Unzoom "
        } else if zh {
            " [z] ⛶ 取消放大 "
        } else {
            " [z] ⛶ Unzoom "
        }
    } else if theme.ascii {
        " [z] Zoom Pane "
    } else if zh {
        " [z] ⛶ 放大窗格 "
    } else {
        " [z] ⛶ Zoom Pane "
    };
    let sync_full = if view.synchronized {
        if theme.ascii {
            " [s] !SYNC ON "
        } else if zh {
            " [s] ⚡同步输入: 开 "
        } else {
            " [s] ⚡SYNC: ON "
        }
    } else if theme.ascii {
        " [s] Sync Panes OFF "
    } else if zh {
        " [s] 同步输入: 关 "
    } else {
        " [s] Sync Panes: OFF "
    };
    let toggle_pills = [
        ToolbarPill {
            action: ControllerAction::ToggleZoom,
            full: zoom_full,
            short: if view.zoomed {
                "[z] Unzoom"
            } else {
                "[z] Zoom"
            },
            color: if view.zoomed {
                theme.accent
            } else {
                theme.secondary
            },
            enabled: true,
            emphasized: view.zoomed,
        },
        ToolbarPill {
            action: ControllerAction::ToggleSync,
            full: sync_full,
            short: if view.synchronized {
                "[s] !SYNC"
            } else {
                "[s] Sync"
            },
            color: if view.synchronized {
                theme.warning
            } else {
                theme.secondary
            },
            enabled: true,
            emphasized: view.synchronized,
        },
    ];
    controller_hits.extend(render_controller_toolbar(
        frame,
        toggle_row,
        &toggle_pills,
        theme,
    ));

    if note_area.height > 0 {
        if let Some(note) = view.status_note {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate_to_width(&theme.adapt(note), usize::from(note_area.width)),
                    status_style(note, theme),
                ))),
                note_area,
            );
        } else {
            let tip = if zh {
                "提示: 1-5 或点击上方卡片一键应用预设 · 方向键实时微调窗格大小"
            } else {
                "Tip: Press 1-5 or click a card to apply preset · Arrows resize active pane live"
            };
            frame.render_widget(
                Paragraph::new(styled_key_hint_line(
                    &truncate_to_width(&theme.adapt(tip), usize::from(note_area.width)),
                    theme.style(theme.muted),
                    theme,
                )),
                note_area,
            );
        }
    }

    let hydra_rendered =
        render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);

    WorkspaceSubviewLayout {
        hydra_rendered,
        controller_hits,
        sash_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_layout_studio(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &LayoutStudioView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 12 || area.height < 4 {
        return WorkspaceSubviewLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head, body, bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let (w, h) = view.pane_size;
    let title = if zh {
        format!("布局 {w}x{h} ({}格)", view.pane_count)
    } else {
        format!("Layout {w}x{h} ({}p)", view.pane_count)
    };
    frame.render_widget(
        Paragraph::new(truncate_to_width(&title, usize::from(head.width)))
            .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        head,
    );
    let mut hits = render_layout_preset_cards(frame, body, view.selected_preset, theme);
    let pills = [
        ToolbarPill {
            action: ControllerAction::ToggleZoom,
            full: "[z] Zoom",
            short: "[z]",
            color: theme.accent,
            enabled: true,
            emphasized: view.zoomed,
        },
        ToolbarPill {
            action: ControllerAction::ToggleSync,
            full: if view.synchronized {
                "[s] !SYNC"
            } else {
                "[s] Sync"
            },
            short: "[s]",
            color: if view.synchronized {
                theme.warning
            } else {
                theme.secondary
            },
            enabled: true,
            emphasized: view.synchronized,
        },
    ];
    hits.extend(render_controller_toolbar(frame, bar, &pills, theme));
    WorkspaceSubviewLayout {
        controller_hits: hits,
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 4. Client Manager Subview (`/clients`)
// ---------------------------------------------------------------------------

fn build_client_list_items(
    clients: &[ClientRowView],
    selected: usize,
    list_width: u16,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let bg = theme.pill_bg();
    let max_w = usize::from(list_width);
    clients
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let is_sel = i == selected;
            let base = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            let dim = if theme.ascii {
                format!(" {}x{} ", c.width, c.height)
            } else {
                format!(" {}×{} ", c.width, c.height)
            };
            let mut spans = vec![
                Span::styled(format!("{marker} "), base),
                Span::styled(
                    truncate_to_width(&single_line(&c.name), max_w / 3),
                    base.add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                pill_span(
                    format!(" {} ", single_line(&c.session_name)),
                    theme.secondary,
                    bg,
                    theme,
                    is_sel,
                ),
                Span::raw(" "),
                pill_span(dim, theme.accent, bg, theme, false),
            ];
            if c.readonly {
                spans.push(Span::raw(" "));
                spans.push(pill_span(
                    if zh { " 只读 " } else { " RO " },
                    theme.warning,
                    bg,
                    theme,
                    true,
                ));
            }
            if c.is_current {
                spans.push(Span::raw(" "));
                spans.push(pill_span(
                    if zh {
                        " (当前终端) "
                    } else {
                        " (this client) "
                    },
                    theme.success,
                    bg,
                    theme,
                    true,
                ));
            }
            let used: usize = spans.iter().map(Span::width).sum();
            let rem = max_w.saturating_sub(used + 2);
            if rem >= 4 && !c.tty.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate_to_width(&single_line(&c.tty), rem),
                    if is_sel {
                        base
                    } else {
                        theme.style(theme.muted)
                    },
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

pub(super) fn render_subview_clients(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    view: &ClientsView<'_>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 12 {
        return render_compact_clients(frame, card_inner, view, theme);
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let mismatch = view.size_mismatch_warning || detect_client_dimension_mismatch(view.clients);
    let has_other_clients = view.clients.len() > 1;
    let has_any = !view.clients.is_empty();

    let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 6, false);
    let [head_area, warn_area, toolbar_area, note_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(u16::from(view.status_note.is_some())),
    ])
    .areas(upper_band);

    let title = if theme.ascii {
        format!("[] Attached tmux Clients ({})", view.clients.len())
    } else if zh {
        format!("👥 已连接的 tmux 客户端 ({})", view.clients.len())
    } else {
        format!("👥 Attached tmux Clients ({})", view.clients.len())
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(&title, usize::from(head_area.width)),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        ))),
        head_area,
    );

    // Dotted-border / dimension-mismatch info card.
    if mismatch {
        let warn_icon = if theme.ascii { "!" } else { "⚠" };
        let warn_lines = vec![
            Line::from(Span::styled(
                truncate_to_width(
                    &theme.adapt(&if zh {
                        format!(
                            "{warn_icon} 检测到多端分辨率不一致 — 可能导致 tmux 边缘出现虚线框限制"
                        )
                    } else {
                        format!(
                            "{warn_icon} Mismatched client dimensions detected — tmux constrains viewport with dotted borders"
                        )
                    }),
                    usize::from(warn_area.width),
                ),
                theme.style(theme.warning).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate_to_width(
                    if zh {
                        "  点击下方 [踢出其他端恢复全屏] 或按 d 立即释放较小客户端尺寸锁定"
                    } else {
                        "  Click [Detach Other Clients] or press d to reclaim full screen immediately"
                    },
                    usize::from(warn_area.width),
                ),
                theme.style(theme.text),
            )),
        ];
        frame.render_widget(Paragraph::new(warn_lines), warn_area);
    } else {
        let info = if zh {
            "管理所有附着到当前 tmux 服务的终端客户端，排查屏幕尺寸锁定或安全踢出旧连接"
        } else {
            "Inspect every terminal attached to this tmux server, fix viewport locks, or detach stale clients"
        };
        let wrapped = wrap_description(&theme.adapt(info), warn_area.width);
        let lines: Vec<Line<'static>> = wrapped
            .into_iter()
            .take(usize::from(warn_area.height))
            .map(|l| Line::from(Span::styled(l, theme.style(theme.muted))))
            .collect();
        frame.render_widget(Paragraph::new(lines), warn_area);
    }

    let pills = [
        ToolbarPill {
            action: ControllerAction::DetachOtherClients,
            full: if zh {
                " [d] 踢出其他端恢复全屏 "
            } else {
                " [d] Detach Other Clients "
            },
            short: if zh {
                "[d] 踢出其他"
            } else {
                "[d] Detach Others"
            },
            color: if mismatch {
                theme.warning
            } else {
                theme.accent
            },
            enabled: has_other_clients,
            emphasized: mismatch || has_other_clients,
        },
        ToolbarPill {
            action: ControllerAction::DetachSelectedClient,
            full: if zh {
                " [x] 分离选中客户端 "
            } else {
                " [x] Detach Selected "
            },
            short: if zh { "[x] 分离选中" } else { "[x] Detach" },
            color: theme.secondary,
            enabled: has_any,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::KillServer,
            full: if zh {
                " [K] 关闭服务 "
            } else {
                " [K] Kill Server "
            },
            short: "[K] Kill",
            color: theme.error,
            enabled: true,
            emphasized: false,
        },
    ];
    let controller_hits = render_controller_toolbar(frame, toolbar_area, &pills, theme);

    if let Some(note) = view.status_note
        && note_area.height > 0
    {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(&theme.adapt(note), usize::from(note_area.width)),
                status_style(note, theme),
            ))),
            note_area,
        );
    }

    render_horizontal_divider_line(frame, rule_area, theme);
    let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);

    if view.clients.is_empty() {
        frame.render_widget(
            Paragraph::new(if zh {
                "当前没有已附着的 tmux 客户端"
            } else {
                "No attached tmux clients found"
            })
            .style(theme.style(theme.muted)),
            left_band,
        );
    } else {
        let clamped = view.selected.min(view.clients.len() - 1);
        let items = build_client_list_items(view.clients, clamped, left_band.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), left_band, &mut state);
    }

    let hydra_rendered =
        render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);

    WorkspaceSubviewLayout {
        list_rect: Some(left_band),
        hydra_rendered,
        controller_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_clients(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &ClientsView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 12 || area.height < 4 {
        return WorkspaceSubviewLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head, list_area, bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(if zh {
            format!("客户端 ({})", view.clients.len())
        } else {
            format!("Clients ({})", view.clients.len())
        })
        .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        head,
    );
    if !view.clients.is_empty() {
        let clamped = view.selected.min(view.clients.len() - 1);
        let items = build_client_list_items(view.clients, clamped, list_area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
    }
    let pills = [
        ToolbarPill {
            action: ControllerAction::DetachOtherClients,
            full: "[d] Detach Others",
            short: "[d] Others",
            color: theme.warning,
            enabled: view.clients.len() > 1,
            emphasized: true,
        },
        ToolbarPill {
            action: ControllerAction::DetachSelectedClient,
            full: "[x] Detach",
            short: "[x]",
            color: theme.secondary,
            enabled: !view.clients.is_empty(),
            emphasized: false,
        },
    ];
    let hits = render_controller_toolbar(frame, bar, &pills, theme);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        controller_hits: hits,
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 5. Paste Buffer Manager Subview (`/buffers`)
// ---------------------------------------------------------------------------

fn build_buffer_list_items(
    buffers: &[BufferRowView],
    selected: usize,
    list_width: u16,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let bg = theme.pill_bg();
    let max_w = usize::from(list_width);
    buffers
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let is_sel = i == selected;
            let base = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            let size_pill = pill_span(
                format!(" {} ", format_byte_size(b.size)),
                theme.secondary,
                bg,
                theme,
                false,
            );
            let mut spans = vec![
                Span::styled(format!("{marker} "), base),
                Span::styled(
                    truncate_to_width(&single_line(&b.name), (max_w / 3).max(8)),
                    base.add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                size_pill,
            ];
            let used: usize = spans.iter().map(Span::width).sum();
            let rem = max_w.saturating_sub(used + 2);
            if rem >= 4 && !b.sample.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate_to_width(&single_line(&theme.adapt(&b.sample)), rem),
                    if is_sel {
                        base
                    } else {
                        theme.style(theme.muted)
                    },
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

pub(super) fn render_subview_buffers(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    view: &BuffersView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 8 || card_inner.width < 28 {
        return render_compact_buffers(frame, card_inner, view, theme);
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let has_any = !view.buffers.is_empty();
    let [head_area, body_area, toolbar_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(card_inner);

    let mut head_spans = vec![Span::styled(
        if theme.ascii {
            format!("[] tmux Paste Buffers ({})", view.buffers.len())
        } else if zh {
            format!("◈ tmux 粘贴缓冲区管理 ({})", view.buffers.len())
        } else {
            format!("◈ tmux Paste Buffers ({})", view.buffers.len())
        },
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(note) = view.status_note {
        head_spans.push(Span::raw("  "));
        head_spans.push(Span::styled(
            truncate_to_width(
                &theme.adapt(note),
                usize::from(head_area.width).saturating_sub(28),
            ),
            status_style(note, theme),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(head_spans)), head_area);

    let (list_rect, preview_rect) = if body_area.width >= 68 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)])
            .areas(body_area);
        (l, r)
    } else {
        let [t, b] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)])
            .areas(body_area);
        (t, b)
    };

    // Left: buffer list inside subtle card.
    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.secondary))
        .title(Line::from(Span::styled(
            if zh { " 缓冲区列表 " } else { " Buffers " },
            theme.style(theme.secondary).add_modifier(Modifier::BOLD),
        )));
    let list_inner = list_block.inner(list_rect);
    frame.render_widget(list_block, list_rect);

    if view.buffers.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.adapt(if zh {
                "暂无粘贴缓冲区 · 按 n 新建或使用预览区复制"
            } else {
                "No paste buffers · Press n to create one"
            }))
            .style(theme.style(theme.muted)),
            list_inner,
        );
    } else {
        let clamped = view.selected.min(view.buffers.len() - 1);
        let items = build_buffer_list_items(view.buffers, clamped, list_inner.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), list_inner, &mut state);
    }

    // Right: top-down scrollable full content preview card (scroll=0 starts at line 0).
    let selected_buf = view
        .buffers
        .get(view.selected.min(view.buffers.len().saturating_sub(1)));
    let prev_title = selected_buf.map_or_else(
        || {
            if zh {
                " 缓冲区内容预览 "
            } else {
                " Buffer Content "
            }
            .to_owned()
        },
        |b| format!(" {} ({}) ", single_line(&b.name), format_byte_size(b.size)),
    );
    let inner_h = usize::from(preview_rect.height.saturating_sub(2));
    let total_lines = view.preview_lines.len();
    let max_scroll = total_lines.saturating_sub(inner_h);
    let offset = view.preview_scroll.min(max_scroll);

    let adapted_title = theme.adapt(&prev_title);
    let left_used = adapted_title.width() + 2;
    let mut prev_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.accent))
        .title(Line::from(Span::styled(
            truncate_to_width(
                &adapted_title,
                usize::from(preview_rect.width.saturating_sub(2)),
            ),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )));

    if inner_h > 0 && max_scroll > 0 {
        let (full, compact, style) = if offset > 0 {
            let st = theme.style(theme.accent).add_modifier(Modifier::BOLD);
            if theme.ascii {
                (
                    format!(" [+{offset}/{max_scroll} PgUp/PgDn] "),
                    format!(" [+{offset}] "),
                    st,
                )
            } else {
                (
                    format!(" ↓ +{offset}/{max_scroll} · PgUp/PgDn "),
                    format!(" ↓ +{offset} "),
                    st,
                )
            }
        } else {
            let st = theme.style(theme.muted);
            if theme.ascii {
                (
                    format!(" [{total_lines}L PgDn/Wheel] "),
                    format!(" [{total_lines}L] "),
                    st,
                )
            } else {
                let lines_unit = if zh { "行" } else { "L" };
                let wheel = if zh { "滚轮" } else { "Wheel" };
                (
                    format!(" ↕ {total_lines}{lines_unit} · PgDn/{wheel} "),
                    format!(" ↕ {total_lines}{lines_unit} "),
                    st,
                )
            }
        };
        let avail = usize::from(preview_rect.width);
        if left_used + full.width() + 2 <= avail {
            prev_block =
                prev_block.title_top(styled_key_hint_line(&full, style, theme).right_aligned());
        } else if left_used + compact.width() + 2 <= avail {
            prev_block =
                prev_block.title_top(styled_key_hint_line(&compact, style, theme).right_aligned());
        }
    }

    let prev_inner = prev_block.inner(preview_rect);
    frame.render_widget(prev_block, preview_rect);
    if prev_inner.width > 0 && prev_inner.height > 0 {
        if view.preview_lines.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate_to_width(
                        if zh {
                            " (空粘贴缓冲区)"
                        } else {
                            " (empty paste buffer)"
                        },
                        usize::from(prev_inner.width),
                    ),
                    theme.style(theme.muted),
                ))),
                prev_inner,
            );
        } else {
            let max_w = usize::from(prev_inner.width);
            let lines: Vec<Line<'static>> = view
                .preview_lines
                .iter()
                .skip(offset)
                .take(usize::from(prev_inner.height))
                .map(|raw| parse_ansi_preview_line(raw, max_w, theme))
                .collect();
            frame.render_widget(Paragraph::new(lines), prev_inner);
        }
    }

    let pills = [
        ToolbarPill {
            action: ControllerAction::PasteBuffer,
            full: if zh {
                " [Enter] 粘贴到窗格 "
            } else {
                " [Enter] Paste to Pane "
            },
            short: if zh {
                "[Enter] 粘贴"
            } else {
                "[Enter] Paste"
            },
            color: theme.success,
            enabled: has_any,
            emphasized: has_any,
        },
        ToolbarPill {
            action: ControllerAction::CopyBufferToSystem,
            full: if zh {
                " [y] 复制到系统剪贴板 "
            } else {
                " [y] Copy to System "
            },
            short: if zh { "[y] 复制" } else { "[y] Copy" },
            color: theme.accent,
            enabled: has_any,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::NewBuffer,
            full: if zh {
                " [n] + 新建缓冲区 "
            } else {
                " [n] + New Buffer "
            },
            short: if zh { "[n] + 新建" } else { "[n] + New" },
            color: theme.secondary,
            enabled: true,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::DeleteBuffer,
            full: if zh { " [x] 删除 " } else { " [x] Delete " },
            short: "[x] Del",
            color: theme.warning,
            enabled: has_any,
            emphasized: false,
        },
    ];
    let controller_hits = render_controller_toolbar(frame, toolbar_area, &pills, theme);

    WorkspaceSubviewLayout {
        list_rect: Some(list_inner),
        preview_area: Some(preview_rect),
        controller_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_buffers(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &BuffersView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 12 || area.height < 4 {
        return WorkspaceSubviewLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head, list_area, bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(if zh {
            format!("缓冲区 ({})", view.buffers.len())
        } else {
            format!("Buffers ({})", view.buffers.len())
        })
        .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        head,
    );
    if !view.buffers.is_empty() {
        let clamped = view.selected.min(view.buffers.len() - 1);
        let items = build_buffer_list_items(view.buffers, clamped, list_area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
    }
    let pills = [
        ToolbarPill {
            action: ControllerAction::PasteBuffer,
            full: "[Enter] Paste",
            short: "[Enter]",
            color: theme.success,
            enabled: !view.buffers.is_empty(),
            emphasized: true,
        },
        ToolbarPill {
            action: ControllerAction::CopyBufferToSystem,
            full: "[y] Copy",
            short: "[y]",
            color: theme.accent,
            enabled: !view.buffers.is_empty(),
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::NewBuffer,
            full: "[n] New",
            short: "[n]",
            color: theme.secondary,
            enabled: true,
            emphasized: false,
        },
    ];
    let hits = render_controller_toolbar(frame, bar, &pills, theme);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        controller_hits: hits,
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 6. Workspace Snapshots / Templates Subview (`/snapshot`)
// ---------------------------------------------------------------------------

fn build_snapshot_list_items(
    snapshots: &[SnapshotRowView],
    selected: usize,
    list_width: u16,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let bg = theme.pill_bg();
    let max_w = usize::from(list_width);
    snapshots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let is_sel = i == selected;
            let base = if is_sel {
                theme.selected_style()
            } else {
                theme.style(theme.text)
            };
            let marker = if is_sel { theme.marker() } else { " " };
            let counts = if zh {
                format!(
                    " {}会话·{}窗·{}格 ",
                    s.session_count, s.window_count, s.pane_count
                )
            } else if theme.ascii {
                format!(
                    " {}s/{}w/{}p ",
                    s.session_count, s.window_count, s.pane_count
                )
            } else {
                format!(
                    " {}s·{}w·{}p ",
                    s.session_count, s.window_count, s.pane_count
                )
            };
            let mut spans = vec![
                Span::styled(format!("{marker} "), base),
                Span::styled(
                    truncate_to_width(&single_line(&s.name), (max_w / 2).max(10)),
                    base.add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                pill_span(counts, theme.secondary, bg, theme, false),
            ];
            let used: usize = spans.iter().map(Span::width).sum();
            let rem = max_w.saturating_sub(used + 2);
            if rem >= 4 && !s.created_label.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate_to_width(&single_line(&s.created_label), rem),
                    if is_sel {
                        base
                    } else {
                        theme.style(theme.muted)
                    },
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

pub(super) fn render_subview_snapshots(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    view: &SnapshotsView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 8 || card_inner.width < 28 {
        return render_compact_snapshots(frame, card_inner, view, theme);
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let has_any = !view.snapshots.is_empty();
    let [head_area, body_area, toolbar_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(card_inner);

    let mut head_spans = vec![Span::styled(
        if theme.ascii {
            format!("[] Workspace Snapshots ({})", view.snapshots.len())
        } else if zh {
            format!("◈ 工作区快照与模板 ({})", view.snapshots.len())
        } else {
            format!(
                "◈ Workspace Snapshots & Templates ({})",
                view.snapshots.len()
            )
        },
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(note) = view.status_note {
        head_spans.push(Span::raw("  "));
        head_spans.push(Span::styled(
            truncate_to_width(
                &theme.adapt(note),
                usize::from(head_area.width).saturating_sub(32),
            ),
            status_style(note, theme),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(head_spans)), head_area);

    let (list_rect, preview_rect) = if body_area.width >= 68 {
        let [l, r] = Layout::horizontal([Constraint::Percentage(46), Constraint::Percentage(54)])
            .areas(body_area);
        (l, r)
    } else {
        let [t, b] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)])
            .areas(body_area);
        (t, b)
    };

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.secondary))
        .title(Line::from(Span::styled(
            if zh {
                " 已保存快照 "
            } else {
                " Saved Snapshots "
            },
            theme.style(theme.secondary).add_modifier(Modifier::BOLD),
        )));
    let list_inner = list_block.inner(list_rect);
    frame.render_widget(list_block, list_rect);

    if view.snapshots.is_empty() {
        frame.render_widget(
            Paragraph::new(theme.adapt(if zh {
                "暂无保存的工作区快照 · 按 n 保存当前 tmux 布局"
            } else {
                "No saved snapshots yet · Press n to capture current workspace"
            }))
            .style(theme.style(theme.muted)),
            list_inner,
        );
    } else {
        let clamped = view.selected.min(view.snapshots.len() - 1);
        let items = build_snapshot_list_items(view.snapshots, clamped, list_inner.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), list_inner, &mut state);
    }

    // Right: structured tree preview box.
    let selected_snap = view
        .snapshots
        .get(view.selected.min(view.snapshots.len().saturating_sub(1)));
    let tree_title = selected_snap.map_or_else(
        || {
            if zh {
                " 工作区结构预览 "
            } else {
                " Workspace Tree Preview "
            }
            .to_owned()
        },
        |s| {
            format!(
                " {} · {} ",
                single_line(&s.name),
                single_line(&s.created_label)
            )
        },
    );
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.accent))
        .title(Line::from(Span::styled(
            truncate_to_width(
                &theme.adapt(&tree_title),
                usize::from(preview_rect.width.saturating_sub(2)),
            ),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )));
    let tree_inner = tree_block.inner(preview_rect);
    frame.render_widget(tree_block, preview_rect);

    if view.tree_preview.is_empty() {
        frame.render_widget(
            Paragraph::new(if zh {
                "选择左侧快照以预览会话、窗口与目录结构"
            } else {
                "Select a snapshot to inspect sessions, windows & paths"
            })
            .style(theme.style(theme.muted)),
            tree_inner,
        );
    } else {
        let inner_h = usize::from(tree_inner.height);
        let max_scroll = view.tree_preview.len().saturating_sub(inner_h);
        let offset = view.preview_scroll.min(max_scroll);
        let lines: Vec<Line<'static>> = view
            .tree_preview
            .iter()
            .skip(offset)
            .take(inner_h)
            .map(|raw| {
                let adapted = theme.adapt(raw);
                let trimmed = adapted.trim_start();
                let style = if !raw.starts_with(' ')
                    && !trimmed.starts_with('|')
                    && !trimmed.starts_with('├')
                    && !trimmed.starts_with('└')
                {
                    theme.style(theme.accent).add_modifier(Modifier::BOLD)
                } else if raw.starts_with("  ") && !raw.starts_with("    ") {
                    theme.style(theme.secondary)
                } else {
                    theme.style(theme.text)
                };
                Line::from(Span::styled(
                    truncate_to_width(&adapted, usize::from(tree_inner.width)),
                    style,
                ))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), tree_inner);
    }

    let pills = [
        ToolbarPill {
            action: ControllerAction::SaveSnapshot,
            full: if zh {
                " [n] 保存当前工作区 "
            } else {
                " [n] Save Current Workspace "
            },
            short: if zh { "[n] 保存快照" } else { "[n] Save" },
            color: theme.accent,
            enabled: true,
            emphasized: !has_any,
        },
        ToolbarPill {
            action: ControllerAction::RestoreSnapshot,
            full: if zh {
                " [Enter] 恢复布局 "
            } else {
                " [Enter] Restore "
            },
            short: if zh {
                "[Enter] 恢复"
            } else {
                "[Enter] Restore"
            },
            color: theme.success,
            enabled: has_any,
            emphasized: has_any,
        },
        ToolbarPill {
            action: ControllerAction::DeleteSnapshot,
            full: if zh { " [x] 删除 " } else { " [x] Delete " },
            short: "[x] Del",
            color: theme.warning,
            enabled: has_any,
            emphasized: false,
        },
    ];
    let controller_hits = render_controller_toolbar(frame, toolbar_area, &pills, theme);

    WorkspaceSubviewLayout {
        list_rect: Some(list_inner),
        preview_area: Some(preview_rect),
        controller_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_snapshots(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &SnapshotsView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 12 || area.height < 4 {
        return WorkspaceSubviewLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head, list_area, bar] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(if zh {
            format!("工作区快照 ({})", view.snapshots.len())
        } else {
            format!("Snapshots ({})", view.snapshots.len())
        })
        .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        head,
    );
    if !view.snapshots.is_empty() {
        let clamped = view.selected.min(view.snapshots.len() - 1);
        let items = build_snapshot_list_items(view.snapshots, clamped, list_area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
    }
    let pills = [
        ToolbarPill {
            action: ControllerAction::SaveSnapshot,
            full: "[n] Save",
            short: "[n]",
            color: theme.accent,
            enabled: true,
            emphasized: false,
        },
        ToolbarPill {
            action: ControllerAction::RestoreSnapshot,
            full: "[Enter] Restore",
            short: "[Enter]",
            color: theme.success,
            enabled: !view.snapshots.is_empty(),
            emphasized: true,
        },
    ];
    let hits = render_controller_toolbar(frame, bar, &pills, theme);
    WorkspaceSubviewLayout {
        list_rect: Some(list_area),
        controller_hits: hits,
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 7. Generic Target Picker with Action Banner (`SubviewContent::TargetPicker`)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn render_subview_target_picker(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    title: &str,
    action_banner: &str,
    rows: &[RichPickerRow],
    selected: usize,
    empty: &str,
    preview: Option<PanePreview<'_>>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 4 {
        return WorkspaceSubviewLayout::default();
    }
    let [banner_area, rest_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(card_inner);
    let icon = if theme.ascii { "-> " } else { "▸ " };
    let banner_text = format!(
        "{icon}{}",
        single_line(&theme.adapt(if action_banner.is_empty() {
            title
        } else {
            action_banner
        }))
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(&banner_text, usize::from(banner_area.width)),
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        ))),
        banner_area,
    );

    let render_list = |frame: &mut Frame<'_>, area: Rect| {
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(theme.adapt(empty)).style(theme.style(theme.muted)),
                area,
            );
            return;
        }
        let clamped = selected.min(rows.len() - 1);
        let items = build_rich_picker_list_items(rows, clamped, area.width, theme);
        let mut state = ListState::default();
        state.select(Some(clamped));
        frame.render_stateful_widget(List::new(items), area, &mut state);
    };

    if rest_area.height >= 12 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(rest_area, 5, true);
        let preview_layout = if let Some(card) = preview {
            render_pane_preview_card_inner(frame, upper_band, card, theme)
        } else {
            let counter = if rows.is_empty() {
                " [0]".to_owned()
            } else {
                format!(" [{}/{}]", selected.min(rows.len() - 1) + 1, rows.len())
            };
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![
                        Span::styled(
                            theme.adapt(title),
                            theme.style(theme.accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(counter, theme.style(theme.muted)),
                    ]),
                    Line::from(Span::styled(
                        theme.adapt(theme.tr(
                            "Select destination target below and press Enter to confirm",
                            "在下方选择目标对象，按 Enter 立即执行",
                        )),
                        theme.style(theme.muted),
                    )),
                ]),
                upper_band,
            );
            PreviewCardLayout::default()
        };
        let preview_area = preview_layout.area;
        let preview_action_hits = preview_layout.action_hits.clone();
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        render_list(frame, left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            list_rect: Some(left_band),
            preview_area,
            hydra_rendered,
            preview_action_hits,
            preview_layout,
            ..WorkspaceSubviewLayout::default()
        };
    }

    render_list(frame, rest_area);
    WorkspaceSubviewLayout {
        list_rect: Some(rest_area),
        ..WorkspaceSubviewLayout::default()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_compact_target_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    action_banner: &str,
    query: &TextInput,
    rows: &[RichPickerRow],
    selected: usize,
    empty: &str,
    footer_hint: &str,
    preview: Option<PanePreview<'_>>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let heading = if action_banner.is_empty() {
        title
    } else {
        action_banner
    };
    render_rich_picker_with_preview(
        frame,
        area,
        heading,
        query,
        rows,
        selected,
        empty,
        footer_hint,
        preview,
        theme,
    );
    let [_, _, body_area, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let list_rect = if preview.is_some() && area.width >= 68 && body_area.height >= 4 {
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .areas::<2>(body_area)[0]
    } else {
        body_area
    };
    WorkspaceSubviewLayout {
        list_rect: Some(list_rect),
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 8. Dual-Field Form Subview (`SubviewContent::DualForm` — e.g. New Window Name + CWD)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn render_subview_dual_form(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    title: &str,
    first_label: &str,
    first_input: &TextInput,
    second_label: &str,
    second_input: &TextInput,
    active_field: usize,
    dir_exists: bool,
    auto_name: bool,
    completions: &[PathCompletionItem<'_>],
    completion_selected: usize,
    completion_open: bool,
    border_color: Color,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    let zh = theme.language.is_zh() && !theme.ascii;
    let render_fields = |frame: &mut Frame<'_>, area: Rect| -> [Option<Rect>; 2] {
        let mut field_hits = [None, None];
        let spacious = area.height >= 7;
        let mut lines = vec![Line::from(Span::styled(
            theme.adapt(title),
            theme.style(border_color).add_modifier(Modifier::BOLD),
        ))];
        if spacious {
            lines.push(Line::default());
        }
        for (idx, (lbl, inp)) in [(first_label, first_input), (second_label, second_input)]
            .into_iter()
            .enumerate()
        {
            let active = idx == active_field.min(1);
            let start_y = area
                .y
                .saturating_add(u16::try_from(lines.len()).unwrap_or(u16::MAX));
            let marker = if active { theme.marker() } else { " " };
            let lbl_style = if active {
                theme.style(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                theme.style(theme.muted)
            };
            let lbl_text = format!("{marker} {}", theme.adapt(lbl));
            let mut lbl_spans = vec![Span::styled(
                truncate_to_width(&lbl_text, usize::from(area.width)),
                lbl_style,
            )];
            if idx == 0 && auto_name {
                let badge = if zh {
                    "  [自动跟随目录]"
                } else {
                    "  [auto from folder]"
                };
                if lbl_text.width() + badge.width() <= usize::from(area.width) {
                    lbl_spans.push(Span::styled(theme.adapt(badge), theme.style(theme.muted)));
                }
            } else if idx == 1 {
                let (badge, color) = if dir_exists {
                    (
                        if theme.ascii {
                            "  [OK]"
                        } else if zh {
                            "  ✓ 目录存在"
                        } else {
                            "  ✓ exists"
                        },
                        theme.success,
                    )
                } else {
                    (
                        if theme.ascii {
                            "  [! not found]"
                        } else if zh {
                            "  ✕ 目录不存在"
                        } else {
                            "  ✕ not found"
                        },
                        theme.error,
                    )
                };
                if lbl_text.width() + badge.width() <= usize::from(area.width) {
                    lbl_spans.push(Span::styled(theme.adapt(badge), theme.style(color)));
                }
            }
            lines.push(Line::from(lbl_spans));

            let empty_hint = if idx == 0 {
                if zh {
                    "(留空使用目录名)"
                } else {
                    "(empty = folder name)"
                }
            } else if zh {
                "(留空使用默认目录)"
            } else {
                "(empty = default dir)"
            };
            if active {
                let val_avail = area.width.saturating_sub(4);
                let mut val_line = inp.rendered_width(true, theme, val_avail);
                val_line.spans.insert(0, Span::raw("    "));
                if inp.value().is_empty() {
                    let hint = theme.adapt(&format!(" {empty_hint}"));
                    if val_line.width() + hint.width() <= usize::from(area.width) {
                        val_line
                            .spans
                            .push(Span::styled(hint, theme.style(theme.muted)));
                    }
                }
                lines.push(val_line);
            } else {
                let val_text = if inp.value().is_empty() {
                    theme.adapt(empty_hint)
                } else {
                    single_line(inp.value())
                };
                let val_style = if inp.value().is_empty() {
                    theme.style(theme.muted)
                } else {
                    theme.style(theme.text)
                };
                lines.push(Line::from(Span::styled(
                    truncate_to_width(&format!("    {val_text}"), usize::from(area.width)),
                    val_style,
                )));
            }
            if start_y < area.bottom() && area.width > 0 {
                let h = area.bottom().saturating_sub(start_y).min(2);
                if h > 0 {
                    field_hits[idx] = Some(Rect {
                        x: area.x,
                        y: start_y,
                        width: area.width,
                        height: h,
                    });
                }
            }
            if idx == 0 && spacious {
                lines.push(Line::default());
            }
        }
        frame.render_widget(Paragraph::new(lines), area);
        field_hits
    };

    if completion_open && !completions.is_empty() && card_inner.height >= 8 {
        let (upper_band, rule_area, lower_band) = if card_inner.height >= 14 {
            split_subview_bands(card_inner, 7, false)
        } else {
            let [upper, rule, lower] = Layout::vertical([
                Constraint::Length(5),
                Constraint::Length(1),
                Constraint::Min(2),
            ])
            .areas(card_inner);
            (upper, rule, lower)
        };
        let dual_form_field_hits = render_fields(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let [header_area, list_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(lower_band);
        let head_label = format!(
            "{} ({})",
            theme.tr("Directories", "可选目录"),
            completions.len()
        );
        let head_hint = if theme.ascii {
            " | Tab/Up/Down Pick | / Drill | Enter Accept | Esc Close"
        } else if zh {
            " · Tab/↑↓ 选择 · / 钻入 · Enter 确认 · Esc 收起"
        } else {
            " · Tab/↑↓ Pick · / Drill · Enter Accept · Esc Close"
        };
        let mut head_spans = vec![Span::styled(
            truncate_to_width(&head_label, usize::from(header_area.width)),
            theme.style(theme.secondary).add_modifier(Modifier::BOLD),
        )];
        let rem = usize::from(header_area.width).saturating_sub(head_label.width());
        if rem > 4 {
            head_spans.push(Span::styled(
                truncate_to_width(head_hint, rem),
                theme.style(theme.muted),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(head_spans)), header_area);

        let clamped_selected = completion_selected.min(completions.len().saturating_sub(1));
        let items: Vec<ListItem<'_>> = completions
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let is_sel = idx == clamped_selected;
                let style = if is_sel {
                    theme.selected_style()
                } else {
                    theme.style(theme.text)
                };
                let marker = if is_sel { theme.marker() } else { " " };
                let suffix = if entry.is_symlink { "  @" } else { "" };
                let text = truncate_to_width(
                    &format!("{marker} {}{suffix}", single_line(entry.display)),
                    usize::from(list_area.width),
                );
                ListItem::new(Line::from(Span::styled(text, style)))
            })
            .collect();
        let mut state = ListState::default();
        state.select(Some(clamped_selected));
        frame.render_stateful_widget(List::new(items), list_area, &mut state);
        return WorkspaceSubviewLayout {
            list_rect: Some(list_area),
            dual_form_field_hits,
            ..WorkspaceSubviewLayout::default()
        };
    }

    if card_inner.height >= 14 {
        let (upper_band, rule_area, lower_band) = split_subview_bands(card_inner, 7, false);
        let dual_form_field_hits = render_fields(frame, upper_band);
        render_horizontal_divider_line(frame, rule_area, theme);
        let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
        let tips = Paragraph::new(vec![
            Line::from(Span::styled(
                theme.adapt(if zh {
                    "◈ 终端级名称与目录设置"
                } else {
                    "◈ Project Defaults & Path Completion"
                }),
                theme.style(theme.secondary).add_modifier(Modifier::BOLD),
            )),
            styled_key_hint_line(
                if zh {
                    "名称默认使用目录名 (直接打字覆盖) · 目录按 Tab 补全并列出可选项"
                } else {
                    "Name defaults to folder (type to replace) · Tab completes Directory"
                },
                theme.style(theme.text),
                theme,
            ),
            styled_key_hint_line(
                if zh {
                    "Enter 创建并进入 · Alt+Enter 仅创建 · Ctrl+W 退回上一级 · Esc 返回"
                } else {
                    "Enter Create & enter · Alt+Enter Stay · Ctrl+W Up dir · Esc Back"
                },
                theme.style(theme.muted),
                theme,
            ),
        ]);
        frame.render_widget(tips, left_band);
        let hydra_rendered =
            render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);
        return WorkspaceSubviewLayout {
            hydra_rendered,
            dual_form_field_hits,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let dual_form_field_hits = render_fields(frame, card_inner);
    WorkspaceSubviewLayout {
        dual_form_field_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_compact_dual_form(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    first_label: &str,
    first_input: &TextInput,
    second_label: &str,
    second_input: &TextInput,
    active_field: usize,
    completions: &[PathCompletionItem<'_>],
    completion_selected: usize,
    completion_open: bool,
    theme: Theme,
) {
    let (lbl, inp) = if active_field == 0 {
        (first_label.to_owned(), first_input)
    } else if completion_open && !completions.is_empty() {
        let clamped = completion_selected.min(completions.len() - 1);
        (
            format!(
                "{second_label} [{}/{} {}]",
                clamped + 1,
                completions.len(),
                completions[clamped].display
            ),
            second_input,
        )
    } else {
        (second_label.to_owned(), second_input)
    };
    let footer = if completion_open {
        theme.tr(
            "Tab Pick · / Drill · Enter Accept · Esc Close",
            "Tab 选 · / 钻 · Enter 确认 · Esc 收",
        )
    } else {
        theme.tr(
            "Tab Complete/Next · Enter Submit · Esc Back",
            "Tab 补全/切栏 · Enter 提交 · Esc 返回",
        )
    };
    super::render_form(frame, area, title, &lbl, inp, footer, theme);
}

// ---------------------------------------------------------------------------
// 9. Bespoke Visual Diagrams for `[tmux]` Comfort Config Toggles + Reload
// ---------------------------------------------------------------------------

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

pub(super) fn build_tmux_comfort_config_guide(
    key: &str,
    value: &str,
    description: &str,
    art_w: u16,
    art_h: u16,
    theme: Theme,
) -> Option<ConfigGuideSpec> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let fallback_summary = |default_text: &str| {
        if description.is_empty() {
            default_text.to_owned()
        } else {
            description.to_owned()
        }
    };

    match key {
        "tmux_mouse" => {
            let enabled = is_truthy(value);
            let art = config_art_tmux_mouse(enabled, art_w, art_h, theme);
            let compact = config_art_tmux_mouse(enabled, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "tmux 原生鼠标支持 (mouse)"
                } else {
                    "tmux Native Mouse Support"
                },
                summary: fallback_summary(if zh {
                    "写入 set -g mouse on 到 ~/.tmux.conf 托管块并热加载，支持滚轮回看、点击切窗格和拖拽边框"
                } else {
                    "Writes set -g mouse on to ~/.tmux.conf managed block & hot-applies wheel scroll, click focus, and border drag"
                }),
                traits: if zh {
                    [
                        "true: 滚轮查看历史 · 点击切窗格 · 拖拽调整大小".into(),
                        "false: 不托管 mouse，保留你的 ~/.tmux.conf 原样".into(),
                    ]
                } else {
                    [
                        "true: Wheel scrollback · Click panes · Drag borders".into(),
                        "false: Unmanaged; leaves your ~/.tmux.conf untouched".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 立即生效 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space apply live · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "tmux_history_limit" => {
            let limit = value.trim().parse::<u32>().unwrap_or(0);
            let art = config_art_tmux_history_limit(limit, art_w, art_h, theme);
            let compact = config_art_tmux_history_limit(limit, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "tmux 滚动历史深度 (history-limit)"
                } else {
                    "tmux Scrollback History Limit"
                },
                summary: fallback_summary(if zh {
                    "将 tmux 默认 2000 行历史扩展到 10K/50K/100K，长编译日志不再被截断"
                } else {
                    "Raises tmux's tiny 2,000-line default scrollback to 10K/50K/100K so build logs never clip"
                }),
                traits: if zh {
                    [
                        format!(
                            "当前值: {}",
                            if limit == 0 {
                                "0 (未托管 / tmux 默认 2000 行)".to_owned()
                            } else {
                                format!("{limit} 行 (set -g history-limit {limit})")
                            }
                        ),
                        "预设档位: 0 (关) · 10000 · 50000 · 100000".into(),
                    ]
                } else {
                    [
                        format!(
                            "Current: {}",
                            if limit == 0 {
                                "0 (unmanaged / tmux default 2000)".to_owned()
                            } else {
                                format!("{limit} lines (set -g history-limit {limit})")
                            }
                        ),
                        "Presets: 0 (off) · 10000 · 50000 · 100000".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 切换档位 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space cycle depth · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "tmux_base_index_one" => {
            let enabled = is_truthy(value);
            let art = config_art_tmux_base_index(enabled, art_w, art_h, theme);
            let compact = config_art_tmux_base_index(enabled, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "窗口与窗格从 1 开始编号"
                } else {
                    "1-Based Window & Pane Indexing"
                },
                summary: fallback_summary(if zh {
                    "启用 base-index 1 与 pane-base-index 1，数字键 1 在键盘最左侧更顺手"
                } else {
                    "Sets base-index 1 and pane-base-index 1 so the first window matches key 1 on your left hand"
                }),
                traits: if zh {
                    [
                        "true: 窗口/窗格编号 1, 2, 3... 与键盘左上角对齐".into(),
                        "false: 保留 tmux 默认从 0 开始编号".into(),
                    ]
                } else {
                    [
                        "true: Windows & panes start at 1 (matches keyboard)".into(),
                        "false: Keeps default tmux 0-based indexing".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 立即生效 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space apply live · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "tmux_renumber_windows" => {
            let enabled = is_truthy(value);
            let art = config_art_tmux_renumber(enabled, art_w, art_h, theme);
            let compact = config_art_tmux_renumber(enabled, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "关闭窗口后自动连续重排编号"
                } else {
                    "Auto-Renumber Windows on Close"
                },
                summary: fallback_summary(if zh {
                    "启用 set -g renumber-windows on，关闭中间窗口后自动消除编号空洞"
                } else {
                    "Enables set -g renumber-windows on so closing window 2 compacts [1][3] cleanly into [1][2]"
                }),
                traits: if zh {
                    [
                        "true: [1][2][3] 关 [2] -> 自动变为 [1][2] 无断号".into(),
                        "false: 保持原编号，可能留下 [1] _ [3] 空隙".into(),
                    ]
                } else {
                    [
                        "true: Closing [2] in [1][2][3] compacts to [1][2]".into(),
                        "false: Leaves numbering gaps like [1] _ [3]".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 立即生效 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space apply live · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "tmux_zero_escape_time" => {
            let enabled = is_truthy(value);
            let art = config_art_tmux_escape_time(enabled, art_w, art_h, theme);
            let compact = config_art_tmux_escape_time(enabled, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "消除 Esc 按键延迟 (escape-time 0)"
                } else {
                    "Zero Esc Key Delay (escape-time 0)"
                },
                summary: fallback_summary(if zh {
                    "设置 set -sg escape-time 0，彻底消除 Vim/Neovim 与终端 TUI 中按 Esc 的半秒卡顿"
                } else {
                    "Applies set -sg escape-time 0 to eliminate tmux's sluggish 500ms Esc delay in Vim/Neovim & TUIs"
                }),
                traits: if zh {
                    [
                        "true: 0ms 响应 — Neovim/Vim 切换模式零延迟".into(),
                        "false: 不修改 tmux 默认 escape-time (约 500ms)".into(),
                    ]
                } else {
                    [
                        "true: 0ms Esc latency — instant Neovim/Vim mode switch".into(),
                        "false: Keeps tmux default (~500ms Esc pause)".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 立即生效 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space apply live · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "live_stream" => {
            let enabled = is_truthy(value);
            let art = config_art_live_stream(enabled, art_w, art_h, theme);
            let compact = config_art_live_stream(enabled, 38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "实时终端流式预览 (live_stream)"
                } else {
                    "Live Terminal Preview Streaming"
                },
                summary: fallback_summary(if zh {
                    "在全屏工作台与 Exposé 全景墙中自动定时刷新终端画面与活跃脉冲，Inline 模式下空闲零刷新"
                } else if theme.ascii {
                    "Auto-streams live pane output & sparklines on Home and Expose wall; stays zero-idle-byte in Inline mode"
                } else {
                    "Auto-streams live pane output & sparklines on Home and Exposé wall; stays zero-idle-byte in Inline mode"
                }),
                traits: if zh {
                    [
                        "true: Home 预览 / Exposé 全景墙自动流式刷新与实时直控".into(),
                        "false: 仅在切换选中窗口或操作时按需抓取画面".into(),
                    ]
                } else if theme.ascii {
                    [
                        "true: Streams live Home preview, Expose wall & sparklines".into(),
                        "false: On-demand capture only when switching windows".into(),
                    ]
                } else {
                    [
                        "true: Streams live Home preview, Exposé wall & sparklines".into(),
                        "false: On-demand capture only when switching windows".into(),
                    ]
                },
                keys_hint: if zh {
                    "←→ / Space 切换开关 · s 保存 · Esc 返回"
                } else {
                    "←→ / Space toggle · s Save · Esc Back"
                },
                art,
                compact_art,
            })
        }
        "tmux_reload" | "reload_tmux_conf" => {
            let art = config_art_tmux_reload(art_w, art_h, theme);
            let compact = config_art_tmux_reload(38, 5, theme);
            let compact_art = compact.into_iter().take(2).collect();
            Some(ConfigGuideSpec {
                title: if zh {
                    "立即热重载 ~/.tmux.conf"
                } else {
                    "Live Reload ~/.tmux.conf"
                },
                summary: fallback_summary(if zh {
                    "对当前运行中的 tmux 服务执行 source-file ~/.tmux.conf，无需重启任何会话或窗格"
                } else {
                    "Runs tmux source-file ~/.tmux.conf on the active server without restarting any session or pane"
                }),
                traits: if zh {
                    [
                        "按 Enter / Space 立即重载 ~/.tmux.conf".into(),
                        "安全零破坏: 仅刷新配置，保留所有运行中的进程".into(),
                    ]
                } else {
                    [
                        "Press Enter / Space to reload ~/.tmux.conf now".into(),
                        "Non-destructive: keeps every running pane alive".into(),
                    ]
                },
                keys_hint: if zh {
                    "Enter / Space 立即重载 · Esc 返回"
                } else {
                    "Enter / Space reload now · Esc Back"
                },
                art,
                compact_art,
            })
        }
        _ => None,
    }
}

fn config_art_tmux_mouse(
    enabled: bool,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let ok = if enabled {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let dim = theme.style(theme.muted);
    let w = usize::from(art_w).clamp(32, 44);
    let body = if enabled {
        vec![
            Line::from(Span::styled(
                if zh {
                    "  [●] set -g mouse on"
                } else if theme.ascii {
                    "  [*] set -g mouse on"
                } else {
                    "  [●] set -g mouse on"
                },
                ok,
            )),
            Line::from(Span::styled(
                if zh {
                    "  滚轮回看 · 点击切窗格 · 拖拽边框"
                } else {
                    "  Wheel scroll | Click focus | Drag"
                },
                theme.style(theme.text),
            )),
            Line::from(Span::styled(
                if theme.ascii {
                    "  +--------+   <-- drag --> +------+"
                } else {
                    "  ┌────────┐  ◀─ drag ─▶  ┌──────┐"
                },
                hi,
            )),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                if zh {
                    "  [○] 未启用托管 (tmux 默认)"
                } else if theme.ascii {
                    "  [o] unmanaged (tmux default)"
                } else {
                    "  [○] unmanaged (tmux default)"
                },
                dim,
            )),
            Line::from(Span::styled(
                if zh {
                    "  按 Space 开启原生鼠标交互"
                } else {
                    "  Press Space to enable tmux mouse"
                },
                dim,
            )),
            Line::from(Span::styled("  ~/.tmux.conf untouched", dim)),
        ]
    };
    build_art_box(
        w,
        Line::from(Span::styled(
            if enabled {
                " tmux mouse ON "
            } else {
                " tmux mouse OFF "
            },
            hi,
        )),
        body,
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_tmux_history_limit(
    limit: u32,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let enabled = limit > 0;
    let hi = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let filled = match limit {
        0 => 1,
        1..=10_000 => 6,
        10_001..=50_000 => 14,
        _ => 22,
    };
    let bar_w = 22_usize;
    let fill_ch = if theme.ascii { "#" } else { "█" };
    let empty_ch = if theme.ascii { "-" } else { "░" };
    let bar = format!(
        "  [{}{}]",
        fill_ch.repeat(filled),
        empty_ch.repeat(bar_w.saturating_sub(filled))
    );
    let label = if limit == 0 {
        if zh {
            "  默认 2,000 行 (未托管)"
        } else {
            "  default 2,000 lines (off)"
        }
        .to_owned()
    } else {
        format!("  set -g history-limit {limit}")
    };
    let w = usize::from(art_w).clamp(32, 44);
    build_art_box(
        w,
        Line::from(Span::styled(" history-limit ", hi)),
        vec![
            Line::from(Span::styled(
                label,
                if enabled {
                    theme.style(theme.success).add_modifier(Modifier::BOLD)
                } else {
                    theme.style(theme.muted)
                },
            )),
            Line::from(Span::styled(bar, hi)),
            Line::from(Span::styled(
                "  2K      10K     50K    100K",
                theme.style(theme.muted),
            )),
        ],
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_tmux_base_index(
    enabled: bool,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let w = usize::from(art_w).clamp(32, 44);
    let body = if enabled {
        vec![
            Line::from(Span::styled("  [1][2][3][4] ... [9]", hi)),
            Line::from(Span::styled(
                if zh {
                    "   ^ 第 1 个窗口就在左手边!"
                } else {
                    "   ^ First window right on key 1!"
                },
                theme.style(theme.success).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  base-index 1 & pane-base-index 1",
                theme.style(theme.muted),
            )),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                "  [1][2][3] ... [9][0]",
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                if zh {
                    "                    ^ 默认 0 在最右"
                } else {
                    "                    ^ default 0 far right"
                },
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                "  base-index 0 (tmux default)",
                theme.style(theme.muted),
            )),
        ]
    };
    build_art_box(
        w,
        Line::from(Span::styled(
            if enabled {
                " base-index = 1 "
            } else {
                " base-index = 0 "
            },
            hi,
        )),
        body,
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_tmux_renumber(
    enabled: bool,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let arrow = if theme.ascii { "->" } else { "→" };
    let w = usize::from(art_w).clamp(32, 44);
    let body = if enabled {
        vec![
            Line::from(Span::styled(
                format!("  [1][2][3]  kill 2 {arrow}  [1][2]"),
                hi,
            )),
            Line::from(Span::styled(
                if zh {
                    "  自动消除断号，窗口序永远紧凑"
                } else {
                    "  Auto-compacts gaps on window close"
                },
                theme.style(theme.success).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  set -g renumber-windows on",
                theme.style(theme.muted),
            )),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                format!("  [1][2][3]  kill 2 {arrow}  [1] _ [3]"),
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                if zh {
                    "  关闭后保留编号空洞 (默认)"
                } else {
                    "  Leaves numbering gaps (default)"
                },
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                "  renumber-windows unmanaged",
                theme.style(theme.muted),
            )),
        ]
    };
    build_art_box(
        w,
        Line::from(Span::styled(
            if enabled {
                " renumber-windows ON "
            } else {
                " renumber-windows OFF "
            },
            hi,
        )),
        body,
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_tmux_escape_time(
    enabled: bool,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = if enabled {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.warning)
    };
    let w = usize::from(art_w).clamp(32, 44);
    let body = if enabled {
        vec![
            Line::from(Span::styled("  Esc latency: [|] 0 ms", hi)),
            Line::from(Span::styled(
                if zh {
                    "  Vim / Neovim 切模式即刻响应!"
                } else {
                    "  Instant Vim / Neovim Esc response!"
                },
                theme.style(theme.text),
            )),
            Line::from(Span::styled(
                "  set -sg escape-time 0",
                theme.style(theme.muted),
            )),
        ]
    } else {
        vec![
            Line::from(Span::styled("  Esc latency: [##########] 500 ms", hi)),
            Line::from(Span::styled(
                if zh {
                    "  默认 500ms 延迟可能让 Vim 发粘"
                } else {
                    "  Default 500ms pause feels laggy"
                },
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                "  escape-time unmanaged",
                theme.style(theme.muted),
            )),
        ]
    };
    build_art_box(
        w,
        Line::from(Span::styled(
            if enabled {
                " escape-time = 0ms "
            } else {
                " escape-time = default "
            },
            hi,
        )),
        body,
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_live_stream(
    enabled: bool,
    art_w: u16,
    _art_h: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = if enabled {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let ok = if enabled {
        theme.style(theme.success).add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.muted)
    };
    let w = usize::from(art_w).clamp(32, 44);
    let body = if enabled {
        vec![
            Line::from(Span::styled(
                if theme.ascii {
                    "  [*LIVE] _.-=+*#@  200ms stream"
                } else if zh {
                    "  [● 实时] ▁▂▃▄▅▆▇█  自动流式刷新"
                } else {
                    "  [● LIVE] ▁▂▃▄▅▆▇█  auto-streaming"
                },
                ok,
            )),
            Line::from(Span::styled(
                if zh {
                    "  Home 预览 + Exposé 全景墙实时跳动"
                } else if theme.ascii {
                    "  Home preview + Expose wall live"
                } else {
                    "  Home preview + Exposé wall live"
                },
                theme.style(theme.text),
            )),
            Line::from(Span::styled(
                if zh {
                    "  Inline 模式空闲零字节不干扰滚动"
                } else {
                    "  Inline mode stays 0 idle bytes"
                },
                theme.style(theme.muted),
            )),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                if theme.ascii {
                    "  [o] manual capture on switch"
                } else if zh {
                    "  [○] 仅切换窗口时按需抓取"
                } else {
                    "  [○] capture on window switch"
                },
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                if zh {
                    "  按 Space 开启实时流式预览"
                } else {
                    "  Press Space for live streaming"
                },
                theme.style(theme.muted),
            )),
            Line::from(Span::styled(
                "  live_stream = false",
                theme.style(theme.muted),
            )),
        ]
    };
    build_art_box(
        w,
        Line::from(Span::styled(
            if enabled {
                " live_stream = true "
            } else {
                " live_stream = false "
            },
            hi,
        )),
        body,
        None,
        hi,
        theme.ascii,
    )
}

fn config_art_tmux_reload(art_w: u16, _art_h: u16, theme: Theme) -> Vec<Line<'static>> {
    let zh = theme.language.is_zh() && !theme.ascii;
    let hi = theme.style(theme.accent).add_modifier(Modifier::BOLD);
    let ok = theme.style(theme.success).add_modifier(Modifier::BOLD);
    let arrow = if theme.ascii { "->" } else { "→" };
    let w = usize::from(art_w).clamp(32, 44);
    build_art_box(
        w,
        Line::from(Span::styled(" source-file ~/.tmux.conf ", hi)),
        vec![
            Line::from(Span::styled(
                format!("  ~/.tmux.conf {arrow} tmux server"),
                hi,
            )),
            Line::from(Span::styled(
                if zh {
                    "  按 Enter 立即热重载，会话不中断"
                } else {
                    "  Press Enter to live-reload safely"
                },
                ok,
            )),
            Line::from(Span::styled(
                if zh {
                    "  全部运行中窗格保持存活"
                } else {
                    "  All running panes stay untouched"
                },
                theme.style(theme.muted),
            )),
        ],
        None,
        hi,
        theme.ascii,
    )
}

#[cfg(test)]
mod tests {
    use super::super::{
        ConfigItemView, DockZone, HomeDetails, HomeTelemetry, SubviewContent, ThemePreset,
        WorkspaceHomeLayout, WorkspaceHomeSpec, WorkspaceSubviewSpec,
        render_pane_preview_card_interactive, render_workspace_home, render_workspace_subview,
        tests::buffer_logical_text,
    };
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn assert_ascii_and_no_color(term: &Terminal<TestBackend>, label: &str) {
        let text = term.backend().to_string();
        assert!(
            text.chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in {label}:\n{text}"
        );
        let bad_cell = term
            .backend()
            .buffer()
            .content()
            .iter()
            .enumerate()
            .find(|(_, c)| c.fg != Color::Reset || c.bg != Color::Reset);
        assert!(
            bad_cell.is_none(),
            "color emitted under no_color in {label}: {bad_cell:?}"
        );
    }

    #[test]
    fn upgraded_preview_card_highlights_search_renders_badges_and_clickable_hits() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let lines = vec![
            "compiling justmux v0.3.0".into(),
            "\x1b[32mok\x1b[0m found ERROR in worker".into(),
            "second error line".into(),
        ];
        let preview = PanePreview {
            title: "📺 Window Preview",
            subtitle: "dev · 0:build",
            lines: &lines,
            scroll: 0,
            search_query: Some("error"),
            search_active: true,
            match_count: 2,
            current_match: 1,
            pane_index: Some((2, 3)),
            zoomed: true,
            synchronized: true,
            piping: true,
            show_actions: true,
            ..PanePreview::default()
        };

        let mut term = Terminal::new(TestBackend::new(96, 10)).expect("preview term");
        let mut layout = PreviewCardLayout::default();
        term.draw(|frame| {
            layout = render_pane_preview_card_interactive(frame, frame.area(), preview, aurora);
        })
        .expect("draw preview");

        let out = buffer_logical_text(term.backend().buffer());
        for needle in [
            "ZOOM",
            "SYNC",
            "PIPE",
            "Pane 2/3",
            "[f Search]",
            "[y Copy]",
            "[Log]",
            "Search:",
            "1/2",
        ] {
            assert!(out.contains(needle), "missing {needle} in:\n{out}");
        }
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::PrevPane)
        );
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::NextPane)
        );
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::Search)
        );
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::Copy)
        );
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::ExportLog)
        );
        assert!(
            layout
                .action_hits
                .iter()
                .any(|(_, a)| *a == PreviewAction::TogglePipe)
        );

        // Matched substring cells carry the warning highlight background.
        let buf = term.backend().buffer();
        let has_highlight = buf
            .content()
            .iter()
            .any(|c| c.bg == aurora.warning && c.symbol() == "E");
        assert!(has_highlight, "expected highlighted 'E' in ERROR");

        // ZH + ASCII/NO_COLOR checks.
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        term.draw(|frame| {
            render_pane_preview_card_interactive(frame, frame.area(), preview, zh);
        })
        .expect("draw zh preview");
        let zh_out = buffer_logical_text(term.backend().buffer());
        for needle in [
            "缩放",
            "同步",
            "录制",
            "窗格 2/3",
            "[f 搜索]",
            "[y 复制]",
            "搜索:",
        ] {
            assert!(zh_out.contains(needle), "missing ZH {needle}:\n{zh_out}");
        }

        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let mut nc_term = Terminal::new(TestBackend::new(96, 10)).expect("nc preview term");
        nc_term
            .draw(|frame| {
                render_pane_preview_card_interactive(frame, frame.area(), preview, ascii_nc);
            })
            .expect("draw ascii preview");
        assert_ascii_and_no_color(&nc_term, "preview card");
    }

    #[test]
    fn home_chrome_shows_clickable_detach_when_inside_tmux_and_multi_client_badge() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let mut term = Terminal::new(TestBackend::new(110, 28)).expect("home term");
        let mut layout = WorkspaceHomeLayout::default();
        term.draw(|frame| {
            layout = render_workspace_home(
                frame,
                frame.area(),
                &TextInput::default(),
                WorkspaceHomeSpec {
                    telemetry: HomeTelemetry {
                        sessions: Some(2),
                        windows: Some(4),
                        attached: Some(1),
                        clients: Some(3),
                        hint_badge: None,
                    },
                    inside_tmux: true,
                    ..WorkspaceHomeSpec::default()
                },
                aurora,
                |_, _| {},
            );
        })
        .expect("draw inside tmux home");
        let text = buffer_logical_text(term.backend().buffer());
        assert!(text.contains("Detach"), "missing Detach pill:\n{text}");
        assert!(text.contains("👥 3"), "missing multi-client badge:\n{text}");
        assert!(
            layout
                .quick_action_hits
                .iter()
                .any(|(_, a)| *a == HomeQuickAction::Detach)
        );

        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let mut zh_term = Terminal::new(TestBackend::new(110, 28)).expect("zh home term");
        zh_term
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec {
                        inside_tmux: true,
                        ..WorkspaceHomeSpec::default()
                    },
                    zh,
                    |_, _| {},
                );
            })
            .expect("draw zh home");
        assert!(buffer_logical_text(zh_term.backend().buffer()).contains("分离"));

        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let mut nc_term = Terminal::new(TestBackend::new(110, 28)).expect("nc home term");
        nc_term
            .draw(|frame| {
                render_workspace_home(
                    frame,
                    frame.area(),
                    &TextInput::default(),
                    WorkspaceHomeSpec {
                        telemetry: HomeTelemetry {
                            sessions: Some(2),
                            windows: Some(4),
                            attached: Some(1),
                            clients: Some(3),
                            hint_badge: None,
                        },
                        inside_tmux: true,
                        ..WorkspaceHomeSpec::default()
                    },
                    ascii_nc,
                    |_, _| {},
                );
            })
            .expect("draw ascii home");
        assert_ascii_and_no_color(&nc_term, "home inside_tmux");
        let asc_text = nc_term.backend().to_string();
        assert!(asc_text.contains("[Detach]"));
        assert!(asc_text.contains("3c"));
    }

    #[test]
    fn layout_studio_clients_buffers_snapshots_and_target_picker_render_across_modes() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };

        let clients = [
            ClientRowView {
                name: "/dev/ttys001".into(),
                session_name: "dev".into(),
                width: 140,
                height: 42,
                tty: "/dev/ttys001".into(),
                readonly: false,
                is_current: true,
            },
            ClientRowView {
                name: "/dev/ttys002".into(),
                session_name: "dev".into(),
                width: 80,
                height: 24,
                tty: "/dev/ttys002".into(),
                readonly: true,
                is_current: false,
            },
        ];
        let buffers = [BufferRowView {
            name: "buffer0000".into(),
            size: 1536,
            sample: "cargo test --all".into(),
        }];
        let buf_lines = vec!["cargo test --all".into(), "test result: ok.".into()];
        let snapshots = [SnapshotRowView {
            name: "backend-dev".into(),
            created_label: "2026-11-23 14:00".into(),
            session_count: 2,
            window_count: 5,
            pane_count: 8,
        }];
        let tree_lines = vec![
            "dev (/Users/demo/project)".into(),
            "  ├─ 0: editor [main-vertical]".into(),
            "  │   ├─ %1 nvim".into(),
            "  │   └─ %2 cargo watch".into(),
        ];
        let picker_rows = [RichPickerRow {
            primary: "prod (3w)".into(),
            primary_indices: vec![],
            secondary: "$2".into(),
            badge: Some(("attached".into(), aurora.success)),
            is_action: false,
        }];

        let draw_spec = |w: u16, h: u16, theme: Theme, content: SubviewContent<'_>| {
            let mut term = Terminal::new(TestBackend::new(w, h)).expect("term");
            let mut layout = WorkspaceSubviewLayout::default();
            term.draw(|frame| {
                layout = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content,
                        ..WorkspaceSubviewSpec::default()
                    },
                    theme,
                );
            })
            .expect("draw subview");
            (term, layout)
        };

        // 1. LayoutStudio (EN, ZH, ASCII/NO_COLOR, compact)
        let (t, l) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::LayoutStudio(LayoutStudioView {
                target_label: "dev:0 editor",
                selected_preset: 3,
                pane_size: (120, 40),
                pane_count: 3,
                resize_step: 5,
                zoomed: true,
                synchronized: true,
                status_note: None,
                hovered_sash: None,
                dragging_sash: None,
            }),
        );
        let text = buffer_logical_text(t.backend().buffer());
        for needle in [
            "Even Horizontal",
            "Main Vertical",
            "Tiled",
            "120×40",
            "SYNC: ON",
        ] {
            assert!(text.contains(needle), "missing {needle}:\n{text}");
        }
        assert!(l.controller_hits.len() >= 11);
        assert!(
            l.sash_hits
                .iter()
                .any(|(_, id)| *id == SashId::LayoutResize(false))
        );

        let (tzh, _) = draw_spec(
            120,
            32,
            zh,
            SubviewContent::LayoutStudio(LayoutStudioView {
                target_label: "dev:0 editor",
                selected_preset: 0,
                pane_size: (80, 24),
                pane_count: 2,
                resize_step: 5,
                zoomed: false,
                synchronized: false,
                status_note: None,
                hovered_sash: None,
                dragging_sash: None,
            }),
        );
        assert!(buffer_logical_text(tzh.backend().buffer()).contains("水平均分"));
        let (tnc, _) = draw_spec(
            120,
            32,
            ascii_nc,
            SubviewContent::LayoutStudio(LayoutStudioView {
                target_label: "dev:0",
                selected_preset: 1,
                pane_size: (80, 24),
                pane_count: 2,
                resize_step: 5,
                zoomed: false,
                synchronized: true,
                status_note: None,
                hovered_sash: Some(SashId::LayoutResize(false)),
                dragging_sash: None,
            }),
        );
        assert_ascii_and_no_color(&tnc, "layout studio");
        let _ = draw_spec(
            32,
            10,
            aurora,
            SubviewContent::LayoutStudio(LayoutStudioView {
                target_label: "dev:0",
                selected_preset: 0,
                pane_size: (80, 24),
                pane_count: 2,
                resize_step: 5,
                zoomed: false,
                synchronized: false,
                status_note: None,
                hovered_sash: None,
                dragging_sash: None,
            }),
        );

        // 2. Clients (EN, ZH, ASCII/NO_COLOR, compact)
        let (tc, lc) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::Clients(ClientsView {
                clients: &clients,
                selected: 0,
                size_mismatch_warning: false, // auto-detected from 140x42 vs 80x24!
                status_note: None,
            }),
        );
        let ctext = buffer_logical_text(tc.backend().buffer());
        for needle in [
            "Detach Other Clients",
            "dotted borders",
            "(this client)",
            "RO",
            "140×42",
        ] {
            assert!(ctext.contains(needle), "missing {needle}:\n{ctext}");
        }
        assert!(lc.list_rect.is_some());
        assert!(
            lc.controller_hits
                .iter()
                .any(|(_, a)| *a == ControllerAction::DetachOtherClients)
        );
        let (tczh, _) = draw_spec(
            120,
            32,
            zh,
            SubviewContent::Clients(ClientsView {
                clients: &clients,
                selected: 0,
                size_mismatch_warning: true,
                status_note: None,
            }),
        );
        assert!(buffer_logical_text(tczh.backend().buffer()).contains("踢出其他端恢复全屏"));
        let (tcnc, _) = draw_spec(
            120,
            32,
            ascii_nc,
            SubviewContent::Clients(ClientsView {
                clients: &clients,
                selected: 0,
                size_mismatch_warning: true,
                status_note: None,
            }),
        );
        assert_ascii_and_no_color(&tcnc, "clients");
        let _ = draw_spec(
            32,
            10,
            aurora,
            SubviewContent::Clients(ClientsView {
                clients: &clients,
                selected: 0,
                size_mismatch_warning: true,
                status_note: None,
            }),
        );

        // 3. Buffers (EN, ZH, ASCII/NO_COLOR, compact)
        let (tb, lb) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::Buffers(BuffersView {
                buffers: &buffers,
                selected: 0,
                preview_lines: &buf_lines,
                preview_scroll: 0,
                status_note: None,
            }),
        );
        let btext = buffer_logical_text(tb.backend().buffer());
        for needle in [
            "buffer0000",
            "1.5 KB",
            "Paste to Pane",
            "Copy to System",
            "New Buffer",
        ] {
            assert!(btext.contains(needle), "missing {needle}:\n{btext}");
        }
        assert!(lb.list_rect.is_some() && lb.preview_area.is_some());
        let (tbnc, _) = draw_spec(
            120,
            32,
            ascii_nc,
            SubviewContent::Buffers(BuffersView {
                buffers: &buffers,
                selected: 0,
                preview_lines: &buf_lines,
                preview_scroll: 0,
                status_note: None,
            }),
        );
        assert_ascii_and_no_color(&tbnc, "buffers");
        let _ = draw_spec(
            32,
            10,
            zh,
            SubviewContent::Buffers(BuffersView {
                buffers: &buffers,
                selected: 0,
                preview_lines: &buf_lines,
                preview_scroll: 0,
                status_note: None,
            }),
        );

        // 4. Snapshots (EN, ZH, ASCII/NO_COLOR, compact)
        let (ts, ls) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::Snapshots(SnapshotsView {
                snapshots: &snapshots,
                selected: 0,
                tree_preview: &tree_lines,
                preview_scroll: 0,
                status_note: None,
            }),
        );
        let stext = buffer_logical_text(ts.backend().buffer());
        for needle in [
            "backend-dev",
            "2s·5w·8p",
            "Save Current Workspace",
            "Restore",
            "cargo watch",
        ] {
            assert!(stext.contains(needle), "missing {needle}:\n{stext}");
        }
        assert!(ls.list_rect.is_some() && ls.preview_area.is_some());
        let (tsnc, _) = draw_spec(
            120,
            32,
            ascii_nc,
            SubviewContent::Snapshots(SnapshotsView {
                snapshots: &snapshots,
                selected: 0,
                tree_preview: &tree_lines,
                preview_scroll: 0,
                status_note: None,
            }),
        );
        assert_ascii_and_no_color(&tsnc, "snapshots");

        // 5. TargetPicker & DualForm
        let query = TextInput::default();
        let (tp, lp) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::TargetPicker {
                title: "Move Window",
                action_banner: "Move window dev:1 logs -> select destination session",
                query: &query,
                rows: &picker_rows,
                selected: 0,
                empty: "No target sessions",
                preview: None,
            },
        );
        assert!(buffer_logical_text(tp.backend().buffer()).contains("Move window dev:1 logs"));
        assert!(lp.list_rect.is_some());

        let f1 = TextInput::new("logs");
        let f2 = TextInput::new("/tmp/work");
        let (tf, _) = draw_spec(
            120,
            32,
            aurora,
            SubviewContent::DualForm {
                title: "New window",
                first_label: "Window name (optional)",
                first_input: &f1,
                second_label: "Working directory (optional)",
                second_input: &f2,
                active_field: 1,
                dir_exists: false,
                auto_name: false,
                completions: &[],
                completion_selected: 0,
                completion_open: false,
            },
        );
        let ftext = buffer_logical_text(tf.backend().buffer());
        assert!(ftext.contains("logs") && ftext.contains("/tmp/work"));
    }

    #[test]
    fn tmux_comfort_config_diagrams_render_in_en_zh_and_ascii_no_color() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let items = [
            ConfigItemView {
                key: "tmux_mouse",
                value: "true".into(),
                description: "",
            },
            ConfigItemView {
                key: "tmux_history_limit",
                value: "50000".into(),
                description: "",
            },
            ConfigItemView {
                key: "tmux_base_index_one",
                value: "true".into(),
                description: "",
            },
            ConfigItemView {
                key: "tmux_renumber_windows",
                value: "true".into(),
                description: "",
            },
            ConfigItemView {
                key: "tmux_zero_escape_time",
                value: "true".into(),
                description: "",
            },
            ConfigItemView {
                key: "live_stream",
                value: "true".into(),
                description: "",
            },
            ConfigItemView {
                key: "tmux_reload",
                value: "run".into(),
                description: "",
            },
        ];
        for idx in 0..items.len() {
            for (theme, check_ascii) in [(aurora, false), (zh, false), (ascii_nc, true)] {
                let mut term = Terminal::new(TestBackend::new(120, 32)).expect("cfg term");
                term.draw(|frame| {
                    render_workspace_subview(
                        frame,
                        frame.area(),
                        &WorkspaceSubviewSpec {
                            details: HomeDetails::default(),
                            telemetry: HomeTelemetry::default(),
                            emblem_step: 0,
                            emblem_mode: EmblemMode::Spin,
                            footer_hint: "",
                            content: SubviewContent::Config {
                                path_label: "/tmp/config.toml",
                                items: &items,
                                selected: idx,
                                status_note: None,
                            },
                            ..WorkspaceSubviewSpec::default()
                        },
                        theme,
                    );
                })
                .expect("draw comfort config");
                if check_ascii {
                    assert_ascii_and_no_color(&term, items[idx].key);
                }
            }
        }
    }

    #[test]
    fn window_status_badges_respect_unicode_ascii_and_no_color() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let flags = WindowStatusFlags {
            zoomed: true,
            synchronized: true,
            activity: false,
            bell: true,
            silence: false,
            dead: true,
            multi_clients: 2,
        };
        let uni: String = window_status_badge_spans(flags, aurora)
            .into_iter()
            .map(|s| s.content.into_owned())
            .collect();
        for needle in ["⛶", "⚡SYNC", "🔔", "✕", "👥 2"] {
            assert!(uni.contains(needle), "missing {needle} in {uni}");
        }
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            ..aurora
        };
        let asc_spans = window_status_badge_spans(flags, ascii_nc);
        let asc: String = asc_spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(asc.is_ascii(), "non-ASCII badges: {asc}");
        for needle in ["[Z]", "[!SYNC]", "[!]", "[X]", "2c"] {
            assert!(asc.contains(needle), "missing {needle} in {asc}");
        }
        assert!(
            asc_spans
                .iter()
                .all(|s| s.style.fg.is_none() && s.style.bg.is_none())
        );
    }

    #[test]
    fn buffers_preview_scrolls_top_down_and_hides_dead_action_pills() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let buffers = [BufferRowView {
            name: "buffer0000".into(),
            size: 1024,
            sample: "BUF_LINE_01".into(),
        }];
        let preview_lines: Vec<String> = (1..=40).map(|i| format!("BUF_LINE_{i:02}")).collect();

        let render_buffers = |scroll: usize, theme: Theme| {
            let mut term = Terminal::new(TestBackend::new(100, 18)).expect("buf term");
            term.draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Buffers(BuffersView {
                            buffers: &buffers,
                            selected: 0,
                            preview_lines: &preview_lines,
                            preview_scroll: scroll,
                            status_note: None,
                        }),
                        ..WorkspaceSubviewSpec::default()
                    },
                    theme,
                );
            })
            .expect("draw buffers");
            term
        };

        // scroll = 0 starts at the TOP (BUF_LINE_01 visible, BUF_LINE_40 hidden, no dead pills).
        let t0 = render_buffers(0, aurora);
        let text0 = buffer_logical_text(t0.backend().buffer());
        assert!(text0.contains("BUF_LINE_01"), "expected top line:\n{text0}");
        assert!(
            !text0.contains("BUF_LINE_40"),
            "scroll=0 must not start at bottom:\n{text0}"
        );
        for dead_pill in ["[f Search]", "[y Copy]", "[Log]", "[Pipe]"] {
            assert!(
                !text0.contains(dead_pill),
                "unexpected dead pill {dead_pill} in /buffers:\n{text0}"
            );
        }

        // scroll = 6 shifts down past BUF_LINE_01..06 to reveal BUF_LINE_07 + scroll badge.
        let t6 = render_buffers(6, aurora);
        let text6 = buffer_logical_text(t6.backend().buffer());
        assert!(
            text6.contains("BUF_LINE_07"),
            "expected scrolled line:\n{text6}"
        );
        assert!(
            !text6.contains("BUF_LINE_02"),
            "scrolled past top lines:\n{text6}"
        );
        assert!(
            text6.contains("↓ +6"),
            "expected scroll indicator:\n{text6}"
        );

        // ASCII + NO_COLOR preserves top-down scroll and strict invariants.
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let tnc = render_buffers(6, ascii_nc);
        assert_ascii_and_no_color(&tnc, "buffers scrolled");
        assert!(tnc.backend().to_string().contains("[+6/"));

        // Direct PanePreview with show_actions: false must render zero action pills/hits.
        let mut pterm = Terminal::new(TestBackend::new(80, 10)).expect("pterm");
        let mut playout = PreviewCardLayout::default();
        pterm
            .draw(|frame| {
                playout = render_pane_preview_card_interactive(
                    frame,
                    frame.area(),
                    PanePreview {
                        title: "Preview",
                        subtitle: "dev",
                        lines: &preview_lines,
                        scroll: 0,
                        show_actions: false,
                        ..PanePreview::default()
                    },
                    aurora,
                );
            })
            .expect("draw preview without actions");
        let ptext = buffer_logical_text(pterm.backend().buffer());
        assert!(playout.action_hits.is_empty());
        assert!(!ptext.contains("[f Search]"));
        assert!(!ptext.contains("[y Copy]"));
        assert!(!ptext.contains("[Log]"));
    }

    #[test]
    fn desktop_preview_card_renders_pip_controls_scrollbar_selection_and_dock() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let lines: Vec<String> = (0..40)
            .map(|i| format!("line-{i:02} compiling module alpha"))
            .collect();
        let match_lines = [5_usize, 18_usize];
        let err_lines = [35_usize];

        let preview = PanePreview {
            title: "Window Preview",
            subtitle: "dev · 0:build",
            lines: &lines,
            scroll: 6,
            show_actions: true,
            pip_active: true,
            hovered_action: Some(PreviewAction::TogglePip),
            scrollbar_hovered: true,
            scrollbar_dragging: false,
            search_match_lines: &match_lines,
            error_lines: &err_lines,
            selection_range: Some(((1, 8), (1, 16))),
            dock_highlight: None,
            ..PanePreview::default()
        };

        let mut term = Terminal::new(TestBackend::new(100, 12)).expect("desktop preview term");
        let mut layout = PreviewCardLayout::default();
        term.draw(|frame| {
            layout = render_pane_preview_card_interactive(frame, frame.area(), preview, aurora);
        })
        .expect("draw desktop preview");

        assert!(layout.content_rect.is_some());
        let track = layout.scrollbar_track_rect.expect("scrollbar track");
        let thumb = layout.scrollbar_thumb_rect.expect("scrollbar thumb");
        assert_eq!(track.width, 1);
        assert!(thumb.y >= track.y && thumb.bottom() <= track.bottom());

        for act in [
            PreviewAction::TogglePip,
            PreviewAction::Zoom,
            PreviewAction::Rename,
            PreviewAction::Close,
            PreviewAction::SendEsc,
            PreviewAction::Search,
            PreviewAction::Copy,
        ] {
            assert!(
                layout.action_hits.iter().any(|(_, a)| *a == act),
                "missing preview action hit {act:?}"
            );
        }

        let buf = term.backend().buffer();
        let text = buffer_logical_text(buf);
        for needle in ["LIVE INPUT", "[Send Esc]", "[⌨]", "[⛶]", "[✎]", "[✕]"] {
            assert!(text.contains(needle), "missing {needle}:\n{text}");
        }
        // Selection range applies accent background on visible row 1 (`content_rect.y + 1`).
        let content_rect = layout.content_rect.expect("content rect");
        assert_eq!(
            buf[(content_rect.x + 10, content_rect.y + 1)].bg,
            aurora.accent,
            "expected drag selection highlight on visible row 1"
        );
        // Error tick renders in error color along the scrollbar column.
        let has_err_tick = (track.y..track.bottom()).any(|y| buf[(track.x, y)].fg == aurora.error);
        assert!(has_err_tick, "expected error tick on VS Code scrollbar");

        // ZH + Dock overlay + ASCII/NO_COLOR invariants.
        let dock_preview = PanePreview {
            dock_highlight: Some(DockZone::Bottom),
            ..preview
        };
        let mut zh_term = Terminal::new(TestBackend::new(100, 12)).expect("zh dock term");
        zh_term
            .draw(|frame| {
                render_pane_preview_card_interactive(frame, frame.area(), dock_preview, zh);
            })
            .expect("draw zh dock");
        let zh_out = buffer_logical_text(zh_term.backend().buffer());
        assert!(zh_out.contains("下方分屏 50%") && zh_out.contains("[发送 Esc]"));

        let mut nc_term = Terminal::new(TestBackend::new(100, 12)).expect("nc desktop preview");
        nc_term
            .draw(|frame| {
                render_pane_preview_card_interactive(frame, frame.area(), dock_preview, ascii_nc);
            })
            .expect("draw nc desktop preview");
        assert_ascii_and_no_color(&nc_term, "desktop preview card");
    }
}
