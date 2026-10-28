//! The daemon: one thread owns all state and runs a Win32 message loop.
//!
//! Inputs (all event-driven, no polling while idle):
//! * WinEvent hooks (window shown/hidden/destroyed/moved/focused...)
//! * keyboard binding messages (hook thread or WM_HOTKEY)
//! * IPC requests (pipe thread -> posted message)
//! * host-window broadcasts (display/work-area change, resume, tray menu)
//! * a directory change notification for live config reload
//! * a high-resolution waitable timer that ticks only while animating
//!
//! After each batch of inputs the core `Wm` produces a `Desired` state and
//! `apply` reconciles real windows with it.

use crate::win::host::{self, HostEvent, MenuItem, Tray};
use crate::win::keyboard::{self, Backend};
use crate::win::overlay::{self, Border};
use crate::win::{events, events::Event, monitors, window};
use crate::{ipc, paths, recovery};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use twm_core::actions::Action;
use twm_core::anim::{Bezier, Tween};
use twm_core::config::{self, Config, Diagnostic, LoadReport, Severity};
use twm_core::geom::Rect;
use twm_core::ipc::{Request, Response};
use twm_core::rules::{classify, Decision};
use twm_core::state::{Effect, Wm};
use twm_core::WinId;
use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq)]
enum Anim {
    None,
    Windows,
}

struct FadeState {
    start: Instant,
    to_active: bool,
}

#[derive(Default, serde::Serialize)]
struct Stats {
    events: u64,
    applies: u64,
    placements: u64,
    anim_frames: u64,
    last_apply_us: u64,
    max_apply_us: u64,
    enforcements: u64,
}

struct Enforce {
    count: u32,
    since: Instant,
    gave_up: bool,
}

pub struct Daemon {
    wm: Wm,
    config_path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    kb: Backend,
    bind_failures: Vec<String>,
    tray: Tray,
    /// Last visual rect we asked each window to take.
    applied: HashMap<WinId, Rect>,
    offsets: HashMap<WinId, (i32, i32, i32, i32)>,
    /// Windows hidden by us for inactive workspaces.
    hidden: HashSet<WinId>,
    borders: HashMap<WinId, Border>,
    fades: HashMap<WinId, FadeState>,
    tweens: HashMap<WinId, Tween>,
    /// Interactive move/size in progress: (window, frame at start).
    moving: Option<(WinId, Rect)>,
    enforce: HashMap<WinId, Enforce>,
    /// Windows we recoloured/re-cornered through DWM (reset on exit).
    dwm_touched: HashSet<WinId>,
    foreground: Option<WinId>,
    zoomed: HashSet<WinId>,
    /// Placement of windows that existed before we started (restored on quit).
    original: HashMap<WinId, WINDOWPLACEMENT>,
    start: Instant,
    frame_ms: f64,
    timer: HANDLE,
    timer_armed: bool,
    watch: HANDLE,
    cfg_stamp: Option<(SystemTime, u64)>,
    reload_due: Option<Instant>,
    quit: bool,
    stats: Stats,
}

fn theme_lookup(name: &str) -> Option<String> {
    std::fs::read_to_string(paths::themes_dir().join(format!("{name}.toml"))).ok()
}

pub fn load_config(path: &PathBuf, fallback: Option<&Config>) -> LoadReport {
    match std::fs::read_to_string(path) {
        Ok(text) => config::load_str(&text, &path.display().to_string(), fallback, &theme_lookup),
        Err(e) => {
            let mut r = config::load_str(config::DEFAULT_CONFIG, "<built-in default>", None, &|_| None);
            if path.exists() {
                r.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    file: path.display().to_string(),
                    line: None,
                    setting: "<file>".into(),
                    message: format!("cannot read file: {e}; using built-in defaults"),
                });
                r.used_fallback = true;
            }
            r
        }
    }
}

fn file_stamp(p: &PathBuf) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn accent(c: &Config) -> ((u8, u8, u8), (u8, u8, u8)) {
    let g = &c.active_border;
    let a = g.colors[0];
    let b = *g.colors.last().unwrap();
    ((a.r, a.g, a.b), (b.r, b.g, b.b))
}

