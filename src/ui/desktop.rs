//! Desktop-grade terminal GUI widgets: Top IDE Menubar, Mission Control / Exposé live
//! multi-window wall with sparklines, draggable split sashes, VS Code edge dock overlays,
//! 2D drop-shadow right-click context menus, drag ghost cards, hover tooltips, toast
//! notification cards, and inline rename popovers.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::{
    EmblemMode, TextInput, Theme, WindowStatusFlags, WorkspaceSubviewLayout,
    parse_ansi_preview_line, pill_span, single_line, status_style, styled_key_hint_line,
    truncate_to_width, window_status_badge_spans,
};

/// Clickable item inside the Top Desktop IDE Menubar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenubarItem {
    TabStudio,
    TabExpose,
    TabSplit,
    TabLayout,
    MenuDropdown,
    PipToggle,
    ThemeCycle,
    LanguageToggle,
    Config,
    Detach,
}

/// Active top-level workspace tab highlighted in the Top Desktop IDE Menubar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveDesktopTab {
    #[default]
    Studio,
    Expose,
    Split,
    Layout,
    Other,
}

/// Input state for rendering the Top Desktop IDE Menubar on Home & Subviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DesktopMenubarState<'a> {
    pub active_tab: ActiveDesktopTab,
    pub hovered: Option<MenubarItem>,
    pub menu_open: bool,
    pub pip_active: bool,
    pub inside_tmux: bool,
    pub sparkline: &'a [u8],
}

/// Draggable split divider identifier returned in `sash_hits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SashId {
    /// Horizontal divider between Home upper preview and lower matrix.
    HomePreviewMatrix,
    /// Split Studio binary-tree divider identified by 0-based heap index (`root=0`, `first=2*i+1`, `second=2*i+2`).
    SplitNode(usize),
    /// Layout Studio interactive divider (`horizontal = true` for left/right, `false` for top/bottom).
    LayoutResize(bool),
}

/// Per-tile window control button shown on Mission Control / Exposé cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileControl {
    Pip,
    Zoom,
    Rename,
    Close,
}

/// VS Code-style edge dock drop zone highlighted during drag-and-drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

/// Single window tile rendered inside [`ExposeView`].
#[derive(Debug, Clone)]
pub struct ExposeTileView<'a> {
    pub index: usize,
    pub session_name: &'a str,
    pub window_index: u32,
    pub window_name: &'a str,
    pub command: &'a str,
    pub preview_lines: &'a [String],
    pub sparkline: &'a [u8],
    pub flags: WindowStatusFlags,
    pub selected: bool,
    pub hovered: bool,
    pub hovered_control: Option<TileControl>,
    pub pip_active: bool,
    pub dock_highlight: Option<DockZone>,
}

/// View model for Mission Control / Exposé (`SubviewContent::Expose`).
#[derive(Debug, Clone)]
pub struct ExposeView<'a> {
    pub tiles: &'a [ExposeTileView<'a>],
    pub selected: usize,
    pub page: usize,
    pub total_pages: usize,
    pub total_windows: usize,
    pub pip_active: bool,
    pub status_note: Option<&'a str>,
}

/// Geometry and clickable traffic-light control hit-boxes for one Exposé tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeTileHit {
    pub tile_index: usize,
    pub rect: Rect,
    pub control_hits: Vec<(Rect, TileControl)>,
}

/// Single entry inside a floating right-click or dropdown [`ContextMenuView`].
#[derive(Debug, Clone)]
pub struct ContextMenuItemView {
    pub label: String,
    pub icon: String,
    pub shortcut: String,
    pub danger: bool,
    pub divider_before: bool,
    pub enabled: bool,
}

/// Floating 2D drop-shadow context menu specification.
#[derive(Debug, Clone)]
pub struct ContextMenuView<'a> {
    pub title: &'a str,
    pub anchor: (u16, u16),
    pub items: &'a [ContextMenuItemView],
    pub selected: usize,
    pub dim_backdrop: bool,
}

/// Floating ghost card following the cursor during drag-and-drop.
#[derive(Debug, Clone)]
pub struct DragGhostView<'a> {
    pub label: &'a str,
    pub detail: &'a str,
    pub cursor: (u16, u16),
    pub dock_target: Option<(Rect, DockZone)>,
}

/// Severity tone for [`ToastView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Bottom-right floating toast notification with shrinking countdown progress bar.
#[derive(Debug, Clone)]
pub struct ToastView<'a> {
    pub kind: ToastKind,
    pub message: &'a str,
    /// Remaining countdown fraction in `0.0..=1.0`.
    pub progress: f32,
}

/// Compact 1-line hover tooltip pill anchored near a hovered control.
#[derive(Debug, Clone)]
pub struct HoverTooltipView<'a> {
    pub text: &'a str,
    pub anchor: (u16, u16),
}

/// Floating inline rename popover triggered by `F2` or `[✎]`.
#[derive(Debug, Clone)]
pub struct InlineRenameView<'a> {
    pub title: &'a str,
    pub target_label: &'a str,
    pub input: &'a TextInput,
    pub anchor: Option<(u16, u16)>,
}

/// Combined floating desktop overlay payload passed to [`render_desktop_overlays`].
#[derive(Debug, Clone, Default)]
pub struct DesktopOverlaySpec<'a> {
    pub context_menu: Option<ContextMenuView<'a>>,
    pub drag_ghost: Option<DragGhostView<'a>>,
    pub hover_tooltip: Option<HoverTooltipView<'a>>,
    pub toast: Option<ToastView<'a>>,
    pub inline_rename: Option<InlineRenameView<'a>>,
}

/// Geometry and mouse hit-boxes returned by [`render_desktop_overlays`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesktopOverlayLayout {
    pub menu_rect: Option<Rect>,
    pub menu_item_hits: Vec<(Rect, usize)>,
    pub rename_rect: Option<Rect>,
    pub toast_rect: Option<Rect>,
}

// ---------------------------------------------------------------------------
// 1. Sparklines & Sash Dividers
// ---------------------------------------------------------------------------

/// Format an output-velocity ring buffer into a compact Unicode (`▁▂▃▄▅▆▇█`) or pure ASCII sparkline.
pub fn sparkline_string(samples: &[u8], ascii: bool) -> String {
    if samples.is_empty() {
        return String::new();
    }
    const UNI: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const ASC: [char; 8] = ['_', '.', '-', '=', '+', '*', '#', '@'];
    let table = if ascii { &ASC } else { &UNI };
    let peak = samples.iter().copied().max().unwrap_or(0);
    samples
        .iter()
        .map(|&v| {
            if v == 0 || peak == 0 {
                table[0]
            } else {
                let scaled = (usize::from(v) * 7 + usize::from(peak) / 2) / usize::from(peak);
                table[scaled.min(7)]
            }
        })
        .collect()
}

