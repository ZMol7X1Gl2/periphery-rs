//! Periphery adapter for Rongyuan-firmware mice (protocol in the `rongyuan`
//! crate): the Ajazz AJ179 and AJ159 series, wired or through their receiver.

use anyhow::{Result, bail};
use periphery_core::hid::{Api, DeviceInfo as HidInfo, Filter};
use periphery_core::{ButtonPos, Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Role, Setting, Snapshot, Value, async_trait};
use rongyuan::pan1080::{
    ACTIONS, BUTTONS, DEFAULT_COLORS, DEFAULT_DPIS, DpiStage, Options, REPORT_RATES, action_label,
};
use rongyuan::{Model, Mouse, models};

pub struct RongyuanDriver;

#[async_trait(?Send)]
impl Driver for RongyuanDriver {
    fn name(&self) -> &'static str {
        "rongyuan"
    }

    fn filters(&self) -> Vec<Filter> {
        vec![Mouse::FILTER]
    }

    /// Only known models: Akko, MonsGeek and other Rongyuan devices share
    /// the filter but speak other command sets.
    fn claims(&self, info: &HidInfo) -> bool {
        Mouse::is_candidate(info)
    }

    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>> {
        let mut out = Vec::new();
        for info in Mouse::candidates(api) {
            match Mouse::open(api, info).await {
                Ok(Some(m)) => out.push(Ok(Box::new(RongyuanDevice::new(m)) as Box<dyn Device>)),
                Ok(None) => {}
                Err(e) => out.push(Err(e.context(format!("{:04x}:{:04x}", info.vendor_id, info.product_id)))),
            }
        }
        out
    }
}

pub struct RongyuanDevice {
    mouse: Mouse,
    info: DeviceInfo,
}

/// Choice value for a button action this adapter can't name (left as is).
const CHOICE_OTHER: i64 = 1000;
/// Where each of [`BUTTONS`] sits on the mouse.
const BUTTON_POS: [ButtonPos; 6] =
    [ButtonPos::Left, ButtonPos::Right, ButtonPos::Middle, ButtonPos::Forward, ButtonPos::Back, ButtonPos::Top];

fn device_info(mouse: &Mouse) -> DeviceInfo {
    let (brand, model) = match mouse.model() {
        Some(m) => ("Ajazz", m.name.to_string()),
        None if mouse.is_wireless() && mouse.id.is_none() => ("Rongyuan", "wireless mouse".to_string()),
        None => ("Rongyuan", format!("mouse (id {})", mouse.id.unwrap_or_default())),
    };
    DeviceInfo {
        brand: brand.into(),
        model,
        kind: DeviceKind::Mouse,
        vid: models::VENDOR_ID,
        pid: mouse.product_id,
    }
}

impl RongyuanDevice {
    fn new(mouse: Mouse) -> Self {
        let info = device_info(&mouse);
        RongyuanDevice { mouse, info }
    }

    async fn require_model(&mut self) -> Result<&'static Model> {
        if self.mouse.identify().await?.is_none() {
            bail!("the mouse is asleep or switched off — move it to wake it up");
        }
        match self.mouse.model() {
            Some(m) => Ok(m),
            None => bail!("this mouse model (id {}) isn't supported yet", self.mouse.id.unwrap_or_default()),
        }
    }

    fn rates(&self) -> Vec<(i64, String)> {
        REPORT_RATES
            .iter()
            .filter(|r| **r <= self.mouse.max_rate)
            .map(|r| (*r as i64, format!("{r} Hz")))
            .collect()
    }

    /// Changes one option-block field and checks it reads back.
    async fn edit_options<T: PartialEq + std::fmt::Debug>(
        &self,
        edit: impl FnOnce(&mut Options) -> Result<()>,
        check: impl Fn(&Options) -> T,
    ) -> Result<()> {
        let mut o = self.mouse.options().await?;
        edit(&mut o)?;
        self.mouse.set_options(&o, check).await
    }
}

