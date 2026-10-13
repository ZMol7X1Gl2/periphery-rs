//! Keychron firmware extensions on top of VIA (commands 0xA0-0xAC), recovered
//! from launcher.keychron.com.

use anyhow::{Result, bail};

use crate::transport::Raw;

pub const KC_PROTOCOL: u8 = 0xA0;
pub const KC_FIRMWARE: u8 = 0xA1;
pub const KC_MISC: u8 = 0xA7;
pub const KC_HE: u8 = 0xA9;
pub const KC_FACTORY: u8 = 0xAB;

mod misc {
    pub const DEBOUNCE_GET: u8 = 5;
    pub const DEBOUNCE_SET: u8 = 6;
    pub const SLEEP_GET: u8 = 11;
    pub const SLEEP_SET: u8 = 12;
    pub const POLL_GET: u8 = 13;
    pub const POLL_SET: u8 = 14;
    pub const DIP_GET: u8 = 15;
    pub const DIP_SET: u8 = 16;
    pub const PERF_GET: u8 = 17;
    pub const PERF_SET: u8 = 18;
    pub const NKRO_GET: u8 = 18;
    pub const NKRO_SET: u8 = 19;
}

mod amc {
    pub const GET_VERSION: u8 = 1;
    pub const GET_PROFILES_INFO: u8 = 16;
    pub const SELECT_PROFILE: u8 = 17;
    pub const GET_PROFILE_BUFFER: u8 = 18;
    pub const SET_TRAVEL: u8 = 20;
    pub const SET_ACT_ON_AXIS: u8 = 22;
    pub const SAVE_PROFILE_BUFFER: u8 = 31;
}

pub const KC_FEATURES: u8 = 0xA2;

/// Feature flags from `KC_GET_SUPPORT_FEATURE` (two bitmap generations).
#[derive(Clone, Copy, Debug, Default)]
pub struct Features {
    pub bluetooth: bool,
    pub rf: bool,
    pub debounce: bool,
    pub snap_click: bool,
    pub keychron_rgb: bool,
    pub performance: bool,
    pub bootloader_jump: bool,
}

fn features(raw: &Raw, version: u8, instruction_set: u8) -> Result<Features> {
    let Some(r) = raw.try_query(&[KC_FEATURES])? else { return Ok(Features::default()) };
    let bit = |v: u32, n: u32| v >> n & 1 == 1;
    Ok(if version >= 4 {
        let a = u16::from_le_bytes([r[3], r[4]]) as u32;
        let b = u32::from_le_bytes([r[6], r[7], r[8], 0]);
        Features {
            bluetooth: bit(a, 0),
            rf: bit(a, 1),
            debounce: bit(b, 6),
            snap_click: bit(b, 7),
            keychron_rgb: bit(b, 2),
            performance: bit(b, 17),
            bootloader_jump: bit(b, 16),
        }
    } else {
        // Which two bytes hold the bitmap depends on the instruction set.
        let (a, b) = match instruction_set {
            2 | 4 => (r[2] as u32, r[3] as u32),
            _ => (r[1] as u32, r[2] as u32),
        };
        Features {
            bluetooth: bit(a, 1),
            rf: bit(a, 2),
            debounce: bit(a, 5),
            snap_click: bit(a, 6),
            keychron_rgb: bit(a, 7),
            performance: bit(b, 0) && instruction_set != 2,
            bootloader_jump: bit(b, 3) && matches!(instruction_set, 2 | 4),
        }
    })
}

pub struct KeychronInfo {
    pub version: u8,
    pub features: Features,
    pub firmware: Option<String>,
    pub transport: Option<&'static str>,
}

pub fn info(raw: &Raw) -> Result<Option<KeychronInfo>> {
    let Some(v) = raw.try_query(&[KC_PROTOCOL])? else { return Ok(None) };
    let features = features(raw, v[1], v[3])?;
    let firmware = raw.try_query(&[KC_FIRMWARE])?.map(|r| {
        let end = r[1..].iter().position(|b| *b == 0).map_or(r.len(), |p| p + 1);
        String::from_utf8_lossy(&r[1..end]).trim().to_string()
    });
    let mut t = [0u8; 31];
    t[0] = KC_FACTORY;
    t[1] = 5;
    t[30] = 5;
    let transport = raw.try_query(&t)?.and_then(|r| match r[2] {
        1 => Some("USB"),
        2 => Some("Bluetooth"),
        4 => Some("2.4 GHz"),
        _ => None,
    });
    Ok(Some(KeychronInfo { version: v[1], features, firmware, transport }))
}