/// Render a vertical or horizontal draggable split sash divider with bright hover/drag highlight
/// and floating percentage ratio badge (`64% │ 36%`).
pub(super) fn render_sash_divider(
    frame: &mut Frame<'_>,
    sash_rect: Rect,
    vertical_bar: bool,
    ratio_pct: u16,
    hovered: bool,
    dragging: bool,
    theme: Theme,
) {
    if sash_rect.width == 0 || sash_rect.height == 0 {
        return;
    }
    let active = hovered || dragging;
    let first_pct = if ratio_pct == 0 {
        50
    } else {
        ratio_pct.clamp(10, 90)
    };
    let second_pct = 100_u16.saturating_sub(first_pct);
    let style = if active {
        if theme.no_color {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            theme.style(theme.accent).add_modifier(Modifier::BOLD)
        }
    } else {
        theme.style(theme.muted)
    };

    if vertical_bar {
        let ch = match (theme.ascii, active) {
            (true, true) => "#",
            (true, false) => "|",
            (false, true) => "┃",
            (false, false) => "│",
        };
        let lines: Vec<Line<'static>> = (0..sash_rect.height)
            .map(|_| Line::from(Span::styled(ch, style)))
            .collect();
        frame.render_widget(Paragraph::new(lines), sash_rect);
    } else {
        let ch = match (theme.ascii, active) {
            (true, true) => "=",
            (true, false) => "-",
            (false, true) => "━",
            (false, false) => "─",
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                ch.repeat(usize::from(sash_rect.width)),
                style,
            ))),
            sash_rect,
        );
    }

    if active {
        let badge = if theme.ascii {
            format!(" {first_pct}% | {second_pct}% ")
        } else {
            format!(" {first_pct}% │ {second_pct}% ")
        };
        let badge_w = u16::try_from(badge.width()).unwrap_or(0);
        let frame_area = frame.area();
        if badge_w > 0 && badge_w <= frame_area.width {
            let mid_x = sash_rect.x.saturating_add(sash_rect.width / 2);
            let mid_y = sash_rect.y.saturating_add(sash_rect.height / 2);
            let bx = mid_x
                .saturating_sub(badge_w / 2)
                .clamp(frame_area.x, frame_area.right().saturating_sub(badge_w));
            let by = mid_y.clamp(frame_area.y, frame_area.bottom().saturating_sub(1));
            let pill_rect = Rect {
                x: bx,
                y: by,
                width: badge_w,
                height: 1,
            };
            let span = if theme.no_color {
                Span::styled(
                    badge,
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                )
            } else {
                Span::styled(
                    badge,
                    Style::default()
                        .fg(theme.keycap_fg())
                        .bg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            };
            frame.render_widget(Paragraph::new(Line::from(span)), pill_rect);
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Top Desktop IDE Menubar
// ---------------------------------------------------------------------------

/// Render the 1-row Top Desktop IDE Menubar (`Studio`, `Expose`, `Split`, `Layout`, `Menu ▾`
/// on the left; Sparkline, `PIP`, `Theme`, `中/EN`, `Config`, `Detach` on the right).
pub(super) fn render_desktop_menubar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: DesktopMenubarState<'_>,
    theme: Theme,
) -> Vec<(Rect, MenubarItem)> {
    let mut hits = Vec::new();
    if area.width < 12 || area.height == 0 {
        return hits;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let bg = theme.pill_bg();
    let bar_bg_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = bg {
        Style::default().bg(surface)
    } else {
        Style::default()
    };

    // Paint subtle full-width bar surface first.
    if !theme.no_color && bg.is_some() {
        frame.render_widget(
            Paragraph::new(" ".repeat(usize::from(area.width))).style(bar_bg_style),
            area,
        );
    }

    let compact = area.width < 88;
    let left_specs = [
        (
            MenubarItem::TabStudio,
            state.active_tab == ActiveDesktopTab::Studio,
            if theme.ascii {
                "[Studio]"
            } else if zh {
                if compact {
                    "[⌂台]"
                } else {
                    "[⌂ 工作台]"
                }
            } else if compact {
                "[⌂ Std]"
            } else {
                "[⌂ Studio]"
            },
        ),
        (
            MenubarItem::TabExpose,
            state.active_tab == ActiveDesktopTab::Expose,
            if theme.ascii {
                "[Expose]"
            } else if zh {
                if compact {
                    "[⊞墙]"
                } else {
                    "[⊞ 全景墙]"
                }
            } else if compact {
                "[⊞ Exp]"
            } else {
                "[⊞ Expose]"
            },
        ),
        (
            MenubarItem::TabSplit,
            state.active_tab == ActiveDesktopTab::Split,
            if theme.ascii {
                "[Split]"
            } else if zh {
                if compact { "[◫分]" } else { "[◫ 分屏]" }
            } else if compact {
                "[◫ Spl]"
            } else {
                "[◫ Split]"
            },
        ),
        (
            MenubarItem::TabLayout,
            state.active_tab == ActiveDesktopTab::Layout,
            if theme.ascii {
                "[Layout]"
            } else if zh {
                if compact { "[▦布]" } else { "[▦ 布局]" }
            } else if compact {
                "[▦ Lay]"
            } else {
                "[▦ Layout]"
            },
        ),
        (
            MenubarItem::MenuDropdown,
            state.menu_open,
            if theme.ascii {
                "[Menu v]"
            } else if zh {
                "[≡ 菜单 ▾]"
            } else {
                "[≡ Menu ▾]"
            },
        ),
    ];

    // Build right-side icons/status controls (highest priority closest to right edge).
    let pip_lbl = if theme.ascii {
        if state.pip_active { "[*PIP]" } else { "[PIP]" }
    } else if zh {
        if state.pip_active {
            "[● 直控]"
        } else {
            "[⌨ 直控]"
        }
    } else if state.pip_active {
        "[● PIP]"
    } else {
        "[⌨ PIP]"
    };
    let theme_lbl = if theme.ascii { "[Clr]" } else { "[🎨]" };
    let lang_lbl = if theme.ascii {
        if theme.language.is_zh() {
            "[EN/ZH]"
        } else {
            "[ZH/EN]"
        }
    } else {
        "[中/EN]"
    };
    let cfg_lbl = if theme.ascii { "[Cfg]" } else { "[⚙]" };
    let detach_lbl = if theme.ascii { "[Det]" } else { "[⏏]" };

    let mut right_specs: Vec<(MenubarItem, &str, Color, bool)> = Vec::new();
    right_specs.push((
        MenubarItem::PipToggle,
        pip_lbl,
        if state.pip_active {
            theme.success
        } else {
            theme.secondary
        },
        state.pip_active,
    ));
    right_specs.push((MenubarItem::ThemeCycle, theme_lbl, theme.tertiary, false));
    right_specs.push((MenubarItem::LanguageToggle, lang_lbl, theme.accent, false));
    right_specs.push((MenubarItem::Config, cfg_lbl, theme.secondary, false));
    if state.inside_tmux {
        right_specs.push((MenubarItem::Detach, detach_lbl, theme.warning, true));
    }

    // Reserve minimum room on right for critical right-side controls so left tabs never crowd them off.
    let min_right_reserve = usize::from(area.width >= 60) * 18;
    let max_left_x = area
        .right()
        .saturating_sub(u16::try_from(min_right_reserve).unwrap_or(0));

    let mut left_spans = Vec::new();
    let mut x = area.x;
    for (idx, (item, active, label)) in left_specs.into_iter().enumerate() {
        let w = u16::try_from(label.width()).unwrap_or(0);
        let gap = u16::from(idx > 0);
        if x.saturating_add(gap).saturating_add(w) > max_left_x {
            break;
        }
        if gap > 0 {
            left_spans.push(Span::styled(" ", bar_bg_style));
            x = x.saturating_add(1);
        }
        let hovered = state.hovered == Some(item);
        let style = if theme.no_color {
            if active || hovered {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default()
            }
        } else if active {
            Style::default()
                .fg(theme.keycap_fg())
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default()
                .fg(theme.accent)
                .bg(bg.unwrap_or(Color::Reset))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            theme.style(theme.text).patch(bar_bg_style)
        };
        left_spans.push(Span::styled(label, style));
        hits.push((
            Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            },
            item,
        ));
        x = x.saturating_add(w);
    }

    frame.render_widget(
        Paragraph::new(Line::from(left_spans)),
        Rect {
            x: area.x,
            y: area.y,
            width: x.saturating_sub(area.x),
            height: 1,
        },
    );

    // Paint right-aligned controls right-to-left within remaining space.
    let mut right_avail = usize::from(area.right().saturating_sub(x.saturating_add(1)));
    let mut chosen_right: Vec<(MenubarItem, &str, Color, bool)> = Vec::new();
    for entry in right_specs.into_iter().rev() {
        let need = entry.1.width() + usize::from(!chosen_right.is_empty());
        if need <= right_avail {
            right_avail = right_avail.saturating_sub(need);
            chosen_right.push(entry);
        }
    }
    chosen_right.reverse();

    let spark = if !state.sparkline.is_empty() && right_avail >= state.sparkline.len().min(8) + 2 {
        let tail_len = state.sparkline.len().min(8);
        let slice = &state.sparkline[state.sparkline.len() - tail_len..];
        let s = sparkline_string(slice, theme.ascii);
        if s.width() + 2 <= right_avail {
            Some(s)
        } else {
            None
        }
    } else {
        None
    };

    let right_pills_w: usize = chosen_right
        .iter()
        .map(|(_, l, _, _)| l.width())
        .sum::<usize>()
        + chosen_right.len().saturating_sub(1);
    let spark_w = spark.as_ref().map_or(0, |s| s.width() + 2);
    let total_right_w = right_pills_w + spark_w;
    if total_right_w > 0
        && u16::try_from(total_right_w).unwrap_or(u16::MAX) <= area.right().saturating_sub(x)
    {
        let start_x = area
            .right()
            .saturating_sub(u16::try_from(total_right_w).unwrap_or(0));
        let mut rx = start_x;
        let mut right_spans = Vec::new();
        if let Some(sp) = spark {
            let sw = u16::try_from(sp.width()).unwrap_or(0);
            right_spans.push(Span::styled(
                sp,
                theme.style(theme.accent).patch(bar_bg_style),
            ));
            right_spans.push(Span::styled("  ", bar_bg_style));
            rx = rx.saturating_add(sw).saturating_add(2);
        }
        for (idx, (item, label, color, emphasized)) in chosen_right.into_iter().enumerate() {
            if idx > 0 {
                right_spans.push(Span::styled(" ", bar_bg_style));
                rx = rx.saturating_add(1);
            }
            let hovered = state.hovered == Some(item);
            let w = u16::try_from(label.width()).unwrap_or(0);
            let span = if theme.no_color {
                if hovered || emphasized {
                    Span::styled(
                        label,
                        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                    )
                } else {
                    Span::raw(label)
                }
            } else if hovered {
                Span::styled(
                    label,
                    Style::default()
                        .fg(theme.keycap_fg())
                        .bg(color)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                pill_span(label, color, bg, theme, emphasized)
            };
            right_spans.push(span);
            hits.push((
                Rect {
                    x: rx,
                    y: area.y,
                    width: w,
                    height: 1,
                },
                item,
            ));
            rx = rx.saturating_add(w);
        }
        frame.render_widget(
            Paragraph::new(Line::from(right_spans)),
            Rect {
                x: start_x,
                y: area.y,
                width: u16::try_from(total_right_w).unwrap_or(0),
                height: 1,
            },
        );
    }

    hits
}

// ---------------------------------------------------------------------------
// 3. VS Code 5-Zone Edge Dock Overlay
// ---------------------------------------------------------------------------

/// Render a highlighted VS Code-style edge dock drop zone (`Left`, `Right`, `Top`, `Bottom`, `Center`)
/// inside `target`.
pub fn render_dock_zone_overlay(frame: &mut Frame<'_>, target: Rect, zone: DockZone, theme: Theme) {
    if target.width < 6 || target.height < 3 {
        return;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let sub = match zone {
        DockZone::Left => Rect {
            x: target.x,
            y: target.y,
            width: (target.width / 2).max(4),
            height: target.height,
        },
        DockZone::Right => {
            let w = (target.width / 2).max(4);
            Rect {
                x: target.right().saturating_sub(w),
                y: target.y,
                width: w,
                height: target.height,
            }
        }
        DockZone::Top => Rect {
            x: target.x,
            y: target.y,
            width: target.width,
            height: (target.height / 2).max(3).min(target.height),
        },
        DockZone::Bottom => {
            let h = (target.height / 2).max(3).min(target.height);
            Rect {
                x: target.x,
                y: target.bottom().saturating_sub(h),
                width: target.width,
                height: h,
            }
        }
        DockZone::Center => {
            let inset_x = u16::from(target.width >= 10);
            let inset_y = u16::from(target.height >= 5);
            Rect {
                x: target.x.saturating_add(inset_x),
                y: target.y.saturating_add(inset_y),
                width: target.width.saturating_sub(inset_x * 2).max(4),
                height: target.height.saturating_sub(inset_y * 2).max(3),
            }
        }
    };

    let label = match (zone, theme.ascii, zh) {
        (DockZone::Left, true, _) => "[< Left 50%]",
        (DockZone::Left, false, true) => "◧ 左侧分屏 50%",
        (DockZone::Left, false, false) => "◧ Left 50%",
        (DockZone::Right, true, _) => "[Right 50% >]",
        (DockZone::Right, false, true) => "◨ 右侧分屏 50%",
        (DockZone::Right, false, false) => "◨ Right 50%",
        (DockZone::Top, true, _) => "[^ Top 50%]",
        (DockZone::Top, false, true) => "⬒ 上方分屏 50%",
        (DockZone::Top, false, false) => "⬒ Top 50%",
        (DockZone::Bottom, true, _) => "[v Bottom 50%]",
        (DockZone::Bottom, false, true) => "⬓ 下方分屏 50%",
        (DockZone::Bottom, false, false) => "⬓ Bottom 50%",
        (DockZone::Center, true, _) => "[# Swap / Open]",
        (DockZone::Center, false, true) => "⊡ 互换 / 合并",
        (DockZone::Center, false, false) => "⊡ Swap / Open",
    };

    frame.render_widget(Clear, sub);
    let border_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        theme.style(theme.accent).add_modifier(Modifier::BOLD)
    };
    let fill_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = theme.pill_bg() {
        Style::default().bg(surface)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Thick
        })
        .border_style(border_style)
        .style(fill_style);
    let inner = block.inner(sub);
    frame.render_widget(block, sub);

    if inner.width > 0 && inner.height > 0 {
        let shown = truncate_to_width(label, usize::from(inner.width));
        let pad_x = usize::from(inner.width).saturating_sub(shown.width()) / 2;
        let mid_y = inner.y.saturating_add(inner.height / 2);
        let pill = if theme.no_color {
            Span::styled(
                shown,
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            )
        } else {
            Span::styled(
                shown,
                Style::default()
                    .fg(theme.keycap_fg())
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::raw(" ".repeat(pad_x)), pill])),
            Rect {
                x: inner.x,
                y: mid_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Mission Control / Exposé (`SubviewContent::Expose`)
// ---------------------------------------------------------------------------

fn choose_expose_grid(tile_count: usize, area: Rect) -> (usize, usize) {
    let n = tile_count.clamp(1, 9);
    if area.width < 46 {
        return (1, n.min(3));
    }
    if area.width < 84 {
        return match n {
            1 => (1, 1),
            2 => (2, 1),
            3..=4 => (2, 2),
            _ => (2, 3),
        };
    }
    match n {
        1 => (1, 1),
        2 => (2, 1),
        3..=4 => (2, 2),
        5..=6 => (3, 2),
        _ => (3, 3),
    }
}

fn render_expose_tile(
    frame: &mut Frame<'_>,
    area: Rect,
    tile: &ExposeTileView<'_>,
    theme: Theme,
) -> ExposeTileHit {
    let mut control_hits = Vec::new();
    if area.width < 8 || area.height < 3 {
        return ExposeTileHit {
            tile_index: tile.index,
            rect: area,
            control_hits,
        };
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let border_color = if tile.pip_active {
        theme.success
    } else if tile.selected {
        theme.accent
    } else if tile.hovered {
        theme.secondary
    } else {
        theme.muted
    };
    let border_type = if theme.ascii {
        BorderType::Plain
    } else if tile.pip_active || tile.selected {
        BorderType::Thick
    } else {
        BorderType::Rounded
    };
    let border_style = if tile.pip_active || tile.selected || tile.hovered {
        theme.style(border_color).add_modifier(Modifier::BOLD)
    } else {
        theme.style(border_color)
    };

    // Reserve right edge of top border for traffic-light controls `[⌨][⛶][✕]`.
    let show_controls = area.width >= 20;
    let ctrl_reserve = if show_controls { 11_usize } else { 0 };
    let top_budget = usize::from(area.width.saturating_sub(2)).saturating_sub(ctrl_reserve);

    let title_raw = format!(
        " {}:{} {}",
        single_line(tile.session_name),
        tile.window_index,
        single_line(tile.window_name)
    );
    let mut title_spans = vec![Span::styled(
        truncate_to_width(&theme.adapt(&title_raw), top_budget.saturating_sub(1)),
        if tile.pip_active {
            theme.style(theme.success).add_modifier(Modifier::BOLD)
        } else if tile.selected {
            theme.style(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            theme.style(theme.text).add_modifier(Modifier::BOLD)
        },
    )];

    // Append compact sparkline if it fits cleanly.
    if !tile.sparkline.is_empty() {
        let sp = sparkline_string(
            &tile.sparkline[tile.sparkline.len().saturating_sub(6)..],
            theme.ascii,
        );
        let used: usize = title_spans.iter().map(Span::width).sum();
        if used + 1 + sp.width() < top_budget {
            title_spans.push(Span::raw(" "));
            title_spans.push(Span::styled(sp, theme.style(theme.secondary)));
        }
    }

    // Append window status badges if room remains.
    let badge_spans = window_status_badge_spans(tile.flags, theme);
    if !badge_spans.is_empty() {
        let bw: usize = badge_spans.iter().map(Span::width).sum();
        let used: usize = title_spans.iter().map(Span::width).sum();
        if used + 1 + bw < top_budget {
            title_spans.push(Span::raw(" "));
            title_spans.extend(badge_spans);
        }
    }
    title_spans.push(Span::raw(" "));

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title(Line::from(title_spans));

    if tile.pip_active {
        let pip_footer = if theme.ascii {
            " [*LIVE INPUT | Esc exit] "
        } else if zh {
            " [ ● LIVE INPUT · Esc 退出直控 ] "
        } else {
            " [ ● LIVE INPUT · Esc exit ] "
        };
        let shown = truncate_to_width(pip_footer, usize::from(area.width.saturating_sub(2)));
        block = block.title_bottom(
            Line::from(Span::styled(
                shown,
                theme.style(theme.success).add_modifier(Modifier::BOLD),
            ))
            .centered(),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Paint traffic-light window controls `[⌨][⛶][✕]` on top-right border with exact hit rects.
    if show_controls {
        let ctrls = [
            (
                TileControl::Pip,
                if theme.ascii { "[I]" } else { "[⌨]" },
                theme.success,
                tile.pip_active,
            ),
            (
                TileControl::Zoom,
                if theme.ascii { "[Z]" } else { "[⛶]" },
                theme.warning,
                tile.flags.zoomed,
            ),
            (
                TileControl::Close,
                if theme.ascii { "[X]" } else { "[✕]" },
                theme.error,
                false,
            ),
        ];
        let total_w: usize = ctrls.iter().map(|(_, s, _, _)| s.width()).sum();
        let start_x = area
            .right()
            .saturating_sub(1)
            .saturating_sub(u16::try_from(total_w).unwrap_or(0));
        let mut cx = start_x;
        let mut spans = Vec::new();
        for (ctrl, lbl, color, active) in ctrls {
            let hovered = tile.hovered_control == Some(ctrl);
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
            spans.push(Span::styled(lbl, style));
            control_hits.push((
                Rect {
                    x: cx,
                    y: area.y,
                    width: w,
                    height: 1,
                },
                ctrl,
            ));
            cx = cx.saturating_add(w);
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: start_x,
                y: area.y,
                width: u16::try_from(total_w).unwrap_or(0),
                height: 1,
            },
        );
    }

    if inner.width > 0 && inner.height > 0 {
        let max_w = usize::from(inner.width);
        let max_h = usize::from(inner.height);
        if tile.preview_lines.is_empty() {
            let msg = if tile.command.is_empty() {
                if zh { "(空终端)" } else { "(empty)" }.to_owned()
            } else {
                format!("$ {}", single_line(tile.command))
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate_to_width(&theme.adapt(&msg), max_w),
                    theme.style(theme.muted),
                ))),
                inner,
            );
        } else {
            let start = tile.preview_lines.len().saturating_sub(max_h);
            let lines: Vec<Line<'static>> = tile.preview_lines[start..]
                .iter()
                .map(|raw| parse_ansi_preview_line(raw, max_w, theme))
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }

    if let Some(zone) = tile.dock_highlight {
        render_dock_zone_overlay(frame, area, zone, theme);
    }

    ExposeTileHit {
        tile_index: tile.index,
        rect: area,
        control_hits,
    }
}

pub(super) fn render_subview_expose(
    frame: &mut Frame<'_>,
    card_inner: Rect,
    view: &ExposeView<'_>,
    _emblem_step: usize,
    _emblem_mode: EmblemMode,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if card_inner.width < 12 || card_inner.height < 4 {
        return render_compact_expose(frame, card_inner, view, theme);
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head_area, grid_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(card_inner);

    let bg = theme.pill_bg();
    let title = if theme.ascii {
        format!("[] Mission Control | {} windows", view.total_windows)
    } else if zh {
        format!("⊞ 全景任务墙 · Exposé · {} 个窗口", view.total_windows)
    } else {
        format!(
            "⊞ Mission Control · Exposé · {} windows",
            view.total_windows
        )
    };
    let mut head_spans = vec![Span::styled(
        title,
        theme.style(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if view.total_pages > 1 {
        let page_pill = if zh {
            format!(" 第 {}/{} 页 ", view.page + 1, view.total_pages)
        } else {
            format!(" Page {}/{} ", view.page + 1, view.total_pages)
        };
        head_spans.push(Span::raw(" "));
        head_spans.push(pill_span(page_pill, theme.secondary, bg, theme, false));
    }
    let mut preview_action_hits = Vec::new();
    if view.pip_active {
        let pip_badge = if theme.ascii {
            " [*LIVE INPUT | Esc exit] "
        } else if zh {
            " ● LIVE INPUT · Esc 退出直控 "
        } else {
            " ● LIVE INPUT · Esc exit "
        };
        head_spans.push(Span::raw(" "));
        head_spans.push(pill_span(pip_badge, theme.success, bg, theme, true));
    } else if let Some(note) = view.status_note {
        let used: usize = head_spans.iter().map(Span::width).sum();
        let rem = usize::from(head_area.width).saturating_sub(used + 2);
        if rem >= 6 {
            head_spans.push(Span::raw("  "));
            head_spans.push(Span::styled(
                truncate_to_width(&theme.adapt(note), rem),
                status_style(note, theme),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(head_spans)), head_area);

    // Clickable `[Send Esc]` button on top-right when PIP is active inside Exposé.
    if view.pip_active {
        let esc_lbl = if zh { "[发送 Esc]" } else { "[Send Esc]" };
        let w = u16::try_from(esc_lbl.width()).unwrap_or(0);
        if w > 0 && w + 2 < head_area.width {
            let ex = head_area.right().saturating_sub(w);
            let esc_rect = Rect {
                x: ex,
                y: head_area.y,
                width: w,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(pill_span(
                    esc_lbl,
                    theme.warning,
                    bg,
                    theme,
                    true,
                ))),
                esc_rect,
            );
            preview_action_hits.push((esc_rect, super::PreviewAction::SendEsc));
        }
    }

    if view.tiles.is_empty() {
        frame.render_widget(
            Paragraph::new(if zh {
                "暂无活跃 tmux 窗口 · 输入 /new 新建会话"
            } else {
                "No active tmux windows · Type /new to create a session"
            })
            .style(theme.style(theme.muted)),
            grid_area,
        );
        return WorkspaceSubviewLayout {
            preview_action_hits,
            ..WorkspaceSubviewLayout::default()
        };
    }

    let visible = &view.tiles[..view.tiles.len().min(9)];
    let (cols, rows) = choose_expose_grid(visible.len(), grid_area);
    let row_constraints = vec![Constraint::Ratio(1, u32::try_from(rows).unwrap_or(1)); rows];
    let row_rects = Layout::vertical(row_constraints).split(grid_area);

    let mut tile_hits = Vec::with_capacity(visible.len());
    let mut idx = 0_usize;
    for row_rect in row_rects.iter() {
        let remaining = visible.len().saturating_sub(idx);
        if remaining == 0 {
            break;
        }
        let row_cols = remaining.min(cols);
        let col_constraints =
            vec![Constraint::Ratio(1, u32::try_from(row_cols).unwrap_or(1)); row_cols];
        let cell_rects = Layout::horizontal(col_constraints).split(*row_rect);
        for cell_rect in cell_rects.iter() {
            if let Some(tile) = visible.get(idx) {
                tile_hits.push(render_expose_tile(frame, *cell_rect, tile, theme));
                idx += 1;
            }
        }
    }

    WorkspaceSubviewLayout {
        preview_action_hits,
        expose_tile_hits: tile_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

pub(super) fn render_compact_expose(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &ExposeView<'_>,
    theme: Theme,
) -> WorkspaceSubviewLayout {
    if area.width < 10 || area.height < 3 {
        return WorkspaceSubviewLayout::default();
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(if zh {
            format!("全景墙 ({})", view.total_windows)
        } else {
            format!("Expose ({})", view.total_windows)
        })
        .style(theme.style(theme.accent).add_modifier(Modifier::BOLD)),
        head,
    );
    let mut tile_hits = Vec::new();
    for (row_i, tile) in view.tiles.iter().take(usize::from(body.height)).enumerate() {
        let rect = Rect {
            x: body.x,
            y: body.y.saturating_add(u16::try_from(row_i).unwrap_or(0)),
            width: body.width,
            height: 1,
        };
        let marker = if tile.selected { theme.marker() } else { " " };
        let style = if tile.selected {
            theme.selected_style()
        } else {
            theme.style(theme.text)
        };
        let line = format!(
            "{marker} {}:{} {}",
            single_line(tile.session_name),
            tile.window_index,
            single_line(tile.window_name)
        );
        frame.render_widget(
            Paragraph::new(truncate_to_width(&line, usize::from(rect.width))).style(style),
            rect,
        );
        tile_hits.push(ExposeTileHit {
            tile_index: tile.index,
            rect,
            control_hits: Vec::new(),
        });
    }
    WorkspaceSubviewLayout {
        expose_tile_hits: tile_hits,
        ..WorkspaceSubviewLayout::default()
    }
}

// ---------------------------------------------------------------------------
// 5. Floating Desktop Overlays (Scrim, Drop-Shadow Menu, Ghost, Tooltip, Toast, Rename)
// ---------------------------------------------------------------------------

fn apply_backdrop_scrim(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let buf = frame.buffer_mut();
    let max_x = area.right().min(buf.area.right());
    let max_y = area.bottom().min(buf.area.bottom());
    for y in area.y..max_y {
        for x in area.x..max_x {
            let cell = &mut buf[(x, y)];
            cell.modifier.insert(Modifier::DIM);
            if !theme.no_color {
                cell.fg = theme.muted;
                cell.bg = Color::Reset;
            }
        }
    }
}

fn draw_2d_drop_shadow(frame: &mut Frame<'_>, popup: Rect, bounds: Rect, theme: Theme) {
    let buf = frame.buffer_mut();
    let shadow_ch = if theme.ascii { "." } else { "░" };
    let shadow_style = if theme.no_color {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default()
            .fg(theme.muted)
            .bg(Color::Reset)
            .add_modifier(Modifier::DIM)
    };
    // Right edge shadow (2 columns wide, shifted down by 1).
    let right_start = popup.right();
    let right_end = right_start
        .saturating_add(2)
        .min(bounds.right())
        .min(buf.area.right());
    let y_start = popup.y.saturating_add(1).min(bounds.bottom());
    let y_end = popup
        .bottom()
        .saturating_add(1)
        .min(bounds.bottom())
        .min(buf.area.bottom());
    for y in y_start..y_end {
        for x in right_start..right_end {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(shadow_ch);
            cell.set_style(shadow_style);
        }
    }
    // Bottom edge shadow (1 row tall, shifted right by 2).
    if popup.bottom() < bounds.bottom() && popup.bottom() < buf.area.bottom() {
        let by = popup.bottom();
        let x_start = popup.x.saturating_add(2).min(bounds.right());
        let x_end = popup
            .right()
            .saturating_add(2)
            .min(bounds.right())
            .min(buf.area.right());
        for x in x_start..x_end {
            let cell = &mut buf[(x, by)];
            cell.set_symbol(shadow_ch);
            cell.set_style(shadow_style);
        }
    }
}

fn adapt_desktop_icon(raw: &str, theme: Theme) -> String {
    if !theme.ascii {
        return raw.to_owned();
    }
    let mapped = match raw.trim() {
        "▸" | "▶" => ">",
        "◀" => "<",
        "✎" => "*",
        "✕" | "✖" => "x",
        "⌨" => "I",
        "⛶" => "Z",
        "⊞" | "▦" => "#",
        "◫" => "|",
        "⏏" => "^",
        "🎨" => "C",
        "⚙" => "*",
        "👥" => "c",
        "🔔" | "⚡" | "⚠" => "!",
        "●" | "✓" => "*",
        "○" => "o",
        "ℹ" => "i",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_owned();
    }
    let adapted = theme.adapt(raw);
    adapted
        .chars()
        .map(|c| if c.is_ascii() { c } else { '*' })
        .collect()
}

fn render_context_menu(
    frame: &mut Frame<'_>,
    bounds: Rect,
    menu: &ContextMenuView<'_>,
    theme: Theme,
) -> (Option<Rect>, Vec<(Rect, usize)>) {
    if menu.items.is_empty() || bounds.width < 14 || bounds.height < 4 {
        return (None, Vec::new());
    }
    let divider_count = menu
        .items
        .iter()
        .enumerate()
        .filter(|(i, it)| *i > 0 && it.divider_before)
        .count();
    let content_rows = menu.items.len() + divider_count;
    let max_item_w = menu
        .items
        .iter()
        .map(|it| {
            let icon_w = if it.icon.is_empty() {
                0
            } else {
                adapt_desktop_icon(&it.icon, theme).width() + 1
            };
            let sc_w = if it.shortcut.is_empty() {
                0
            } else {
                theme.adapt(&it.shortcut).width() + 2
            };
            // 2 borders + 2 ("marker ") + icon + label + shortcut gap/text + 2 padding.
            2 + 2 + icon_w + theme.adapt(&it.label).width() + sc_w + 2
        })
        .max()
        .unwrap_or(18);
    let title_w = theme.adapt(menu.title).width() + 6;
    let desired_w = max_item_w.max(title_w).clamp(20, 48);
    let max_w = usize::from(bounds.width.saturating_sub(2))
        .max(12)
        .min(usize::from(bounds.width));
    let menu_w = u16::try_from(desired_w.min(max_w))
        .unwrap_or(14)
        .min(bounds.width);
    let max_h = bounds.height.saturating_sub(1).max(4).min(bounds.height);
    let menu_h = u16::try_from(content_rows + 2).unwrap_or(max_h).min(max_h);

    let x = menu
        .anchor
        .0
        .clamp(bounds.x, bounds.right().saturating_sub(menu_w));
    let y = menu
        .anchor
        .1
        .clamp(bounds.y, bounds.bottom().saturating_sub(menu_h));
    let rect = Rect {
        x,
        y,
        width: menu_w,
        height: menu_h,
    };

    draw_2d_drop_shadow(frame, rect, bounds, theme);
    frame.render_widget(Clear, rect);

    let bg_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = theme.pill_bg() {
        Style::default().bg(surface)
    } else {
        Style::default()
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.accent).add_modifier(Modifier::BOLD))
        .style(bg_style);
    if !menu.title.is_empty() {
        block = block.title(Line::from(Span::styled(
            format!(
                " {} ",
                truncate_to_width(
                    &single_line(&theme.adapt(menu.title)),
                    usize::from(menu_w.saturating_sub(4))
                )
            ),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )));
    }
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut item_hits = Vec::new();
    let clamped_sel = menu.selected.min(menu.items.len() - 1);
    let div_ch = if theme.ascii { "-" } else { "─" };
    let mut row_y = inner.y;

    for (idx, item) in menu.items.iter().enumerate() {
        if row_y >= inner.bottom() {
            break;
        }
        if idx > 0 && item.divider_before {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    div_ch.repeat(usize::from(inner.width)),
                    theme.style(theme.muted).patch(bg_style),
                ))),
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
            );
            row_y = row_y.saturating_add(1);
            if row_y >= inner.bottom() {
                break;
            }
        }
        let is_sel = idx == clamped_sel;
        let row_rect = Rect {
            x: inner.x,
            y: row_y,
            width: inner.width,
            height: 1,
        };
        let row_style = if !item.enabled {
            theme.style(theme.muted).patch(bg_style)
        } else if is_sel {
            if theme.no_color {
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                let bg_c = if item.danger {
                    theme.error
                } else {
                    theme.accent
                };
                Style::default()
                    .fg(theme.keycap_fg())
                    .bg(bg_c)
                    .add_modifier(Modifier::BOLD)
            }
        } else if item.danger {
            theme.style(theme.error).patch(bg_style)
        } else {
            theme.style(theme.text).patch(bg_style)
        };
        let sc_style = if is_sel {
            row_style
        } else {
            theme.style(theme.muted).patch(bg_style)
        };

        let marker = if is_sel { theme.marker() } else { " " };
        let icon_str = if item.icon.is_empty() {
            String::new()
        } else {
            format!("{} ", adapt_desktop_icon(&item.icon, theme))
        };
        let sc_str = theme.adapt(&item.shortcut);
        let sc_reserve = if sc_str.is_empty() {
            0
        } else {
            sc_str.width() + 2
        };
        let prefix = format!("{marker} {icon_str}");
        let avail_lbl = usize::from(inner.width).saturating_sub(prefix.width() + sc_reserve + 1);
        let lbl = truncate_to_width(&single_line(&theme.adapt(&item.label)), avail_lbl);
        let used = prefix.width() + lbl.width() + sc_str.width();
        let pad = usize::from(inner.width).saturating_sub(used);

        let mut spans = vec![
            Span::styled(prefix, row_style),
            Span::styled(lbl, row_style),
            Span::styled(
                " ".repeat(pad.saturating_sub(usize::from(!sc_str.is_empty()))),
                row_style,
            ),
        ];
        if !sc_str.is_empty() {
            spans.push(Span::styled(sc_str, sc_style));
            if pad > 0 {
                spans.push(Span::styled(" ", row_style));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row_rect);
        if item.enabled {
            item_hits.push((row_rect, idx));
        }
        row_y = row_y.saturating_add(1);
    }

    (Some(rect), item_hits)
}

fn render_toast_card(
    frame: &mut Frame<'_>,
    bounds: Rect,
    toast: &ToastView<'_>,
    theme: Theme,
) -> Option<Rect> {
    if bounds.width < 20 || bounds.height < 5 || toast.message.is_empty() {
        return None;
    }
    let (icon, color) = match toast.kind {
        ToastKind::Info => (if theme.ascii { "i" } else { "ℹ" }, theme.accent),
        ToastKind::Success => (if theme.ascii { "*" } else { "✓" }, theme.success),
        ToastKind::Warning => (if theme.ascii { "!" } else { "⚠" }, theme.warning),
        ToastKind::Error => (if theme.ascii { "X" } else { "✕" }, theme.error),
    };
    let clean_msg = single_line(&theme.adapt(toast.message));
    let desired_w = (clean_msg.width() + 6).clamp(24, 52);
    let max_w = usize::from(bounds.width.saturating_sub(2))
        .max(18)
        .min(usize::from(bounds.width));
    let w = u16::try_from(desired_w.min(max_w))
        .unwrap_or(18)
        .min(bounds.width);
    let h = 4_u16;
    let x = bounds
        .right()
        .saturating_sub(w)
        .saturating_sub(1)
        .max(bounds.x);
    let y = bounds
        .bottom()
        .saturating_sub(h)
        .saturating_sub(1)
        .max(bounds.y);
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, rect);
    let bg_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = theme.pill_bg() {
        Style::default().bg(surface)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(color).add_modifier(Modifier::BOLD))
        .style(bg_style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.width > 0 && inner.height >= 2 {
        let msg_text = truncate_to_width(&format!("{icon} {clean_msg}"), usize::from(inner.width));
        let bar_w = usize::from(inner.width);
        let frac = toast.progress.clamp(0.0, 1.0);
        let filled = ((bar_w as f32) * frac).round().clamp(0.0, bar_w as f32) as usize;
        let fill_ch = if theme.ascii { "=" } else { "━" };
        let rest_ch = if theme.ascii { "-" } else { "─" };
        let bar = Line::from(vec![
            Span::styled(fill_ch.repeat(filled), theme.style(color).patch(bg_style)),
            Span::styled(
                rest_ch.repeat(bar_w.saturating_sub(filled)),
                theme.style(theme.muted).patch(bg_style),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    msg_text,
                    theme
                        .style(theme.text)
                        .add_modifier(Modifier::BOLD)
                        .patch(bg_style),
                )),
                bar,
            ]),
            inner,
        );
    }
    Some(rect)
}

fn render_hover_tooltip(
    frame: &mut Frame<'_>,
    bounds: Rect,
    tooltip: &HoverTooltipView<'_>,
    theme: Theme,
) {
    if tooltip.text.is_empty() || bounds.width < 12 || bounds.height < 3 {
        return;
    }
    let clean = single_line(&theme.adapt(tooltip.text));
    let max_w = usize::from(bounds.width.saturating_sub(2)).min(54);
    let shown = format!(" {} ", truncate_to_width(&clean, max_w.saturating_sub(2)));
    let w = u16::try_from(shown.width()).unwrap_or(0);
    if w == 0 {
        return;
    }
    let x = tooltip
        .anchor
        .0
        .clamp(bounds.x, bounds.right().saturating_sub(w));
    let y = if tooltip.anchor.1.saturating_add(1) < bounds.bottom() {
        tooltip.anchor.1.saturating_add(1)
    } else {
        tooltip.anchor.1.saturating_sub(1).max(bounds.y)
    };
    let rect = Rect {
        x,
        y,
        width: w,
        height: 1,
    };
    let style = if theme.no_color {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(theme.keycap_fg()).bg(theme.secondary)
    };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(shown, style))), rect);
}

fn render_inline_rename_popover(
    frame: &mut Frame<'_>,
    bounds: Rect,
    rename: &InlineRenameView<'_>,
    theme: Theme,
) -> Option<Rect> {
    if bounds.width < 20 || bounds.height < 5 {
        return None;
    }
    let zh = theme.language.is_zh() && !theme.ascii;
    let max_w = bounds.width.saturating_sub(2).max(16).min(bounds.width);
    let w = bounds.width.saturating_sub(4).clamp(16, 52).min(max_w);
    let h = 5_u16.min(bounds.height);
    let (x, y) = if let Some((ax, ay)) = rename.anchor {
        (
            ax.clamp(bounds.x, bounds.right().saturating_sub(w)),
            ay.clamp(bounds.y, bounds.bottom().saturating_sub(h)),
        )
    } else {
        (
            bounds.x.saturating_add(bounds.width.saturating_sub(w) / 2),
            bounds.y.saturating_add(bounds.height.saturating_sub(h) / 2),
        )
    };
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    draw_2d_drop_shadow(frame, rect, bounds, theme);
    frame.render_widget(Clear, rect);

    let bg_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = theme.pill_bg() {
        Style::default().bg(surface)
    } else {
        Style::default()
    };
    let title_str = format!(
        " {} {} ",
        if theme.ascii { ">" } else { "✎" },
        truncate_to_width(
            &single_line(&theme.adapt(rename.title)),
            usize::from(w.saturating_sub(6))
        )
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.warning).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            title_str,
            theme.style(theme.warning).add_modifier(Modifier::BOLD),
        )))
        .style(bg_style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    if inner.width > 0 && inner.height > 0 {
        let mut lines = Vec::new();
        if inner.height >= 3 {
            lines.push(Line::from(Span::styled(
                truncate_to_width(
                    &single_line(&theme.adapt(rename.target_label)),
                    usize::from(inner.width),
                ),
                theme.style(theme.muted).patch(bg_style),
            )));
        }
        let prefix = format!("{} ", theme.marker());
        let mut inp_line = rename.input.rendered_width(
            true,
            theme,
            inner
                .width
                .saturating_sub(u16::try_from(prefix.width()).unwrap_or(2)),
        );
        inp_line.spans.insert(
            0,
            Span::styled(
                prefix,
                theme.style(theme.warning).add_modifier(Modifier::BOLD),
            ),
        );
        lines.push(inp_line);
        if inner.height >= 3 {
            let hint = if zh {
                "Enter 确认重命名 · Esc 取消"
            } else {
                "Enter Save · Esc Cancel"
            };
            lines.push(styled_key_hint_line(
                &truncate_to_width(&theme.adapt(hint), usize::from(inner.width)),
                theme.style(theme.muted),
                theme,
            ));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
    Some(rect)
}

fn render_drag_ghost(frame: &mut Frame<'_>, bounds: Rect, ghost: &DragGhostView<'_>, theme: Theme) {
    if ghost.label.is_empty() || bounds.width < 12 || bounds.height < 3 {
        return;
    }
    let icon = if theme.ascii { "*" } else { "👻" };
    let clean_label = single_line(&theme.adapt(ghost.label));
    let clean_detail = single_line(&theme.adapt(ghost.detail));
    let title = format!(" {icon} {clean_label} ");
    let desired_w = title.width().max(clean_detail.width() + 4).clamp(14, 34);
    let w = u16::try_from(desired_w.min(usize::from(bounds.width))).unwrap_or(14);
    let h = 3_u16.min(bounds.height);
    let x = ghost
        .cursor
        .0
        .saturating_add(2)
        .clamp(bounds.x, bounds.right().saturating_sub(w));
    let y = ghost
        .cursor
        .1
        .saturating_add(1)
        .clamp(bounds.y, bounds.bottom().saturating_sub(h));
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, rect);
    let bg_style = if theme.no_color {
        Style::default()
    } else if let Some(surface) = theme.pill_bg() {
        Style::default().bg(surface)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.style(theme.accent).add_modifier(Modifier::BOLD))
        .title(Line::from(Span::styled(
            truncate_to_width(&title, usize::from(w.saturating_sub(2))),
            theme.style(theme.accent).add_modifier(Modifier::BOLD),
        )))
        .style(bg_style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width > 0 && inner.height > 0 {
        let body = if clean_detail.is_empty() {
            theme.adapt(theme.tr("Drop to move / dock", "松开以移动 / 停靠分屏"))
        } else {
            clean_detail
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_to_width(&body, usize::from(inner.width)),
                theme.style(theme.text).patch(bg_style),
            ))),
            inner,
        );
    }
}

