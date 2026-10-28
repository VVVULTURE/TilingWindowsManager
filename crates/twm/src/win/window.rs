//! Facts about, and operations on, top-level windows of other processes.
//!
//! Every call here is a documented Win32 API usable from an unelevated
//! process. Operations that could block on a hung application use the
//! asynchronous variants (SWP_ASYNCWINDOWPOS, ShowWindowAsync, PostMessage).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use twm_core::geom::Rect;
use twm_core::rules::WindowFacts;
use twm_core::WinId;
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Security::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn hwnd(id: WinId) -> HWND {
    HWND(id as usize as *mut c_void)
}

pub fn id(h: HWND) -> WinId {
    h.0 as usize as u64
}

pub fn exists(id: WinId) -> bool {
    unsafe { IsWindow(Some(hwnd(id))).as_bool() }
}

pub fn title(id: WinId) -> String {
    let mut buf = [0u16; 512];
    // For windows of other processes GetWindowText reads the stored caption
    // without sending WM_GETTEXT, so it cannot hang on a frozen app.
    let n = unsafe { GetWindowTextW(hwnd(id), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn class(id: WinId) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd(id), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn pid(id: WinId) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd(id), Some(&mut pid)) };
    pid
}

thread_local! {
    /// pid -> (exe name, integrity RID or None if the token is unreadable)
    static PROC_CACHE: RefCell<HashMap<u32, (String, Option<u32>)>> = RefCell::new(HashMap::new());
}

fn query_process(pid: u32) -> (String, Option<u32>) {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), None);
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let name = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            full.rsplit('\\').next().unwrap_or("").to_string()
        } else {
            String::new()
        };
        let rid = integrity_of(h);
        let _ = CloseHandle(h);
        (name, rid)
    }
}

/// Mandatory integrity level RID of a process token (e.g. 0x2000 medium,
/// 0x3000 high/elevated).
unsafe fn integrity_of(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenIntegrityLevel, Some(buf.as_mut_ptr() as *mut c_void), len, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        Some(*GetSidSubAuthority(sid, (count - 1) as u32))
    }
}

pub fn own_integrity() -> u32 {
    static ONCE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *ONCE.get_or_init(|| unsafe { integrity_of(GetCurrentProcess()).unwrap_or(0x2000) })
}

pub fn process_info(pid: u32) -> (String, Option<u32>) {
    PROC_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear(); // pids are reused; keep the cache small and fresh
        }
        c.entry(pid).or_insert_with(|| query_process(pid)).clone()
    })
}

pub fn forget_process(pid: u32) {
    PROC_CACHE.with(|c| c.borrow_mut().remove(&pid));
}

pub fn style(id: WinId) -> WINDOW_STYLE {
    WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd(id), GWL_STYLE) } as u32)
}

pub fn ex_style(id: WinId) -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd(id), GWL_EXSTYLE) } as u32)
}

pub fn is_visible(id: WinId) -> bool {
    unsafe { IsWindowVisible(hwnd(id)).as_bool() }
}

pub fn is_minimized(id: WinId) -> bool {
    unsafe { IsIconic(hwnd(id)).as_bool() }
}

pub fn is_maximized(id: WinId) -> bool {
    unsafe { IsZoomed(hwnd(id)).as_bool() }
}

pub fn is_cloaked(id: WinId) -> bool {
    let mut v = 0u32;
    let r = unsafe { DwmGetWindowAttribute(hwnd(id), DWMWA_CLOAKED, &mut v as *mut _ as *mut c_void, 4) };
    r.is_ok() && v != 0
}

pub fn window_rect(id: WinId) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd(id), &mut r) }.ok()?;
    Some(Rect::from_ltrb(r.left, r.top, r.right, r.bottom))
}

/// The visible frame (without the invisible resize borders Windows 10/11
/// adds around most windows).
pub fn frame_rect(id: WinId) -> Option<Rect> {
    let mut r = RECT::default();
    let ok = unsafe { DwmGetWindowAttribute(hwnd(id), DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut c_void, 16) };
    if ok.is_ok() && r.right > r.left {
        Some(Rect::from_ltrb(r.left, r.top, r.right, r.bottom))
    } else {
        window_rect(id)
    }
}

