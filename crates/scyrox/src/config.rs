//! Decoded mouse state and the set of changes the driver can apply, shared by
//! the CLI and the GUI.

use anyhow::{Result, bail};

use crate::device::{Info, Mouse};
use crate::protocol::{
    self, CHUNK, FIRE_INTERVAL, FIRE_TIMES, KeyAction, Macro, Sensor, Shortcut, addr,
    decode_report_rate, pair_valid,
};

pub const STAGES: u8 = 8;
pub const KEYS: u8 = 6;
pub const PROFILES: u8 = 4;
pub const KEY_NAMES: [&str; 6] = ["left", "right", "middle", "back", "forward", "bottom"];
pub const REPORT_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];
pub const MAX_DEBOUNCE: u8 = 30;
/// Sleep choices offered by the web driver, in seconds (stored as 10 s units).
pub const SLEEP_OPTIONS: [u32; 7] = [10, 30, 60, 120, 180, 300, 600];
pub const ANGLE_TUNE_RANGE: std::ops::RangeInclusive<i8> = -30..=30;
/// Windows presets from the "Combo key" menu on scyrox.net.
pub const COMBO_PRESETS: [(&str, &str); 16] = [
    ("Dictation", "LWin+H"),
    ("Action Center", "LWin+A"),
    ("Task View", "LWin+Tab"),
    ("Show/Hide Desktop", "LWin+D"),
    ("Screenshot", "LWin+LShift+S"),
    ("Close Window", "LAlt+F4"),
    ("Maximize Window", "LWin+Up"),
    ("Minimize Window", "LWin+Down"),
    ("Cut", "LCtrl+X"),
    ("Copy", "LCtrl+C"),
    ("Paste", "LCtrl+V"),
    ("Undo", "LCtrl+Z"),
    ("Zoom In", "LCtrl+Equal"),
    ("Zoom Out", "LCtrl+Minus"),
    ("New Browser Tab", "LCtrl+T"),
    ("Lock", "LWin+L"),
];

#[derive(Clone, Debug)]
pub struct Settings {
    pub report_rate: u32,
    pub stage_count: u8,
    /// 0-based
    pub current_stage: u8,
    pub dpis: Vec<(u32, u32)>,
    pub colors: Vec<[u8; 3]>,
    pub lod: u8,
    pub debounce: u8,
    pub motion_sync: bool,
    pub angle_snap: bool,
    pub ripple: bool,
    pub competition: Option<bool>,
    pub scan_20k: Option<bool>,
    pub angle_tune: Option<i8>,
    pub sleep_secs: u32,
    pub keys: Vec<KeyAction>,
    /// Combo/media keys for buttons bound to [`KeyAction::Shortcut`].
    pub shortcuts: Vec<Option<Shortcut>>,
    /// Macros for buttons bound to [`KeyAction::Macro`].
    pub macros: Vec<Option<Macro>>,
    // Present in the generic driver code but not shown by scyrox.net.
    /// 0 = LP, 1 = HP
    pub sensor_mode: u8,
    pub release_debounce: Option<u8>,
    pub wheel_debounce: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct Online {
    pub firmware: String,
    pub battery: (u8, bool, u16),
    pub profile: Option<u8>,
    pub long_range: Option<bool>,
    pub settings: Settings,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub product: String,
    pub product_id: u16,
    pub info: Info,
    pub sensor: Sensor,
    pub dongle_version: Option<String>,
    /// `None` when the receiver is present but the mouse is asleep/off.
    pub mouse: Option<Online>,
}

#[derive(Clone, Debug)]
pub enum Change {
    ReportRate(u32),
    /// 0-based stage, DPI
    Dpi(u8, u32),
    StageCount(u8),
    CurrentStage(u8),
    DpiColor(u8, [u8; 3]),
    Lod(u8),
    Debounce(u8),
    ReleaseDebounce(u8),
    WheelDebounce(u8),
    MotionSync(bool),
    AngleSnap(bool),
    Ripple(bool),
    Competition(bool),
    Scan20k(bool),
    SensorMode(u8),
    AngleTune(i8),
    SleepSecs(u32),
    LongRange(bool),
    /// Simple action or fire key for a button.
    Key(u8, KeyAction),
    /// Bind a combo or media key to a button.
    Shortcut(u8, Shortcut),
    /// Bind a macro to a button with a repeat mode (see `protocol::cycle`).
    Macro(u8, Macro, u8),
    /// 0-based profile
    Profile(u8),
}

impl Change {
    /// Whether the GUI should re-read everything after applying this change.
    pub fn needs_reload(&self) -> bool {
        matches!(self, Change::Profile(_))
    }
}

pub fn lod_options(sensor: Sensor) -> &'static [(u8, &'static str)] {
    match sensor {
        Sensor::Paw3950 => &[(3, "0.7 mm"), (1, "1 mm"), (2, "2 mm")],
        Sensor::Paw3955 => &[(1, "0.7 mm"), (2, "0.9 mm"), (3, "1.2 mm"), (4, "1.4 mm"), (5, "1.6 mm")],
    }
}

