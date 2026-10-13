//! Wire protocol of the Scyrox web driver (www.scyrox.net), recovered from its
//! WebHID bundle. Every exchange is a 16-byte report with report ID 8:
//!
//! ```text
//! [cmd, status, addr_hi, addr_lo, len, data[0..10], checksum]
//! ```
//!
//! `checksum` makes the 16 bytes plus the report ID sum to 0x55 (mod 256).
//! Replies echo the header; `status == 1` means "not supported / failed".

use crate::keys::HidKey;

pub const VENDOR_ID: u16 = 0x3554;
/// Wireless dongles, then wired mice (cfg.json "pid.mouse").
pub const PRODUCT_IDS: &[u16] = &[0xF5F4, 0xF5F7, 0xF517, 0xF5F6, 0xF515];
pub const REPORT_ID: u8 = 8;
pub const PACKET_LEN: usize = 16;
pub const CHUNK: usize = 10;

pub mod cmd {
    pub const ENCRYPTION_DATA: u8 = 1; // handshake; reply carries cid/mid/type
    pub const DEVICE_ONLINE: u8 = 3;
    pub const BATTERY_LEVEL: u8 = 4;
    pub const DONGLE_ENTER_PAIR: u8 = 5;
    pub const GET_PAIR_STATE: u8 = 6;
    pub const WRITE_FLASH: u8 = 7;
    pub const READ_FLASH: u8 = 8;
    pub const CLEAR_SETTING: u8 = 9; // factory reset
    pub const STATUS_CHANGED: u8 = 10; // unsolicited: settings changed on the mouse
    pub const GET_CURRENT_CONFIG: u8 = 14;
    pub const SET_CURRENT_CONFIG: u8 = 15;
    pub const READ_VERSION: u8 = 18;
    pub const SET_LONG_RANGE: u8 = 22;
    pub const GET_LONG_RANGE: u8 = 23;
    pub const GET_DONGLE_VERSION: u8 = 29;
}

/// Mouse flash map (`Se` in the bundle). Single-byte settings are stored as
/// the pair `[value, 0x55 - value]`.
pub mod addr {
    pub const REPORT_RATE: u16 = 0;
    pub const DPI_STAGE_COUNT: u16 = 2;
    pub const CURRENT_DPI: u16 = 4;
    pub const SCAN_20K: u16 = 8; // "20K" switch on scyrox.net (Set_MS_Scyrox20K)
    pub const LOD: u16 = 10;
    pub const DPI_VALUE: u16 = 12; // 8 stages x 4 bytes
    pub const DPI_COLOR: u16 = 44; // 8 stages x [r, g, b, cs]
    pub const KEY_FUNCTION: u16 = 96; // 8 keys x [type, hi, lo, cs]
    pub const DEBOUNCE: u16 = 169;
    pub const MOTION_SYNC: u16 = 171;
    pub const SLEEP_TIME: u16 = 173;
    pub const ANGLE_SNAP: u16 = 175;
    pub const RIPPLE: u16 = 177;
    pub const COMPETITION: u16 = 181; // "Competition mode" (PerformanceState)
    pub const PERFORMANCE: u16 = 183;
    pub const SENSOR_MODE: u16 = 185;
    pub const ANGLE_TUNE: u16 = 189;
    pub const ANGLE_TUNE_STATE: u16 = 191;
    pub const WHEEL_DEBOUNCE: u16 = 227;
    pub const DEBOUNCE_RELEASE: u16 = 229;
    pub const SHORTCUT: u16 = 256; // 32 bytes per key
    pub const MACRO: u16 = 768; // 384 bytes per key
    pub const SENSOR_3955_DPI: u16 = 6912; // 8 stages x 6 bytes
}

/// `0x55 - sum(bytes)`, the checksum used for data blocks inside flash.
pub fn data_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0x55u8, |acc, b| acc.wrapping_sub(*b))
}

pub fn pair_valid(v: u8, check: u8) -> bool {
    v.wrapping_add(check) == 0x55
}

