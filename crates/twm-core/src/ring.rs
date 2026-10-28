//! Software rasteriser for the focus border: an anti-aliased rounded-rect
//! ring filled with a linear gradient, output as premultiplied BGRA pixels
//! ready for `UpdateLayeredWindow`. The centre is fully transparent, so the
//! overlay never hides window content and (with WS_EX_TRANSPARENT) never
//! intercepts input.

use crate::color::Gradient;

/// Signed distance from (px,py) to a rounded rectangle centred at (cx,cy)
/// with half-extents (hx,hy) and corner radius r. Negative inside.
fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let r = r.min(hx).min(hy).max(0.0);
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Renders a `w`x`h` bitmap containing a ring of `thickness` px along the
/// outer edge. `radius` is the outer corner radius. Pixels are packed as
/// 0xAARRGGBB with premultiplied colour (the layout of a 32-bpp BI_RGB DIB).
pub fn render_ring(w: u32, h: u32, thickness: f32, radius: f32, grad: &Gradient, opacity: f32) -> Vec<u32> {
    render_ring_part(w, h, (0, 0, w, h), thickness, radius, grad, opacity)
}

/// The four strips (top, bottom, left, right) that together contain every
/// non-transparent pixel of a `w`x`h` ring, as (x, y, w, h) parts. Drawing
/// these instead of one full-size bitmap keeps memory proportional to the
/// perimeter, not the window area.
pub fn ring_strips(w: u32, h: u32, thickness: f32, radius: f32) -> [(u32, u32, u32, u32); 4] {
    let band = ((thickness.max(radius) + 1.0).ceil() as u32).min(h / 2).max(1);
    let side = ((thickness + 1.0).ceil() as u32).min(w / 2).max(1);
    let mid = h.saturating_sub(2 * band);
    [(0, 0, w, band), (0, h - band, w, band), (0, band, side, mid), (w - side, band, side, mid)]
}