/// Flash regions that make up one profile, for backup/restore.
fn profile_regions(sensor: Sensor) -> Vec<(u16, usize)> {
    let mut r = vec![
        (0, 256),
        (addr::SHORTCUT, KEYS as usize * Shortcut::BLOCK as usize),
        (addr::MACRO, KEYS as usize * Macro::BLOCK as usize),
    ];
    if sensor == Sensor::Paw3955 {
        r.push((addr::SENSOR_3955_DPI, STAGES as usize * 6));
    }
    r
}

const BACKUP_MAGIC: &[u8; 4] = b"SCYX";
const BACKUP_VERSION: u8 = 1;

impl Mouse {
    /// Reads `first` bytes, then the rest of a variable-length block whose
    /// total size `total` computes from what was read (like `Read_Macro`).
    async fn read_block(&self, start: u16, first: usize, total: impl Fn(&[u8]) -> usize) -> Result<Vec<u8>> {
        let mut b = self.read_flash(start, first).await?;
        let len = total(&b);
        if len > b.len() {
            b.extend(self.read_flash(start + b.len() as u16, len - b.len()).await?);
        }
        Ok(b)
    }

    async fn read_shortcut(&self, key: u8) -> Result<Option<Shortcut>> {
        let start = addr::SHORTCUT + key as u16 * Shortcut::BLOCK;
        let b = self.read_block(start, CHUNK, |b| {
            ((b[0] as usize / 2) * 6 + 2).min(Shortcut::BLOCK as usize)
        }).await?;
        Ok(Shortcut::decode(&b))
    }

    async fn read_macro(&self, key: u8) -> Result<Option<Macro>> {
        let start = addr::MACRO + key as u16 * Macro::BLOCK;
        let b = self.read_block(start, 32 + CHUNK, |b| {
            33 + (b[31] as usize).min(Macro::MAX_EVENTS) * 5
        }).await?;
        Ok(Macro::decode(&b))
    }

    pub async fn read_settings(&self) -> Result<Settings> {
        let sensor = self.sensor();
        let f = self.read_flash(0, 256).await?;
        let dpi_blocks = match sensor {
            Sensor::Paw3950 => f[addr::DPI_VALUE as usize..][..32].to_vec(),
            Sensor::Paw3955 => self.read_flash(addr::SENSOR_3955_DPI, 48).await?,
        };
        let block = dpi_blocks.len() / STAGES as usize;
        let at = |a: u16| f[a as usize];
        let opt = |a: u16| pair_valid(at(a), at(a + 1)).then(|| at(a));
        let keys: Vec<KeyAction> = (0..KEYS as u16)
            .map(|k| {
                let b = &f[(addr::KEY_FUNCTION + k * 4) as usize..];
                KeyAction::decode(b[0], u16::from_be_bytes([b[1], b[2]]))
            })
            .collect();
        let mut shortcuts = vec![None; KEYS as usize];
        let mut macros = vec![None; KEYS as usize];
        for (k, a) in keys.iter().enumerate() {
            match a {
                KeyAction::Shortcut => shortcuts[k] = self.read_shortcut(k as u8).await?,
                KeyAction::Macro { .. } => macros[k] = self.read_macro(k as u8).await?,
                _ => {}
            }
        }
        Ok(Settings {
            report_rate: decode_report_rate(at(addr::REPORT_RATE)),
            stage_count: at(addr::DPI_STAGE_COUNT).clamp(1, STAGES),
            current_stage: at(addr::CURRENT_DPI).min(STAGES - 1),
            dpis: (0..STAGES as usize)
                .map(|s| sensor.decode_dpi(&dpi_blocks[s * block..]))
                .collect(),
            colors: (0..STAGES as u16)
                .map(|s| f[(addr::DPI_COLOR + s * 4) as usize..][..3].try_into().unwrap())
                .collect(),
            lod: at(addr::LOD),
            debounce: at(addr::DEBOUNCE),
            motion_sync: at(addr::MOTION_SYNC) == 1,
            angle_snap: at(addr::ANGLE_SNAP) == 1,
            ripple: at(addr::RIPPLE) == 1,
            competition: opt(addr::COMPETITION).map(|v| v == 1),
            scan_20k: opt(addr::SCAN_20K).map(|v| v == 1),
            angle_tune: opt(addr::ANGLE_TUNE_STATE)
                .and(opt(addr::ANGLE_TUNE))
                .map(|v| v as i8),
            sleep_secs: at(addr::SLEEP_TIME) as u32 * 10,
            keys,
            shortcuts,
            macros,
            sensor_mode: at(addr::SENSOR_MODE),
            release_debounce: opt(addr::DEBOUNCE_RELEASE),
            wheel_debounce: opt(addr::WHEEL_DEBOUNCE),
        })
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let mouse = if self.online().await? {
            Some(Online {
                firmware: self.version().await?,
                battery: self.battery().await?,
                profile: self.profile().await?,
                long_range: if self.info.wired() { None } else { self.long_range().await? },
                settings: self.read_settings().await?,
            })
        } else {
            None
        };
        Ok(Snapshot {
            product: self.product.clone(),
            product_id: self.product_id,
            info: self.info,
            sensor: self.sensor(),
            dongle_version: self.dongle_version().await?,
            mouse,
        })
    }

