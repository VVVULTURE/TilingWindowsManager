//! Crash safety for hidden windows.
//!
//! Workspaces hide windows with SW_HIDE. If the manager were killed while
//! windows are hidden they would stay invisible, so every change to the
//! hidden set is persisted to `%LOCALAPPDATA%\TilingWindowsManager\
//! hidden-windows.json`. On the next start (or `twmctl restore`) every
//! listed window that still exists and still belongs to the same process is
//! shown again. A panic hook and normal shutdown also restore them.

use crate::win::window;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;
use twm_core::WinId;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    pub title: String,
}

static HIDDEN: Mutex<BTreeMap<WinId, Entry>> = Mutex::new(BTreeMap::new());

fn save(map: &BTreeMap<WinId, Entry>) {
    let path = crate::paths::recovery_file();
    if map.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(s) = serde_json::to_string_pretty(map) {
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn mark_hidden(id: WinId) {
    let mut m = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    m.insert(id, Entry { pid: window::pid(id), title: window::title(id) });
    save(&m);
}

pub fn mark_shown(id: WinId) {
    let mut m = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    if m.remove(&id).is_some() {
        save(&m);
    }
}

pub fn is_hidden(id: WinId) -> bool {
    HIDDEN.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id)
}

/// Shows every window this process hid. Safe to call from a panic hook.
pub fn restore_all() -> usize {
    let mut m = HIDDEN.lock().unwrap_or_else(|e| e.into_inner());
    let n = m.len();
    for id in m.keys() {
        window::show(*id);
    }
    m.clear();
    save(&m);
    n
}

/// Restores windows left hidden by a previous instance that did not exit
/// cleanly. Returns (restored, stale entries skipped).
pub fn restore_from_file() -> (usize, usize) {
    let path = crate::paths::recovery_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return (0, 0) };
    let map: BTreeMap<WinId, Entry> = serde_json::from_str(&text).unwrap_or_default();
    let (mut ok, mut stale) = (0, 0);
    for (id, e) in &map {
        // HWND values are recycled: only touch the window if it still
        // belongs to the same process.
        if window::exists(*id) && window::pid(*id) == e.pid {
            if !window::is_visible(*id) {
                window::show(*id);
            }
            ok += 1;
        } else {
            stale += 1;
        }
    }
    let _ = std::fs::remove_file(&path);
    (ok, stale)
}