/// Renders only `part` = (x, y, w, h) of the ring described by the full
/// `fw`x`fh` size; the gradient is laid out over the full size so the
/// pieces join seamlessly.
pub fn render_ring_part(fw: u32, fh: u32, part: (u32, u32, u32, u32), thickness: f32, radius: f32, grad: &Gradient, opacity: f32) -> Vec<u32> {
    let (ox, oy, w, h) = part;
    let mut px = vec![0u32; (w as usize) * (h as usize)];
    if w == 0 || h == 0 || fw == 0 || fh == 0 || thickness <= 0.0 {
        return px;
    }
    let (wf, hf) = (fw as f32, fh as f32);
    let (cx, cy) = (wf / 2.0, hf / 2.0);
    let outer_r = radius.max(0.0);
    let inner_r = (radius - thickness).max(0.0);
    // Gradient axis: angle measured clockwise from +x in screen space.
    let a = grad.angle.to_radians();
    let (dx, dy) = (a.cos(), a.sin());
    // Project the corners to normalise t into 0..=1 over the bitmap.
    let proj = |x: f32, y: f32| (x - cx) * dx + (y - cy) * dy;
    let corners = [proj(0.0, 0.0), proj(wf, 0.0), proj(0.0, hf), proj(wf, hf)];
    let pmin = corners.iter().cloned().fold(f32::MAX, f32::min);
    let pmax = corners.iter().cloned().fold(f32::MIN, f32::max);
    let span = (pmax - pmin).max(1.0);
    let t_max = thickness + 1.0;
    for y in 0..h {
        let fy = (y + oy) as f32 + 0.5;
        // Rows away from the corners only have pixels near the left/right edges.
        let row_inner = fy > t_max + outer_r && fy < hf - t_max - outer_r;
        for x in 0..w {
            let fx = (x + ox) as f32 + 0.5;
            if row_inner && fx > t_max && fx < wf - t_max {
                continue;
            }
            let d_out = sd_round_rect(fx, fy, cx, cy, cx, cy, outer_r);
            let d_in = sd_round_rect(fx, fy, cx, cy, cx - thickness, cy - thickness, inner_r);
            // Coverage: inside outer (d_out<0) and outside inner (d_in>0), 1px AA.
            let cov = ((0.5 - d_out).clamp(0.0, 1.0)) * ((0.5 + d_in).clamp(0.0, 1.0));
            if cov <= 0.0 {
                continue;
            }
            let c = grad.sample((proj(fx, fy) - pmin) / span);
            let alpha = (c.a as f32 / 255.0) * cov * opacity.clamp(0.0, 1.0);
            let a8 = (alpha * 255.0).round() as u32;
            let pm = |v: u8| ((v as f32) * alpha).round() as u32;
            px[(y * w + x) as usize] = a8 << 24 | pm(c.r) << 16 | pm(c.g) << 8 | pm(c.b);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Color, Gradient};

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn ring_is_opaque_on_edge_and_clear_inside() {
        let g = Gradient::solid(Color::rgba(255, 0, 0, 255));
        let (w, h) = (100, 60);
        let px = render_ring(w, h, 3.0, 0.0, &g, 1.0);
        let at = |x: u32, y: u32| px[(y * w + x) as usize];
        assert_eq!(alpha(at(1, 30)), 255, "left edge");
        assert_eq!(alpha(at(98, 30)), 255, "right edge");
        assert_eq!(alpha(at(50, 1)), 255, "top edge");
        assert_eq!(alpha(at(50, 30)), 0, "centre transparent");
        assert_eq!(alpha(at(5, 30)), 0, "just inside the ring");
        assert_eq!(at(1, 30), 0xFFFF0000, "premultiplied red");
    }

    #[test]
    fn rounded_corner_is_transparent() {
        let g = Gradient::solid(Color::rgba(255, 255, 255, 255));
        let px = render_ring(100, 100, 2.0, 12.0, &g, 1.0);
        assert_eq!(alpha(px[0]), 0, "outside the rounded corner");
        assert_eq!(alpha(px[50]), 255, "middle of the top edge");
    }

    #[test]
    fn strips_cover_every_visible_pixel_exactly_once() {
        let g = Gradient::parse("rgba(33ccffee) rgba(00ff99ee) 45deg").unwrap();
        for (w, h, t, r) in [(300u32, 200u32, 2.0f32, 10.0f32), (120, 40, 4.0, 0.0), (50, 600, 3.0, 8.0)] {
            let full = render_ring(w, h, t, r, &g, 1.0);
            let mut covered = vec![0u8; (w * h) as usize];
            for part in ring_strips(w, h, t, r) {
                let px = render_ring_part(w, h, part, t, r, &g, 1.0);
                for y in 0..part.3 {
                    for x in 0..part.2 {
                        let (fx, fy) = (part.0 + x, part.1 + y);
                        let i = (fy * w + fx) as usize;
                        covered[i] += 1;
                        assert_eq!(px[(y * part.2 + x) as usize], full[i], "pixel mismatch at {fx},{fy}");
                    }
                }
            }
            for (i, p) in full.iter().enumerate() {
                if *p != 0 {
                    assert_eq!(covered[i], 1, "visible pixel {i} of {w}x{h} not covered exactly once");
                }
                assert!(covered[i] <= 1, "strips overlap at {i}");
            }
        }
    }

    #[test]
    fn gradient_varies_and_opacity_scales() {
        let g = Gradient::parse("rgba(ff0000ff) rgba(0000ffff) 0deg").unwrap();
        let w = 200;
        let px = render_ring(w, 50, 2.0, 0.0, &g, 1.0);
        let left = px[(25 * w) as usize];
        let right = px[(25 * w + w - 1) as usize];
        assert!((left >> 16) & 0xff > 200 && (right & 0xff) > 200, "{left:x} {right:x}");
        let half = render_ring(w, 50, 2.0, 0.0, &g, 0.5);
        assert_eq!(alpha(half[(25 * w) as usize]), 128);
    }
}