pub fn packet(cmd: u8, address: u16, data: &[u8]) -> [u8; PACKET_LEN] {
    assert!(data.len() <= CHUNK);
    let mut p = [0u8; PACKET_LEN];
    p[0] = cmd;
    p[2] = (address >> 8) as u8;
    p[3] = address as u8;
    p[4] = data.len() as u8;
    p[5..5 + data.len()].copy_from_slice(data);
    p[15] = data_checksum(&p[..15]).wrapping_sub(REPORT_ID);
    p
}

/// Read requests carry a length but no data.
pub fn read_packet(address: u16, len: u8) -> [u8; PACKET_LEN] {
    let mut p = packet(cmd::READ_FLASH, address, &[]);
    p[4] = len;
    p[15] = data_checksum(&p[..15]).wrapping_sub(REPORT_ID);
    p
}

/// Polling rate in Hz -> flash byte: <=1000 Hz stores the divider (1000/rate),
/// faster rates store rate/2000*16.
pub fn encode_report_rate(hz: u32) -> Option<u8> {
    match hz {
        125 | 250 | 500 | 1000 => Some((1000 / hz) as u8),
        2000 | 4000 | 8000 => Some((hz / 2000 * 16) as u8),
        _ => None,
    }
}

pub fn decode_report_rate(v: u8) -> u32 {
    match v {
        0 => 1000,
        v if v >= 16 => v as u32 / 16 * 2000,
        v => 1000 / v as u32,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    Paw3950,
    Paw3955,
}

impl Sensor {
    /// cfg.json: cid 62, mid {1,2,10..13} -> 3950, everything else -> 3955.
    pub fn from_mid(mid: u8) -> Self {
        match mid {
            1 | 2 | 10..=13 => Sensor::Paw3950,
            _ => Sensor::Paw3955,
        }
    }

    pub fn max_dpi(self) -> u32 {
        match self {
            Sensor::Paw3950 => 30000,
            Sensor::Paw3955 => 50000,
        }
    }

    /// (base step, minimum of the doubled range) from sensor.json.
    fn ranges(self) -> (u32, u32) {
        match self {
            Sensor::Paw3950 => (50, 30100),
            Sensor::Paw3955 => (1, 42002),
        }
    }

    pub fn dpi_addr(self, stage: u8) -> u16 {
        match self {
            Sensor::Paw3950 => addr::DPI_VALUE + stage as u16 * 4,
            Sensor::Paw3955 => addr::SENSOR_3955_DPI + stage as u16 * 6,
        }
    }

    pub fn validate_dpi(self, dpi: u32) -> Result<(), String> {
        let (step, hi_min) = self.ranges();
        let step = if dpi >= hi_min { step * 2 } else { step };
        if dpi < step || dpi > self.max_dpi() || dpi % step != 0 {
            return Err(format!(
                "{dpi} DPI is not valid for {self:?} (multiples of {step}, max {})",
                self.max_dpi()
            ));
        }
        Ok(())
    }

    /// Flash block for one DPI stage, X and Y set to the same value (`E0` +
    /// `Set_MS_DPIValue` in the bundle). Flag byte layout:
    /// bits 0-1 X multiplier, 2-3 X high bits, 4-5 Y multiplier, 6-7 Y high bits.
    pub fn encode_dpi(self, dpi: u32) -> Vec<u8> {
        let (step, hi_min) = self.ranges();
        let (doubled, mult) = if dpi >= hi_min { (true, 2) } else { (false, 1) };
        let raw = dpi / mult / step - 1;
        let ex = if doubled { 0x11u8 } else { 0 };
        let mut out = match self {
            Sensor::Paw3950 => {
                let hi = ((raw >> 8) & 3) as u8;
                vec![raw as u8, raw as u8, (hi << 2) | (hi << 6) | ex]
            }
            Sensor::Paw3955 => {
                let hi = ((raw >> 16) & 3) as u8;
                vec![
                    raw as u8,
                    (raw >> 8) as u8,
                    raw as u8,
                    (raw >> 8) as u8,
                    (hi << 2) | (hi << 6) | ex,
                ]
            }
        };
        out.push(data_checksum(&out));
        out
    }

    /// Returns (x, y) DPI for a stage block read from flash (`Update_Mouse_Dpi`).
    pub fn decode_dpi(self, b: &[u8]) -> (u32, u32) {
        let (step, _) = self.ranges();
        let value = |raw: u32, mult: u8| {
            let mut v = (raw + 1) * step;
            if mult & 1 != 0 {
                v *= 2;
            }
            if mult & 2 != 0 {
                v *= 2;
            }
            v
        };
        match self {
            Sensor::Paw3950 => {
                let f = b[2];
                let x = b[0] as u32 | (((f >> 2) & 3) as u32) << 8;
                let y = b[1] as u32 | (((f >> 6) & 3) as u32) << 8;
                (value(x, f & 3), value(y, (f >> 4) & 3))
            }
            Sensor::Paw3955 => {
                let f = b[4];
                let x = b[0] as u32 | (b[1] as u32) << 8 | (((f >> 2) & 3) as u32) << 16;
                let y = b[2] as u32 | (b[3] as u32) << 8 | (((f >> 6) & 3) as u32) << 16;
                (value(x, f & 3), value(y, (f >> 4) & 3))
            }
        }
    }
}

/// Button actions (`Ro` in the bundle, labels from lang/en.json).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Disable,
    Left,
    Right,
    Middle,
    Back,
    Forward,
    DpiLoop,
    DpiUp,
    DpiDown,
    ScrollLeft,
    ScrollRight,
    ScrollUp,
    ScrollDown,
    ReportRateSwitch,
    /// Repeated left clicks every `interval` ms (10-255), `times` 0-3 (0 = while held).
    Fire { interval: u8, times: u8 },
    /// Combo key or media key; the keys live in the button's shortcut block.
    Shortcut,
    /// Macro stored in the button's macro block; `cycle` is a repeat count or
    /// one of the [`cycle`] modes.
    Macro { cycle: u8 },
    Other(u8, u16),
}

