//! PAN1080 mouse commands (web driver class `CommonMsPan1080`).
//!
//! | cmd  | direction | payload |
//! |------|-----------|---------|
//! | `8F` | read  | bytes 1..5 = model id (u32 LE, `FFFFFFFF` = bootloader) |
//! | `80` | read  | bytes 1..3 = firmware version (u16 LE); `AD` on the 8K subclass |
//! | `D3` / `53` | read / write | option block, see [`Options`] |
//! | `D4 p` / `54 p` | read / write | DPI stages of profile `p`, see [`DpiTable`] |
//! | `D0 p` | read | button matrix of profile `p`: 16 × 4 bytes, see [`Action`] |
//! | `50 p slot` | write | bytes 8..12 = one button's 4-byte action |
//! | `02` | write | factory reset |
//!
//! The driver writes the option block whole: it reads it, changes one field
//! and sends it back with command `53`. Profile changes are made the same
//! way (byte 8), not with the generic `05` command.

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use hidapi::{DeviceInfo, HidApi};

use crate::models::{self, Model};
use crate::transport::{Link, MouseStatus, REPORT_LEN, Report, checksum, frame};

pub mod cmd {
    pub const RESET: u8 = 0x02;
    pub const GET_VERSION: u8 = 0x80;
    pub const GET_INFO: u8 = 0x8F;
    pub const SET_KEY: u8 = 0x50;
    pub const GET_KEYS: u8 = 0xD0;
    pub const SET_OPTIONS: u8 = 0x53;
    pub const GET_OPTIONS: u8 = 0xD3;
    pub const SET_DPI: u8 = 0x54;
    pub const GET_DPI: u8 = 0xD4;
}

/// Pause after every write (the web driver's `vendorSleep`, 100 ms).
const WRITE_SETTLE: Duration = Duration::from_millis(100);

pub const REPORT_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

pub fn encode_rate(hz: u32) -> Option<u8> {
    Some(match hz {
        8000 => 0x81,
        4000 => 0x82,
        2000 => 0x84,
        1000 => 1,
        500 => 2,
        250 => 4,
        125 => 8,
        _ => return None,
    })
}

pub fn decode_rate(b: u8) -> Option<u32> {
    Some(match b {
        0x81 => 8000,
        0x82 => 4000,
        0x84 => 2000,
        1 => 1000,
        2 => 500,
        4 => 250,
        8 => 125,
        _ => return None,
    })
}

/// The option block (`D3` reply / `53` payload). Byte offsets:
///
/// | byte | field |
/// |------|-------|
/// | 8  | active profile |
/// | 9  | polling rate code ([`decode_rate`]) |
/// | 10 | debounce |
/// | 12 | flags: bit 0 lighting on, 1 wheel light off, 2 ripple control, 3 LED select, 4 power saving |
/// | 14..17 | button change, wheel→button, button→wheel |
/// | 24..32 | lighting effect |
/// | 40..48 | sleep times, u16 LE: BT, BT deep, 2.4 GHz, 2.4 GHz deep |
/// | 50, 51 | X / Y sensitivity |
/// | 52 | lift-off distance index ([`crate::Sensor::lod_mm`]) |
/// | 53 | angle snapping (1 = on) |
/// | 54 | 20K FPS (1 = on) |
/// | 60 | charging switch |
///
/// Bytes this crate doesn't change are written back as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub raw: Report,
}

const FLAG_RIPPLE: u8 = 1 << 2;

impl Options {
    pub fn profile(&self) -> u8 {
        self.raw[8]
    }

    pub fn set_profile(&mut self, p: u8) {
        self.raw[8] = p;
    }

    pub fn report_rate(&self) -> Option<u32> {
        decode_rate(self.raw[9])
    }

    pub fn set_report_rate(&mut self, hz: u32) -> Result<()> {
        let Some(code) = encode_rate(hz) else { bail!("unsupported polling rate {hz} Hz") };
        self.raw[9] = code;
        Ok(())
    }

    pub fn ripple(&self) -> bool {
        self.raw[12] & FLAG_RIPPLE != 0
    }

    pub fn set_ripple(&mut self, on: bool) {
        if on {
            self.raw[12] |= FLAG_RIPPLE;
        } else {
            self.raw[12] &= !FLAG_RIPPLE;
        }
    }

    pub fn lod(&self) -> u8 {
        self.raw[52]
    }

    pub fn set_lod(&mut self, v: u8) {
        self.raw[52] = v;
    }

    pub fn angle_snap(&self) -> bool {
        self.raw[53] == 1
    }

    pub fn set_angle_snap(&mut self, on: bool) {
        self.raw[53] = on as u8;
    }

