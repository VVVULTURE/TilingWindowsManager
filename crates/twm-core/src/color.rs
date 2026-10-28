//! Colours and gradients. Accepts Hyprland-style notation so existing
//! colour schemes can be reused:
//!   `rgba(33ccffee)`, `rgb(33ccff)`, `rgba(51, 204, 255, 0.93)`,
//!   `0xee33ccff` (AARRGGBB), `#33ccff`, `#33ccffee`.
//! A gradient is one or more colours followed by an optional angle, e.g.
//!   `rgba(33ccffee) rgba(00ff99ee) 45deg`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub fn parse(s: &str) -> Result<Color, String> {
        let s = s.trim();
        let hex = |h: &str| -> Result<Color, String> {
            let v = u32::from_str_radix(h, 16).map_err(|_| format!("invalid hex colour `{s}`"))?;
            match h.len() {
                6 => Ok(Color::rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)),
                8 => Ok(Color::rgba((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)),
                _ => Err(format!("colour `{s}` must have 6 or 8 hex digits")),
            }
        };
        if let Some(h) = s.strip_prefix('#') {
            return hex(h);
        }
        if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            // Hyprland legacy notation is AARRGGBB.
            if h.len() != 8 {
                return Err(format!("colour `{s}`: 0x notation needs 8 hex digits (AARRGGBB)"));
            }
            let c = hex(h)?;
            return Ok(Color::rgba(c.g, c.b, c.a, c.r));
        }
        let lower = s.to_ascii_lowercase();
        for (prefix, has_alpha) in [("rgba(", true), ("rgb(", false)] {
            if let Some(inner) = lower.strip_prefix(prefix).and_then(|x| x.strip_suffix(')')) {
                if !inner.contains(',') {
                    let c = hex(inner.trim())?;
                    if !has_alpha && inner.trim().len() != 6 {
                        return Err(format!("`{s}`: rgb() takes 6 hex digits"));
                    }
                    return Ok(c);
                }
                let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
                let want = if has_alpha { 4 } else { 3 };
                if parts.len() != want {
                    return Err(format!("`{s}`: expected {want} components"));
                }
                let ch = |p: &str| p.parse::<u8>().map_err(|_| format!("`{s}`: component `{p}` must be 0-255"));
                let a = if has_alpha {
                    let f: f32 = parts[3].parse().map_err(|_| format!("`{s}`: alpha `{}` must be 0.0-1.0", parts[3]))?;
                    if !(0.0..=1.0).contains(&f) {
                        return Err(format!("`{s}`: alpha must be 0.0-1.0"));
                    }
                    (f * 255.0).round() as u8
                } else {
                    255
                };
                return Ok(Color::rgba(ch(parts[0])?, ch(parts[1])?, ch(parts[2])?, a));
            }
        }
        Err(format!("unrecognised colour `{s}` (use #RRGGBB, #RRGGBBAA, rgba(RRGGBBAA) or rgb(RRGGBB))"))
    }

    /// Win32 COLORREF (0x00BBGGRR); alpha is dropped.
    pub fn colorref(&self) -> u32 {
        (self.b as u32) << 16 | (self.g as u32) << 8 | self.r as u32
    }

    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
    }

    pub fn lerp(a: Color, b: Color, t: f32) -> Color {
        let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
        Color::rgba(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b), f(a.a, b.a))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub colors: Vec<Color>,
    /// Degrees; 0 = left-to-right, 90 = top-to-bottom (Hyprland convention
    /// measures clockwise in screen space).
    pub angle: f32,
}

impl Gradient {
    pub fn solid(c: Color) -> Gradient {
        Gradient { colors: vec![c], angle: 0.0 }
    }

    pub fn parse(s: &str) -> Result<Gradient, String> {
        // Split on whitespace outside parentheses.
        let mut tokens = Vec::new();
        let mut depth = 0;
        let mut cur = String::new();
        for ch in s.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    cur.push(ch)
                }
                ')' => {
                    depth -= 1;
                    cur.push(ch)
                }
                c if c.is_whitespace() && depth == 0 => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }
        let mut angle = 0.0;
        if let Some(last) = tokens.last() {
            if let Some(deg) = last.strip_suffix("deg") {
                angle = deg.parse::<f32>().map_err(|_| format!("invalid gradient angle `{last}`"))?;
                tokens.pop();
            }
        }
        if tokens.is_empty() {
            return Err("gradient needs at least one colour".into());
        }
        if tokens.len() > 10 {
            return Err("gradient supports at most 10 colours".into());
        }
        let colors = tokens.iter().map(|t| Color::parse(t)).collect::<Result<Vec<_>, _>>()?;
        Ok(Gradient { colors, angle })
    }

    /// Colour at position `t` (0..=1) along the gradient.
    pub fn sample(&self, t: f32) -> Color {
        let n = self.colors.len();
        if n == 1 {
            return self.colors[0];
        }
        let t = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (t.floor() as usize).min(n - 2);
        Color::lerp(self.colors[i], self.colors[i + 1], t - i as f32)
    }

    pub fn to_config_string(&self) -> String {
        let mut s: Vec<String> = self.colors.iter().map(|c| format!("rgba({:02x}{:02x}{:02x}{:02x})", c.r, c.g, c.b, c.a)).collect();
        if self.colors.len() > 1 || self.angle != 0.0 {
            s.push(format!("{}deg", self.angle));
        }
        s.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_notations() {
        let c = Color::rgba(0x33, 0xcc, 0xff, 0xee);
        assert_eq!(Color::parse("rgba(33ccffee)").unwrap(), c);
        assert_eq!(Color::parse("#33ccffee").unwrap(), c);
        assert_eq!(Color::parse("0xee33ccff").unwrap(), c);
        assert_eq!(Color::parse("rgb(33ccff)").unwrap(), Color::rgba(0x33, 0xcc, 0xff, 255));
        assert_eq!(Color::parse("#33CCFF").unwrap(), Color::rgba(0x33, 0xcc, 0xff, 255));
        assert_eq!(Color::parse("rgba(51, 204, 255, 1.0)").unwrap(), Color::rgba(51, 204, 255, 255));
        assert!(Color::parse("blue").is_err());
        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("rgba(1,2,3,7)").is_err());
        assert!(Color::parse("0x33ccff").is_err());
    }

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(Color::rgba(0x11, 0x22, 0x33, 0xff).colorref(), 0x332211);
    }

    #[test]
    fn gradient_parse_and_sample() {
        let g = Gradient::parse("rgba(33ccffee) rgba(00ff99ee) 45deg").unwrap();
        assert_eq!(g.colors.len(), 2);
        assert_eq!(g.angle, 45.0);
        assert_eq!(g.sample(0.0), g.colors[0]);
        assert_eq!(g.sample(1.0), g.colors[1]);
        let g2 = Gradient::parse(&g.to_config_string()).unwrap();
        assert_eq!(g, g2);
        assert!(Gradient::parse("45deg").is_err());
        assert!(Gradient::parse("rgba(51, 204, 255, 1.0) #000000").is_ok(), "spaces inside parentheses");
    }
}
