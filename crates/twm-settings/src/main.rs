//! twm-settings — settings and control window for TilingWindowsManager.
//!
//! Edits config.toml in place (comments and order are kept, see `doc.rs`),
//! validates every change with the same loader the manager uses, and talks
//! to the running manager over the IPC pipe for live state. Saving writes
//! the file; the manager notices and reloads it by itself.
#![windows_subsystem = "windows"]

#[path = "../../twm/src/paths.rs"]
#[allow(dead_code)]
mod paths;

mod doc;
mod link;

use egui::{Color32, RichText};
use link::Link;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use toml_edit::DocumentMut;
use twm_core::config::{self, Config, LoadReport, Severity};
use twm_core::ipc::Request;

const TITLE: &str = "TilingWindowsManager Settings";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Appearance,
    Layout,
    Animations,
    Binds,
    Windows,
    About,
}

const PAGES: &[(Page, &str)] = &[
    (Page::Overview, "Overview"),
    (Page::Appearance, "Appearance"),
    (Page::Layout, "Layout & behaviour"),
    (Page::Animations, "Animations"),
    (Page::Binds, "Key bindings"),
    (Page::Windows, "Windows & rules"),
    (Page::About, "About"),
];

const OK: Color32 = Color32::from_rgb(80, 200, 120);
const WARN: Color32 = Color32::from_rgb(230, 180, 60);
const ERR: Color32 = Color32::from_rgb(235, 90, 90);

fn validate(text: &str, path: &Path) -> LoadReport {
    let themes = paths::themes_dir();
    config::load_str(text, &path.display().to_string(), None, &|n| std::fs::read_to_string(themes.join(format!("{n}.toml"))).ok())
}

fn stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn shell_open(target: &Path) {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let t = HSTRING::from(target.as_os_str());
    unsafe {
        let r = ShellExecuteW(None, w!("open"), &t, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        // <= 32 is failure; .toml without a file association lands here.
        if r.0 as isize <= 32 {
            let _ = std::process::Command::new("notepad.exe").arg(target).spawn();
        }
    }
}

fn round(v: f32, digits: i32) -> f64 {
    let m = 10f64.powi(digits);
    (v as f64 * m).round() / m
}

/// Coloured status dot (painted: the bundled fonts have no bullet glyph).
fn dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, color);
        ui.colored_label(color, text);
    });
}

/// Rule tri-state (unset / yes / no) for `Option<bool>` effects.
fn tri(ui: &mut egui::Ui, id: impl egui::AsIdSalt, cur: Option<bool>, yes: &str, no: &str) -> Option<Option<bool>> {
    let mut v = cur;
    let text = |v: Option<bool>| match v {
        None => "not set".to_string(),
        Some(true) => yes.to_string(),
        Some(false) => no.to_string(),
    };
    egui::ComboBox::from_id_salt(id).selected_text(text(v)).width(150.0).show_ui(ui, |ui| {
        for o in [None, Some(true), Some(false)] {
            ui.selectable_value(&mut v, o, text(o));
        }
    });
    (v != cur).then_some(v)
}

struct App {
    path: PathBuf,
    defaults: DocumentMut,
    doc: DocumentMut,
    /// Text of the file as last loaded/saved.
    disk: String,
    disk_stamp: Option<(SystemTime, u64)>,
    exists: bool,
    /// Write CRLF line endings (the file had them).
    crlf: bool,
    /// The file on disk is not valid TOML; editing is disabled until fixed.
    syntax_error: Option<String>,
    report: LoadReport,
    dirty: bool,
    page: Page,
    link: Link,
    binds: Vec<(String, String)>,
    bind_filter: String,
    new_var: String,
    new_curve: String,
    status: Option<(String, bool, Instant)>,
    backed_up: bool,
    confirm_errors: bool,
    external_change: bool,
    last_disk_check: Instant,
    explain: Option<u64>,
    open_rule: Option<usize>,
    confirm_close: bool,
    allow_close: bool,
    zoom: f32,
}

impl App {
    fn new(path: PathBuf) -> App {
        let defaults: DocumentMut = config::DEFAULT_CONFIG.parse().expect("built-in default config parses");
        let mut app = App {
            report: validate(config::DEFAULT_CONFIG, &path),
            path,
            doc: defaults.clone(),
            defaults,
            disk: String::new(),
            disk_stamp: None,
            exists: false,
            crlf: false,
            syntax_error: None,
            dirty: false,
            page: Page::Overview,
            link: Link::new(paths::pipe_name()),
            binds: Vec::new(),
            bind_filter: String::new(),
            new_var: String::new(),
            new_curve: String::new(),
            status: None,
            backed_up: false,
            confirm_errors: false,
            external_change: false,
            last_disk_check: Instant::now(),
            explain: None,
            open_rule: None,
            confirm_close: false,
            allow_close: false,
            zoom: 1.0,
        };
        app.load();
        // Test hook: start on a given page (used by screenshot checks).
        if let Ok(p) = std::env::var("TWM_SETTINGS_PAGE") {
            if let Some((page, _)) = PAGES.iter().find(|(_, n)| n.to_ascii_lowercase().starts_with(&p.to_ascii_lowercase())) {
                app.page = *page;
            }
        }
        app
    }

    fn cfg(&self) -> &Config {
        &self.report.config
    }