/// Invisible border widths (left, top, right, bottom) = window rect minus frame.
pub fn frame_offsets(id: WinId) -> (i32, i32, i32, i32) {
    match (window_rect(id), frame_rect(id)) {
        (Some(w), Some(f)) => (f.x - w.x, f.y - w.y, w.right() - f.right(), w.bottom() - f.bottom()),
        _ => (0, 0, 0, 0),
    }
}

pub fn monitor_rect_of(id: WinId) -> Option<Rect> {
    unsafe {
        let m = MonitorFromWindow(hwnd(id), MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(m, &mut mi).ok().ok()?;
        let r = mi.rcMonitor;
        Some(Rect::from_ltrb(r.left, r.top, r.right, r.bottom))
    }
}

/// For UWP apps the top-level window belongs to ApplicationFrameHost.exe;
/// the real app owns the hosted CoreWindow child. Returns its pid if found.
fn uwp_app_pid(id: WinId) -> Option<u32> {
    struct Find {
        host_pid: u32,
        found: Option<u32>,
    }
    unsafe extern "system" fn cb(child: HWND, lp: LPARAM) -> BOOL {
        let f = unsafe { &mut *(lp.0 as *mut Find) };
        let c = crate::win::window::id(child);
        let p = pid(c);
        if p != f.host_pid && class(c) == "Windows.UI.Core.CoreWindow" {
            f.found = Some(p);
            return false.into();
        }
        true.into()
    }
    let mut f = Find { host_pid: pid(id), found: None };
    unsafe {
        let _ = EnumChildWindows(Some(hwnd(id)), Some(cb), LPARAM(&mut f as *mut _ as isize));
    }
    f.found
}

/// Gathers everything the core's classifier needs.
pub fn facts(id: WinId) -> WindowFacts {
    let st = style(id);
    let ex = ex_style(id);
    let class = class(id);
    let mut pid = pid(id);
    if class == "ApplicationFrameWindow" {
        if let Some(p) = uwp_app_pid(id) {
            pid = p;
        }
    }
    let (process, rid) = process_info(pid);
    let rect = frame_rect(id).unwrap_or_default();
    let has_caption = st.contains(WS_CAPTION);
    let fullscreen = !has_caption && monitor_rect_of(id).is_some_and(|m| rect.x <= m.x && rect.y <= m.y && rect.right() >= m.right() && rect.bottom() >= m.bottom());
    let own_pid = unsafe { GetCurrentProcessId() };
    WindowFacts {
        hwnd: id,
        title: title(id),
        class,
        process,
        pid,
        visible: is_visible(id),
        cloaked: is_cloaked(id),
        minimized: is_minimized(id),
        maximized: is_maximized(id),
        owned: unsafe { GetWindow(hwnd(id), GW_OWNER) }.is_ok_and(|h| !h.is_invalid()),
        child: st.contains(WS_CHILD),
        tool_window: ex.contains(WS_EX_TOOLWINDOW),
        no_activate: ex.contains(WS_EX_NOACTIVATE),
        has_caption,
        resizable: st.contains(WS_THICKFRAME),
        popup: st.contains(WS_POPUP),
        topmost: ex.contains(WS_EX_TOPMOST),
        // Unknown integrity (token unreadable) is treated as "higher": we
        // could not manage it reliably anyway.
        elevated: match rid {
            Some(r) => r > own_integrity(),
            None => pid != 0 && pid != own_pid,
        },
        fullscreen,
        rect,
        own_process: pid == own_pid,
    }
}

/// Places a window so that its *visible frame* matches `visual`, adding the
/// window's invisible resize borders. Non-blocking.
pub fn place(id: WinId, visual: Rect, offsets: (i32, i32, i32, i32)) -> bool {
    let (l, t, r, b) = offsets;
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS;
    unsafe { SetWindowPos(hwnd(id), None, visual.x - l, visual.y - t, visual.w + l + r, visual.h + t + b, flags).is_ok() }
}

/// Moves without resizing (cheap; used by animation frames).
pub fn place_move(id: WinId, visual: Rect, offsets: (i32, i32, i32, i32)) -> bool {
    let (l, t, _, _) = offsets;
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS | SWP_NOSIZE;
    unsafe { SetWindowPos(hwnd(id), None, visual.x - l, visual.y - t, 0, 0, flags).is_ok() }
}

/// Saved normal/maximized placement of a window.
pub fn placement(id: WinId) -> Option<WINDOWPLACEMENT> {
    let mut p = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe { GetWindowPlacement(hwnd(id), &mut p) }.ok().map(|_| p)
}

pub fn set_placement(id: WinId, p: &WINDOWPLACEMENT) {
    let mut p = *p;
    p.flags |= WPF_ASYNCWINDOWPLACEMENT;
    // Never re-activate: restore minimized/normal windows without focus.
    if p.showCmd == SW_SHOWNORMAL.0 as u32 || p.showCmd == SW_RESTORE.0 as u32 {
        p.showCmd = SW_SHOWNOACTIVATE.0 as u32;
    }
    unsafe {
        let _ = SetWindowPlacement(hwnd(id), &p);
    }
}

pub fn hide(id: WinId) {
    unsafe {
        let _ = ShowWindowAsync(hwnd(id), SW_HIDE);
    }
}

pub fn show(id: WinId) {
    unsafe {
        let _ = ShowWindowAsync(hwnd(id), SW_SHOWNA);
    }
}

pub fn minimize(id: WinId) {
    unsafe {
        let _ = ShowWindowAsync(hwnd(id), SW_SHOWMINNOACTIVE);
    }
}

pub fn restore(id: WinId) {
    unsafe {
        let _ = ShowWindowAsync(hwnd(id), SW_SHOWNOACTIVATE);
    }
}

pub fn close(id: WinId) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(id)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Test hook: `TWM_NO_ACTIVATE=1` makes the manager never activate or
/// raise windows, so automated tests cannot steal keyboard focus from the
/// person using the machine.
pub fn no_activate() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("TWM_NO_ACTIVATE").is_ok_and(|v| v == "1"))
}

