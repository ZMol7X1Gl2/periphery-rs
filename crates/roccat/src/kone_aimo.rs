//! ROCCAT Kone Aimo (`1E7D:2E27`) and Kone Aimo Remastered / 16K
//! (`1E7D:2E2C`).
//!
//! Everything is a numbered feature report on interface 0 (on Windows the
//! collection with usage page `0x0B`). Every report starts with its id; the
//! long ones carry their total length in byte 1 and the profile in byte 2.
//!
//! | id   | size | use                                                      |
//! |------|------|----------------------------------------------------------|
//! | `04` | 3    | write `04 profile what` to pick what the next read of `06`/`07` returns (`80` settings, `90` buttons); read: byte 1 is the status (`03` busy) |
//! | `05` | 3    | active profile: read byte 2; write `05 03 profile`       |
//! | `06` | 126  | settings of one profile, see [`Settings`]                |
//! | `07` | 75   | buttons of one profile, see [`Buttons`]                  |
//! | `0E` | 6    | `0E id 01 00 00 FF` makes the mouse use report `id` just written |
//!
//! A write is: select with `04`, wait until not busy, write the report, wait,
//! activate with `0E`, wait. Reports `08` (macros) and `0D` (live LED colours)
//! exist but aren't used here.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use hid_transport::{Api, Device, DeviceInfo, Filter, sleep};

pub const VENDOR_ID: u16 = 0x1E7D;
pub const USAGE_PAGE: u16 = 0x0B;

pub const PROFILES: u8 = 5;
pub const DPI_STAGES: usize = 5;
pub const DPI_STEP: u16 = 50;
/// Polling rates by their index in the settings report.
pub const REPORT_RATES: [u16; 4] = [125, 250, 500, 1000];

#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub product_id: u16,
    pub name: &'static str,
    pub min_dpi: u16,
    pub max_dpi: u16,
}

pub const MODELS: [Model; 2] = [
    Model { product_id: 0x2E27, name: "Kone Aimo", min_dpi: 100, max_dpi: 12000 },
    Model { product_id: 0x2E2C, name: "Kone Aimo Remastered", min_dpi: 100, max_dpi: 16000 },
];

pub fn model(product_id: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.product_id == product_id)
}

mod report {
    pub const CONTROL: u8 = 0x04;
    pub const PROFILE: u8 = 0x05;
    pub const SETTINGS: u8 = 0x06;
    pub const BUTTONS: u8 = 0x07;
    pub const ACTIVATE: u8 = 0x0E;

    /// `04 profile SELECT_*`
    pub const SELECT_SETTINGS: u8 = 0x80;
    pub const SELECT_BUTTONS: u8 = 0x90;
    /// Byte 1 of report `04` while the mouse is still working.
    pub const BUSY: u8 = 0x03;
}

/// Big enough for any feature report on the interface (the largest, `1A`,
/// is 1029 bytes). Windows refuses reads into a buffer shorter than that.
const LARGE_READ: usize = 1100;
const READY_POLLS: usize = 20;
const READY_DELAY: Duration = Duration::from_millis(50);

pub struct KoneAimo {
    dev: Device,
    pub model: &'static Model,
}

impl KoneAimo {
    pub fn filters() -> Vec<Filter> {
        MODELS
            .iter()
            .map(|m| Filter {
                vendor_id: Some(VENDOR_ID),
                product_id: Some(m.product_id),
                usage_page: Some(USAGE_PAGE),
                usage: None,
            })
            .collect()
    }

    /// Every listed HID collection of a Kone Aimo, whichever interface.
    pub fn listings(api: &Api) -> impl Iterator<Item = &DeviceInfo> {
        api.devices().filter(|d| d.vendor_id == VENDOR_ID && model(d.product_id).is_some())
    }

    /// The control interface of every Kone Aimo in `api`'s enumeration.
    ///
    /// In the browser every listing of a device opens the same device and
    /// the report id picks the collection, so when the browser doesn't list
    /// the usage page `0x0B` collection, any listing of the mouse will do.
    pub fn candidates(api: &Api) -> Vec<&DeviceInfo> {
        let exact: Vec<_> = Self::listings(api)
            .filter(|d| Self::filters().iter().any(|f| f.matches(d)) && d.interface_number <= 0)
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        let mut out: Vec<&DeviceInfo> = Vec::new();
        for d in Self::listings(api).filter(|d| d.interface_number < 0) {
            if !out.iter().any(|o| o.product_id == d.product_id) {
                out.push(d);
            }
        }
        out
    }

