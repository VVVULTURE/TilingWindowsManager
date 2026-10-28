//! Test-window helper for end-to-end tests.
//!
//!   twm-testwin [--count N] [--title PREFIX] [--x X --y Y --w W --h H]
//!               [--style normal|fixed|tool|dialog|popup] [--lifetime-ms MS]
//!               [--log FILE] [--color RRGGBB]
//!
//! Windows are shown WITHOUT activation (they never steal keyboard focus)
//! and use the class `TwmTestWindow`, so a test configuration can opt in to
//! managing only these windows. With `--log`, every WM_WINDOWPOSCHANGED is
//! appended as `ms hwnd x y w h` (used to measure animation frame delivery).
//! Prints `hwnd=<hex>` per window to stdout, then runs until closed.
#![windows_subsystem = "console"]

use std::io::Write;
use std::sync::Mutex;
use std::time::Instant;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static LOG: Mutex<Option<(std::fs::File, Instant)>> = Mutex::new(None);
static COLOR: Mutex<u32> = Mutex::new(0x00303040);
static OPEN: Mutex<u32> = Mutex::new(0);

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_WINDOWPOSCHANGED => {
                if let Some((f, t0)) = LOG.lock().unwrap().as_mut() {
                    let p = &*(lp.0 as *const WINDOWPOS);
                    let _ = writeln!(f, "{:.3} {:#x} {} {} {} {}", t0.elapsed().as_secs_f64() * 1000.0, h.0 as usize, p.x, p.y, p.cx, p.cy);
                }
                DefWindowProcW(h, m, wp, lp)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if let Some((f, t0)) = LOG.lock().unwrap().as_mut() {
                    let _ = writeln!(f, "{:.3} {:#x} key {:#04x}", t0.elapsed().as_secs_f64() * 1000.0, h.0 as usize, wp.0);
                }
                DefWindowProcW(h, m, wp, lp)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                let c = *COLOR.lock().unwrap();
                let br = CreateSolidBrush(COLORREF(c));
                let mut r = RECT::default();
                let _ = GetClientRect(h, &mut r);
                FillRect(dc, &r, br);
                let _ = DeleteObject(br.into());
                let mut buf = [0u16; 128];
                let n = GetWindowTextW(h, &mut buf);
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00F0F0F0));
                let _ = DrawTextW(dc, &mut buf[..n as usize], &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                let _ = EndPaint(h, &ps);
                LRESULT(0)
            }
            WM_SIZE => {
                let _ = InvalidateRect(Some(h), None, true);
                DefWindowProcW(h, m, wp, lp)
            }
            WM_DESTROY => {
                let mut open = OPEN.lock().unwrap();
                *open -= 1;
                if *open == 0 {
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let num = |k: &str, d: i32| get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let count = num("--count", 1).max(1);
    let title = get("--title").unwrap_or_else(|| "twm-test".into());
    let style = get("--style").unwrap_or_else(|| "normal".into());
    let lifetime = get("--lifetime-ms").and_then(|v| v.parse::<u32>().ok());
    if let Some(c) = get("--color").and_then(|c| u32::from_str_radix(&c, 16).ok()) {
        *COLOR.lock().unwrap() = (c & 0xff) << 16 | (c & 0xff00) | (c >> 16) & 0xff;
    }
    if let Some(p) = get("--log") {
        let f = std::fs::OpenOptions::new().create(true).append(true).open(p).expect("log file");
        *LOG.lock().unwrap() = Some((f, Instant::now()));
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("TwmTestWindow"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let (st, ex) = match style.as_str() {
            "fixed" => (WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, WINDOW_EX_STYLE(0)),
            "tool" => (WS_OVERLAPPEDWINDOW, WS_EX_TOOLWINDOW),
            "popup" => (WS_POPUP, WINDOW_EX_STYLE(0)),
            _ => (WS_OVERLAPPEDWINDOW, WINDOW_EX_STYLE(0)),
        };
        let mut first = HWND::default();
        for i in 0..count {
            let owner = if style == "dialog" && i > 0 { Some(first) } else { None };
            let t = HSTRING::from(if count > 1 { format!("{title}-{}", i + 1) } else { title.clone() });
            let h = CreateWindowExW(
                ex,
                w!("TwmTestWindow"),
                &t,
                st,
                num("--x", 100) + i * 30,
                num("--y", 100) + i * 30,
                num("--w", 400),
                num("--h", 300),
                owner,
                None,
                Some(inst.into()),
                None,
            )
            .expect("CreateWindowEx");
            if i == 0 {
                first = h;
            }
            *OPEN.lock().unwrap() += 1;
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            println!("hwnd={:#x}", h.0 as usize);
        }
        let _ = std::io::stdout().flush();
        // Exit only on *our* timer: when a window gains keyboard focus the
        // text input framework installs its own thread timers too.
        let timer = lifetime.map(|ms| SetTimer(None, 0, ms, None)).unwrap_or(0);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() && timer != 0 && msg.wParam.0 == timer {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
