//! Periphery adapter for ROCCAT mice (protocol in the `roccat` crate): the
//! Kone Aimo and Kone Aimo Remastered. Settings shown and edited are those of
//! the active onboard profile.

use anyhow::{Result, bail};
use periphery_core::hid::{Api, Filter};
use periphery_core::{Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Setting, Snapshot, Value, async_trait};
use roccat::keys::{parse_shortcut, shortcut_label};
use roccat::kone_aimo::{
    ACTIONS, BUTTON_NAMES, DPI_STAGES, EFFECTS, PROFILES, REPORT_RATES, SLOTS, SPEEDS, VENDOR_ID, Zone, action,
    action_name,
};
use roccat::{Buttons, KoneAimo};

pub struct RoccatDriver;

#[async_trait(?Send)]
impl Driver for RoccatDriver {
    fn name(&self) -> &'static str {
        "roccat"
    }

    fn filters(&self) -> Vec<Filter> {
        KoneAimo::filters()
    }

    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>> {
        let mut out = Vec::new();
        for info in KoneAimo::candidates(api) {
            match KoneAimo::open(api, info).await {
                Ok(m) => out.push(Ok(Box::new(RoccatDevice::new(m)) as Box<dyn Device>)),
                Err(e) => out.push(Err(e.context(format!("{:04x}:{:04x}", info.vendor_id, info.product_id)))),
            }
        }
        out
    }
}

pub struct RoccatDevice {
    mouse: KoneAimo,
    info: DeviceInfo,
}

/// Shortcut used when a button is switched to "Keyboard shortcut".
const DEFAULT_SHORTCUT: (u8, u8) = (0x01, 0x06); // Ctrl + C

impl RoccatDevice {
    fn new(mouse: KoneAimo) -> Self {
        let info = DeviceInfo {
            brand: "ROCCAT".into(),
            model: mouse.model.name.into(),
            kind: DeviceKind::Mouse,
            vid: VENDOR_ID,
            pid: mouse.model.product_id,
        };
        RoccatDevice { mouse, info }
    }

    /// The active profile's buttons, with Swarm's defaults in a blank profile.
    async fn buttons(&self, profile: u8) -> Result<Buttons> {
        let mut b = self.mouse.buttons(profile).await?;
        b.fill_blank();
        Ok(b)
    }

    async fn set_button(&self, slot: usize, edit: impl FnOnce(&mut Buttons) -> Result<()>) -> Result<()> {
        if slot >= SLOTS {
            bail!("no button {slot}");
        }
        let profile = self.mouse.active_profile().await?;
        let old = self.buttons(profile).await?;
        let mut new = old.clone();
        edit(&mut new)?;
        if new == old {
            return Ok(());
        }
        if let (None, Some(problem)) = (old.problem(), new.problem()) {
            bail!("not changed: {problem}");
        }
        self.mouse.set_buttons(profile, &new).await
    }
}

fn slot_label(slot: usize) -> String {
    let name = BUTTON_NAMES[slot % BUTTON_NAMES.len()];
    if slot < BUTTON_NAMES.len() { name.into() } else { format!("Easy-Shift + {}", name.to_lowercase()) }
}

/// Adds `value` to `options` under `label` unless it's already there.
fn with_current(mut options: Vec<(i64, String)>, value: u8, label: impl FnOnce() -> String) -> Kind {
    if !options.iter().any(|(v, _)| *v == value as i64) {
        options.push((value as i64, label()));
    }
    Kind::Choice(options)
}

fn choices(items: &[(u8, &str)]) -> Vec<(i64, String)> {
    items.iter().map(|(v, l)| (*v as i64, l.to_string())).collect()
}

