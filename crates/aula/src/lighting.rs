//! The lighting / settings page (commands `04` / `84`): 128 bytes ending in
//! `5A A5`.
//!
//! | offset | meaning                                                    |
//! |--------|------------------------------------------------------------|
//! | `0x0A` | live effect ([`crate::models::EFFECTS`] value)             |
//! | `0x18` | sleep timer, in 30 s units                                 |
//! | `0x3A` | per-effect table: 2 bytes for effect `n` at `0x3A + 2(n-1)`|
//! | `0x7E` | `5A A5` end marker                                         |
//!
//! Each table entry is a brightness byte followed by `speed << 4 | colour`
//! (colour 0-6 = fixed colours, 7 = colourful). The keyboard remembers the
//! settings of every effect separately.
//!
//! The effect, sleep and table offsets were confirmed on an F75 by
//! AulaSofwareLinux: Fn+Tab (next colour) changed only byte `0x3B` while
//! "Fixed on" (1) was live. Everything else in the page is kept as read.

use anyhow::{Result, bail};

pub const PAGE_LEN: usize = 128;
const EFFECT: usize = 0x0A;
const SLEEP: usize = 0x18;
const TABLE: usize = 0x3A;
const MARKER: [u8; 2] = [0x5A, 0xA5];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectSettings {
    pub brightness: u8,
    pub speed: u8,
    pub color: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightingPage {
    bytes: Vec<u8>,
}

impl LightingPage {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let page = LightingPage { bytes };
        page.check()?;
        Ok(page)
    }

    /// A page without its end marker is a failed read; never write one back.
    pub fn check(&self) -> Result<()> {
        if self.bytes.len() != PAGE_LEN || self.bytes[PAGE_LEN - 2..] != MARKER {
            bail!("the keyboard sent a malformed lighting page");
        }
        Ok(())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Bytes that differ from `old`, as `offset: old -> new`.
    pub fn diff(&self, old: &LightingPage) -> String {
        let changes: Vec<String> = (0..PAGE_LEN)
            .filter(|&i| self.bytes[i] != old.bytes[i])
            .map(|i| format!("{i:#04x}: {:02x} -> {:02x}", old.bytes[i], self.bytes[i]))
            .collect();
        if changes.is_empty() { "no changes".into() } else { changes.join(", ") }
    }

    pub fn effect(&self) -> u8 {
        self.bytes[EFFECT]
    }

    pub fn set_effect(&mut self, v: u8) {
        self.bytes[EFFECT] = v;
    }

    pub fn sleep(&self) -> u8 {
        self.bytes[SLEEP]
    }

    pub fn set_sleep(&mut self, v: u8) {
        self.bytes[SLEEP] = v;
    }

    fn entry(effect: u8) -> Result<usize> {
        let at = TABLE + 2 * (effect as usize).saturating_sub(1);
        if effect == 0 || at + 2 > PAGE_LEN - 2 {
            bail!("effect {effect} has no settings slot");
        }
        Ok(at)
    }

    pub fn settings(&self, effect: u8) -> Result<EffectSettings> {
        let at = Self::entry(effect)?;
        let packed = self.bytes[at + 1];
        Ok(EffectSettings { brightness: self.bytes[at], speed: packed >> 4, color: packed & 0x0F })
    }

    pub fn set_settings(&mut self, effect: u8, s: EffectSettings) -> Result<()> {
        let at = Self::entry(effect)?;
        if s.speed > 0x0F || s.color > 0x0F {
            bail!("speed and colour must fit four bits");
        }
        self.bytes[at] = s.brightness;
        self.bytes[at + 1] = (s.speed << 4) | s.color;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An F75's page as read by AulaSofwareLinux (`aula dump`).
    const F75: [u8; PAGE_LEN] = [
        0x00, 0x03, 0x03, 0x01, 0x00, 0x00, 0x04, 0x04, 0x07, 0x00, 0x01, 0x20, 0x01, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x02, 0x00, 0x02, 0x01, 0x00, 0xff, 0x0a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x01, //
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x09, 0x40, 0x09, 0x47, 0x09, 0x47, //
        0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, //
        0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x47, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, 0x09, 0x37, //
        0x07, 0x47, 0x07, 0x47, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, 0x07, 0x44, //
        0x07, 0x44, 0x04, 0x09, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x5a, 0xa5,
    ];

    fn page() -> LightingPage {
        LightingPage::new(F75.to_vec()).unwrap()
    }

    #[test]
    fn reads_an_f75_page() {
        let p = page();
        assert_eq!(p.effect(), 1, "Fixed on");
        assert_eq!(p.sleep(), 10, "300 s");
        assert_eq!(p.settings(1).unwrap(), EffectSettings { brightness: 9, speed: 4, color: 0 });
        assert_eq!(p.settings(2).unwrap(), EffectSettings { brightness: 9, speed: 4, color: 7 });
        assert_eq!(p.settings(18).unwrap(), EffectSettings { brightness: 9, speed: 3, color: 7 });
    }

    #[test]
    fn fn_tab_moves_one_byte() {
        // Fn+Tab on the F75 changed byte 0x3B from 40 to 41 (red → green).
        let mut p = page();
        let s = p.settings(1).unwrap();
        p.set_settings(1, EffectSettings { color: 1, ..s }).unwrap();
        let diff: Vec<usize> = (0..PAGE_LEN).filter(|&i| p.bytes()[i] != F75[i]).collect();
        assert_eq!(diff, [0x3B]);
        assert_eq!(p.bytes()[0x3B], 0x41);
    }

    #[test]
    fn rejects_pages_without_the_marker() {
        let mut bad = F75.to_vec();
        bad[127] = 0;
        assert!(LightingPage::new(bad).is_err());
        assert!(LightingPage::new(vec![0; 64]).is_err());
    }

    #[test]
    fn table_stops_before_the_marker() {
        let mut p = page();
        assert!(p.settings(21).is_ok());
        assert!(p.settings(0).is_err());
        assert!(p.set_settings(34, EffectSettings { brightness: 1, speed: 0, color: 0 }).is_ok());
        assert!(p.set_settings(35, EffectSettings { brightness: 1, speed: 0, color: 0 }).is_err());
        assert_eq!(p.bytes()[126..], [0x5A, 0xA5]);
    }
}
