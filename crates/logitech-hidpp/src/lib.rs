//! Logitech HID++ 2.0 over the long-report vendor collection
//! (usage page 0xFF00, usage 0x0002, report id 0x11, 20 bytes).
//!
//! A message is `[0x11, device index, feature index, function << 4 | sw id,
//! params[16]]`. Device index 0xFF addresses a directly connected device;
//! 1-6 address devices paired to a receiver. Feature indexes are looked up
//! from feature ids through the root feature (index 0).
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let api = hid_transport::Api::new().await?;
//! for dev in logitech_hidpp::discover(&api).await {
//!     let mut dev = dev?;
//!     println!("{} battery {:?}", dev.name().await?, dev.battery().await?);
//! }
//! # Ok(())
//! # }
//! ```

pub mod profile;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hid_transport::{Api, Device as HidDevice, Filter, Instant};

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
    pub const EXT_ADJUSTABLE_DPI: u16 = 0x2202;
    pub const REPORT_RATE: u16 = 0x8060;
    pub const EXT_REPORT_RATE: u16 = 0x8061;
    pub const ONBOARD_PROFILES: u16 = 0x8100;
}

/// One HID++ long-report channel; shared by every device behind a receiver.
pub type Channel = Rc<HidDevice>;

/// The long-report collection of receivers and directly connected devices.
pub const FILTER: Filter = Filter { vendor_id: Some(VENDOR_ID), product_id: None, usage_page: Some(USAGE_PAGE), usage: Some(USAGE_LONG) };

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
pub async fn discover(api: &Api) -> Vec<Result<Device>> {
    let mut out = Vec::new();
    for info in api.devices().filter(|d| FILTER.matches(d)) {
        let channel: Channel = match api.open(info).await {
            Ok(d) => Rc::new(d),
            Err(e) => {
                out.push(Err(e.context("opening Logitech HID++ interface")));
                continue;
            }
        };
        for index in [0xFF, 1, 2, 3, 4, 5, 6] {
            if let Ok(dev) = Device::open(channel.clone(), index, info.product_id).await {
                out.push(Ok(dev));
            }
        }
    }
    out
}