    pub async fn open(api: &Api, info: &DeviceInfo) -> Result<Self> {
        let Some(model) = model(info.product_id) else { bail!("not a Kone Aimo") };
        let dev = api.open(info).await.context("open HID device")?;
        Ok(KoneAimo { dev, model })
    }

    /// Reads feature report `id` (`len` bytes including the id).
    async fn get(&self, id: u8, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        buf[0] = id;
        let n = match self.dev.get_feature_report(&mut buf).await {
            Ok(n) => n,
            Err(_) => {
                buf = vec![0u8; LARGE_READ];
                buf[0] = id;
                self.dev.get_feature_report(&mut buf).await.with_context(|| format!("read report {id:#04x}"))?
            }
        };
        if n < len || buf[0] != id {
            bail!("short or wrong reply to report {id:#04x} ({n} bytes)");
        }
        buf.truncate(len);
        Ok(buf)
    }

    async fn put(&self, data: &[u8]) -> Result<()> {
        self.dev.send_feature_report(data).await.with_context(|| format!("write report {:#04x}", data[0]))
    }

    /// Status byte of report `04` (`03` = busy).
    pub async fn status(&self) -> Result<u8> {
        Ok(self.get(report::CONTROL, 3).await?[1])
    }

    async fn wait_ready(&self) -> Result<()> {
        for _ in 0..READY_POLLS {
            sleep(READY_DELAY).await;
            if self.status().await? != report::BUSY {
                return Ok(());
            }
        }
        bail!("the mouse stayed busy")
    }

    async fn select(&self, profile: u8, what: u8) -> Result<()> {
        if profile >= PROFILES {
            bail!("no profile {profile}");
        }
        self.put(&[report::CONTROL, profile, what]).await?;
        self.wait_ready().await
    }

    /// Writes a settings or button report and makes the mouse use it.
    async fn commit(&self, data: &[u8]) -> Result<()> {
        self.put(data).await?;
        self.wait_ready().await?;
        self.put(&[report::ACTIVATE, data[0], 0x01, 0x00, 0x00, 0xFF]).await?;
        self.wait_ready().await
    }

    /// The profile the mouse uses (0-based).
    pub async fn active_profile(&self) -> Result<u8> {
        let p = self.get(report::PROFILE, 3).await?[2];
        if p >= PROFILES {
            bail!("the mouse reports profile {p}");
        }
        Ok(p)
    }

    pub async fn set_active_profile(&self, profile: u8) -> Result<()> {
        if profile >= PROFILES {
            bail!("no profile {profile}");
        }
        self.put(&[report::PROFILE, 0x03, profile]).await?;
        self.wait_ready().await
    }

    pub async fn settings(&self, profile: u8) -> Result<Settings> {
        self.select(profile, report::SELECT_SETTINGS).await?;
        let raw = self.get(report::SETTINGS, Settings::LEN).await?;
        let s = Settings { raw: raw.try_into().unwrap() };
        if s.profile() != profile {
            bail!("asked for profile {profile}'s settings, got profile {}'s", s.profile());
        }
        Ok(s)
    }

    /// Writes `s` (to the profile it names), then reads it back to check.
    pub async fn set_settings(&self, s: &Settings) -> Result<()> {
        let mut s = s.clone();
        s.update_checksum();
        self.select(s.profile(), report::SELECT_SETTINGS).await?;
        self.commit(&s.raw).await?;
        if self.settings(s.profile()).await?.raw != s.raw {
            bail!("the mouse did not keep the new settings");
        }
        Ok(())
    }

    pub async fn buttons(&self, profile: u8) -> Result<Buttons> {
        self.select(profile, report::SELECT_BUTTONS).await?;
        let raw = self.get(report::BUTTONS, Buttons::LEN).await?;
        Ok(Buttons { raw: raw.try_into().unwrap() })
    }