/// Macro repeat modes (radio values in the macro editor).
pub mod cycle {
    pub const UNTIL_PRESSED_AGAIN: u8 = 253;
    pub const WHILE_HELD: u8 = 254;
    pub const UNTIL_ANY_KEY: u8 = 255;
    pub const MAX_COUNT: u8 = 252;

    pub fn label(c: u8) -> String {
        match c {
            UNTIL_PRESSED_AGAIN => "until pressed again".into(),
            WHILE_HELD => "while held".into(),
            UNTIL_ANY_KEY => "until any key".into(),
            1 => "once".into(),
            n => format!("{n} times"),
        }
    }
}

pub const FIRE_INTERVAL: std::ops::RangeInclusive<u8> = 10..=255;
pub const FIRE_TIMES: std::ops::RangeInclusive<u8> = 0..=3;

impl KeyAction {
    /// Plain actions, in menu order.
    pub const SIMPLE: [KeyAction; 14] = [
        KeyAction::Left,
        KeyAction::Right,
        KeyAction::Middle,
        KeyAction::Back,
        KeyAction::Forward,
        KeyAction::DpiLoop,
        KeyAction::DpiUp,
        KeyAction::DpiDown,
        KeyAction::ScrollUp,
        KeyAction::ScrollDown,
        KeyAction::ScrollLeft,
        KeyAction::ScrollRight,
        KeyAction::ReportRateSwitch,
        KeyAction::Disable,
    ];
    pub const DEFAULT_FIRE: KeyAction = KeyAction::Fire { interval: 10, times: 0 };

    pub fn label(self) -> String {
        use KeyAction::*;
        match self {
            Disable => "Disable",
            Left => "Left Click",
            Right => "Right Click",
            Middle => "Wheel Click",
            Back => "Backward",
            Forward => "Forward",
            DpiLoop => "DPI Loop",
            DpiUp => "DPI +",
            DpiDown => "DPI -",
            ScrollLeft => "Scroll Left",
            ScrollRight => "Scroll Right",
            ScrollUp => "Scroll Up",
            ScrollDown => "Scroll Down",
            ReportRateSwitch => "Polling Rate Switch",
            Fire { interval, times } => {
                let n = if times == 0 { "while held".to_string() } else { format!("x{times}") };
                return format!("Fire ({interval} ms, {n})");
            }
            Shortcut => "Combo / Media key",
            Macro { cycle } => return format!("Macro ({})", cycle::label(cycle)),
            Other(t, p) => return format!("Custom ({t}, 0x{p:04X})"),
        }
        .to_string()
    }

