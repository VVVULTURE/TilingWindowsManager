//! Configuration: TOML file -> validated `Config` plus diagnostics.
//!
//! Loading never fails. Syntax errors keep the caller's fallback (defaults
//! or the last good configuration); a bad section is replaced by its
//! defaults while the remaining sections still load; bad individual values
//! (out-of-range numbers, unknown colours, invalid bindings/rules) are
//! reported and skipped. Every diagnostic names the file and the setting.

use crate::actions::Action;
use crate::color::Gradient;
use crate::keys::Chord;
use crate::layout::dwindle::DwindleOptions;
use crate::layout::master::{MasterOptions, NewStatus, Orientation};
use crate::layout::{LayoutKind, LayoutOptions};
use crate::rules::{ClassifyOptions, Rule, RuleEffects, RuleMatch};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

// ---------------------------------------------------------------- raw file

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct General {
    pub layout: String,
    pub gaps_in: i32,
    pub gaps_out: i32,
    pub border_size: i32,
    /// Manage windows that no rule mentions (false = opt-in via rules).
    pub manage_by_default: bool,
    pub float_unresizable: bool,
    pub float_dialogs: bool,
    /// How windows on inactive workspaces are hidden: "hide" or "minimize".
    pub hide_method: String,
    pub focus_new_windows: bool,
    /// When a window on a hidden workspace activates itself, switch to that
    /// workspace (true) or keep the current one and take focus back (false,
    /// Hyprland's `misc:focus_on_activate` default).
    pub focus_on_activate: bool,
    pub mouse_follows_focus: bool,
    /// Drag a tiled window onto another to swap them.
    pub drag_to_swap: bool,
    /// On quit, put windows that existed before the manager started back
    /// where they were (including maximized state).
    pub restore_on_exit: bool,
}

