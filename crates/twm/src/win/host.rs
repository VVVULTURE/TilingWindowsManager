//! A hidden top-level window that receives system broadcasts (display and
//! work-area changes, session end, resume, Explorer restarts) and hosts the
//! notification-area (tray) icon and its menu. Also the per-user
//! "start with Windows" toggle (HKCU Run key — no elevation needed).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 3;
const TRAY_ID: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEvent {
    DisplayChanged,
    WorkAreaChanged,
    Resumed,
    SessionEnding,
    TaskbarCreated,
    Menu(MenuItem),
    TrayActivated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MenuItem {
    Pause = 100,
    Reload,
    EditConfig,
    OpenConfigFolder,
    Settings,
    ShowLog,
    Autostart,
    Quit,
}

impl MenuItem {
    fn from(id: u32) -> Option<MenuItem> {
        use MenuItem::*;
        [Pause, Reload, EditConfig, OpenConfigFolder, Settings, ShowLog, Autostart, Quit].into_iter().find(|m| *m as u32 == id)
    }
}

thread_local! {
    static EVENTS: RefCell<VecDeque<HostEvent>> = const { RefCell::new(VecDeque::new()) };
    static TASKBAR_CREATED: RefCell<u32> = const { RefCell::new(0) };
}

fn push(e: HostEvent) {
    EVENTS.with(|q| q.borrow_mut().push_back(e));
}

pub fn drain() -> Vec<HostEvent> {
    EVENTS.with(|q| q.borrow_mut().drain(..).collect())
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let tc = TASKBAR_CREATED.with(|t| *t.borrow());
        match m {
            WM_DISPLAYCHANGE | WM_DPICHANGED => push(HostEvent::DisplayChanged),
            WM_SETTINGCHANGE if wp.0 as u32 == SPI_SETWORKAREA.0 => push(HostEvent::WorkAreaChanged),
            WM_POWERBROADCAST if wp.0 as u32 == PBT_APMRESUMEAUTOMATIC => push(HostEvent::Resumed),
            WM_QUERYENDSESSION => return LRESULT(1),
            WM_ENDSESSION if wp.0 != 0 => push(HostEvent::SessionEnding),
            WM_TRAY => match (lp.0 as u32) & 0xFFFF {
                WM_RBUTTONUP | WM_CONTEXTMENU => push(HostEvent::TrayActivated),
                WM_LBUTTONDBLCLK => push(HostEvent::Menu(MenuItem::Settings)),
                _ => {}
            },
            WM_COMMAND => {
                if let Some(item) = MenuItem::from((wp.0 & 0xFFFF) as u32) {
                    push(HostEvent::Menu(item));
                }
            }
            _ if m == tc && tc != 0 => push(HostEvent::TaskbarCreated),
            _ => return DefWindowProcW(h, m, wp, lp),
        }
        LRESULT(0)
    }
}

pub fn create() -> HWND {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: w!("TwmHost"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        TASKBAR_CREATED.with(|t| *t.borrow_mut() = RegisterWindowMessageW(w!("TaskbarCreated")));
        // A real (never shown) top-level window: message-only windows do not
        // receive broadcasts such as WM_DISPLAYCHANGE.
        CreateWindowExW(WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0), w!("TwmHost"), w!("TilingWindowsManager"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None)
            .unwrap_or_default()
    }
}

