//! Dispatchable actions. The text form is used by key bindings, the IPC
//! (`twmctl dispatch ...`) and the settings UI. Both readable names
//! (`focus left`) and Hyprland dispatcher names (`movefocus l`) are accepted.

use crate::geom::Direction;
use crate::layout::LayoutKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WsTarget {
    Id(u32),
    /// Next/previous existing (non-empty or visible) workspace on the focused monitor.
    Next,
    Prev,
    /// The previously focused workspace (Hyprland `previous`).
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonTarget {
    Next,
    Prev,
    Dir(Direction),
    /// 1-based monitor number.
    Index(usize),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Focus(Direction),
    /// Focus the next/previous window of the workspace in layout order.
    Cycle(bool),
    Move(Direction),
    Resize(Direction, f32),
    Workspace(WsTarget),
    MoveToWorkspace(WsTarget, bool /* follow */),
    FocusMonitor(MonTarget),
    MoveToMonitor(MonTarget),
    SetLayout(LayoutKind),
    NextLayout,
    ToggleFloat,
    ToggleFullscreen,
    ToggleSplit,
    SplitRatio(f32, bool /* exact */),
    SwapWithMaster,
    OrientationNext,
    Close,
    Exec(String),
    Reload,
    TogglePause,
    Retile,
    OpenSettings,
    OpenConfig,
    Quit,
}

fn parse_ws(s: &str) -> Result<WsTarget, String> {
    match s {
        "next" | "+1" | "e+1" | "r+1" => Ok(WsTarget::Next),
        "prev" | "-1" | "e-1" | "r-1" => Ok(WsTarget::Prev),
        "previous" | "last" | "back" => Ok(WsTarget::Previous),
        n => match n.parse::<u32>() {
            Ok(i) if (1..=99).contains(&i) => Ok(WsTarget::Id(i)),
            _ => Err(format!("workspace must be 1-99, next, prev or previous (got `{s}`)")),
        },
    }
}

fn parse_mon(s: &str) -> Result<MonTarget, String> {
    match s {
        "next" | "+1" => Ok(MonTarget::Next),
        "prev" | "-1" => Ok(MonTarget::Prev),
        _ => {
            if let Some(d) = Direction::parse(s) {
                return Ok(MonTarget::Dir(d));
            }
            match s.parse::<usize>() {
                Ok(i) if i >= 1 => Ok(MonTarget::Index(i)),
                _ => Err(format!("monitor must be next, prev, a direction or a number ≥ 1 (got `{s}`)")),
            }
        }
    }
}

fn dir(arg: Option<&str>, verb: &str) -> Result<Direction, String> {
    let a = arg.ok_or_else(|| format!("`{verb}` needs a direction: left, right, up or down"))?;
    Direction::parse(a).ok_or_else(|| format!("`{verb}`: `{a}` is not a direction (left, right, up, down)"))
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        let s = s.trim();
        let (verb, rest) = match s.find(char::is_whitespace) {
            Some(i) => (&s[..i], s[i..].trim()),
            None => (s, ""),
        };
        let arg = (!rest.is_empty()).then_some(rest);
        let verb_l = verb.to_ascii_lowercase();
        let need = |what: &str| arg.ok_or_else(|| format!("`{verb}` needs {what}"));
        let a = match verb_l.as_str() {
            "focus" | "movefocus" => Action::Focus(dir(arg, verb)?),
            "cyclenext" => Action::Cycle(true),
            "cycleprev" => Action::Cycle(false),
            "move" | "movewindow" | "swap" | "swapwindow" => Action::Move(dir(arg, verb)?),
            "resize" | "resizeactive" => {
                let a = need("a direction and optional amount, e.g. `resize right 0.05`")?;
                let mut it = a.split_whitespace();
                let d = dir(it.next(), verb)?;
                let amt = match it.next() {
                    Some(x) => x.parse::<f32>().map_err(|_| format!("`resize`: amount `{x}` is not a number"))?,
                    None => 0.05,
                };
                if !(0.0..=0.9).contains(&amt) {
                    return Err("`resize`: amount must be between 0 and 0.9 (fraction of the screen)".into());
                }
                Action::Resize(d, amt)
            }
            "workspace" => Action::Workspace(parse_ws(need("a workspace")?)?),
            "movetoworkspace" => Action::MoveToWorkspace(parse_ws(need("a workspace")?)?, true),
            "movetoworkspacesilent" => Action::MoveToWorkspace(parse_ws(need("a workspace")?)?, false),
            "focusmonitor" => Action::FocusMonitor(parse_mon(need("a monitor")?)?),
            "movetomonitor" | "movewindowtomonitor" => Action::MoveToMonitor(parse_mon(need("a monitor")?)?),
            "layout" => {
                let a = need("dwindle, master, monocle or next")?;
                if a == "next" {
                    Action::NextLayout
                } else {
                    Action::SetLayout(LayoutKind::parse(a).ok_or_else(|| format!("unknown layout `{a}` (dwindle, master, monocle, next)"))?)
                }
            }
            "nextlayout" | "cyclelayout" => Action::NextLayout,
            "togglefloating" | "togglefloat" | "float" => Action::ToggleFloat,
            "fullscreen" | "togglefullscreen" | "maximize" => Action::ToggleFullscreen,
            "togglesplit" => Action::ToggleSplit,
            "splitratio" => {
                let a = need("a value, e.g. `splitratio +0.1` or `splitratio exact 1.2`")?;
                let (exact, v) = match a.strip_prefix("exact") {
                    Some(v) => (true, v.trim()),
                    None => (false, a),
                };
                let f = v.parse::<f32>().map_err(|_| format!("`splitratio`: `{v}` is not a number"))?;
                Action::SplitRatio(f, exact)
            }
            "swapwithmaster" => Action::SwapWithMaster,
            "orientationnext" => Action::OrientationNext,
            "close" | "killactive" => Action::Close,
            "exec" => Action::Exec(need("a command line")?.to_string()),
            "reload" => Action::Reload,
            "togglepause" | "pause" | "toggletiling" => Action::TogglePause,
            "retile" => Action::Retile,
            "settings" | "opensettings" => Action::OpenSettings,
            "openconfig" | "editconfig" => Action::OpenConfig,
            "quit" | "exit" => Action::Quit,
            _ => return Err(format!("unknown action `{verb}`")),
        };
        // Reject stray arguments for argument-less actions.
        let takes_arg = matches!(
            a,
            Action::Focus(_) | Action::Move(_) | Action::Resize(..) | Action::Workspace(_) | Action::MoveToWorkspace(..)
                | Action::FocusMonitor(_) | Action::MoveToMonitor(_) | Action::SetLayout(_) | Action::SplitRatio(..) | Action::Exec(_)
        ) || (verb_l == "layout");
        if !takes_arg && arg.is_some() {
            return Err(format!("`{verb}` takes no arguments"));
        }
        Ok(a)
    }

    /// Short human-readable description for the binding cheat sheet.
    pub fn describe(&self) -> String {
        match self {
            Action::Focus(d) => format!("Focus window {d:?}").to_lowercase(),
            Action::Cycle(n) => if *n { "Focus next window".into() } else { "Focus previous window".into() },
            Action::Move(d) => format!("Move window {d:?}").to_lowercase(),
            Action::Resize(d, a) => format!("Grow window {} by {:.0}%", format!("{d:?}").to_lowercase(), a * 100.0),
            Action::Workspace(t) => format!("Go to workspace {}", ws_name(t)),
            Action::MoveToWorkspace(t, f) => format!("Move window to workspace {}{}", ws_name(t), if *f { "" } else { " (stay)" }),
            Action::FocusMonitor(_) => "Focus monitor".into(),
            Action::MoveToMonitor(_) => "Move window to monitor".into(),
            Action::SetLayout(k) => format!("Layout: {}", k.name()),
            Action::NextLayout => "Cycle layout".into(),
            Action::ToggleFloat => "Toggle floating".into(),
            Action::ToggleFullscreen => "Toggle fullscreen (fill workspace)".into(),
            Action::ToggleSplit => "Toggle split direction".into(),
            Action::SplitRatio(..) => "Change split ratio".into(),
            Action::SwapWithMaster => "Swap with master".into(),
            Action::OrientationNext => "Next master orientation".into(),
            Action::Close => "Close window".into(),
            Action::Exec(c) => format!("Run {c}"),
            Action::Reload => "Reload configuration".into(),
            Action::TogglePause => "Pause/resume tiling".into(),
            Action::Retile => "Re-apply layout".into(),
            Action::OpenSettings => "Open settings".into(),
            Action::OpenConfig => "Edit config file".into(),
            Action::Quit => "Quit (restores all windows)".into(),
        }
    }
}