    /// The `53` write report.
    pub fn to_write(&self) -> Report {
        let mut w = self.raw;
        w[0] = cmd::SET_OPTIONS;
        w[1..8].fill(0);
        w[7] = checksum(&w);
        w
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiStage {
    pub x: u16,
    pub y: u16,
    pub color: [u8; 3],
}

/// DPI stages (`D4 p` reply / `54 p` payload): byte 2 = active stage, 3 =
/// stage count, then per stage `i`: X at `8 + 2i`, Y at `24 + 2i` (u16 LE),
/// colour at `40 + 3i` (RGB). At most 8 stages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiTable {
    pub current: u8,
    pub stages: Vec<DpiStage>,
}

pub const MAX_STAGES: usize = 8;

impl DpiTable {
    pub fn parse(r: &Report) -> Self {
        let count = (r[3] as usize).min(MAX_STAGES);
        let stages = (0..count)
            .map(|i| DpiStage {
                x: u16::from_le_bytes([r[8 + i * 2], r[9 + i * 2]]),
                y: u16::from_le_bytes([r[24 + i * 2], r[25 + i * 2]]),
                color: [r[40 + i * 3], r[41 + i * 3], r[42 + i * 3]],
            })
            .collect();
        // The web driver treats an out-of-range index as stage 1.
        let current = if r[2] as usize >= count.max(1) { 0 } else { r[2] };
        DpiTable { current, stages }
    }

    pub fn to_write(&self, profile: u8) -> Report {
        let mut w = [0u8; REPORT_LEN];
        w[0] = cmd::SET_DPI;
        w[1] = profile;
        w[2] = self.current;
        w[3] = self.stages.len() as u8;
        for (i, s) in self.stages.iter().take(MAX_STAGES).enumerate() {
            w[8 + i * 2..10 + i * 2].copy_from_slice(&s.x.to_le_bytes());
            w[24 + i * 2..26 + i * 2].copy_from_slice(&s.y.to_le_bytes());
            w[40 + i * 3..43 + i * 3].copy_from_slice(&s.color);
        }
        w[7] = checksum(&w);
        w
    }
}

/// Default DPIs and colours for stages added beyond what the mouse stores.
pub const DEFAULT_DPIS: [u16; MAX_STAGES] = [400, 800, 1200, 1600, 2400, 3200, 4000, 6400];
pub const DEFAULT_COLORS: [[u8; 3]; MAX_STAGES] = [
    [0xFF, 0, 0],
    [0xFF, 0x80, 0],
    [0xFF, 0xFF, 0],
    [0, 0xFF, 0],
    [0, 0xFF, 0xFF],
    [0, 0, 0xFF],
    [0xFF, 0, 0xFF],
    [0xFF, 0xFF, 0xFF],
];

/// Button slots: the first six entries of the default matrix the web driver
/// ships for these mice (`1 0 F0 0`, `1 0 F1 0`, `1 0 F2 0`, `1 0 F4 0`,
/// `1 0 F3 0`, `14 0 0 0`). Slots 12..16 are wheel functions.
pub const BUTTONS: [&str; 6] = ["Left button", "Right button", "Middle button", "Forward button", "Back button", "DPI button"];

/// One 4-byte button action (web driver tables `On` and `K0`). Byte 0 is the
/// type: 0 keyboard (modifiers, key, key 2), 1 mouse, 3 consumer, 5 polling
/// rate, 8 profile, 20 DPI.
pub type Action = [u8; 4];

pub const ACTIONS: &[(&str, Action)] = &[
    ("Left click", [1, 0, 0xF0, 0]),
    ("Right click", [1, 0, 0xF1, 0]),
    ("Middle click", [1, 0, 0xF2, 0]),
    ("Back", [1, 0, 0xF3, 0]),
    ("Forward", [1, 0, 0xF4, 0]),
    ("DPI cycle", [20, 0, 0, 0]),
    ("DPI +", [20, 0, 1, 0]),
    ("Scroll up", [1, 0, 0xF5, 1]),
    ("Scroll down", [1, 0, 0xF5, 0xFF]),
    ("Profile cycle", [8, 0, 3, 0]),
    ("Polling rate cycle", [5, 0, 3, 0]),
    ("Play / pause", [3, 0, 0xCD, 0]),
    ("Next track", [3, 0, 0xB5, 0]),
    ("Previous track", [3, 0, 0xB6, 0]),
    ("Volume up", [3, 0, 0xE9, 0]),
    ("Volume down", [3, 0, 0xEA, 0]),
    ("Mute", [3, 0, 0xE2, 0]),
    ("Copy (Ctrl+C)", [0, 0, 0xE0, 0x06]),
    ("Paste (Ctrl+V)", [0, 0, 0xE0, 0x19]),
    ("Disabled", [0, 0, 0, 0]),
];

/// Reads the button matrix out of a `D0` reply. The web driver uses the
/// reply from byte 0; if the firmware echoes the command like it does for
/// other reads, the matrix starts after the 8-byte header instead.
pub fn parse_buttons(r: &Report) -> [Action; BUTTONS.len()] {
    let base = if r[0] == cmd::GET_KEYS { 8 } else { 0 };
    std::array::from_fn(|i| r[base + i * 4..base + i * 4 + 4].try_into().unwrap())
}

pub fn action_label(a: &Action) -> String {
    match ACTIONS.iter().find(|(_, v)| v == a) {
        Some((l, _)) => l.to_string(),
        None => format!("Other ({:02X} {:02X} {:02X} {:02X})", a[0], a[1], a[2], a[3]),
    }
}

pub struct Mouse {
    link: Link,
    /// Model id (`8F`); `None` while a wireless mouse is asleep.
    pub id: Option<u32>,
    pub product_id: u16,
    /// Highest polling rate over this connection.
    pub max_rate: u32,
}

impl Mouse {
    /// The vendor collection of a supported wired mouse or receiver.
    pub fn candidates(api: &HidApi) -> impl Iterator<Item = &DeviceInfo> {
        api.device_list().filter(|d| {
            d.vendor_id() == models::VENDOR_ID
                && d.usage_page() == models::USAGE_PAGE
                && d.usage() == models::USAGE
                && (models::MOUSE_PIDS.contains(&d.product_id()) || models::receiver_max_rate(d.product_id()).is_some())
        })
    }

