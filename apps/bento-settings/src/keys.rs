//! Shortcut recorder: turns a Slint key press into the text of a
//! [`Hotkey`].

use bento_core::hotkey::Hotkey;
use slint::platform::Key;

/// Special keys accepted in a shortcut, with their virtual-key code.
const SPECIAL: &[(Key, u16)] = &[
    (Key::PageUp, 0x21),
    (Key::PageDown, 0x22),
    (Key::End, 0x23),
    (Key::Home, 0x24),
    (Key::LeftArrow, 0x25),
    (Key::UpArrow, 0x26),
    (Key::RightArrow, 0x27),
    (Key::DownArrow, 0x28),
    (Key::Insert, 0x2D),
    (Key::Delete, 0x2E),
    (Key::Tab, 0x09),
    (Key::Space, 0x20),
    (Key::F1, 0x70),
    (Key::F2, 0x71),
    (Key::F3, 0x72),
    (Key::F4, 0x73),
    (Key::F5, 0x74),
    (Key::F6, 0x75),
    (Key::F7, 0x76),
    (Key::F8, 0x77),
    (Key::F9, 0x78),
    (Key::F10, 0x79),
    (Key::F11, 0x7A),
    (Key::F12, 0x7B),
];

/// Top row of the French AZERTY layout, which types symbols for the digits.
const AZERTY_DIGITS: &[(char, u16)] = &[
    ('&', 0x31),
    ('é', 0x32),
    ('"', 0x33),
    ('\'', 0x34),
    ('(', 0x35),
    ('-', 0x36),
    ('è', 0x37),
    ('_', 0x38),
    ('ç', 0x39),
    ('à', 0x30),
];

fn virtual_key(c: char) -> Option<u16> {
    if c.is_ascii_alphanumeric() {
        return Some(c.to_ascii_uppercase() as u16);
    }
    if c == '`' || c == '²' {
        return Some(0xC0);
    }
    if let Some((_, vk)) = SPECIAL.iter().find(|(k, _)| char::from(*k) == c) {
        return Some(*vk);
    }
    AZERTY_DIGITS
        .iter()
        .find(|(d, _)| *d == c)
        .map(|(_, vk)| *vk)
}

/// Text of the shortcut, or an empty string when the key cannot be used
/// (a modifier alone, or no modifier at all).
pub fn capture(text: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> String {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return String::new();
    };
    let Some(key) = virtual_key(c) else {
        return String::new();
    };
    let hk = Hotkey::new(ctrl, alt, shift, win, key);
    if hk.is_valid() {
        hk.to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_specials() {
        assert_eq!(capture("d", true, true, false, false), "Ctrl+Alt+D");
        let pd = char::from(Key::PageDown).to_string();
        assert_eq!(capture(&pd, true, false, false, true), "Ctrl+Win+PageDown");
        assert_eq!(capture("é", true, false, false, false), "Ctrl+2");
    }

    #[test]
    fn needs_a_modifier_and_a_key() {
        assert_eq!(capture("d", false, false, false, false), "");
        assert_eq!(capture("", true, false, false, false), "");
        let ctrl = char::from(Key::Control).to_string();
        assert_eq!(capture(&ctrl, true, false, false, false), "");
    }
}
