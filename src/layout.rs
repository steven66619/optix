/// Tabs and split-pane layout tree.
///
/// A tab owns a binary tree of splits; leaves are pane IDs that reference the
/// application's pane table.
pub type PaneId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    /// Side by side (a vertical divider).
    Horizontal,
    /// Stacked (a horizontal divider).
    Vertical,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

#[derive(Debug)]
pub struct SplitBox {
    pub orientation: Orientation,
    pub ratio: f32,
    pub first: Box<LayoutNode>,
    pub second: Box<LayoutNode>,
}

#[derive(Debug)]
pub enum LayoutNode {
    Pane { id: PaneId },
    Split(SplitBox),
}

impl LayoutNode {
    pub fn pane_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Pane { id } => out.push(*id),
            Self::Split(split) => {
                split.first.pane_ids(out);
                split.second.pane_ids(out);
            },
        }
    }

    /// Split the leaf pane `pane` into two leaves along `orientation`.
    fn split_leaf(&mut self, pane: PaneId, new_id: PaneId, orientation: Orientation) -> bool {
        match self {
            Self::Pane { id } => {
                if *id == pane {
                    *self = Self::Split(SplitBox {
                        orientation,
                        ratio: 0.5,
                        first: Box::new(Self::Pane { id: *id }),
                        second: Box::new(Self::Pane { id: new_id }),
                    });
                    true
                } else {
                    false
                }
            },
            Self::Split(split) => {
                split.first.split_leaf(pane, new_id, orientation)
                    || split.second.split_leaf(pane, new_id, orientation)
            },
        }
    }

    /// Remove the leaf pane `pane`, collapsing splits that lose a child.
    fn remove_pane(&mut self, pane: PaneId) -> bool {
        match self {
            Self::Pane { id } => *id == pane,
            Self::Split(split) => {
                let first_removed = split.first.remove_pane(pane);
                if first_removed && matches!(&*split.first, Self::Pane { .. }) {
                    // The pane leaf under `first` was removed; promote `second`.
                    *self = std::mem::replace(&mut *split.second, Self::Pane { id: usize::MAX });
                    return true;
                }
                if first_removed {
                    return true;
                }
                let second_removed = split.second.remove_pane(pane);
                if second_removed && matches!(&*split.second, Self::Pane { .. }) {
                    *self = std::mem::replace(&mut *split.first, Self::Pane { id: usize::MAX });
                    return true;
                }
                second_removed
            },
        }
    }
}

impl Clone for LayoutNode {
    fn clone(&self) -> Self {
        match self {
            Self::Pane { id } => Self::Pane { id: *id },
            Self::Split(split) => Self::Split(SplitBox {
                orientation: split.orientation,
                ratio: split.ratio,
                first: split.first.clone(),
                second: split.second.clone(),
            }),
        }
    }
}

/// A single pane tree holding the split layout.
#[derive(Debug)]
pub struct Tab {
    pub nodes: LayoutNode,
    pub focused: PaneId,
}

impl Tab {
    pub fn new(pane: PaneId) -> Self {
        Self { nodes: LayoutNode::Pane { id: pane }, focused: pane }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.nodes.pane_ids(&mut out);
        out
    }

    pub fn split(&mut self, pane: PaneId, new_id: PaneId, orientation: Orientation) -> bool {
        self.nodes.split_leaf(pane, new_id, orientation)
    }

