//! Dwindle: a binary-space-partition layout modelled on Hyprland's
//! `DwindleAlgorithm` (src/layout/algorithm/tiled/dwindle). Every window is a
//! leaf; every inner node splits its rectangle in two. A new window splits
//! the focused window's leaf. The split axis is chosen from the node's
//! aspect ratio each time the layout is computed (`w > h * multiplier` means
//! side-by-side) unless `preserve_split` pinned it. The ratio follows
//! Hyprland's convention: 1.0 is an even split, valid range 0.1..=1.9, and the
//! first child receives `size / 2 * ratio`.

use crate::geom::{Direction, Rect};
use crate::WinId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// Children side by side (left | right).
    Horizontal,
    /// Children stacked (top / bottom).
    Vertical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Node {
    Leaf(WinId),
    Split {
        children: Box<[Node; 2]>,
        ratio: f32,
        /// Axis fixed by preserve_split/togglesplit; `None` = dynamic.
        fixed_axis: Option<Axis>,
        /// Axis used by the most recent `compute` (cache for resize logic).
        last_axis: Axis,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Dwindle {
    pub root: Option<Node>,
}

#[derive(Clone, Copy, Debug)]
pub struct DwindleOptions {
    pub split_width_multiplier: f32,
    pub default_split_ratio: f32,
    pub preserve_split: bool,
    /// 0 = new window goes second (right/bottom), 1 = first (left/top).
    /// Hyprland's 0 follows the mouse; without a cursor-driven placement we
    /// treat 0 and 2 the same.
    pub force_split: u8,
}

impl Default for DwindleOptions {
    fn default() -> Self {
        DwindleOptions { split_width_multiplier: 1.0, default_split_ratio: 1.0, preserve_split: false, force_split: 0 }
    }
}

const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 1.9;

impl Node {
    fn contains(&self, w: WinId) -> bool {
        match self {
            Node::Leaf(x) => *x == w,
            Node::Split { children, .. } => children[0].contains(w) || children[1].contains(w),
        }
    }

    fn collect(&self, out: &mut Vec<WinId>) {
        match self {
            Node::Leaf(x) => out.push(*x),
            Node::Split { children, .. } => {
                children[0].collect(out);
                children[1].collect(out);
            }
        }
    }

    fn first_leaf(&self) -> WinId {
        match self {
            Node::Leaf(x) => *x,
            Node::Split { children, .. } => children[0].first_leaf(),
        }
    }
}

fn axis_for(r: &Rect, mult: f32) -> Axis {
    if r.w as f32 > r.h as f32 * mult { Axis::Horizontal } else { Axis::Vertical }
}

fn split_rect(r: &Rect, axis: Axis, ratio: f32) -> (Rect, Rect) {
    match axis {
        Axis::Horizontal => {
            let first = ((r.w as f32 / 2.0) * ratio).round() as i32;
            (Rect::new(r.x, r.y, first, r.h), Rect::new(r.x + first, r.y, r.w - first, r.h))
        }
        Axis::Vertical => {
            let first = ((r.h as f32 / 2.0) * ratio).round() as i32;
            (Rect::new(r.x, r.y, r.w, first), Rect::new(r.x, r.y + first, r.w, r.h - first))
        }
    }
}

impl Dwindle {
    pub fn windows(&self) -> Vec<WinId> {
        let mut v = Vec::new();
        if let Some(r) = &self.root {
            r.collect(&mut v);
        }
        v
    }

    pub fn contains(&self, w: WinId) -> bool {
        self.root.as_ref().is_some_and(|r| r.contains(w))
    }

    /// Inserts `w` by splitting the leaf of `target` (or the first leaf if
    /// `target` is absent). `area` is used to pick the axis when
    /// `preserve_split` is enabled, matching Hyprland's behaviour of fixing the
    /// axis at creation time.
    pub fn insert(&mut self, w: WinId, target: Option<WinId>, area: Rect, opts: &DwindleOptions) {
        if self.contains(w) {
            return;
        }
        let Some(root) = self.root.as_mut() else {
            self.root = Some(Node::Leaf(w));
            return;
        };
        let target = target.filter(|t| root.contains(*t)).unwrap_or_else(|| root.first_leaf());
        let ratio = opts.default_split_ratio.clamp(MIN_RATIO, MAX_RATIO);
        // Find the target leaf and its rectangle so a fixed axis can be chosen.
        fn walk(n: &mut Node, r: Rect, target: WinId, w: WinId, ratio: f32, opts: &DwindleOptions) -> bool {
            match n {
                Node::Leaf(x) if *x == target => {
                    let axis = axis_for(&r, opts.split_width_multiplier);
                    let (a, b) = if opts.force_split == 1 { (Node::Leaf(w), Node::Leaf(target)) } else { (Node::Leaf(target), Node::Leaf(w)) };
                    *n = Node::Split {
                        children: Box::new([a, b]),
                        ratio,
                        fixed_axis: opts.preserve_split.then_some(axis),
                        last_axis: axis,
                    };
                    true
                }
                Node::Leaf(_) => false,
                Node::Split { children, ratio: nr, fixed_axis, .. } => {
                    let axis = fixed_axis.unwrap_or_else(|| axis_for(&r, opts.split_width_multiplier));
                    let (ra, rb) = split_rect(&r, axis, *nr);
                    let [c0, c1] = &mut **children;
                    walk(c0, ra, target, w, ratio, opts) || walk(c1, rb, target, w, ratio, opts)
                }
            }
        }
        walk(root, area, target, w, ratio, opts);
    }

    /// Removes `w`; its sibling takes over the parent's space.
    pub fn remove(&mut self, w: WinId) -> bool {
        fn rm(n: Node, w: WinId) -> (Option<Node>, bool) {
            match n {
                Node::Leaf(x) if x == w => (None, true),
                Node::Leaf(_) => (Some(n), false),
                Node::Split { children, ratio, fixed_axis, last_axis } => {
                    let [c0, c1] = *children;
                    let (a, ra) = rm(c0, w);
                    let (b, rb) = if ra { (Some(c1), false) } else { rm(c1, w) };
                    let node = match (a, b) {
                        (Some(a), Some(b)) => Some(Node::Split { children: Box::new([a, b]), ratio, fixed_axis, last_axis }),
                        (Some(x), None) | (None, Some(x)) => Some(x),
                        (None, None) => None,
                    };
                    (node, ra || rb)
                }
            }
        }
        let Some(root) = self.root.take() else { return false };
        let (r, removed) = rm(root, w);
        self.root = r;
        removed
    }

    /// Computes the tile rectangle of every window.
    pub fn compute(&mut self, area: Rect, opts: &DwindleOptions) -> Vec<(WinId, Rect)> {
        fn go(n: &mut Node, r: Rect, opts: &DwindleOptions, out: &mut Vec<(WinId, Rect)>) {
            match n {
                Node::Leaf(w) => out.push((*w, r)),
                Node::Split { children, ratio, fixed_axis, last_axis } => {
                    let axis = fixed_axis.unwrap_or_else(|| axis_for(&r, opts.split_width_multiplier));
                    *last_axis = axis;
                    let (a, b) = split_rect(&r, axis, *ratio);
                    let [c0, c1] = &mut **children;
                    go(c0, a, opts, out);
                    go(c1, b, opts, out);
                }
            }
        }
        let mut out = Vec::new();
        if let Some(r) = self.root.as_mut() {
            go(r, area, opts, &mut out);
        }
        out
    }

    pub fn swap(&mut self, a: WinId, b: WinId) {
        fn go(n: &mut Node, a: WinId, b: WinId) {
            match n {
                Node::Leaf(x) if *x == a => *x = b,
                Node::Leaf(x) if *x == b => *x = a,
                Node::Leaf(_) => {}
                Node::Split { children, .. } => {
                    let [c0, c1] = &mut **children;
                    go(c0, a, b);
                    go(c1, a, b);
                }
            }
        }
        if let Some(r) = self.root.as_mut() {
            go(r, a, b);
        }
    }

    /// Returns the split node that is the direct parent of `w`'s leaf.
    fn parent_of(&mut self, w: WinId) -> Option<&mut Node> {
        fn go(n: &mut Node, w: WinId) -> Option<&mut Node> {
            let is_parent = matches!(n, Node::Split { children, .. }
                if matches!(children[0], Node::Leaf(x) if x == w) || matches!(children[1], Node::Leaf(x) if x == w));
            if is_parent {
                return Some(n);
            }
            match n {
                Node::Split { children, .. } => {
                    let [c0, c1] = &mut **children;
                    if c0.contains(w) { go(c0, w) } else if c1.contains(w) { go(c1, w) } else { None }
                }
                Node::Leaf(_) => None,
            }
        }
        self.root.as_mut().and_then(|r| go(r, w))
    }

    /// Toggles the split axis of `w`'s parent and pins it (Hyprland
    /// `togglesplit`).
    pub fn toggle_split(&mut self, w: WinId) -> bool {
        if let Some(Node::Split { fixed_axis, last_axis, .. }) = self.parent_of(w) {
            let next = match fixed_axis.unwrap_or(*last_axis) {
                Axis::Horizontal => Axis::Vertical,
                Axis::Vertical => Axis::Horizontal,
            };
            *fixed_axis = Some(next);
            *last_axis = next;
            return true;
        }
        false
    }

    /// Adds `delta` to the ratio of `w`'s parent split (Hyprland
    /// `splitratio`), or sets it when `exact` is true.
    pub fn split_ratio(&mut self, w: WinId, delta: f32, exact: bool) -> bool {
        if let Some(Node::Split { ratio, .. }) = self.parent_of(w) {
            *ratio = if exact { delta } else { *ratio + delta }.clamp(MIN_RATIO, MAX_RATIO);
            return true;
        }
        false
    }

    /// Grows `w` towards `dir` by `frac` of the relevant split's size by
    /// adjusting the nearest ancestor split along that axis whose shared edge
    /// lies in that direction. Requires a preceding `compute`.
    pub fn resize(&mut self, w: WinId, dir: Direction, frac: f32) -> bool {
        // Walk from root, remembering the deepest qualifying ancestor.
        fn go(n: &mut Node, w: WinId, dir: Direction, frac: f32) -> Option<bool> {
            match n {
                Node::Leaf(x) => (*x == w).then_some(false),
                Node::Split { children, ratio, last_axis, .. } => {
                    let in_first = children[0].contains(w);
                    if !in_first && !children[1].contains(w) {
                        return None;
                    }
                    let axis = *last_axis;
                    let [c0, c1] = &mut **children;
                    let handled = if in_first { go(c0, w, dir, frac)? } else { go(c1, w, dir, frac)? };
                    if handled {
                        return Some(true);
                    }
                    let along = matches!((axis, dir), (Axis::Horizontal, Direction::Left | Direction::Right) | (Axis::Vertical, Direction::Up | Direction::Down));
                    // The shared edge is to the right/bottom of the first child.
                    let edge_ahead = in_first == matches!(dir, Direction::Right | Direction::Down);
                    if along && edge_ahead {
                        let d = if in_first { frac } else { -frac } * 2.0; // ratio is relative to half the size
                        *ratio = (*ratio + d).clamp(MIN_RATIO, MAX_RATIO);
                        return Some(true);
                    }
                    Some(false)
                }
            }
        }
        self.root.as_mut().and_then(|r| go(r, w, dir, frac)).unwrap_or(false)
    }

    /// Sets ratios so that `w`'s tile edges match `target` as closely as the
    /// tree allows (used after the user drags a tile's border with the mouse).
    pub fn fit_to(&mut self, w: WinId, area: Rect, target: Rect, opts: &DwindleOptions) {
        fn go(n: &mut Node, r: Rect, w: WinId, t: &Rect, opts: &DwindleOptions) {
            if let Node::Split { children, ratio, fixed_axis, .. } = n {
                let axis = fixed_axis.unwrap_or_else(|| axis_for(&r, opts.split_width_multiplier));
                let in_first = children[0].contains(w);
                if !in_first && !children[1].contains(w) {
                    return;
                }
                // Position of the shared edge requested by the target rect.
                let (start, size, edge) = match (axis, in_first) {
                    (Axis::Horizontal, true) => (r.x, r.w, t.right()),
                    (Axis::Horizontal, false) => (r.x, r.w, t.x),
                    (Axis::Vertical, true) => (r.y, r.h, t.bottom()),
                    (Axis::Vertical, false) => (r.y, r.h, t.y),
                };
                if size > 0 {
                    let want = (edge - start) as f32 / (size as f32 / 2.0);
                    if (want - *ratio).abs() > 0.01 {
                        *ratio = want.clamp(MIN_RATIO, MAX_RATIO);
                    }
                }
                let (a, b) = split_rect(&r, axis, *ratio);
                let [c0, c1] = &mut **children;
                if in_first { go(c0, a, w, t, opts) } else { go(c1, b, w, t, opts) }
            }
        }
        if let Some(r) = self.root.as_mut() {
            go(r, area, w, &target, opts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1600, 900);

    fn build(n: u64) -> Dwindle {
        let mut d = Dwindle::default();
        let o = DwindleOptions::default();
        for i in 1..=n {
            // Hyprland splits the focused window; new windows take focus.
            d.insert(i, if i > 1 { Some(i - 1) } else { None }, AREA, &o);
        }
        d
    }

    #[test]
    fn first_window_fills_area() {
        let mut d = build(1);
        assert_eq!(d.compute(AREA, &DwindleOptions::default()), vec![(1, AREA)]);
    }

    #[test]
    fn spiral_alternates_axes_by_aspect() {
        let mut d = build(3);
        let r = d.compute(AREA, &DwindleOptions::default());
        // 1600x900 is wide -> side by side; 800x900 is tall -> stacked.
        assert_eq!(r[0], (1, Rect::new(0, 0, 800, 900)));
        assert_eq!(r[1], (2, Rect::new(800, 0, 800, 450)));
        assert_eq!(r[2], (3, Rect::new(800, 450, 800, 450)));
    }

    #[test]
    fn tiles_cover_area_exactly() {
        for n in 1..=9 {
            let mut d = build(n);
            let r = d.compute(AREA, &DwindleOptions::default());
            assert_eq!(r.len() as u64, n);
            let total: i64 = r.iter().map(|(_, r)| r.area()).sum();
            assert_eq!(total, AREA.area(), "n={n}");
            for (i, (_, a)) in r.iter().enumerate() {
                for (_, b) in r.iter().skip(i + 1) {
                    assert!(a.intersection(b).is_none(), "overlap n={n}");
                }
            }
        }
    }

    #[test]
    fn remove_gives_space_to_sibling() {
        let mut d = build(3);
        assert!(d.remove(2));
        let r = d.compute(AREA, &DwindleOptions::default());
        assert_eq!(r, vec![(1, Rect::new(0, 0, 800, 900)), (3, Rect::new(800, 0, 800, 900))]);
        assert!(d.remove(1));
        assert!(d.remove(3));
        assert!(d.root.is_none());
        assert!(!d.remove(3));
    }

    #[test]
    fn remove_first_child_subtree() {
        // 1 | (2 / 3), then split 1 -> (1 / 4): removing 1 keeps 4 in its subtree.
        let mut d = build(3);
        d.insert(4, Some(1), AREA, &DwindleOptions::default());
        assert!(d.remove(1));
        let ids: Vec<_> = d.windows();
        assert_eq!(ids, vec![4, 2, 3]);
    }

    #[test]
    fn force_split_first_places_new_window_left() {
        let mut d = Dwindle::default();
        let o = DwindleOptions { force_split: 1, ..Default::default() };
        d.insert(1, None, AREA, &o);
        d.insert(2, Some(1), AREA, &o);
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].0, 2);
        assert_eq!(r[0].1.x, 0);
    }

    #[test]
    fn ratio_and_toggle() {
        let mut d = build(2);
        let o = DwindleOptions::default();
        d.compute(AREA, &o);
        assert!(d.split_ratio(1, 0.5, false));
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].1.w, 1200); // 1600/2*1.5
        assert!(d.split_ratio(1, 5.0, true));
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].1.w, (800.0 * 1.9f32).round() as i32, "clamped to 1.9");
        assert!(d.toggle_split(1));
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].1.w, 1600, "now stacked");
    }

    #[test]
    fn swap_exchanges_positions() {
        let mut d = build(2);
        d.swap(1, 2);
        let r = d.compute(AREA, &DwindleOptions::default());
        assert_eq!(r[0].0, 2);
        assert_eq!(r[1].0, 1);
    }

    #[test]
    fn directional_resize_moves_shared_edge() {
        let mut d = build(3);
        let o = DwindleOptions::default();
        d.compute(AREA, &o);
        // Window 1 is the left half: growing right moves the root split.
        assert!(d.resize(1, Direction::Right, 0.1));
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].1.w, 960);
        // Window 3 (bottom-right) growing up moves the 2/3 split.
        assert!(d.resize(3, Direction::Up, 0.1));
        let r = d.compute(AREA, &o);
        assert!(r[2].1.h > 450);
        // Window 1 cannot grow left: there is no edge there.
        assert!(!d.resize(1, Direction::Left, 0.1));
    }

    #[test]
    fn fit_to_matches_dragged_edge() {
        let mut d = build(2);
        let o = DwindleOptions::default();
        d.compute(AREA, &o);
        d.fit_to(1, AREA, Rect::new(0, 0, 1000, 900), &o);
        let r = d.compute(AREA, &o);
        assert_eq!(r[0].1.w, 1000);
        assert_eq!(r[1].1.x, 1000);
    }
}
