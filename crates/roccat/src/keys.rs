//! Keyboard shortcuts as the Kone Aimo stores them on a button: a HID
//! modifier byte (bit 0 left Ctrl … bit 7 right GUI) and one key from the
//! HID keyboard usage page.

/// Modifier bits in the order they're printed.
const MODIFIERS: [(u8, &str); 8] = [
    (0x01, "Ctrl"),
    (0x02, "Shift"),
    (0x04, "Alt"),
    (0x08, "Win"),
    (0x10, "RCtrl"),
    (0x20, "RShift"),
    (0x40, "RAlt"),
    (0x80, "RWin"),
];

/// Named keys outside the A–Z, 0–9 and F1–F12 ranges.
const NAMED: [(u8, &str); 32] = [
    (0x28, "Enter"),
    (0x29, "Esc"),
    (0x2A, "Backspace"),
    (0x2B, "Tab"),
    (0x2C, "Space"),
    (0x2D, "-"),
    (0x2E, "="),
    (0x2F, "["),
    (0x30, "]"),
    (0x31, "\\"),
    (0x33, ";"),
    (0x34, "'"),
    (0x35, "`"),
    (0x36, ","),
    (0x37, "."),
    (0x38, "/"),
    (0x39, "CapsLock"),
    (0x46, "PrintScreen"),
    (0x47, "ScrollLock"),
    (0x48, "Pause"),
    (0x49, "Insert"),
    (0x4A, "Home"),
    (0x4B, "PageUp"),
    (0x4C, "Delete"),
    (0x4D, "End"),
    (0x4E, "PageDown"),
    (0x4F, "Right"),
    (0x50, "Left"),
    (0x51, "Down"),
    (0x52, "Up"),
    (0x53, "NumLock"),
    (0x65, "Menu"),
];

/// The name of HID keyboard usage `usage`.
pub fn key_name(usage: u8) -> String {
    match usage {
        0x04..=0x1D => ((b'A' + usage - 0x04) as char).to_string(),
        0x1E..=0x26 => ((b'1' + usage - 0x1E) as char).to_string(),
        0x27 => "0".into(),
        0x3A..=0x45 => format!("F{}", usage - 0x39),
        _ => match NAMED.iter().find(|(u, _)| *u == usage) {
            Some((_, n)) => n.to_string(),
            None => format!("0x{usage:02X}"),
        },
    }
}

/// The HID usage for a key name (case-insensitive; `0x..` for any usage).
pub fn key_usage(name: &str) -> Option<u8> {
    let n = name.trim();
    if let Some(hex) = n.strip_prefix("0x").or_else(|| n.strip_prefix("0X")) {
        return u8::from_str_radix(hex, 16).ok().filter(|u| *u != 0);
    }
    (1..=0xFF).find(|u| key_name(*u).eq_ignore_ascii_case(n))
}

/// `"Ctrl + Shift + S"`.
pub fn shortcut_label(modifiers: u8, usage: u8) -> String {
    let mut parts: Vec<String> =
        MODIFIERS.iter().filter(|(bit, _)| modifiers & bit != 0).map(|(_, n)| n.to_string()).collect();
    if usage != 0 {
        parts.push(key_name(usage));
    }
    parts.join(" + ")
}

/// Parses `"Ctrl+Shift+S"` into `(modifiers, usage)`: any modifiers, then
/// exactly one key.
pub fn parse_shortcut(text: &str) -> Option<(u8, u8)> {
    let mut modifiers = 0;
    let mut usage = None;
    for part in text.split('+').map(str::trim) {
        if let Some((bit, _)) = MODIFIERS.iter().find(|(_, n)| n.eq_ignore_ascii_case(part)) {
            modifiers |= bit;
        } else if usage.is_none() {
            usage = Some(key_usage(part)?);
        } else {
            return None;
        }
    }
    Some((modifiers, usage?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for u in 1..=0xFF {
            assert_eq!(key_usage(&key_name(u)), Some(u), "usage {u:#04x}");
        }
        assert_eq!(key_name(0x04), "A");
        assert_eq!(key_name(0x27), "0");
        assert_eq!(key_name(0x45), "F12");
    }

    #[test]
    fn shortcuts() {
        assert_eq!(parse_shortcut("ctrl + shift + s"), Some((0x03, 0x16)));
        assert_eq!(shortcut_label(0x03, 0x16), "Ctrl + Shift + S");
        assert_eq!(parse_shortcut("RAlt+F4"), Some((0x40, 0x3D)));
        assert_eq!(parse_shortcut("Ctrl"), None);
        assert_eq!(parse_shortcut("A+B"), None);
        assert_eq!(parse_shortcut("Ctrl+Nope"), None);
    }
}