    /// Writes `b` to `profile`, then reads it back to check.
    pub async fn set_buttons(&self, profile: u8, b: &Buttons) -> Result<()> {
        let mut b = b.clone();
        b.raw[0] = report::BUTTONS;
        b.raw[1] = Buttons::LEN as u8;
        b.raw[2] = profile;
        self.select(profile, report::SELECT_BUTTONS).await?;
        self.commit(&b.raw).await?;
        if self.buttons(profile).await?.raw[3..] != b.raw[3..] {
            bail!("the mouse did not keep the new button assignment");
        }
        Ok(())
    }
}

/// Lighting effects (settings byte 33).
pub const EFFECTS: [(u8, &str); 6] =
    [(0, "Off"), (1, "Static"), (2, "Blinking"), (3, "Breathing"), (4, "Heartbeat"), (7, "Wave")];
/// Effect speeds (settings byte 34).
pub const SPEEDS: [(u8, &str); 3] = [(1, "Slow"), (2, "Medium"), (3, "Fast")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Wheel,
    Left,
    Right,
    Bottom,
}

impl Zone {
    pub const ALL: [Zone; 4] = [Zone::Wheel, Zone::Left, Zone::Right, Zone::Bottom];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Wheel => "Scroll wheel",
            Zone::Left => "Left strip",
            Zone::Right => "Right strip",
            Zone::Bottom => "Underglow",
        }
    }

    /// Offsets of this zone's LED slots in the settings report.
    fn slots(self) -> &'static [usize] {
        match self {
            Zone::Wheel => &[61],
            Zone::Left => &[66, 71, 76, 81],
            Zone::Right => &[86, 91, 96, 101],
            Zone::Bottom => &[106, 111],
        }
    }
}

/// Report `06`: one profile's settings.
///
/// | offset  | size | field                                              |
/// |---------|------|----------------------------------------------------|
/// | 0       | 1    | `06`                                               |
/// | 1       | 1    | length (126)                                       |
/// | 2       | 1    | profile                                            |
/// | 3       | 1    | sensitivity (unused here)                          |
/// | 4       | 1    | X/Y flags (unused here)                            |
/// | 5       | 1    | active DPI stage, 0–4                              |
/// | 6       | 10   | X DPI of the 5 stages, u16 LE, units of 50 DPI     |
/// | 16      | 10   | Y DPI, same                                        |
/// | 26      | 1    | polling rate index into [`REPORT_RATES`]           |
/// | 33      | 1    | lighting effect, see [`EFFECTS`]                   |
/// | 34      | 1    | effect speed, see [`SPEEDS`]                       |
/// | 35      | 1    | brightness, 0–255                                  |
/// | 61      | 55   | 11 LED slots of `intensity R G B 00`: wheel, left strip ×4, right strip ×4, underglow ×2 |
/// | 124     | 2    | checksum: u16 LE sum of bytes 0–123                |
///
/// Other bytes are kept as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub raw: [u8; Settings::LEN],
}

impl Settings {
    pub const LEN: usize = 126;

    pub fn profile(&self) -> u8 {
        self.raw[2]
    }

    pub fn checksum(&self) -> u16 {
        self.raw[..Self::LEN - 2].iter().fold(0u16, |a, b| a.wrapping_add(*b as u16))
    }

    pub fn checksum_ok(&self) -> bool {
        u16::from_le_bytes([self.raw[124], self.raw[125]]) == self.checksum()
    }

    pub fn update_checksum(&mut self) {
        let c = self.checksum().to_le_bytes();
        self.raw[124..].copy_from_slice(&c);
    }

    pub fn stage(&self) -> u8 {
        self.raw[5]
    }

    pub fn set_stage(&mut self, stage: u8) -> Result<()> {
        if stage as usize >= DPI_STAGES {
            bail!("no DPI stage {}", stage + 1);
        }
        self.raw[5] = stage;
        Ok(())
    }

    fn u16_at(&self, o: usize) -> u16 {
        u16::from_le_bytes([self.raw[o], self.raw[o + 1]])
    }

    /// `(x, y)` DPI of stage `i`.
    pub fn dpi(&self, i: usize) -> (u16, u16) {
        (self.u16_at(6 + 2 * i) * DPI_STEP, self.u16_at(16 + 2 * i) * DPI_STEP)
    }

