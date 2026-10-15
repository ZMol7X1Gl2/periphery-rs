//! Periphery adapter for Logitech HID++ 2.0 mice (protocol in the
//! `logitech-hidpp` crate).

use anyhow::{Result, bail};
use logitech_hidpp::profile::{Binding, DPI_STAGES, NAME_LEN, Profile, special};
use logitech_hidpp::{Charging, Device as Hidpp, DpiInfo, Mode, OnboardInfo, ROM_BASE};
use periphery_core::hid::{Api, Filter};
use periphery_core::{Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Setting, Snapshot, Value, async_trait};

pub struct LogitechDriver;

#[async_trait(?Send)]
impl Driver for LogitechDriver {
    fn name(&self) -> &'static str {
        "logitech-hidpp"
    }

    fn filters(&self) -> Vec<Filter> {
        vec![logitech_hidpp::FILTER]
    }

    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>> {
        let mut out = Vec::new();
        for r in logitech_hidpp::discover(api).await {
            out.push(match r {
                Ok(d) => LogitechMouse::new(d).await.map(|m| Box::new(m) as Box<dyn Device>),
                Err(e) => Err(e),
            });
        }
        out
    }
}

pub struct LogitechMouse {
    dev: Hidpp,
    info: DeviceInfo,
    onboard: Option<OnboardInfo>,
    /// Profile sector shown in the editor, and its last-read contents.
    edit_sector: Option<u16>,
    profile: Option<Profile>,
}

/// Button choice values beyond the mouse-button bitmasks.
const SPECIAL_BASE: i64 = 0x100;
const CHOICE_DISABLED: i64 = 0x1FF;
const CHOICE_OTHER: i64 = 0x200;

const MOUSE_BUTTONS: [(u16, &str); 5] =
    [(0x01, "Left click"), (0x02, "Right click"), (0x04, "Middle click"), (0x08, "Back"), (0x10, "Forward")];
const SPECIALS: [(u8, &str); 10] = [
    (special::DPI_CYCLE, "DPI cycle"),
    (special::DPI_UP, "DPI up"),
    (special::DPI_DOWN, "DPI down"),
    (special::DPI_SHIFT, "DPI shift (hold)"),
    (special::DPI_DEFAULT, "DPI default"),
    (special::WHEEL_LEFT, "Scroll left"),
    (special::WHEEL_RIGHT, "Scroll right"),
    (special::PROFILE_CYCLE, "Profile cycle"),
    (special::PROFILE_UP, "Next profile"),
    (special::PROFILE_DOWN, "Previous profile"),
];
const BUTTON_NAMES: [&str; 5] = ["Left", "Right", "Middle", "Back", "Forward"];

fn hz(ms: u8) -> String {
    format!("{} Hz", 1000 / ms.max(1) as u32)
}

fn binding_choice(b: Binding) -> (i64, Option<String>) {
    match b {
        Binding::Mouse(m) if MOUSE_BUTTONS.iter().any(|(v, _)| *v == m) => (m as i64, None),
        Binding::Special(c) if SPECIALS.iter().any(|(v, _)| *v == c) => (SPECIAL_BASE + c as i64, None),
        Binding::Disabled => (CHOICE_DISABLED, None),
        Binding::Key { modifiers, key } => (CHOICE_OTHER, Some(format!("Key {key:#04x} (mods {modifiers:#04x})"))),
        Binding::Consumer(u) => (CHOICE_OTHER, Some(format!("Media key {u:#06x}"))),
        other => (CHOICE_OTHER, Some(format!("Custom {:02x?}", other.encode()))),
    }
}

impl LogitechMouse {
    async fn new(mut dev: Hidpp) -> Result<Self> {
        let model = dev.name().await.unwrap_or_else(|_| format!("device {:04x}", dev.product_id));
        let onboard = dev.onboard_info().await?;
        let info = DeviceInfo {
            brand: "Logitech".into(),
            model,
            kind: DeviceKind::Mouse,
            vid: logitech_hidpp::VENDOR_ID,
            pid: dev.product_id,
        };
        Ok(LogitechMouse { dev, info, onboard, edit_sector: None, profile: None })
    }