/// Render all active floating desktop overlays (backdrop scrim, VS Code edge dock highlight,
/// toast notification, hover tooltip, 2D drop-shadow context menu, inline rename popover,
/// and cursor-following drag ghost card) on top of the active workspace frame.
pub fn render_desktop_overlays(
    frame: &mut Frame<'_>,
    area: Rect,
    spec: &DesktopOverlaySpec<'_>,
    theme: Theme,
) -> DesktopOverlayLayout {
    if area.width == 0 || area.height == 0 {
        return DesktopOverlayLayout::default();
    }
    let mut layout = DesktopOverlayLayout::default();

    // 1. Optional dimmed backdrop scrim behind modal context menu or inline rename popover.
    let should_dim =
        spec.context_menu.as_ref().is_some_and(|m| m.dim_backdrop) || spec.inline_rename.is_some();
    if should_dim {
        apply_backdrop_scrim(frame, area, theme);
    }

    // 2. VS Code edge dock-zone drop preview inside target rect.
    if let Some(ghost) = &spec.drag_ghost
        && let Some((target_rect, zone)) = ghost.dock_target
    {
        render_dock_zone_overlay(frame, target_rect, zone, theme);
    }

    // 3. Bottom-right toast notification card with countdown bar.
    if let Some(toast) = &spec.toast {
        layout.toast_rect = render_toast_card(frame, area, toast, theme);
    }

    // 4. Hover micro-tooltip (suppressed while modal menu/rename/drag is open).
    if spec.context_menu.is_none()
        && spec.inline_rename.is_none()
        && spec.drag_ghost.is_none()
        && let Some(tip) = &spec.hover_tooltip
    {
        render_hover_tooltip(frame, area, tip, theme);
    }

    // 5. 2D drop-shadow floating context menu.
    if let Some(menu) = &spec.context_menu {
        let (menu_rect, menu_item_hits) = render_context_menu(frame, area, menu, theme);
        layout.menu_rect = menu_rect;
        layout.menu_item_hits = menu_item_hits;
    }

    // 6. Inline rename popover (`F2` / `[✎]`).
    if let Some(rename) = &spec.inline_rename {
        layout.rename_rect = render_inline_rename_popover(frame, area, rename, theme);
    }

    // 7. Floating drag ghost card on top of everything.
    if let Some(ghost) = &spec.drag_ghost {
        render_drag_ghost(frame, area, ghost, theme);
    }

    layout
}