    /// Sets both axes of stage `i`, rounded to 50 DPI.
    pub fn set_dpi(&mut self, i: usize, dpi: u16, model: &Model) -> Result<()> {
        if i >= DPI_STAGES {
            bail!("no DPI stage {}", i + 1);
        }
        if !(model.min_dpi..=model.max_dpi).contains(&dpi) {
            bail!("DPI must be {}..={}", model.min_dpi, model.max_dpi);
        }
        let v = ((dpi + DPI_STEP / 2) / DPI_STEP).to_le_bytes();
        self.raw[6 + 2 * i..8 + 2 * i].copy_from_slice(&v);
        self.raw[16 + 2 * i..18 + 2 * i].copy_from_slice(&v);
        Ok(())
    }

    /// Polling rate in Hz; `None` for an index this crate doesn't know.
    pub fn report_rate(&self) -> Option<u16> {
        REPORT_RATES.get(self.raw[26] as usize).copied()
    }

    pub fn set_report_rate(&mut self, hz: u16) -> Result<()> {
        let Some(i) = REPORT_RATES.iter().position(|r| *r == hz) else {
            bail!("polling rate must be one of {REPORT_RATES:?} Hz")
        };
        self.raw[26] = i as u8;
        Ok(())
    }

    pub fn effect(&self) -> u8 {
        self.raw[33]
    }

    pub fn set_effect(&mut self, effect: u8) -> Result<()> {
        if !EFFECTS.iter().any(|(e, _)| *e == effect) {
            bail!("unknown lighting effect {effect}");
        }
        self.raw[33] = effect;
        Ok(())
    }

    pub fn speed(&self) -> u8 {
        self.raw[34]
    }

    pub fn set_speed(&mut self, speed: u8) -> Result<()> {
        if !SPEEDS.iter().any(|(s, _)| *s == speed) {
            bail!("effect speed must be 1..=3");
        }
        self.raw[34] = speed;
        Ok(())
    }

    pub fn brightness(&self) -> u8 {
        self.raw[35]
    }

    pub fn set_brightness(&mut self, v: u8) {
        self.raw[35] = v;
    }

    /// The zone's colour (its first LED's).
    pub fn color(&self, zone: Zone) -> [u8; 3] {
        let o = zone.slots()[0];
        [self.raw[o + 1], self.raw[o + 2], self.raw[o + 3]]
    }

    /// Sets every LED of `zone` to `rgb` at full intensity.
    pub fn set_color(&mut self, zone: Zone, rgb: [u8; 3]) {
        for &o in zone.slots() {
            self.raw[o..o + 5].copy_from_slice(&[0xFF, rgb[0], rgb[1], rgb[2], 0x00]);
        }
    }
}

/// Button slots: the first 12 are the normal layer, the next 12 the layer
/// active while the Easy-Shift button is held.
pub const BUTTON_NAMES: [&str; 12] = [
    "Left button",
    "Right button",
    "Wheel click",
    "Wheel tilt left",
    "Wheel tilt right",
    "Wheel up",
    "Wheel down",
    "Side button forward",
    "Side button back",
    "Easy-Shift button",
    "Top button up",
    "Top button down",
];
pub const SLOTS: usize = 2 * BUTTON_NAMES.len();
/// Normal-layer slots that must keep an action, or the wheel stops scrolling.
pub const WHEEL_SLOTS: [usize; 4] = [3, 4, 5, 6];

pub mod action {
    pub const DISABLED: u8 = 0;
    pub const LEFT_CLICK: u8 = 1;
    /// Uses the slot's modifier and key bytes.
    pub const SHORTCUT: u8 = 5;
    /// Plays the macro stored for the slot (report `08`, not implemented).
    pub const MACRO: u8 = 48;
}

