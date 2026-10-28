//! Animation timing. Curves are CSS-style cubic Béziers from (0,0) to (1,1)
//! with control points (x1,y1),(x2,y2) — the same model Hyprland uses for
//! its `bezier` definitions.

use crate::geom::Rect;

#[derive(Clone, Copy, Debug)]
pub struct Bezier {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl Bezier {
    pub fn new(p: [f32; 4]) -> Bezier {
        Bezier { x1: p[0] as f64, y1: p[1] as f64, x2: p[2] as f64, y2: p[3] as f64 }
    }

    fn coord(t: f64, p1: f64, p2: f64) -> f64 {
        // B(t) = 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    }

    fn dx(&self, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * self.x1 + 6.0 * u * t * (self.x2 - self.x1) + 3.0 * t * t * (1.0 - self.x2)
    }

    /// Progress (y) at time fraction `x` in 0..=1.
    pub fn eval(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        // Newton-Raphson, falling back to bisection (x(t) is monotonic
        // because x1, x2 are validated to lie in 0..=1).
        let mut t = x;
        for _ in 0..8 {
            let err = Self::coord(t, self.x1, self.x2) - x;
            if err.abs() < 1e-7 {
                return Self::coord(t, self.y1, self.y2);
            }
            let d = self.dx(t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= err / d;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        t = x;
        for _ in 0..60 {
            let v = Self::coord(t, self.x1, self.x2);
            if (v - x).abs() < 1e-7 {
                break;
            }
            if v < x {
                lo = t
            } else {
                hi = t
            }
            t = (lo + hi) / 2.0;
        }
        Self::coord(t, self.y1, self.y2)
    }
}

/// One window's movement from `from` to `to`.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    pub from: Rect,
    pub to: Rect,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub curve: Bezier,
}

impl Tween {
    /// Rectangle at time `now_ms`, and whether the tween has finished.
    pub fn at(&self, now_ms: f64) -> (Rect, bool) {
        if self.duration_ms <= 0.0 {
            return (self.to, true);
        }
        let x = ((now_ms - self.start_ms) / self.duration_ms).clamp(0.0, 1.0);
        if x >= 1.0 {
            return (self.to, true);
        }
        (Rect::lerp(&self.from, &self.to, self.curve.eval(x)), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_is_identity() {
        let b = Bezier::new([0.0, 0.0, 1.0, 1.0]);
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((b.eval(x) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn ease_matches_css_reference() {
        // CSS `ease` = cubic-bezier(0.25, 0.1, 0.25, 1); at x=0.5 y≈0.8024
        let b = Bezier::new([0.25, 0.1, 0.25, 1.0]);
        assert!((b.eval(0.5) - 0.8024).abs() < 1e-3, "{}", b.eval(0.5));
        assert_eq!(b.eval(0.0), 0.0);
        assert_eq!(b.eval(1.0), 1.0);
    }

    #[test]
    fn monotonic_for_smooth_curve() {
        let b = Bezier::new([0.05, 0.9, 0.1, 1.0]);
        let mut prev = 0.0;
        for i in 1..=100 {
            let y = b.eval(i as f64 / 100.0);
            assert!(y >= prev - 1e-9);
            prev = y;
        }
    }

    #[test]
    fn overshoot_allowed() {
        let b = Bezier::new([0.05, 0.9, 0.1, 1.1]);
        assert!((0..100).any(|i| b.eval(i as f64 / 100.0) > 1.0));
    }

    #[test]
    fn tween_endpoints() {
        let t = Tween { from: Rect::new(0, 0, 100, 100), to: Rect::new(100, 0, 200, 100), start_ms: 10.0, duration_ms: 100.0, curve: Bezier::new([0.0, 0.0, 1.0, 1.0]) };
        assert_eq!(t.at(10.0), (Rect::new(0, 0, 100, 100), false));
        assert_eq!(t.at(60.0).0, Rect::new(50, 0, 150, 100));
        assert_eq!(t.at(500.0), (Rect::new(100, 0, 200, 100), true));
        let zero = Tween { duration_ms: 0.0, ..t };
        assert!(zero.at(0.0).1);
    }
}