fn ws_name(t: &WsTarget) -> String {
    match t {
        WsTarget::Id(i) => i.to_string(),
        WsTarget::Next => "next".into(),
        WsTarget::Prev => "prev".into(),
        WsTarget::Previous => "previous".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_readable_and_hyprland_names() {
        assert_eq!(Action::parse("focus left").unwrap(), Action::Focus(Direction::Left));
        assert_eq!(Action::parse("movefocus l").unwrap(), Action::Focus(Direction::Left));
        assert_eq!(Action::parse("workspace 3").unwrap(), Action::Workspace(WsTarget::Id(3)));
        assert_eq!(Action::parse("movetoworkspacesilent 2").unwrap(), Action::MoveToWorkspace(WsTarget::Id(2), false));
        assert_eq!(Action::parse("layout master").unwrap(), Action::SetLayout(LayoutKind::Master));
        assert_eq!(Action::parse("layout next").unwrap(), Action::NextLayout);
        assert_eq!(Action::parse("splitratio exact 1.5").unwrap(), Action::SplitRatio(1.5, true));
        assert_eq!(Action::parse("resize right").unwrap(), Action::Resize(Direction::Right, 0.05));
        assert_eq!(Action::parse("exec wt.exe -w 0").unwrap(), Action::Exec("wt.exe -w 0".into()));
        assert_eq!(Action::parse("killactive").unwrap(), Action::Close);
    }

    #[test]
    fn rejects_bad_actions() {
        assert!(Action::parse("focus sideways").is_err());
        assert!(Action::parse("workspace 0").is_err());
        assert!(Action::parse("workspace").is_err());
        assert!(Action::parse("fly away").unwrap_err().contains("unknown action"));
        assert!(Action::parse("close now").unwrap_err().contains("takes no arguments"));
        assert!(Action::parse("resize right 5").is_err());
    }
}