pub fn raise(id: WinId) {
    if no_activate() {
        return;
    }
    unsafe {
        let _ = SetWindowPos(hwnd(id), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
    }
}

pub fn foreground() -> Option<WinId> {
    let h = unsafe { GetForegroundWindow() };
    (!h.is_invalid()).then(|| id(h))
}

/// Brings a window to the foreground. Windows only lets a process do this
/// under documented conditions (see SetForegroundWindow remarks); the
/// keyboard binding that triggered us normally satisfies "received the last
/// input event". If it is refused, an unassigned virtual key is injected so
/// that this process becomes the last input source, then it is retried.
pub fn focus(id: WinId) -> bool {
    if no_activate() {
        return false;
    }
    unsafe {
        let h = hwnd(id);
        if is_minimized(id) {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h).as_bool() && GetForegroundWindow() == h {
            return true;
        }
        // Not allowed to take the foreground (see SetForegroundWindow
        // remarks): make this process the source of the last input event
        // with an unassigned key, then retry.
        crate::win::keyboard::inject_mask_key();
        let ok = SetForegroundWindow(h).as_bool() && GetForegroundWindow() == h;
        if !ok {
            log::debug!("could not activate {id:#x} (foreground is {:#x})", GetForegroundWindow().0 as usize);
        }
        ok
    }
}

pub fn set_border_color(id: WinId, colorref: u32) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd(id), DWMWA_BORDER_COLOR, &colorref as *const _ as *const c_void, 4);
    }
}

pub const DWM_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;
pub const DWM_COLOR_NONE: u32 = 0xFFFF_FFFE;

pub fn set_corner_preference(id: WinId, pref: &str) {
    let v: i32 = match pref {
        "square" => 1,
        "round" => 2,
        "small" => 3,
        _ => 0,
    };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd(id), DWMWA_WINDOW_CORNER_PREFERENCE, &v as *const _ as *const c_void, 4);
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn set_cursor_pos(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
}

/// All top-level windows in z-order (topmost first).
pub fn enumerate() -> Vec<WinId> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let v = unsafe { &mut *(lp.0 as *mut Vec<WinId>) };
        v.push(id(h));
        true.into()
    }
    let mut v: Vec<WinId> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

#[allow(dead_code)]
pub fn monitor_handle(id: WinId) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd(id), MONITOR_DEFAULTTONEAREST) }
}