    /// (Re)load the file from disk, discarding unsaved edits.
    fn load(&mut self) {
        self.disk_stamp = stamp(&self.path);
        self.external_change = false;
        self.confirm_errors = false;
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                self.exists = true;
                self.crlf = doc::uses_crlf(&text);
                self.disk = text.clone();
                match text.parse::<DocumentMut>() {
                    Ok(d) => {
                        self.doc = d;
                        self.syntax_error = None;
                    }
                    Err(e) => self.syntax_error = Some(e.to_string()),
                }
            }
            Err(_) => {
                // No file yet: start from the commented default; Save creates it.
                self.exists = false;
                self.crlf = false;
                self.disk = String::new();
                self.doc = self.defaults.clone();
                self.syntax_error = None;
            }
        }
        self.binds = doc::binds(&self.doc);
        self.changed();
    }

    fn changed(&mut self) {
        let text = doc::render(&self.doc, self.crlf);
        self.report = validate(&text, &self.path);
        self.dirty = text != self.disk;
        self.confirm_errors = false;
    }

    fn set(&mut self, path: &[&str], v: impl Into<toml_edit::Value>) {
        if let Err(e) = doc::set(&mut self.doc, path, v) {
            self.flash(format!("cannot change {}: {e}", path.join(".")), true);
        }
        self.changed();
    }

    fn unset(&mut self, path: &[&str]) {
        let _ = doc::remove(&mut self.doc, path);
        self.changed();
    }

    fn flash(&mut self, msg: impl Into<String>, error: bool) {
        self.status = Some((msg.into(), error, Instant::now()));
    }

    fn error_count(&self) -> usize {
        self.report.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()
    }

    fn save(&mut self) {
        let text = doc::render(&self.doc, self.crlf);
        let r = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Keep the file as it was before this session's first save.
            if !self.backed_up && self.path.exists() {
                std::fs::copy(&self.path, self.path.with_extension("toml.bak"))?;
                self.backed_up = true;
            }
            // Write-then-rename so the manager never reads a half-written file.
            let tmp = self.path.with_extension("toml.tmp");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &self.path)
        })();
        match r {
            Ok(()) => {
                self.disk = text;
                self.disk_stamp = stamp(&self.path);
                self.exists = true;
                self.dirty = false;
                self.confirm_errors = false;
                self.flash("Saved. The manager reloads the file automatically.", false);
            }
            Err(e) => self.flash(format!("Could not save {}: {e}", self.path.display()), true),
        }
    }

    fn check_disk(&mut self) {
        if self.last_disk_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_disk_check = Instant::now();
        let now = stamp(&self.path);
        if now != self.disk_stamp && !self.external_change {
            if self.dirty {
                self.external_change = true;
            } else {
                self.load();
                self.flash("The file changed on disk and was reloaded.", false);
            }
        }
    }

    fn dispatch(&mut self, action: &str, ctx: &egui::Context) {
        self.link.request("dispatch", Request::Dispatch { action: action.into() }, ctx);
    }

    fn manager_exe() -> Option<PathBuf> {
        std::env::current_exe().ok()?.parent().map(|d| d.join("twm.exe")).filter(|p| p.exists())
    }

    // ---------------------------------------------------------------- widgets

    /// Show a "Default" button when `path` differs from the built-in default.
    fn reset_button(&mut self, ui: &mut egui::Ui, path: &[&str]) {
        let cur = doc::get(&self.doc, path).map(|i| i.to_string().trim().to_string());
        let def = doc::get(&self.defaults, path).map(|i| i.to_string().trim().to_string());
        if cur != def && ui.small_button("Default").on_hover_text("Back to the built-in default").clicked() {
            match doc::get(&self.defaults, path).and_then(|i| i.as_value()).cloned() {
                Some(v) => self.set(path, v),
                None => self.unset(path),
            }
        }
    }

    /// Row label; returns its id so the control can name itself after it
    /// (screen readers and UI Automation see "Border width", not "slider").
    fn help(ui: &mut egui::Ui, label: &str, help: &str) -> egui::Id {
        ui.label(label).on_hover_text(help).id
    }

    fn int(&mut self, ui: &mut egui::Ui, path: &[&str], label: &str, help: &str, cur: i64, lo: i64, hi: i64, unit: &str) {
        let lid = Self::help(ui, label, help);
        ui.horizontal(|ui| {
            let mut v = cur;
            if ui.add(egui::Slider::new(&mut v, lo..=hi).suffix(unit)).labelled_by(lid).changed() {
                self.set(path, v);
            }
            self.reset_button(ui, path);
        });
        ui.end_row();
    }

    fn float(&mut self, ui: &mut egui::Ui, path: &[&str], label: &str, help: &str, cur: f32, lo: f32, hi: f32) {
        let lid = Self::help(ui, label, help);
        ui.horizontal(|ui| {
            let mut v = cur;
            if ui.add(egui::Slider::new(&mut v, lo..=hi).step_by(0.01).fixed_decimals(2)).labelled_by(lid).changed() {
                self.set(path, round(v, 2));
            }
            self.reset_button(ui, path);
        });
        ui.end_row();
    }

    fn boolean(&mut self, ui: &mut egui::Ui, path: &[&str], label: &str, help: &str, cur: bool) {
        let lid = Self::help(ui, label, help);
        ui.horizontal(|ui| {
            let mut v = cur;
            if ui.checkbox(&mut v, "").labelled_by(lid).changed() {
                self.set(path, v);
            }
            self.reset_button(ui, path);
        });
        ui.end_row();
    }

    fn choice(&mut self, ui: &mut egui::Ui, path: &[&str], label: &str, help: &str, cur: &str, options: &[(&str, &str)]) {
        let lid = Self::help(ui, label, help);
        ui.horizontal(|ui| {
            let mut v = cur.to_string();
            let shown = options.iter().find(|o| o.0 == cur).map_or(cur, |o| o.1);
            egui::ComboBox::from_id_salt(path.join(".")).selected_text(shown).width(220.0).show_ui(ui, |ui| {
                for (val, text) in options {
                    ui.selectable_value(&mut v, val.to_string(), *text);
                }
            })
            .response
            .labelled_by(lid);
            if v != cur {
                self.set(path, v);
            }
            self.reset_button(ui, path);
        });
        ui.end_row();
    }

    fn grid(id: &str) -> egui::Grid {
        egui::Grid::new(id).num_columns(2).spacing([28.0, 10.0]).min_col_width(190.0)
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.add_space(14.0);
        ui.label(RichText::new(title).strong().size(16.0));
        ui.separator();
    }

    fn diag_lines(&self, ui: &mut egui::Ui, prefix: &str) {
        for d in self.report.diagnostics.iter().filter(|d| d.setting.starts_with(prefix)) {
            let c = if d.severity == Severity::Error { ERR } else { WARN };
            ui.colored_label(c, format!("{}: {}", d.setting, d.message));
        }
    }

    // ------------------------------------------------------------------ pages

    fn page_overview(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.link.refresh("state", Request::State, Duration::from_secs(1), &ctx);
        self.link.refresh("stats", Request::Stats, Duration::from_secs(2), &ctx);
        self.link.refresh("diag", Request::Diagnostics, Duration::from_secs(3), &ctx);
        ui.heading("Overview");
        let state = self.link.get("state");
        match &state {
            Some(Ok(s)) => {
                let paused = s["paused"].as_bool().unwrap_or(false);
                ui.horizontal(|ui| {
                    if paused {
                        dot(ui, WARN, "Tiling is paused");
                    } else {
                        dot(ui, OK, "The window manager is running");
                    }
                    if let Some(Ok(v)) = self.link.get("version") {
                        ui.label(format!("version {}", v["version"].as_str().unwrap_or("?")));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(if paused { "Resume tiling" } else { "Pause tiling" }).on_hover_text("Pausing shows every hidden window and stops arranging windows").clicked() {
                        self.dispatch("togglepause", &ctx);
                    }
                    if ui.button("Reload configuration").clicked() {
                        self.link.request("reload", Request::Reload, &ctx);
                    }
                    if ui.button("Re-arrange windows").clicked() {
                        self.dispatch("retile", &ctx);
                    }
                    if ui.button("Quit window manager").on_hover_text("Shows all hidden windows and puts windows back where they were").clicked() {
                        self.link.request("quit", Request::Quit, &ctx);
                    }
                });
            }
            Some(Err(e)) => {
                dot(ui, WARN, "The window manager is not running");
                ui.label(RichText::new(e).small().weak());
                match Self::manager_exe() {
                    Some(exe) => {
                        if ui.button("Start window manager").clicked() {
                            match std::process::Command::new(&exe).arg("--config").arg(&self.path).spawn() {
                                Ok(_) => self.flash("Starting the window manager…", false),
                                Err(e) => self.flash(format!("Could not start {}: {e}", exe.display()), true),
                            }
                        }
                    }
                    None => {
                        ui.label("twm.exe was not found next to this program.");
                    }
                }
            }
            None => {
                ui.label("Connecting…");
            }
        }
        if let Some(Ok(r)) = self.link.take("reload") {
            let n = r["diagnostics"].as_array().map_or(0, |a| a.len());
            self.flash(if n == 0 { "Reloaded without problems.".to_string() } else { format!("Reloaded; {n} problem(s) — see below.") }, n > 0);
        }

        if let Some(Ok(s)) = &state {
            Self::section(ui, "Monitors");
            egui::Grid::new("monitors").num_columns(5).spacing([24.0, 6.0]).striped(true).show(ui, |ui| {
                for h in ["#", "Device", "Resolution", "Position", "Workspace"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, m) in s["monitors"].as_array().into_iter().flatten().enumerate() {
                    let r = &m["rect"];
                    ui.label(format!("{}", i + 1));
                    ui.label(m["id"].as_str().unwrap_or("?"));
                    ui.label(format!("{} × {} @ {} dpi", r["w"], r["h"], m["dpi"]));
                    ui.label(format!("{}, {}", r["x"], r["y"]));
                    ui.label(format!("{}", m["active_ws"]));
                    ui.end_row();
                }
            });
            Self::section(ui, "Workspaces");
            let windows = s["windows"].as_object().cloned().unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                for (id, w) in s["workspaces"].as_object().into_iter().flatten() {
                    let n = windows.values().filter(|x| x["ws"].to_string() == *id).count();
                    let active = s["monitors"].as_array().into_iter().flatten().any(|m| m["active_ws"].to_string() == *id);
                    let text = format!("{id}  ({n} window{})", if n == 1 { "" } else { "s" });
                    let mon = w["monitor"].as_u64().unwrap_or(0) + 1;
                    if ui.selectable_label(active, text).on_hover_text(format!("monitor {mon}; click to switch")).clicked() {
                        self.dispatch(&format!("workspace {id}"), &ctx);
                    }
                }
            });
        }
        if let Some(Ok(st)) = self.link.get("stats") {
            Self::section(ui, "Resource use (measured by the manager)");
            let mib = |v: &Value| v.as_f64().map_or("?".into(), |b| format!("{:.1} MiB", b / 1048576.0));
            ui.label(format!(
                "Memory: {} working set, {} private  ·  CPU time: {} ms  ·  Managed windows: {}  ·  Up {} min",
                mib(&st["working_set_bytes"]),
                mib(&st["private_bytes"]),
                st["cpu_kernel_ms"].as_u64().unwrap_or(0) + st["cpu_user_ms"].as_u64().unwrap_or(0),
                st["managed_windows"],
                st["uptime_ms"].as_u64().unwrap_or(0) / 60000
            ));
        }

        Self::section(ui, "Configuration problems");
        if self.report.diagnostics.is_empty() {
            ui.colored_label(OK, "No problems in the configuration being edited.");
        } else {
            for d in &self.report.diagnostics {
                let c = if d.severity == Severity::Error { ERR } else { WARN };
                let line = d.line.map(|l| format!(" (line {l})")).unwrap_or_default();
                ui.colored_label(c, format!("{}{line}: {}", d.setting, d.message));
            }
        }
        if let Some(Ok(d)) = self.link.get("diag") {
            for f in d["bind_failures"].as_array().into_iter().flatten() {
                ui.colored_label(WARN, format!("Key binding not registered by the manager: {}", f.as_str().unwrap_or("?")));
            }
        }
    }

    fn gradient_preview(ui: &mut egui::Ui, text: &str) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 18.0), egui::Sense::hover());
        match twm_core::color::Gradient::parse(text) {
            Ok(g) => {
                let n = 48;
                for i in 0..n {
                    let c = g.sample(i as f32 / (n - 1) as f32);
                    let x0 = rect.left() + rect.width() * i as f32 / n as f32;
                    let x1 = rect.left() + rect.width() * (i + 1) as f32 / n as f32 + 0.5;
                    let r = egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
                    ui.painter().rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a));
                }
            }
            Err(_) => {
                ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ERR), egui::StrokeKind::Inside);
            }
        }
    }

    fn color_field(&mut self, ui: &mut egui::Ui, key: &str, label: &str, effective: String) {
        let path = ["decoration", key];
        let lid = Self::help(ui, label, "Hyprland colour syntax: one or more colours (rgba(RRGGBBAA), rgb(RRGGBB), 0xAARRGGBB, #RRGGBB) and an optional angle such as 45deg for a gradient.");
        ui.vertical(|ui| {
            let in_file = doc::get(&self.doc, &path).and_then(|i| i.as_str()).map(str::to_string);
            ui.horizontal(|ui| {
                let mut text = in_file.clone().unwrap_or(effective);
                Self::gradient_preview(ui, &text);
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(280.0).min_size(egui::vec2(280.0, 0.0)).font(egui::TextStyle::Monospace)).labelled_by(lid).changed() {
                    self.set(&path, text);
                }
                if in_file.is_some() && ui.small_button("Use theme").on_hover_text("Remove this override so the theme's colour applies").clicked() {
                    self.unset(&path);
                }
            });
            if in_file.is_none() {
                ui.label(RichText::new("from the theme").small().weak());
            }
            self.diag_lines(ui, &format!("decoration.{key}"));
        });
        ui.end_row();
    }

    fn theme_names() -> Vec<String> {
        let mut v: Vec<String> = config::BUILTIN_THEMES.iter().map(|(n, _)| n.to_string()).collect();
        if let Ok(rd) = std::fs::read_dir(paths::themes_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) {
                    if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                        if !v.iter().any(|x| x == s) {
                            v.push(s.to_string());
                        }
                    }
                }
            }
        }
        v
    }

    fn page_appearance(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        let c = self.cfg().clone();
        Self::section(ui, "Theme and borders");
        Self::grid("appearance").show(ui, |ui| {
            let lid = Self::help(ui, "Theme", "Border colours. Custom themes live in the themes folder next to config.toml.");
            ui.horizontal(|ui| {
                let cur = c.theme.clone().unwrap_or_default();
                let mut v = cur.clone();
                egui::ComboBox::from_id_salt("theme").selected_text(if cur.is_empty() { "(none)" } else { &cur }).width(220.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut v, String::new(), "(none)");
                    for n in Self::theme_names() {
                        ui.selectable_value(&mut v, n.clone(), n);
                    }
                })
                .response
                .labelled_by(lid);
                if v != cur {
                    if v.is_empty() {
                        self.unset(&["theme"]);
                    } else {
                        self.set(&["theme"], v);
                    }
                }
                if ui.small_button("Open themes folder").clicked() {
                    let _ = std::fs::create_dir_all(paths::themes_dir());
                    shell_open(&paths::themes_dir());
                }
            });
            ui.end_row();
            self.color_field(ui, "active_border", "Focused window border", c.active_border.to_config_string());
            self.color_field(ui, "inactive_border", "Other window borders", c.inactive_border.to_config_string());
            self.choice(ui, &["decoration", "border_style"], "Border style", "overlay: a gradient ring drawn around the window. dwm: Windows 11's own 1-pixel border recoloured. none: no border.", &c.decoration.border_style, &[("overlay", "Overlay ring (gradients, any width)"), ("dwm", "Windows border (1 px, solid)"), ("none", "No border")]);
            self.int(ui, &["general", "border_size"], "Border width", "Thickness of the overlay ring.", c.general.border_size as i64, 0, 20, " px");
            self.int(ui, &["decoration", "rounding"], "Ring corner radius", "Windows 11 windows use 8 px.", c.decoration.rounding as i64, 0, 50, " px");
            self.choice(ui, &["decoration", "corner_preference"], "Window corners", "Asks Windows 11 to round or square managed windows' corners.", &c.decoration.corner_preference, &[("default", "Windows default"), ("round", "Round"), ("small", "Small round"), ("square", "Square")]);
            self.boolean(ui, &["decoration", "inactive_borders"], "Borders on unfocused windows", "Draw the inactive colour around windows that are not focused.", c.decoration.inactive_borders);
        });
        Self::section(ui, "Gaps");
        Self::grid("gaps").show(ui, |ui| {
            self.int(ui, &["general", "gaps_in"], "Between windows", "Space around each window; neighbours are separated by twice this.", c.general.gaps_in as i64, 0, 200, " px");
            self.int(ui, &["general", "gaps_out"], "Screen edge", "Space between windows and the edge of the monitor's work area.", c.general.gaps_out as i64, 0, 400, " px");
        });
    }

    fn page_layout(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layout & behaviour");
        let c = self.cfg().clone();
        Self::section(ui, "Layout");
        Self::grid("layout").show(ui, |ui| {
            self.choice(ui, &["general", "layout"], "Default layout", "Layout for new workspaces. Each workspace can switch layout with the `layout` action.", &c.general.layout, &[("dwindle", "Dwindle (binary splits)"), ("master", "Master and stack"), ("monocle", "Monocle (one at a time)")]);
        });
        Self::section(ui, "Dwindle");
        Self::grid("dwindle").show(ui, |ui| {
            self.float(ui, &["dwindle", "split_width_multiplier"], "Side-by-side bias", "A container splits side by side when width > height × this value.", c.dwindle.split_width_multiplier, 0.1, 3.0);
            self.float(ui, &["dwindle", "default_split_ratio"], "Default split ratio", "1.0 = 50/50; lower gives the first window less space.", c.dwindle.default_split_ratio, 0.1, 1.9);
            self.boolean(ui, &["dwindle", "preserve_split"], "Keep split direction", "Do not change a split's direction when its container changes shape.", c.dwindle.preserve_split);
            let lid = Self::help(ui, "New window goes", "Where a new window is placed relative to the focused one.");
            ui.horizontal(|ui| {
                let cur = c.dwindle.force_split as i64;
                let mut v = cur;
                let text = |v: i64| if v == 1 { "Left / above" } else { "Right / below" };
                egui::ComboBox::from_id_salt("force_split").selected_text(text(cur)).width(220.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut v, 0, text(0));
                    ui.selectable_value(&mut v, 1, text(1));
                })
                .response
                .labelled_by(lid);
                if v != cur {
                    self.set(&["dwindle", "force_split"], v);
                }
                self.reset_button(ui, &["dwindle", "force_split"]);
            });
            ui.end_row();
        });
        Self::section(ui, "Master");
        Self::grid("master").show(ui, |ui| {
            self.float(ui, &["master", "mfact"], "Master area size", "Fraction of the screen used by the master area.", c.master.mfact, 0.05, 0.95);
            self.choice(ui, &["master", "orientation"], "Master position", "Which side of the screen the master area is on.", &c.master.orientation, &[("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom"), ("center", "Center")]);
            self.choice(ui, &["master", "new_status"], "New windows become", "Whether a new window joins the master area or the stack.", &c.master.new_status, &[("slave", "Stack"), ("master", "Master"), ("inherit", "Same as the focused window")]);
            self.boolean(ui, &["master", "new_on_top"], "New windows on top", "Insert new windows at the top of their area instead of the bottom.", c.master.new_on_top);
        });
        Self::section(ui, "Window management");
        Self::grid("general").show(ui, |ui| {
            let g = &c.general;
            self.boolean(ui, &["general", "manage_by_default"], "Tile new windows", "Off: only windows enabled by a rule are managed (opt-in mode).", g.manage_by_default);
            self.boolean(ui, &["general", "float_dialogs"], "Float dialogs", "Owned windows such as Save As dialogs float instead of tiling.", g.float_dialogs);
            self.boolean(ui, &["general", "float_unresizable"], "Float fixed-size windows", "Windows without a resize border float.", g.float_unresizable);
            self.boolean(ui, &["general", "focus_new_windows"], "Focus new windows", "Give keyboard focus to newly managed windows.", g.focus_new_windows);
            self.boolean(ui, &["general", "focus_on_activate"], "Follow apps that activate themselves", "When a window on a hidden workspace activates itself, switch to its workspace. Off: stay and keep focus on the visible window.", g.focus_on_activate);
            self.boolean(ui, &["general", "mouse_follows_focus"], "Mouse follows focus", "Move the cursor to windows focused from the keyboard.", g.mouse_follows_focus);
            self.boolean(ui, &["general", "drag_to_swap"], "Drag to swap", "Dropping a dragged tiled window onto another swaps them.", g.drag_to_swap);
            self.boolean(ui, &["general", "restore_on_exit"], "Restore on exit", "On quit, put windows that existed before the manager started back where they were.", g.restore_on_exit);
            self.choice(ui, &["general", "hide_method"], "Hide other workspaces by", "hide: windows disappear from the taskbar while on another workspace (recovered after a crash). minimize: they stay on the taskbar, minimized.", &g.hide_method, &[("hide", "Hiding (cleaner taskbar)"), ("minimize", "Minimizing")]);
        });
    }

    fn curve_plot(ui: &mut egui::Ui, p: [f32; 4]) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 40.0), egui::Sense::hover());
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
        let b = twm_core::anim::Bezier::new(p);
        let pts: Vec<egui::Pos2> = (0..=32)
            .map(|i| {
                let x = i as f64 / 32.0;
                egui::pos2(rect.left() + rect.width() * x as f32, rect.bottom() - rect.height() * b.eval(x).clamp(-0.2, 1.2) as f32)
            })
            .collect();
        painter.line(pts, egui::Stroke::new(1.5, ui.visuals().selection.bg_fill));
    }

    fn page_animations(&mut self, ui: &mut egui::Ui) {
        ui.heading("Animations");
        let c = self.cfg().clone();
        Self::grid("anim-global").show(ui, |ui| {
            self.boolean(ui, &["animations", "enabled"], "Animations", "Turn off to move windows instantly.", c.animations.enabled);
        });
        let curves: Vec<String> = c.animations.bezier.keys().cloned().collect();
        for (key, title, spec) in [("windows", "Windows moving between tiles", &c.animations.windows), ("workspaces", "Workspace switching", &c.animations.workspaces), ("border", "Border colour fade", &c.animations.border)] {
            Self::section(ui, title);
            ui.add_enabled_ui(c.animations.enabled, |ui| {
                Self::grid(key).show(ui, |ui| {
                    self.boolean(ui, &["animations", key, "enabled"], "Enabled", "", spec.enabled);
                    self.int(ui, &["animations", key, "duration"], "Duration", "", spec.duration as i64, 0, 2000, " ms");
                    let lid = Self::help(ui, "Curve", "Easing curve; define more below.");
                    ui.horizontal(|ui| {
                        let mut v = spec.curve.clone();
                        egui::ComboBox::from_id_salt(format!("curve-{key}")).selected_text(&v).width(160.0).show_ui(ui, |ui| {
                            for n in &curves {
                                ui.selectable_value(&mut v, n.clone(), n);
                            }
                        })
                        .response
                        .labelled_by(lid);
                        if v != spec.curve {
                            self.set(&["animations", key, "curve"], v.clone());
                        }
                        Self::curve_plot(ui, c.bezier(&v));
                    });
                    ui.end_row();
                    if key == "workspaces" {
                        let style = spec.style.clone().unwrap_or_else(|| "slide".into());
                        self.choice(ui, &["animations", key, "style"], "Style", "", &style, &[("slide", "Slide horizontally"), ("slidevert", "Slide vertically"), ("none", "No movement (instant)")]);
                    }
                });
            });
        }
        Self::section(ui, "Curves");
        ui.label(RichText::new("Cubic Bézier control points x1, y1, x2, y2 — the same numbers as CSS cubic-bezier() and Hyprland's bezier.").weak());
        egui::Grid::new("curves").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
            for (name, p) in &c.animations.bezier {
                ui.label(RichText::new(name).monospace());
                ui.horizontal(|ui| {
                    let mut q = *p;
                    let mut changed = false;
                    for (i, v) in q.iter_mut().enumerate() {
                        // x values must stay in 0..1 for the curve to be a function of time.
                        let range = if i % 2 == 0 { 0.0..=1.0 } else { -1.0..=2.0 };
                        changed |= ui.add(egui::DragValue::new(v).speed(0.01).range(range).fixed_decimals(2)).changed();
                    }
                    if changed {
                        let arr: toml_edit::Array = q.iter().map(|v| round(*v, 3)).collect();
                        self.set(&["animations", "bezier", name], arr);
                    }
                    Self::curve_plot(ui, q);
                });
                let used = [&c.animations.windows.curve, &c.animations.workspaces.curve, &c.animations.border.curve].contains(&name);
                if ui.add_enabled(!used, egui::Button::new("Remove").small()).on_disabled_hover_text("In use by an animation").clicked() {
                    self.unset(&["animations", "bezier", name]);
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_curve).hint_text("new curve name").desired_width(160.0).min_size(egui::vec2(160.0, 0.0)));
            let name = self.new_curve.trim().to_string();
            let ok = !name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') && !c.animations.bezier.contains_key(&name);
            if ui.add_enabled(ok, egui::Button::new("Add curve")).clicked() {
                let arr: toml_edit::Array = [0.25, 0.1, 0.25, 1.0].into_iter().collect();
                self.set(&["animations", "bezier", &name], arr);
                self.new_curve.clear();
            }
        });
        self.diag_lines(ui, "animations");
    }

    fn page_binds(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.link.refresh("binds", Request::Binds, Duration::from_secs(2), &ctx);
        ui.heading("Key bindings");
        let c = self.cfg().clone();
        Self::grid("kb").show(ui, |ui| {
            self.choice(ui, &["input", "keyboard_backend"], "Keyboard method", "hook: a low-level keyboard hook; Win-key chords work and replace Windows shortcuts that use the same keys. hotkey: RegisterHotKey; no hook, but chords Windows already owns (Win+1, Win+Left…) cannot be used.", &c.input.keyboard_backend, &[("hook", "Keyboard hook (recommended)"), ("hotkey", "Registered hotkeys")]);
        });

        Self::section(ui, "Variables");
        ui.label(RichText::new("Use $name in chords and actions, e.g. $mod+return = exec $terminal.").weak());
        egui::Grid::new("vars").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
            for (k, v) in c.variables.clone() {
                ui.label(RichText::new(format!("${k}")).monospace());
                let mut t = v.clone();
                if ui.add(egui::TextEdit::singleline(&mut t).desired_width(240.0).min_size(egui::vec2(240.0, 0.0))).changed() {
                    self.set(&["variables", &k], t);
                }
                if ui.small_button("Remove").clicked() {
                    self.unset(&["variables", &k]);
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_var).hint_text("new variable name").desired_width(160.0).min_size(egui::vec2(160.0, 0.0)));
            let name = self.new_var.trim().trim_start_matches('$').to_string();
            let ok = !name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') && !c.variables.contains_key(&name);
            if ui.add_enabled(ok, egui::Button::new("Add variable")).clicked() {
                self.set(&["variables", &name], "");
                self.new_var.clear();
            }
        });

        Self::section(ui, "Bindings");
        let live = self.link.get("binds");
        let registered: Vec<(twm_core::keys::Chord, bool)> = match &live {
            Some(Ok(v)) => v["binds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| Some((twm_core::keys::Chord::parse(b["chord"].as_str()?, &|_| None).ok()?, b["registered"].as_bool().unwrap_or(true))))
                .collect(),
            _ => Vec::new(),
        };
        let vars = c.variables.clone();
        let lookup = |n: &str| vars.get(n).cloned();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.bind_filter).hint_text("Filter…").desired_width(220.0).min_size(egui::vec2(220.0, 0.0)));
            if ui.button("Add binding").clicked() {
                self.binds.push((String::new(), String::new()));
                self.bind_filter.clear();
            }
            if ui.button("Restore default bindings").on_hover_text("Replace all bindings with the built-in defaults").clicked() {
                self.binds = doc::binds(&self.defaults);
                doc::set_binds(&mut self.doc, &self.binds);
                self.changed();
            }
            if live.as_ref().is_some_and(|r| r.is_err()) {
                ui.label(RichText::new("(manager not running — registration status unknown)").weak());
            }
        });
        let filter = self.bind_filter.to_ascii_lowercase();
        let mut edited = false;
        let mut delete = None;
        egui::Grid::new("binds").num_columns(4).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
            for h in ["Keys", "Action", "", "Status"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for (i, (chord, action)) in self.binds.iter_mut().enumerate() {
                if !filter.is_empty() && !chord.to_ascii_lowercase().contains(&filter) && !action.to_ascii_lowercase().contains(&filter) {
                    continue;
                }
                edited |= ui.add(egui::TextEdit::singleline(chord).desired_width(190.0).min_size(egui::vec2(190.0, 0.0)).font(egui::TextStyle::Monospace).hint_text("win+shift+x")).changed();
                edited |= ui.add(egui::TextEdit::singleline(action).desired_width(250.0).min_size(egui::vec2(250.0, 0.0)).font(egui::TextStyle::Monospace).hint_text("focus left")).changed();
                if ui.small_button("Remove").clicked() {
                    delete = Some(i);
                }
                let parsed = twm_core::keys::Chord::parse(chord, &lookup);
                let act = twm_core::keys::expand_vars(action, &lookup).and_then(|a| twm_core::actions::Action::parse(&a));
                let diag = self.report.diagnostics.iter().find(|d| d.setting == format!("binds.\"{}\"", chord.trim()));
                match (&parsed, &act, diag) {
                    (Err(e), _, _) | (_, Err(e), _) => {
                        ui.colored_label(ERR, e);
                    }
                    (_, _, Some(d)) => {
                        ui.colored_label(if d.severity == Severity::Error { ERR } else { WARN }, &d.message);
                    }
                    (Ok(ch), Ok(a), None) => {
                        let reg = registered.iter().find(|(c, _)| c == ch).map(|r| r.1);
                        let (col, note) = match reg {
                            Some(false) => (WARN, " — not registered: another program owns this chord"),
                            _ => (ui.visuals().text_color(), ""),
                        };
                        ui.colored_label(col, format!("{}{note}", a.describe()));
                    }
                }
                ui.end_row();
            }
        });
        if let Some(i) = delete {
            self.binds.remove(i);
            edited = true;
        }
        if edited {
            doc::set_binds(&mut self.doc, &self.binds);
            self.changed();
        }
        egui::CollapsingHeader::new("Key names and actions").show(ui, |ui| {
            ui.label("Modifiers: win, ctrl, alt, shift. Keys: a–z, 0–9, f1–f24, left, right, up, down, return, space, tab, escape, backspace, delete, insert, home, end, pageup, pagedown, comma, period, slash, backslash, bracketleft, bracketright, grave, minus, equal, semicolon, apostrophe, numpad keys. A binding needs win, ctrl or alt (function keys may be used alone, except F12). Win+L cannot be bound.");
            ui.add_space(6.0);
            ui.label("Actions: focus/move <left|right|up|down> · resize <dir> [amount] · workspace <n|next|prev|previous> · movetoworkspace <n> · movetoworkspacesilent <n> · focusmonitor <n|next|prev> · movetomonitor <n|next|prev> · layout <dwindle|master|monocle|next> · togglefloating · fullscreen · togglesplit · splitratio <+/-x | exact x> · swapwithmaster · orientationnext · cyclenext · cycleprev · close · exec <command> · reload · togglepause · retile · settings · openconfig · quit");
        });
    }

    fn page_windows(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.link.refresh("windows", Request::Windows, Duration::from_secs(2), &ctx);
        ui.heading("Windows & rules");
        Self::section(ui, "Open windows");
        match self.link.get("windows") {
            Some(Ok(list)) => {
                egui::ScrollArea::vertical().id_salt("winlist").max_height(260.0).show(ui, |ui| {
                    egui::Grid::new("windows").num_columns(5).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                        for h in ["", "Decision", "Process", "Class", "Title"] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for w in list.as_array().into_iter().flatten() {
                            let hwnd = w["hwnd"].as_u64().unwrap_or(0);
                            let process = w["process"].as_str().unwrap_or("").to_string();
                            ui.horizontal(|ui| {
                                if ui.small_button("Why?").clicked() {
                                    self.explain = Some(hwnd);
                                    self.link.request("explain", Request::Explain { hwnd }, &ctx);
                                }
                                if ui.small_button("Add rule").on_hover_text("Create a rule for this program (floats it; adjust below)").clicked() && !process.is_empty() {
                                    let mut t = toml_edit::Table::new();
                                    t.insert("name", toml_edit::value(format!("{process} rule")));
                                    let mut m = toml_edit::InlineTable::new();
                                    m.insert("process", regex::escape(&process).into());
                                    t.insert("match", toml_edit::value(m));
                                    t.insert("float", toml_edit::value(true));
                                    doc::add_rule(&mut self.doc, t);
                                    self.open_rule = Some(doc::rule_count(&self.doc) - 1);
                                    self.changed();
                                }
                            });
                            let dec = w["decision"].as_str().unwrap_or("?");
                            let ws = w["workspace"].as_u64().map(|n| format!(" (ws {n})")).unwrap_or_default();
                            ui.label(format!("{dec}{ws}"));
                            ui.label(&process);
                            let class: String = w["class"].as_str().unwrap_or("").chars().take(32).collect();
                            ui.label(class);
                            let title: String = w["title"].as_str().unwrap_or("").chars().take(48).collect();
                            ui.label(title);
                            ui.end_row();
                        }
                    });
                });
            }
            Some(Err(e)) => {
                ui.colored_label(WARN, format!("The window list needs the running manager ({e})."));
            }
            None => {
                ui.label("Loading…");
            }
        }
        if let Some(hwnd) = self.explain {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Why window {hwnd:#x} is handled this way")).strong());
                    if ui.small_button("Close").clicked() {
                        self.explain = None;
                    }
                });
                match self.link.get("explain") {
                    Some(Ok(v)) => {
                        let cl = &v["classification"];
                        ui.label(format!("Decision: {}", cl["decision"].as_str().unwrap_or("?")));
                        for r in cl["reasons"].as_array().into_iter().flatten() {
                            ui.label(format!("• {}", r.as_str().unwrap_or("")));
                        }
                        let rules: Vec<&str> = cl["matched_rules"].as_array().into_iter().flatten().filter_map(|r| r.as_str()).collect();
                        ui.label(if rules.is_empty() { "No rules match.".to_string() } else { format!("Matching rules: {}", rules.join(", ")) });
                    }
                    Some(Err(e)) => {
                        ui.colored_label(ERR, e);
                    }
                    None => {
                        ui.label("…");
                    }
                }
            });
        }

        Self::section(ui, "Rules");
        ui.label(RichText::new("Patterns are regular expressions that must match the whole value; process names ignore case. Rules apply in order and later rules win.").weak());
        let n = doc::rule_count(&self.doc);
        let mut action: Option<(usize, i32)> = None;
        for i in 0..n {
            let Some(t) = doc::rules(&self.doc).and_then(|a| a.get(i)).cloned() else { continue };
            let s = |k: &str| t.get(k).and_then(|i| i.as_str()).map(str::to_string);
            let m = |k: &str| t.get("match").and_then(|i| i.as_table_like()).and_then(|m| m.get(k)).and_then(|i| i.as_str()).map(str::to_string);
            let b = |k: &str| t.get(k).and_then(|i| i.as_bool());
            let num = |k: &str| t.get(k).and_then(|i| i.as_integer());
            let name = s("name").unwrap_or_else(|| format!("rule #{}", i + 1));
            let has_err = self.report.diagnostics.iter().any(|d| d.setting.starts_with(&format!("rule[{i}]")));
            let header = if has_err { RichText::new(format!("{name}  (has problems)")).color(ERR) } else { RichText::new(&name) };
            let open = self.open_rule == Some(i);
            let resp = egui::CollapsingHeader::new(header).id_salt(("rule", i)).open(open.then_some(true)).show(ui, |ui| {
                Self::grid(&format!("rule{i}")).show(ui, |ui| {
                    let text_field = |ui: &mut egui::Ui, label: &str, path: &[&str], cur: Option<String>, hint: &str| -> Option<(Vec<String>, Option<String>)> {
                        ui.label(label);
                        let mut v = cur.clone().unwrap_or_default();
                        let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(300.0).min_size(egui::vec2(300.0, 0.0)).font(egui::TextStyle::Monospace).hint_text(hint));
                        ui.end_row();
                        r.changed().then(|| (path.iter().map(|s| s.to_string()).collect(), (!v.is_empty()).then_some(v)))
                    };
                    let mut edits: Vec<(Vec<String>, Option<toml_edit::Value>)> = Vec::new();
                    for (label, path, cur, hint) in [
                        ("Name", vec!["name"], s("name"), "description"),
                        ("Process", vec!["match", "process"], m("process"), "e.g. spotify\\.exe"),
                        ("Window class", vec!["match", "class"], m("class"), "e.g. Chrome_WidgetWin_1"),
                        ("Title", vec!["match", "title"], m("title"), "e.g. (?i).*settings.*"),
                    ] {
                        if let Some((p, v)) = text_field(ui, label, &path, cur, hint) {
                            edits.push((p, v.map(toml_edit::Value::from)));
                        }
                    }
                    for (label, key, yes, no) in [("Manage", "manage", "manage", "ignore completely"), ("Float", "float", "float", "tile"), ("Border", "border", "show", "hide"), ("Center", "center", "center when floating", "don't center")] {
                        ui.label(label);
                        if let Some(v) = tri(ui, (i, key), b(key), yes, no) {
                            edits.push((vec![key.into()], v.map(toml_edit::Value::from)));
                        }
                        ui.end_row();
                    }
                    for (label, key, lo, hi) in [("Workspace", "workspace", 1i64, 99i64), ("Monitor", "monitor", 1, 16)] {
                        ui.label(label);
                        ui.horizontal(|ui| {
                            let cur = num(key);
                            let mut on = cur.is_some();
                            let mut v = cur.unwrap_or(1);
                            let a = ui.checkbox(&mut on, "").changed();
                            let d = ui.add_enabled(on, egui::DragValue::new(&mut v).range(lo..=hi)).changed();
                            if a || d {
                                edits.push((vec![key.into()], on.then(|| toml_edit::Value::from(v))));
                            }
                        });
                        ui.end_row();
                    }
                    for (p, v) in edits {
                        let p: Vec<&str> = p.iter().map(String::as_str).collect();
                        let _ = doc::set_rule(&mut self.doc, i, &p, v);
                        self.changed();
                    }
                });
                self.diag_lines(ui, &format!("rule[{i}]"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(i > 0, egui::Button::new("Move up").small()).clicked() {
                        action = Some((i, -1));
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("Move down").small()).clicked() {
                        action = Some((i, 1));
                    }
                    if ui.small_button("Delete rule").clicked() {
                        action = Some((i, 0));
                    }
                });
            });
            if resp.header_response.clicked() {
                self.open_rule = if open { None } else { Some(i) };
            }
        }
        match action {
            Some((i, -1)) => {
                doc::swap_rules(&mut self.doc, i - 1);
                self.open_rule = Some(i - 1);
                self.changed();
            }
            Some((i, 1)) => {
                doc::swap_rules(&mut self.doc, i);
                self.open_rule = Some(i + 1);
                self.changed();
            }
            Some((i, _)) => {
                doc::remove_rule(&mut self.doc, i);
                self.open_rule = None;
                self.changed();
            }
            None => {}
        }
        if ui.button("Add empty rule").clicked() {
            let mut t = toml_edit::Table::new();
            t.insert("name", toml_edit::value("New rule"));
            t.insert("float", toml_edit::value(true));
            doc::add_rule(&mut self.doc, t);
            self.open_rule = Some(doc::rule_count(&self.doc) - 1);
            self.changed();
        }
    }

    fn page_about(&mut self, ui: &mut egui::Ui) {
        ui.heading("About");
        ui.label(format!("TilingWindowsManager {} — a keyboard-driven tiling window manager for Windows 11, inspired by Hyprland.", env!("CARGO_PKG_VERSION")));
        ui.label("MIT licence. Not affiliated with the Hyprland project; layouts and configuration names follow Hyprland's documented behaviour.");
        Self::section(ui, "Files");
        egui::Grid::new("paths").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
            for (label, p) in [("Configuration", self.path.clone()), ("Themes", paths::themes_dir()), ("Log", paths::log_file()), ("Crash recovery", paths::recovery_file())] {
                ui.label(label);
                ui.label(RichText::new(p.display().to_string()).monospace());
                ui.horizontal(|ui| {
                    if ui.add_enabled(p.exists(), egui::Button::new("Open").small()).clicked() {
                        shell_open(&p);
                    }
                    if let Some(parent) = p.parent() {
                        if ui.add_enabled(parent.exists(), egui::Button::new("Folder").small()).clicked() {
                            shell_open(parent);
                        }
                    }
                });
                ui.end_row();
            }
        });
        Self::section(ui, "Display");
        ui.horizontal(|ui| {
            ui.label("Text size");
            if ui.add(egui::Slider::new(&mut self.zoom, 0.75..=2.0).step_by(0.05).fixed_decimals(2)).changed() {
                ui.ctx().set_zoom_factor(self.zoom);
            }
            ui.label(RichText::new("(Ctrl + plus / minus also works)").weak());
        });
    }

    // ------------------------------------------------------------- top level

    fn bottom_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let errors = self.error_count();
            let warnings = self.report.diagnostics.len() - errors;
            let save_label = if self.confirm_errors { "Save anyway" } else { "Save" };
            let save = ui.add_enabled(self.dirty && self.syntax_error.is_none(), egui::Button::new(RichText::new(save_label).strong())).on_hover_text("Ctrl+S");
            if save.clicked() {
                if errors > 0 && !self.confirm_errors {
                    self.confirm_errors = true;
                    self.flash(format!("{errors} error(s): the manager will use defaults for those settings. Click “Save anyway” to save."), true);
                } else {
                    self.save();
                }
            }
            if ui.add_enabled(self.dirty, egui::Button::new("Revert")).on_hover_text("Discard unsaved changes").clicked() {
                self.load();
            }
            if self.dirty {
                ui.label(RichText::new(if self.exists { "Unsaved changes" } else { "New file — not saved yet" }).color(WARN));
            }
            if errors + warnings > 0 && ui.link(format!("{errors} error(s), {warnings} warning(s)")).clicked() {
                self.page = Page::Overview;
            }
            if let Some((msg, err, at)) = &self.status {
                if at.elapsed() < Duration::from_secs(8) {
                    ui.colored_label(if *err { ERR } else { ui.visuals().text_color() }, msg);
                }
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.check_disk();
        self.link.refresh("version", Request::Version, Duration::from_secs(2), &ctx);
        // Poll the disk and manager even when idle.
        ctx.request_repaint_after(Duration::from_secs(1));
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::S))) && self.dirty && self.syntax_error.is_none() {
            self.save();
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if let Some(Ok(r)) = self.link.take("dispatch") {
            let _ = r;
            self.link.request("state", Request::State, &ctx);
        }

        egui::Panel::left("nav").resizable(false).exact_size(190.0).show(ui, |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("TilingWindowsManager").strong().size(15.0));
            let running = matches!(self.link.get("version"), Some(Ok(_)));
            dot(ui, if running { OK } else { WARN }, if running { "running" } else { "not running" });
            ui.add_space(10.0);
            for (p, name) in PAGES {
                if ui.add_sized([170.0, 28.0], egui::Button::selectable(self.page == *p, *name)).clicked() {
                    self.page = *p;
                }
            }
        });
        egui::Panel::bottom("bar").show(ui, |ui| {
            ui.add_space(4.0);
            self.bottom_bar(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.spacing_mut().slider_width = 240.0;
            if self.external_change {
                ui.horizontal(|ui| {
                    ui.colored_label(WARN, "config.toml was changed by another program.");
                    if ui.button("Load it (discard my changes)").clicked() {
                        self.load();
                    }
                    if ui.button("Keep my changes").clicked() {
                        self.external_change = false;
                        self.disk_stamp = stamp(&self.path);
                    }
                });
                ui.separator();
            }
            if let Some(e) = self.syntax_error.clone() {
                ui.heading("The configuration file cannot be read");
                ui.label(format!("{} is not valid TOML, so it cannot be edited here. The manager keeps using the last working configuration.", self.path.display()));
                ui.add_space(6.0);
                ui.label(RichText::new(e).monospace().color(ERR));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Open in text editor").clicked() {
                        shell_open(&self.path);
                    }
                    if ui.button("Try again").clicked() {
                        self.load();
                    }
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(900.0);
                match self.page {
                    Page::Overview => self.page_overview(ui),
                    Page::Appearance => self.page_appearance(ui),
                    Page::Layout => self.page_layout(ui),
                    Page::Animations => self.page_animations(ui),
                    Page::Binds => self.page_binds(ui),
                    Page::Windows => self.page_windows(ui),
                    Page::About => self.page_about(ui),
                }
                ui.add_space(20.0);
            });
        });

        if self.confirm_close {
            egui::Modal::new(egui::Id::unique("twm-settings-close")).show(&ctx, |ui| {
                ui.label(RichText::new("Save changes to config.toml?").strong());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                        if !self.dirty {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        self.confirm_close = false;
                    }
                    if ui.button("Discard").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_close = false;
                    }
                });
            });
        }
    }
}