    /// Validates and writes one setting. Caller must check the mouse is online.
    pub async fn apply(&self, change: &Change) -> Result<()> {
        let sensor = self.sensor();
        let stage = |s: u8| {
            if s >= STAGES {
                bail!("stage must be 1-{STAGES}");
            }
            Ok(s)
        };
        let key = |k: u8| {
            if k >= KEYS {
                bail!("unknown button");
            }
            Ok(k)
        };
        match change {
            &Change::ReportRate(hz) => {
                let Some(v) = protocol::encode_report_rate(hz) else {
                    bail!("unsupported rate {hz}");
                };
                if hz > self.info.max_report_rate() {
                    bail!("this connection supports at most {} Hz", self.info.max_report_rate());
                }
                self.write_setting(addr::REPORT_RATE, v).await
            }
            &Change::Dpi(s, dpi) => {
                sensor.validate_dpi(dpi).map_err(anyhow::Error::msg)?;
                self.write_flash(sensor.dpi_addr(stage(s)?), &sensor.encode_dpi(dpi)).await
            }
            &Change::StageCount(n) => {
                if !(1..=STAGES).contains(&n) {
                    bail!("stage count must be 1-{STAGES}");
                }
                self.write_setting(addr::DPI_STAGE_COUNT, n).await
            }
            &Change::CurrentStage(s) => self.write_setting(addr::CURRENT_DPI, stage(s)?).await,
            &Change::DpiColor(s, [r, g, b]) => {
                let mut block = [r, g, b, 0];
                block[3] = protocol::data_checksum(&block[..3]);
                self.write_flash(addr::DPI_COLOR + stage(s)? as u16 * 4, &block).await
            }
            &Change::Lod(v) => self.write_setting(addr::LOD, v).await,
            &Change::Debounce(ms) => self.write_setting(addr::DEBOUNCE, ms.min(MAX_DEBOUNCE)).await,
            &Change::ReleaseDebounce(ms) => {
                self.write_setting(addr::DEBOUNCE_RELEASE, ms.min(MAX_DEBOUNCE)).await
            }
            &Change::WheelDebounce(ms) => {
                self.write_setting(addr::WHEEL_DEBOUNCE, ms.min(MAX_DEBOUNCE)).await
            }
            &Change::MotionSync(on) => self.write_setting(addr::MOTION_SYNC, on as u8).await,
            &Change::AngleSnap(on) => self.write_setting(addr::ANGLE_SNAP, on as u8).await,
            &Change::Ripple(on) => self.write_setting(addr::RIPPLE, on as u8).await,
            &Change::Competition(on) => self.write_setting(addr::COMPETITION, on as u8).await,
            &Change::Scan20k(on) => self.write_setting(addr::SCAN_20K, on as u8).await,
            &Change::SensorMode(m) => {
                if m > 1 {
                    bail!("sensor mode must be LP (0) or HP (1)");
                }
                self.write_setting(addr::SENSOR_MODE, m).await
            }
            &Change::AngleTune(deg) => {
                if !ANGLE_TUNE_RANGE.contains(&deg) {
                    bail!("angle tune must be -30..30 degrees");
                }
                self.write_setting(addr::ANGLE_TUNE_STATE, 1).await?;
                self.write_setting(addr::ANGLE_TUNE, deg as u8).await
            }
            &Change::SleepSecs(secs) => {
                // The Scyrox UI writes the same value to both slots.
                let v = (secs / 10).clamp(1, 255) as u8;
                self.write_setting(addr::SLEEP_TIME, v).await?;
                self.write_setting(addr::PERFORMANCE, v).await
            }
            &Change::LongRange(on) => {
                if self.info.wired() {
                    bail!("long-range mode only applies to the wireless receiver");
                }
                self.set_long_range(on).await
            }
            &Change::Key(k, action) => {
                match action {
                    KeyAction::Shortcut | KeyAction::Macro { .. } => {
                        bail!("use Change::Shortcut / Change::Macro to bind these")
                    }
                    KeyAction::Fire { interval, times }
                        if !FIRE_INTERVAL.contains(&interval) || !FIRE_TIMES.contains(&times) =>
                    {
                        bail!("fire key: interval 10-255 ms, times 0-3")
                    }
                    _ => {}
                }
                let k = key(k)?;
                self.write_flash(addr::KEY_FUNCTION + k as u16 * 4, &action.flash_block(k)).await
            }
            Change::Shortcut(k, s) => {
                let k = key(*k)?;
                if s.0.is_empty() || s.0.len() > Shortcut::MAX_KEYS {
                    bail!("a combo key has 1-{} keys", Shortcut::MAX_KEYS);
                }
                // Same order as the web driver: keys first, then the binding.
                self.write_flash(addr::SHORTCUT + k as u16 * Shortcut::BLOCK, &s.encode()).await?;
                let block = KeyAction::Shortcut.flash_block(k);
                self.write_flash(addr::KEY_FUNCTION + k as u16 * 4, &block).await
            }
            Change::Macro(k, m, cycle) => {
                let k = key(*k)?;
                if m.name.trim().is_empty() {
                    bail!("macro needs a name");
                }
                if m.events.is_empty() || m.events.len() > Macro::MAX_EVENTS {
                    bail!("a macro has 1-{} events", Macro::MAX_EVENTS);
                }
                if *cycle == 0 {
                    bail!("macro repeat count must be at least 1");
                }
                // Same order as the web driver: binding first, then the macro.
                let block = KeyAction::Macro { cycle: *cycle }.flash_block(k);
                self.write_flash(addr::KEY_FUNCTION + k as u16 * 4, &block).await?;
                self.write_flash(addr::MACRO + k as u16 * Macro::BLOCK, &m.encode()).await
            }
            &Change::Profile(p) => {
                if p >= PROFILES {
                    bail!("profile must be 1-{PROFILES}");
                }
                if self.profile().await?.is_none() {
                    bail!("this device does not support onboard profiles");
                }
                self.set_profile(p).await
            }
        }
    }