    /// Opens a candidate. Returns `None` for a receiver without a mouse.
    pub fn open(api: &HidApi, info: &DeviceInfo) -> Result<Option<Mouse>> {
        let pid = info.product_id();
        let receiver = models::receiver_max_rate(pid);
        let link = Link::new(info.open_device(api)?, receiver.is_some());
        let mut mouse = Mouse { link, id: None, product_id: pid, max_rate: receiver.unwrap_or(1000) };
        if mouse.link.is_receiver() {
            let s = mouse.link.receiver_status()?;
            if !s.paired {
                return Ok(None);
            }
            if s.online {
                mouse.id = Some(mouse.read_id()?);
            }
        } else {
            mouse.id = Some(mouse.read_id()?);
        }
        Ok(Some(mouse))
    }

    pub fn is_wireless(&self) -> bool {
        self.link.is_receiver()
    }

    fn read_id(&self) -> Result<u32> {
        let r = self.link.query_echo(&[cmd::GET_INFO])?;
        let id = u32::from_le_bytes(r[1..5].try_into().unwrap());
        if id == u32::MAX {
            bail!("the mouse is in firmware-update mode");
        }
        Ok(id)
    }

    pub fn model(&self) -> Option<&'static Model> {
        self.id.and_then(models::model)
    }

    /// Receiver status (`None` when wired).
    pub fn status(&self) -> Result<Option<MouseStatus>> {
        if !self.link.is_receiver() {
            return Ok(None);
        }
        self.link.receiver_status().map(Some)
    }

    /// Reads the model id if it isn't known yet (a mouse that woke up).
    pub fn identify(&mut self) -> Result<Option<u32>> {
        if self.id.is_none() && self.status()?.is_none_or(|s| s.online) {
            self.id = Some(self.read_id()?);
        }
        Ok(self.id)
    }

    pub fn firmware(&self) -> Result<u16> {
        let c = self.model().map_or(cmd::GET_VERSION, |m| m.version_cmd);
        let r = self.link.query_echo(&[c])?;
        Ok(u16::from_le_bytes([r[1], r[2]]))
    }

    pub fn options(&self) -> Result<Options> {
        Ok(Options { raw: self.link.query_echo(&[cmd::GET_OPTIONS])? })
    }

    /// Writes the option block and checks that `check` reads back the same.
    pub fn set_options<T: PartialEq + std::fmt::Debug>(&self, o: &Options, check: impl Fn(&Options) -> T) -> Result<()> {
        self.link.send(&o.to_write())?;
        sleep(WRITE_SETTLE);
        let back = self.options()?;
        if check(&back) != check(o) {
            bail!("the mouse did not keep the change (reads back {:?})", check(&back));
        }
        Ok(())
    }

    pub fn dpi(&self, profile: u8) -> Result<DpiTable> {
        Ok(DpiTable::parse(&self.link.query_echo(&[cmd::GET_DPI, profile])?))
    }