#[cfg(test)]
mod tests {
    use super::super::{
        HomeDetails, HomeTelemetry, Language, SubviewContent, ThemePreset, WorkspaceSubviewSpec,
        render_workspace_subview, tests::buffer_logical_text,
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
        let bad = term
            .backend()
            .buffer()
            .content()
            .iter()
            .enumerate()
            .find(|(_, c)| c.fg != Color::Reset || c.bg != Color::Reset);
        assert!(
            bad.is_none(),
            "color emitted under no_color in {label}: {bad:?}"
        );
    }

    #[test]
    fn sparkline_and_menubar_render_all_hits_en_zh_and_ascii_no_color() {
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

        let samples = [0_u8, 4, 12, 30, 80, 180, 255];
        let uni_sp = sparkline_string(&samples, false);
        let asc_sp = sparkline_string(&samples, true);
        assert_eq!(uni_sp.chars().count(), samples.len());
        assert!(asc_sp.is_ascii());

        let state = DesktopMenubarState {
            active_tab: ActiveDesktopTab::Expose,
            hovered: Some(MenubarItem::LanguageToggle),
            menu_open: false,
            pip_active: true,
            inside_tmux: true,
            sparkline: &samples,
        };

        let mut term = Terminal::new(TestBackend::new(120, 3)).expect("menubar term");
        let mut hits = Vec::new();
        term.draw(|frame| {
            let area = Rect::new(0, 0, 120, 1);
            hits = render_desktop_menubar(frame, area, state, aurora);
        })
        .expect("draw menubar");
        let text = buffer_logical_text(term.backend().buffer());
        for needle in [
            "Studio", "Expose", "Split", "Layout", "Menu", "PIP", "中/EN",
        ] {
            assert!(text.contains(needle), "missing {needle} in:\n{text}");
        }
        for item in [
            MenubarItem::TabStudio,
            MenubarItem::TabExpose,
            MenubarItem::TabSplit,
            MenubarItem::TabLayout,
            MenubarItem::MenuDropdown,
            MenubarItem::PipToggle,
            MenubarItem::ThemeCycle,
            MenubarItem::LanguageToggle,
            MenubarItem::Config,
            MenubarItem::Detach,
        ] {
            assert!(
                hits.iter().any(|(_, m)| *m == item),
                "missing menubar hit {item:?}"
            );
        }

        let mut zh_term = Terminal::new(TestBackend::new(120, 3)).expect("zh menubar");
        zh_term
            .draw(|frame| {
                render_desktop_menubar(frame, Rect::new(0, 0, 120, 1), state, zh);
            })
            .expect("draw zh menubar");
        let zh_text = buffer_logical_text(zh_term.backend().buffer());
        for needle in ["工作台", "全景墙", "分屏", "布局", "菜单", "直控"] {
            assert!(zh_text.contains(needle), "missing ZH {needle}:\n{zh_text}");
        }

        let mut nc_term = Terminal::new(TestBackend::new(120, 3)).expect("nc menubar");
        nc_term
            .draw(|frame| {
                render_desktop_menubar(frame, Rect::new(0, 0, 120, 1), state, ascii_nc);
            })
            .expect("draw nc menubar");
        assert_ascii_and_no_color(&nc_term, "menubar");
    }