async fn exchange(dev: &HidDevice, index: u8, feat: u8, func: u8, params: &[u8]) -> Result<Params> {
    let mut msg = [0u8; 20];
    msg[0] = REPORT_LONG;
    msg[1] = index;
    msg[2] = feat;
    msg[3] = func << 4 | SW_ID;
    msg[4..4 + params.len()].copy_from_slice(params);
    dev.write(&msg).await.context("HID write")?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            bail!("no reply from device {index} (asleep or out of range?)");
        }
        let mut r = [0u8; 64];
        let n = dev.read_timeout(&mut r, left).await.context("HID read")?;
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
    pub async fn open(channel: Channel, index: u8, product_id: u16) -> Result<Device> {
        let mut dev = Device { channel, index, product_id, features: HashMap::new(), protocol: (0, 0) };
        let p = dev.call_index(0, 1, &[0, 0, 0xAA]).await?;
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

    async fn call_index(&self, feat: u8, func: u8, params: &[u8]) -> Result<Params> {
        exchange(&self.channel, self.index, feat, func, params).await
    }

    /// Index of a feature, `None` if the device lacks it.
    pub async fn feature_index(&mut self, id: u16) -> Result<Option<u8>> {
        if let Some(i) = self.features.get(&id) {
            return Ok(*i);
        }
        let p = self.call_index(0, 0, &id.to_be_bytes()).await?;
        let idx = (p[0] != 0 || id == feature::ROOT).then_some(p[0]);
        self.features.insert(id, idx);
        Ok(idx)
    }

    /// Every feature the device reports: (index, id, type flags, version).
    pub async fn feature_list(&mut self) -> Result<Vec<(u8, u16, u8, u8)>> {
        let Some(fs) = self.feature_index(feature::FEATURE_SET).await? else { return Ok(Vec::new()) };
        let count = self.call_index(fs, 0, &[]).await?[0];
        let mut out = Vec::new();
        for i in 0..=count {
            let p = self.call_index(fs, 1, &[i]).await?;
            out.push((i, u16::from_be_bytes([p[0], p[1]]), p[2], p[3]));
        }
        Ok(out)
    }

    pub async fn has(&mut self, id: u16) -> Result<bool> {
        Ok(self.feature_index(id).await?.is_some())
    }

    /// Calls `func` of feature `id`.
    pub async fn call(&mut self, id: u16, func: u8, params: &[u8]) -> Result<Params> {
        let Some(idx) = self.feature_index(id).await? else { bail!("device lacks feature {id:#06x}") };
        self.call_index(idx, func, params).await
    }

    /// True when the device sent unsolicited notifications (e.g. battery or
    /// profile changes) since the last call. Non-blocking.
    pub async fn poll_notifications(&self) -> bool {
        let mut seen = false;
        let mut r = [0u8; 64];
        while let Ok(n) = self.channel.read_timeout(&mut r, Duration::ZERO).await {
            if n == 0 {
                break;
            }
            if n >= 4 && r[1] == self.index && r[3] & 0x0F == 0 {
                seen = true;
            }
        }
        seen
    }

    pub async fn name(&mut self) -> Result<String> {
        let len = self.call(feature::DEVICE_NAME, 0, &[]).await?[0] as usize;
        let mut name = Vec::with_capacity(len);
        while name.len() < len {
            let p = self.call(feature::DEVICE_NAME, 1, &[name.len() as u8]).await?;
            let take = (len - name.len()).min(16);
            name.extend_from_slice(&p[..take]);
        }
        Ok(String::from_utf8_lossy(&name).trim_end_matches('\0').to_string())
    }

    /// Main firmware as "MPM25.01 build 0018".
    pub async fn firmware(&mut self) -> Result<Option<String>> {
        if !self.has(feature::FW_VERSION).await? {
            return Ok(None);
        }
        let count = self.call(feature::FW_VERSION, 0, &[]).await?[0];
        for i in 0..count {
            let p = self.call(feature::FW_VERSION, 1, &[i]).await?;
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

    pub async fn battery(&mut self) -> Result<Option<Battery>> {
        if self.has(feature::UNIFIED_BATTERY).await? {
            let caps = self.call(feature::UNIFIED_BATTERY, 0, &[]).await?;
            let s = self.call(feature::UNIFIED_BATTERY, 1, &[]).await?;
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
        if self.has(feature::BATTERY_STATUS).await? {
            let s = self.call(feature::BATTERY_STATUS, 0, &[]).await?;
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

    // ---------------------------------------- DPI (0x2201, extended 0x2202)

    /// Expands a DPI list: u16 big-endian values, `0xE000 | step` meaning
    /// "every `step` up to the next value", terminated by 0.
    fn expand_dpi_list(words: &[u16]) -> Vec<u16> {
        let mut out: Vec<u16> = Vec::new();
        let mut i = 0;
        while i < words.len() && words[i] != 0 {
            let w = words[i];
            if w >> 13 == 0b111 && i + 1 < words.len() {
                let (step, last) = (w & 0x1FFF, words[i + 1]);
                if let Some(&start) = out.last() {
                    if step > 0 {
                        out.extend((start + step..=last).step_by(step as usize));
                    }
                }
                i += 2;
            } else {
                out.push(w);
                i += 1;
            }
        }
        out
    }

    /// Big-endian u16 words from reply bytes, ignoring a trailing odd byte.
    fn dpi_words(bytes: &[u8]) -> Vec<u16> {
        bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
    }

    fn dpi_info_from(list: Vec<u16>) -> Option<DpiInfo> {
        let (&min, &max) = (list.first()?, list.last()?);
        let step = list.get(1).map_or(1, |s| s - min);
        let even = list.windows(2).all(|w| w[1] - w[0] == step);
        Some(if even && step > 0 {
            DpiInfo { min, max, step, values: Vec::new() }
        } else {
            DpiInfo { min, max, step: 0, values: list }
        })
    }

    pub async fn dpi_info(&mut self) -> Result<Option<DpiInfo>> {
        if self.has(feature::ADJUSTABLE_DPI).await? {
            let p = self.call(feature::ADJUSTABLE_DPI, 1, &[0]).await?;
            return Ok(Self::dpi_info_from(Self::expand_dpi_list(&Self::dpi_words(&p[1..]))));
        }
        if self.has(feature::EXT_ADJUSTABLE_DPI).await? {
            // getSensorDpiRanges(sensor 0, direction X, page n): data from byte 3,
            // continued over pages until the list ends with 0x0000.
            let mut bytes = Vec::new();
            for page in 0..16u8 {
                let p = self.call(feature::EXT_ADJUSTABLE_DPI, 2, &[0, 0, page]).await?;
                bytes.extend_from_slice(&p[3..]);
                if bytes.ends_with(&[0, 0]) || bytes.chunks(2).any(|c| c == [0, 0]) {
                    break;
                }
            }
            return Ok(Self::dpi_info_from(Self::expand_dpi_list(&Self::dpi_words(&bytes))));
        }
        Ok(None)
    }

    pub async fn dpi(&mut self) -> Result<u16> {
        if self.has(feature::ADJUSTABLE_DPI).await? {
            let p = self.call(feature::ADJUSTABLE_DPI, 2, &[0]).await?;
            return Ok(u16::from_be_bytes([p[1], p[2]]));
        }
        // Extended: [sensor, x (0 = use default), default x, y, default y, lod]
        let p = self.call(feature::EXT_ADJUSTABLE_DPI, 5, &[0]).await?;
        let x = u16::from_be_bytes([p[1], p[2]]);
        Ok(if x == 0 { u16::from_be_bytes([p[3], p[4]]) } else { x })
    }

    pub async fn set_dpi(&mut self, dpi: u16) -> Result<()> {
        let [hi, lo] = dpi.to_be_bytes();
        if self.has(feature::ADJUSTABLE_DPI).await? {
            self.call(feature::ADJUSTABLE_DPI, 3, &[0, hi, lo]).await?;
            return Ok(());
        }
        // Capabilities byte 2: bit 0 separate Y, bit 1 lift-off distance.
        let caps = self.call(feature::EXT_ADJUSTABLE_DPI, 1, &[0]).await?[2];
        let current = self.call(feature::EXT_ADJUSTABLE_DPI, 5, &[0]).await?;
        let (yhi, ylo) = if caps & 1 != 0 { (hi, lo) } else { (0, 0) };
        let lod = if caps & 2 != 0 { current[9] } else { 0 };
        self.call(feature::EXT_ADJUSTABLE_DPI, 6, &[0, hi, lo, yhi, ylo, lod]).await?;
        Ok(())
    }

    // ---------------------------------- report rate (0x8060, extended 0x8061)

    /// Intervals in ms supported by the classic report-rate feature, which is
    /// also the unit stored in onboard profiles. `None` without 0x8060.
    pub async fn report_intervals(&mut self) -> Result<Option<Vec<u8>>> {
        if !self.has(feature::REPORT_RATE).await? {
            return Ok(None);
        }
        let mask = self.call(feature::REPORT_RATE, 0, &[]).await?[0];
        Ok(Some((0..8).filter(|b| mask >> b & 1 == 1).map(|b| b + 1).collect()))
    }

    /// Supported polling rates in Hz (either feature).
    pub async fn report_rates(&mut self) -> Result<Option<Vec<u32>>> {
        if let Some(ms) = self.report_intervals().await? {
            return Ok(Some(ms.into_iter().map(|m| 1000 / m as u32).collect()));
        }
        if self.has(feature::EXT_REPORT_RATE).await? {
            // Bit i = 125 << i Hz (125 Hz … 8000 Hz).
            let p = self.call(feature::EXT_REPORT_RATE, 1, &[]).await?;
            let mask = u16::from_be_bytes([p[0], p[1]]);
            return Ok(Some((0..7).filter(|i| mask >> i & 1 == 1).map(|i| 125u32 << i).collect()));
        }
        Ok(None)
    }

    pub async fn report_rate(&mut self) -> Result<u32> {
        if self.has(feature::REPORT_RATE).await? {
            return Ok(1000 / self.call(feature::REPORT_RATE, 1, &[]).await?[0].max(1) as u32);
        }
        Ok(125u32 << self.call(feature::EXT_REPORT_RATE, 2, &[]).await?[0].min(6))
    }

    pub async fn set_report_rate(&mut self, hz: u32) -> Result<()> {
        if self.has(feature::REPORT_RATE).await? {
            let ms = (1000 / hz.max(1)).clamp(1, 255) as u8;
            self.call(feature::REPORT_RATE, 2, &[ms]).await?;
            return Ok(());
        }
        let Some(index) = (0..7u8).find(|i| 125u32 << i == hz) else { bail!("unsupported polling rate {hz} Hz") };
        self.call(feature::EXT_REPORT_RATE, 3, &[index]).await?;
        Ok(())
    }

    // ----------------------------------------------- onboard profiles (0x8100)

    pub async fn onboard_info(&mut self) -> Result<Option<OnboardInfo>> {
        if !self.has(feature::ONBOARD_PROFILES).await? {
            return Ok(None);
        }
        let p = self.call(feature::ONBOARD_PROFILES, 0, &[]).await?;
        Ok(Some(OnboardInfo {
            profile_format: p[1],
            profile_count: p[3],
            button_count: p[5],
            sector_size: u16::from_be_bytes([p[7], p[8]]),
        }))
    }

    pub async fn mode(&mut self) -> Result<Mode> {
        Ok(match self.call(feature::ONBOARD_PROFILES, 2, &[]).await?[0] {
            1 => Mode::Onboard,
            _ => Mode::Host,
        })
    }

    pub async fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 1, &[mode as u8]).await?;
        Ok(())
    }

    /// Sector of the active profile (0 when none, e.g. in host mode).
    pub async fn current_profile(&mut self) -> Result<u16> {
        let p = self.call(feature::ONBOARD_PROFILES, 4, &[]).await?;
        Ok(u16::from_be_bytes([p[0], p[1]]))
    }

    pub async fn set_current_profile(&mut self, sector: u16) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 3, &sector.to_be_bytes()).await?;
        Ok(())
    }

    pub async fn current_dpi_index(&mut self) -> Result<u8> {
        Ok(self.call(feature::ONBOARD_PROFILES, 11, &[]).await?[0])
    }

    pub async fn set_current_dpi_index(&mut self, i: u8) -> Result<()> {
        self.call(feature::ONBOARD_PROFILES, 12, &[i]).await?;
        Ok(())
    }

    /// Reads a whole sector (reads are 16 bytes; the last one is aligned to
    /// the end because reading past the sector is rejected).
    pub async fn read_sector(&mut self, sector: u16, size: u16) -> Result<Vec<u8>> {
        let size = size as usize;
        let mut out = Vec::with_capacity(size);
        while out.len() < size {
            let off = out.len().min(size.saturating_sub(16));
            let mut req = sector.to_be_bytes().to_vec();
            req.extend((off as u16).to_be_bytes());
            let p = self.call(feature::ONBOARD_PROFILES, 5, &req).await?;
            let skip = out.len() - off;
            out.extend_from_slice(&p[skip..16.min(size - off)]);
        }
        Ok(out)
    }

    /// Writes a whole sector; `data` must already carry its CRC.
    pub async fn write_sector(&mut self, sector: u16, data: &[u8]) -> Result<()> {
        if sector & ROM_BASE != 0 {
            bail!("sector {sector:#06x} is read-only");
        }
        let mut start = sector.to_be_bytes().to_vec();
        start.extend(0u16.to_be_bytes());
        start.extend((data.len() as u16).to_be_bytes());
        self.call(feature::ONBOARD_PROFILES, 6, &start).await?;
        for chunk in data.chunks(16) {
            let mut c = [0u8; 16];
            c[..chunk.len()].copy_from_slice(chunk);
            self.call(feature::ONBOARD_PROFILES, 7, &c).await?;
        }
        self.call(feature::ONBOARD_PROFILES, 8, &[]).await?;
        Ok(())
    }

    /// Profile sectors listed in the directory with their enabled flag. When
    /// no software has written profiles yet, the flash directory is empty and
    /// the factory (read-only, `ROM_BASE | n`) profiles are listed instead.
    pub async fn profile_directory(&mut self, info: &OnboardInfo) -> Result<Vec<(u16, bool)>> {
        let mut dir = self.read_sector(DIRECTORY_SECTOR, info.sector_size).await?;
        if dir[..4] == [0; 4] || dir[..4] == [0xFF; 4] {
            dir = self.read_sector(ROM_BASE, info.sector_size).await?;
        }
        Ok(dir
            .chunks(4)
            .take(info.profile_count as usize)
            .take_while(|e| !(e[0] == 0xFF && e[1] == 0xFF))
            .map(|e| (u16::from_be_bytes([e[0], e[1]]), e[2] != 0))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_reply_from_superlight() {
        // getSensorDpiList reply params from a PRO X Superlight (16 bytes):
        // sensor 0, then 100, 0xE032 (step 50), 25600, 0 … — 15 bytes of words.
        let reply = [0x00, 0x00, 0x64, 0xe0, 0x32, 0x64, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let info = Device::dpi_info_from(Device::expand_dpi_list(&Device::dpi_words(&reply[1..]))).unwrap();
        assert_eq!((info.min, info.max, info.step), (100, 25600, 50));
    }

    #[test]
    fn dpi_list_with_range() {
        // PRO X Superlight: 100, step 50, 25600.
        let list = Device::expand_dpi_list(&[100, 0xE032, 25600, 0]);
        assert_eq!(list.len(), (25600 - 100) / 50 + 1);
        let info = Device::dpi_info_from(list).unwrap();
        assert_eq!((info.min, info.max, info.step), (100, 25600, 50));
        assert!(info.values.is_empty());
    }

    #[test]
    fn dpi_list_with_mixed_steps() {
        // 100-1000 in 50s, then up to 2000 in 100s: kept as an explicit list.
        let info = Device::dpi_info_from(Device::expand_dpi_list(&[100, 0xE032, 1000, 0xE064, 2000, 0])).unwrap();
        assert_eq!((info.min, info.max), (100, 2000));
        assert_eq!(info.values.len(), 19 + 10);
        assert!(info.values.contains(&1900) && !info.values.contains(&1950));
    }
}
