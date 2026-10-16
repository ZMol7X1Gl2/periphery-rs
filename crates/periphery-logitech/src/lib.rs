//! Periphery adapter for Logitech HID++ 2.0 mice (protocol in the
//! `logitech-hidpp` crate).

use anyhow::{Result, bail};
use logitech_hidpp::profile::{Binding, DPI_STAGES, LED_SLOTS, Led, NAME_LEN, Profile, effect, special};
use logitech_hidpp::{Charging, Device as Hidpp, DpiInfo, LedZone, Mode, OnboardInfo, ROM_BASE};
use periphery_core::hid::{Api, Filter};
use periphery_core::{Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Role, Setting, Snapshot, Value, async_trait};

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
    /// Lighting zones (none on mice without lights), the last live effect
    /// set on each, and whether software has taken lighting control.
    zones: Vec<LedZone>,
    live_leds: Vec<Option<Led>>,
    led_control: bool,
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
const EFFECTS: [(u16, &str); 4] =
    [(effect::OFF, "Off"), (effect::STATIC, "Static colour"), (effect::CYCLE, "Colour cycle"), (effect::BREATHE, "Breathing")];

/// Effect, colour, cycle time and brightness rows for one lighting zone;
/// ids are `{prefix}.effect` etc.
fn led_settings(g: &mut Group, prefix: &str, zone: &LedZone, led: Led, help: &str) {
    let mut opts: Vec<(i64, String)> =
        EFFECTS.iter().filter(|(id, _)| zone.effects.contains(id)).map(|(id, l)| (*id as i64, l.to_string())).collect();
    let current = led.effect();
    if !opts.iter().any(|(v, _)| *v == current as i64) {
        opts.push((current as i64, format!("Effect {current:#04x}")));
    }
    let name = zone.name();
    g.push(Setting::new(format!("{prefix}.effect"), format!("{name} lighting"), Kind::Choice(opts), Value::Int(current as i64)).help(help));
    if let Some(c) = led.color() {
        g.push(Setting::new(format!("{prefix}.color"), format!("{name} colour"), Kind::Color, Value::Color(c)));
    }
    if let Some((period, brightness)) = led.animation() {
        g.push(Setting::new(
            format!("{prefix}.period"),
            format!("{name} cycle time"),
            Kind::range_unit(1000, 20_000, 500, 0.001, " s"),
            Value::Int((period as i64).clamp(1000, 20_000)),
        ));
        g.push(Setting::new(
            format!("{prefix}.brightness"),
            format!("{name} brightness"),
            Kind::range_unit(1, 100, 1, 1.0, "%"),
            Value::Int(brightness.clamp(1, 100) as i64),
        ));
    }
}

