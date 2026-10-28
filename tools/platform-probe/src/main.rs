//! Platform probe: small, reversible experiments used to verify Windows
//! behaviour that the tiling manager depends on. Results are recorded in
//! docs/RESEARCH.md. Every command restores what it changes.
//!
//! Usage:
//!   platform-probe hotkeys
//!   platform-probe list
//!   platform-probe window <hwnd-hex>     (border colour, cloak, frame bounds, VDM, move)
//!   platform-probe hide <hwnd-hex> <ms>  (SW_HIDE, wait, SW_SHOWNA)

use std::ffi::c_void;
use windows::core::{BOOL, GUID};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn main() {
    unsafe { let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("hotkeys") => hotkeys(),
        Some("list") => list(),
        Some("window") => window(parse_hwnd(&args[2])),
        Some("hide") => hide(parse_hwnd(&args[2]), args[3].parse().unwrap()),
        Some("border") => border(parse_hwnd(&args[2]), u32::from_str_radix(&args[3], 16).unwrap(), args[4].parse().unwrap()),
        _ => eprintln!("see source for usage"),
    }
}

fn parse_hwnd(s: &str) -> HWND {
    HWND(usize::from_str_radix(s.trim_start_matches("0x"), 16).unwrap() as *mut c_void)
}

fn hotkeys() {
    let cases: &[(&str, HOT_KEY_MODIFIERS, u32)] = &[
        ("Win+1", MOD_WIN, '1' as u32),
        ("Win+H", MOD_WIN, 'H' as u32),
        ("Win+J", MOD_WIN, 'J' as u32),
        ("Win+Q", MOD_WIN, 'Q' as u32),
        ("Win+Return", MOD_WIN, VK_RETURN.0 as u32),
        ("Win+Left", MOD_WIN, VK_LEFT.0 as u32),
        ("Win+L", MOD_WIN, 'L' as u32),
        ("Win+Shift+1", MOD_WIN | MOD_SHIFT, '1' as u32),
        ("Win+Shift+H", MOD_WIN | MOD_SHIFT, 'H' as u32),
        ("Win+Alt+H", MOD_WIN | MOD_ALT, 'H' as u32),
        ("Win+Alt+1", MOD_WIN | MOD_ALT, '1' as u32),
        ("Win+Ctrl+1", MOD_WIN | MOD_CONTROL, '1' as u32),
        ("Alt+1", MOD_ALT, '1' as u32),
        ("Alt+H", MOD_ALT, 'H' as u32),
        ("Alt+Shift+H", MOD_ALT | MOD_SHIFT, 'H' as u32),
        ("Alt+Return", MOD_ALT, VK_RETURN.0 as u32),
        ("Alt+Tab", MOD_ALT, VK_TAB.0 as u32),
        ("Ctrl+Alt+Delete", MOD_ALT | MOD_CONTROL, VK_DELETE.0 as u32),
    ];
    for (i, (name, m, vk)) in cases.iter().enumerate() {
        let r = unsafe { RegisterHotKey(None, 100 + i as i32, *m | MOD_NOREPEAT, *vk) };
        match r {
            Ok(()) => {
                println!("{name:<16} OK");
                unsafe { UnregisterHotKey(None, 100 + i as i32).ok() };
            }
            Err(e) => println!("{name:<16} FAIL {:?} {}", e.code(), e.message()),
        }
    }
}

unsafe extern "system" fn enum_cb(hwnd: HWND, _: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(hwnd).as_bool() {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, &mut buf);
            let mut cls = [0u16; 256];
            let c = GetClassNameW(hwnd, &mut cls);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, 4);
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let st = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            println!(
                "{:#x} pid={pid} cloaked={cloaked} style={st:#010x} ex={ex:#010x} class={:?} title={:?}",
                hwnd.0 as usize,
                String::from_utf16_lossy(&cls[..c as usize]),
                String::from_utf16_lossy(&buf[..n as usize])
            );
        }
    }
    true.into()
}

fn list() {
    unsafe { EnumWindows(Some(enum_cb), LPARAM(0)).ok() };
}