    /// Remove `pane`; returns `true` only when the tab becomes empty (last pane).
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let ids_before = self.pane_ids();
        if ids_before.len() == 1 && ids_before[0] == pane {
            return true;
        }
        self.nodes.remove_pane(pane);
        self.focus_something();
        false
    }

    fn focus_something(&mut self) {
        if self.pane_ids().contains(&self.focused) {
            return;
        }
        if let Some(&first) = self.pane_ids().first() {
            self.focused = first;
        }
    }

    pub fn focus(&mut self, pane: PaneId) {
        if self.pane_ids().contains(&pane) {
            self.focused = pane;
        }
    }

    pub fn focus_next(&mut self, wrap: bool) {
        let ids = self.pane_ids();
        if ids.len() < 2 {
            return;
        }
        let idx = ids.iter().position(|&p| p == self.focused).unwrap_or(0);
        let next = if idx + 1 < ids.len() { idx + 1 } else if wrap { 0 } else { idx };
        self.focused = ids[next];
    }

    pub fn focus_prev(&mut self, wrap: bool) {
        let ids = self.pane_ids();
        if ids.len() < 2 {
            return;
        }
        let idx = ids.iter().position(|&p| p == self.focused).unwrap_or(0);
        let next = if idx > 0 { idx - 1 } else if wrap { ids.len() - 1 } else { 0 };
        self.focused = ids[next];
    }

    /// Compute pixel rectangles for every pane given the tab's client area.
    pub fn layout_rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.compute(&self.nodes, area, &mut out);
        out
    }

    /// 1px divider line rects for every split, used to draw pane borders.
    pub fn split_borders(&self, area: Rect) -> Vec<Rect> {
        let mut out = Vec::new();
        self.borders(&self.nodes, area, &mut out);
        out
    }

    fn borders(&self, node: &LayoutNode, rect: Rect, out: &mut Vec<Rect>) {
        let LayoutNode::Split(split) = node else { return };
        let (first_rect, second_rect) = match split.orientation {
            Orientation::Horizontal => {
                let w = rect.w * split.ratio;
                out.push(Rect { x: rect.x + w, y: rect.y, w: 1.0, h: rect.h });
                (Rect { w, ..rect }, Rect { x: rect.x + w, w: rect.w - w, ..rect })
            },
            Orientation::Vertical => {
                let h = rect.h * split.ratio;
                out.push(Rect { x: rect.x, y: rect.y + h, w: rect.w, h: 1.0 });
                (Rect { h, ..rect }, Rect { y: rect.y + h, h: rect.h - h, ..rect })
            },
        };
        self.borders(&split.first, first_rect, out);
        self.borders(&split.second, second_rect, out);
    }

    fn compute(&self, node: &LayoutNode, rect: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match node {
            LayoutNode::Pane { id } => out.push((*id, rect)),
            LayoutNode::Split(split) => {
                let (first_rect, second_rect) = match split.orientation {
                    Orientation::Horizontal => {
                        let w = rect.w * split.ratio;
                        (Rect { w, ..rect }, Rect { x: rect.x + w, w: rect.w - w, ..rect })
                    },
                    Orientation::Vertical => {
                        let h = rect.h * split.ratio;
                        (Rect { h, ..rect }, Rect { y: rect.y + h, h: rect.h - h, ..rect })
                    },
                };
                self.compute(&split.first, first_rect, out);
                self.compute(&split.second, second_rect, out);
            },
        }
    }

    /// Move focus to the pane nearest `direction` from the current focus.
    pub fn focus_direction(&mut self, dir: Direction) {
        let area = Rect { x: 0.0, y: 0.0, w: f32::MAX, h: f32::MAX };
        let rects = self.layout_rects(area);
        let Some(&(_, current_rect)) = rects.iter().find(|(id, _)| *id == self.focused) else { return };
        let (cx, cy) = current_rect.center();

        let mut best: Option<(f32, PaneId)> = None;
        for (id, rect) in &rects {
            if *id == self.focused {
                continue;
            }
            let (px, py) = rect.center();
            let (dx, dy) = (px - cx, py - cy);
            let in_dir = match dir {
                Direction::Up => dy < 0.0 && dx.abs() <= dy.abs() + 1.0,
                Direction::Down => dy > 0.0 && dx.abs() <= dy.abs() + 1.0,
                Direction::Left => dx < 0.0 && dy.abs() <= dx.abs() + 1.0,
                Direction::Right => dx > 0.0 && dy.abs() <= dx.abs() + 1.0,
            };
            if !in_dir {
                continue;
            }
            let dist = dx * dx + dy * dy;
            if best.map(|(d, _)| dist < d).unwrap_or(true) {
                best = Some((dist, *id));
            }
        }
        if let Some((_, id)) = best {
            self.focused = id;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug)]
pub struct Layout {
    pub tab: Tab,
}

impl Layout {
    pub fn new(pane: PaneId) -> Self {
        Self { tab: Tab::new(pane) }
    }

    pub fn tab(&self) -> &Tab {
        &self.tab
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tab
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.tab.pane_ids()
    }

    pub fn focused(&self) -> PaneId {
        self.tab.focused
    }

    /// True if `pane` still exists in the layout.
    pub fn contains(&self, pane: PaneId) -> bool {
        self.tab.pane_ids().contains(&pane)
    }

    /// Repair focus if the focused pane was removed.
    pub fn refresh_focus(&mut self) {
        self.tab.focus_something();
    }
}
