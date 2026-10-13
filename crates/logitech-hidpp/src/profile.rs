//! Onboard profile sector layout (profile formats 1-5):
//!
//! | offset | field |
//! |---|---|
//! | 0 | report interval in ms |
//! | 1 | default DPI stage |
//! | 2 | shift ("sniper") DPI stage |
//! | 3..13 | 5 DPI values, u16 little-endian, 0 = unused |
//! | 32.. | button bindings, 4 bytes each |
//! | 96.. | G-shift bindings |
//! | 160..208 | name, UTF-16LE, 24 characters |
//! | last 2 | CRC-16/CCITT (init 0xFFFF) of the rest, big-endian |
//!
//! Every other byte is kept as read, so encoding only touches edited fields.

use anyhow::{Result, bail};

pub const DPI_STAGES: usize = 5;
const OFF_RATE: usize = 0;
const OFF_DEFAULT_DPI: usize = 1;
const OFF_SHIFT_DPI: usize = 2;
const OFF_DPI: usize = 3;
const OFF_BUTTONS: usize = 32;
const OFF_NAME: usize = 160;
pub const NAME_LEN: usize = 24;

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for b in data {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Special functions (type 0x90), codes as documented by libratbag.
pub mod special {
    pub const WHEEL_LEFT: u8 = 0x01;
    pub const WHEEL_RIGHT: u8 = 0x02;
    pub const DPI_UP: u8 = 0x03;
    pub const DPI_DOWN: u8 = 0x04;
    pub const DPI_CYCLE: u8 = 0x05;
    pub const DPI_DEFAULT: u8 = 0x06;
    pub const DPI_SHIFT: u8 = 0x07;
    pub const PROFILE_CYCLE: u8 = 0x08;
    pub const PROFILE_DOWN: u8 = 0x09;
    pub const PROFILE_UP: u8 = 0x0A;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Mouse buttons as a bitmask: 1 left, 2 right, 4 middle, 8 back, 16 forward.
    Mouse(u16),
    /// Keyboard key: modifier bitmask and HID usage.
    Key { modifiers: u8, key: u8 },
    /// Consumer-page usage (media keys).
    Consumer(u16),
    Special(u8),
    Disabled,
    Raw([u8; 4]),
}

impl Binding {
    pub fn decode(b: [u8; 4]) -> Binding {
        match b {
            [0x80, 0x01, hi, lo] => Binding::Mouse(u16::from_be_bytes([hi, lo])),
            [0x80, 0x02, m, k] => Binding::Key { modifiers: m, key: k },
            [0x80, 0x03, hi, lo] => Binding::Consumer(u16::from_be_bytes([hi, lo])),
            [0x90, code, _, _] => Binding::Special(code),
            [0xFF, ..] => Binding::Disabled,
            raw => Binding::Raw(raw),
        }
    }

    pub fn encode(self) -> [u8; 4] {
        match self {
            Binding::Mouse(m) => {
                let [hi, lo] = m.to_be_bytes();
                [0x80, 0x01, hi, lo]
            }
            Binding::Key { modifiers, key } => [0x80, 0x02, modifiers, key],
            Binding::Consumer(u) => {
                let [hi, lo] = u.to_be_bytes();
                [0x80, 0x03, hi, lo]
            }
            Binding::Special(c) => [0x90, c, 0, 0],
            Binding::Disabled => [0xFF; 4],
            Binding::Raw(r) => r,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    raw: Vec<u8>,
    buttons: usize,
}

impl Profile {
    /// Parses a sector; fails on a bad CRC.
    pub fn parse(raw: Vec<u8>, buttons: u8) -> Result<Profile> {
        if raw.len() < OFF_NAME + NAME_LEN * 2 + 2 {
            bail!("profile sector too small ({} bytes)", raw.len());
        }
        let n = raw.len();
        let stored = u16::from_be_bytes([raw[n - 2], raw[n - 1]]);
        if crc16(&raw[..n - 2]) != stored {
            bail!("profile CRC mismatch");
        }
        Ok(Profile { raw, buttons: buttons.min(16) as usize })
    }

    /// Builds a profile from a factory ROM copy, which carries no valid CRC.
    pub fn from_rom(mut raw: Vec<u8>, buttons: u8) -> Result<Profile> {
        let n = raw.len();
        let crc = crc16(&raw[..n - 2]).to_be_bytes();
        raw[n - 2..].copy_from_slice(&crc);
        Profile::parse(raw, buttons)
    }

    /// Sector bytes with a fresh CRC.
    pub fn encode(&self) -> Vec<u8> {
        let mut raw = self.raw.clone();
        let n = raw.len();
        let crc = crc16(&raw[..n - 2]).to_be_bytes();
        raw[n - 2..].copy_from_slice(&crc);
        raw
    }

    pub fn report_interval(&self) -> u8 {
        self.raw[OFF_RATE]
    }

    pub fn set_report_interval(&mut self, ms: u8) {
        self.raw[OFF_RATE] = ms;
    }

    pub fn default_dpi_stage(&self) -> u8 {
        self.raw[OFF_DEFAULT_DPI]
    }

    pub fn set_default_dpi_stage(&mut self, i: u8) {
        self.raw[OFF_DEFAULT_DPI] = i;
    }

    pub fn shift_dpi_stage(&self) -> u8 {
        self.raw[OFF_SHIFT_DPI]
    }

    pub fn set_shift_dpi_stage(&mut self, i: u8) {
        self.raw[OFF_SHIFT_DPI] = i;
    }

    /// DPI per stage; 0 = stage unused.
    pub fn dpi(&self, stage: usize) -> u16 {
        let o = OFF_DPI + stage * 2;
        u16::from_le_bytes([self.raw[o], self.raw[o + 1]])
    }

    pub fn set_dpi(&mut self, stage: usize, dpi: u16) {
        let o = OFF_DPI + stage * 2;
        self.raw[o..o + 2].copy_from_slice(&dpi.to_le_bytes());
    }

    pub fn button_count(&self) -> usize {
        self.buttons
    }

    pub fn button(&self, i: usize) -> Binding {
        let o = OFF_BUTTONS + i * 4;
        Binding::decode(self.raw[o..o + 4].try_into().unwrap())
    }

    pub fn set_button(&mut self, i: usize, b: Binding) {
        let o = OFF_BUTTONS + i * 4;
        self.raw[o..o + 4].copy_from_slice(&b.encode());
    }

    pub fn name(&self) -> String {
        let units: Vec<u16> = self.raw[OFF_NAME..OFF_NAME + NAME_LEN * 2]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0 && *u != 0xFFFF)
            .collect();
        String::from_utf16_lossy(&units)
    }

    pub fn set_name(&mut self, name: &str) {
        let mut units: Vec<u16> = name.encode_utf16().take(NAME_LEN).collect();
        units.resize(NAME_LEN, 0);
        for (i, u) in units.iter().enumerate() {
            let o = OFF_NAME + i * 2;
            self.raw[o..o + 2].copy_from_slice(&u.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Profile sector 1 as read from a PRO X Superlight.
    fn superlight_profile() -> Vec<u8> {
        let mut s = vec![0xFFu8; 255];
        let head = [0x01, 0x01, 0x00, 0x90, 0x01, 0x58, 0x02, 0x40, 0x06, 0x80, 0x0c, 0x00, 0x19];
        s[..13].copy_from_slice(&head);
        s[17] = 0x00;
        let buttons = [0x80, 0x01, 0x00, 0x01, 0x80, 0x01, 0x00, 0x02, 0x80, 0x01, 0x00, 0x04, 0x80, 0x01, 0x00, 0x08, 0x80, 0x01, 0x00, 0x10];
        s[0x20..0x34].copy_from_slice(&buttons);
        s[0x60..0x74].copy_from_slice(&buttons);
        let name: Vec<u8> = "PROFILE_NAME_DEFAULT".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        s[0xa0..0xa0 + name.len()].copy_from_slice(&name);
        s[0xa0 + name.len()..0xd0].fill(0);
        let tail = [0, 0, 0, 0, 0, 0, 0x1f, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x1f, 0x40, 0, 0, 0];
        s[0xd0..0xd0 + tail.len()].copy_from_slice(&tail);
        s[253] = 0x4a;
        s[254] = 0x3a;
        s
    }

    #[test]
    fn parses_real_profile() {
        let p = Profile::parse(superlight_profile(), 5).unwrap();
        assert_eq!(p.report_interval(), 1);
        assert_eq!(p.default_dpi_stage(), 1);
        assert_eq!((0..5).map(|i| p.dpi(i)).collect::<Vec<_>>(), [400, 600, 1600, 3200, 6400]);
        assert_eq!(p.button(0), Binding::Mouse(1));
        assert_eq!(p.button(4), Binding::Mouse(0x10));
        assert_eq!(p.name(), "PROFILE_NAME_DEFAULT");
        // Unchanged profile re-encodes byte for byte (CRC 0x4a3a).
        assert_eq!(p.encode(), superlight_profile());
    }

    #[test]
    fn edits_keep_crc_valid() {
        let mut p = Profile::parse(superlight_profile(), 5).unwrap();
        p.set_dpi(1, 800);
        p.set_button(3, Binding::Special(special::DPI_CYCLE));
        p.set_name("Gaming");
        let bytes = p.encode();
        let q = Profile::parse(bytes, 5).unwrap();
        assert_eq!(q.dpi(1), 800);
        assert_eq!(q.button(3), Binding::Special(special::DPI_CYCLE));
        assert_eq!(q.name(), "Gaming");
        assert!(Profile::parse(superlight_profile().into_iter().map(|b| b ^ 1).collect(), 5).is_err());
    }
}