// ---------------------------------------------------------------- misc group

#[derive(Clone, Copy, Debug)]
pub struct Sleep {
    pub backlight: u16,
    pub sleep: u16,
    pub magnet_scan: u16,
}

pub fn sleep_get(raw: &Raw) -> Result<Option<Sleep>> {
    Ok(raw.try_query(&[KC_MISC, misc::SLEEP_GET])?.and_then(|r| {
        let s = Sleep {
            backlight: u16::from_le_bytes([r[3], r[4]]),
            sleep: u16::from_le_bytes([r[5], r[6]]),
            magnet_scan: u16::from_le_bytes([r[7], r[8]]),
        };
        (s.backlight > 0 || s.sleep > 0).then_some(s)
    }))
}

pub fn sleep_set(raw: &Raw, s: Sleep) -> Result<()> {
    let [b0, b1] = s.backlight.to_le_bytes();
    let [s0, s1] = s.sleep.to_le_bytes();
    let [m0, m1] = s.magnet_scan.to_le_bytes();
    let r = raw.query(&[KC_MISC, misc::SLEEP_SET, b0, b1, s0, s1, m0, m1])?;
    if r[2] != 0 {
        bail!("keyboard rejected the sleep settings");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum Debounce {
    /// QMK algorithm (0 defer global, 2 defer per key, 4 eager per key,
    /// 5 eager-defer per key, 6 none) and time in ms.
    Qmk { mode: u8, ms: u8 },
    Telink { press: u8, release: u8 },
}

pub fn debounce_get(raw: &Raw) -> Result<Option<Debounce>> {
    let Some(r) = raw.try_query(&[KC_MISC, misc::DEBOUNCE_GET, 255])? else { return Ok(None) };
    if r[2] == 255 {
        return Ok(None);
    }
    Ok(match r[3] {
        0 => Some(Debounce::Qmk { mode: r[4], ms: r[5] }),
        1 => Some(Debounce::Telink { press: r[4], release: r[5] }),
        _ => None,
    })
}

pub fn debounce_set(raw: &Raw, d: Debounce) -> Result<()> {
    let (a, b) = match d {
        Debounce::Qmk { mode, ms } => (mode, ms),
        Debounce::Telink { press, release } => (press, release),
    };
    raw.try_query(&[KC_MISC, misc::DEBOUNCE_SET, a, b])?;
    Ok(())
}

pub const POLL_RATES: [u32; 7] = [8000, 4000, 2000, 1000, 500, 250, 125];

/// (current rate, supported rates). Rate = 8000 >> index.
pub fn poll_get(raw: &Raw) -> Result<Option<(u32, Vec<u32>)>> {
    Ok(raw.try_query(&[KC_MISC, misc::POLL_GET])?.map(|r| {
        let supported = if r[4] == 0 {
            POLL_RATES.to_vec()
        } else {
            POLL_RATES.iter().enumerate().filter(|(i, _)| r[4] >> i & 1 == 1).map(|(_, v)| *v).collect()
        };
        (8000 >> r[3].min(6), supported)
    }))
}

pub fn poll_set(raw: &Raw, hz: u32) -> Result<()> {
    let Some(index) = POLL_RATES.iter().position(|r| *r == hz) else { bail!("unsupported rate {hz}") };
    raw.try_query(&[KC_MISC, misc::POLL_SET, index as u8])?;
    Ok(())
}

/// (command version, mode 0 battery saver / 1 normal / 2 high performance)
pub fn perf_get(raw: &Raw) -> Result<Option<(u8, u8)>> {
    Ok(raw.try_query(&[KC_MISC, misc::PERF_GET])?.map(|r| (r[3], r[4])))
}

pub fn perf_set(raw: &Raw, version: u8, mode: u8) -> Result<()> {
    raw.try_query(&[KC_MISC, misc::PERF_SET, version, mode])?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct Nkro {
    pub switchable: bool,
    pub on: bool,
}

/// Only call when the board has no performance mode: on those boards misc
/// sub-command 18 means "set performance" instead of "get NKRO".
pub fn nkro_get(raw: &Raw, kc_version: u8) -> Result<Option<Nkro>> {
    Ok(raw.try_query(&[KC_MISC, misc::NKRO_GET])?.map(|r| {
        let b = r[3];
        if kc_version >= 4 {
            let auto = b & 2 != 0;
            Nkro { switchable: b & 1 != 0 && !auto, on: auto || b & 4 != 0 }
        } else {
            Nkro { switchable: b & 2 != 0 && b & 4 == 0, on: b & 1 != 0 }
        }
    }))
}

pub fn nkro_set(raw: &Raw, on: bool) -> Result<()> {
    raw.try_query(&[KC_MISC, misc::NKRO_SET, on as u8])?;
    Ok(())
}

/// DIP switch function: 1 polling rate, 2 profile.
pub fn dip_get(raw: &Raw) -> Result<Option<u8>> {
    Ok(raw.try_query(&[KC_MISC, misc::DIP_GET])?.map(|r| r[5]))
}

pub fn dip_set(raw: &Raw, func: u8) -> Result<()> {
    raw.try_query(&[KC_MISC, misc::DIP_SET, func])?;
    Ok(())
}

// ---------------------------------------------------------------- Hall effect

/// Travel settings packed into 32 bits per key (and once globally):
/// bits 0-1 mode, 2-7 actuation, 8-13 rapid-trigger press, 14-19 release,
/// 20-23 advanced mode, 24-31 advanced index. Distances in 0.1 mm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Travel {
    /// 1 static, 2 rapid trigger
    pub mode: u8,
    pub actuation: u8,
    pub rt_press: u8,
    pub rt_release: u8,
}

impl Travel {
    pub fn decode(b: [u8; 4]) -> Travel {
        let v = u32::from_le_bytes(b);
        Travel {
            mode: (v & 3) as u8,
            actuation: (v >> 2 & 63) as u8,
            rt_press: (v >> 8 & 63) as u8,
            rt_release: (v >> 14 & 63) as u8,
        }
    }
}

pub const HE_STATIC: u8 = 1;
pub const HE_RAPID_TRIGGER: u8 = 2;
/// Device range from the launcher (`axis` 0.2-3.8 mm), in 0.1 mm.
pub const HE_MIN: u8 = 2;
pub const HE_MAX: u8 = 38;

#[derive(Clone, Debug)]
pub struct HeState {
    pub version: u8,
    pub current: u8,
    /// Bytes per profile buffer.
    pub size: u16,
    pub names: Vec<String>,
    pub global: Travel,
}

const PROFILE_NAMES: [&str; 3] = ["Default", "Gaming", "Gamepad"];

fn read_buffer(raw: &Raw, profile: u8, offset: u16, len: u8) -> Result<Vec<u8>> {
    let [lo, hi] = offset.to_le_bytes();
    let r = raw.query(&[KC_HE, amc::GET_PROFILE_BUFFER, profile, lo, hi, len])?;
    if r[3] != lo || r[4] != hi {
        bail!("profile buffer reply for the wrong offset");
    }
    Ok(r[6..6 + (len as usize).min(26)].to_vec())
}

pub fn he_read(raw: &Raw) -> Result<Option<HeState>> {
    let Some(v) = raw.try_query(&[KC_HE, amc::GET_VERSION])? else { return Ok(None) };
    let version = v[2];
    if version != 4 {
        // Other generations shift the profile buffer; not decoded yet.
        return Ok(Some(HeState { version, current: 0, size: 0, names: Vec::new(), global: Travel::decode([0; 4]) }));
    }
    let p = raw.query(&[KC_HE, amc::GET_PROFILES_INFO])?;
    let (current, total, size) = (p[2], p[3], u16::from_le_bytes([p[4], p[5]]));
    let mut names = Vec::new();
    for i in 0..total {
        let b = read_buffer(raw, i, size.saturating_sub(32), 26)?;
        let name: String = b
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .filter(|c| *c != 0 && *c != 0xFFFF)
            .filter_map(|c| char::from_u32(c as u32))
            .collect();
        let fallback = PROFILE_NAMES.get(i as usize).copied().unwrap_or("Profile");
        names.push(if name.trim().is_empty() { fallback.to_string() } else { name });
    }
    let g = read_buffer(raw, current, 0, 4)?;
    let global = Travel::decode([g[0], g[1], g[2], g[3]]);
    Ok(Some(HeState { version, current, size, names, global }))
}

/// Applies travel settings to every key of `profile` and saves the profile.
pub fn he_set_global(raw: &Raw, profile: u8, t: Travel) -> Result<()> {
    if !(HE_MIN..=HE_MAX).contains(&t.actuation) {
        bail!("actuation must be 0.2-3.8 mm");
    }
    // keyMode 1 = all keys (0 would take a per-key bitmap).
    let r = raw.query(&[KC_HE, amc::SET_TRAVEL, profile, t.mode, t.actuation, t.rt_press, t.rt_release, 1])?;
    if r[2] != 0 {
        bail!("keyboard rejected the travel settings");
    }
    let r = raw.query(&[KC_HE, amc::SAVE_PROFILE_BUFFER, profile])?;
    if r[2] != 0 {
        bail!("keyboard failed to save the profile");
    }
    Ok(())
}

pub fn he_select_profile(raw: &Raw, profile: u8) -> Result<()> {
    raw.query(&[KC_HE, amc::SELECT_PROFILE, profile])?;
    Ok(())
}

/// One key's entry in the profile buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyTravel {
    /// `mode == 0` means the key follows the global setting.
    pub travel: Travel,
    /// 0 none, 3 DKS, 4 analog, 5 toggle, 7 DKR, 8 turbo
    pub advanced: u8,
    pub advanced_index: u8,
}

impl KeyTravel {
    fn decode(b: &[u8]) -> KeyTravel {
        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        KeyTravel {
            travel: Travel::decode([b[0], b[1], b[2], b[3]]),
            advanced: (v >> 20 & 15) as u8,
            advanced_index: (v >> 24) as u8,
        }
    }
}

pub fn advanced_label(mode: u8) -> Option<&'static str> {
    match mode {
        3 => Some("DKS"),
        4 => Some("Analog"),
        5 => Some("Toggle"),
        7 => Some("DKR"),
        8 => Some("Turbo"),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapPair {
    pub slot: u8,
    pub a: (u8, u8),
    pub b: (u8, u8),
    /// 2 SC, 3 last key priority, 4 first key wins, 5 second key wins, 6 cancel
    pub mode: u8,
}

pub const SNAP_MODES: [(u8, &str); 5] = [
    (2, "Snap click (most recent wins)"),
    (3, "Last key priority"),
    (4, "First key always wins"),
    (5, "Second key always wins"),
    (6, "Cancel both"),
];
/// Slots the launcher uses for snap tap (1-5).
pub const SNAP_SLOTS: std::ops::RangeInclusive<u8> = 1..=5;

#[derive(Clone, Debug)]
pub struct HeProfile {
    pub keys: Vec<KeyTravel>,
    pub snap: Vec<SnapPair>,
}

const DKS_SLOTS: usize = 20;
const DKS_LEN: usize = 19;
const SNAP_COUNT: usize = 20;

/// Reads and decodes the whole buffer of `profile` (layout of HE version 4).
pub fn he_read_profile(raw: &Raw, he: &HeState, rows: u8, cols: u8) -> Result<HeProfile> {
    let mut buf = Vec::with_capacity(he.size as usize);
    while buf.len() < he.size as usize {
        let n = (he.size as usize - buf.len()).min(26) as u8;
        buf.extend(read_buffer(raw, he.current, buf.len() as u16, n)?);
    }
    let n_keys = rows as usize * cols as usize;
    let keys_end = 4 + 4 * n_keys;
    let snap_start = keys_end + DKS_SLOTS * DKS_LEN;
    if buf.len() < snap_start + SNAP_COUNT * 3 {
        bail!("profile buffer is smaller than expected for a {rows}x{cols} matrix");
    }
    let keys = buf[4..keys_end].chunks(4).map(KeyTravel::decode).collect();
    // Each snap key byte packs the column in the high 5 bits, the row in the low 3.
    let unpack = |b: u8| (b & 7, b >> 3);
    let snap = buf[snap_start..snap_start + SNAP_COUNT * 3]
        .chunks(3)
        .enumerate()
        .filter(|(_, c)| c[0] != 0 || c[1] != 0)
        .map(|(i, c)| SnapPair { slot: i as u8, a: unpack(c[0]), b: unpack(c[1]), mode: c[2] })
        .collect();
    Ok(HeProfile { keys, snap })
}

/// Sets travel for specific keys (bitmap of 3 bytes per row) and saves.
pub fn he_set_keys(raw: &Raw, profile: u8, keys: &[(u8, u8)], t: Travel) -> Result<()> {
    if !(HE_MIN..=HE_MAX).contains(&t.actuation) {
        bail!("actuation must be 0.2-3.8 mm");
    }
    let mut req = [0u8; 32];
    req[..8].copy_from_slice(&[KC_HE, amc::SET_TRAVEL, profile, t.mode, t.actuation, t.rt_press, t.rt_release, 0]);
    for &(row, col) in keys {
        let byte = row as usize * 3 + col as usize / 8;
        if col >= 24 || byte >= 24 {
            bail!("key {row},{col} is outside the bitmap");
        }
        req[8 + byte] |= 1 << (col % 8);
    }
    let r = raw.query(&req)?;
    if r[2] != 0 {
        bail!("keyboard rejected the per-key travel settings");
    }
    he_save(raw, profile)
}

pub fn he_save(raw: &Raw, profile: u8) -> Result<()> {
    let r = raw.query(&[KC_HE, amc::SAVE_PROFILE_BUFFER, profile])?;
    if r[2] != 0 {
        bail!("keyboard failed to save the profile");
    }
    Ok(())
}

/// Writes one snap-tap slot; mode 0 with zero keys clears it.
pub fn he_set_snap(raw: &Raw, profile: u8, slot: u8, a: (u8, u8), b: (u8, u8), mode: u8) -> Result<()> {
    let r = raw.query(&[KC_HE, amc::SET_ACT_ON_AXIS, profile, a.0, a.1, b.0, b.1, slot, mode])?;
    if r[2] != 0 {
        bail!("keyboard rejected the snap tap setting");
    }
    Ok(())
}

// ---------------------------------------------------------------- per-key RGB

pub const KC_RGB: u8 = 0xA8;

mod rgb {
    pub const SAVE: u8 = 2;
    pub const LED_NUMBER: u8 = 6;
    pub const GET_COLOR: u8 = 9;
    pub const SET_COLOR: u8 = 10;
}

/// Matrix position → LED index (`None` for keys without an LED).
pub fn led_map(raw: &Raw, rows: u8, cols: u8) -> Result<Vec<Option<u8>>> {
    let mut map = Vec::with_capacity(rows as usize * cols as usize);
    for row in 0..rows {
        let r = raw.query(&[KC_RGB, rgb::LED_NUMBER, row, 255, 255, 255])?;
        map.extend(r[3..3 + cols as usize].iter().map(|b| (*b != 255).then_some(*b)));
    }
    Ok(map)
}

/// HSV colour of every LED, `count` LEDs.
pub fn led_colors(raw: &Raw, count: u8) -> Result<Vec<[u8; 3]>> {
    let mut out = Vec::with_capacity(count as usize);
    while out.len() < count as usize {
        let n = (count as usize - out.len()).min(9) as u8;
        let r = raw.query(&[KC_RGB, rgb::GET_COLOR, out.len() as u8, n])?;
        out.extend(r[3..3 + 3 * n as usize].chunks(3).map(|c| [c[0], c[1], c[2]]));
    }
    Ok(out)
}

/// Sets consecutive LEDs from `start` (HSV); call [`led_save`] afterwards.
pub fn led_set(raw: &Raw, start: u8, hsv: &[[u8; 3]]) -> Result<()> {
    for (i, chunk) in hsv.chunks(9).enumerate() {
        let mut req = vec![KC_RGB, rgb::SET_COLOR, start + (i * 9) as u8, chunk.len() as u8];
        req.extend(chunk.iter().flatten());
        raw.query(&req)?;
    }
    Ok(())
}

pub fn led_save(raw: &Raw) -> Result<()> {
    raw.query(&[KC_RGB, rgb::SAVE])?;
    Ok(())
}

pub fn led_count(raw: &Raw) -> Result<Option<u8>> {
    Ok(raw.try_query(&[KC_RGB, 5])?.map(|r| r[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_from_k2_he() {
        // Global word read from a K2 HE: static, 2.0 mm, RT 0.4/0.4 mm.
        let t = Travel::decode([0x51, 0x04, 0x01, 0x00]);
        assert_eq!(t, Travel { mode: 1, actuation: 20, rt_press: 4, rt_release: 4 });
    }

    #[test]
    fn per_key_entries_from_k2_he() {
        // W/A/S/D entry: rapid trigger at 2.0 mm, 0.4/0.4.
        let k = KeyTravel::decode(&[0x52, 0x04, 0x01, 0x00]);
        assert_eq!(k.travel, Travel { mode: 2, actuation: 20, rt_press: 4, rt_release: 4 });
        assert_eq!(k.advanced, 0);
        // Home: fixed 3.8 mm.
        assert_eq!(KeyTravel::decode(&[0x99, 0x41, 0, 0]).travel.actuation, 38);
    }

    #[test]
    fn snap_key_packing() {
        // Bytes 0x0b, 0x1b from a K2 HE: A (row 3, col 1) and D (row 3, col 3).
        let unpack = |b: u8| (b & 7, b >> 3);
        assert_eq!(unpack(0x0b), (3, 1));
        assert_eq!(unpack(0x1b), (3, 3));
    }
}
