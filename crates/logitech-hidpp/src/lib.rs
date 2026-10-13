//! Logitech HID++ 2.0 over the long-report vendor collection
//! (usage page 0xFF00, usage 0x0002, report id 0x11, 20 bytes).
//!
//! A message is `[0x11, device index, feature index, function << 4 | sw id,
//! params[16]]`. Device index 0xFF addresses a directly connected device;
//! 1-6 address devices paired to a receiver. Feature indexes are looked up
//! from feature ids through the root feature (index 0).
//!
//! ```no_run
//! let api = hidapi::HidApi::new()?;
//! for dev in logitech_hidpp::discover(&api) {
//!     let mut dev = dev?;
//!     println!("{} battery {:?}", dev.name()?, dev.battery()?);
//! }
//! # anyhow::Ok(())
//! ```

pub mod profile;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hidapi::{HidApi, HidDevice};

pub const VENDOR_ID: u16 = 0x046D;
pub const USAGE_PAGE: u16 = 0xFF00;
pub const USAGE_LONG: u16 = 0x0002;
const REPORT_LONG: u8 = 0x11;
const SW_ID: u8 = 0x0B;
const TIMEOUT: Duration = Duration::from_millis(500);

/// Feature ids used here.
pub mod feature {
    pub const ROOT: u16 = 0x0000;
    pub const FEATURE_SET: u16 = 0x0001;
    pub const FW_VERSION: u16 = 0x0003;
    pub const DEVICE_NAME: u16 = 0x0005;
    pub const BATTERY_STATUS: u16 = 0x1000;
    pub const UNIFIED_BATTERY: u16 = 0x1004;
    pub const ADJUSTABLE_DPI: u16 = 0x2201;
    pub const REPORT_RATE: u16 = 0x8060;
    pub const ONBOARD_PROFILES: u16 = 0x8100;
}

/// One HID++ long-report channel; shared by every device behind a receiver.
pub type Channel = Arc<Mutex<HidDevice>>;

pub type Params = [u8; 16];

pub struct Device {
    channel: Channel,
    pub index: u8,
    pub product_id: u16,
    /// Feature id -> index (None = not supported), filled lazily.
    features: HashMap<u16, Option<u8>>,
    pub protocol: (u8, u8),
}

/// Opens every HID++ 2.0 device reachable through Logitech long-report
/// collections (directly connected devices and receiver pairings).
pub fn discover(api: &HidApi) -> Vec<Result<Device>> {
    let mut out = Vec::new();
    for info in api.device_list().filter(|d| {
        d.vendor_id() == VENDOR_ID && d.usage_page() == USAGE_PAGE && d.usage() == USAGE_LONG
    }) {
        let channel: Channel = match info.open_device(api) {
            Ok(d) => Arc::new(Mutex::new(d)),
            Err(e) => {
                out.push(Err(anyhow::anyhow!(e).context("opening Logitech HID++ interface")));
                continue;
            }
        };
        for index in [0xFF, 1, 2, 3, 4, 5, 6] {
            if let Ok(dev) = Device::open(channel.clone(), index, info.product_id()) {
                out.push(Ok(dev));
            }
        }
    }
    out
}

fn exchange(dev: &HidDevice, index: u8, feat: u8, func: u8, params: &[u8]) -> Result<Params> {
    let mut msg = [0u8; 20];
    msg[0] = REPORT_LONG;
    msg[1] = index;
    msg[2] = feat;
    msg[3] = func << 4 | SW_ID;
    msg[4..4 + params.len()].copy_from_slice(params);
    dev.write(&msg).context("HID write")?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            bail!("no reply from device {index} (asleep or out of range?)");
        }
        let mut r = [0u8; 64];
        let n = dev.read_timeout(&mut r, left.as_millis() as i32).context("HID read")?;
        if n < 7 || r[1] != index {
            continue;
        }
        if r[2] == 0xFF && r[3] == feat && r[4] == msg[3] {
            bail!("device error {:#04x} ({})", r[5], error_name(r[5]));
        }
        if r[2] == 0x8F && r[3] == feat {
            bail!("HID++ 1.0 error {:#04x}", r[5]);
        }
        if r[0] == REPORT_LONG && r[2] == feat && r[3] == msg[3] {
            let mut p = [0u8; 16];
            p.copy_from_slice(&r[4..20]);
            return Ok(p);
        }
    }
}