    pub fn set_dpi(&self, profile: u8, t: &DpiTable) -> Result<()> {
        if t.stages.is_empty() || t.stages.len() > MAX_STAGES {
            bail!("1 to {MAX_STAGES} DPI stages");
        }
        self.link.send(&t.to_write(profile))?;
        sleep(WRITE_SETTLE);
        let back = self.dpi(profile)?;
        if &back != t {
            bail!("the mouse did not keep the DPI change");
        }
        Ok(())
    }

    pub fn buttons_raw(&self, profile: u8) -> Result<Report> {
        self.link.query(&[cmd::GET_KEYS, profile])
    }

    pub fn buttons(&self, profile: u8) -> Result<[Action; BUTTONS.len()]> {
        Ok(parse_buttons(&self.buttons_raw(profile)?))
    }

    pub fn set_button(&self, profile: u8, slot: u8, action: Action) -> Result<()> {
        if slot as usize >= BUTTONS.len() {
            bail!("no button {slot}");
        }
        let mut c = [0u8; 12];
        c[0] = cmd::SET_KEY;
        c[1] = profile;
        c[2] = slot;
        c[8..12].copy_from_slice(&action);
        self.link.send(&frame(&c))?;
        sleep(WRITE_SETTLE);
        Ok(())
    }

    pub fn factory_reset(&self) -> Result<()> {
        self.link.send(&frame(&[cmd::RESET]))?;
        sleep(WRITE_SETTLE);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_roundtrip() {
        for hz in REPORT_RATES {
            assert_eq!(decode_rate(encode_rate(hz).unwrap()), Some(hz));
        }
        assert_eq!(encode_rate(333), None);
    }

    /// Bytes as `setMouseOption0` lays them out (no capture from a real
    /// mouse yet).
    fn sample_options() -> Options {
        let mut r = [0u8; REPORT_LEN];
        r[0] = cmd::GET_OPTIONS;
        r[8] = 1; // profile 2
        r[9] = 0x81; // 8000 Hz
        r[10] = 4;
        r[12] = 0b1_0001; // lighting on, power saving
        r[14..19].copy_from_slice(&[1, 10, 10, 0xFF, 8]);
        r[40..48].copy_from_slice(&[120, 0, 0x90, 6, 120, 0, 0x90, 6]);
        r[50] = 100;
        r[51] = 100;
        r[52] = 1;
        Options { raw: r }
    }

    #[test]
    fn options_fields() {
        let mut o = sample_options();
        assert_eq!(o.profile(), 1);
        assert_eq!(o.report_rate(), Some(8000));
        assert!(!o.ripple() && !o.angle_snap());
        o.set_ripple(true);
        o.set_angle_snap(true);
        o.set_report_rate(1000).unwrap();
        assert_eq!(o.raw[12], 0b1_0101);
        assert_eq!(o.raw[53], 1);
        assert_eq!(o.raw[9], 1);
        o.set_ripple(false);
        assert_eq!(o.raw[12], 0b1_0001);
    }

    #[test]
    fn options_write_keeps_unknown_bytes() {
        let o = sample_options();
        let w = o.to_write();
        assert_eq!(w[0], cmd::SET_OPTIONS);
        assert_eq!(w[7], 0xFF - cmd::SET_OPTIONS);
        assert_eq!(w[8..], o.raw[8..]);
    }

    #[test]
    fn dpi_roundtrip() {
        let t = DpiTable {
            current: 1,
            stages: (0..6)
                .map(|i| DpiStage { x: DEFAULT_DPIS[i], y: DEFAULT_DPIS[i], color: DEFAULT_COLORS[i] })
                .collect(),
        };
        let w = t.to_write(2);
        assert_eq!(&w[..4], &[cmd::SET_DPI, 2, 1, 6]);
        assert_eq!(&w[8..10], &400u16.to_le_bytes());
        assert_eq!(&w[24..26], &400u16.to_le_bytes());
        assert_eq!(&w[43..46], &[0xFF, 0x80, 0]);
        assert_eq!(DpiTable::parse(&w), t);
    }

    #[test]
    fn button_matrix_offsets() {
        // The default matrix from the web driver, as a headerless reply...
        let default: [u8; 24] = [1, 0, 240, 0, 1, 0, 241, 0, 1, 0, 242, 0, 1, 0, 244, 0, 1, 0, 243, 0, 20, 0, 0, 0];
        let mut r = [0u8; REPORT_LEN];
        r[..24].copy_from_slice(&default);
        let b = parse_buttons(&r);
        assert_eq!(action_label(&b[0]), "Left click");
        assert_eq!(action_label(&b[3]), "Forward");
        assert_eq!(action_label(&b[5]), "DPI cycle");
        // ...and with an echoed header.
        let mut r = [0u8; REPORT_LEN];
        r[0] = cmd::GET_KEYS;
        r[8..32].copy_from_slice(&default);
        assert_eq!(parse_buttons(&r), b);
    }
}