    /// (type, param) as written to the key-function block of button `key`.
    pub fn encode(self, key: u8) -> (u8, u16) {
        use KeyAction::*;
        match self {
            Disable => (0, 0),
            Left => (1, 0x0100),
            Right => (1, 0x0200),
            Middle => (1, 0x0400),
            Back => (1, 0x0800),
            Forward => (1, 0x1000),
            DpiLoop => (2, 0x0100),
            DpiUp => (2, 0x0200),
            DpiDown => (2, 0x0300),
            ScrollLeft => (3, 0x0100),
            ScrollRight => (3, 0x0200),
            Fire { interval, times } => (4, (interval as u16) << 8 | times as u16),
            Shortcut => (5, 0),
            Macro { cycle } => (6, (key as u16) << 8 | cycle as u16),
            ReportRateSwitch => (7, 0),
            ScrollUp => (11, 0x0100),
            ScrollDown => (11, 0x0200),
            Other(t, p) => (t, p),
        }
    }

    pub fn decode(kind: u8, param: u16) -> Self {
        match kind {
            4 => KeyAction::Fire { interval: (param >> 8) as u8, times: param as u8 },
            5 => KeyAction::Shortcut,
            6 => KeyAction::Macro { cycle: param as u8 },
            _ => KeyAction::SIMPLE
                .into_iter()
                .find(|a| {
                    let (t, p) = a.encode(0);
                    // Disable/rate switch ignore the parameter.
                    t == kind && (p == param || matches!(kind, 0 | 7))
                })
                .unwrap_or(KeyAction::Other(kind, param)),
        }
    }

    /// Simple actions plus `fire[:interval[:times]]`.
    pub fn parse(s: &str) -> Option<Self> {
        use KeyAction::*;
        let l = s.to_ascii_lowercase();
        if let Some(rest) = l.strip_prefix("fire") {
            let mut it = rest.split(':').skip(1).map(str::parse::<u8>);
            let interval = it.next().transpose().ok()?.unwrap_or(10);
            let times = it.next().transpose().ok()?.unwrap_or(0);
            if !FIRE_INTERVAL.contains(&interval) || !FIRE_TIMES.contains(&times) {
                return None;
            }
            return Some(Fire { interval, times });
        }
        Some(match l.as_str() {
            "disable" | "none" => Disable,
            "left" => Left,
            "right" => Right,
            "middle" | "wheel" => Middle,
            "back" | "backward" => Back,
            "forward" => Forward,
            "dpi-loop" => DpiLoop,
            "dpi-up" => DpiUp,
            "dpi-down" => DpiDown,
            "scroll-left" => ScrollLeft,
            "scroll-right" => ScrollRight,
            "scroll-up" => ScrollUp,
            "scroll-down" => ScrollDown,
            "rate-switch" => ReportRateSwitch,
            _ => return None,
        })
    }

    pub fn flash_block(self, key: u8) -> [u8; 4] {
        let (t, p) = self.encode(key);
        let mut b = [t, (p >> 8) as u8, p as u8, 0];
        b[3] = data_checksum(&b[..3]);
        b
    }
}

/// Combo key (up to 3 keys, modifiers first) or a single media key, stored in
/// the button's 32-byte shortcut block as press events followed by releases in
/// reverse order: `[n*2, (kind|0x80, lo, hi)*n, (kind|0x40, lo, hi)*n, cs]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut(pub Vec<HidKey>);

impl Shortcut {
    pub const MAX_KEYS: usize = 3;
    pub const BLOCK: u16 = 32;

    pub fn media(usage: u16) -> Self {
        Shortcut(vec![HidKey::new(2, usage)])
    }

