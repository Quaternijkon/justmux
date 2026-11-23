//! Interactive 2x2 visual split layout tree and studio state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::layout::Rect;

use crate::tmux::{PaneId, SessionId, SplitDirection, Window, WindowId};

/// Identifier of a single leaf pane inside [`SplitStudio`].
pub type SlotId = usize;

/// Content assigned to one leaf slot in the 2x2 split layout draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitSlotFill {
    Window(WindowId),
    NewShell,
}

/// Binary split tree constrained to at most 2 columns by 2 rows (4 panes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitNode {
    Leaf {
        id: SlotId,
        fill: SplitSlotFill,
    },
    Split {
        direction: SplitDirection,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

/// Interactive toolbar button exposed inside the visual Split Studio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitToolbarAction {
    SplitHorizontal,
    SplitVertical,
    FlipDirection,
    RemoveSlot,
    Reset,
    ApplyAndEnter,
}

/// Candidate existing tmux window that can be placed into any split slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitWindowOption {
    pub window: Window,
    pub active_pane_id: Option<PaneId>,
    pub command: String,
    pub path: String,
}

/// Full interactive state for the GUI-like 2x2 Split Studio subview.
#[derive(Debug, Clone)]
pub struct SplitStudio {
    pub anchor_session_id: SessionId,
    pub anchor_cwd: PathBuf,
    pub initial_fill: SplitSlotFill,
    pub root: SplitNode,
    pub next_slot_id: SlotId,
    pub selected_slot: SlotId,
    pub window_cursor: usize,
    pub window_options: Vec<SplitWindowOption>,
    pub previews: HashMap<WindowId, Vec<String>>,
    pub preview_scrolls: HashMap<SlotId, usize>,
    pub slot_hits: Vec<(Rect, SlotId)>,
    pub toolbar_hits: Vec<(Rect, SplitToolbarAction)>,
    pub window_list_rect: Option<Rect>,
}

impl SplitNode {
    /// Construct a leaf pane node.
    pub fn new_leaf(id: SlotId, fill: SplitSlotFill) -> Self {
        Self::Leaf { id, fill }
    }

