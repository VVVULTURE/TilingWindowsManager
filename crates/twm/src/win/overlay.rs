//! Border overlays. Each border is four small layered strip windows (top,
//! bottom, left, right) around a managed window, drawn with per-pixel alpha
//! via UpdateLayeredWindow. They are WS_EX_TRANSPARENT (input passes
//! through), WS_EX_NOACTIVATE and WS_EX_TOOLWINDOW (never focused, not in
//! Alt+Tab or the taskbar), and are kept directly below their window in the
//! z-order so other windows correctly cover them.

use std::ffi::c_void;
use twm_core::color::Gradient;
use twm_core::geom::Rect;
use twm_core::ring::{render_ring_part, ring_strips};
use twm_core::WinId;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: PCWSTR = w!("TwmBorderStrip");

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_NCHITTEST {
            return LRESULT(HTTRANSPARENT as isize);
        }
        DefWindowProcW(h, m, wp, lp)
    }
}

pub fn register_class() {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

fn create_strip() -> HWND {
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
        .unwrap_or_default()
    }
}

/// Pushes premultiplied BGRA pixels to a layered window at `pos`.
fn upload(h: HWND, pos: (i32, i32), size: (u32, u32), pixels: &[u32]) {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.0 as i32,
                biHeight: -(size.1 as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        if let Ok(dib) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
            }
            let old = SelectObject(mem, dib.into());
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pt = POINT { x: pos.0, y: pos.1 };
            let sz = SIZE { cx: size.0 as i32, cy: size.1 as i32 };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(h, Some(screen), Some(&pt), Some(&sz), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mem, old);
            let _ = DeleteObject(dib.into());
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

#[derive(Clone, PartialEq)]
struct Style {
    thickness: i32,
    radius: i32,
    grad: Gradient,
}

pub struct Border {
    pub target: WinId,
    strips: [HWND; 4],
    size: (i32, i32),
    style: Option<Style>,
    visible: bool,
    pos: (i32, i32),
}

impl Border {
    pub fn new(target: WinId) -> Border {
        Border { target, strips: std::array::from_fn(|_| create_strip()), size: (0, 0), style: None, visible: false, pos: (i32::MIN, i32::MIN) }
    }

    /// Shows/updates the ring whose outer edge is `outer`.
    pub fn update(&mut self, outer: Rect, thickness: i32, radius: i32, grad: &Gradient) {
        if outer.w <= 2 * thickness || outer.h <= 2 * thickness || thickness <= 0 {
            self.hide();
            return;
        }
        let style = Style { thickness, radius, grad: grad.clone() };
        let size = (outer.w, outer.h);
        let parts = ring_strips(outer.w as u32, outer.h as u32, thickness as f32, radius as f32);
        if self.size != size || self.style.as_ref() != Some(&style) {
            for (h, p) in self.strips.iter().zip(parts) {
                let px = render_ring_part(outer.w as u32, outer.h as u32, p, thickness as f32, radius as f32, grad, 1.0);
                upload(*h, (outer.x + p.0 as i32, outer.y + p.1 as i32), (p.2.max(1), p.3.max(1)), &px);
            }
            self.size = size;
            self.style = Some(style);
            self.pos = (outer.x, outer.y);
            self.restack(true);
        } else if self.pos != (outer.x, outer.y) {
            self.pos = (outer.x, outer.y);
            for (h, p) in self.strips.iter().zip(parts) {
                unsafe {
                    let _ = SetWindowPos(*h, None, outer.x + p.0 as i32, outer.y + p.1 as i32, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
        if !self.visible {
            self.restack(true);
        }
    }

    /// Keeps the strips directly below the target window in z-order.
    pub fn restack(&mut self, show: bool) {
        let after = super::window::hwnd(self.target);
        for h in &self.strips {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { SET_WINDOW_POS_FLAGS(0) };
            unsafe {
                let _ = SetWindowPos(*h, Some(after), 0, 0, 0, 0, flags);
            }
        }
        if show {
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            for h in &self.strips {
                unsafe {
                    let _ = ShowWindow(*h, SW_HIDE);
                }
            }
            self.visible = false;
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn hwnds(&self) -> [WinId; 4] {
        self.strips.map(super::window::id)
    }
}

impl Drop for Border {
    fn drop(&mut self) {
        for h in &self.strips {
            unsafe {
                let _ = DestroyWindow(*h);
            }
        }
    }
}

/// Blends two gradients (used for the border colour fade).
pub fn mix(a: &Gradient, b: &Gradient, t: f32) -> Gradient {
    if t <= 0.0 {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    let n = a.colors.len().max(b.colors.len()).max(2);
    let colors = (0..n)
        .map(|i| {
            let p = i as f32 / (n - 1) as f32;
            twm_core::color::Color::lerp(a.sample(p), b.sample(p), t)
        })
        .collect();
    Gradient { colors, angle: if t < 0.5 { a.angle } else { b.angle } }
}
