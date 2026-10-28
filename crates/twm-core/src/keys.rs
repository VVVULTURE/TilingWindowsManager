//! Key chords such as `win+shift+1`. Virtual-key codes are the documented
//! Windows VK_* values (https://learn.microsoft.com/windows/win32/inputdev/virtual-key-codes);
//! they are plain numbers so parsing and validation stay testable here.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Mods {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Chord {
    pub mods: Mods,
    pub vk: u16,
}

const NAMED: &[(&str, u16)] = &[
    ("backspace", 0x08), ("tab", 0x09), ("return", 0x0D), ("enter", 0x0D),
    ("pause", 0x13), ("capslock", 0x14), ("escape", 0x1B), ("esc", 0x1B), ("space", 0x20),
    ("pageup", 0x21), ("prior", 0x21), ("pagedown", 0x22), ("next", 0x22), ("end", 0x23), ("home", 0x24),
    ("left", 0x25), ("up", 0x26), ("right", 0x27), ("down", 0x28),
    ("print", 0x2C), ("printscreen", 0x2C), ("insert", 0x2D), ("delete", 0x2E), ("del", 0x2E),
    ("numpad0", 0x60), ("numpad1", 0x61), ("numpad2", 0x62), ("numpad3", 0x63), ("numpad4", 0x64),
    ("numpad5", 0x65), ("numpad6", 0x66), ("numpad7", 0x67), ("numpad8", 0x68), ("numpad9", 0x69),
    ("multiply", 0x6A), ("add", 0x6B), ("subtract", 0x6D), ("decimal", 0x6E), ("divide", 0x6F),
    ("semicolon", 0xBA), (";", 0xBA), ("equal", 0xBB), ("equals", 0xBB), ("=", 0xBB), ("plus", 0xBB),
    ("comma", 0xBC), (",", 0xBC), ("minus", 0xBD), ("-", 0xBD), ("period", 0xBE), (".", 0xBE),
    ("slash", 0xBF), ("/", 0xBF), ("grave", 0xC0), ("backtick", 0xC0), ("`", 0xC0),
    ("bracketleft", 0xDB), ("[", 0xDB), ("backslash", 0xDC), ("\\", 0xDC),
    ("bracketright", 0xDD), ("]", 0xDD), ("apostrophe", 0xDE), ("quote", 0xDE), ("'", 0xDE),
];

pub fn key_from_name(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase() as u16);
        }
        if c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(f) = n.strip_prefix('f') {
        if let Ok(i) = f.parse::<u16>() {
            if (1..=24).contains(&i) {
                return Some(0x70 + i - 1);
            }
        }
    }
    NAMED.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 => (vk as u8 as char).to_string(),
        0x41..=0x5A => (vk as u8 as char).to_ascii_lowercase().to_string(),
        0x70..=0x87 => format!("f{}", vk - 0x70 + 1),
        _ => NAMED.iter().find(|(_, v)| *v == vk).map(|(k, _)| k.to_string()).unwrap_or_else(|| format!("vk{vk:#04x}")),
    }
}

impl Chord {
    /// Parses `mod+mod+key`. `vars` supplies `$name` substitutions (e.g.
    /// `$mod` -> `win`), mirroring Hyprland's `$mainMod` convention.
    pub fn parse(s: &str, vars: &dyn Fn(&str) -> Option<String>) -> Result<Chord, String> {
        let expanded = expand_vars(s, vars)?;
        let parts: Vec<&str> = expanded.split('+').map(str::trim).collect();
        // "+" as the key itself: "ctrl++" -> ["ctrl", "", ""]
        let (mod_parts, key) = if expanded.ends_with("++") {
            (&parts[..parts.len() - 2], "plus")
        } else {
            let (k, m) = parts.split_last().ok_or("empty key chord")?;
            (m, *k)
        };
        let mut mods = Mods::default();
        for m in mod_parts {
            match m.to_ascii_lowercase().as_str() {
                "win" | "super" | "mod4" | "meta" => mods.win = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "mod1" => mods.alt = true,
                "shift" => mods.shift = true,
                "" => return Err(format!("`{s}`: empty modifier")),
                other => return Err(format!("`{s}`: unknown modifier `{other}` (use win, ctrl, alt, shift)")),
            }
        }
        let vk = key_from_name(key).ok_or_else(|| format!("`{s}`: unknown key `{key}`"))?;
        if vk == 0x7B {
            return Err(format!("`{s}`: F12 is reserved by Windows for debuggers"));
        }
        if !(mods.win || mods.ctrl || mods.alt) && !(0x70..=0x87).contains(&vk) {
            return Err(format!("`{s}`: a binding needs win, ctrl or alt (or must be a function key) so normal typing is not captured"));
        }
        Ok(Chord { mods, vk })
    }
}

pub fn expand_vars(s: &str, vars: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let name = &after[..end];
        if name.is_empty() {
            return Err(format!("`{s}`: `$` must be followed by a variable name"));
        }
        out.push_str(&vars(name).ok_or_else(|| format!("`{s}`: undefined variable `${name}` (define it under [variables])"))?);
        rest = &after[end..];
    }
    out.push_str(rest);
    Ok(out)
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = &self.mods;
        for (on, n) in [(m.win, "win"), (m.ctrl, "ctrl"), (m.alt, "alt"), (m.shift, "shift")] {
            if on {
                write!(f, "{n}+")?;
            }
        }
        write!(f, "{}", key_name(self.vk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(n: &str) -> Option<String> {
        (n == "mod").then(|| "win".to_string())
    }

    #[test]
    fn parse_chords() {
        let c = Chord::parse("$mod+shift+1", &vars).unwrap();
        assert!(c.mods.win && c.mods.shift && !c.mods.alt);
        assert_eq!(c.vk, b'1' as u16);
        assert_eq!(c.to_string(), "win+shift+1");
        assert_eq!(Chord::parse("Alt+Return", &vars).unwrap().vk, 0x0D);
        assert_eq!(Chord::parse("win+left", &vars).unwrap().vk, 0x25);
        assert_eq!(Chord::parse("ctrl+alt+f5", &vars).unwrap().vk, 0x74);
        assert_eq!(Chord::parse("ctrl++", &vars).unwrap().vk, 0xBB);
        assert!(Chord::parse("f13", &vars).is_ok(), "function keys may be bound alone");
    }

    #[test]
    fn rejects_bad_chords() {
        assert!(Chord::parse("hyper+x", &vars).unwrap_err().contains("unknown modifier"));
        assert!(Chord::parse("win+nokey", &vars).unwrap_err().contains("unknown key"));
        assert!(Chord::parse("$undefined+x", &vars).unwrap_err().contains("undefined variable"));
        assert!(Chord::parse("shift+a", &vars).is_err(), "would capture typing");
        assert!(Chord::parse("win+f12", &vars).is_err());
    }

    #[test]
    fn names_roundtrip() {
        for vk in [0x41u16, 0x31, 0x25, 0x0D, 0x20, 0x70, 0xBC] {
            assert_eq!(key_from_name(&key_name(vk)), Some(vk));
        }
    }
}