fn error_name(code: u8) -> &'static str {
    match code {
        1 => "unknown",
        2 => "invalid argument",
        3 => "out of range",
        4 => "hardware error",
        5 => "logitech internal",
        6 => "invalid feature index",
        7 => "invalid function",
        8 => "busy",
        9 => "unsupported",
        _ => "?",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charging {
    Discharging,
    Charging,
    Full,
    Error,
}

#[derive(Clone, Copy, Debug)]
pub struct Battery {
    pub percent: Option<u8>,
    pub charging: Charging,
}

#[derive(Clone, Debug)]
pub struct DpiInfo {
    pub min: u16,
    pub max: u16,
    pub step: u16,
    /// Explicit list when the sensor has no range.
    pub values: Vec<u16>,
}

#[derive(Clone, Copy, Debug)]
pub struct OnboardInfo {
    pub profile_format: u8,
    pub profile_count: u8,
    pub button_count: u8,
    pub sector_size: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Settings come from the profiles stored on the device.
    Onboard = 1,
    /// Settings are controlled live by software (G HUB uses this).
    Host = 2,
}

/// Directory sector listing profile sectors and whether they are enabled.
pub const DIRECTORY_SECTOR: u16 = 0x0000;
/// Read-only factory copy of profile `n` (1-based) is at `ROM_BASE + n`.
pub const ROM_BASE: u16 = 0x0100;

impl Device {
    pub fn open(channel: Channel, index: u8, product_id: u16) -> Result<Device> {
        let mut dev = Device { channel, index, product_id, features: HashMap::new(), protocol: (0, 0) };
        let p = dev.call_index(0, 1, &[0, 0, 0xAA])?;
        if p[2] != 0xAA {
            bail!("bad ping reply");
        }
        dev.protocol = (p[0], p[1]);
        if dev.protocol.0 < 2 {
            bail!("device speaks HID++ {}.{}, not 2.0", p[0], p[1]);
        }
        Ok(dev)
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    fn call_index(&self, feat: u8, func: u8, params: &[u8]) -> Result<Params> {
        let dev = self.channel.lock().unwrap();
        exchange(&dev, self.index, feat, func, params)
    }

    /// Index of a feature, `None` if the device lacks it.
    pub fn feature_index(&mut self, id: u16) -> Result<Option<u8>> {
        if let Some(i) = self.features.get(&id) {
            return Ok(*i);
        }
        let p = self.call_index(0, 0, &id.to_be_bytes())?;
        let idx = (p[0] != 0 || id == feature::ROOT).then_some(p[0]);
        self.features.insert(id, idx);
        Ok(idx)
    }

    pub fn has(&mut self, id: u16) -> Result<bool> {
        Ok(self.feature_index(id)?.is_some())
    }

    /// Calls `func` of feature `id`.
    pub fn call(&mut self, id: u16, func: u8, params: &[u8]) -> Result<Params> {
        let Some(idx) = self.feature_index(id)? else { bail!("device lacks feature {id:#06x}") };
        self.call_index(idx, func, params)
    }

    /// True when the device sent unsolicited notifications (e.g. battery or
    /// profile changes) since the last call. Non-blocking.
    pub fn poll_notifications(&self) -> bool {
        let dev = self.channel.lock().unwrap();
        let mut seen = false;
        let mut r = [0u8; 64];
        while let Ok(n) = dev.read_timeout(&mut r, 0) {
            if n == 0 {
                break;
            }
            if n >= 4 && r[1] == self.index && r[3] & 0x0F == 0 {
                seen = true;
            }
        }
        seen
    }

    pub fn name(&mut self) -> Result<String> {
        let len = self.call(feature::DEVICE_NAME, 0, &[])?[0] as usize;
        let mut name = Vec::with_capacity(len);
        while name.len() < len {
            let p = self.call(feature::DEVICE_NAME, 1, &[name.len() as u8])?;
            let take = (len - name.len()).min(16);
            name.extend_from_slice(&p[..take]);
        }
        Ok(String::from_utf8_lossy(&name).trim_end_matches('\0').to_string())
    }

    /// Main firmware as "MPM25.01 build 0018".
    pub fn firmware(&mut self) -> Result<Option<String>> {
        if !self.has(feature::FW_VERSION)? {
            return Ok(None);
        }
        let count = self.call(feature::FW_VERSION, 0, &[])?[0];
        for i in 0..count {
            let p = self.call(feature::FW_VERSION, 1, &[i])?;
            if p[0] == 0 {
                return Ok(Some(format!(
                    "{}{:02x}.{:02x} build {:02x}{:02x}",
                    String::from_utf8_lossy(&p[1..4]),
                    p[4],
                    p[5],
                    p[6],
                    p[7]
                )));
            }
        }
        Ok(None)
    }

    pub fn battery(&mut self) -> Result<Option<Battery>> {
        if self.has(feature::UNIFIED_BATTERY)? {
            let caps = self.call(feature::UNIFIED_BATTERY, 0, &[])?;
            let s = self.call(feature::UNIFIED_BATTERY, 1, &[])?;
            // Capability flag bit 1: state of charge in percent is reported.
            let percent = (caps[1] & 0x02 != 0).then_some(s[0]);
            let charging = match s[2] {
                0 => Charging::Discharging,
                1 | 2 => Charging::Charging,
                3 => Charging::Full,
                _ => Charging::Error,
            };
            return Ok(Some(Battery { percent, charging }));
        }
        if self.has(feature::BATTERY_STATUS)? {
            let s = self.call(feature::BATTERY_STATUS, 0, &[])?;
            let charging = match s[2] {
                0 => Charging::Discharging,
                1 | 2 | 4 => Charging::Charging,
                3 => Charging::Full,
                _ => Charging::Error,
            };
            return Ok(Some(Battery { percent: Some(s[0]), charging }));
        }
        Ok(None)
    }

    // ------------------------------------------------------------ DPI (0x2201)

    pub fn dpi_info(&mut self) -> Result<Option<DpiInfo>> {
        if !self.has(feature::ADJUSTABLE_DPI)? {
            return Ok(None);
        }
        let p = self.call(feature::ADJUSTABLE_DPI, 1, &[0])?;
        // List of big-endian u16 after the sensor index; 0xE000|step marks a
        // range between the neighbouring values; 0 ends the list.
        let words: Vec<u16> =
            p[1..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).take_while(|w| *w != 0).collect();
        let mut info = DpiInfo { min: 0, max: 0, step: 0, values: Vec::new() };
        if let [a, s, b, ..] = words[..] {
            if s & 0xE000 == 0xE000 {
                info.min = a;
                info.step = s & 0x1FFF;
                info.max = b;
                return Ok(Some(info));
            }
        }
        info.values = words;
        info.min = info.values.first().copied().unwrap_or(0);
        info.max = info.values.last().copied().unwrap_or(0);
        Ok(Some(info))
    }

    pub fn dpi(&mut self) -> Result<u16> {
        let p = self.call(feature::ADJUSTABLE_DPI, 2, &[0])?;
        Ok(u16::from_be_bytes([p[1], p[2]]))
    }

    pub fn set_dpi(&mut self, dpi: u16) -> Result<()> {
        let [hi, lo] = dpi.to_be_bytes();
        self.call(feature::ADJUSTABLE_DPI, 3, &[0, hi, lo])?;
        Ok(())
    }

    // ---------------------------------------------------- report rate (0x8060)

    /// Supported intervals in ms (1 = 1000 Hz).
    pub fn report_intervals(&mut self) -> Result<Option<Vec<u8>>> {
        if !self.has(feature::REPORT_RATE)? {
            return Ok(None);
        }
        let mask = self.call(feature::REPORT_RATE, 0, &[])?[0];
        Ok(Some((0..8).filter(|b| mask >> b & 1 == 1).map(|b| b + 1).collect()))
    }

    pub fn report_interval(&mut self) -> Result<u8> {
        Ok(self.call(feature::REPORT_RATE, 1, &[])?[0])
    }

    pub fn set_report_interval(&mut self, ms: u8) -> Result<()> {
        self.call(feature::REPORT_RATE, 2, &[ms])?;
        Ok(())
    }

    // ----------------------------------------------- onboard profiles (0x8100)

    pub fn onboard_info(&mut self) -> Result<Option<OnboardInfo>> {
        if !self.has(feature::ONBOARD_PROFILES)? {
            return Ok(None);
        }
        let p = self.call(feature::ONBOARD_PROFILES, 0, &[])?;
        Ok(Some(OnboardInfo {
            profile_format: p[1],
            profile_count: p[3],
            button_count: p[5],
            sector_size: u16::from_be_bytes([p[7], p[8]]),
        }))
    }

    pub fn mode(&mut self) -> Result<Mode> {
        Ok(match self.call(feature::ONBOARD_PROFILES, 2, &[])?[0] {
            1 => Mode::Onboard,
            _ => Mode::Host,
        })
    }

    pub fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 1, &[mode as u8])?;
        Ok(())
    }

    /// Sector of the active profile (0 when none, e.g. in host mode).
    pub fn current_profile(&mut self) -> Result<u16> {
        let p = self.call(feature::ONBOARD_PROFILES, 4, &[])?;
        Ok(u16::from_be_bytes([p[0], p[1]]))
    }

    pub fn set_current_profile(&mut self, sector: u16) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 3, &sector.to_be_bytes())?;
        Ok(())
    }

    pub fn current_dpi_index(&mut self) -> Result<u8> {
        Ok(self.call(feature::ONBOARD_PROFILES, 11, &[])?[0])
    }

    pub fn set_current_dpi_index(&mut self, i: u8) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 12, &[i])?;
        Ok(())
    }

    /// Reads a whole sector (reads are 16 bytes; the last one is aligned to
    /// the end because reading past the sector is rejected).
    pub fn read_sector(&mut self, sector: u16, size: u16) -> Result<Vec<u8>> {
        let size = size as usize;
        let mut out = Vec::with_capacity(size);
        while out.len() < size {
            let off = out.len().min(size.saturating_sub(16));
            let mut req = sector.to_be_bytes().to_vec();
            req.extend((off as u16).to_be_bytes());
            let p = self.call(feature::ONBOARD_PROFILES, 5, &req)?;
            let skip = out.len() - off;
            out.extend_from_slice(&p[skip..16.min(size - off)]);
        }
        Ok(out)
    }

    /// Writes a whole sector; `data` must already carry its CRC.
    pub fn write_sector(&mut self, sector: u16, data: &[u8]) -> Result<()> {
        if sector & ROM_BASE != 0 {
            bail!("sector {sector:#06x} is read-only");
        }
        let mut start = sector.to_be_bytes().to_vec();
        start.extend(0u16.to_be_bytes());
        start.extend((data.len() as u16).to_be_bytes());
        self.call(feature::ONBOARD_PROFILES, 6, &start)?;
        for chunk in data.chunks(16) {
            let mut c = [0u8; 16];
            c[..chunk.len()].copy_from_slice(chunk);
            self.call(feature::ONBOARD_PROFILES, 7, &c)?;
        }
        self.call(feature::ONBOARD_PROFILES, 8, &[])?;
        Ok(())
    }

    /// Profile sectors listed in the directory with their enabled flag.
    pub fn profile_directory(&mut self, info: &OnboardInfo) -> Result<Vec<(u16, bool)>> {
        let dir = self.read_sector(DIRECTORY_SECTOR, info.sector_size)?;
        Ok(dir
            .chunks(4)
            .take(info.profile_count as usize)
            .take_while(|e| !(e[0] == 0xFF && e[1] == 0xFF))
            .map(|e| (u16::from_be_bytes([e[0], e[1]]), e[2] != 0))
            .collect())
    }
}
