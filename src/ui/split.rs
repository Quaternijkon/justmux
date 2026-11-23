//! Interactive 2x2 Visual Split Studio canvas, toolbar, and window filler widgets.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::{
    DockZone, EmblemMode, SashId, Theme, WorkspaceSubviewLayout, parse_ansi_preview_line,
    pill_span, render_dock_zone_overlay, render_horizontal_divider_line,
    render_hydra_emblem_if_some, render_sash_divider, single_line, split_lower_band_for_hydra,
    truncate_to_width,
};

/// Clickable action pill displayed inside the interactive 2x2 Split Studio toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitToolbarButton {
    SplitHorizontal,
    SplitVertical,
    FlipDirection,
    RatioPreset,
    SyncToggle,
    PresetGrid,
    BreakOut,
    RemoveSlot,
    Reset,
    ApplyAndEnter,
}

/// Leaf pane rendered inside the 2x2 Split Studio canvas with optional live capture preview.
#[derive(Debug, Clone)]
pub struct SplitCanvasPaneView<'a> {
    pub slot_id: usize,
    pub ordinal: usize,
    pub title: String,
    pub subtitle: String,
    pub selected: bool,
    pub is_new_shell: bool,
    pub preview_lines: &'a [String],
    pub preview_scroll: usize,
}

/// Recursive 2x2 split tree rendered onto the Split Studio canvas.
#[derive(Debug, Clone)]
pub enum SplitCanvasNode<'a> {
    Pane(SplitCanvasPaneView<'a>),
    Split {
        horizontal: bool,
        /// Percentage of space allocated to `first` child (`20..=80`; `0` defaults to `50`).
        ratio: u16,
        first: Box<SplitCanvasNode<'a>>,
        second: Box<SplitCanvasNode<'a>>,
    },
}

/// Selectable tmux window (or `+ New shell`) row shown in the Split Studio filler list.
#[derive(Debug, Clone)]
pub struct SplitWindowRowView {
    pub primary: String,
    pub detail: String,
    pub active_in_session: bool,
    pub assigned_slot: Option<usize>,
    pub is_new_shell: bool,
}

/// View model for the interactive 2x2 Split Studio (`super::SubviewContent::Split`).
#[derive(Debug, Clone)]
pub struct SplitStudioView<'a> {
    pub root: SplitCanvasNode<'a>,
    pub selected_slot: usize,
    pub selected_ordinal: usize,
    pub slot_count: usize,
    pub can_split_h: bool,
    pub can_split_v: bool,
    pub can_flip: bool,
    pub can_remove: bool,
    /// Active split ratio percentage (`20..=80`, e.g. `50`, `70`, `30`; `0` displays `50`).
    pub active_ratio: u16,
    /// Whether ratio presets/adjustments can be applied (`slot_count >= 2`).
    pub can_adjust_ratio: bool,
    /// Whether `synchronize-panes` is currently toggled ON for this split layout.
    pub sync_panes: bool,
    /// Whether the selected pane can be broken out into its own window.
    pub can_break_out: bool,
    /// True when editing an existing multi-pane window instead of composing a new split.
    pub editing_existing: bool,
    pub window_rows: &'a [SplitWindowRowView],
    pub window_cursor: usize,
    pub hovered_sash: Option<SashId>,
    pub dragging_sash: Option<SashId>,
    pub hovered_dock: Option<(usize, DockZone)>,
}