    fn dpi_kind(info: &DpiInfo) -> Kind {
        if info.values.is_empty() {
            Kind::range_unit(info.min as i64, info.max as i64, info.step.max(1) as i64, 1.0, " DPI")
        } else {
            Kind::choice(info.values.iter().map(|v| (*v as i64, format!("{v} DPI"))))
        }
    }

    fn hz_kind(rates: &[u32]) -> Kind {
        Kind::choice(rates.iter().map(|hz| (*hz as i64, format!("{hz} Hz"))))
    }

    /// Reads a profile sector; factory (ROM) copies carry no valid CRC.
    async fn load_profile(&mut self, info: &OnboardInfo, sector: u16) -> Result<Profile> {
        let raw = self.dev.read_sector(sector, info.sector_size).await?;
        if sector & ROM_BASE != 0 { Profile::from_rom(raw, info.button_count) } else { Profile::parse(raw, info.button_count) }
    }

    fn rate_kind(intervals: &[u8]) -> Kind {
        Kind::choice(intervals.iter().map(|ms| (*ms as i64, hz(*ms))))
    }

    /// Applies `edit` to the edited profile, writes it, verifies it and makes
    /// the mouse reload it when it's the active one.
    async fn edit_profile(&mut self, edit: impl FnOnce(&mut Profile) -> Result<()>) -> Result<()> {
        let (Some(info), Some(sector)) = (self.onboard, self.edit_sector) else { bail!("no onboard profile loaded") };
        if sector & ROM_BASE != 0 {
            bail!("this is the mouse's factory profile, which is read-only; save a profile with G HUB once to create editable copies");
        }
        let mut p = Profile::parse(self.dev.read_sector(sector, info.sector_size).await?, info.button_count)?;
        edit(&mut p)?;
        let bytes = p.encode();
        self.dev.write_sector(sector, &bytes).await?;
        if self.dev.read_sector(sector, info.sector_size).await? != bytes {
            bail!("profile verification failed after writing");
        }
        if self.dev.mode().await? == Mode::Onboard && self.dev.current_profile().await? == sector {
            self.dev.set_current_profile(sector).await?;
        }
        self.profile = Some(p);
        Ok(())
    }

