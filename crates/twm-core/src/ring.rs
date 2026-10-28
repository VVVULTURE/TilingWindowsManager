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
    let mut px = vec![0u32; (w as usize) * (h as usize)];
    if w == 0 || h == 0 || thickness <= 0.0 {
        return px;
    }
    let (wf, hf) = (w as f32, h as f32);
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
        let fy = y as f32 + 0.5;
        // Skip the transparent middle rows quickly.
        let row_inner = fy > t_max + outer_r && fy < hf - t_max - outer_r;
        for x in 0..w {
            let fx = x as f32 + 0.5;
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
