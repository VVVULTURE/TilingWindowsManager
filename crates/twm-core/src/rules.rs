//! Window classification and per-application rules.
//!
//! The platform layer gathers verified facts about a window (styles, owner,
//! process, class...) into `WindowFacts`; `classify` turns them into a
//! decision with human-readable reasons. Keeping this pure makes the policy
//! testable and lets `twmctl explain <hwnd>` show exactly why a window is or
//! is not tiled.

use crate::geom::Rect;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WindowFacts {
    pub hwnd: u64,
    pub title: String,
    pub class: String,
    /// Executable file name, e.g. `notepad.exe` (empty if unavailable).
    pub process: String,
    pub pid: u32,
    pub visible: bool,
    /// DWM reports the window cloaked (e.g. suspended UWP app, other virtual desktop).
    pub cloaked: bool,
    pub minimized: bool,
    pub maximized: bool,
    /// Has an owner window (dialogs, property sheets, tool palettes).
    pub owned: bool,
    pub child: bool,
    pub tool_window: bool,
    pub no_activate: bool,
    pub has_caption: bool,
    /// WS_THICKFRAME: the user can resize it.
    pub resizable: bool,
    pub popup: bool,
    pub topmost: bool,
    /// Process runs at a higher integrity level (e.g. elevated) so this
    /// unelevated manager cannot move it (UIPI).
    pub elevated: bool,
    /// Window covers its whole monitor without a caption (borderless
    /// fullscreen game/video).
    pub fullscreen: bool,
    pub rect: Rect,
    pub own_process: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    Normal,
    Dialog,
    Tool,
    Popup,
}

impl WindowFacts {
    pub fn kind(&self) -> WindowKind {
        if self.tool_window {
            WindowKind::Tool
        } else if self.owned || (!self.resizable && self.has_caption && self.class == "#32770") {
            WindowKind::Dialog
        } else if self.popup && !self.has_caption {
            WindowKind::Popup
        } else {
            WindowKind::Normal
        }
    }
}

/// Shell and system surfaces that must never be managed. Verified class
/// names observed on Windows 11 build 26200 (see docs/RESEARCH.md).
const SHELL_CLASSES: &[&str] = &[
    "Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow", "XamlExplorerHostIslandWindow_WASDK", "TopLevelWindowForOverflowXamlIsland",
    "NotifyIconOverflowWindow", "MultitaskingViewFrame", "ForegroundStaging", "TaskListThumbnailWnd",
    "TaskManagerWindow_Overlay", "PseudoConsoleWindow", "IME", "MSCTFIME UI",
];