    async fn profile_groups(&mut self, out: &mut Snapshot, dpi: Option<&DpiInfo>, intervals: &[u8], mode: Mode) -> Result<()> {
        let Some(info) = self.onboard else { return Ok(()) };
        let dir = self.dev.profile_directory(&info).await?;
        let enabled: Vec<u16> = dir.iter().filter(|(_, on)| *on).map(|(s, _)| *s).collect();
        let current = self.dev.current_profile().await?;
        let sector = match self.edit_sector {
            Some(s) if enabled.contains(&s) => s,
            _ if enabled.contains(&current) => current,
            _ => match enabled.first() {
                Some(s) => *s,
                None => return Ok(()),
            },
        };
        self.edit_sector = Some(sector);
        let mut names = Vec::new();
        for s in &enabled {
            let p = self.load_profile(&info, *s).await?;
            names.push((*s as i64, format!("Profile {s}: {}", p.name())));
            if *s == sector {
                self.profile = Some(p);
            }
        }
        let mut g = Group::new("Onboard profiles");
        let label = if mode == Mode::Onboard { "Active profile" } else { "Profile to edit" };
        g.push(
            Setting::new("onboard.profile", label, Kind::Choice(names), Value::Int(sector as i64))
                .help("Profiles stored on the mouse; used when the mode is Onboard memory"),
        );
        out.groups.push(g);

        let Some(p) = self.profile.clone() else { return Ok(()) };
        if sector & ROM_BASE != 0 {
            let mut g = Group::new("Factory profile (read-only)");
            let info_row = |id: &str, label: &str, text: String| Setting::new(id, label, Kind::Info, Value::Text(text));
            let dpis: Vec<String> = (0..DPI_STAGES).map(|i| p.dpi(i)).filter(|d| *d != 0).map(|d| d.to_string()).collect();
            g.push(info_row("rom.dpi", "DPI stages", dpis.join(" / ")));
            g.push(info_row("rom.default", "Default stage", format!("Stage {}", p.default_dpi_stage() + 1)));
            g.push(
                info_row("rom.note", "Editing", "Not available yet".into())
                    .help("This mouse only has its factory profiles, which are read-only. Saving a profile once with G HUB creates editable copies."),
            );
            out.groups.push(g);
            return Ok(());
        }
        let mut g = Group::new(format!("Profile {sector}"));
        g.push(Setting::new("p.name", "Name", Kind::Text { max_len: NAME_LEN }, Value::Text(p.name())));
        if !intervals.is_empty() {
            g.push(Setting::new("p.rate", "Polling rate", Self::rate_kind(intervals), Value::Int(p.report_interval() as i64)));
        }
        if let Some(d) = dpi {
            for i in 0..DPI_STAGES {
                let v = p.dpi(i);
                if v != 0 {
                    g.push(Setting::new(format!("p.dpi.{i}"), format!("DPI stage {}", i + 1), Self::dpi_kind(d), Value::Int(v as i64)));
                }
            }
        }
        let stages = || Kind::choice((0..DPI_STAGES as i64).filter(|i| p.dpi(*i as usize) != 0).map(|i| (i, format!("Stage {}", i + 1))));
        g.push(Setting::new("p.default", "Default DPI stage", stages(), Value::Int(p.default_dpi_stage() as i64)));
        g.push(
            Setting::new("p.shift", "DPI shift stage", stages(), Value::Int(p.shift_dpi_stage() as i64))
                .help("DPI used while a button set to “DPI shift” is held"),
        );
        out.groups.push(g);

        let mut b = Group::new("Buttons");
        for i in 0..p.button_count().min(BUTTON_NAMES.len()) {
            let (value, other) = binding_choice(p.button(i));
            let mut opts: Vec<(i64, String)> = MOUSE_BUTTONS.iter().map(|(v, l)| (*v as i64, l.to_string())).collect();
            opts.extend(SPECIALS.iter().map(|(c, l)| (SPECIAL_BASE + *c as i64, l.to_string())));
            opts.push((CHOICE_DISABLED, "Disabled".into()));
            if let Some(o) = other {
                opts.push((CHOICE_OTHER, o));
            }
            b.push(
                Setting::new(format!("p.button.{i}"), BUTTON_NAMES[i], Kind::Choice(opts), Value::Int(value))
                    .help("Stored in the onboard profile. DPI and profile functions use libratbag's documented codes."),
            );
        }
        b.push(Setting::action(
            "p.factory",
            "Reset profile to factory",
            Some("Replace this profile with the mouse's factory default (stored in its ROM)?"),
        ));
        out.groups.push(b);
        Ok(())
    }
}

#[async_trait(?Send)]
impl Device for LogitechMouse {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    async fn snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        let via = if self.dev.index == 0xFF { "USB / direct" } else { "Wireless receiver" };
        out.status.push(("Connection".into(), via.into()));
        if let Some(fw) = self.dev.firmware().await? {
            out.status.push(("Firmware".into(), fw));
        }
        if let Some(b) = self.dev.battery().await? {
            let state = match b.charging {
                Charging::Discharging => "",
                Charging::Charging => " charging",
                Charging::Full => " full",
                Charging::Error => " (battery error)",
            };
            let pct = b.percent.map_or("?".to_string(), |p| format!("{p}%"));
            out.status.push(("Battery".into(), format!("{pct}{state}")));
        }
        // A failing optional read shouldn't hide everything else (incl. mode).
        let dpi = self.dev.dpi_info().await.unwrap_or(None);
        let intervals = self.dev.report_intervals().await.unwrap_or(None).unwrap_or_default();
        let rates = self.dev.report_rates().await.unwrap_or(None).unwrap_or_default();
        let mode = if self.onboard.is_some() { self.dev.mode().await? } else { Mode::Host };