#[async_trait(?Send)]
impl Device for RoccatDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    async fn snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        let model = self.mouse.model;
        out.status.push(("Connection".into(), "USB".into()));
        let profile = self.mouse.active_profile().await?;
        let s = self.mouse.settings(profile).await?;
        if !s.checksum_ok() {
            out.status.push(("Profile data".into(), "checksum mismatch (rewritten on the next change)".into()));
        }

        let mut g = Group::new("Profile");
        g.push(Setting::new(
            "profile",
            "Onboard profile",
            Kind::choice((0..PROFILES as i64).map(|i| (i, format!("Profile {}", i + 1)))),
            Value::Int(profile as i64),
        ));
        out.groups.push(g);

        let mut perf = Group::new("Performance");
        let mut rates: Vec<(i64, String)> = REPORT_RATES.iter().map(|r| (*r as i64, format!("{r} Hz"))).collect();
        let rate = match s.report_rate() {
            Some(hz) => hz as i64,
            None => {
                rates.push((0, format!("Unknown (index {})", s.raw[26])));
                0
            }
        };
        perf.push(Setting::new("rate", "Polling rate", Kind::Choice(rates), Value::Int(rate)));
        perf.push(Setting::new(
            "stage",
            "Active DPI stage",
            Kind::choice((0..DPI_STAGES as i64).map(|i| (i, format!("Stage {}", i + 1)))),
            Value::Int(s.stage() as i64),
        ));
        out.groups.push(perf);

        let mut g = Group::new("DPI");
        for i in 0..DPI_STAGES {
            let (x, y) = s.dpi(i);
            let mut d = Setting::new(
                format!("dpi.{i}"),
                format!("Stage {} DPI", i + 1),
                Kind::range_unit(model.min_dpi as i64, model.max_dpi as i64, 50, 1.0, " DPI"),
                Value::Int(x as i64),
            );
            if x != y {
                d = d.help(format!("Y axis: {y} DPI (setting this sets both axes)"));
            }
            g.push(d);
        }
        out.groups.push(g);

        let mut g = Group::new("Lighting");
        g.push(Setting::new(
            "effect",
            "Effect",
            with_current(choices(&EFFECTS), s.effect(), || format!("Effect {}", s.effect())),
            Value::Int(s.effect() as i64),
        ));
        g.push(Setting::new(
            "speed",
            "Effect speed",
            with_current(choices(&SPEEDS), s.speed(), || format!("Speed {}", s.speed())),
            Value::Int(s.speed() as i64),
        ));
        g.push(Setting::new("brightness", "Brightness", Kind::range(0, 255), Value::Int(s.brightness() as i64)));
        for (i, z) in Zone::ALL.iter().enumerate() {
            g.push(Setting::new(format!("color.{i}"), z.name(), Kind::Color, Value::Color(s.color(*z))));
        }
        out.groups.push(g);

        let b = self.buttons(profile).await?;
        let mut normal = Group::new("Buttons");
        let mut shift = Group::new("Easy-Shift layer");
        for slot in 0..SLOTS {
            let a = b.action(slot);
            let g = if slot < BUTTON_NAMES.len() { &mut normal } else { &mut shift };
            let mut setting = Setting::new(
                format!("key.{slot}"),
                slot_label(slot),
                with_current(choices(&ACTIONS), a, || action_name(a)),
                Value::Int(a as i64),
            );
            if a == action::MACRO {
                setting = setting.help("Macros can't be edited here yet; choosing another action replaces it");
            }
            g.push(setting);
            if a == action::SHORTCUT {
                let (m, k) = b.shortcut(slot);
                g.push(
                    Setting::new(
                        format!("shortcut.{slot}"),
                        "  Keys",
                        Kind::Text { max_len: 60 },
                        Value::Text(shortcut_label(m, k)),
                    )
                    .help("Modifiers and one key joined by +, e.g. Ctrl+Shift+S"),
                );
            }
        }
        out.groups.push(normal);
        out.groups.push(shift);
        Ok(out)
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        let model = self.mouse.model;
        let (name, index) = match id.split_once('.') {
            Some((n, i)) => (n, i.parse::<usize>().ok()),
            None => (id, None),
        };
        let idx = || index.ok_or_else(|| anyhow::anyhow!("missing index in {id}"));
        match name {
            "profile" => {
                let p = value.int()?;
                if !(0..PROFILES as i64).contains(&p) {
                    bail!("profile must be 1..={PROFILES}");
                }
                self.mouse.set_active_profile(p as u8).await
            }
            "rate" | "stage" | "dpi" | "effect" | "speed" | "brightness" | "color" => {
                let profile = self.mouse.active_profile().await?;
                let old = self.mouse.settings(profile).await?;
                let mut s = old.clone();
                match name {
                    "rate" => {
                        let hz = value.int()?;
                        // 0 stands for an index this crate doesn't know; keep it.
                        if hz != 0 || old.report_rate().is_some() {
                            s.set_report_rate(hz.clamp(0, u16::MAX as i64) as u16)?;
                        }
                    }
                    "stage" => s.set_stage(value.int()?.clamp(0, u8::MAX as i64) as u8)?,
                    "dpi" => s.set_dpi(idx()?, value.int()?.clamp(0, u16::MAX as i64) as u16, model)?,
                    "effect" => {
                        let e = value.int()?;
                        if e != old.effect() as i64 {
                            s.set_effect(e.clamp(0, u8::MAX as i64) as u8)?;
                        }
                    }
                    "speed" => {
                        let v = value.int()?;
                        if v != old.speed() as i64 {
                            s.set_speed(v.clamp(0, u8::MAX as i64) as u8)?;
                        }
                    }
                    "brightness" => {
                        let v = value.int()?;
                        if !(0..=255).contains(&v) {
                            bail!("brightness must be 0..=255");
                        }
                        s.set_brightness(v as u8);
                    }
                    _ => {
                        let Some(z) = Zone::ALL.get(idx()?) else { bail!("no lighting zone {id}") };
                        let rgb = value.color()?;
                        if s.color(*z) != rgb {
                            s.set_color(*z, rgb);
                        }
                    }
                }
                if s == old {
                    return Ok(());
                }
                self.mouse.set_settings(&s).await
            }
            "key" => {
                let slot = idx()?;
                let a = value.int()?;
                self.set_button(slot, |b| {
                    let current = b.action(slot);
                    if a == current as i64 {
                        return Ok(());
                    }
                    let Some((a, _)) = ACTIONS.iter().find(|(c, _)| *c as i64 == a) else {
                        bail!("unknown button action {a}")
                    };
                    match *a {
                        action::SHORTCUT => b.set_shortcut(slot, DEFAULT_SHORTCUT.0, DEFAULT_SHORTCUT.1),
                        a => b.set_action(slot, a),
                    }
                    Ok(())
                })
                .await
            }
            "shortcut" => {
                let slot = idx()?;
                let Value::Text(t) = value else { bail!("a shortcut needs text") };
                let Some((m, k)) = parse_shortcut(t) else {
                    bail!("can't read the shortcut {t:?}: use modifiers and one key joined by +, e.g. Ctrl+Shift+S")
                };
                self.set_button(slot, |b| {
                    b.set_shortcut(slot, m, k);
                    Ok(())
                })
                .await
            }
            _ => bail!("unknown setting {id}"),
        }
    }
}

