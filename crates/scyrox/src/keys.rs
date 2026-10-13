//! Key tables recovered from the web driver, used by combo keys and macros.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HidKey {
    /// 0 modifier (bitmask), 1 keyboard usage, 2 consumer (media), 4 mouse button, 7 menu
    pub kind: u8,
    pub value: u16,
}

pub const MOUSE_BUTTONS: [(&str, u16); 5] = [
    ("Mouse Left", 0x01),
    ("Mouse Right", 0x02),
    ("Mouse Middle", 0x04),
    ("Mouse Back", 0x08),
    ("Mouse Forward", 0x10),
];

impl HidKey {
    pub const fn new(kind: u8, value: u16) -> Self {
        HidKey { kind, value }
    }

    pub fn is_modifier(self) -> bool {
        self.kind == 0
    }

    pub fn label(self) -> String {
        let found = match self.kind {
            2 => MEDIA_TABLE.iter().find(|(_, v)| *v == self.value).map(|(l, _)| *l),
            4 => MOUSE_BUTTONS.iter().find(|(_, v)| *v == self.value).map(|(l, _)| *l),
            _ => KEY_TABLE
                .iter()
                .find(|(_, _, k, v)| *k == self.kind && *v == self.value)
                .map(|(_, l, _, _)| *l),
        };
        found.map_or_else(|| format!("{}:{:#06x}", self.kind, self.value), str::to_string)
    }

    /// Accepts table names ("A", "F5", "ControlLeft"), labels ("LCtrl", "Esc"),
    /// common aliases (ctrl, shift, alt, win), `mouse-left`.. and `media:<label>`.
    pub fn parse(s: &str) -> Option<HidKey> {
        let l = s.trim().to_ascii_lowercase();
        let alias = match l.as_str() {
            "ctrl" | "control" => "lctrl",
            "shift" => "lshift",
            "alt" => "lalt",
            "win" | "cmd" | "meta" | "super" => "lwin",
            "escape" => "esc",
            "return" => "enter",
            "del" | "delete" => "del",
            "up" => "arrowup",
            "down" => "arrowdown",
            "left" => "arrowleft",
            "right" => "arrowright",
            "=" | "equal" => "+",
            other => other,
        };
        if let Some(m) = alias.strip_prefix("mouse-") {
            let i = ["left", "right", "middle", "back", "forward"].iter().position(|n| *n == m)?;
            return Some(HidKey::new(4, MOUSE_BUTTONS[i].1));
        }
        if let Some(m) = alias.strip_prefix("media:") {
            return MEDIA_TABLE
                .iter()
                .find(|(n, _)| n.to_ascii_lowercase() == m)
                .map(|(_, v)| HidKey::new(2, *v));
        }
        KEY_TABLE
            .iter()
            .find(|(n, label, _, _)| n.to_ascii_lowercase() == alias || label.to_ascii_lowercase() == alias)
            .map(|(_, _, k, v)| HidKey::new(*k, *v))
    }
}