const SHELL_PROCESSES: &[&str] = &[
    "shellexperiencehost.exe", "startmenuexperiencehost.exe", "searchhost.exe", "searchapp.exe",
    "textinputhost.exe", "lockapp.exe", "systemsettingsbroker.exe",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Not managed at all (left exactly as the application placed it).
    Ignore,
    /// Managed (follows workspaces) but keeps its own size/position.
    Float,
    Tile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Classification {
    pub decision: Decision,
    pub workspace: Option<u32>,
    pub monitor: Option<usize>,
    pub border: bool,
    pub float_size: Option<(i32, i32)>,
    pub center: bool,
    pub reasons: Vec<String>,
    pub matched_rules: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuleMatch {
    pub process: Option<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub kind: Option<WindowKind>,
    pub resizable: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuleEffects {
    /// false = ignore completely; true = manage even if heuristics say ignore.
    pub manage: Option<bool>,
    pub float: Option<bool>,
    pub workspace: Option<u32>,
    pub monitor: Option<usize>,
    pub border: Option<bool>,
    pub size: Option<(i32, i32)>,
    pub center: Option<bool>,
}

/// A compiled rule. Patterns are full-match regular expressions
/// (`^(?:pattern)$`); process names are matched case-insensitively.
#[derive(Clone, Debug)]
pub struct Rule {
    pub name: String,
    pub process: Option<Regex>,
    pub class: Option<Regex>,
    pub title: Option<Regex>,
    pub kind: Option<WindowKind>,
    pub resizable: Option<bool>,
    pub effects: RuleEffects,
}

impl Rule {
    pub fn compile(name: String, m: &RuleMatch, effects: RuleEffects) -> Result<Rule, String> {
        let re = |field: &str, p: &Option<String>, ci: bool| -> Result<Option<Regex>, String> {
            p.as_ref()
                .map(|p| {
                    let flags = if ci { "(?i)" } else { "" };
                    Regex::new(&format!("{flags}^(?:{p})$")).map_err(|e| format!("match.{field}: invalid regular expression `{p}`: {e}"))
                })
                .transpose()
        };
        let r = Rule {
            name,
            process: re("process", &m.process, true)?,
            class: re("class", &m.class, false)?,
            title: re("title", &m.title, false)?,
            kind: m.kind,
            resizable: m.resizable,
            effects,
        };
        if r.process.is_none() && r.class.is_none() && r.title.is_none() && r.kind.is_none() && r.resizable.is_none() {
            return Err("rule has an empty `match`; it would apply to every window".into());
        }
        Ok(r)
    }

    pub fn matches(&self, f: &WindowFacts) -> bool {
        self.process.as_ref().is_none_or(|r| r.is_match(&f.process))
            && self.class.as_ref().is_none_or(|r| r.is_match(&f.class))
            && self.title.as_ref().is_none_or(|r| r.is_match(&f.title))
            && self.kind.is_none_or(|k| k == f.kind())
            && self.resizable.is_none_or(|v| v == f.resizable)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClassifyOptions {
    pub manage_by_default: bool,
    pub float_unresizable: bool,
    pub float_dialogs: bool,
}

/// Decides how a window is handled. Hard platform limits (invisible,
/// elevated, our own windows) cannot be overridden by rules; heuristics can.
pub fn classify(f: &WindowFacts, rules: &[Rule], opts: &ClassifyOptions) -> Classification {
    let mut c = Classification {
        decision: Decision::Tile,
        workspace: None,
        monitor: None,
        border: true,
        float_size: None,
        center: false,
        reasons: Vec::new(),
        matched_rules: Vec::new(),
    };
    let hard_ignore = |c: &mut Classification, why: &str| {
        c.decision = Decision::Ignore;
        c.reasons.push(why.to_string());
    };
    if f.own_process {
        hard_ignore(&mut c, "belongs to the window manager itself");
        return c;
    }
    if !f.visible {
        hard_ignore(&mut c, "not visible");
        return c;
    }
    if f.child {
        hard_ignore(&mut c, "child window (WS_CHILD)");
        return c;
    }
    if f.cloaked {
        hard_ignore(&mut c, "cloaked by DWM (suspended app or another virtual desktop)");
        return c;
    }
    if f.elevated {
        hard_ignore(&mut c, "runs elevated; an unelevated process cannot move it (UIPI)");
        return c;
    }
    if SHELL_CLASSES.contains(&f.class.as_str()) || SHELL_PROCESSES.contains(&f.process.to_ascii_lowercase().as_str()) {
        hard_ignore(&mut c, "Windows shell surface");
        return c;
    }

    // Heuristics (overridable by rules).
    let mut decision = Decision::Tile;
    let mut heuristic = |d: Decision, why: String, reasons: &mut Vec<String>| {
        if (d as u8) < (decision as u8) {
            decision = d;
        }
        reasons.push(why);
    };
    if f.tool_window {
        heuristic(Decision::Ignore, "tool window (WS_EX_TOOLWINDOW)".into(), &mut c.reasons);
    }
    if f.no_activate {
        heuristic(Decision::Ignore, "cannot be activated (WS_EX_NOACTIVATE)".into(), &mut c.reasons);
    }
    if !f.has_caption && !f.resizable {
        heuristic(Decision::Ignore, "no title bar and not resizable (popup/overlay)".into(), &mut c.reasons);
    }
    if f.fullscreen {
        heuristic(Decision::Ignore, "covers the whole monitor without a title bar (fullscreen app/game)".into(), &mut c.reasons);
    }
    if f.owned && opts.float_dialogs {
        heuristic(Decision::Float, "owned window (dialog)".into(), &mut c.reasons);
    }
    if !f.resizable && opts.float_unresizable {
        heuristic(Decision::Float, "not resizable".into(), &mut c.reasons);
    }
    if f.topmost {
        heuristic(Decision::Float, "always-on-top window".into(), &mut c.reasons);
    }
    if f.process.eq_ignore_ascii_case("twm-settings.exe") {
        heuristic(Decision::Float, "the window manager's settings window".into(), &mut c.reasons);
    }
    if !opts.manage_by_default {
        heuristic(Decision::Ignore, "general.manage_by_default = false (a rule can enable it)".into(), &mut c.reasons);
    }
    c.decision = decision;

    // Rules: applied in order; later rules override earlier ones per effect.
    for r in rules.iter().filter(|r| r.matches(f)) {
        c.matched_rules.push(r.name.clone());
        let e = &r.effects;
        if let Some(m) = e.manage {
            if m {
                if c.decision == Decision::Ignore {
                    c.decision = if f.resizable || !opts.float_unresizable { Decision::Tile } else { Decision::Float };
                }
            } else {
                c.decision = Decision::Ignore;
            }
            c.reasons.push(format!("rule `{}`: manage = {m}", r.name));
        }
        if let Some(fl) = e.float {
            if c.decision != Decision::Ignore {
                c.decision = if fl { Decision::Float } else { Decision::Tile };
            }
            c.reasons.push(format!("rule `{}`: float = {fl}", r.name));
        }
        if let Some(w) = e.workspace {
            c.workspace = Some(w);
        }
        if let Some(m) = e.monitor {
            c.monitor = Some(m);
        }
        if let Some(b) = e.border {
            c.border = b;
        }
        if let Some(s) = e.size {
            c.float_size = Some(s);
        }
        if let Some(ce) = e.center {
            c.center = ce;
        }
    }
    if c.reasons.is_empty() {
        c.reasons.push("normal resizable top-level window".into());
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTS: ClassifyOptions = ClassifyOptions { manage_by_default: true, float_unresizable: true, float_dialogs: true };

    fn normal() -> WindowFacts {
        WindowFacts {
            hwnd: 1,
            title: "Untitled - Notepad".into(),
            class: "Notepad".into(),
            process: "Notepad.exe".into(),
            visible: true,
            has_caption: true,
            resizable: true,
            ..Default::default()
        }
    }

    fn rule(m: RuleMatch, e: RuleEffects) -> Rule {
        Rule::compile("r".into(), &m, e).unwrap()
    }

    #[test]
    fn normal_window_tiles() {
        assert_eq!(classify(&normal(), &[], &OPTS).decision, Decision::Tile);
    }

    #[test]
    fn hard_limits_win_over_rules() {
        let force = rule(RuleMatch { class: Some(".*".into()), ..Default::default() }, RuleEffects { manage: Some(true), ..Default::default() });
        for f in [
            WindowFacts { elevated: true, ..normal() },
            WindowFacts { visible: false, ..normal() },
            WindowFacts { cloaked: true, ..normal() },
            WindowFacts { class: "Shell_TrayWnd".into(), ..normal() },
            WindowFacts { own_process: true, ..normal() },
        ] {
            assert_eq!(classify(&f, std::slice::from_ref(&force), &OPTS).decision, Decision::Ignore, "{f:?}");
        }
    }

    #[test]
    fn heuristics() {
        assert_eq!(classify(&WindowFacts { owned: true, ..normal() }, &[], &OPTS).decision, Decision::Float);
        assert_eq!(classify(&WindowFacts { resizable: false, ..normal() }, &[], &OPTS).decision, Decision::Float);
        assert_eq!(classify(&WindowFacts { tool_window: true, ..normal() }, &[], &OPTS).decision, Decision::Ignore);
        assert_eq!(classify(&WindowFacts { fullscreen: true, ..normal() }, &[], &OPTS).decision, Decision::Ignore);
        assert_eq!(classify(&WindowFacts { process: "twm-settings.exe".into(), ..normal() }, &[], &OPTS).decision, Decision::Float);
        let off = ClassifyOptions { float_dialogs: false, float_unresizable: false, ..OPTS };
        assert_eq!(classify(&WindowFacts { owned: true, resizable: false, ..normal() }, &[], &off).decision, Decision::Tile);
    }

    #[test]
    fn rules_full_match_and_case_insensitive_process() {
        let r = rule(RuleMatch { process: Some("notepad\\.exe".into()), ..Default::default() }, RuleEffects { float: Some(true), workspace: Some(3), ..Default::default() });
        let c = classify(&normal(), std::slice::from_ref(&r), &OPTS);
        assert_eq!(c.decision, Decision::Float);
        assert_eq!(c.workspace, Some(3));
        assert_eq!(c.matched_rules, vec!["r"]);
        // Full match: "note" must not match "Notepad.exe".
        let partial = rule(RuleMatch { process: Some("note".into()), ..Default::default() }, RuleEffects { float: Some(true), ..Default::default() });
        assert_eq!(classify(&normal(), &[partial], &OPTS).decision, Decision::Tile);
        // Class matching is case-sensitive.
        let cls = rule(RuleMatch { class: Some("notepad".into()), ..Default::default() }, RuleEffects { float: Some(true), ..Default::default() });
        assert_eq!(classify(&normal(), &[cls], &OPTS).decision, Decision::Tile);
    }

    #[test]
    fn later_rules_override_and_opt_in_mode() {
        let a = rule(RuleMatch { class: Some("Notepad".into()), ..Default::default() }, RuleEffects { float: Some(true), ..Default::default() });
        let b = rule(RuleMatch { title: Some(".*Notepad".into()), ..Default::default() }, RuleEffects { float: Some(false), ..Default::default() });
        assert_eq!(classify(&normal(), &[a, b], &OPTS).decision, Decision::Tile);

        let opt_in = ClassifyOptions { manage_by_default: false, ..OPTS };
        assert_eq!(classify(&normal(), &[], &opt_in).decision, Decision::Ignore);
        let enable = rule(RuleMatch { class: Some("Notepad".into()), ..Default::default() }, RuleEffects { manage: Some(true), ..Default::default() });
        assert_eq!(classify(&normal(), &[enable], &opt_in).decision, Decision::Tile);
    }

    #[test]
    fn rule_compile_errors_are_reported() {
        assert!(Rule::compile("x".into(), &RuleMatch { title: Some("(".into()), ..Default::default() }, RuleEffects::default()).unwrap_err().contains("match.title"));
        assert!(Rule::compile("x".into(), &RuleMatch::default(), RuleEffects::default()).unwrap_err().contains("empty"));
    }

    #[test]
    fn kind_detection() {
        assert_eq!(normal().kind(), WindowKind::Normal);
        assert_eq!(WindowFacts { owned: true, ..normal() }.kind(), WindowKind::Dialog);
        assert_eq!(WindowFacts { tool_window: true, ..normal() }.kind(), WindowKind::Tool);
        let r = rule(RuleMatch { kind: Some(WindowKind::Dialog), ..Default::default() }, RuleEffects { manage: Some(false), ..Default::default() });
        assert_eq!(classify(&WindowFacts { owned: true, ..normal() }, &[r], &OPTS).decision, Decision::Ignore);
    }
}
