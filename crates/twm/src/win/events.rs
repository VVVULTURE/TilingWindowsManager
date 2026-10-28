//! Out-of-context WinEvent hooks (no DLL injection, no elevation). The
//! callbacks run on the installing thread inside its message loop; they
//! only queue events, which the daemon drains after each message so that
//! window operations never run re-entrantly inside a hook callback.

use std::cell::RefCell;
use std::collections::VecDeque;
use twm_core::WinId;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Shown(WinId),
    Hidden(WinId),
    Destroyed(WinId),
    Foreground(WinId),
    MoveSizeStart(WinId),
    MoveSizeEnd(WinId),
    MinimizeStart(WinId),
    MinimizeEnd(WinId),
    Location(WinId),
    TitleChanged(WinId),
    Cloaked(WinId),
    Uncloaked(WinId),
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
    static HOOKS: RefCell<Vec<HWINEVENTHOOK>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn callback(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, id_child: i32, _: u32, _: u32) {
    // Only whole-window events (not carets, cursors, scrollbars...).
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    let w = super::window::id(hwnd);
    let ev = match event {
        EVENT_OBJECT_SHOW => Event::Shown(w),
        EVENT_OBJECT_HIDE => Event::Hidden(w),
        EVENT_OBJECT_DESTROY => Event::Destroyed(w),
        EVENT_SYSTEM_FOREGROUND => Event::Foreground(w),
        EVENT_SYSTEM_MOVESIZESTART => Event::MoveSizeStart(w),
        EVENT_SYSTEM_MOVESIZEEND => Event::MoveSizeEnd(w),
        EVENT_SYSTEM_MINIMIZESTART => Event::MinimizeStart(w),
        EVENT_SYSTEM_MINIMIZEEND => Event::MinimizeEnd(w),
        EVENT_OBJECT_LOCATIONCHANGE => Event::Location(w),
        EVENT_OBJECT_NAMECHANGE => Event::TitleChanged(w),
        EVENT_OBJECT_CLOAKED => Event::Cloaked(w),
        EVENT_OBJECT_UNCLOAKED => Event::Uncloaked(w),
        _ => return,
    };
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        // Coalesce bursts of location changes for the same window.
        if matches!(ev, Event::Location(_)) && q.back() == Some(&ev) {
            return;
        }
        q.push_back(ev);
    });
}

/// Installs the hooks on the calling thread (which must pump messages).
pub fn install() -> Result<(), String> {
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        (EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
        (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE),
        (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
    ];
    for (lo, hi) in ranges {
        let h = unsafe { SetWinEventHook(lo, hi, None, Some(callback), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) };
        if h.is_invalid() {
            return Err(format!("SetWinEventHook({lo:#x}..{hi:#x}) failed"));
        }
        HOOKS.with(|v| v.borrow_mut().push(h));
    }
    Ok(())
}

pub fn uninstall() {
    HOOKS.with(|v| {
        for h in v.borrow_mut().drain(..) {
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
    });
}

pub fn drain() -> Vec<Event> {
    QUEUE.with(|q| q.borrow_mut().drain(..).collect())
}

pub fn pending() -> bool {
    QUEUE.with(|q| !q.borrow().is_empty())
}