#[async_trait(?Send)]
impl Device for RongyuanDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    async fn snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        let status = self.mouse.status().await?;
        out.status.push((
            "Connection".into(),
            if status.is_some() { "2.4 GHz receiver".into() } else { "Wired".into() },
        ));
        if let Some(s) = status {
            if !s.online {
                out.notice = Some("The mouse is asleep or switched off — move it to wake it up.".into());
                return Ok(out);
            }
            out.status.push(("Battery".into(), format!("{}%", s.battery)));
        }
        if self.mouse.identify().await?.is_some() {
            let info = device_info(&self.mouse);
            if info.model != self.info.model {
                self.info = info;
            }
        }
        let id = self.mouse.id.unwrap_or_default();
        let Some(model) = self.mouse.model() else {
            out.status.push(("Model id".into(), id.to_string()));
            out.notice = Some(format!(
                "This Rongyuan mouse (model id {id}) isn't in Periphery's list yet. Please report the id together with the mouse's name."
            ));
            return Ok(out);
        };
        out.status.push(("Firmware".into(), format!("{:x}", self.mouse.firmware().await?)));
        out.status.push(("Sensor".into(), format!("{:?}", model.sensor).to_uppercase()));
        out.status.push(("Model id".into(), id.to_string()));

        let o = self.mouse.options().await?;
        let profile = o.profile();
        let mut g = Group::new("Profile");
        g.push_as(Role::Profile, Setting::new(
            "profile",
            "Onboard profile",
            Kind::choice((0..model.profiles as i64).map(|i| (i, format!("Profile {}", i + 1)))),
            Value::Int(profile as i64),
        ));
        out.groups.push(g);

        let mut perf = Group::new("Performance");
        if let Some(rate) = o.report_rate() {
            perf.push_as(Role::PollingRate, Setting::new("rate", "Polling rate", Kind::Choice(self.rates()), Value::Int(rate as i64)));
        }
        let dpi = self.mouse.dpi(profile).await?;
        perf.push_as(Role::StageCount, Setting::new(
            "stages",
            "DPI stages",
            Kind::range(1, model.stages as i64),
            Value::Int(dpi.stages.len() as i64),
        ));
        // Read-only: the AJ179 PRO ignores the active-stage byte of a `54`
        // write (the stage only changes with the DPI button).
        perf.push_as(
            Role::ActiveStage,
            Setting::new("stage", "Active stage", Kind::Info, Value::Text(format!("Stage {}", dpi.current + 1)))
                .help("Change it with the DPI button on the mouse"),
        );
        out.groups.push(perf);

        let mut g = Group::new("DPI");
        for (i, s) in dpi.stages.iter().enumerate() {
            let mut dpi_setting = Setting::new(
                format!("dpi.{i}"),
                format!("Stage {} DPI", i + 1),
                Kind::range_unit(model.min_dpi as i64, model.max_dpi as i64, model.step as i64, 1.0, " DPI"),
                Value::Int(s.x as i64),
            );
            if s.x != s.y {
                dpi_setting = dpi_setting.help(format!("Y axis: {} DPI (setting this sets both axes)", s.y));
            }
            g.push_as(Role::Dpi(i as u8), dpi_setting);
            g.push_as(Role::StageColor(i as u8), Setting::new(format!("color.{i}"), format!("Stage {} colour", i + 1), Kind::Color, Value::Color(s.color)));
        }
        out.groups.push(g);

        let mut g = Group::new("Sensor");
        let lods = model.sensor.lod_mm();
        if !lods.is_empty() {
            g.push_as(Role::LiftOff, Setting::new(
                "lod",
                "Lift-off distance",
                Kind::choice(lods.iter().enumerate().map(|(i, mm)| (i as i64, format!("{mm} mm")))),
                Value::Int(o.lod() as i64),
            ));
        }
        g.push_as(
            Role::AngleSnap,
            Setting::toggle("angle_snap", "Angle snapping", o.angle_snap())
                .help("Straightens slightly diagonal movements"),
        );
        g.push_as(Role::Ripple, Setting::toggle("ripple", "Ripple control", o.ripple()));
        out.groups.push(g);

        let buttons = self.mouse.buttons(profile).await?;
        let mut g = Group::new("Buttons");
        for (k, (name, action)) in BUTTONS.iter().zip(buttons.iter()).enumerate() {
            let mut options: Vec<(i64, String)> =
                ACTIONS.iter().enumerate().map(|(i, (l, _))| (i as i64, l.to_string())).collect();
            let value = match ACTIONS.iter().position(|(_, a)| a == action) {
                Some(i) => i as i64,
                None => {
                    options.push((CHOICE_OTHER, action_label(action)));
                    CHOICE_OTHER
                }
            };
            g.push_as(Role::Button(BUTTON_POS[k]), Setting::new(format!("key.{k}"), *name, Kind::Choice(options), Value::Int(value)));
        }
        out.groups.push(g);

        let mut g = Group::new("Maintenance");
        g.push_as(Role::FactoryReset, Setting::action("reset", "Factory reset", Some("Reset every setting on the mouse to factory defaults?")));
        out.groups.push(g);
        Ok(out)
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        let model = self.require_model().await?;
        let (name, index) = match id.split_once('.') {
            Some((n, i)) => (n, i.parse::<usize>().ok()),
            None => (id, None),
        };
        let idx = || index.ok_or_else(|| anyhow::anyhow!("missing index in {id}"));
        match name {
            "profile" => {
                let p = value.int()?;
                if !(0..model.profiles as i64).contains(&p) {
                    bail!("profile must be 1..={}", model.profiles);
                }
                self.edit_options(|o| Ok(o.set_profile(p as u8)), Options::profile).await
            }
            "rate" => {
                let hz = value.int()? as u32;
                if hz > self.mouse.max_rate {
                    bail!("{hz} Hz needs a faster connection (max {} Hz here)", self.mouse.max_rate);
                }
                self.edit_options(|o| o.set_report_rate(hz), Options::report_rate).await
            }
            "lod" => {
                let v = value.int()?;
                if !(0..model.sensor.lod_mm().len() as i64).contains(&v) {
                    bail!("invalid lift-off distance");
                }
                self.edit_options(|o| Ok(o.set_lod(v as u8)), Options::lod).await
            }
            "angle_snap" => {
                let on = value.bool()?;
                self.edit_options(|o| Ok(o.set_angle_snap(on)), Options::angle_snap).await
            }
            "ripple" => {
                let on = value.bool()?;
                self.edit_options(|o| Ok(o.set_ripple(on)), Options::ripple).await
            }
            "stages" | "dpi" | "color" => {
                let profile = self.mouse.options().await?.profile();
                let mut t = self.mouse.dpi(profile).await?;
                match name {
                    "stages" => {
                        let n = value.int()?.clamp(1, model.stages as i64) as usize;
                        while t.stages.len() < n {
                            let i = t.stages.len();
                            let d = model.snap_dpi(DEFAULT_DPIS[i] as u32) as u16;
                            t.stages.push(DpiStage { x: d, y: d, color: DEFAULT_COLORS[i] });
                        }
                        t.stages.truncate(n);
                        if t.current as usize >= n {
                            t.current = 0;
                        }
                    }
                    _ => {
                        let i = idx()?;
                        let Some(stage) = t.stages.get_mut(i) else { bail!("no stage {}", i + 1) };
                        if name == "dpi" {
                            let d = model.snap_dpi(value.int()?.max(0) as u32) as u16;
                            stage.x = d;
                            stage.y = d;
                        } else {
                            stage.color = value.color()?;
                        }
                    }
                }
                self.mouse.set_dpi(profile, &t).await
            }
            "key" => {
                let k = idx()?;
                let v = value.int()?;
                if v == CHOICE_OTHER {
                    return Ok(());
                }
                let Some((_, action)) = ACTIONS.get(v as usize) else { bail!("unknown button action {v}") };
                if k >= BUTTONS.len() {
                    bail!("no button {k}");
                }
                let profile = self.mouse.options().await?.profile();
                self.mouse.set_button(profile, k as u8, *action).await
            }
            "reset" => self.mouse.factory_reset().await,
            _ => bail!("unknown setting {id}"),
        }
    }
}