    #[test]
    fn expose_mission_control_renders_grid_sparklines_controls_pip_and_dock_zones() {
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
        let lines_a = vec!["$ cargo test".into(), "test result: ok".into()];
        let lines_b = vec!["tail -f app.log".into(), "INFO ready".into()];
        let spark = [2_u8, 8, 24, 60, 95, 40];
        let tiles = vec![
            ExposeTileView {
                index: 0,
                session_name: "dev",
                window_index: 0,
                window_name: "editor",
                command: "nvim",
                preview_lines: &lines_a,
                sparkline: &spark,
                flags: WindowStatusFlags {
                    zoomed: true,
                    ..WindowStatusFlags::default()
                },
                selected: true,
                hovered: false,
                hovered_control: Some(TileControl::Pip),
                pip_active: true,
                dock_highlight: None,
            },
            ExposeTileView {
                index: 1,
                session_name: "dev",
                window_index: 1,
                window_name: "logs",
                command: "tail",
                preview_lines: &lines_b,
                sparkline: &spark,
                flags: WindowStatusFlags {
                    synchronized: true,
                    ..WindowStatusFlags::default()
                },
                selected: false,
                hovered: true,
                hovered_control: Some(TileControl::Close),
                pip_active: false,
                dock_highlight: Some(DockZone::Right),
            },
        ];

        let make_spec = || WorkspaceSubviewSpec {
            details: HomeDetails::default(),
            telemetry: HomeTelemetry::default(),
            menubar: DesktopMenubarState {
                active_tab: ActiveDesktopTab::Expose,
                pip_active: true,
                inside_tmux: true,
                sparkline: &spark,
                ..DesktopMenubarState::default()
            },
            emblem_step: 0,
            emblem_mode: EmblemMode::Spin,
            footer_hint: "",
            content: SubviewContent::Expose(ExposeView {
                tiles: &tiles,
                selected: 0,
                page: 0,
                total_pages: 2,
                total_windows: 11,
                pip_active: true,
                status_note: None,
            }),
        };

        let mut term = Terminal::new(TestBackend::new(120, 30)).expect("expose term");
        let mut layout = WorkspaceSubviewLayout::default();
        term.draw(|frame| {
            layout = render_workspace_subview(frame, frame.area(), &make_spec(), aurora);
        })
        .expect("draw expose");
        let out = buffer_logical_text(term.backend().buffer());
        for needle in [
            "Mission Control",
            "Page 1/2",
            "LIVE INPUT",
            "[Send Esc]",
            "dev:0 editor",
            "Right 50%",
        ] {
            assert!(out.contains(needle), "missing {needle}:\n{out}");
        }
        assert_eq!(layout.expose_tile_hits.len(), 2);
        assert_eq!(layout.expose_tile_hits[0].control_hits.len(), 3);
        assert!(
            layout
                .preview_action_hits
                .iter()
                .any(|(_, a)| *a == super::super::PreviewAction::SendEsc)
        );

        let mut zh_term = Terminal::new(TestBackend::new(120, 30)).expect("zh expose");
        zh_term
            .draw(|frame| {
                render_workspace_subview(frame, frame.area(), &make_spec(), zh);
            })
            .expect("draw zh expose");
        let zh_out = buffer_logical_text(zh_term.backend().buffer());
        assert!(zh_out.contains("全景任务墙") && zh_out.contains("右侧分屏 50%"));

        let mut nc_term = Terminal::new(TestBackend::new(120, 30)).expect("nc expose");
        nc_term
            .draw(|frame| {
                render_workspace_subview(frame, frame.area(), &make_spec(), ascii_nc);
            })
            .expect("draw nc expose");
        assert_ascii_and_no_color(&nc_term, "expose");

        // Compact fallback (< 36x12).
        let mut compact = Terminal::new(TestBackend::new(30, 8)).expect("compact expose");
        let mut clayout = WorkspaceSubviewLayout::default();
        compact
            .draw(|frame| {
                clayout = render_workspace_subview(frame, frame.area(), &make_spec(), aurora);
            })
            .expect("draw compact expose");
        assert_eq!(clayout.expose_tile_hits.len(), 2);
    }

