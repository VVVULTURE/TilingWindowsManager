//! Integer screen geometry. Coordinates are physical pixels in the Windows
//! virtual-screen space (the primary monitor's top-left is 0,0; other
//! monitors may have negative coordinates).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn from_ltrb(l: i32, t: i32, r: i32, b: i32) -> Self {
        Rect { x: l, y: t, w: r - l, h: b - t }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn area(&self) -> i64 {
        self.w.max(0) as i64 * self.h.max(0) as i64
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > l && b > t).then(|| Rect::from_ltrb(l, t, r, b))
    }

    /// Shrinks each side by the given amount; never produces negative sizes.
    pub fn inset(&self, l: i32, t: i32, r: i32, b: i32) -> Rect {
        let w = (self.w - l - r).max(1);
        let h = (self.h - t - b).max(1);
        Rect { x: self.x + l, y: self.y + t, w, h }
    }

    pub fn shrink(&self, n: i32) -> Rect {
        self.inset(n, n, n, n)
    }

    /// Linear interpolation between two rectangles (t in 0..=1).
    pub fn lerp(a: &Rect, b: &Rect, t: f64) -> Rect {
        let f = |x: i32, y: i32| (x as f64 + (y - x) as f64 * t).round() as i32;
        Rect { x: f(a.x, b.x), y: f(a.y, b.y), w: f(a.w, b.w), h: f(a.h, b.h) }
    }

    /// A rectangle of the given size centred inside `self`.
    pub fn centered(&self, w: i32, h: i32) -> Rect {
        let w = w.min(self.w);
        let h = h.min(self.h);
        Rect { x: self.x + (self.w - w) / 2, y: self.y + (self.h - h) / 2, w, h }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Direction> {
        match s.to_ascii_lowercase().as_str() {
            "l" | "left" => Some(Direction::Left),
            "r" | "right" => Some(Direction::Right),
            "u" | "t" | "up" | "top" => Some(Direction::Up),
            "d" | "b" | "down" | "bottom" => Some(Direction::Down),
            _ => None,
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Direction::Left | Direction::Right)
    }
}

/// Finds the rectangle that lies in `dir` from `from`, Hyprland-style: the
/// candidate must be on that side and overlap on the perpendicular axis if
/// possible; the nearest edge wins, and among equal distances the one with
/// the largest perpendicular overlap wins. Returns the index into `cands`.
pub fn find_in_direction(from: &Rect, dir: Direction, cands: &[Rect]) -> Option<usize> {
    const SLOP: i32 = 4; // tolerate gaps/rounding between adjacent tiles
    let mut best: Option<(i64, i64, usize)> = None; // (distance, -overlap, idx)
    for (i, c) in cands.iter().enumerate() {
        let (dist, overlap) = match dir {
            Direction::Left if c.right() <= from.x + SLOP => {
                (from.x - c.right(), overlap_1d(from.y, from.bottom(), c.y, c.bottom()))
            }
            Direction::Right if c.x >= from.right() - SLOP => {
                (c.x - from.right(), overlap_1d(from.y, from.bottom(), c.y, c.bottom()))
            }
            Direction::Up if c.bottom() <= from.y + SLOP => {
                (from.y - c.bottom(), overlap_1d(from.x, from.right(), c.x, c.right()))
            }
            Direction::Down if c.y >= from.bottom() - SLOP => {
                (c.y - from.bottom(), overlap_1d(from.x, from.right(), c.x, c.right()))
            }
            _ => continue,
        };
        // Candidates without perpendicular overlap are only used as a fallback:
        // penalise them heavily so an overlapping neighbour always wins.
        let dist = dist.max(0) as i64 + if overlap > 0 { 0 } else { 1_000_000 };
        let key = (dist, -(overlap as i64), i);
        if best.is_none_or(|b| key < b) {
            best = Some(key);
        }
    }
    best.map(|b| b.2)
}

fn overlap_1d(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_and_lerp() {
        let r = Rect::new(0, 0, 100, 50);
        assert_eq!(r.shrink(10), Rect::new(10, 10, 80, 30));
        assert_eq!(r.inset(200, 0, 0, 0).w, 1, "never negative");
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(100, 50, 30, 10);
        assert_eq!(Rect::lerp(&a, &b, 0.0), a);
        assert_eq!(Rect::lerp(&a, &b, 1.0), b);
        assert_eq!(Rect::lerp(&a, &b, 0.5), Rect::new(50, 25, 20, 10));
    }

    #[test]
    fn direction_search_prefers_overlapping_neighbour() {
        // [A][B]
        //    [C]
        let a = Rect::new(0, 0, 100, 200);
        let b = Rect::new(100, 0, 100, 100);
        let c = Rect::new(100, 100, 100, 100);
        let cands = [a, b, c];
        assert_eq!(find_in_direction(&a, Direction::Right, &cands), Some(1));
        assert_eq!(find_in_direction(&c, Direction::Left, &cands), Some(0));
        assert_eq!(find_in_direction(&b, Direction::Down, &cands), Some(2));
        assert_eq!(find_in_direction(&c, Direction::Up, &cands), Some(1));
        assert_eq!(find_in_direction(&a, Direction::Left, &cands), None);
    }

    #[test]
    fn direction_search_tolerates_gaps() {
        let a = Rect::new(10, 10, 100, 100);
        let b = Rect::new(120, 10, 100, 100); // 10px gap
        assert_eq!(find_in_direction(&a, Direction::Right, &[a, b]), Some(1));
    }
}