fn rect_str(r: &RECT) -> String {
    format!("({},{})-({},{}) {}x{}", r.left, r.top, r.right, r.bottom, r.right - r.left, r.bottom - r.top)
}

fn window(hwnd: HWND) {
    unsafe {
        let mut wr = RECT::default();
        GetWindowRect(hwnd, &mut wr).unwrap();
        let mut fr = RECT::default();
        let hr = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut fr as *mut _ as *mut c_void, 16);
        println!("GetWindowRect      {}", rect_str(&wr));
        println!("EXTENDED_FRAME     {} ({hr:?})", rect_str(&fr));
        println!(
            "invisible border   l={} t={} r={} b={}",
            fr.left - wr.left, fr.top - wr.top, wr.right - fr.right, wr.bottom - fr.bottom
        );

        // Border colour on a window owned by another process.
        let red: u32 = 0x0000_00FF; // COLORREF 0x00BBGGRR
        let r = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &red as *const _ as *const c_void, 4);
        println!("DWMWA_BORDER_COLOR set red  -> {r:?}");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let def: u32 = 0xFFFF_FFFF;
        let r = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &def as *const _ as *const c_void, 4);
        println!("DWMWA_BORDER_COLOR reset    -> {r:?}");

        // Documented cloak attribute on a foreign window.
        let on = BOOL(1);
        let r = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &on as *const _ as *const c_void, 4);
        println!("DWMWA_CLOAK on (foreign)    -> {r:?}");
        if r.is_ok() {
            let off = BOOL(0);
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &off as *const _ as *const c_void, 4);
        }

        // Documented virtual-desktop manager.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let vdm: windows::core::Result<IVirtualDesktopManager> =
            CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL);
        match vdm {
            Ok(vdm) => {
                let cur = vdm.GetWindowDesktopId(hwnd);
                println!("VDM GetWindowDesktopId      -> {cur:?}");
                if let Ok(id) = cur {
                    println!("VDM MoveWindowToDesktop(same) -> {:?}", vdm.MoveWindowToDesktop(hwnd, &id));
                }
                let bogus = GUID::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
                println!("VDM MoveWindowToDesktop(bogus) -> {:?}", vdm.MoveWindowToDesktop(hwnd, &bogus));
            }
            Err(e) => println!("VDM create failed {e:?}"),
        }

        // Move/resize a foreign window, then restore it.
        let r = SetWindowPos(hwnd, None, wr.left + 40, wr.top + 40, wr.right - wr.left - 80, wr.bottom - wr.top - 80,
            SWP_NOZORDER | SWP_NOACTIVATE);
        let mut nr = RECT::default();
        GetWindowRect(hwnd, &mut nr).unwrap();
        println!("SetWindowPos -> {r:?}, now {}", rect_str(&nr));
        let r = SetWindowPos(hwnd, None, wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top, SWP_NOZORDER | SWP_NOACTIVATE);
        println!("SetWindowPos restore -> {r:?}");
    }
}

/// Sets DWMWA_BORDER_COLOR (COLORREF hex, 0x00BBGGRR) for `ms`, then resets it.
fn border(hwnd: HWND, color: u32, ms: u64) {
    unsafe {
        let r = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &color as *const _ as *const c_void, 4);
        println!("set {color:#010x} -> {r:?}");
        std::thread::sleep(std::time::Duration::from_millis(ms));
        let def: u32 = 0xFFFF_FFFF;
        let r = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &def as *const _ as *const c_void, 4);
        println!("reset -> {r:?}");
    }
}

fn hide(hwnd: HWND, ms: u64) {
    unsafe {
        let was = ShowWindow(hwnd, SW_HIDE);
        println!("SW_HIDE prev-visible={} now-visible={}", was.as_bool(), IsWindowVisible(hwnd).as_bool());
        std::thread::sleep(std::time::Duration::from_millis(ms));
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        println!("SW_SHOWNA now-visible={} still-a-window={}", IsWindowVisible(hwnd).as_bool(), IsWindow(Some(hwnd)).as_bool());
    }
}