/// Raw replies for bug reports (`periphery-cli diag`); read-only.
pub async fn diag(api: &Api) -> Vec<String> {
    let mut out = Vec::new();
    for info in Mouse::candidates(api) {
        let head = format!("rongyuan {:04x}:{:04x}", info.vendor_id, info.product_id);
        let mut mouse = match Mouse::open(api, info).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                out.push(format!("{head}: receiver without a mouse"));
                continue;
            }
            Err(e) => {
                out.push(format!("{head}: {e:#}"));
                continue;
            }
        };
        out.push(format!("{head}: {}", if mouse.is_wireless() { "receiver" } else { "wired" }));
        let hex = |r: &[u8]| r.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
        let mut show = |label: &str, r: Result<String>| match r {
            Ok(v) => out.push(format!("  {label:<10} {v}")),
            Err(e) => out.push(format!("  {label:<10} ERROR {e:#}")),
        };
        show("status", mouse.status().await.map(|s| format!("{s:?}")));
        let id = mouse.identify().await;
        show("model id", id.map(|id| format!("{id:?} {:?}", mouse.model().map(|m| m.name))));
        show("firmware", mouse.firmware().await.map(|v| format!("{v:x}")));
        let options = mouse.options().await;
        let profile = options.as_ref().map(|o| o.profile()).unwrap_or(0);
        show("options", options.map(|o| hex(&o.raw)));
        show("dpi", mouse.dpi(profile).await.map(|t| format!("{t:?}")));
        show("dpi raw", mouse.dpi_raw(profile).await.map(|r| hex(&r)));
        show("buttons", mouse.buttons_raw(profile).await.map(|r| hex(&r)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_choices_are_unique() {
        for (i, (_, a)) in ACTIONS.iter().enumerate() {
            assert_eq!(ACTIONS.iter().position(|(_, b)| b == a), Some(i));
        }
        assert!(ACTIONS.len() < CHOICE_OTHER as usize);
        assert_eq!(BUTTONS.len(), 6);
    }
}