/// Raw replies for bug reports (`periphery-cli diag`); read-only.
pub async fn diag(api: &Api) -> Vec<String> {
    let mut out = Vec::new();
    let hex = |r: &[u8]| r.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
    for info in KoneAimo::candidates(api) {
        let head = format!("roccat {:04x}:{:04x}", info.vendor_id, info.product_id);
        let mouse = match KoneAimo::open(api, info).await {
            Ok(m) => m,
            Err(e) => {
                out.push(format!("{head}: {e:#}"));
                continue;
            }
        };
        out.push(format!("{head}: {}", mouse.model.name));
        let mut show = |label: &str, r: Result<String>| match r {
            Ok(v) => out.push(format!("  {label:<10} {v}")),
            Err(e) => out.push(format!("  {label:<10} ERROR {e:#}")),
        };
        show("status", mouse.status().await.map(|s| format!("{s:#04x}")));
        let profile = mouse.active_profile().await;
        show("profile", profile.as_ref().map(|p| p.to_string()).map_err(|e| anyhow::anyhow!("{e:#}")));
        let profile = profile.unwrap_or(0);
        show(
            "settings",
            mouse
                .settings(profile)
                .await
                .map(|s| format!("checksum {}: {}", if s.checksum_ok() { "ok" } else { "BAD" }, hex(&s.raw))),
        );
        show("buttons", mouse.buttons(profile).await.map(|b| hex(&b.raw)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_labels() {
        assert_eq!(slot_label(0), "Left button");
        assert_eq!(slot_label(12), "Easy-Shift + left button");
        assert_eq!(slot_label(SLOTS - 1), "Easy-Shift + top button down");
    }

    #[test]
    fn unknown_values_get_an_option() {
        let Kind::Choice(o) = with_current(choices(&EFFECTS), 9, || "Effect 9".into()) else { panic!() };
        assert_eq!(o.last(), Some(&(9, "Effect 9".to_string())));
        let Kind::Choice(o) = with_current(choices(&EFFECTS), 1, || unreachable!()) else { panic!() };
        assert_eq!(o.len(), EFFECTS.len());
    }
}
