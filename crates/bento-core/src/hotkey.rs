//! Keyboard shortcut description, parsing and formatting.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A global shortcut. `key` is a Win32 virtual-key code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: u16,
}

const NAMED: &[(&str, u16)] = &[
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Tab", 0x09),
    ("Escape", 0x1B),
    ("Backquote", 0xC0),
];

fn key_name(vk: u16) -> Option<String> {
    if let Some((n, _)) = NAMED.iter().find(|(_, v)| *v == vk) {
        return Some((*n).to_owned());
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(u32::from(vk)).map(|c| c.to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => None,
    }
}

fn key_code(name: &str) -> Option<u16> {
    if let Some((_, v)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(*v);
    }
    let upper = name.to_ascii_uppercase();
    let mut chars = upper.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && (c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some(c as u16);
    }
    if let Some(n) = upper.strip_prefix('F') {
        let n: u16 = n.parse().ok()?;
        if (1..=24).contains(&n) {
            return Some(0x6F + n);
        }
    }
    None
}

impl Hotkey {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, win: bool, key: u16) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            win,
            key,
        }
    }

    /// A shortcut needs a key and at least one modifier.
    pub fn is_valid(&self) -> bool {
        self.key != 0 && (self.ctrl || self.alt || self.shift || self.win)
    }

    /// Parses `Ctrl+Alt+Right`, case-insensitive.
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey::default();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" => hk.alt = true,
                "shift" | "maj" => hk.shift = true,
                "win" | "windows" => hk.win = true,
                _ => {
                    if hk.key != 0 {
                        return None;
                    }
                    hk.key = key_code(part)?;
                }
            }
        }
        hk.is_valid().then_some(hk)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.win {
            parts.push("Win".into());
        }
        parts.push(key_name(self.key).unwrap_or_else(|| format!("0x{:02X}", self.key)));
        f.write_str(&parts.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format() {
        let hk = Hotkey::parse("ctrl+win+space").unwrap();
        assert!(hk.ctrl && hk.win && !hk.alt);
        assert_eq!(hk.key, 0x20);
        assert_eq!(hk.to_string(), "Ctrl+Win+Space");
        assert_eq!(Hotkey::parse("Alt+F12").unwrap().key, 0x7B);
        assert_eq!(Hotkey::parse("Shift+a").unwrap().to_string(), "Shift+A");
    }

    #[test]
    fn rejects_invalid() {
        assert!(Hotkey::parse("Space").is_none());
        assert!(Hotkey::parse("Ctrl+A+B").is_none());
        assert!(Hotkey::parse("Ctrl+Nope").is_none());
        assert!(Hotkey::parse("").is_none());
    }
}