/// Button actions this crate can assign; order is menu order.
pub const ACTIONS: [(u8, &str); 26] = [
    (0, "Disabled"),
    (1, "Left click"),
    (2, "Right click"),
    (3, "Middle click"),
    (4, "Double click"),
    (7, "Browser forward"),
    (8, "Browser back"),
    (13, "Scroll up"),
    (14, "Scroll down"),
    (9, "Scroll left"),
    (10, "Scroll right"),
    (20, "DPI cycle"),
    (21, "DPI up"),
    (22, "DPI down"),
    (16, "Profile cycle"),
    (17, "Profile up"),
    (18, "Profile down"),
    (35, "Play / pause"),
    (33, "Previous track"),
    (34, "Next track"),
    (36, "Stop"),
    (37, "Mute"),
    (38, "Volume up"),
    (39, "Volume down"),
    (65, "Easy-Shift"),
    (action::SHORTCUT, "Keyboard shortcut"),
];

pub fn action_name(a: u8) -> String {
    match ACTIONS.iter().find(|(c, _)| *c == a) {
        Some((_, n)) => n.to_string(),
        None if a == action::MACRO => "Macro".into(),
        None => format!("Action {a}"),
    }
}

/// Report `07`: 24 slots of `action, modifiers, key` from byte 3. There is
/// no checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub raw: [u8; Buttons::LEN],
}

impl Buttons {
    pub const LEN: usize = 75;

    pub fn action(&self, slot: usize) -> u8 {
        self.raw[3 + 3 * slot]
    }

    /// `(modifiers, key)` of a shortcut slot.
    pub fn shortcut(&self, slot: usize) -> (u8, u8) {
        (self.raw[4 + 3 * slot], self.raw[5 + 3 * slot])
    }

    /// Assigns an action without a shortcut.
    pub fn set_action(&mut self, slot: usize, action: u8) {
        self.raw[3 + 3 * slot..6 + 3 * slot].copy_from_slice(&[action, 0, 0]);
    }

    pub fn set_shortcut(&mut self, slot: usize, modifiers: u8, key: u8) {
        self.raw[3 + 3 * slot..6 + 3 * slot].copy_from_slice(&[action::SHORTCUT, modifiers, key]);
    }

    /// Actions ROCCAT Swarm writes to slots 0–10 (from libratbag PR #1883,
    /// which observed them in Swarm captures).
    pub const DEFAULTS: [u8; 11] = [1, 2, 3, 9, 10, 13, 14, 7, 8, 65, 16];

    /// A profile Swarm never wrote reads back with no actions at all, while
    /// the mouse itself behaves normally.
    pub fn is_blank(&self) -> bool {
        (0..BUTTON_NAMES.len()).all(|s| self.action(s) == action::DISABLED)
    }

    /// Fills a [blank](Self::is_blank) profile with [`Self::DEFAULTS`] so
    /// writing one button doesn't disable the others.
    pub fn fill_blank(&mut self) {
        if self.is_blank() {
            for (s, a) in Self::DEFAULTS.iter().enumerate() {
                self.set_action(s, *a);
            }
        }
    }

    /// Why this assignment would make the mouse hard to use, if it would.
    pub fn problem(&self) -> Option<&'static str> {
        if !(0..BUTTON_NAMES.len()).any(|s| self.action(s) == action::LEFT_CLICK) {
            return Some("no button would be left as Left click");
        }
        if WHEEL_SLOTS.iter().any(|s| self.action(*s) == action::DISABLED) {
            return Some("the wheel needs an action on tilt and scroll, or it stops scrolling");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut raw = [0u8; Settings::LEN];
        raw[..3].copy_from_slice(&[0x06, 0x7E, 0x02]);
        raw[5] = 1;
        for (i, d) in [400u16, 800, 1600, 3200, 6400].iter().enumerate() {
            raw[6 + 2 * i..8 + 2 * i].copy_from_slice(&(d / 50).to_le_bytes());
            raw[16 + 2 * i..18 + 2 * i].copy_from_slice(&(d / 50).to_le_bytes());
        }
        raw[26] = 3;
        raw[33..36].copy_from_slice(&[1, 2, 0xFF]);
        let mut s = Settings { raw };
        s.update_checksum();
        s
    }

    #[test]
    fn settings_fields() {
        let s = settings();
        assert!(s.checksum_ok());
        assert_eq!(s.profile(), 2);
        assert_eq!(s.stage(), 1);
        assert_eq!(s.dpi(3), (3200, 3200));
        assert_eq!(s.report_rate(), Some(1000));
        assert_eq!((s.effect(), s.speed(), s.brightness()), (1, 2, 255));
    }

