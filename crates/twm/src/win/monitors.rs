//! Display enumeration. Monitor numbering used in the configuration: the
//! primary monitor is 1, the rest follow left-to-right, then top-to-bottom.

use twm_core::geom::Rect;
use twm_core::state::Monitor;
use windows::core::BOOL;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::*;

struct Raw {
    id: String,
    rect: Rect,
    work: Rect,
    dpi: u32,
    primary: bool,
    hz: u32,
}

unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    unsafe {
        let out = &mut *(lp.0 as *mut Vec<Raw>);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
            let name_len = mi.szDevice.iter().position(|c| *c == 0).unwrap_or(mi.szDevice.len());
            let id = String::from_utf16_lossy(&mi.szDevice[..name_len]);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let r = mi.monitorInfo.rcMonitor;
            let w = mi.monitorInfo.rcWork;
            let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
            let hz = if EnumDisplaySettingsW(windows::core::PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
                dm.dmDisplayFrequency
            } else {
                60
            };
            out.push(Raw {
                id,
                rect: Rect::from_ltrb(r.left, r.top, r.right, r.bottom),
                work: Rect::from_ltrb(w.left, w.top, w.right, w.bottom),
                dpi: dx,
                primary: mi.monitorInfo.dwFlags & 1 != 0 /* MONITORINFOF_PRIMARY */,
                hz: if hz <= 1 { 60 } else { hz },
            });
        }
        true.into()
    }
}

/// Returns the monitors in configuration order and the highest refresh rate.
pub fn enumerate() -> (Vec<Monitor>, u32) {
    let mut raw: Vec<Raw> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut raw as *mut _ as isize));
    }
    raw.sort_by_key(|m| (!m.primary, m.rect.x, m.rect.y));
    let hz = raw.iter().map(|m| m.hz).max().unwrap_or(60);
    let mons = raw.into_iter().map(|m| Monitor { id: m.id, rect: m.rect, work: m.work, dpi: m.dpi, active_ws: 0 }).collect();
    (mons, hz)
}
