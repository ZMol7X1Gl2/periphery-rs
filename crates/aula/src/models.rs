//! Models of the `258A:010C` family and the lighting effects they offer.
//!
//! Identities and option lists come from each model's `Dev/kb/*/KB.ini` in
//! the vendor's driver package (`Psd=`, `Light=`, `SleepUI=`/`SleepHW=`,
//! `LedOptN=`).

use hid_transport::Filter;

pub const VENDOR_ID: u16 = 0x258A;
pub const PRODUCT_ID: u16 = 0x010C;
pub const USAGE_PAGE: u16 = 0xFF00;
pub const USAGE: u16 = 0x01;

pub const FILTER: Filter =
    Filter { vendor_id: Some(VENDOR_ID), product_id: Some(PRODUCT_ID), usage_page: Some(USAGE_PAGE), usage: Some(USAGE) };

#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub name: &'static str,
    /// The identity the keyboard reports (`Psd=` in `KB.ini`).
    pub identity: [u8; 6],
    /// Highest brightness level (`Light=0..n`).
    pub max_brightness: u8,
    /// Whether the driver offers the sleep timer (`SleepUI=` present).
    pub sleep: bool,
}

pub const MODELS: &[Model] = &[
    Model { name: "F75", identity: [3, 0, 0, 0, 0x00, 0xCD], max_brightness: 9, sleep: true },
    Model { name: "F87", identity: [3, 0, 0, 0, 0x00, 0x8F], max_brightness: 4, sleep: false },
    Model { name: "F87 PRO", identity: [3, 0, 0, 0, 0x01, 0x0B], max_brightness: 4, sleep: false },
    Model { name: "F87 PRO Wired", identity: [3, 0, 0, 0, 0x00, 0xA9], max_brightness: 4, sleep: false },
    Model { name: "F99", identity: [3, 0, 0, 0, 0x00, 0xA4], max_brightness: 4, sleep: true },
    Model { name: "F99 Pro", identity: [3, 0, 0, 0, 0x01, 0x58], max_brightness: 4, sleep: true },
    Model { name: "F99 Wired", identity: [3, 0, 0, 0, 0x00, 0x9A], max_brightness: 4, sleep: true },
    Model { name: "H108", identity: [3, 0, 0, 0, 0x03, 0x7C], max_brightness: 4, sleep: true },
];

pub fn model(identity: &[u8; 6]) -> Option<&'static Model> {
    MODELS.iter().find(|m| &m.identity == identity)
}

/// Sleep timer choices: (stored value, seconds). The stored unit is 30 s
/// (`SleepUI=` / `SleepHW=`).
pub const SLEEP: &[(u8, u32)] =
    &[(2, 60), (3, 90), (4, 120), (6, 180), (8, 240), (10, 300), (20, 600), (30, 900), (40, 1200)];

/// One hardware lighting effect (`LedOptN=hw,effect,speed,light,direct,random,color`).
#[derive(Debug, PartialEq, Eq)]
pub struct Effect {
    /// The value stored in the lighting page.
    pub value: u8,
    pub name: &'static str,
    pub speed: bool,
    pub brightness: bool,
    /// Offers the seven fixed colours.
    pub color: bool,
    /// Offers "colourful" (the firmware cycles colours).
    pub colorful: bool,
}

const fn fx(value: u8, name: &'static str, speed: bool, color: bool) -> Effect {
    Effect { value, name, speed, brightness: true, color, colorful: color }
}

/// The same list for every model of the family. Names follow the vendor
/// app's English strings.
pub const EFFECTS: &[Effect] = &[
    fx(1, "Fixed on", false, true),
    fx(2, "Respire", true, true),
    fx(3, "Rainbow", true, false),
    fx(4, "Flash away", true, true),
    fx(5, "Raindrops", true, true),
    fx(6, "Rainbow wheel", true, true),
    fx(7, "Ripples shining", true, true),
    fx(8, "Stars twinkle", true, true),
    fx(9, "Shadow disappear", true, true),
    fx(10, "Retro snake", true, true),
    fx(11, "Neon stream", true, true),
    fx(12, "Reaction", true, true),
    fx(13, "Sine wave", true, true),
    fx(14, "Retinue scanning", true, true),
    fx(15, "Rotating windmill", true, false),
    fx(16, "Colorful waterfall", true, false),
    fx(17, "Blossoming", true, false),
    fx(18, "Rotating storm", true, true),
    Effect { value: 21, name: "Custom (per-key colours)", speed: false, brightness: false, color: false, colorful: false },
];

pub fn effect(value: u8) -> Option<&'static Effect> {
    EFFECTS.iter().find(|e| e.value == value)
}

/// The fixed colours, by their stored value; 7 is [`COLORFUL`].
pub const COLORS: [&str; 7] = ["Red", "Green", "Blue", "Yellow", "Magenta", "Cyan", "White"];
pub const COLORFUL: u8 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_unique() {
        for (i, a) in MODELS.iter().enumerate() {
            assert!(MODELS[i + 1..].iter().all(|b| a.identity != b.identity), "{}", a.name);
        }
        assert_eq!(model(&[3, 0, 0, 0, 0, 0xCD]).unwrap().name, "F75");
    }
}
