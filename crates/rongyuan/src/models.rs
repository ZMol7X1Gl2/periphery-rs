//! Known devices. The firmware reports a model id (command `0x8F`); USB ids
//! are shared by many models. Values come from the v4 web driver's device
//! list (`dpi: { count, max, min, delt }`, `layer`, `sensor`) and its
//! per-model chunks (all listed models use `CommonMsPan1080` or a subclass
//! that only changes the firmware-version command or the debounce encoding).

pub const VENDOR_ID: u16 = 0x3151;

/// Wired PAN1080/CH585 mice (`type: "mouse"`, usage page 0xFFFF, usage 2).
pub const MOUSE_PIDS: &[u16] = &[0x400B, 0x4026, 0x402A, 0x5032, 0x5035, 0x503A, 0x5043, 0x5058];

/// Receivers (`type: "dongle"`) with the highest polling rate the web driver
/// offers through them (1000 Hz when the list gives none).
pub const RECEIVERS: &[(u16, u32)] = &[
    (0x4011, 1000),
    (0x4014, 1000),
    (0x4017, 1000),
    (0x4021, 1000),
    (0x4027, 1000),
    (0x402D, 8000),
    (0x4038, 1000),
    (0x5006, 1000),
    (0x5007, 8000),
    (0x5008, 1000),
    (0x5020, 1000),
    (0x5026, 1000),
    (0x502B, 8000),
    (0x5031, 1000),
    (0x5037, 8000),
    (0x5038, 8000),
    (0x503D, 8000),
    (0x503F, 8000),
    (0x5040, 8000),
    (0x5042, 1000),
    (0x5045, 1000),
    (0x504B, 1000),
    (0x504E, 8000),
    (0x5050, 1000),
    (0x5052, 1000),
    (0x5055, 8000),
    (0x505A, 8000),
    (0x5072, 1000),
];

pub const USAGE_PAGE: u16 = 0xFFFF;
pub const USAGE: u16 = 2;

/// Highest polling rate through a receiver, or `None` if `pid` isn't one.
pub fn receiver_max_rate(pid: u16) -> Option<u32> {
    RECEIVERS.iter().find(|(p, _)| *p == pid).map(|(_, r)| *r)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    Paw3311,
    Paw3395,
    Paw3950,
    Paw3955,
}

impl Sensor {
    /// Lift-off distances in millimetres, indexed by the stored value
    /// (web driver `is_静默高度`, without firmware toggles).
    pub fn lod_mm(self) -> &'static [&'static str] {
        match self {
            Sensor::Paw3395 => &["1", "2"],
            Sensor::Paw3950 | Sensor::Paw3955 => &["0.7", "1", "2"],
            Sensor::Paw3311 => &[],
        }
    }

    /// The DPI step doubles (or more) above these values (`is_DPIStep`).
    fn step_factor(self, dpi: u32) -> u32 {
        match self {
            Sensor::Paw3955 => if dpi < 40_000 { 1 } else { 2 },
            Sensor::Paw3950 => if dpi < 30_000 { 1 } else { 2 },
            Sensor::Paw3395 => if dpi < 26_000 { 1 } else { 2 },
            Sensor::Paw3311 => match dpi {
                ..10_000 => 1,
                ..12_000 => 2,
                _ => 4,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: u32,
    pub name: &'static str,
    pub sensor: Sensor,
    /// DPI stages the model stores.
    pub stages: u8,
    pub min_dpi: u32,
    pub max_dpi: u32,
    /// Base DPI step (`delt`).
    pub step: u32,
    pub profiles: u8,
    /// Firmware version command: `0x80`, or `0xAD` on the 8K subclass.
    pub version_cmd: u8,
}

impl Model {
    /// Rounds to a DPI the sensor accepts.
    pub fn snap_dpi(&self, dpi: u32) -> u32 {
        let dpi = dpi.clamp(self.min_dpi, self.max_dpi);
        let step = self.step * self.sensor.step_factor(dpi);
        ((dpi + step / 2) / step * step).clamp(self.min_dpi, self.max_dpi)
    }
}

const fn m(id: u32, name: &'static str, sensor: Sensor, stages: u8, min_dpi: u32, max_dpi: u32, step: u32) -> Model {
    Model { id, name, sensor, stages, min_dpi, max_dpi, step, profiles: 8, version_cmd: 0x80 }
}

use Sensor::*;

pub const MODELS: &[Model] = &[
    m(1851, "AJ179 PRO", Paw3395, 8, 50, 32_000, 50),
    m(3766, "AJ179 PRO", Paw3950, 6, 50, 42_000, 50),
    m(1852, "AJ179 APEX", Paw3950, 8, 50, 42_000, 50),
    m(3849, "AJ179 APEX", Paw3955, 6, 1, 65_000, 1),
    // Overrides only the debounce encoding, which this crate never rewrites.
    m(3441, "AJ179P MC PRO", Paw3311, 6, 50, 24_000, 50),
    m(1755, "AJ159 APEX", Paw3950, 8, 50, 42_000, 50),
    Model { profiles: 3, version_cmd: 0xAD, ..m(3346, "AJ159 APEX", Paw3950, 6, 50, 30_000, 50) },
    m(3806, "AJ159 APEX", Paw3955, 6, 1, 65_000, 1),
    m(1754, "AJ159 PRO", Paw3395, 8, 50, 32_000, 50),
    m(3767, "AJ159 PRO", Paw3950, 6, 50, 42_000, 50),
    // The web driver caps this one at 3200 DPI, which looks like a typo.
    m(3053, "AJ159 PRO", Paw3395, 6, 50, 26_000, 50),
];

pub fn model(id: u32) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_steps() {
        let aj179 = model(1851).unwrap();
        assert_eq!(aj179.snap_dpi(1630), 1650);
        assert_eq!(aj179.snap_dpi(26_040), 26_000);
        assert_eq!(aj179.snap_dpi(26_060), 26_100);
        assert_eq!(aj179.snap_dpi(99_999), 32_000);
        assert_eq!(aj179.snap_dpi(0), 50);
        let apex = model(3849).unwrap();
        assert_eq!(apex.snap_dpi(1234), 1234);
        assert_eq!(apex.snap_dpi(41_001), 41_002);
    }
}