    pub fn is_media(&self) -> bool {
        matches!(self.0.as_slice(), [k] if k.kind == 2)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![(self.0.len() * 2) as u8];
        let presses = self.0.iter().map(|k| (0x80, k));
        let releases = self.0.iter().rev().map(|k| (0x40, k));
        for (flag, k) in presses.chain(releases) {
            b.extend([k.kind | flag, k.value as u8, (k.value >> 8) as u8]);
        }
        b.push(data_checksum(&b));
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let n = b[0] as usize / 2;
        if n == 0 || n > Self::MAX_KEYS {
            return None;
        }
        Some(Shortcut(
            (0..n)
                .map(|i| {
                    let e = &b[1 + i * 3..];
                    HidKey::new(e[0] & 15, u16::from_le_bytes([e[1], e[2]]))
                })
                .collect(),
        ))
    }

    pub fn label(&self) -> String {
        self.0.iter().map(|k| k.label()).collect::<Vec<_>>().join(" + ")
    }

    /// "ctrl+shift+s" or "media:mute"
    pub fn parse(s: &str) -> Option<Self> {
        let keys = s.split('+').map(HidKey::parse).collect::<Option<Vec<_>>>()?;
        let ok = (1..=Self::MAX_KEYS).contains(&keys.len())
            && (keys.len() == 1 || keys.iter().all(|k| k.kind != 2 && k.kind != 4));
        ok.then_some(Shortcut(keys))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroEvent {
    pub press: bool,
    pub key: HidKey,
    /// Pause after this event.
    pub delay_ms: u16,
}

/// Macro block (384 bytes per button):
/// `[name_len, name[30], count, (state<<6|kind, lo, hi, delay_hi, delay_lo)*count, cs]`
/// where state 2 = press, 1 = release.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Macro {
    pub name: String,
    pub events: Vec<MacroEvent>,
}

impl Macro {
    pub const MAX_EVENTS: usize = 70;
    pub const MAX_NAME: usize = 30;
    pub const BLOCK: u16 = 384;

    pub fn encode(&self) -> Vec<u8> {
        let mut name = self.name.as_str();
        while name.len() > Self::MAX_NAME {
            let mut cut = name.len() - 1;
            while !name.is_char_boundary(cut) {
                cut -= 1;
            }
            name = &name[..cut];
        }
        let events = &self.events[..self.events.len().min(Self::MAX_EVENTS)];
        let mut b = vec![0xFF; 32];
        b[0] = name.len() as u8;
        b[1..1 + name.len()].copy_from_slice(name.as_bytes());
        b[31] = events.len() as u8;
        let start = b.len();
        for e in events {
            let state = if e.press { 2 } else { 1 };
            b.extend([
                state << 6 | (e.key.kind & 15),
                e.key.value as u8,
                (e.key.value >> 8) as u8,
                (e.delay_ms >> 8) as u8,
                e.delay_ms as u8,
            ]);
        }
        let cs = data_checksum(&b[start..]).wrapping_sub(events.len() as u8);
        b.push(cs);
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        let (len, n) = (b[0] as usize, b[31] as usize);
        if !(1..=Self::MAX_NAME).contains(&len) || n > Self::MAX_EVENTS || b.len() < 32 + n * 5 {
            return None;
        }
        let name = String::from_utf8_lossy(&b[1..1 + len]).into_owned();
        let events = b[32..32 + n * 5]
            .chunks(5)
            .map(|e| MacroEvent {
                press: e[0] >> 6 == 2,
                key: HidKey::new(e[0] & 15, u16::from_le_bytes([e[1], e[2]])),
                delay_ms: u16::from_be_bytes([e[3], e[4]]),
            })
            .collect();
        Some(Macro { name, events })
    }
}

/// `DongleEnterPair`: the receiver listens for a mouse with this customer id.
pub fn pair_packet(cid: u8) -> [u8; PACKET_LEN] {
    let mut p = packet(cmd::DONGLE_ENTER_PAIR, 0, &[0, 0, cid]);
    p[4] = 2;
    p[15] = data_checksum(&p[..15]).wrapping_sub(REPORT_ID);
    p
}

/// Status codes from `GetPairState`.
pub mod pair {
    pub const PAIRING: u8 = 1;
    pub const FAILED: u8 = 2;
    pub const SUCCESS: u8 = 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_checksum_includes_report_id() {
        let p = packet(cmd::WRITE_FLASH, 169, &[8, 0x55 - 8]);
        let sum = p.iter().fold(REPORT_ID, |a, b| a.wrapping_add(*b));
        assert_eq!(sum, 0x55);
    }