    #[test]
    fn desktop_overlays_render_menu_shadow_ghost_dock_toast_tooltip_and_rename_safely() {
        let aurora = Theme::default().with_preset(ThemePreset::AntigravityAurora);
        let ascii_nc = Theme {
            ascii: true,
            no_color: true,
            truecolor: false,
            ..aurora
        };
        let items = [
            ContextMenuItemView {
                label: "Open Window".into(),
                icon: "▸".into(),
                shortcut: "Enter".into(),
                danger: false,
                divider_before: false,
                enabled: true,
            },
            ContextMenuItemView {
                label: "Rename".into(),
                icon: "✎".into(),
                shortcut: "F2".into(),
                danger: false,
                divider_before: false,
                enabled: true,
            },
            ContextMenuItemView {
                label: "Close Window".into(),
                icon: "✕".into(),
                shortcut: "MidClick".into(),
                danger: true,
                divider_before: true,
                enabled: true,
            },
        ];
        let rename_input = TextInput::new("worker-api");

        // Test context menu anchored near right edge clamps cleanly + renders shadow & hits
        // alongside dock overlay, drag ghost, inline rename popover, and bottom-right toast.
        let mut term = Terminal::new(TestBackend::new(100, 26)).expect("overlay term");
        let mut ol = DesktopOverlayLayout::default();
        term.draw(|frame| {
            ol = render_desktop_overlays(
                frame,
                frame.area(),
                &DesktopOverlaySpec {
                    context_menu: Some(ContextMenuView {
                        title: "dev:1 logs",
                        anchor: (95, 2),
                        items: &items,
                        selected: 2,
                        dim_backdrop: true,
                    }),
                    drag_ghost: Some(DragGhostView {
                        label: "dev:1 logs",
                        detail: "Drop on edge to split",
                        cursor: (40, 10),
                        dock_target: Some((Rect::new(2, 2, 40, 12), DockZone::Left)),
                    }),
                    hover_tooltip: None,
                    toast: Some(ToastView {
                        kind: ToastKind::Success,
                        message: "Copied 12 lines to clipboard",
                        progress: 0.65,
                    }),
                    inline_rename: Some(InlineRenameView {
                        title: "Rename Window",
                        target_label: "dev:1 logs",
                        input: &rename_input,
                        anchor: Some((6, 16)),
                    }),
                },
                aurora,
            );
        })
        .expect("draw overlays");

        let menu_rect = ol.menu_rect.expect("menu rect");
        assert!(menu_rect.right() <= 100 && menu_rect.bottom() <= 26);
        assert_eq!(ol.menu_item_hits.len(), 3);
        assert!(ol.toast_rect.is_some());
        assert!(ol.rename_rect.is_some());
        let text = buffer_logical_text(term.backend().buffer());
        for needle in [
            "Open Window",
            "Close Window",
            "Left 50%",
            "Copied 12 lines",
            "worker-api",
        ] {
            assert!(text.contains(needle), "missing {needle}:\n{text}");
        }

        // Also verify bottom-right corner clamping for context menu.
        let mut clamp_term = Terminal::new(TestBackend::new(100, 26)).expect("clamp term");
        let mut clamp_ol = DesktopOverlayLayout::default();
        clamp_term
            .draw(|frame| {
                clamp_ol = render_desktop_overlays(
                    frame,
                    frame.area(),
                    &DesktopOverlaySpec {
                        context_menu: Some(ContextMenuView {
                            title: "dev:1 logs",
                            anchor: (98, 25),
                            items: &items,
                            selected: 0,
                            dim_backdrop: false,
                        }),
                        ..DesktopOverlaySpec::default()
                    },
                    aurora,
                );
            })
            .expect("draw clamped menu");
        let c_rect = clamp_ol.menu_rect.expect("clamped menu rect");
        assert!(c_rect.right() <= 100 && c_rect.bottom() <= 26);

        // Hover tooltip standalone + ASCII/NO_COLOR invariants.
        let mut nc_term = Terminal::new(TestBackend::new(100, 26)).expect("nc overlay");
        nc_term
            .draw(|frame| {
                render_desktop_overlays(
                    frame,
                    frame.area(),
                    &DesktopOverlaySpec {
                        hover_tooltip: Some(HoverTooltipView {
                            text: "Toggle Live PIP Direct Input",
                            anchor: (85, 1),
                        }),
                        toast: Some(ToastView {
                            kind: ToastKind::Warning,
                            message: "Sync panes ON",
                            progress: 0.4,
                        }),
                        ..DesktopOverlaySpec::default()
                    },
                    ascii_nc,
                );
            })
            .expect("draw nc tooltip");
        assert_ascii_and_no_color(&nc_term, "desktop overlays");
        assert!(nc_term.backend().to_string().contains("Toggle Live PIP"));

        // Sash divider with floating percentage ratio badge.
        let mut sash_term = Terminal::new(TestBackend::new(60, 12)).expect("sash term");
        sash_term
            .draw(|frame| {
                render_sash_divider(frame, Rect::new(30, 0, 1, 12), true, 64, true, true, aurora);
            })
            .expect("draw sash");
        assert!(buffer_logical_text(sash_term.backend().buffer()).contains("64% │ 36%"));
    }
}