/// Draws the tray icon: a 2x2 tile grid in the accent colours.
fn make_icon(size: i32, a: (u8, u8, u8), b: (u8, u8, u8), paused: bool) -> HICON {
    let n = size as usize;
    let mut px = vec![0u32; n * n];
    let gap = (size / 8).max(1);
    let half = (size - gap) / 2;
    let r = (size as f32 / 10.0).max(1.0);
    for y in 0..n {
        for x in 0..n {
            let (xi, yi) = (x as i32, y as i32);
            let cx = if xi < half { 0 } else if xi >= half + gap { 1 } else { continue };
            let cy = if yi < half { 0 } else if yi >= half + gap { 1 } else { continue };
            // rounded tile corners
            let lx = (if cx == 0 { xi } else { xi - half - gap }) as f32 + 0.5;
            let ly = (if cy == 0 { yi } else { yi - half - gap }) as f32 + 0.5;
            let hs = half as f32;
            let dx = (r - lx).max(lx - (hs - r)).max(0.0);
            let dy = (r - ly).max(ly - (hs - r)).max(0.0);
            let cov = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let t = (x + y) as f32 / (2 * n) as f32;
            let mut c = [(a.0 as f32 + (b.0 as f32 - a.0 as f32) * t), (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t), (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t)];
            if paused {
                let g = (c[0] + c[1] + c[2]) / 3.0 * 0.7;
                c = [g, g, g];
            }
            let al = if cx == 1 && cy == 0 && !paused { 0.55 } else { 1.0 } * cov;
            let a8 = (al * 255.0) as u32;
            px[y * n + x] = a8 << 24 | ((c[0] * al) as u32) << 16 | ((c[1] * al) as u32) << 8 | (c[2] * al) as u32;
        }
    }
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dc = GetDC(None);
        let color = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        ReleaseDC(None, dc);
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

pub struct Tray {
    host: HWND,
    icon: HICON,
    tip: String,
    added: bool,
}

impl Tray {
    pub fn new(host: HWND) -> Tray {
        Tray { host, icon: HICON::default(), tip: String::new(), added: false }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.host, uID: TRAY_ID, ..Default::default() };
        d.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = self.icon;
        copy_wide(&mut d.szTip, &self.tip);
        d
    }

    /// Adds or updates the icon. `accent` colours come from the theme.
    pub fn set(&mut self, tip: &str, accent: ((u8, u8, u8), (u8, u8, u8)), paused: bool) {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let old = self.icon;
        self.icon = make_icon(size, accent.0, accent.1, paused);
        self.tip = tip.to_string();
        let mut d = self.data();
        unsafe {
            if self.added {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
            } else {
                self.added = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
                d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
            }
            if !old.is_invalid() {
                let _ = DestroyIcon(old);
            }
        }
    }

    /// Explorer restarted: the icon must be added again.
    pub fn readd(&mut self) {
        self.added = false;
        let d = self.data();
        self.added = unsafe { Shell_NotifyIconW(NIM_ADD, &d) }.as_bool();
    }

    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        let mut d = self.data();
        d.uFlags = NIF_INFO;
        copy_wide(&mut d.szInfoTitle, title);
        copy_wide(&mut d.szInfo, text);
        d.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn show_menu(&self, status: &str, paused: bool, autostart: bool) {
        unsafe {
            let Ok(m) = CreatePopupMenu() else { return };
            let add = |id: u32, text: &str, flags: MENU_ITEM_FLAGS| {
                let t = HSTRING::from(text);
                let _ = AppendMenuW(m, flags, id as usize, &t);
            };
            add(0, status, MF_STRING | MF_GRAYED);
            add(0, "", MF_SEPARATOR);
            add(MenuItem::Pause as u32, "Pause tiling", MF_STRING | if paused { MF_CHECKED } else { MF_UNCHECKED });
            add(MenuItem::Reload as u32, "Reload configuration", MF_STRING);
            add(MenuItem::Settings as u32, "Settings && key bindings…", MF_STRING);
            add(MenuItem::EditConfig as u32, "Edit config file", MF_STRING);
            add(MenuItem::OpenConfigFolder as u32, "Open config folder", MF_STRING);
            add(MenuItem::ShowLog as u32, "Show log", MF_STRING);
            add(0, "", MF_SEPARATOR);
            add(MenuItem::Autostart as u32, "Start with Windows", MF_STRING | if autostart { MF_CHECKED } else { MF_UNCHECKED });
            add(0, "", MF_SEPARATOR);
            add(MenuItem::Quit as u32, "Quit (restore all windows)", MF_STRING);
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            // Required so the menu closes when the user clicks elsewhere.
            let _ = SetForegroundWindow(self.host);
            let _ = TrackPopupMenu(m, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, p.x, p.y, Some(0), self.host, None);
            let _ = PostMessageW(Some(self.host), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(m);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("TilingWindowsManager");

pub fn autostart_enabled() -> bool {
    unsafe {
        let mut size = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)).is_ok()
    }
}

/// Adds/removes the per-user Run entry pointing at this executable.
pub fn set_autostart(on: bool) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key).ok().map_err(|e| e.message().to_string())?;
        let r = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = format!("\"{}\"", exe.display());
            let w: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
            RegSetValueExW(key, RUN_VALUE, None, REG_SZ, Some(bytes)).ok()
        } else {
            let r = RegDeleteValueW(key, RUN_VALUE);
            if r == ERROR_FILE_NOT_FOUND { Ok(()) } else { r.ok() }
        };
        let _ = RegCloseKey(key);
        r.map_err(|e| e.message().to_string())
    }
}

/// Opens a file, folder or URL with its default handler.
pub fn shell_open(target: &str) {
    let t = HSTRING::from(target);
    unsafe {
        ShellExecuteW(None, w!("open"), &t, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Runs a command line (first token = program, resolved via App Paths/PATH).
pub fn exec(cmdline: &str) -> Result<(), String> {
    let cmdline = cmdline.trim();
    let (prog, args) = if let Some(rest) = cmdline.strip_prefix('"') {
        match rest.find('"') {
            Some(i) => (&rest[..i], rest[i + 1..].trim()),
            None => return Err("unterminated quote".into()),
        }
    } else {
        match cmdline.find(char::is_whitespace) {
            Some(i) => (&cmdline[..i], cmdline[i..].trim()),
            None => (cmdline, ""),
        }
    };
    let p = HSTRING::from(prog);
    let a = HSTRING::from(args);
    let r = unsafe { ShellExecuteW(None, w!("open"), &p, if args.is_empty() { PCWSTR::null() } else { PCWSTR(a.as_ptr()) }, PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("could not start `{prog}` (ShellExecute error {})", r.0 as isize))
    }
}