    #[test]
    fn report_rate_roundtrip() {
        for hz in [125, 250, 500, 1000, 2000, 4000, 8000] {
            assert_eq!(decode_report_rate(encode_report_rate(hz).unwrap()), hz);
        }
        assert_eq!(encode_report_rate(4000), Some(32));
    }

    #[test]
    fn dpi_roundtrip() {
        for (s, dpis) in [
            (Sensor::Paw3950, &[50u32, 800, 1600, 12750, 26000, 30000][..]),
            (Sensor::Paw3955, &[1, 400, 800, 26000, 42000, 42002, 50000][..]),
        ] {
            for &d in dpis {
                s.validate_dpi(d).unwrap();
                let b = s.encode_dpi(d);
                assert_eq!(s.decode_dpi(&b), (d, d), "{s:?} {d}");
                assert_eq!(data_checksum(&b[..b.len() - 1]), *b.last().unwrap());
            }
        }
        assert!(Sensor::Paw3950.validate_dpi(825).is_err());
        assert!(Sensor::Paw3950.validate_dpi(40000).is_err());
    }

    #[test]
    fn key_roundtrip() {
        for a in [
            KeyAction::Left,
            KeyAction::DpiDown,
            KeyAction::ScrollUp,
            KeyAction::Fire { interval: 50, times: 2 },
            KeyAction::Shortcut,
            KeyAction::Macro { cycle: cycle::WHILE_HELD },
        ] {
            let (t, p) = a.encode(3);
            assert_eq!(KeyAction::decode(t, p), a);
        }
        // Macro param carries the button index in the high byte.
        assert_eq!(KeyAction::Macro { cycle: 1 }.encode(3), (6, 0x0301));
    }

    #[test]
    fn key_blocks_match_real_flash() {
        // Bytes read from a Scyrox mouse (key-function area).
        assert_eq!(KeyAction::Left.flash_block(0), [0x01, 0x01, 0x00, 0x53]);
        assert_eq!(KeyAction::DpiLoop.flash_block(5), [0x02, 0x01, 0x00, 0x52]);
        assert_eq!(
            KeyAction::decode(0x04, 0x0a03),
            KeyAction::Fire { interval: 10, times: 3 }
        );
        assert_eq!(KeyAction::Fire { interval: 10, times: 3 }.flash_block(10), [0x04, 0x0a, 0x03, 0x44]);
    }

    #[test]
    fn shortcut_encoding() {
        // Set_MS_Multimedia: [2, 0x82, lo, hi, 0x42, lo, hi, cs]
        let b = Shortcut::media(0x00E2).encode();
        assert_eq!(&b[..7], &[2, 0x82, 0xE2, 0, 0x42, 0xE2, 0]);
        assert_eq!(b.iter().fold(0u8, |a, x| a.wrapping_add(*x)), 0x55);

        let combo = Shortcut::parse("ctrl+shift+s").unwrap();
        let b = combo.encode();
        assert_eq!(b[0], 6);
        assert_eq!(&b[1..4], &[0x80, 1, 0]); // LCtrl press
        assert_eq!(&b[16..19], &[0x40, 1, 0]); // LCtrl released last
        assert_eq!(Shortcut::decode(&b), Some(combo));
        assert!(Shortcut::parse("ctrl+media:mute").is_none());
    }

    #[test]
    fn macro_roundtrip() {
        let a = HidKey::parse("a").unwrap();
        let m = Macro {
            name: "test".into(),
            events: vec![
                MacroEvent { press: true, key: a, delay_ms: 30 },
                MacroEvent { press: false, key: a, delay_ms: 300 },
            ],
        };
        let b = m.encode();
        assert_eq!(b.len(), 33 + 2 * 5);
        assert_eq!(b[32], 0x81); // press, keyboard
        assert_eq!(b[37], 0x41); // release
        // Checksum: 0x55 - sum(events) - count
        let sum = b[32..42].iter().fold(0u8, |s, x| s.wrapping_add(*x));
        assert_eq!(b[42], 0x55u8.wrapping_sub(sum).wrapping_sub(2));
        assert_eq!(Macro::decode(&b), Some(m));
    }
}