    /// Number of leaf panes in this subtree.
    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Ordered left-to-right / top-to-bottom list of leaves.
    pub fn leaves(&self) -> Vec<(SlotId, SplitSlotFill)> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<(SlotId, SplitSlotFill)>) {
        match self {
            Self::Leaf { id, fill } => out.push((*id, fill.clone())),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Leftmost/topmost leaf in this subtree.
    pub fn first_leaf(&self) -> (SlotId, SplitSlotFill) {
        match self {
            Self::Leaf { id, fill } => (*id, fill.clone()),
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Lookup fill assigned to `target`.
    pub fn slot_fill(&self, target: SlotId) -> Option<&SplitSlotFill> {
        match self {
            Self::Leaf { id, fill } if *id == target => Some(fill),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => {
                first.slot_fill(target).or_else(|| second.slot_fill(target))
            }
        }
    }

    /// Find which slot (if any) currently displays `window_id`.
    pub fn find_slot_with_window(&self, window_id: &WindowId) -> Option<SlotId> {
        match self {
            Self::Leaf {
                id,
                fill: SplitSlotFill::Window(wid),
            } if wid == window_id => Some(*id),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => first
                .find_slot_with_window(window_id)
                .or_else(|| second.find_slot_with_window(window_id)),
        }
    }

    /// Count `(horizontal_splits, vertical_splits)` along the path from root to `target`.
    pub fn axis_depths(&self, target: SlotId) -> Option<(usize, usize)> {
        match self {
            Self::Leaf { id, .. } => (*id == target).then_some((0, 0)),
            Self::Split {
                direction,
                first,
                second,
            } => {
                let (h, v) = first
                    .axis_depths(target)
                    .or_else(|| second.axis_depths(target))?;
                Some((
                    h + usize::from(*direction == SplitDirection::SideBySide),
                    v + usize::from(*direction == SplitDirection::Stacked),
                ))
            }
        }
    }

    /// Enforce at most a 2x2 grid: at most 1 horizontal split and 1 vertical split on any path.
    pub fn can_split(&self, target: SlotId, direction: SplitDirection) -> bool {
        if self.leaf_count() >= 4 {
            return false;
        }
        let Some((h, v)) = self.axis_depths(target) else {
            return false;
        };
        match direction {
            SplitDirection::SideBySide => h == 0,
            SplitDirection::Stacked => v == 0,
        }
    }

    /// Whether this layout has at least two panes whose split directions can be transposed.
    pub fn can_flip(&self) -> bool {
        self.leaf_count() >= 2
    }

    /// Transpose every split direction (`SideBySide <-> Stacked`), preserving max 2x2 invariants.
    pub fn flip_directions(&mut self) -> bool {
        if !self.can_flip() {
            return false;
        }
        self.flip_directions_inner();
        true
    }

    fn flip_directions_inner(&mut self) {
        if let Self::Split {
            direction,
            first,
            second,
        } = self
        {
            *direction = match *direction {
                SplitDirection::SideBySide => SplitDirection::Stacked,
                SplitDirection::Stacked => SplitDirection::SideBySide,
            };
            first.flip_directions_inner();
            second.flip_directions_inner();
        }
    }

    /// Replace any [`SplitSlotFill::Window`] references absent from `live_windows` with [`SplitSlotFill::NewShell`].
    pub fn prune_missing_windows(&mut self, live_windows: &HashSet<WindowId>) {
        match self {
            Self::Leaf { fill, .. } => {
                if let SplitSlotFill::Window(wid) = fill
                    && !live_windows.contains(wid)
                {
                    *fill = SplitSlotFill::NewShell;
                }
            }
            Self::Split { first, second, .. } => {
                first.prune_missing_windows(live_windows);
                second.prune_missing_windows(live_windows);
            }
        }
    }

    /// Split `target` leaf in `direction`, moving existing fill to `first` and adding `new_id` as `second`.
    pub fn split_slot(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        if !self.can_split(target, direction) {
            return false;
        }
        self.split_slot_inner(target, direction, new_id, new_fill)
    }

    fn split_slot_inner(
        &mut self,
        target: SlotId,
        direction: SplitDirection,
        new_id: SlotId,
        new_fill: SplitSlotFill,
    ) -> bool {
        match self {
            Self::Leaf { id, fill } if *id == target => {
                let kept_id = *id;
                let kept_fill = std::mem::replace(fill, SplitSlotFill::NewShell);
                *self = Self::Split {
                    direction,
                    first: Box::new(Self::new_leaf(kept_id, kept_fill)),
                    second: Box::new(Self::new_leaf(new_id, new_fill)),
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_slot_inner(target, direction, new_id, new_fill.clone())
                    || second.split_slot_inner(target, direction, new_id, new_fill)
            }
        }
    }

    /// Remove `target` leaf, collapsing its parent split to its sibling and returning the sibling's first leaf.
    pub fn remove_slot(&mut self, target: SlotId) -> Option<SlotId> {
        if self.leaf_count() <= 1 || self.slot_fill(target).is_none() {
            return None;
        }
        let owned = std::mem::replace(self, Self::new_leaf(0, SplitSlotFill::NewShell));
        let (rebuilt, next_focus) = Self::remove_inner(owned, target);
        *self = rebuilt;
        next_focus
    }

    fn remove_inner(node: Self, target: SlotId) -> (Self, Option<SlotId>) {
        match node {
            Self::Leaf { .. } => (node, None),
            Self::Split {
                direction,
                first,
                second,
            } => {
                if matches!(&*first, Self::Leaf { id, .. } if *id == target) {
                    let next = second.first_leaf().0;
                    return (*second, Some(next));
                }
                if matches!(&*second, Self::Leaf { id, .. } if *id == target) {
                    let next = first.first_leaf().0;
                    return (*first, Some(next));
                }
                let (new_first, focus_first) = Self::remove_inner(*first, target);
                if let Some(next) = focus_first {
                    return (
                        Self::Split {
                            direction,
                            first: Box::new(new_first),
                            second,
                        },
                        Some(next),
                    );
                }
                let (new_second, focus_second) = Self::remove_inner(*second, target);
                (
                    Self::Split {
                        direction,
                        first: Box::new(new_first),
                        second: Box::new(new_second),
                    },
                    focus_second,
                )
            }
        }
    }

    /// Assign `fill` to `target`, swapping if `Window(wid)` is already held by another slot.
    pub fn set_slot_fill(&mut self, target: SlotId, fill: SplitSlotFill) -> bool {
        let Some(current) = self.slot_fill(target).cloned() else {
            return false;
        };
        if current == fill {
            return true;
        }
        if let SplitSlotFill::Window(wid) = &fill
            && let Some(other) = self.find_slot_with_window(wid)
            && other != target
        {
            return self.replace_leaf_fill(other, current) && self.replace_leaf_fill(target, fill);
        }
        self.replace_leaf_fill(target, fill)
    }

    fn replace_leaf_fill(&mut self, target: SlotId, next_fill: SplitSlotFill) -> bool {
        match self {
            Self::Leaf { id, fill } if *id == target => {
                *fill = next_fill;
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.replace_leaf_fill(target, next_fill.clone())
                    || second.replace_leaf_fill(target, next_fill)
            }
        }
    }
}

impl SplitStudio {
    /// Build a fresh 1x1 studio seeded with `initial_window` (if present in `options`) or `NewShell`.
    pub fn new(
        anchor_session_id: SessionId,
        anchor_cwd: PathBuf,
        initial_window: Option<&WindowId>,
        mut window_options: Vec<SplitWindowOption>,
        previews: HashMap<WindowId, Vec<String>>,
    ) -> Self {
        window_options.sort_by_key(|opt| usize::from(opt.window.session_id != anchor_session_id));
        let initial_fill = initial_window
            .filter(|wid| window_options.iter().any(|opt| opt.window.id == **wid))
            .cloned()
            .map_or(SplitSlotFill::NewShell, SplitSlotFill::Window);
        let mut studio = Self {
            anchor_session_id,
            anchor_cwd,
            initial_fill: initial_fill.clone(),
            root: SplitNode::new_leaf(1, initial_fill),
            next_slot_id: 2,
            selected_slot: 1,
            window_cursor: 0,
            window_options,
            previews,
            preview_scrolls: HashMap::new(),
            slot_hits: Vec::new(),
            toolbar_hits: Vec::new(),
            window_list_rect: None,
        };
        studio.sync_cursor_to_selected_slot();
        studio
    }

    /// Existing windows plus final `+ New shell` option row.
    pub fn total_window_rows(&self) -> usize {
        self.window_options.len() + 1
    }

    /// Resolve which fill corresponds to row `row` in the bottom window selector.
    pub fn fill_for_row(&self, row: usize) -> SplitSlotFill {
        self.window_options
            .get(row)
            .map_or(SplitSlotFill::NewShell, |opt| {
                SplitSlotFill::Window(opt.window.id.clone())
            })
    }

    /// Point `window_cursor` to whatever fills `selected_slot`.
    pub fn sync_cursor_to_selected_slot(&mut self) {
        self.window_cursor = match self.root.slot_fill(self.selected_slot) {
            Some(SplitSlotFill::Window(wid)) => self
                .window_options
                .iter()
                .position(|opt| opt.window.id == *wid)
                .unwrap_or(self.window_options.len()),
            Some(SplitSlotFill::NewShell) | None => self.window_options.len(),
        };
    }

    /// Split currently focused slot in `direction` and move focus to the newly created `NewShell` slot.
    pub fn split_selected(&mut self, direction: SplitDirection) -> bool {
        let new_id = self.next_slot_id;
        if self.root.split_slot(
            self.selected_slot,
            direction,
            new_id,
            SplitSlotFill::NewShell,
        ) {
            self.next_slot_id += 1;
            self.selected_slot = new_id;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Remove currently focused slot if more than 1 slot exists.
    pub fn remove_selected(&mut self) -> bool {
        if let Some(next_focus) = self.root.remove_slot(self.selected_slot) {
            self.preview_scrolls.remove(&self.selected_slot);
            self.selected_slot = next_focus;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Reset back to initial 1x1 layout.
    pub fn reset(&mut self) {
        self.root = SplitNode::new_leaf(1, self.initial_fill.clone());
        self.next_slot_id = 2;
        self.selected_slot = 1;
        self.preview_scrolls.clear();
        self.sync_cursor_to_selected_slot();
    }

    /// Select slot by `SlotId`.
    pub fn select_slot(&mut self, slot_id: SlotId) -> bool {
        if self.root.slot_fill(slot_id).is_some() {
            self.selected_slot = slot_id;
            self.sync_cursor_to_selected_slot();
            true
        } else {
            false
        }
    }

    /// Cycle focus forward or backward through slots in visual reading order.
    pub fn cycle_slot(&mut self, forward: bool) {
        let leaves = self.root.leaves();
        if leaves.is_empty() {
            return;
        }
        let pos = leaves
            .iter()
            .position(|(id, _)| *id == self.selected_slot)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % leaves.len()
        } else {
            (pos + leaves.len() - 1) % leaves.len()
        };
        self.selected_slot = leaves[next].0;
        self.sync_cursor_to_selected_slot();
    }

    /// Focus slot by 1-based visual number (`1..=4`).
    pub fn select_slot_by_number(&mut self, one_based: usize) -> bool {
        let leaves = self.root.leaves();
        if let Some((slot_id, _)) = one_based.checked_sub(1).and_then(|idx| leaves.get(idx)) {
            self.select_slot(*slot_id)
        } else {
            false
        }
    }

    /// Return 1-based visual index (`1..=4`) for `slot_id`.
    pub fn slot_ordinal(&self, slot_id: SlotId) -> Option<usize> {
        self.root
            .leaves()
            .iter()
            .position(|(id, _)| *id == slot_id)
            .map(|idx| idx + 1)
    }

    /// Navigate candidate window list: auto-fills free windows or `+ New shell`, without stealing occupied windows.
    pub fn move_window_cursor(&mut self, up: bool) {
        let total = self.total_window_rows();
        if total == 0 {
            return;
        }
        let next = if up {
            self.window_cursor.saturating_sub(1)
        } else {
            (self.window_cursor + 1).min(total - 1)
        };
        self.window_cursor = next;
        let candidate = self.fill_for_row(next);
        let should_autofill = match &candidate {
            SplitSlotFill::NewShell => true,
            SplitSlotFill::Window(wid) => self
                .root
                .find_slot_with_window(wid)
                .is_none_or(|owner| owner == self.selected_slot),
        };
        if should_autofill {
            let previous = self.root.slot_fill(self.selected_slot).cloned();
            if previous.as_ref() != Some(&candidate)
                && self.root.set_slot_fill(self.selected_slot, candidate)
            {
                self.preview_scrolls.remove(&self.selected_slot);
            }
        }
    }

    /// Explicitly assign row `row` to the selected slot, swapping if held by another slot.
    pub fn assign_cursor_row(&mut self, row: usize) -> bool {
        if row >= self.total_window_rows() {
            return false;
        }
        self.window_cursor = row;
        let fill = self.fill_for_row(row);
        let changed = self.root.slot_fill(self.selected_slot) != Some(&fill);
        let applied = self.root.set_slot_fill(self.selected_slot, fill);
        if applied && changed {
            self.preview_scrolls.remove(&self.selected_slot);
        }
        applied
    }

    /// Scroll preview history inside `slot_id`.
    pub fn scroll_slot_preview(&mut self, slot_id: SlotId, up: bool, step: usize, max: usize) {
        let entry = self.preview_scrolls.entry(slot_id).or_insert(0);
        if up {
            *entry = entry.saturating_add(step).min(max);
        } else {
            *entry = (*entry).min(max).saturating_sub(step);
        }
    }

    /// Flip horizontal and vertical split directions across the layout tree.
    pub fn flip_layout(&mut self) -> bool {
        self.root.flip_directions()
    }

    /// Lookup window option metadata by ID.
    pub fn window_option(&self, window_id: &WindowId) -> Option<&SplitWindowOption> {
        self.window_options
            .iter()
            .find(|opt| &opt.window.id == window_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(raw: &str) -> SessionId {
        SessionId::new(raw).expect("valid session id")
    }

    fn wid(raw: &str) -> WindowId {
        WindowId::new(raw).expect("valid window id")
    }

    fn win(raw_sid: &str, sname: &str, raw_wid: &str, index: u32, name: &str) -> SplitWindowOption {
        SplitWindowOption {
            window: Window {
                id: wid(raw_wid),
                session_id: sid(raw_sid),
                session_name: sname.into(),
                index,
                name: name.into(),
                active: index == 0,
                panes: 1,
            },
            active_pane_id: Some(PaneId::new(format!("%{}", index + 1)).expect("pane id")),
            command: "zsh".into(),
            path: "/tmp/demo".into(),
        }
    }

    #[test]
    fn split_node_enforces_max_2x2_grid_and_collapses_on_remove() {
        let mut tree = SplitNode::new_leaf(1, SplitSlotFill::Window(wid("@1")));
        assert!(tree.can_split(1, SplitDirection::SideBySide));
        assert!(tree.can_split(1, SplitDirection::Stacked));

        // 1 -> 2 columns (1 | 2)
        assert!(tree.split_slot(1, SplitDirection::SideBySide, 2, SplitSlotFill::NewShell));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
        assert!(!tree.can_split(2, SplitDirection::SideBySide));
        assert!(tree.can_split(1, SplitDirection::Stacked));
        assert!(tree.can_split(2, SplitDirection::Stacked));

        // 2 columns -> split left vertically (1/3 | 2), then right vertically (1/3 | 2/4)
        assert!(tree.split_slot(1, SplitDirection::Stacked, 3, SplitSlotFill::NewShell));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
        assert!(!tree.can_split(1, SplitDirection::Stacked));
        assert!(!tree.can_split(3, SplitDirection::Stacked));
        assert!(tree.split_slot(2, SplitDirection::Stacked, 4, SplitSlotFill::NewShell));
        assert_eq!(tree.leaf_count(), 4);
        for leaf in [1, 2, 3, 4] {
            assert!(!tree.can_split(leaf, SplitDirection::SideBySide));
            assert!(!tree.can_split(leaf, SplitDirection::Stacked));
        }

        // Row-first then column split also reaches 2x2 cleanly.
        let mut row_first = SplitNode::new_leaf(1, SplitSlotFill::NewShell);
        assert!(row_first.split_slot(1, SplitDirection::Stacked, 2, SplitSlotFill::NewShell));
        assert!(!row_first.can_split(1, SplitDirection::Stacked));
        assert!(row_first.split_slot(1, SplitDirection::SideBySide, 3, SplitSlotFill::NewShell));
        assert!(row_first.split_slot(2, SplitDirection::SideBySide, 4, SplitSlotFill::NewShell));
        assert_eq!(row_first.leaf_count(), 4);

        // Removing slot 3 collapses its vertical split back to 1 | (2/4).
        let next = tree.remove_slot(3).expect("removed slot 3");
        assert_eq!(next, 1);
        assert_eq!(tree.leaf_count(), 3);
        assert!(tree.can_split(1, SplitDirection::Stacked));
        assert!(!tree.can_split(1, SplitDirection::SideBySide));
    }

    #[test]
    fn studio_orders_anchor_session_first_auto_fills_free_windows_and_swaps_on_explicit_assign() {
        let w_other = win("$2", "ops", "@20", 0, "logs");
        let w_a0 = win("$1", "dev", "@10", 0, "editor");
        let w_a1 = win("$1", "dev", "@11", 1, "server");

        let mut studio = SplitStudio::new(
            sid("$1"),
            PathBuf::from("/tmp/dev"),
            Some(&wid("@10")),
            vec![w_other, w_a0, w_a1],
            HashMap::new(),
        );
        // Anchor session ($1) sorted first cleanly.
        assert_eq!(studio.window_options[0].window.id, wid("@10"));
        assert_eq!(studio.window_options[1].window.id, wid("@11"));
        assert_eq!(studio.window_options[2].window.id, wid("@20"));
        assert_eq!(studio.selected_slot, 1);
        assert_eq!(studio.window_cursor, 0);

        // Split H -> creates slot 2 (NewShell) and focuses it (row 3 = + New shell).
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert_eq!(studio.selected_slot, 2);
        assert_eq!(studio.window_cursor, 3);
        assert_eq!(studio.root.slot_fill(2), Some(&SplitSlotFill::NewShell));

        // Moving Up to row 2 (@20, unassigned) auto-fills slot 2 without stealing anything.
        studio.move_window_cursor(true);
        assert_eq!(studio.window_cursor, 2);
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@20")))
        );

        // Moving Up twice onto row 0 (@10, owned by slot 1) moves cursor ONLY and does NOT steal @10!
        studio.move_window_cursor(true);
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@11")))
        );
        studio.move_window_cursor(true);
        assert_eq!(studio.window_cursor, 0);
        assert_eq!(
            studio.root.slot_fill(1),
            Some(&SplitSlotFill::Window(wid("@10")))
        );
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@11")))
        );

        // Explicit assign (Space / click) on occupied row 0 SWAPS slot 1 and slot 2 cleanly!
        assert!(studio.assign_cursor_row(0));
        assert_eq!(
            studio.root.slot_fill(1),
            Some(&SplitSlotFill::Window(wid("@11")))
        );
        assert_eq!(
            studio.root.slot_fill(2),
            Some(&SplitSlotFill::Window(wid("@10")))
        );
    }

    #[test]
    fn flip_directions_and_studio_flip_layout_transpose_and_preserve_2x2_caps() {
        let w1 = win("$1", "dev", "@10", 0, "editor");
        let w2 = win("$1", "dev", "@11", 1, "server");

        let mut studio = SplitStudio::new(
            sid("$1"),
            PathBuf::from("/tmp/dev"),
            Some(&wid("@10")),
            vec![w1, w2],
            HashMap::new(),
        );
        // 1x1 layout cannot flip.
        assert!(!studio.root.can_flip());
        assert!(!studio.flip_layout());

        // 1x2 (`1 | 2`) flips to 2x1 (`1 / 2`) and flips back.
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert!(studio.root.can_flip());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::SideBySide,
                ..
            }
        ));
        assert!(studio.flip_layout());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::Stacked,
                ..
            }
        ));
        assert!(studio.flip_layout());
        assert!(matches!(
            &studio.root,
            SplitNode::Split {
                direction: SplitDirection::SideBySide,
                ..
            }
        ));

        // 3-pane flip: `(1 | (2 / 3))` -> `(1 / (2 | 3))` preserves slot caps accurately.
        assert!(studio.select_slot_by_number(2));
        assert!(studio.split_selected(SplitDirection::Stacked));
        assert_eq!(studio.root.leaf_count(), 3);
        assert!(studio.flip_layout());
        assert!(studio.root.can_split(1, SplitDirection::SideBySide));
        assert!(!studio.root.can_split(1, SplitDirection::Stacked));
        assert!(!studio.root.can_split(2, SplitDirection::SideBySide));
        assert!(!studio.root.can_split(2, SplitDirection::Stacked));

        // Reach 2x2 (`(1 | 4) / (2 | 3)`) and verify double flip restores exact tree.
        assert!(studio.select_slot_by_number(1));
        assert!(studio.split_selected(SplitDirection::SideBySide));
        assert_eq!(studio.root.leaf_count(), 4);
        for leaf in [1, 2, 3, 4] {
            assert!(!studio.root.can_split(leaf, SplitDirection::SideBySide));
            assert!(!studio.root.can_split(leaf, SplitDirection::Stacked));
        }
        let before_2x2 = studio.root.clone();
        assert!(studio.flip_layout());
        assert_ne!(studio.root, before_2x2);
        for leaf in [1, 2, 3, 4] {
            assert!(!studio.root.can_split(leaf, SplitDirection::SideBySide));
            assert!(!studio.root.can_split(leaf, SplitDirection::Stacked));
        }
        assert!(studio.flip_layout());
        assert_eq!(studio.root, before_2x2);
    }

    #[test]
    fn prune_missing_windows_resets_vanished_leaves_to_new_shell() {
        let mut tree = SplitNode::new_leaf(1, SplitSlotFill::Window(wid("@10")));
        assert!(tree.split_slot(
            1,
            SplitDirection::SideBySide,
            2,
            SplitSlotFill::Window(wid("@11"))
        ));
        assert!(tree.split_slot(2, SplitDirection::Stacked, 3, SplitSlotFill::NewShell));

        let live: HashSet<WindowId> = [wid("@10")].into_iter().collect();
        tree.prune_missing_windows(&live);

        assert_eq!(tree.slot_fill(1), Some(&SplitSlotFill::Window(wid("@10"))));
        assert_eq!(tree.slot_fill(2), Some(&SplitSlotFill::NewShell));
        assert_eq!(tree.slot_fill(3), Some(&SplitSlotFill::NewShell));
    }
}