/// Applies one edited lighting row (`effect`, `color`, `period` or
/// `brightness`) to `led`.
fn led_edit(led: Led, field: &str, value: &Value) -> Result<Led> {
    Ok(match (field, value) {
        ("effect", v) => led.with_effect(v.int()? as u16),
        ("color", Value::Color(c)) => led.with_color(*c),
        ("period", v) => {
            let Some((_, b)) = led.animation() else { bail!("this effect has no cycle time") };
            led.with_animation(v.int()?.clamp(1000, 20_000) as u16, b)
        }
        ("brightness", v) => {
            let Some((p, _)) = led.animation() else { bail!("this effect has no brightness") };
            led.with_animation(p, v.int()?.clamp(1, 100) as u8)
        }
        _ => bail!("unknown lighting setting {field}"),
    })
}

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
        // Lighting is optional; a mouse that fails to describe it just has none.
        let zones = dev.led_zones().await.unwrap_or_default();
        let info = DeviceInfo {
            brand: "Logitech".into(),
            model,
            kind: DeviceKind::Mouse,
            vid: logitech_hidpp::VENDOR_ID,
            pid: dev.product_id,
        };
        let live_leds = vec![None; zones.len()];
        Ok(LogitechMouse { dev, info, onboard, edit_sector: None, profile: None, zones, live_leds, led_control: false })
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
        g.push_as(
            Role::Profile,
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
        // In software mode the live settings are the ones in effect, so the
        // profile is shown as device-specific and only the live ones get roles.
        let onboard = mode == Mode::Onboard;
        let tag = |s: Setting, role: Role| if onboard { s.role(role) } else { s };
        let mut g = Group::new(format!("Profile {sector}"));
        g.push(Setting::new("p.name", "Name", Kind::Text { max_len: NAME_LEN }, Value::Text(p.name())));
        if !intervals.is_empty() {
            g.push(tag(Setting::new("p.rate", "Polling rate", Self::rate_kind(intervals), Value::Int(p.report_interval() as i64)), Role::PollingRate));
        }
        if let Some(d) = dpi {
            for i in 0..DPI_STAGES {
                let v = p.dpi(i);
                if v != 0 {
                    g.push(tag(Setting::new(format!("p.dpi.{i}"), format!("DPI stage {}", i + 1), Self::dpi_kind(d), Value::Int(v as i64)), Role::Dpi(i as u8)));
                }
            }
        }
        let stages = || Kind::choice((0..DPI_STAGES as i64).filter(|i| p.dpi(*i as usize) != 0).map(|i| (i, format!("Stage {}", i + 1))));
        g.push(tag(
            Setting::new("p.default", "Default DPI stage", stages(), Value::Int(p.default_dpi_stage() as i64))
                .help("Stage the mouse starts in when this profile loads; changing it also switches to it now"),
            Role::ActiveStage,
        ));
        g.push(
            Setting::new("p.shift", "DPI shift stage", stages(), Value::Int(p.shift_dpi_stage() as i64))
                .help("DPI used while a button set to “DPI shift” is held"),
        );
        out.groups.push(g);

        if !self.zones.is_empty() {
            let mut g = Group::new("Lighting");
            for (slot, zone) in self.zones.iter().take(LED_SLOTS).enumerate() {
                led_settings(&mut g, &format!("p.led.{slot}"), zone, p.led(slot), "Stored in the onboard profile");
            }
            out.groups.push(g);
        }

        let mut b = Group::new("Buttons");
        for i in 0..p.button_count().min(BUTTON_NAMES.len()) {
            let (value, other) = binding_choice(p.button(i));
            let mut opts: Vec<(i64, String)> = MOUSE_BUTTONS.iter().map(|(v, l)| (*v as i64, l.to_string())).collect();
            opts.extend(SPECIALS.iter().map(|(c, l)| (SPECIAL_BASE + *c as i64, l.to_string())));
            opts.push((CHOICE_DISABLED, "Disabled".into()));
            if let Some(o) = other {
                opts.push((CHOICE_OTHER, o));
            }
            b.push(tag(
                Setting::new(format!("p.button.{i}"), BUTTON_NAMES[i], Kind::Choice(opts), Value::Int(value))
                    .help("Stored in the onboard profile. DPI and profile functions use libratbag's documented codes."),
                Role::Button(i as u8),
            ));
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
            g.push_as(
                Role::SettingsSource,
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
        // The profile groups are read first so live lighting can start from
        // the profile's colours, but the live group is shown above them.
        let live_at = out.groups.len();
        self.profile_groups(&mut out, dpi.as_ref(), &intervals, mode).await?;
        if mode == Mode::Host {
            let mut g = Group::new("Live settings");
            if let Some(d) = &dpi {
                g.push_as(
                    Role::Dpi(0),
                    Setting::new("live.dpi", "DPI", Self::dpi_kind(d), Value::Int(self.dev.dpi().await? as i64))
                        .help("Applies immediately but isn't stored on the mouse"),
                );
            }
            if !rates.is_empty() {
                g.push_as(Role::PollingRate, Setting::new("live.rate", "Polling rate", Self::hz_kind(&rates), Value::Int(self.dev.report_rate().await? as i64)));
            }
            for i in 0..self.zones.len() {
                let zone = self.zones[i].clone();
                let read = self.dev.led(&zone).await.unwrap_or(None);
                let from_profile = self.profile.as_ref().filter(|_| i < LED_SLOTS).map(|p| p.led(i));
                let led = read.or(self.live_leds[i]).or(from_profile).unwrap_or(Led::Static([0xFF; 3]));
                self.live_leds[i] = Some(led);
                led_settings(&mut g, &format!("live.led.{i}"), &zone, led, "Applies immediately but isn't stored on the mouse");
            }
            if g.settings.is_empty() {
                g.push(
                    Setting::new("live.none", "Live controls", Kind::Info, Value::Text("not reported by this mouse".into()))
                        .help("The mouse exposes neither DPI nor polling-rate features in software mode."),
                );
            }
            out.groups.insert(live_at, g);
        }
        Ok(out)
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        let (head, tail) = id.rsplit_once('.').map_or((id, ""), |(h, t)| (h, t));
        if let Some(slot) = head.strip_prefix("p.led.") {
            let slot: usize = slot.parse()?;
            if slot >= LED_SLOTS.min(self.zones.len()) {
                bail!("no lighting zone {slot}");
            }
            return self
                .edit_profile(|p| {
                    p.set_led(slot, led_edit(p.led(slot), tail, value)?);
                    Ok(())
                })
                .await;
        }
        if let Some(i) = head.strip_prefix("live.led.") {
            let i: usize = i.parse()?;
            let Some(zone) = self.zones.get(i).cloned() else { bail!("no lighting zone {i}") };
            let led = led_edit(self.live_leds[i].unwrap_or(Led::Static([0xFF; 3])), tail, value)?;
            if !self.led_control {
                // Not every mouse needs (or has) the switch; the effect write
                // below reports any real problem.
                for z in &self.zones {
                    let _ = self.dev.set_led_control(z, true).await;
                }
                self.led_control = true;
            }
            self.dev.set_led(&zone, led).await?;
            self.live_leds[i] = Some(led);
            return Ok(());
        }
        match (head, tail) {
            ("mode", _) => {
                let mode = if value.int()? == Mode::Onboard as i64 { Mode::Onboard } else { Mode::Host };
                self.dev.set_mode(mode).await?;
                if mode == Mode::Onboard && self.led_control {
                    // Hand lighting back to the onboard profile.
                    for z in &self.zones {
                        let _ = self.dev.set_led_control(z, false).await;
                    }
                    self.led_control = false;
                }
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
                self.edit_profile(|p| Ok(p.set_default_dpi_stage(i))).await?;
                // The mouse only applies the default stage when a profile
                // loads; switch the running stage too when it's the active one.
                if self.dev.mode().await? == Mode::Onboard && Some(self.dev.current_profile().await?) == self.edit_sector {
                    self.dev.set_current_dpi_index(i).await?;
                }
                Ok(())
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