pub(super) fn render_subview_split(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    studio: &SplitStudioView<'_>,
    emblem_step: usize,
    emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.height < 14 {
        return render_compact_split_studio(frame, card_inner, studio, theme);
    }
    let lower_h = if card_inner.height >= 24 {
        (card_inner.height / 3).clamp(9, 12)
    } else {
        card_inner.height.saturating_sub(10).clamp(8, 10)
    };
    let [upper_band, rule_area, lower_band] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(1),
        Constraint::Length(lower_h),
    ])
    .areas(card_inner);
    let show_sync_banner = studio.sync_panes && upper_band.height >= 7;
    let (canvas_area, toolbar_area) = if show_sync_banner {
        let [banner_area, c_area, t_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .areas(upper_band);
        let zh = theme.language.is_zh() && !theme.ascii;
        let banner_text = if theme.ascii {
            " [!SYNC ON] Typing broadcasts simultaneously to EVERY pane in this window "
        } else if zh {
            " ⚡ 同步输入已开启 — 按键将同时广播到本窗口的所有窗格 "
        } else {
            " ⚡ SYNC PANES ON — typing broadcasts simultaneously to EVERY pane in this window "
        };
        let banner_span = if theme.no_color {
            Span::styled(
                truncate_to_width(banner_text, usize::from(banner_area.width)),
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            )
        } else {
            pill_span(
                truncate_to_width(banner_text, usize::from(banner_area.width)),
                theme.warning,
                theme.pill_bg(),
                theme,
                true,
            )
        };
        frame.render_widget(Paragraph::new(Line::from(banner_span)), banner_area);
        (c_area, t_area)
    } else {
        let [c_area, t_area] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(1)]).areas(upper_band);
        (c_area, t_area)
    };

    let mut slot_hits = Vec::new();
    let mut sash_hits = Vec::new();
    render_split_canvas_node(
        frame,
        canvas_area,
        &studio.root,
        0,
        studio,
        &mut slot_hits,
        &mut sash_hits,
        theme,
    );
    let toolbar_hits = render_split_toolbar(frame, toolbar_area, studio, theme);

    render_horizontal_divider_line(frame, rule_area, theme);
    let (left_band, hydra_box) = split_lower_band_for_hydra(lower_band);
    let list_rect = render_split_window_list(frame, left_band, studio, theme);
    let hydra_rendered =
        render_hydra_emblem_if_some(frame, hydra_box, emblem_step, emblem_mode, theme);

    WorkspaceSubviewLayout {
        list_rect: Some(list_rect),
        hydra_rendered,
        split_slot_hits: slot_hits,
        split_toolbar_hits: toolbar_hits,
        sash_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_split_studio(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: &SplitStudioView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 12 || area.height < 5 {
        return WorkspaceSubviewLayout::default();
    }
    let mut slot_hits = Vec::new();
    let mut sash_hits = Vec::new();
    let (list_rect, toolbar_hits) = if area.width >= 54 && area.height >= 7 {
        let [left_col, right_col] =
            Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
                .areas(area);
        let [canvas_area, toolbar_area] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(1)]).areas(left_col);
        render_split_canvas_node(
            frame,
            canvas_area,
            &studio.root,
            0,
            studio,
            &mut slot_hits,
            &mut sash_hits,
            theme,
        );
        let toolbar_hits = render_split_toolbar(frame, toolbar_area, studio, theme);
        let list_rect = render_split_window_list(frame, right_col, studio, theme);
        (list_rect, toolbar_hits)
    } else if area.height <= 6 {
        let [canvas_area, toolbar_area, list_band] = Layout::vertical([
            Constraint::Min(2),
            Constraint::Length(1),
            Constraint::Min(2),
        ])
        .areas(area);
        render_split_canvas_node(
            frame,
            canvas_area,
            &studio.root,
            0,
            studio,
            &mut slot_hits,
            &mut sash_hits,
            theme,
        );
        let toolbar_hits = render_split_toolbar(frame, toolbar_area, studio, theme);
        let list_rect = render_split_window_list(frame, list_band, studio, theme);
        (list_rect, toolbar_hits)
    } else {
        let [canvas_area, toolbar_area, list_band] = Layout::vertical([
            Constraint::Min(4),
            Constraint::Length(1),
            Constraint::Min(2),
        ])
        .areas(area);
        render_split_canvas_node(
            frame,
            canvas_area,
            &studio.root,
            0,
            studio,
            &mut slot_hits,
            &mut sash_hits,
            theme,
        );
        let toolbar_hits = render_split_toolbar(frame, toolbar_area, studio, theme);
        let list_rect = render_split_window_list(frame, list_band, studio, theme);
        (list_rect, toolbar_hits)
    };
    WorkspaceSubviewLayout {
        list_rect: Some(list_rect),
        split_slot_hits: slot_hits,
        split_toolbar_hits: toolbar_hits,
        sash_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn render_split_canvas_node(
    frame: &mut Frame<'_>,
    area: Rect,
    node: &SplitCanvasNode<'_>,
    node_idx: usize,
    studio: &SplitStudioView<'_>,
    slot_hits: &mut Vec<(Rect, usize)>,
    sash_hits: &mut Vec<(Rect, SashId)>,
    theme: Theme,
) {
    match node {
        SplitCanvasNode::Split {
            horizontal,
            ratio,
            first,
            second,
        } => {
            let first_pct = if *ratio == 0 {
                50
            } else {
                (*ratio).clamp(20, 80)
            };
            let sash_id = SashId::SplitNode(node_idx);
            let hovered = studio.hovered_sash == Some(sash_id);
            let dragging = studio.dragging_sash == Some(sash_id);
            let first_idx = node_idx.saturating_mul(2).saturating_add(1);
            let second_idx = node_idx.saturating_mul(2).saturating_add(2);

            if *horizontal && area.width >= 11 {
                let avail = area.width.saturating_sub(1);
                let first_w = (((u32::from(avail) * u32::from(first_pct) + 50) / 100) as u16)
                    .clamp(4, avail.saturating_sub(4).max(4));
                let second_w = avail.saturating_sub(first_w);
                let [a, sash, b] = Layout::horizontal([
                    Constraint::Length(first_w),
                    Constraint::Length(1),
                    Constraint::Length(second_w),
                ])
                .areas(area);
                render_split_canvas_node(
                    frame, a, first, first_idx, studio, slot_hits, sash_hits, theme,
                );
                render_split_canvas_node(
                    frame, b, second, second_idx, studio, slot_hits, sash_hits, theme,
                );
                render_sash_divider(frame, sash, true, first_pct, hovered, dragging, theme);
                sash_hits.push((sash, sash_id));
            } else if !*horizontal && area.height >= 7 {
                let avail = area.height.saturating_sub(1);
                let first_h = (((u32::from(avail) * u32::from(first_pct) + 50) / 100) as u16)
                    .clamp(3, avail.saturating_sub(3).max(3));
                let second_h = avail.saturating_sub(first_h);
                let [a, sash, b] = Layout::vertical([
                    Constraint::Length(first_h),
                    Constraint::Length(1),
                    Constraint::Length(second_h),
                ])
                .areas(area);
                render_split_canvas_node(
                    frame, a, first, first_idx, studio, slot_hits, sash_hits, theme,
                );
                render_split_canvas_node(
                    frame, b, second, second_idx, studio, slot_hits, sash_hits, theme,
                );
                render_sash_divider(frame, sash, false, first_pct, hovered, dragging, theme);
                sash_hits.push((sash, sash_id));
            } else {
                let dir = if *horizontal {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                };
                let second_pct = 100_u16.saturating_sub(first_pct);
                let [a, b] = Layout::default()
                    .direction(dir)
                    .constraints([
                        Constraint::Percentage(first_pct),
                        Constraint::Percentage(second_pct),
                    ])
                    .areas(area);
                render_split_canvas_node(
                    frame, a, first, first_idx, studio, slot_hits, sash_hits, theme,
                );
                render_split_canvas_node(
                    frame, b, second, second_idx, studio, slot_hits, sash_hits, theme,
                );
            }
        }
        SplitCanvasNode::Pane(pane) => {
            slot_hits.push((area, pane.slot_id));
            render_split_canvas_pane(frame, area, pane, theme);
            if let Some((target_slot, zone)) = studio.hovered_dock
                && target_slot == pane.slot_id
            {
                render_dock_zone_overlay(frame, area, zone, theme);
            }
        }
    }
}

fn render_split_canvas_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &SplitCanvasPaneView<'_>,
    theme: Theme,
) {
    if area.width < 4 || area.height < 2 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let border_style = if pane.selected {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    } else if pane.is_new_shell {
        theme.style(theme.muted)
    } else {
        theme.style(theme.secondary)
    };
    let border_type = if theme.ascii {
        BorderType::Plain
    } else if pane.selected {
        BorderType::Thick
    } else {
        BorderType::Rounded
    };

    let inner_h = usize::from(area.height.saturating_sub(2));
    let total = pane.preview_lines.len();
    let max_scroll = total.saturating_sub(inner_h);
    let actual_scroll = pane.preview_scroll.min(max_scroll);
    let scroll_badge = (!pane.is_new_shell && inner_h > 0 && actual_scroll > 0).then(|| {
        if theme.ascii {
            format!(" [+{actual_scroll}] ")
        } else {
            format!(" ↑ +{actual_scroll} ")
        }
    });

    let tag = if pane.selected {
        let star = if theme.ascii { "*" } else { "★" };
        format!(" #{} {star} ", pane.ordinal)
    } else {
        format!(" #{} ", pane.ordinal)
    };
    let raw_label = if pane.is_new_shell {
        theme.tr("+ New shell", "+ 新建终端").to_owned()
    } else {
        single_line(&theme.adapt(&pane.title))
    };
    let top_inner_w = usize::from(area.width.saturating_sub(2));
    let badge_reserve = scroll_badge
        .as_ref()
        .filter(|b| tag.width() + b.width() + 4 <= top_inner_w)
        .map_or(0, |b| b.width() + 1);
    let max_title_w = usize::from(area.width.saturating_sub(3)).saturating_sub(badge_reserve);
    let label_budget = max_title_w.saturating_sub(tag.width());
    let title_label = truncate_to_width(&raw_label, label_budget);
    let mut title_spans = vec![Span::styled(
        tag,
        if pane.selected {
            theme.style(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            theme.style(theme.secondary).add_modifier(Modifier::BOLD)
        },
    )];
    if !title_label.is_empty() {
        title_spans.push(Span::styled(
            format!("{title_label} "),
            if pane.selected {
                theme.style(theme.text).add_modifier(Modifier::BOLD)
            } else if pane.is_new_shell {
                theme.style(theme.muted)
            } else {
                theme.style(theme.text)
            },
        ));
    }

    let left_used_before_sub: usize = title_spans.iter().map(Span::width).sum();
    let raw_sub = single_line(&theme.adapt(pane.subtitle.trim()));
    if !raw_sub.is_empty() {
        let badge_needed = scroll_badge
            .as_ref()
            .filter(|b| left_used_before_sub + b.width() <= top_inner_w)
            .map_or(0, |b| b.width() + 1);
        let sub_avail = top_inner_w.saturating_sub(left_used_before_sub + badge_needed + 1);
        let min_needed = if raw_sub.width() <= 1 { 4 } else { 5 };
        if sub_avail >= min_needed {
            let trimmed = truncate_to_width(&raw_sub, sub_avail.saturating_sub(3));
            if !trimmed.is_empty() && (trimmed.width() >= 2 || raw_sub.width() <= 1) {
                title_spans.push(Span::styled(
                    format!("[{trimmed}] "),
                    theme.style(theme.muted),
                ));
            }
        }
    }
    let left_used: usize = title_spans.iter().map(Span::width).sum();

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title(Line::from(title_spans));

    if let Some(badge) = scroll_badge
        && left_used + badge.width() <= top_inner_w
    {
        block = block.title_top(
            Line::from(Span::styled(
                badge,
                theme.style(theme.accent).add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let inner_w = usize::from(inner.width);
    if pane.is_new_shell {
        let mut lines = vec![Line::from(Span::styled(
            truncate_to_width(
                if pane.selected {
                    if zh {
                        "+ 新建终端窗格 (当前选中)"
                    } else {
                        "+ New shell pane (selected)"
                    }
                } else if zh {
                    "+ 新建终端窗格"
                } else {
                    "+ New shell pane"
                },
                inner_w,
            ),
            theme
                .style(if pane.selected {
                    theme.success
                } else {
                    theme.muted
                })
                .add_modifier(if pane.selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ))];
        if inner.height >= 2 {
            lines.push(Line::from(Span::styled(
                truncate_to_width(
                    if zh {
                        "在下方选择窗口填充，或保留为空白新建终端"
                    } else {
                        "Fill from list below, h/v split, or Enter to launch"
                    },
                    inner_w,
                ),
                theme.style(theme.muted),
            )));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    } else if pane.preview_lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(
                    theme.tr(" (empty terminal buffer)", " (空终端缓冲区)"),
                    inner_w,
                ),
                theme.style(theme.muted),
            ))),
            inner,
        );
    } else {
        let end = total.saturating_sub(actual_scroll);
        let start = end.saturating_sub(inner_h);
        let lines: Vec<Line<'static>> = pane.preview_lines[start..end]
            .iter()
            .map(|raw| parse_ansi_preview_line(raw, inner_w, theme))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn render_split_toolbar(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: &SplitStudioView<'_>,
    theme: Theme,
) -> Vec<(Rect, SplitToolbarButton)> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let total_w = usize::from(area.width);
    let ratio = if studio.active_ratio == 0 {
        50
    } else {
        studio.active_ratio.clamp(20, 80)
    };
    let ratio_full = format!(" [p] {ratio}/{} ", 100 - ratio);
    let ratio_med = format!("[p]{ratio}/{}", 100 - ratio);
    let ratio_short = format!(" [p]{ratio}% ");

    let sync_full = if studio.sync_panes {
        if theme.ascii {
            " [y] !SYNC ON ".to_owned()
        } else if zh {
            " [y] ⚡同步开 ".to_owned()
        } else {
            " [y] ⚡SYNC ON ".to_owned()
        }
    } else if zh {
        " [y] 同步关 ".to_owned()
    } else {
        " [y] Sync ".to_owned()
    };
    let sync_med = if studio.sync_panes {
        if theme.ascii {
            "[y]!SYNC".to_owned()
        } else if zh {
            "[y]⚡同步".to_owned()
        } else {
            "[y]⚡SYNC".to_owned()
        }
    } else if zh {
        "[y]同步".to_owned()
    } else {
        "[y]Sync".to_owned()
    };
    let sync_short = if studio.sync_panes {
        if theme.ascii { " [y]!S " } else { " [y]⚡ " }
    } else {
        " [y] "
    };

    struct Spec {
        btn: SplitToolbarButton,
        enabled: bool,
        emphasized: bool,
        color: ratatui::style::Color,
        full: String,
        med: String,
        short: String,
    }

    let mut left_specs = vec![
        Spec {
            btn: SplitToolbarButton::SplitHorizontal,
            enabled: studio.can_split_h,
            emphasized: false,
            color: theme.accent,
            full: if zh {
                " [h] 左右分 "
            } else {
                " [h] Split H "
            }
            .into(),
            med: if zh { "[h]左右分" } else { "[h] Split H" }.into(),
            short: " [h] H ".into(),
        },
        Spec {
            btn: SplitToolbarButton::SplitVertical,
            enabled: studio.can_split_v,
            emphasized: false,
            color: theme.accent,
            full: if zh {
                " [v] 上下分 "
            } else {
                " [v] Split V "
            }
            .into(),
            med: if zh { "[v]上下分" } else { "[v] Split V" }.into(),
            short: " [v] V ".into(),
        },
        Spec {
            btn: SplitToolbarButton::FlipDirection,
            enabled: studio.can_flip,
            emphasized: false,
            color: theme.accent,
            full: if zh { " [t] 翻转 " } else { " [t] Flip " }.into(),
            med: if zh { "[t] 翻转" } else { "[t] Flip" }.into(),
            short: " [t] ".into(),
        },
        Spec {
            btn: SplitToolbarButton::RatioPreset,
            enabled: studio.can_adjust_ratio,
            emphasized: false,
            color: theme.secondary,
            full: ratio_full,
            med: ratio_med,
            short: ratio_short,
        },
        Spec {
            btn: SplitToolbarButton::SyncToggle,
            enabled: true,
            emphasized: studio.sync_panes,
            color: if studio.sync_panes {
                theme.warning
            } else {
                theme.secondary
            },
            full: sync_full,
            med: sync_med,
            short: sync_short.into(),
        },
        Spec {
            btn: SplitToolbarButton::PresetGrid,
            enabled: true,
            emphasized: false,
            color: theme.secondary,
            full: if zh { " [g] 预设 " } else { " [g] Grid " }.into(),
            med: if zh { "[g]预设" } else { "[g]Grid" }.into(),
            short: " [g] ".into(),
        },
    ];
    if studio.editing_existing {
        left_specs.push(Spec {
            btn: SplitToolbarButton::BreakOut,
            enabled: studio.can_break_out,
            emphasized: false,
            color: theme.warning,
            full: if zh { " [b] 拆出 " } else { " [b] Break " }.into(),
            med: if zh { "[b]拆出" } else { "[b]Break" }.into(),
            short: " [b] ".into(),
        });
    }
    left_specs.push(Spec {
        btn: SplitToolbarButton::RemoveSlot,
        enabled: studio.can_remove,
        emphasized: false,
        color: theme.warning,
        full: if zh { " [x] 移除 " } else { " [x] Remove " }.into(),
        med: if zh { "[x]移除" } else { "[x]Del" }.into(),
        short: if zh { " [x] 删 " } else { " [x] Del " }.into(),
    });
    left_specs.push(Spec {
        btn: SplitToolbarButton::Reset,
        enabled: true,
        emphasized: false,
        color: theme.secondary,
        full: if zh { " [r] 重置 " } else { " [r] Reset " }.into(),
        med: if zh { "[r]重置" } else { "[r]Reset" }.into(),
        short: " [r] ".into(),
    });

    let cta_full = if zh {
        " [Enter] 确认进入 tmux "
    } else {
        " [Enter] Apply & Enter "
    };
    let cta_compact = if zh {
        " [Enter] 进入 "
    } else {
        " [Enter] Apply "
    };
    let cta_micro = " [Enter] ";

    let full_left_w: usize = left_specs.iter().map(|s| s.full.width()).sum::<usize>()
        + left_specs.len().saturating_sub(1);
    let med_left_w: usize = left_specs.iter().map(|s| s.med.width()).sum::<usize>()
        + left_specs.len().saturating_sub(1);
    let short_left_w: usize = left_specs.iter().map(|s| s.short.width()).sum::<usize>()
        + left_specs.len().saturating_sub(1);

    let (tier, cta_label) = if full_left_w + 1 + cta_full.width() <= total_w {
        (0_u8, cta_full)
    } else if med_left_w + 1 + cta_full.width() <= total_w {
        (1_u8, cta_full)
    } else if short_left_w + 1 + cta_full.width() <= total_w {
        (2_u8, cta_full)
    } else if cta_compact.width() <= total_w {
        (2_u8, cta_compact)
    } else {
        (2_u8, cta_micro)
    };
    let cta_w = cta_label.width();

    let bg = theme.pill_bg();
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut col = 0_usize;

    // Always reserve room for the Enter CTA so secondary actions truncate first on narrow bars.
    let max_left_col = if cta_w <= total_w {
        total_w.saturating_sub(cta_w + 1)
    } else {
        0
    };

    for spec in left_specs {
        let label = match tier {
            0 => spec.full.as_str(),
            1 => spec.med.as_str(),
            _ => spec.short.as_str(),
        };
        let w = label.width();
        let gap = usize::from(col > 0);
        if col + gap + w > max_left_col {
            break;
        }
        if gap > 0 {
            spans.push(Span::raw(" "));
            col += 1;
        }
        spans.push(if !spec.enabled {
            Span::styled(theme.adapt(label), theme.style(theme.muted))
        } else if spec.emphasized && theme.no_color {
            Span::styled(
                theme.adapt(label),
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            )
        } else {
            pill_span(label, spec.color, bg, theme, spec.emphasized)
        });
        hits.push((
            Rect {
                x: area.x + u16::try_from(col).unwrap_or(0),
                y: area.y,
                width: u16::try_from(w).unwrap_or(0),
                height: 1,
            },
            spec.btn,
        ));
        col += w;
    }

    if col + cta_w <= total_w {
        let gap = total_w.saturating_sub(col + cta_w);
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
            col += gap;
        }
        let cta_span = if theme.no_color {
            Span::styled(
                theme.adapt(cta_label),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            pill_span(cta_label, theme.success, bg, theme, true)
        };
        spans.push(cta_span);
        hits.push((
            Rect {
                x: area.x + u16::try_from(col).unwrap_or(0),
                y: area.y,
                width: u16::try_from(cta_w).unwrap_or(0),
                height: 1,
            },
            SplitToolbarButton::ApplyAndEnter,
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    hits
}

fn render_split_window_list(
    frame: &mut Frame<'_>,
    area: Rect,
    studio: &SplitStudioView<'_>,
    theme: Theme,
) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head_area, list_area] = if area.height >= 2 {
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area)
    } else {
        return area;
    };

    let heading = if zh {
        format!(
            "为窗格 #{} 选择填充窗口 ({}/4 格 · 点击或 ↑↓/Space 分配)",
            studio.selected_ordinal, studio.slot_count
        )
    } else {
        format!(
            "Fill Pane #{} with window ({}/4 panes · Click or ↑↓/Space)",
            studio.selected_ordinal, studio.slot_count
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_to_width(&theme.adapt(&heading), usize::from(head_area.width)),
            theme.style(theme.secondary).add_modifier(Modifier::BOLD),
        ))),
        head_area,
    );

    let total = studio.window_rows.len();
    if total == 0 {
        return list_area;
    }
    let clamped = studio.window_cursor.min(total - 1);
    let bg = theme.pill_bg();
    let list_w = usize::from(list_area.width);
    let items: Vec<ListItem<'_>> = studio
        .window_rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_sel = i == clamped;
            let marker = if is_sel { theme.marker() } else { " " };
            let icon = if row.is_new_shell {
                "+"
            } else if row.active_in_session {
                if theme.ascii { "*" } else { "●" }
            } else if theme.ascii {
                "o"
            } else {
                "○"
            };
            let badge = row.assigned_slot.map(|slot_ord| {
                let text = if slot_ord == studio.selected_ordinal {
                    if zh {
                        format!(" [ #{} 当前 ] ", slot_ord)
                    } else {
                        format!(" [ #{} active ] ", slot_ord)
                    }
                } else if zh {
                    format!(" [ #{} · Space 互换 ] ", slot_ord)
                } else {
                    format!(" [ #{} · Space swap ] ", slot_ord)
                };
                let fg = if slot_ord == studio.selected_ordinal {
                    theme.accent
                } else {
                    theme.secondary
                };
                (text, fg)
            });

            let prefix = format!("{marker} {icon} ");
            let badge_w = badge.as_ref().map_or(0, |(t, _)| t.width() + 1);
            let avail = list_w.saturating_sub(prefix.width() + badge_w);
            let adapted_primary = theme.adapt(&row.primary);
            let primary_text = truncate_to_width(&adapted_primary, avail);
            let used = primary_text.width();
            let detail_text = if !row.detail.is_empty() && used + 4 <= avail {
                let sep = if theme.ascii { " | " } else { " · " };
                let rest = avail.saturating_sub(used + sep.width());
                let adapted_detail = theme.adapt(&row.detail);
                let trimmed = truncate_to_width(&adapted_detail, rest);
                if trimmed.is_empty() {
                    String::new()
                } else {
                    format!("{sep}{trimmed}")
                }
            } else {
                String::new()
            };

            let base_style = if is_sel {
                theme.selected_style()
            } else if row.is_new_shell {
                theme.style(theme.success)
            } else {
                theme.style(theme.text)
            };
            let mut spans = vec![
                Span::styled(prefix, base_style),
                Span::styled(primary_text, base_style),
            ];
            if !detail_text.is_empty() {
                spans.push(Span::styled(
                    detail_text,
                    if is_sel {
                        base_style
                    } else {
                        theme.style(theme.muted)
                    },
                ));
            }
            if let Some((badge_text, badge_fg)) = badge {
                let content_w: usize = spans.iter().map(Span::width).sum();
                let target_badge_w = badge_text.width();
                if content_w + 1 + target_badge_w <= list_w {
                    let gap = list_w.saturating_sub(content_w + target_badge_w);
                    spans.push(Span::raw(" ".repeat(gap)));
                    spans.push(pill_span(badge_text, badge_fg, bg, theme, is_sel));
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(clamped));
    frame.render_stateful_widget(List::new(items), list_area, &mut state);
    list_area
}

#[cfg(test)]
mod tests {
    use super::super::{
        HomeDetails, HomeTelemetry, Language, SubviewContent, ThemePreset, WorkspaceSubviewSpec,
        render_workspace_subview, styled_key_hint_spans,
        tests::{buffer_logical_text, buffer_row_logical},
    };
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    fn pane_view<'a>(
        slot_id: usize,
        ordinal: usize,
        title: &str,
        subtitle: &str,
        preview_lines: &'a [String],
        selected: bool,
        is_new_shell: bool,
    ) -> SplitCanvasNode<'a> {
        SplitCanvasNode::Pane(SplitCanvasPaneView {
            slot_id,
            ordinal,
            title: title.into(),
            subtitle: subtitle.into(),
            selected,
            is_new_shell,
            preview_lines,
            preview_scroll: 0,
        })
    }

    fn sample_window_rows() -> Vec<SplitWindowRowView> {
        vec![
            SplitWindowRowView {
                primary: "dev:0 editor".into(),
                detail: "nvim".into(),
                active_in_session: true,
                assigned_slot: Some(1),
                is_new_shell: false,
            },
            SplitWindowRowView {
                primary: "dev:1 server".into(),
                detail: "cargo".into(),
                active_in_session: false,
                assigned_slot: Some(2),
                is_new_shell: false,
            },
            SplitWindowRowView {
                primary: "+ New shell".into(),
                detail: "Open fresh shell in this slot".into(),
                active_in_session: false,
                assigned_slot: None,
                is_new_shell: true,
            },
        ]
    }

    #[test]
    fn split_studio_subview_renders_2x2_hit_boxes_zh_and_ascii_no_color() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let details = HomeDetails {
            directory: Some("project/justmux"),
            context: "git main",
            session: Some("dev"),
            status: None,
        };
        let telemetry = HomeTelemetry::default();
        let edit_lines = vec!["EDIT_LINE_1".into(), "EDIT_LINE_2".into()];
        let srv_lines = vec!["SRV_LINE_1".into()];
        let logs_lines = vec!["LOGS_LINE_1".into()];
        let window_rows = sample_window_rows();

        let render_case = |root: SplitCanvasNode<'_>,
                           slot_count: usize,
                           can_h: bool,
                           can_v: bool,
                           theme: Theme| {
            let mut term = Terminal::new(TestBackend::new(120, 32)).expect("split term");
            let mut layout = WorkspaceSubviewLayout::default();
            term.draw(|frame| {
                layout = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details,
                        telemetry,
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Split(SplitStudioView {
                            root,
                            selected_slot: 1,
                            selected_ordinal: 1,
                            slot_count,
                            can_split_h: can_h,
                            can_split_v: can_v,
                            can_flip: slot_count >= 2,
                            can_remove: slot_count > 1,
                            active_ratio: 50,
                            can_adjust_ratio: slot_count >= 2,
                            sync_panes: false,
                            can_break_out: false,
                            editing_existing: false,
                            window_rows: &window_rows,
                            window_cursor: 0,
                            hovered_sash: None,
                            dragging_sash: None,
                            hovered_dock: None,
                        }),
                        ..WorkspaceSubviewSpec::default()
                    },
                    theme,
                );
            })
            .expect("draw split studio");
            (term, layout)
        };

        let (t1, l1) = render_case(
            pane_view(1, 1, "dev:0 editor", "nvim", &edit_lines, true, false),
            1,
            true,
            true,
            aurora,
        );
        assert_eq!(l1.split_slot_hits.len(), 1);
        assert!(l1.list_rect.is_some());
        assert!(
            l1.split_toolbar_hits
                .iter()
                .any(|(_, b)| *b == SplitToolbarButton::ApplyAndEnter)
        );
        assert!(
            l1.split_toolbar_hits
                .iter()
                .any(|(_, b)| *b == SplitToolbarButton::FlipDirection)
        );
        let text_1 = buffer_logical_text(t1.backend().buffer());
        assert!(text_1.contains("[t] Flip"));
        assert!(text_1.contains("[nvim]"));

        let (_, l2) = render_case(
            SplitCanvasNode::Split {
                horizontal: true,
                ratio: 50,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(pane_view(
                    2,
                    2,
                    "dev:1 server",
                    "cargo",
                    &srv_lines,
                    false,
                    false,
                )),
            },
            2,
            false,
            true,
            aurora,
        );
        assert_eq!(l2.split_slot_hits.len(), 2);

        let (_, l3) = render_case(
            SplitCanvasNode::Split {
                horizontal: true,
                ratio: 50,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(SplitCanvasNode::Split {
                    horizontal: false,
                    ratio: 50,
                    first: Box::new(pane_view(
                        2,
                        2,
                        "dev:1 server",
                        "cargo",
                        &srv_lines,
                        false,
                        false,
                    )),
                    second: Box::new(pane_view(
                        3,
                        3,
                        "dev:2 logs",
                        "tail",
                        &logs_lines,
                        false,
                        false,
                    )),
                }),
            },
            3,
            false,
            true,
            aurora,
        );
        assert_eq!(l3.split_slot_hits.len(), 3);

        let grid_2x2 = || SplitCanvasNode::Split {
            horizontal: true,
            ratio: 50,
            first: Box::new(SplitCanvasNode::Split {
                horizontal: false,
                ratio: 50,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(pane_view(
                    2,
                    2,
                    "dev:1 server",
                    "cargo",
                    &srv_lines,
                    false,
                    false,
                )),
            }),
            second: Box::new(SplitCanvasNode::Split {
                horizontal: false,
                ratio: 50,
                first: Box::new(pane_view(
                    3,
                    3,
                    "dev:2 logs",
                    "tail",
                    &logs_lines,
                    false,
                    false,
                )),
                second: Box::new(pane_view(4, 4, "+ New shell", "", &[], false, true)),
            }),
        };

        let (t4, l4) = render_case(grid_2x2(), 4, false, false, aurora);
        assert_eq!(l4.split_slot_hits.len(), 4);
        assert!(l4.list_rect.is_some());
        let text_2x2 = buffer_logical_text(t4.backend().buffer());
        for needle in [
            "#1 ★",
            "#2",
            "#3",
            "#4",
            "[nvim]",
            "[cargo]",
            "[tail]",
            "[t] Flip",
            "EDIT_LINE_1",
            "SRV_LINE_1",
            "LOGS_LINE_1",
            "+ New shell pane",
        ] {
            assert!(text_2x2.contains(needle), "missing {needle}:\n{text_2x2}");
        }

        let aurora_zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let (tzh, lzh) = render_case(grid_2x2(), 4, false, false, aurora_zh);
        assert_eq!(lzh.split_slot_hits.len(), 4);
        let text_zh = buffer_logical_text(tzh.backend().buffer());
        for needle in ["左右分", "上下分", "[t] 翻转", "确认进入 tmux", "为窗格 #1"]
        {
            assert!(text_zh.contains(needle), "missing {needle}:\n{text_zh}");
        }

        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let (tnc, _) = render_case(grid_2x2(), 4, false, false, ascii_nc);
        let text_nc = tnc.backend().to_string();
        assert!(
            text_nc
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in split ascii/no_color:\n{text_nc}"
        );
        assert!(
            tnc.backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
    }

    #[test]
    fn split_pane_subtitle_renders_muted_and_preserves_scroll_badge() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let preview: Vec<String> = (1..=30).map(|i| format!("output line {i}")).collect();
        let pane = SplitCanvasPaneView {
            slot_id: 1,
            ordinal: 1,
            title: "dev:0 editor".into(),
            subtitle: "nvim".into(),
            selected: true,
            is_new_shell: false,
            preview_lines: &preview,
            preview_scroll: 5,
        };

        let mut wide = Terminal::new(TestBackend::new(60, 8)).expect("wide pane");
        wide.draw(|frame| render_split_canvas_pane(frame, frame.area(), &pane, aurora))
            .expect("draw wide pane");
        let wbuf = wide.backend().buffer();
        let (row, byte_to_col) = buffer_row_logical(wbuf, 0);
        assert!(row.contains("dev:0 editor"), "missing title: {row}");
        assert!(row.contains("[nvim]"), "missing subtitle tag: {row}");
        assert!(row.contains("↑ +5"), "missing scroll badge: {row}");
        let sub_idx = row.find("[nvim]").expect("subtitle index");
        assert_eq!(wbuf[(byte_to_col[sub_idx], 0)].fg, aurora.muted);

        let mut tight = Terminal::new(TestBackend::new(24, 8)).expect("tight pane");
        tight
            .draw(|frame| render_split_canvas_pane(frame, frame.area(), &pane, aurora))
            .expect("draw tight pane");
        let tight_row = buffer_row_logical(tight.backend().buffer(), 0).0;
        assert!(
            tight_row.contains("↑ +5"),
            "badge dropped on tight pane: {tight_row}"
        );
        assert!(
            !tight_row.contains("[nvim]"),
            "subtitle should yield on tight width: {tight_row}"
        );
    }

    #[test]
    fn split_toolbar_reserves_apply_cta_down_to_36_cols_and_handles_flip_state() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let window_rows = sample_window_rows();
        let edit_lines = vec!["EDIT_LINE_1".into()];

        let make_studio = |can_flip: bool| SplitStudioView {
            root: SplitCanvasNode::Split {
                horizontal: true,
                ratio: 50,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(pane_view(2, 2, "+ New shell", "", &[], false, true)),
            },
            selected_slot: 1,
            selected_ordinal: 1,
            slot_count: 2,
            can_split_h: false,
            can_split_v: true,
            can_flip,
            can_remove: true,
            active_ratio: 50,
            can_adjust_ratio: true,
            sync_panes: false,
            can_break_out: false,
            editing_existing: false,
            window_rows: &window_rows,
            window_cursor: 0,
            hovered_sash: None,
            dragging_sash: None,
            hovered_dock: None,
        };

        let mut term36 = Terminal::new(TestBackend::new(36, 12)).expect("36 col term");
        let mut layout36 = WorkspaceSubviewLayout::default();
        term36
            .draw(|frame| {
                layout36 = render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Split(make_studio(true)),
                        ..WorkspaceSubviewSpec::default()
                    },
                    aurora,
                );
            })
            .expect("draw 36 cols");
        let text36 = buffer_logical_text(term36.backend().buffer());
        assert!(
            text36.contains("[Enter] Apply"),
            "missing compact CTA at 36 cols:\n{text36}"
        );
        assert!(
            layout36
                .split_toolbar_hits
                .iter()
                .any(|(_, b)| *b == SplitToolbarButton::ApplyAndEnter)
        );

        let mut compact_en = Terminal::new(TestBackend::new(36, 10)).expect("36 col en compact");
        let mut hits_en = Vec::new();
        compact_en
            .draw(|frame| {
                hits_en =
                    render_compact_split_studio(frame, frame.area(), &make_studio(false), aurora)
                        .split_toolbar_hits;
            })
            .expect("draw en compact");
        let text_en = buffer_logical_text(compact_en.backend().buffer());
        assert!(
            text_en.contains("[Enter] Apply"),
            "missing EN compact CTA:\n{text_en}"
        );
        assert!(
            hits_en
                .iter()
                .any(|(_, b)| *b == SplitToolbarButton::ApplyAndEnter)
        );

        let mut wide_disabled = Terminal::new(TestBackend::new(120, 32)).expect("wide disabled");
        wide_disabled
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Split(make_studio(false)),
                        ..WorkspaceSubviewSpec::default()
                    },
                    aurora,
                );
            })
            .expect("draw wide disabled");
        let wbuf = wide_disabled.backend().buffer();
        let (flip_y, flip_row, flip_cols) = (0..wbuf.area.height)
            .find_map(|y| {
                let (row, cols) = buffer_row_logical(wbuf, y);
                row.contains("[t] Flip").then_some((y, row, cols))
            })
            .expect("flip row");
        let flip_idx = flip_row.find("[t] Flip").expect("flip idx");
        assert_eq!(wbuf[(flip_cols[flip_idx], flip_y)].fg, aurora.muted);

        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
        let mut compact_zh = Terminal::new(TestBackend::new(36, 10)).expect("36 col zh compact");
        let mut hits_zh = Vec::new();
        compact_zh
            .draw(|frame| {
                hits_zh = render_compact_split_studio(frame, frame.area(), &make_studio(false), zh)
                    .split_toolbar_hits;
            })
            .expect("draw zh compact");
        let text_zh = buffer_logical_text(compact_zh.backend().buffer());
        assert!(
            text_zh.contains("[Enter] 进入"),
            "missing ZH compact CTA:\n{text_zh}"
        );
        assert!(
            hits_zh
                .iter()
                .any(|(_, b)| *b == SplitToolbarButton::ApplyAndEnter)
        );
    }

    #[test]
    fn split_keycap_hints_highlight_all_keys_en_and_zh_without_double_enter() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let key_fg = aurora.keycap_fg();
        let key_bg = aurora.keycap_bg().expect("aurora keycap bg");
        let key_style = aurora.keycap_style();
        let window_rows = sample_window_rows();
        let edit_lines = vec!["EDIT_LINE_1".into()];

        let expected_keys = [
            "1-4", "Tab", "h", "v", "t", "x", "r", "↑↓", "Space", "Enter", "Esc",
        ];
        let key_anchors_en = [
            ("1-4", "1-4/Tab"),
            ("Tab", "1-4/Tab"),
            ("h", "h/v Split"),
            ("v", "v Split"),
            ("t", "t Flip"),
            ("x", "x Del"),
            ("r", "r Reset"),
            ("↑↓", "↑↓ Window"),
            ("Space", "Space Swap"),
            ("Enter", "Enter Launch"),
            ("Esc", "Esc Back"),
        ];
        let key_anchors_zh = [
            ("1-4", "1-4/Tab"),
            ("Tab", "1-4/Tab"),
            ("h", "h/v 分屏"),
            ("v", "v 分屏"),
            ("t", "t 翻转"),
            ("x", "x 移除"),
            ("r", "r 重置"),
            ("↑↓", "↑↓ 选窗口"),
            ("Space", "Space 交换"),
            ("Enter", "Enter 进入"),
            ("Esc", "Esc 返回"),
        ];

        for (lang, expected_plain_word, anchors) in [
            (Language::En, "Launch", &key_anchors_en),
            (Language::Zh, "进入", &key_anchors_zh),
        ] {
            let theme = Theme {
                language: lang,
                ..aurora
            };
            let mut term = Terminal::new(TestBackend::new(120, 32)).expect("term");
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
                        content: SubviewContent::Split(SplitStudioView {
                            root: pane_view(1, 1, "dev:0 editor", "nvim", &edit_lines, true, false),
                            selected_slot: 1,
                            selected_ordinal: 1,
                            slot_count: 1,
                            can_split_h: true,
                            can_split_v: true,
                            can_flip: false,
                            can_remove: false,
                            active_ratio: 50,
                            can_adjust_ratio: false,
                            sync_panes: false,
                            can_break_out: false,
                            editing_existing: false,
                            window_rows: &window_rows,
                            window_cursor: 0,
                            hovered_sash: None,
                            dragging_sash: None,
                            hovered_dock: None,
                        }),
                        ..WorkspaceSubviewSpec::default()
                    },
                    theme,
                );
            })
            .expect("draw split");

            let buf = term.backend().buffer();
            let (hint_y, hint_row, byte_to_col) = (0..buf.area.height)
                .find_map(|y| {
                    let (row, cols) = buffer_row_logical(buf, y);
                    row.contains("1-4/Tab").then_some((y, row, cols))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "missing split shortcut hint in buffer:\n{}",
                        buffer_logical_text(buf)
                    )
                });
            assert!(
                hint_row.contains(expected_plain_word),
                "hint missing {expected_plain_word:?}: {hint_row}"
            );
            assert_eq!(
                hint_row.matches("Enter").count(),
                1,
                "expected single Enter in hint: {hint_row}"
            );

            for (key, anchor) in anchors {
                let anchor_start = hint_row
                    .find(anchor)
                    .unwrap_or_else(|| panic!("missing anchor {anchor:?} in {hint_row}"));
                let rel = anchor
                    .find(key)
                    .unwrap_or_else(|| panic!("key {key:?} missing from {anchor:?}"));
                let start = anchor_start + rel;
                let c0 = byte_to_col[start];
                let c1 = byte_to_col[start + key.len() - 1];
                for x in c0..=c1 {
                    let cell = &buf[(x, hint_y)];
                    assert_eq!(cell.fg, key_fg, "key {key:?} col {x} fg in {hint_row}");
                    assert_eq!(cell.bg, key_bg, "key {key:?} col {x} bg in {hint_row}");
                    assert!(
                        cell.modifier.contains(Modifier::BOLD),
                        "key {key:?} col {x} not bold"
                    );
                }
            }

            let slash1 = hint_row.find("1-4/Tab").expect("1-4/Tab") + "1-4".len();
            let slash2 = hint_row.find("h/v").expect("h/v") + 1;
            for pos in [slash1, slash2] {
                let cell = &buf[(byte_to_col[pos], hint_y)];
                assert_ne!(cell.fg, key_fg);
                assert_eq!(cell.bg, Color::Reset);
                assert!(!cell.modifier.contains(Modifier::BOLD));
            }

            let plain_idx = hint_row.find(expected_plain_word).expect("plain word");
            let p0 = byte_to_col[plain_idx];
            let p1 = byte_to_col[plain_idx + expected_plain_word.len() - 1];
            for x in p0..=p1 {
                let cell = &buf[(x, hint_y)];
                assert_ne!(cell.fg, key_fg);
                assert_eq!(cell.bg, Color::Reset);
                assert!(!cell.modifier.contains(Modifier::BOLD));
            }

            let direct_hint = if lang == Language::Zh {
                "1-4/Tab 选窗格 · h/v 分屏 · t 翻转 · x 移除 · r 重置 · ↑↓ 选窗口 · Space 交换 · Enter 进入 · Esc 返回"
            } else {
                "1-4/Tab Pane · h/v Split · t Flip · x Del · r Reset · ↑↓ Window · Space Swap · Enter Launch · Esc Back"
            };
            let base = theme.style(theme.muted);
            let spans = styled_key_hint_spans(direct_hint, base, theme);
            let keys: Vec<&str> = spans
                .iter()
                .filter(|s| s.style == key_style)
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(keys, expected_keys);
            assert!(
                spans
                    .iter()
                    .filter(|s| s.style != key_style)
                    .all(|s| s.style == base),
                "non-key spans altered style: {spans:?}"
            );
        }
    }

    #[test]
    fn split_studio_renders_asymmetric_ratios_sync_warning_and_new_toolbar_buttons() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let window_rows = sample_window_rows();
        let edit_lines = vec!["EDIT_LINE_1".into()];
        let srv_lines = vec!["SRV_LINE_1".into()];

        let make_studio = |ratio: u16, sync_panes: bool, editing_existing: bool| SplitStudioView {
            root: SplitCanvasNode::Split {
                horizontal: true,
                ratio,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(pane_view(
                    2,
                    2,
                    "dev:1 server",
                    "cargo",
                    &srv_lines,
                    false,
                    false,
                )),
            },
            selected_slot: 1,
            selected_ordinal: 1,
            slot_count: 2,
            can_split_h: false,
            can_split_v: true,
            can_flip: true,
            can_remove: true,
            active_ratio: ratio,
            can_adjust_ratio: true,
            sync_panes,
            can_break_out: editing_existing,
            editing_existing,
            window_rows: &window_rows,
            window_cursor: 0,
            hovered_sash: None,
            dragging_sash: None,
            hovered_dock: None,
        };

        let mut term = Terminal::new(TestBackend::new(126, 32)).expect("ratio term");
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
                    content: SubviewContent::Split(make_studio(70, true, true)),
                    ..WorkspaceSubviewSpec::default()
                },
                aurora,
            );
        })
        .expect("draw 70/30 split");

        assert_eq!(layout.split_slot_hits.len(), 2);
        let left_w = layout.split_slot_hits[0].0.width;
        let right_w = layout.split_slot_hits[1].0.width;
        assert!(
            left_w > right_w + 20,
            "expected 70/30 split left ({left_w}) much wider than right ({right_w})"
        );
        for btn in [
            SplitToolbarButton::RatioPreset,
            SplitToolbarButton::SyncToggle,
            SplitToolbarButton::PresetGrid,
            SplitToolbarButton::BreakOut,
            SplitToolbarButton::ApplyAndEnter,
        ] {
            assert!(
                layout.split_toolbar_hits.iter().any(|(_, b)| *b == btn),
                "missing toolbar button {btn:?}"
            );
        }
        let text = buffer_logical_text(term.backend().buffer());
        for needle in ["70/30", "SYNC", "Break", "Grid"] {
            assert!(text.contains(needle), "missing {needle}:\n{text}");
        }

        // Verify ZH + ASCII/NO_COLOR with sync ON.
        let zh = Theme {
            language: Language::Zh,
            ..aurora
        };
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
                    content: SubviewContent::Split(make_studio(30, true, true)),
                    ..WorkspaceSubviewSpec::default()
                },
                zh,
            );
        })
        .expect("draw zh 30/70 split");
        let zh_text = buffer_logical_text(term.backend().buffer());
        assert!(zh_text.contains("30/70"));
        assert!(zh_text.contains("同步输入已开启"));
        assert!(zh_text.contains("拆出"));

        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let mut nc_term = Terminal::new(TestBackend::new(126, 32)).expect("nc split term");
        nc_term
            .draw(|frame| {
                render_workspace_subview(
                    frame,
                    frame.area(),
                    &WorkspaceSubviewSpec {
                        details: HomeDetails::default(),
                        telemetry: HomeTelemetry::default(),
                        emblem_step: 0,
                        emblem_mode: EmblemMode::Spin,
                        footer_hint: "",
                        content: SubviewContent::Split(make_studio(70, true, true)),
                        ..WorkspaceSubviewSpec::default()
                    },
                    ascii_nc,
                );
            })
            .expect("draw ascii sync split");
        let asc_text = nc_term.backend().to_string();
        assert!(
            asc_text
                .chars()
                .all(|c| c.is_ascii() || matches!(c, '┌' | '┐' | '└' | '┘' | '─' | '│')),
            "non-ASCII in sync split:\n{asc_text}"
        );
        assert!(
            nc_term
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
        );
    }

    #[test]
    fn split_studio_exposes_sash_hits_ratio_badge_and_dock_overlay() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let window_rows = sample_window_rows();
        let edit_lines = vec!["EDIT_LINE_1".into()];
        let srv_lines = vec!["SRV_LINE_1".into()];

        let studio = SplitStudioView {
            root: SplitCanvasNode::Split {
                horizontal: true,
                ratio: 64,
                first: Box::new(pane_view(
                    1,
                    1,
                    "dev:0 editor",
                    "nvim",
                    &edit_lines,
                    true,
                    false,
                )),
                second: Box::new(pane_view(
                    2,
                    2,
                    "dev:1 server",
                    "cargo",
                    &srv_lines,
                    false,
                    false,
                )),
            },
            selected_slot: 1,
            selected_ordinal: 1,
            slot_count: 2,
            can_split_h: false,
            can_split_v: true,
            can_flip: true,
            can_remove: true,
            active_ratio: 64,
            can_adjust_ratio: true,
            sync_panes: false,
            can_break_out: false,
            editing_existing: false,
            window_rows: &window_rows,
            window_cursor: 0,
            hovered_sash: Some(SashId::SplitNode(0)),
            dragging_sash: Some(SashId::SplitNode(0)),
            hovered_dock: Some((2, DockZone::Right)),
        };

        let mut term = Terminal::new(TestBackend::new(120, 32)).expect("sash term");
        let mut layout = WorkspaceSubviewLayout::default();
        term.draw(|frame| {
            layout = render_workspace_subview(
                frame,
                frame.area(),
                &WorkspaceSubviewSpec {
                    content: SubviewContent::Split(studio),
                    ..WorkspaceSubviewSpec::default()
                },
                aurora,
            );
        })
        .expect("draw sash split");

        assert!(
            layout
                .sash_hits
                .iter()
                .any(|(r, id)| *id == SashId::SplitNode(0) && r.width == 1 && r.height >= 4),
            "expected SplitNode(0) sash hit, got {:?}",
            layout.sash_hits
        );
        let text = buffer_logical_text(term.backend().buffer());
        assert!(text.contains("64% │ 36%"), "missing ratio badge:\n{text}");
        assert!(text.contains("Right 50%"), "missing dock overlay:\n{text}");
    }
}