impl Daemon {
    pub fn new(config_path: PathBuf) -> Result<Daemon, String> {
        let (restored, _) = recovery::restore_from_file();
        if restored > 0 {
            log::warn!("restored {restored} window(s) hidden by a previous instance that did not exit cleanly");
        }
        if !config_path.exists() {
            if let Some(d) = config_path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            match std::fs::write(&config_path, config::DEFAULT_CONFIG) {
                Ok(()) => log::info!("created default configuration at {}", config_path.display()),
                Err(e) => log::warn!("could not create {}: {e}", config_path.display()),
            }
        }
        let report = load_config(&config_path, None);
        for d in &report.diagnostics {
            log::warn!("{d}");
        }
        let (mons, hz) = monitors::enumerate();
        if mons.is_empty() {
            return Err("no monitors found".into());
        }
        let wm = Wm::new(report.config.clone(), mons);
        overlay::register_class();
        let host = host::create();
        let tray = Tray::new(host);
        let timer = unsafe { CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }
            .or_else(|_| unsafe { CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0) })
            .map_err(|e| format!("CreateWaitableTimer: {e}"))?;
        let dir = config_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(paths::config_dir);
        let watch = unsafe { FindFirstChangeNotificationW(&HSTRING::from(dir.as_os_str()), false, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE) }
            .unwrap_or(INVALID_HANDLE_VALUE);
        let mut d = Daemon {
            wm,
            cfg_stamp: file_stamp(&config_path),
            config_path,
            diagnostics: report.diagnostics,
            kb: Backend::None,
            bind_failures: Vec::new(),
            tray,
            applied: HashMap::new(),
            offsets: HashMap::new(),
            hidden: HashSet::new(),
            borders: HashMap::new(),
            fades: HashMap::new(),
            tweens: HashMap::new(),
            moving: None,
            enforce: HashMap::new(),
            dwm_touched: HashSet::new(),
            foreground: window::foreground(),
            zoomed: HashSet::new(),
            original: HashMap::new(),
            start: Instant::now(),
            frame_ms: 1000.0 / hz.clamp(30, 360) as f64,
            timer,
            timer_armed: false,
            watch,
            reload_due: None,
            quit: false,
            stats: Stats::default(),
        };
        d.start_keyboard();
        d.adopt_existing();
        events::install()?;
        ipc::start(unsafe { GetCurrentThreadId() });
        d.update_tray();
        d.apply(Anim::None);
        d.report_diagnostics(true);
        log::info!(
            "started: {} monitor(s), {} managed window(s), keyboard backend {}, {:.0} Hz animation clock",
            d.wm.monitors.len(),
            d.wm.windows.len(),
            d.kb.kind(),
            1000.0 / d.frame_ms
        );
        Ok(d)
    }

    fn start_keyboard(&mut self) {
        let chords: Vec<_> = self.wm.config().binds.iter().map(|b| b.chord).collect();
        self.kb.stop();
        let (kb, report) = Backend::start(&self.wm.config().input.keyboard_backend, &chords);
        self.kb = kb;
        self.bind_failures = report.failed.iter().map(|(c, e)| format!("{c}: {e}")).collect();
        for f in &self.bind_failures {
            log::warn!("binding not registered: {f}");
        }
    }

    fn report_diagnostics(&self, startup: bool) {
        let errors: Vec<&Diagnostic> = self.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect();
        if !errors.is_empty() {
            let first = &errors[0];
            let loc = first.line.map(|l| format!(" (line {l})")).unwrap_or_default();
            let more = if errors.len() > 1 { format!(" — and {} more; run `twmctl validate`", errors.len() - 1) } else { String::new() };
            self.tray.notify("Configuration problem", &format!("{}{loc}: {}{more}", first.setting, first.message), true);
        } else if !self.bind_failures.is_empty() {
            self.tray.notify("Some key bindings are unavailable", &format!("{} (in use by Windows or another app)", self.bind_failures.join(", ")), true);
        } else if !startup {
            self.tray.notify("Configuration reloaded", "No problems found.", false);
        }
    }

    fn update_tray(&mut self) {
        let ws = self.wm.active_ws();
        let layout = self.wm.workspaces.get(&ws).map(|w| w.layout.kind().name()).unwrap_or("-");
        let tip = if self.wm.paused { "TilingWindowsManager — paused".to_string() } else { format!("TilingWindowsManager — workspace {ws} · {layout}") };
        let acc = accent(self.wm.config());
        self.tray.set(&tip, acc, self.wm.paused);
    }

    fn classify_window(&self, id: WinId) -> (twm_core::rules::WindowFacts, twm_core::rules::Classification) {
        let f = window::facts(id);
        let c = classify(&f, &self.wm.config().rules, &self.wm.config().classify_options());
        (f, c)
    }

    fn adopt_existing(&mut self) {
        // Bottom of the z-order first so the most recently used windows are
        // inserted last (they end up as the "focused" split targets).
        let mut ids = window::enumerate();
        ids.reverse();
        for id in ids {
            if !window::is_visible(id) {
                continue;
            }
            let (f, c) = self.classify_window(id);
            if c.decision != Decision::Ignore {
                if let Some(p) = window::placement(id) {
                    self.original.insert(id, p);
                }
                self.prepare_window(id, f.maximized);
                self.wm.add_window(f, &c, true);
            }
        }
        if let Some(fg) = self.foreground {
            self.wm.on_focus(Some(fg), false);
        }
    }

    /// One-time preparation of a newly managed window.
    fn prepare_window(&mut self, id: WinId, maximized: bool) {
        if maximized {
            // A maximized window cannot be tiled; restore it without activating.
            window::restore(id);
        }
        self.offsets.insert(id, window::frame_offsets(id));
        let deco = &self.wm.config().decoration;
        if deco.corner_preference != "default" {
            window::set_corner_preference(id, &deco.corner_preference);
            self.dwm_touched.insert(id);
        }
    }

    fn forget(&mut self, id: WinId) {
        self.applied.remove(&id);
        self.offsets.remove(&id);
        self.borders.remove(&id);
        self.fades.remove(&id);
        self.tweens.remove(&id);
        self.enforce.remove(&id);
        self.zoomed.remove(&id);
        self.original.remove(&id);
        if self.hidden.remove(&id) {
            recovery::mark_shown(id);
        }
        if self.dwm_touched.remove(&id) && window::exists(id) {
            window::set_border_color(id, window::DWM_COLOR_DEFAULT);
            window::set_corner_preference(id, "default");
        }
    }

    fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    // ------------------------------------------------------------ reconcile

    fn apply(&mut self, anim: Anim) {
        self.apply_with(anim, None)
    }

    fn apply_with(&mut self, anim: Anim, ws_switch: Option<(usize, u32, u32)>) {
        let t0 = Instant::now();
        self.stats.applies += 1;
        let d = self.wm.desired();
        let cfg = self.wm.config().clone();
        let hide_min = cfg.general.hide_method == "minimize";

        // 1. Hide windows of inactive workspaces.
        for id in &d.hidden {
            if self.hidden.insert(*id) {
                self.tweens.remove(id);
                if let Some(b) = self.borders.get_mut(id) {
                    b.hide();
                }
                recovery::mark_hidden(*id);
                if hide_min { window::minimize(*id) } else { window::hide(*id) }
            }
        }
        // 2. Placements (and showing windows of newly visible workspaces).
        let now = self.now_ms();
        let wa = &cfg.animations;
        let ws_anim = wa.enabled && wa.workspaces.enabled && wa.workspaces.duration > 0 && wa.workspaces.style.as_deref() != Some("none");
        let win_anim = anim == Anim::Windows && wa.enabled && wa.windows.enabled && wa.windows.duration > 0;
        let mut placed: HashSet<WinId> = HashSet::new();
        for p in &d.placements {
            placed.insert(p.id);
            let id = p.id;
            let offs = *self.offsets.entry(id).or_insert_with(|| window::frame_offsets(id));
            let was_hidden = self.hidden.remove(&id);
            if was_hidden {
                recovery::mark_shown(id);
                // Slide in from inside its own monitor (never across monitors).
                let start = match ws_switch {
                    Some((mon, from, to)) if ws_anim => {
                        let mr = self.wm.monitors.get(mon).map(|m| m.rect).unwrap_or(p.rect);
                        let vert = wa.workspaces.style.as_deref() == Some("slidevert");
                        let sign = if to > from { 1 } else { -1 };
                        let dist = ((if vert { mr.h } else { mr.w }) as f32 * 0.12) as i32;
                        let mut s = p.rect;
                        if vert {
                            s.y = (s.y + sign * dist).clamp(mr.y, (mr.bottom() - s.h).max(mr.y));
                        } else {
                            s.x = (s.x + sign * dist).clamp(mr.x, (mr.right() - s.w).max(mr.x));
                        }
                        Some(s)
                    }
                    _ => None,
                };
                window::place(id, start.unwrap_or(p.rect), offs);
                if hide_min { window::restore(id) } else { window::show(id) }
                if let Some(s) = start.filter(|s| *s != p.rect) {
                    self.tweens.insert(id, Tween { from: s, to: p.rect, start_ms: now, duration_ms: wa.workspaces.duration as f64, curve: Bezier::new(cfg.bezier(&wa.workspaces.curve)) });
                } else {
                    self.tweens.remove(&id);
                }
                self.applied.insert(id, p.rect);
                self.stats.placements += 1;
                continue;
            }
            if window::is_minimized(id) || window::is_maximized(id) {
                continue;
            }
            let target_changed = self.applied.get(&id) != Some(&p.rect);
            if !target_changed {
                continue;
            }
            self.enforce.remove(&id);
            let current = self.tweens.get(&id).map(|t| t.at(now).0).or_else(|| window::frame_rect(id));
            match current {
                Some(cur) if win_anim && cur != p.rect && self.applied.contains_key(&id) => {
                    self.tweens.insert(id, Tween { from: cur, to: p.rect, start_ms: now, duration_ms: wa.windows.duration as f64, curve: Bezier::new(cfg.bezier(&wa.windows.curve)) });
                }
                _ => {
                    self.tweens.remove(&id);
                    window::place(id, p.rect, offs);
                    self.stats.placements += 1;
                }
            }
            self.applied.insert(id, p.rect);
        }
        // Monocle/fullscreen stacking: raise the focused window.
        if let Some(f) = self.wm.focused {
            if d.placements.iter().filter(|p| !p.floating).any(|p| p.focused && d.placements.iter().any(|q| q.id != p.id && q.rect == p.rect)) {
                window::raise(f);
            }
        }
        // 3. Focus requested by an action.
        if let Some(f) = self.wm.take_focus_request() {
            log::trace!("focus request {f:#x}");
            if window::foreground() != Some(f) {
                window::focus(f);
            }
            if cfg.general.mouse_follows_focus {
                if let Some(p) = d.placements.iter().find(|p| p.id == f) {
                    let (x, y) = p.rect.center();
                    window::set_cursor_pos(x, y);
                }
            }
        }
        // 4. Borders.
        self.sync_borders(&d, &placed);
        self.arm_timer();
        let us = t0.elapsed().as_micros() as u64;
        self.stats.last_apply_us = us;
        self.stats.max_apply_us = self.stats.max_apply_us.max(us);
    }

    fn sync_borders(&mut self, d: &twm_core::state::Desired, placed: &HashSet<WinId>) {
        let cfg = self.wm.config().clone();
        let style = cfg.decoration.border_style.as_str();
        // Remove borders that are no longer wanted.
        let wanted: HashSet<WinId> = if style == "overlay" { d.placements.iter().filter(|p| p.border.is_some()).map(|p| p.id).collect() } else { HashSet::new() };
        self.borders.retain(|id, _| wanted.contains(id));
        if style == "dwm" {
            for p in &d.placements {
                let w = &self.wm.windows[&p.id];
                let c = if !w.border {
                    window::DWM_COLOR_DEFAULT
                } else if Some(p.id) == self.foreground {
                    cfg.active_border.colors[0].colorref()
                } else if cfg.decoration.inactive_borders {
                    cfg.inactive_border.colors[0].colorref()
                } else {
                    window::DWM_COLOR_DEFAULT
                };
                window::set_border_color(p.id, c);
                self.dwm_touched.insert(p.id);
            }
        }
        if style != "overlay" {
            return;
        }
        for p in &d.placements {
            if p.border.is_none() {
                continue;
            }
            let active = Some(p.id) == self.foreground;
            if !active && !cfg.decoration.inactive_borders {
                self.borders.remove(&p.id);
                continue;
            }
            // The native 1px Windows 11 border would draw a second outline
            // inside the ring; suppress it while we draw our own.
            if self.dwm_touched.insert(p.id) {
                window::set_border_color(p.id, window::DWM_COLOR_NONE);
            }
            let prev_active = self.fades.get(&p.id).map(|f| f.to_active);
            if prev_active != Some(active) {
                let fade = cfg.animations.enabled && cfg.animations.border.enabled && prev_active.is_some();
                self.fades.insert(p.id, FadeState { start: if fade { Instant::now() } else { Instant::now() - Duration::from_secs(10) }, to_active: active });
            }
            self.borders.entry(p.id).or_insert_with(|| Border::new(p.id));
            let _ = placed;
            self.refresh_border(p.id);
        }
    }

    /// Redraws a border around the window's *actual* frame so it follows the
    /// window during animations and user drags.
    fn refresh_border(&mut self, id: WinId) {
        let cfg = self.wm.config();
        let Some(w) = self.wm.windows.get(&id) else { return };
        let Some(b) = self.borders.get_mut(&id) else { return };
        if self.hidden.contains(&id) || window::is_minimized(id) || window::is_maximized(id) || !window::is_visible(id) {
            b.hide();
            return;
        }
        let Some(frame) = window::frame_rect(id) else { return };
        let mon = &self.wm.monitors[self.wm.workspaces.get(&w.ws).map(|x| x.monitor).unwrap_or(0).min(self.wm.monitors.len() - 1)];
        let s = mon.scale();
        let bs = ((cfg.general.border_size as f32) * s).round().max(1.0) as i32;
        let radius = ((cfg.decoration.rounding as f32) * s).round() as i32 + bs;
        let outer = Rect::new(frame.x - bs, frame.y - bs, frame.w + 2 * bs, frame.h + 2 * bs);
        let fade = self.fades.get(&id);
        let (grad, t) = match fade {
            Some(f) => {
                let dur = cfg.animations.border.duration.max(1) as f32;
                let t = (f.start.elapsed().as_secs_f32() * 1000.0 / dur).min(1.0);
                let t = Bezier::new(cfg.bezier(&cfg.animations.border.curve)).eval(t as f64) as f32;
                let (from, to) = if f.to_active { (&cfg.inactive_border, &cfg.active_border) } else { (&cfg.active_border, &cfg.inactive_border) };
                (overlay::mix(from, to, t), t)
            }
            None => (cfg.active_border.clone(), 1.0),
        };
        b.update(outer, bs, radius, &grad);
        if t < 1.0 {
            self.arm_timer();
        }
    }

    fn animating(&self) -> bool {
        if !self.tweens.is_empty() {
            return true;
        }
        let dur = self.wm.config().animations.border.duration as f32 / 1000.0;
        self.fades.values().any(|f| f.start.elapsed().as_secs_f32() < dur)
    }

    fn arm_timer(&mut self) {
        if self.animating() && !self.timer_armed {
            let due = -((self.frame_ms * 10_000.0) as i64); // relative, 100 ns units
            unsafe {
                let _ = SetWaitableTimer(self.timer, &due, 0, None, None, false);
            }
            self.timer_armed = true;
        }
    }

    fn frame(&mut self) {
        self.timer_armed = false;
        self.stats.anim_frames += 1;
        let now = self.now_ms();
        let mut done = Vec::new();
        for (id, t) in &self.tweens {
            let (r, finished) = t.at(now);
            let offs = self.offsets.get(id).copied().unwrap_or_default();
            let same_size = t.from.w == t.to.w && t.from.h == t.to.h;
            if finished || !same_size {
                window::place(*id, r, offs);
            } else {
                window::place_move(*id, r, offs);
            }
            if finished {
                done.push(*id);
            }
        }
        for id in done {
            self.tweens.remove(&id);
        }
        let fading: Vec<WinId> = self.fades.iter().filter(|(_, f)| f.start.elapsed().as_millis() < self.wm.config().animations.border.duration as u128 + 20).map(|(id, _)| *id).collect();
        for id in fading {
            self.refresh_border(id);
        }
        self.arm_timer();
    }

    // ------------------------------------------------------------ events

    fn on_event(&mut self, ev: Event) {
        self.stats.events += 1;
        if !matches!(ev, Event::Location(_)) {
            log::trace!("event {ev:?}");
        }
        match ev {
            Event::Shown(id) | Event::Uncloaked(id) => {
                if self.wm.windows.contains_key(&id) || self.hidden.contains(&id) || self.wm.paused {
                    return;
                }
                self.try_manage(id, matches!(ev, Event::Uncloaked(_)));
            }
            Event::Hidden(id) | Event::Cloaked(id) => {
                if self.hidden.contains(&id) || recovery::is_hidden(id) {
                    return; // we hid it ourselves
                }
                if self.wm.windows.contains_key(&id) && !window::is_visible(id) || matches!(ev, Event::Cloaked(_)) && self.wm.windows.contains_key(&id) {
                    self.wm.remove_window(id);
                    self.forget(id);
                    self.apply(Anim::Windows);
                }
            }
            Event::Destroyed(id) => {
                let managed = self.wm.remove_window(id);
                window::forget_process(window::pid(id));
                self.forget(id);
                if managed {
                    self.apply(Anim::Windows);
                }
            }
            Event::Foreground(id) => {
                log::debug!("foreground {id:#x} {:?} ({})", window::title(id), window::class(id));
                self.foreground = Some(id);
                if !self.wm.windows.contains_key(&id) && !self.wm.paused && window::is_visible(id) {
                    self.try_manage(id, false);
                }
                if self.wm.windows.contains_key(&id) {
                    let before = self.wm.active_ws();
                    // With minimize-hiding, a hidden window that is no longer
                    // minimized was restored by the user (taskbar click).
                    let user_restored = self.wm.config().general.hide_method == "minimize" && !window::is_minimized(id);
                    self.wm.on_focus(Some(id), user_restored);
                    let after = self.wm.active_ws();
                    self.apply(if before != after { Anim::None } else { Anim::Windows });
                    self.update_tray();
                } else {
                    // Focus left the managed set (desktop, taskbar, ignored window).
                    let d = self.wm.desired();
                    let placed = d.placements.iter().map(|p| p.id).collect();
                    self.sync_borders(&d, &placed);
                }
            }
            Event::MoveSizeStart(id) => {
                if self.wm.windows.contains_key(&id) {
                    self.tweens.remove(&id);
                    self.moving = window::frame_rect(id).map(|r| (id, r));
                }
            }
            Event::MoveSizeEnd(id) => {
                if let Some((mid, before)) = self.moving.take() {
                    if mid == id {
                        if let Some(after) = window::frame_rect(id) {
                            let resized = (after.w - before.w).abs() > 2 || (after.h - before.h).abs() > 2;
                            let cursor = window::cursor_pos();
                            log::debug!("user {} {id:#x}: {before:?} -> {after:?}, cursor {cursor:?}", if resized { "resized" } else { "moved" });
                            self.wm.on_user_move(id, after, cursor, resized);
                            self.applied.remove(&id); // force re-place
                            self.apply(Anim::Windows);
                        }
                    }
                }
            }
            Event::MinimizeStart(id) => {
                if self.hidden.contains(&id) {
                    return;
                }
                if self.wm.windows.contains_key(&id) {
                    self.wm.set_minimized(id, true);
                    if let Some(b) = self.borders.get_mut(&id) {
                        b.hide();
                    }
                    self.applied.remove(&id);
                    self.apply(Anim::Windows);
                }
            }
            Event::MinimizeEnd(id) => {
                if self.hidden.contains(&id) {
                    return;
                }
                if self.wm.windows.contains_key(&id) {
                    self.wm.set_minimized(id, false);
                    self.applied.remove(&id);
                    self.apply(Anim::Windows);
                }
            }
            Event::Location(id) => self.on_location(id),
            Event::TitleChanged(id) => {
                if let Some(w) = self.wm.windows.get(&id) {
                    let mut f = w.facts.clone();
                    f.title = window::title(id);
                    self.wm.update_facts(f);
                }
            }
        }
    }

    fn try_manage(&mut self, id: WinId, adopting: bool) {
        let (f, c) = self.classify_window(id);
        if c.decision == Decision::Ignore {
            return;
        }
        log::debug!("managing {id:#x} {:?} ({}) as {:?}", f.title, f.process, c.decision);
        self.prepare_window(id, f.maximized);
        if self.wm.add_window(f, &c, adopting) {
            self.apply(Anim::Windows);
            self.update_tray();
        }
    }

    fn on_location(&mut self, id: WinId) {
        if !self.wm.windows.contains_key(&id) {
            return;
        }
        // Maximize/restore transitions.
        let zoomed = window::is_maximized(id);
        if zoomed != self.zoomed.contains(&id) {
            if zoomed {
                self.zoomed.insert(id);
            } else {
                self.zoomed.remove(&id);
                self.applied.remove(&id);
                self.apply(Anim::Windows);
            }
        }
        if self.borders.contains_key(&id) {
            self.refresh_border(id);
        }
        // Keep tiles where the layout wants them (apps sometimes restore
        // their own saved position right after showing). Give up on windows
        // that keep fighting (e.g. a minimum size larger than the tile).
        let in_move = self.moving.is_some_and(|(m, _)| m == id);
        if in_move || self.tweens.contains_key(&id) || zoomed || self.hidden.contains(&id) || window::is_minimized(id) {
            return;
        }
        let w = &self.wm.windows[&id];
        if w.mode != twm_core::state::Mode::Tiled {
            return;
        }
        let (Some(target), Some(actual)) = (self.applied.get(&id).copied(), window::frame_rect(id)) else { return };
        let off = (actual.x - target.x).abs().max((actual.y - target.y).abs()).max((actual.w - target.w).abs()).max((actual.h - target.h).abs());
        if off <= 2 {
            return;
        }
        let e = self.enforce.entry(id).or_insert(Enforce { count: 0, since: Instant::now(), gave_up: false });
        if e.since.elapsed() > Duration::from_secs(3) {
            *e = Enforce { count: 0, since: Instant::now(), gave_up: e.gave_up };
        }
        if e.gave_up {
            return;
        }
        e.count += 1;
        if e.count > 6 {
            e.gave_up = true;
            log::info!("{id:#x} ({}) keeps resizing itself away from its tile ({}x{} vs {}x{}); leaving it", w.facts.process, actual.w, actual.h, target.w, target.h);
            return;
        }
        // The window's invisible borders may have changed (DPI change etc.).
        let offs = window::frame_offsets(id);
        self.offsets.insert(id, offs);
        window::place(id, target, offs);
        self.stats.enforcements += 1;
    }

    fn on_host(&mut self, e: HostEvent) {
        match e {
            HostEvent::DisplayChanged | HostEvent::WorkAreaChanged | HostEvent::Resumed => {
                let (mons, hz) = monitors::enumerate();
                log::info!("display configuration changed: {} monitor(s)", mons.len());
                self.frame_ms = 1000.0 / hz.clamp(30, 360) as f64;
                self.wm.set_monitors(mons);
                self.applied.clear();
                let ids: Vec<WinId> = self.wm.windows.keys().copied().collect();
                for id in ids {
                    self.offsets.insert(id, window::frame_offsets(id));
                }
                self.apply(Anim::None);
            }
            HostEvent::SessionEnding => {
                recovery::restore_all();
            }
            HostEvent::TaskbarCreated => {
                self.tray.readd();
                self.update_tray();
            }
            HostEvent::TrayActivated => {
                let ws = self.wm.active_ws();
                let status = format!("Workspace {ws} · {} windows managed", self.wm.windows.len());
                self.tray.show_menu(&status, self.wm.paused, host::autostart_enabled());
            }
            HostEvent::Menu(item) => match item {
                MenuItem::Pause => self.run_action(&Action::TogglePause),
                MenuItem::Reload => self.reload(true),
                MenuItem::EditConfig => self.run_action(&Action::OpenConfig),
                MenuItem::OpenConfigFolder => host::shell_open(&self.config_path.parent().unwrap_or(&paths::config_dir()).display().to_string()),
                MenuItem::Settings => self.run_action(&Action::OpenSettings),
                MenuItem::ShowLog => host::shell_open(&paths::log_file().display().to_string()),
                MenuItem::Autostart => {
                    let on = !host::autostart_enabled();
                    match host::set_autostart(on) {
                        Ok(()) => log::info!("start with Windows: {on}"),
                        Err(e) => self.tray.notify("Could not change startup setting", &e, true),
                    }
                }
                MenuItem::Quit => self.quit = true,
            },
        }
    }

    // ------------------------------------------------------------ actions

    pub fn run_action(&mut self, a: &Action) {
        log::trace!("action {a:?}");
        let fx = self.wm.dispatch(a);
        let mut switch = None;
        for e in fx {
            match e {
                Effect::Exec(c) => {
                    if let Err(e) = host::exec(&c) {
                        log::warn!("exec: {e}");
                        self.tray.notify("Command failed", &e, true);
                    }
                }
                Effect::Close(id) => window::close(id),
                Effect::Reload => self.reload(true),
                Effect::Quit => self.quit = true,
                Effect::OpenSettings => self.open_settings(),
                Effect::OpenConfig => self.open_config(),
                Effect::Paused(p) => {
                    if p {
                        // Release everything: show hidden windows, drop borders.
                        for id in self.hidden.drain() {
                            window::show(id);
                        }
                        recovery::restore_all();
                        self.borders.clear();
                        self.tweens.clear();
                    }
                    self.applied.clear();
                    log::info!("tiling {}", if p { "paused" } else { "resumed" });
                }
                Effect::WorkspaceSwitch { monitor, from, to } => switch = Some((monitor, from, to)),
            }
        }
        self.apply_with(Anim::Windows, switch);
        self.update_tray();
    }

    fn open_config(&self) {
        let p = self.config_path.display().to_string();
        // Prefer the user's associated editor; fall back to Notepad when
        // .toml has no association (ShellExecute fails with code 31).
        if host::exec(&format!("\"{p}\"")).is_err() {
            let _ = host::exec(&format!("notepad.exe \"{p}\""));
        }
    }

    fn open_settings(&self) {
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("twm-settings.exe")));
        match exe.filter(|p| p.exists()) {
            Some(p) => {
                let _ = host::exec(&format!("\"{}\" --config \"{}\"", p.display(), self.config_path.display()));
            }
            None => self.open_config(),
        }
    }

    fn reload(&mut self, notify: bool) {
        let report = load_config(&self.config_path, Some(self.wm.config()));
        for d in &report.diagnostics {
            log::warn!("{d}");
        }
        let kb_changed = report.config.input.keyboard_backend != self.wm.config().input.keyboard_backend;
        self.diagnostics = report.diagnostics;
        self.wm.set_config(report.config);
        if kb_changed {
            self.start_keyboard();
        } else {
            let chords: Vec<_> = self.wm.config().binds.iter().map(|b| b.chord).collect();
            let r = self.kb.rebind(&chords);
            self.bind_failures = r.failed.iter().map(|(c, e)| format!("{c}: {e}")).collect();
        }
        // Re-evaluate decorations for all windows.
        self.applied.clear();
        self.borders.clear();
        self.fades.clear();
        for id in self.dwm_touched.drain().collect::<Vec<_>>() {
            window::set_border_color(id, window::DWM_COLOR_DEFAULT);
            window::set_corner_preference(id, "default");
        }
        let pref = self.wm.config().decoration.corner_preference.clone();
        if pref != "default" {
            for id in self.wm.windows.keys().copied().collect::<Vec<_>>() {
                window::set_corner_preference(id, &pref);
                self.dwm_touched.insert(id);
            }
        }
        self.apply(Anim::Windows);
        self.update_tray();
        if notify {
            self.report_diagnostics(false);
        }
        log::info!("configuration reloaded ({} diagnostics)", self.diagnostics.len());
    }

    fn check_config_change(&mut self) {
        let stamp = file_stamp(&self.config_path);
        if stamp != self.cfg_stamp {
            self.cfg_stamp = stamp;
            // Editors often write in several steps; wait for them to finish.
            self.reload_due = Some(Instant::now() + Duration::from_millis(200));
        }
    }

    // ------------------------------------------------------------ IPC

    fn handle_ipc(&mut self, req: Result<Request, String>) -> Response {
        let req = match req {
            Ok(r) => r,
            Err(e) => return Response::err(e),
        };
        match req {
            Request::Ping => Response::ok(json!("pong")),
            Request::Version => Response::ok(json!({ "version": env!("CARGO_PKG_VERSION"), "instance": paths::instance() })),
            Request::Dispatch { action } => match Action::parse(&action) {
                Ok(a) => {
                    self.run_action(&a);
                    Response::ok(json!({ "focused": self.wm.focused, "workspace": self.wm.active_ws() }))
                }
                Err(e) => Response::err(e),
            },
            Request::State => {
                let mut v = serde_json::to_value(&self.wm).unwrap_or_default();
                v["hidden"] = json!(self.hidden.iter().collect::<Vec<_>>());
                v["foreground"] = json!(self.foreground);
                v["config_path"] = json!(self.config_path.display().to_string());
                v["keyboard_backend"] = json!(self.kb.kind());
                v["desired"] = serde_json::to_value(self.wm.desired()).unwrap_or_default();
                v["actual"] = json!(self.wm.windows.keys().map(|id| (id.to_string(), window::frame_rect(*id))).collect::<BTreeMap<_, _>>());
                v["borders"] = json!(self.borders.iter().map(|(id, b)| (id.to_string(), json!({ "visible": b.is_visible(), "strips": b.hwnds() }))).collect::<BTreeMap<_, _>>());
                Response::ok(v)
            }
            Request::Windows => {
                let mut out = Vec::new();
                for id in window::enumerate() {
                    if !window::is_visible(id) && !self.hidden.contains(&id) {
                        continue;
                    }
                    let (f, c) = self.classify_window(id);
                    out.push(json!({
                        "hwnd": id, "title": f.title, "class": f.class, "process": f.process,
                        "managed": self.wm.windows.contains_key(&id), "decision": c.decision,
                        "workspace": self.wm.windows.get(&id).map(|w| w.ws),
                    }));
                }
                Response::ok(json!(out))
            }
            Request::Explain { hwnd } => {
                if !window::exists(hwnd) {
                    return Response::err(format!("no window {hwnd:#x}"));
                }
                let (f, c) = self.classify_window(hwnd);
                Response::ok(json!({ "facts": f, "classification": c, "managed": self.wm.windows.get(&hwnd) }))
            }
            Request::Binds => {
                let failed: HashSet<String> = self.bind_failures.iter().map(|s| s.split(':').next().unwrap_or("").to_string()).collect();
                let v: Vec<_> = self
                    .wm
                    .config()
                    .binds
                    .iter()
                    .map(|b| json!({ "chord": b.chord.to_string(), "action": format!("{:?}", b.action), "description": b.action.describe(), "registered": !failed.contains(&b.chord.to_string()) }))
                    .collect();
                Response::ok(json!({ "backend": self.kb.kind(), "binds": v }))
            }
            Request::Diagnostics => Response::ok(json!({ "config_path": self.config_path.display().to_string(), "diagnostics": self.diagnostics, "bind_failures": self.bind_failures })),
            Request::Reload => {
                self.reload(false);
                Response::ok(json!({ "diagnostics": self.diagnostics }))
            }
            Request::Stats => {
                let mut pmc = PROCESS_MEMORY_COUNTERS::default();
                unsafe {
                    let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32);
                }
                let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                unsafe {
                    let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
                }
                let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
                Response::ok(json!({
                    "uptime_ms": self.start.elapsed().as_millis() as u64,
                    "working_set_bytes": pmc.WorkingSetSize,
                    "private_bytes": pmc.PagefileUsage,
                    "cpu_kernel_ms": ft(k), "cpu_user_ms": ft(u),
                    "managed_windows": self.wm.windows.len(),
                    "borders": self.borders.len(),
                    "frame_ms": self.frame_ms,
                    "counters": self.stats,
                }))
            }
            Request::Quit => {
                self.quit = true;
                Response::ok(json!("bye"))
            }
        }
    }

    // ------------------------------------------------------------ loop

    pub fn run(&mut self) {
        let mut msg = MSG::default();
        while !self.quit {
            let handles: Vec<HANDLE> = if self.watch == INVALID_HANDLE_VALUE { vec![self.timer] } else { vec![self.timer, self.watch] };
            let timeout = match self.reload_due {
                Some(t) => t.saturating_duration_since(Instant::now()).as_millis() as u32,
                None => INFINITE,
            };
            let r = unsafe { MsgWaitForMultipleObjectsEx(Some(&handles), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
            if r.0 == WAIT_OBJECT_0.0 {
                self.frame();
            } else if r.0 == WAIT_OBJECT_0.0 + 1 && handles.len() > 1 {
                unsafe {
                    let _ = FindNextChangeNotification(self.watch);
                }
                self.check_config_change();
            }
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    match msg.message {
                        WM_QUIT => self.quit = true,
                        keyboard::WM_APP_BIND | WM_HOTKEY => {
                            let idx = msg.wParam.0;
                            if let Some(b) = self.wm.config().binds.get(idx) {
                                let a = b.action.clone();
                                log::debug!("bind {} -> {:?}", b.chord, a);
                                self.run_action(&a);
                            }
                        }
                        ipc::WM_APP_IPC => {
                            for p in ipc::take_pending() {
                                let resp = self.handle_ipc(p.request);
                                let _ = p.reply.send(resp);
                            }
                        }
                        _ => {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }
            }
            while events::pending() {
                for ev in events::drain() {
                    self.on_event(ev);
                }
            }
            for e in host::drain() {
                self.on_host(e);
            }
            if self.reload_due.is_some_and(|t| Instant::now() >= t) {
                self.reload_due = None;
                self.reload(true);
            }
        }
        self.shutdown();
    }

    pub fn shutdown(&mut self) {
        log::info!("shutting down");
        events::uninstall();
        self.kb.stop();
        let n = self.hidden.len();
        for id in self.hidden.drain() {
            window::show(id);
        }
        recovery::restore_all();
        if self.wm.config().general.restore_on_exit {
            let mut n = 0;
            for (id, p) in &self.original {
                if window::exists(*id) && self.wm.windows.contains_key(id) {
                    window::set_placement(*id, p);
                    n += 1;
                }
            }
            log::info!("restored the original placement of {n} window(s)");
        }
        for id in self.dwm_touched.drain() {
            if window::exists(id) {
                window::set_border_color(id, window::DWM_COLOR_DEFAULT);
                window::set_corner_preference(id, "default");
            }
        }
        self.borders.clear();
        self.tray.remove();
        if n > 0 {
            log::info!("restored {n} hidden window(s)");
        }
    }
}