/// A small 4-tile icon in the default theme's accent colours.
fn icon() -> egui::IconData {
    let n = 32u32;
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let (tx, ty) = (x % 16, y % 16);
            if (1..15).contains(&tx) && (1..15).contains(&ty) {
                let t = (x + y) as f32 / (2 * n) as f32;
                let (a, b) = ([0x33, 0xcc, 0xff], [0x00, 0xff, 0x99]);
                let i = ((y * n + x) * 4) as usize;
                for c in 0..3 {
                    rgba[i + c] = (a[c] as f32 * (1.0 - t) + b[c] as f32 * t) as u8;
                }
                rgba[i + 3] = 255;
            }
        }
    }
    egui::IconData { rgba, width: n, height: n }
}

/// Bring an already open settings window to the front instead of opening another.
fn already_open() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    unsafe {
        let name = HSTRING::from(format!("Local\\TilingWindowsManager-settings-{}", paths::instance()));
        // Intentionally leaked: held for the life of the process.
        let _m = CreateMutexW(None, false, &name);
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(h) = FindWindowW(None, &HSTRING::from(TITLE)) {
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        true
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.iter().position(|a| a == "--config").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(paths::default_config_file);
    if already_open() {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(TITLE).with_inner_size([1000.0, 720.0]).with_min_inner_size([680.0, 440.0]).with_icon(icon())
            // Test hook (same as the manager's): open without taking focus.
            .with_active(std::env::var_os("TWM_NO_ACTIVATE").is_none()),
        ..Default::default()
    };
    eframe::run_native(TITLE, options, Box::new(move |_cc| Ok(Box::new(App::new(path)))))
}
