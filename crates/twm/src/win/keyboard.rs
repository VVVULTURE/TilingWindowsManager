//! Keyboard bindings.
//!
//! Two backends (config `input.keyboard_backend`):
//!
//! * `hook` — a WH_KEYBOARD_LL hook on a dedicated thread. It can claim
//!   Win-key chords that the shell has already registered (Win+1, Win+Left;
//!   RegisterHotKey fails for those with ERROR_HOTKEY_ALREADY_REGISTERED,
//!   verified in docs/RESEARCH.md). Only chords that are bound are
//!   swallowed; everything else passes through untouched. The hook procedure
//!   does a table lookup and posts a message, so it stays far below the
//!   system's LowLevelHooksTimeout even if the main thread is busy.
//! * `hotkey` — RegisterHotKey on the main thread. No hook, but chords
//!   owned by Windows or other apps cannot be registered (reported).
//!
//! When a Win or Alt chord is consumed, releasing the modifier would
//! normally open the Start menu / activate the app's menu bar, because the
//! shell saw "modifier pressed and released alone". An unassigned virtual
//! key (0xE8, the same "menu mask" technique AutoHotkey uses) is injected
//! before the release to prevent that.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{OnceLock, RwLock};
use std::thread::JoinHandle;
use twm_core::keys::{Chord, Mods};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_APP_BIND: u32 = WM_APP + 1;
const MASK_VK: u16 = 0xE8;
/// dwExtraInfo tag on input we inject ourselves ("TWM!").
const INJECT_TAG: usize = 0x5457_4D21;

static BINDS: OnceLock<RwLock<HashMap<Chord, usize>>> = OnceLock::new();
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static WIN_USED: AtomicBool = AtomicBool::new(false);
static ALT_USED: AtomicBool = AtomicBool::new(false);

fn binds() -> &'static RwLock<HashMap<Chord, usize>> {
    BINDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn inject_mask_key() {
    let mk = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(MASK_VK), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: INJECT_TAG } },
    };
    let inputs = [mk(KEYBD_EVENT_FLAGS(0)), mk(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

fn current_mods() -> Mods {
    Mods { win: down(VK_LWIN) || down(VK_RWIN), ctrl: down(VK_CONTROL), alt: down(VK_MENU), shift: down(VK_SHIFT) }
}

thread_local! {
    static SWALLOWED: std::cell::RefCell<HashSet<u16>> = std::cell::RefCell::new(HashSet::new());
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let k = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if k.dwExtraInfo == INJECT_TAG {
            return CallNextHookEx(None, code, wparam, lparam);
        }
        let vk = k.vkCode as u16;
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        let v = VIRTUAL_KEY(vk);
        let is_win = v == VK_LWIN || v == VK_RWIN;
        let is_alt = v == VK_MENU || v == VK_LMENU || v == VK_RMENU;
        let is_mod = is_win || is_alt || [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_SHIFT, VK_LSHIFT, VK_RSHIFT].contains(&v);
        if is_mod {
            if is_up && ((is_win && WIN_USED.swap(false, Ordering::Relaxed)) || (is_alt && ALT_USED.swap(false, Ordering::Relaxed))) {
                inject_mask_key();
            }
            return CallNextHookEx(None, code, wparam, lparam);
        }
        if is_down {
            let chord = Chord { mods: current_mods(), vk };
            let hit = binds().read().ok().and_then(|b| b.get(&chord).copied());
            if let Some(idx) = hit {
                let first = SWALLOWED.with(|s| s.borrow_mut().insert(vk));
                if first {
                    let _ = PostThreadMessageW(MAIN_THREAD.load(Ordering::Relaxed), WM_APP_BIND, WPARAM(idx), LPARAM(0));
                }
                if chord.mods.win {
                    WIN_USED.store(true, Ordering::Relaxed);
                }
                if chord.mods.alt {
                    ALT_USED.store(true, Ordering::Relaxed);
                }
                return LRESULT(1); // swallow (auto-repeat is swallowed without re-firing)
            }
        } else if is_up && SWALLOWED.with(|s| s.borrow_mut().remove(&vk)) {
            return LRESULT(1);
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

pub enum Backend {
    Hook { thread: Option<JoinHandle<()>>, thread_id: u32 },
    Hotkey { registered: Vec<i32> },
    None,
}

/// Result of (re)binding: chords that could not be registered.
pub struct BindReport {
    pub failed: Vec<(Chord, String)>,
}

impl Backend {
    /// Starts the requested backend. Must be called on the main thread.
    pub fn start(kind: &str, chords: &[Chord]) -> (Backend, BindReport) {
        MAIN_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
        let mut b = if kind == "hotkey" {
            Backend::Hotkey { registered: Vec::new() }
        } else {
            match start_hook_thread() {
                Ok((t, id)) => Backend::Hook { thread: Some(t), thread_id: id },
                Err(e) => {
                    log::error!("keyboard hook unavailable ({e}); falling back to RegisterHotKey");
                    Backend::Hotkey { registered: Vec::new() }
                }
            }
        };
        let r = b.rebind(chords);
        (b, r)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Backend::Hook { .. } => "hook",
            Backend::Hotkey { .. } => "hotkey",
            Backend::None => "none",
        }
    }

    /// Replaces the active bindings; index i fires WM_APP_BIND with wparam i.
    pub fn rebind(&mut self, chords: &[Chord]) -> BindReport {
        let mut failed = Vec::new();
        match self {
            Backend::Hook { .. } => {
                let map: HashMap<Chord, usize> = chords.iter().enumerate().map(|(i, c)| (*c, i)).collect();
                *binds().write().unwrap() = map;
            }
            Backend::Hotkey { registered } => {
                for id in registered.drain(..) {
                    unsafe {
                        let _ = UnregisterHotKey(None, id);
                    }
                }
                for (i, c) in chords.iter().enumerate() {
                    let mut m = MOD_NOREPEAT;
                    if c.mods.win {
                        m |= MOD_WIN;
                    }
                    if c.mods.ctrl {
                        m |= MOD_CONTROL;
                    }
                    if c.mods.alt {
                        m |= MOD_ALT;
                    }
                    if c.mods.shift {
                        m |= MOD_SHIFT;
                    }
                    match unsafe { RegisterHotKey(None, i as i32, m, c.vk as u32) } {
                        Ok(()) => registered.push(i as i32),
                        Err(e) => failed.push((*c, e.message().to_string())),
                    }
                }
            }
            Backend::None => {}
        }
        BindReport { failed }
    }

    pub fn stop(&mut self) {
        match std::mem::replace(self, Backend::None) {
            Backend::Hook { thread, thread_id } => {
                binds().write().unwrap().clear();
                unsafe {
                    let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
                if let Some(t) = thread {
                    let _ = t.join();
                }
            }
            Backend::Hotkey { registered } => {
                for id in registered {
                    unsafe {
                        let _ = UnregisterHotKey(None, id);
                    }
                }
            }
            Backend::None => {}
        }
    }
}

fn start_hook_thread() -> Result<(JoinHandle<()>, u32), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<u32, String>>();
    let t = std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || unsafe {
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
                Ok(h) => h,
                Err(e) => {
                    let _ = tx.send(Err(e.message().to_string()));
                    return;
                }
            };
            // Make sure the thread has a message queue before reporting ready.
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = tx.send(Ok(GetCurrentThreadId()));
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        })
        .map_err(|e| e.to_string())?;
    match rx.recv() {
        Ok(Ok(id)) => Ok((t, id)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("hook thread exited".into()),
    }
}