    /// Saves the active profile (settings, buttons, combo keys and macros).
    pub async fn backup(&self) -> Result<Vec<u8>> {
        let sensor = self.sensor();
        let mut out = BACKUP_MAGIC.to_vec();
        out.extend([BACKUP_VERSION, self.info.cid, self.info.mid, sensor as u8]);
        for (start, len) in profile_regions(sensor) {
            out.extend(self.read_flash(start, len).await?);
        }
        Ok(out)
    }

    /// Writes a backup into the active profile, touching only changed chunks.
    pub async fn restore(&self, data: &[u8]) -> Result<()> {
        let sensor = self.sensor();
        if data.len() < 8 || &data[..4] != BACKUP_MAGIC || data[4] != BACKUP_VERSION {
            bail!("not a Scyrox backup file");
        }
        if data[5] != self.info.cid || data[7] != sensor as u8 {
            bail!("this backup was made for a different mouse model");
        }
        let regions = profile_regions(sensor);
        let expected: usize = 8 + regions.iter().map(|r| r.1).sum::<usize>();
        if data.len() != expected {
            bail!("backup file is truncated or corrupt");
        }
        let mut body = &data[8..];
        for (start, len) in regions {
            self.write_diff(start, &body[..len]).await?;
            body = &body[len..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_presets_parse() {
        for (name, keys) in COMBO_PRESETS {
            let s = Shortcut::parse(keys).unwrap_or_else(|| panic!("{name}: {keys}"));
            assert!(!s.is_media());
        }
    }
}