impl Default for General {
    fn default() -> Self {
        General {
            layout: "dwindle".into(),
            gaps_in: 5,
            gaps_out: 12,
            border_size: 2,
            manage_by_default: true,
            float_unresizable: true,
            float_dialogs: true,
            hide_method: "hide".into(),
            focus_new_windows: true,
            focus_on_activate: false,
            mouse_follows_focus: false,
            drag_to_swap: true,
            restore_on_exit: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Decoration {
    /// "overlay" (drawn ring, gradients), "dwm" (native 1px Windows 11
    /// border recoloured), or "none".
    pub border_style: String,
    pub active_border: String,
    pub inactive_border: String,
    /// Corner radius of the overlay ring (logical px). Windows 11 rounds
    /// window corners at 8 px.
    pub rounding: i32,
    /// Windows 11 corner preference applied to managed windows:
    /// "default", "round", "small", "square".
    pub corner_preference: String,
    /// Draw borders around inactive windows too.
    pub inactive_borders: bool,
}

impl Default for Decoration {
    fn default() -> Self {
        Decoration {
            border_style: "overlay".into(),
            active_border: "rgba(33ccffee) rgba(00ff99ee) 45deg".into(),
            inactive_border: "rgba(595959aa)".into(),
            rounding: 8,
            corner_preference: "default".into(),
            inactive_borders: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AnimSpec {
    pub enabled: bool,
    /// Duration in milliseconds.
    pub duration: u32,
    pub curve: String,
    /// Workspace switch style: "slide", "slidevert" or "none".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

impl AnimSpec {
    fn new(duration: u32, curve: &str, style: Option<&str>) -> Self {
        AnimSpec { enabled: true, duration, curve: curve.into(), style: style.map(Into::into) }
    }
}

impl Default for AnimSpec {
    fn default() -> Self {
        AnimSpec::new(200, "smooth", None)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Animations {
    pub enabled: bool,
    /// Named cubic Bézier curves: name = [x1, y1, x2, y2].
    pub bezier: BTreeMap<String, [f32; 4]>,
    pub windows: AnimSpec,
    pub workspaces: AnimSpec,
    pub border: AnimSpec,
}

impl Default for Animations {
    fn default() -> Self {
        let mut bezier = BTreeMap::new();
        bezier.insert("smooth".into(), [0.05, 0.9, 0.1, 1.0]);
        bezier.insert("linear".into(), [0.0, 0.0, 1.0, 1.0]);
        bezier.insert("easeOutQuint".into(), [0.23, 1.0, 0.32, 1.0]);
        bezier.insert("overshot".into(), [0.05, 0.9, 0.1, 1.05]);
        Animations {
            enabled: true,
            bezier,
            windows: AnimSpec::new(180, "smooth", None),
            workspaces: AnimSpec::new(220, "smooth", Some("slide")),
            border: AnimSpec::new(150, "linear", None),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct DwindleCfg {
    pub split_width_multiplier: f32,
    pub default_split_ratio: f32,
    pub preserve_split: bool,
    pub force_split: u8,
}

impl Default for DwindleCfg {
    fn default() -> Self {
        let d = DwindleOptions::default();
        DwindleCfg { split_width_multiplier: d.split_width_multiplier, default_split_ratio: d.default_split_ratio, preserve_split: d.preserve_split, force_split: d.force_split }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct MasterCfg {
    pub mfact: f32,
    pub orientation: String,
    pub new_status: String,
    pub new_on_top: bool,
}

impl Default for MasterCfg {
    fn default() -> Self {
        MasterCfg { mfact: 0.55, orientation: "left".into(), new_status: "slave".into(), new_on_top: false }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Input {
    /// "hook" (low-level keyboard hook; allows Win-key bindings) or
    /// "hotkey" (RegisterHotKey; cannot use Win combos owned by the shell).
    pub keyboard_backend: String,
}

impl Default for Input {
    fn default() -> Self {
        Input { keyboard_backend: "hook".into() }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RawRule {
    pub name: Option<String>,
    #[serde(rename = "match")]
    pub matcher: RuleMatch,
    pub manage: Option<bool>,
    pub float: Option<bool>,
    pub workspace: Option<u32>,
    pub monitor: Option<usize>,
    pub border: Option<bool>,
    pub size: Option<[i32; 2]>,
    pub center: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkspaceRule {
    pub id: u32,
    /// 1-based monitor number (ordered left-to-right, then top-to-bottom).
    pub monitor: Option<usize>,
    pub layout: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Reserved {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MonitorRule {
    /// 1-based monitor number (primary first, then left-to-right).
    pub index: usize,
    /// Space kept free at the monitor edges (logical px), e.g. for a status bar.
    pub reserved: Reserved,
}

// ---------------------------------------------------------------- compiled

#[derive(Clone, Debug)]
pub struct Bind {
    pub chord: Chord,
    pub action: Action,
    /// The text as written in the config, for display.
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub theme: Option<String>,
    pub variables: BTreeMap<String, String>,
    pub general: General,
    pub decoration: Decoration,
    pub active_border: Gradient,
    pub inactive_border: Gradient,
    pub animations: Animations,
    pub dwindle: DwindleCfg,
    pub master: MasterCfg,
    pub input: Input,
    pub binds: Vec<Bind>,
    pub rules: Vec<Rule>,
    pub workspaces: Vec<WorkspaceRule>,
    pub monitors: Vec<MonitorRule>,
    pub default_layout: LayoutKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub file: String,
    /// 1-based line, when known.
    pub line: Option<usize>,
    /// Dotted setting path, e.g. `general.gaps_in`.
    pub setting: String,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match self.line {
            Some(l) => write!(f, "{sev}: {}:{l}: {}: {}", self.file, self.setting, self.message),
            None => write!(f, "{sev}: {}: {}: {}", self.file, self.setting, self.message),
        }
    }
}

/// Result of loading a configuration.
#[derive(Debug)]
pub struct LoadReport {
    pub config: Config,
    pub diagnostics: Vec<Diagnostic>,
    /// True when the file could not be parsed at all and `config` is the fallback.
    pub used_fallback: bool,
}

impl LoadReport {
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error)
    }
    pub fn has_errors(&self) -> bool {
        self.errors().next().is_some()
    }
}

pub const DEFAULT_CONFIG: &str = include_str!("../../../config/default.toml");

/// Built-in themes, also shipped as files in `themes/`.
pub const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("hypr", include_str!("../../../config/themes/hypr.toml")),
    ("catppuccin-mocha", include_str!("../../../config/themes/catppuccin-mocha.toml")),
    ("nord", include_str!("../../../config/themes/nord.toml")),
    ("gruvbox", include_str!("../../../config/themes/gruvbox.toml")),
    ("rose-pine", include_str!("../../../config/themes/rose-pine.toml")),
    ("minimal", include_str!("../../../config/themes/minimal.toml")),
];

impl Default for Config {
    fn default() -> Self {
        // The shipped default file must always load cleanly (unit-tested).
        load_str(DEFAULT_CONFIG, "<built-in default>", None, &|_| None).config
    }
}

impl Config {
    pub fn layout_options(&self) -> LayoutOptions {
        LayoutOptions {
            dwindle: DwindleOptions {
                split_width_multiplier: self.dwindle.split_width_multiplier,
                default_split_ratio: self.dwindle.default_split_ratio,
                preserve_split: self.dwindle.preserve_split,
                force_split: self.dwindle.force_split,
            },
            master: MasterOptions {
                new_status: match self.master.new_status.as_str() {
                    "master" => NewStatus::Master,
                    "inherit" => NewStatus::Inherit,
                    _ => NewStatus::Slave,
                },
                new_on_top: self.master.new_on_top,
            },
            mfact: self.master.mfact,
            orientation: Orientation::parse(&self.master.orientation).unwrap_or_default(),
        }
    }

    pub fn classify_options(&self) -> ClassifyOptions {
        ClassifyOptions {
            manage_by_default: self.general.manage_by_default,
            float_unresizable: self.general.float_unresizable,
            float_dialogs: self.general.float_dialogs,
        }
    }

    pub fn bezier(&self, name: &str) -> [f32; 4] {
        self.animations.bezier.get(name).copied().unwrap_or([0.0, 0.0, 1.0, 1.0])
    }
}

// ---------------------------------------------------------------- loading

struct Ctx<'a> {
    file: &'a str,
    src: &'a str,
    diags: Vec<Diagnostic>,
}

impl Ctx<'_> {
    fn push(&mut self, severity: Severity, setting: &str, line: Option<usize>, message: impl Into<String>) {
        let line = line.or_else(|| find_key_line(self.src, setting));
        self.diags.push(Diagnostic { severity, file: self.file.to_string(), line, setting: setting.to_string(), message: message.into() });
    }
    fn err(&mut self, setting: &str, msg: impl Into<String>) {
        self.push(Severity::Error, setting, None, msg)
    }
    fn warn(&mut self, setting: &str, msg: impl Into<String>) {
        self.push(Severity::Warning, setting, None, msg)
    }
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte.min(src.len())].matches('\n').count() + 1
}

/// Best-effort line lookup for a dotted setting path: finds the section
/// header and then the key below it.
fn find_key_line(src: &str, setting: &str) -> Option<usize> {
    let mut parts: Vec<&str> = setting.split('.').collect();
    let key = parts.pop()?.trim_matches('"');
    let key = key.split('[').next().unwrap_or(key);
    let section = parts.join(".");
    let section = section.split('[').next().unwrap_or("");
    let mut in_section = section.is_empty();
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            let name = t.trim_matches(|c| c == '[' || c == ']').trim();
            in_section = name == section;
            if in_section && key.is_empty() {
                return Some(i + 1);
            }
            continue;
        }
        if in_section {
            let k = t.split('=').next().unwrap_or("").trim().trim_matches('"');
            if !k.is_empty() && k == key {
                return Some(i + 1);
            }
        }
    }
    None
}

fn section<T: DeserializeOwned + Default>(ctx: &mut Ctx, table: &toml::Table, name: &str) -> T {
    let Some(v) = table.get(name) else { return T::default() };
    let mut unknown = Vec::new();
    let r: Result<T, _> = serde_ignored::deserialize(v.clone(), |p| unknown.push(p.to_string()));
    match r {
        Ok(t) => {
            for u in unknown {
                ctx.warn(&format!("{name}.{u}"), "unknown option (ignored)");
            }
            t
        }
        Err(e) => {
            let msg = e.to_string();
            ctx.err(name, format!("{} — using defaults for [{name}]", msg.trim()));
            T::default()
        }
    }
}

fn check_range<T: PartialOrd + Copy + fmt::Display>(ctx: &mut Ctx, setting: &str, v: &mut T, lo: T, hi: T, def: T) {
    if *v < lo || *v > hi {
        ctx.err(setting, format!("{} is out of range {lo}..={hi}; using {def}", *v));
        *v = def;
    }
}

fn check_choice(ctx: &mut Ctx, setting: &str, v: &mut String, choices: &[&str], def: &str) {
    if !choices.contains(&v.as_str()) {
        ctx.err(setting, format!("`{v}` is not one of {}; using `{def}`", choices.join(", ")));
        *v = def.to_string();
    }
}

/// Theme lookup for [`load_str`] that reads `<dir>/<name>.toml`.
pub fn theme_file_lookup(dir: &std::path::Path, name: &str) -> Option<(String, String)> {
    let p = dir.join(format!("{name}.toml"));
    std::fs::read_to_string(&p).ok().map(|t| (t, p.display().to_string()))
}

/// Parses `src` as a configuration. `theme_lookup` resolves a theme name to
/// its TOML text and a file name for diagnostics (user theme directory
/// first, then built-ins).
pub fn load_str(src: &str, file: &str, fallback: Option<&Config>, theme_lookup: &dyn Fn(&str) -> Option<(String, String)>) -> LoadReport {
    let mut ctx = Ctx { file, src, diags: Vec::new() };
    let table: toml::Table = match src.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map(|s| line_of(src, s.start));
            ctx.push(Severity::Error, "<syntax>", line, format!("{} — configuration not applied", e.message().trim()));
            let config = match fallback {
                Some(c) => c.clone(),
                None if src == DEFAULT_CONFIG => panic!("built-in default config has a syntax error"),
                None => Config::default(),
            };
            return LoadReport { config, diagnostics: ctx.diags, used_fallback: true };
        }
    };

    const KNOWN: &[&str] = &["theme", "variables", "general", "decoration", "animations", "dwindle", "master", "input", "binds", "rule", "workspace", "monitor"];
    for k in table.keys() {
        if !KNOWN.contains(&k.as_str()) {
            ctx.warn(k, format!("unknown section or option (ignored); known: {}", KNOWN.join(", ")));
        }
    }

    // Theme: applied first, then the user's [decoration] overrides it.
    let theme = match table.get("theme") {
        None => None,
        Some(toml::Value::String(s)) => Some(s.clone()),
        Some(_) => {
            ctx.err("theme", "must be a string (theme name)");
            None
        }
    };
    let mut theme_table = toml::Table::new();
    let mut theme_src: Option<(String, String)> = None;
    if let Some(name) = &theme {
        let builtin = || BUILTIN_THEMES.iter().find(|(n, _)| n == name).map(|(_, s)| (s.to_string(), format!("<built-in theme {name}>")));
        match theme_lookup(name).or_else(builtin) {
            Some((text, origin)) => match text.parse::<toml::Table>() {
                Ok(t) => {
                    theme_table = t;
                    theme_src = Some((text, origin));
                }
                Err(e) => {
                    let line = e.span().map(|s| line_of(&text, s.start));
                    ctx.diags.push(Diagnostic {
                        severity: Severity::Error,
                        file: origin,
                        line,
                        setting: "<syntax>".into(),
                        message: format!("theme `{name}` has a syntax error: {} — theme not applied", e.message().trim()),
                    });
                }
            },
            None => {
                let names: Vec<&str> = BUILTIN_THEMES.iter().map(|(n, _)| *n).collect();
                ctx.err("theme", format!("theme `{name}` not found (built-in: {})", names.join(", ")));
            }
        }
    }
    let mut deco_table = toml::Table::new();
    // Decoration keys whose value comes from the theme file (not overridden).
    let mut from_theme: Vec<String> = Vec::new();
    if let Some(toml::Value::Table(t)) = theme_table.get("decoration") {
        deco_table.extend(t.clone());
        let user = table.get("decoration").and_then(|v| v.as_table());
        from_theme = t.keys().filter(|k| !user.is_some_and(|u| u.contains_key(*k))).cloned().collect();
    }
    if let Some(toml::Value::Table(t)) = table.get("decoration") {
        deco_table.extend(t.clone());
    } else if table.contains_key("decoration") {
        ctx.err("decoration", "must be a table");
    }
    let mut merged = toml::Table::new();
    merged.insert("decoration".into(), toml::Value::Table(deco_table));

    let variables: BTreeMap<String, String> = section(&mut ctx, &table, "variables");
    let mut general: General = section(&mut ctx, &table, "general");
    let mut decoration: Decoration = section(&mut ctx, &merged, "decoration");
    let mut animations: Animations = section(&mut ctx, &table, "animations");
    let mut dwindle: DwindleCfg = section(&mut ctx, &table, "dwindle");
    let mut master: MasterCfg = section(&mut ctx, &table, "master");
    let mut input: Input = section(&mut ctx, &table, "input");

    // ---- value validation
    check_choice(&mut ctx, "general.layout", &mut general.layout, &["dwindle", "master", "monocle"], "dwindle");
    check_range(&mut ctx, "general.gaps_in", &mut general.gaps_in, 0, 200, 5);
    check_range(&mut ctx, "general.gaps_out", &mut general.gaps_out, 0, 400, 12);
    check_range(&mut ctx, "general.border_size", &mut general.border_size, 0, 20, 2);
    check_choice(&mut ctx, "general.hide_method", &mut general.hide_method, &["hide", "minimize"], "hide");
    check_choice(&mut ctx, "decoration.border_style", &mut decoration.border_style, &["overlay", "dwm", "none"], "overlay");
    check_choice(&mut ctx, "decoration.corner_preference", &mut decoration.corner_preference, &["default", "round", "small", "square"], "default");
    check_range(&mut ctx, "decoration.rounding", &mut decoration.rounding, 0, 50, 8);
    check_range(&mut ctx, "dwindle.split_width_multiplier", &mut dwindle.split_width_multiplier, 0.1, 3.0, 1.0);
    check_range(&mut ctx, "dwindle.default_split_ratio", &mut dwindle.default_split_ratio, 0.1, 1.9, 1.0);
    check_range(&mut ctx, "dwindle.force_split", &mut dwindle.force_split, 0, 2, 0);
    check_range(&mut ctx, "master.mfact", &mut master.mfact, 0.05, 0.95, 0.55);
    check_choice(&mut ctx, "master.orientation", &mut master.orientation, &["left", "right", "top", "bottom", "center"], "left");
    check_choice(&mut ctx, "master.new_status", &mut master.new_status, &["master", "slave", "inherit"], "slave");
    check_choice(&mut ctx, "input.keyboard_backend", &mut input.keyboard_backend, &["hook", "hotkey"], "hook");

    let default_anim = Animations::default();
    for (k, v) in animations.bezier.iter_mut() {
        if !(0.0..=1.0).contains(&v[0]) || !(0.0..=1.0).contains(&v[2]) {
            ctx.err(&format!("animations.bezier.{k}"), "x1 and x2 must be within 0..=1 (y values may overshoot)");
            *v = [0.0, 0.0, 1.0, 1.0];
        }
    }
    for (name, def) in [("smooth", [0.05, 0.9, 0.1, 1.0]), ("linear", [0.0, 0.0, 1.0, 1.0])] {
        animations.bezier.entry(name.into()).or_insert(def);
    }
    let curves: Vec<String> = animations.bezier.keys().cloned().collect();
    for (key, spec, def) in [
        ("windows", &mut animations.windows, &default_anim.windows),
        ("workspaces", &mut animations.workspaces, &default_anim.workspaces),
        ("border", &mut animations.border, &default_anim.border),
    ] {
        check_range(&mut ctx, &format!("animations.{key}.duration"), &mut spec.duration, 0, 2000, def.duration);
        if !curves.contains(&spec.curve) {
            ctx.err(&format!("animations.{key}.curve"), format!("curve `{}` is not defined under [animations.bezier]; using `smooth`", spec.curve));
            spec.curve = "smooth".into();
        }
        if key == "workspaces" {
            let mut style = spec.style.clone().unwrap_or_else(|| "slide".into());
            check_choice(&mut ctx, "animations.workspaces.style", &mut style, &["slide", "slidevert", "none"], "slide");
            spec.style = Some(style);
        } else if spec.style.is_some() {
            ctx.warn(&format!("animations.{key}.style"), "only animations.workspaces has a style (ignored)");
        }
    }

    let parse_grad = |ctx: &mut Ctx, key: &str, s: &str, def: &str| match Gradient::parse(s) {
        Ok(g) => g,
        Err(e) => {
            ctx.err(&format!("decoration.{key}"), format!("{e}; using default"));
            Gradient::parse(def).expect("default gradient")
        }
    };
    let dd = Decoration::default();
    let active_border = parse_grad(&mut ctx, "active_border", &decoration.active_border, &dd.active_border);
    let inactive_border = parse_grad(&mut ctx, "inactive_border", &decoration.inactive_border, &dd.inactive_border);

    // ---- variables, binds
    let mut vars = variables.clone();
    vars.entry("mod".into()).or_insert_with(|| "win".into());
    let lookup = |n: &str| vars.get(n).cloned();
    let mut binds = Vec::new();
    let raw_binds: BTreeMap<String, String> = match table.get("binds") {
        None => BTreeMap::new(),
        Some(toml::Value::Table(t)) => {
            let mut m = BTreeMap::new();
            for (k, v) in t {
                match v {
                    toml::Value::String(s) => {
                        m.insert(k.clone(), s.clone());
                    }
                    _ => ctx.err(&format!("binds.\"{k}\""), "action must be a string, e.g. \"focus left\""),
                }
            }
            m
        }
        Some(_) => {
            ctx.err("binds", "must be a table of \"chord\" = \"action\"");
            BTreeMap::new()
        }
    };
    let mut seen: BTreeMap<Chord, String> = BTreeMap::new();
    for (k, v) in &raw_binds {
        let setting = format!("binds.\"{k}\"");
        let line = src.lines().position(|l| l.trim_start().starts_with(&format!("\"{k}\""))).map(|i| i + 1);
        let chord = match Chord::parse(k, &lookup) {
            Ok(c) => c,
            Err(e) => {
                ctx.push(Severity::Error, &setting, line, format!("{e} — binding skipped"));
                continue;
            }
        };
        let action = match crate::keys::expand_vars(v, &lookup).and_then(|v| Action::parse(&v)) {
            Ok(a) => a,
            Err(e) => {
                ctx.push(Severity::Error, &setting, line, format!("{e} — binding skipped"));
                continue;
            }
        };
        if let Some(prev) = seen.get(&chord) {
            ctx.push(Severity::Error, &setting, line, format!("chord {chord} is already bound by \"{prev}\" — binding skipped"));
            continue;
        }
        seen.insert(chord, k.clone());
        binds.push(Bind { chord, action, source: k.clone() });
    }
    if binds.is_empty() {
        ctx.warn("binds", "no key bindings are configured; the manager can only be controlled from the tray and twmctl");
    }

    // ---- rules
    let mut rules = Vec::new();
    match table.get("rule") {
        None => {}
        Some(toml::Value::Array(arr)) => {
            for (i, v) in arr.iter().enumerate() {
                let setting = format!("rule[{i}]");
                let mut unknown = Vec::new();
                let raw: Result<RawRule, _> = serde_ignored::deserialize(v.clone(), |p| unknown.push(p.to_string()));
                let raw = match raw {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.err(&setting, format!("{} — rule skipped", e.to_string().trim()));
                        continue;
                    }
                };
                for u in unknown {
                    ctx.warn(&format!("{setting}.{u}"), "unknown option (ignored)");
                }
                let name = raw.name.clone().unwrap_or_else(|| format!("rule #{}", i + 1));
                if let Some(w) = raw.workspace {
                    if !(1..=99).contains(&w) {
                        ctx.err(&format!("{setting}.workspace"), "must be 1-99 — rule skipped");
                        continue;
                    }
                }
                let effects = RuleEffects {
                    manage: raw.manage,
                    float: raw.float,
                    workspace: raw.workspace,
                    monitor: raw.monitor,
                    border: raw.border,
                    size: raw.size.map(|[w, h]| (w.max(50), h.max(50))),
                    center: raw.center,
                };
                match Rule::compile(name, &raw.matcher, effects) {
                    Ok(r) => rules.push(r),
                    Err(e) => ctx.err(&setting, format!("{e} — rule skipped")),
                }
            }
        }
        Some(_) => ctx.err("rule", "use [[rule]] array-of-tables syntax"),
    }

    // ---- workspace rules
    let mut workspaces: Vec<WorkspaceRule> = Vec::new();
    match table.get("workspace") {
        None => {}
        Some(toml::Value::Array(arr)) => {
            for (i, v) in arr.iter().enumerate() {
                let setting = format!("workspace[{i}]");
                match v.clone().try_into::<WorkspaceRule>() {
                    Ok(w) if (1..=99).contains(&w.id) => {
                        if let Some(l) = &w.layout {
                            if LayoutKind::parse(l).is_none() {
                                ctx.err(&format!("{setting}.layout"), format!("unknown layout `{l}` — workspace rule skipped"));
                                continue;
                            }
                        }
                        workspaces.push(w)
                    }
                    Ok(_) => ctx.err(&format!("{setting}.id"), "id must be 1-99 — workspace rule skipped"),
                    Err(e) => ctx.err(&setting, format!("{} — workspace rule skipped", e.to_string().trim())),
                }
            }
        }
        Some(_) => ctx.err("workspace", "use [[workspace]] array-of-tables syntax"),
    }

    // ---- monitor rules
    let mut monitors: Vec<MonitorRule> = Vec::new();
    match table.get("monitor") {
        None => {}
        Some(toml::Value::Array(arr)) => {
            for (i, v) in arr.iter().enumerate() {
                let setting = format!("monitor[{i}]");
                match v.clone().try_into::<MonitorRule>() {
                    Ok(m) if m.index >= 1 => {
                        let r = m.reserved;
                        if [r.top, r.bottom, r.left, r.right].iter().any(|x| !(0..=4000).contains(x)) {
                            ctx.err(&format!("{setting}.reserved"), "values must be 0-4000 — monitor rule skipped");
                            continue;
                        }
                        monitors.push(m)
                    }
                    Ok(_) => ctx.err(&format!("{setting}.index"), "index must be 1 or more — monitor rule skipped"),
                    Err(e) => ctx.err(&setting, format!("{} — monitor rule skipped", e.to_string().trim())),
                }
            }
        }
        Some(_) => ctx.err("monitor", "use [[monitor]] array-of-tables syntax"),
    }

    let default_layout = LayoutKind::parse(&general.layout).unwrap_or(LayoutKind::Dwindle);
    let config = Config {
        theme,
        variables,
        general,
        decoration,
        active_border,
        inactive_border,
        animations,
        dwindle,
        master,
        input,
        binds,
        rules,
        workspaces,
        monitors,
        default_layout,
    };
    // Problems with values taken from the theme point at the theme file.
    if let (Some((text, origin)), Some(name)) = (&theme_src, &config.theme) {
        for d in &mut ctx.diags {
            let key = d.setting.strip_prefix("decoration.").map(|k| k.split(['.', '[']).next().unwrap_or(k));
            if key.is_some_and(|k| from_theme.iter().any(|t| t == k)) {
                d.file = origin.clone();
                d.line = find_key_line(text, &d.setting);
                d.message = format!("{} (from theme `{name}`)", d.message);
            }
        }
    }
    LoadReport { config, diagnostics: ctx.diags, used_fallback: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(s: &str) -> LoadReport {
        load_str(s, "test.toml", None, &|_| None)
    }

    #[test]
    fn default_config_loads_cleanly() {
        let r = load_str(DEFAULT_CONFIG, "default.toml", None, &|_| None);
        assert!(r.diagnostics.is_empty(), "{:#?}", r.diagnostics);
        assert!(r.config.binds.len() >= 30);
        assert_eq!(r.config.default_layout, LayoutKind::Dwindle);
    }

    #[test]
    fn all_builtin_themes_load() {
        for (name, _) in BUILTIN_THEMES {
            let r = load(&format!("theme = \"{name}\"\n[binds]\n\"win+t\" = \"retile\""));
            assert!(r.diagnostics.is_empty(), "{name}: {:#?}", r.diagnostics);
        }
    }

    #[test]
    fn empty_file_gives_defaults_with_warning() {
        let r = load("");
        assert!(!r.has_errors());
        assert_eq!(r.config.general.gaps_in, 5);
        assert!(r.diagnostics.iter().any(|d| d.setting == "binds"));
    }

    #[test]
    fn syntax_error_uses_fallback_and_reports_line() {
        let mut good = Config::default();
        good.general.gaps_in = 42;
        let r = load_str("[general]\ngaps_in = 3\ngaps_out = = 4\n", "user.toml", Some(&good), &|_| None);
        assert!(r.used_fallback);
        assert_eq!(r.config.general.gaps_in, 42, "kept last good config");
        let d = r.errors().next().unwrap();
        assert_eq!(d.line, Some(3));
        assert_eq!(d.file, "user.toml");
    }

    #[test]
    fn bad_value_in_one_section_keeps_others() {
        let r = load("[general]\ngaps_in = \"wide\"\n[dwindle]\npreserve_split = true\n");
        assert!(r.has_errors());
        assert_eq!(r.config.general.gaps_in, 5, "section fell back to defaults");
        assert!(r.config.dwindle.preserve_split, "other section loaded");
        let d = r.errors().next().unwrap();
        assert_eq!(d.setting, "general");
    }

    #[test]
    fn unknown_keys_warn_but_keep_rest_of_section() {
        let r = load("[general]\ngapz_in = 3\ngaps_out = 30\n[bogus]\nx=1\n");
        assert!(!r.has_errors(), "{:#?}", r.diagnostics);
        assert_eq!(r.config.general.gaps_out, 30);
        let w: Vec<_> = r.diagnostics.iter().map(|d| d.setting.as_str()).collect();
        assert!(w.contains(&"general.gapz_in"), "{w:?}");
        assert!(w.contains(&"bogus"));
        let d = r.diagnostics.iter().find(|d| d.setting == "general.gapz_in").unwrap();
        assert_eq!(d.line, Some(2));
    }

    #[test]
    fn out_of_range_and_bad_choices() {
        let r = load("[general]\ngaps_in = 9999\nlayout = \"spiral\"\n[decoration]\nactive_border = \"blue\"\n");
        let settings: Vec<_> = r.errors().map(|d| d.setting.clone()).collect();
        assert_eq!(settings, vec!["general.layout", "general.gaps_in", "decoration.active_border"]);
        assert_eq!(r.config.general.gaps_in, 5);
        assert_eq!(r.config.default_layout, LayoutKind::Dwindle);
    }

    #[test]
    fn binds_with_variables_and_conflicts() {
        let r = load("[variables]\nmod = \"alt\"\n[binds]\n\"$mod+h\" = \"focus left\"\n\"alt+h\" = \"focus right\"\n\"$mod+x\" = \"explode\"\n\"$nope+y\" = \"close\"\n");
        assert_eq!(r.config.binds.len(), 1);
        assert!(r.config.binds[0].chord.mods.alt);
        let errs: Vec<_> = r.errors().map(|d| d.message.clone()).collect();
        assert_eq!(errs.len(), 3, "{errs:#?}");
        assert!(errs.iter().any(|e| e.contains("already bound")));
        assert!(errs.iter().any(|e| e.contains("unknown action")));
        assert!(errs.iter().any(|e| e.contains("undefined variable")));
    }

    #[test]
    fn rules_compile_and_bad_rules_are_skipped() {
        let r = load(
            "[[rule]]\nname = \"calc\"\nmatch = { process = \"CalculatorApp\\\\.exe\" }\nfloat = true\n\
             [[rule]]\nmatch = { title = \"(\" }\nfloat = true\n\
             [[rule]]\nmatch = { class = \"x\" }\nworkspace = 300\n\
             [[rule]]\nmatch = { class = \"y\" }\nflaot = true\n",
        );
        assert_eq!(r.config.rules.len(), 2, "{:#?}", r.diagnostics);
        assert_eq!(r.config.rules[0].name, "calc");
        assert_eq!(r.errors().count(), 2);
        assert!(r.diagnostics.iter().any(|d| d.setting == "rule[3].flaot" && d.severity == Severity::Warning));
    }

    #[test]
    fn theme_errors_name_the_theme_file() {
        let lookup = |_: &str| Some(("[decoration]\nrounding = 4\nactive_border = \"rgba(nothex)\"\n".to_string(), "themes/bad.toml".to_string()));
        let r = load_str("theme = \"bad\"\n", "config.toml", None, &lookup);
        let d = r.errors().find(|d| d.setting == "decoration.active_border").expect("error reported");
        assert_eq!((d.file.as_str(), d.line), ("themes/bad.toml", Some(3)));
        assert!(d.message.contains("from theme `bad`"));
        // Overridden in the user's file: the error belongs to config.toml.
        let r = load_str("theme = \"bad\"\n[decoration]\nactive_border = \"zz\"\n", "config.toml", None, &lookup);
        let d = r.errors().find(|d| d.setting == "decoration.active_border").unwrap();
        assert_eq!((d.file.as_str(), d.line), ("config.toml", Some(3)));
        let r = load_str("theme = \"bad\"\n", "config.toml", None, &|_| Some(("[decoration\n".into(), "themes/bad.toml".into())));
        assert!(r.errors().any(|d| d.file == "themes/bad.toml" && d.line == Some(1)));
    }

    #[test]
    fn theme_then_user_override() {
        let r = load("theme = \"nord\"\n[decoration]\nrounding = 3\n");
        assert_eq!(r.config.decoration.rounding, 3);
        assert_ne!(r.config.decoration.active_border, Decoration::default().active_border);
        let missing = load("theme = \"nope\"");
        assert!(missing.errors().any(|d| d.setting == "theme"));
    }

    #[test]
    fn monitor_rules() {
        let r = load("[[monitor]]
index = 2
reserved = { top = 40 }
[[monitor]]
index = 0
[[monitor]]
index = 1
reserved = { left = -5 }
");
        assert_eq!(r.config.monitors.len(), 1);
        assert_eq!(r.config.monitors[0].reserved.top, 40);
        assert_eq!(r.errors().count(), 2);
    }

    #[test]
    fn animation_curve_validation() {
        let r = load("[animations.windows]\ncurve = \"bouncy\"\n[animations.bezier]\nbad = [2.0, 0, 1, 1]\n");
        let s: Vec<_> = r.errors().map(|d| d.setting.clone()).collect();
        assert!(s.contains(&"animations.windows.curve".to_string()));
        assert!(s.contains(&"animations.bezier.bad".to_string()));
        assert_eq!(r.config.animations.windows.curve, "smooth");
    }
}