    #[test]
    fn checksum_is_le_byte_sum() {
        let mut s = settings();
        s.raw[50] = 0xFF;
        s.raw[51] = 0xFF;
        assert!(!s.checksum_ok());
        s.update_checksum();
        let sum: u32 = s.raw[..124].iter().map(|b| *b as u32).sum();
        assert_eq!(s.raw[124] as u32 | (s.raw[125] as u32) << 8, sum & 0xFFFF);
    }

    #[test]
    fn dpi_edit_sets_both_axes_and_rounds() {
        let model = &MODELS[1];
        let mut s = settings();
        s.set_dpi(4, 16000, model).unwrap();
        assert_eq!(s.dpi(4), (16000, 16000));
        assert_eq!(&s.raw[14..16], &[0x40, 0x01]); // 320 * 50
        s.set_dpi(0, 1234, model).unwrap();
        assert_eq!(s.dpi(0), (1250, 1250));
        assert!(s.set_dpi(0, 16050, model).is_err());
        assert!(s.set_dpi(0, 13000, &MODELS[0]).is_err());
        assert!(s.set_dpi(5, 800, model).is_err());
    }

    #[test]
    fn colours_fill_every_led_of_a_zone() {
        let mut s = settings();
        s.set_color(Zone::Left, [1, 2, 3]);
        for o in [66, 71, 76, 81] {
            assert_eq!(&s.raw[o..o + 5], &[0xFF, 1, 2, 3, 0]);
        }
        assert_eq!(s.raw[86..91], [0; 5]);
        assert_eq!(s.color(Zone::Left), [1, 2, 3]);
        s.set_color(Zone::Bottom, [9, 9, 9]);
        assert_eq!(&s.raw[111..116], &[0xFF, 9, 9, 9, 0]);
        // The LED slots end before the bytes that hold the theme and checksum.
        assert_eq!(Zone::ALL.iter().flat_map(|z| z.slots()).map(|o| o + 5).max(), Some(116));
    }

    #[test]
    fn rate_and_lighting_validation() {
        let mut s = settings();
        s.set_report_rate(250).unwrap();
        assert_eq!(s.raw[26], 1);
        assert!(s.set_report_rate(2000).is_err());
        assert!(s.set_effect(5).is_err());
        s.set_effect(7).unwrap();
        assert!(s.set_speed(0).is_err());
    }

    fn default_buttons() -> Buttons {
        let mut b = Buttons { raw: [0; Buttons::LEN] };
        b.raw[..3].copy_from_slice(&[0x07, 0x4B, 0x00]);
        assert!(b.is_blank());
        b.fill_blank();
        assert!(!b.is_blank());
        b
    }

    #[test]
    fn fill_blank_leaves_configured_profiles_alone() {
        let mut b = default_buttons();
        b.set_action(7, action::DISABLED);
        b.fill_blank();
        assert_eq!(b.action(7), action::DISABLED);
    }

    #[test]
    fn buttons_layout() {
        let mut b = default_buttons();
        assert_eq!(b.raw[3..9], [1, 0, 0, 2, 0, 0]);
        assert_eq!(b.problem(), None);
        b.set_shortcut(23, 0x01, 0x06);
        assert_eq!(&b.raw[72..75], &[5, 0x01, 0x06]);
        assert_eq!(b.shortcut(23), (0x01, 0x06));
        b.set_action(23, 2);
        assert_eq!(b.shortcut(23), (0, 0));
    }

    #[test]
    fn buttons_problems() {
        let mut b = default_buttons();
        b.set_action(0, 2);
        assert!(b.problem().is_some());
        b.set_action(11, action::LEFT_CLICK);
        assert_eq!(b.problem(), None);
        b.set_action(5, action::DISABLED);
        assert!(b.problem().is_some());
    }

    #[test]
    fn action_menu_is_unique() {
        for (i, (a, _)) in ACTIONS.iter().enumerate() {
            assert_eq!(ACTIONS.iter().position(|(b, _)| a == b), Some(i));
        }
        assert_eq!(action_name(action::MACRO), "Macro");
    }
}