/// (name, label, kind, value) from the web driver key table (`Ur`).
/// kind 0 = modifier bitmask, 1 = keyboard usage, 7 = menu.
pub const KEY_TABLE: &[(&str, &str, u8, u16)] = &[
    ("Escape", "Esc", 1, 41),
    ("F1", "F1", 1, 58),
    ("F2", "F2", 1, 59),
    ("F3", "F3", 1, 60),
    ("F4", "F4", 1, 61),
    ("F5", "F5", 1, 62),
    ("F6", "F6", 1, 63),
    ("F7", "F7", 1, 64),
    ("F8", "F8", 1, 65),
    ("F9", "F9", 1, 66),
    ("F10", "F10", 1, 67),
    ("F11", "F11", 1, 68),
    ("F12", "F12", 1, 69),
    ("Backquote", "`", 1, 53),
    ("1", "1", 1, 30),
    ("2", "2", 1, 31),
    ("3", "3", 1, 32),
    ("4", "4", 1, 33),
    ("5", "5", 1, 34),
    ("6", "6", 1, 35),
    ("7", "7", 1, 36),
    ("8", "8", 1, 37),
    ("9", "9", 1, 38),
    ("0", "0", 1, 39),
    ("Minus", "-", 1, 45),
    ("Equal", "+", 1, 46),
    ("Backspace", "Back", 1, 42),
    ("Tab", "Tab", 1, 43),
    ("Q", "Q", 1, 20),
    ("W", "W", 1, 26),
    ("E", "E", 1, 8),
    ("R", "R", 1, 21),
    ("T", "T", 1, 23),
    ("Y", "Y", 1, 28),
    ("U", "U", 1, 24),
    ("I", "I", 1, 12),
    ("O", "O", 1, 18),
    ("P", "P", 1, 19),
    ("BracketLeft", "[", 1, 47),
    ("BracketRight", "]", 1, 48),
    ("Backslash", "|", 1, 49),
    ("CapsLock", "CapsLock", 1, 57),
    ("A", "A", 1, 4),
    ("S", "S", 1, 22),
    ("D", "D", 1, 7),
    ("F", "F", 1, 9),
    ("G", "G", 1, 10),
    ("H", "H", 1, 11),
    ("J", "J", 1, 13),
    ("K", "K", 1, 14),
    ("L", "L", 1, 15),
    ("Semicolon", ":", 1, 51),
    ("Quote", "'", 1, 52),
    ("Enter", "Enter", 1, 40),
    ("ShiftLeft", "LShift", 0, 2),
    ("Z", "Z", 1, 29),
    ("X", "X", 1, 27),
    ("C", "C", 1, 6),
    ("V", "V", 1, 25),
    ("B", "B", 1, 5),
    ("N", "N", 1, 17),
    ("M", "M", 1, 16),
    ("Comma", ",", 1, 54),
    ("Period", ".", 1, 55),
    ("Slash", "/", 1, 56),
    ("ShiftRight", "RShift", 0, 32),
    ("ControlLeft", "LCtrl", 0, 1),
    ("MetaLeft", "LWin", 0, 8),
    ("AltLeft", "LAlt", 0, 4),
    ("Space", "Space", 1, 44),
    ("AltRight", "RAlt", 0, 64),
    ("MetaRight", "RWin", 0, 128),
    ("ContextMenu", "Menu", 7, 1),
    ("ControlRight", "RCtrl", 0, 16),
    ("PrintScreen", "Screen", 1, 70),
    ("ScrollLock", "Scroll", 1, 71),
    ("Pause", "Pause", 1, 72),
    ("Insert", "Insert", 1, 73),
    ("Home", "Home", 1, 74),
    ("PageUp", "PageUp", 1, 75),
    ("Delete", "Del", 1, 76),
    ("End", "End", 1, 77),
    ("PageDown", "PageDn", 1, 78),
    ("ArrowUp", "↑", 1, 82),
    ("ArrowLeft", "←", 1, 80),
    ("ArrowDown", "↓", 1, 81),
    ("ArrowRight", "→", 1, 79),
    ("NumLock", "NumLock", 1, 83),
    ("NumpadDivide", "Num/", 1, 84),
    ("NumpadMultiply", "Num*", 1, 85),
    ("NumpadSubtract", "Num-", 1, 86),
    ("NumpadAdd", "Num+", 1, 87),
    ("NumpadDecimal", "Num.", 1, 99),
    ("NumpadEnter", "Enter", 1, 88),
    ("Numpad1", "Num1", 1, 89),
    ("Numpad2", "Num2", 1, 90),
    ("Numpad3", "Num3", 1, 91),
    ("Numpad4", "Num4", 1, 92),
    ("Numpad5", "Num5", 1, 93),
    ("Numpad6", "Num6", 1, 94),
    ("Numpad7", "Num7", 1, 95),
    ("Numpad8", "Num8", 1, 96),
    ("Numpad9", "Num9", 1, 97),
    ("Numpad0", "Num0", 1, 98),
    ("Apps", "Apps", 1, 101),
    ("IntlYen", "K14 |    ¥", 1, 137),
    ("IntlRo", "K56 -\\ろ", 1, 135),
    ("Convert", "K132 変換", 1, 138),
    ("NonConvert", "K131 無変換", 1, 139),
    ("KanaMode", "Roma 力夕力ナ", 1, 136),
    ("IntlBackslash", "K45", 1, 100),
    ("Backslash2", "K42", 1, 50),
    ("HangulHanja", "K150 한사", 1, 145),
    ("Hangul", "151 한/영 かな", 1, 144),
    ("K107", "K107", 1, 133),
];

/// Consumer-page usages offered under "Multimedia" (lang/en.json).
pub const MEDIA_TABLE: &[(&str, u16)] = &[
    ("Media player", 0x0183),
    ("Play/Pause", 0x00CD),
    ("Next Track", 0x00B5),
    ("Previous Track", 0x00B6),
    ("Stop Playback", 0x00B7),
    ("Mute", 0x00E2),
    ("Volume+", 0x00E9),
    ("Volume-", 0x00EA),
    ("Email", 0x018A),
    ("Calculator", 0x0192),
    ("My Computer", 0x0194),
    ("Homepage", 0x0223),
    ("Search", 0x0221),
    ("Next page", 0x0225),
    ("Previous page", 0x0224),
    ("Stop page", 0x0226),
    ("Refresh page", 0x0227),
    ("Favorites", 0x022A),
];