        if self.onboard.is_some() {
            let mut g = Group::new("Mode");
            g.push(
                Setting::new(
                    "mode",
                    "Settings come from",
                    Kind::choice([(Mode::Onboard as i64, "Onboard memory"), (Mode::Host as i64, "Software (live)")]),
                    Value::Int(mode as i64),
                )
                .help("G HUB switches the mouse to software mode while it runs and applies its own settings. Onboard memory uses the profiles below on any computer."),
            );
            out.groups.push(g);
        }
        if mode == Mode::Host {
            let mut g = Group::new("Live settings");
            if let Some(d) = &dpi {
                g.push(
                    Setting::new("live.dpi", "DPI", Self::dpi_kind(d), Value::Int(self.dev.dpi().await? as i64))
                        .help("Applies immediately but isn't stored on the mouse"),
                );
            }
            if !rates.is_empty() {
                g.push(Setting::new("live.rate", "Polling rate", Self::hz_kind(&rates), Value::Int(self.dev.report_rate().await? as i64)));
            }
            if g.settings.is_empty() {
                g.push(
                    Setting::new("live.none", "Live controls", Kind::Info, Value::Text("not reported by this mouse".into()))
                        .help("The mouse exposes neither DPI nor polling-rate features in software mode."),
                );
            }
            out.groups.push(g);
        }
        self.profile_groups(&mut out, dpi.as_ref(), &intervals, mode).await?;
        Ok(out)
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        let (head, tail) = id.rsplit_once('.').map_or((id, ""), |(h, t)| (h, t));
        match (head, tail) {
            ("mode", _) => {
                let mode = if value.int()? == Mode::Onboard as i64 { Mode::Onboard } else { Mode::Host };
                self.dev.set_mode(mode).await?;
                if mode == Mode::Onboard {
                    if let Some(s) = self.edit_sector {
                        self.dev.set_current_profile(s).await?;
                    }
                }
                Ok(())
            }
            ("live", "dpi") => self.dev.set_dpi(value.int()? as u16).await,
            ("live", "rate") => self.dev.set_report_rate(value.int()? as u32).await,
            ("onboard", "profile") => {
                let s = value.int()? as u16;
                self.edit_sector = Some(s);
                if self.dev.mode().await? == Mode::Onboard {
                    self.dev.set_current_profile(s).await?;
                }
                Ok(())
            }
            ("p", "name") => match value {
                Value::Text(t) => self.edit_profile(|p| Ok(p.set_name(t))).await,
                _ => bail!("name must be text"),
            },
            ("p", "rate") => {
                let ms = value.int()? as u8;
                self.edit_profile(|p| Ok(p.set_report_interval(ms))).await
            }
            ("p", "default") => {
                let i = value.int()? as u8;
                self.edit_profile(|p| Ok(p.set_default_dpi_stage(i))).await
            }
            ("p", "shift") => {
                let i = value.int()? as u8;
                self.edit_profile(|p| Ok(p.set_shift_dpi_stage(i))).await
            }
            ("p", "factory") => {
                let (Some(info), Some(sector)) = (self.onboard, self.edit_sector) else { bail!("no profile") };
                let rom = self.dev.read_sector(ROM_BASE + sector, info.sector_size).await?;
                let factory = Profile::from_rom(rom, info.button_count)?;
                self.edit_profile(|p| {
                    *p = factory;
                    Ok(())
                }).await
            }
            ("p.dpi", stage) => {
                let stage: usize = stage.parse()?;
                let dpi = value.int()? as u16;
                if let Some(d) = self.dev.dpi_info().await? {
                    if dpi < d.min || dpi > d.max {
                        bail!("DPI must be {}-{}", d.min, d.max);
                    }
                }
                self.edit_profile(|p| Ok(p.set_dpi(stage, dpi))).await
            }
            ("p.button", i) => {
                let i: usize = i.parse()?;
                let binding = match value.int()? {
                    CHOICE_OTHER => return Ok(()),
                    CHOICE_DISABLED => Binding::Disabled,
                    v if v >= SPECIAL_BASE => Binding::Special((v - SPECIAL_BASE) as u8),
                    v => Binding::Mouse(v as u16),
                };
                self.edit_profile(|p| Ok(p.set_button(i, binding))).await
            }
            _ => bail!("unknown setting {id}"),
        }
    }

    async fn poll_changes(&mut self) -> Result<bool> {
        Ok(self.dev.poll_notifications().await)
    }
}
