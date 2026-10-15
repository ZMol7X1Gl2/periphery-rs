//! Periphery adapter for AULA keyboards (protocol in the `aula` crate): the
//! wired F75 and its siblings on `258A:010C`.

use anyhow::{Result, bail};
use aula::lighting::EffectSettings;
use aula::models::{self, COLORFUL, COLORS, EFFECTS, SLEEP};
use aula::{Keyboard, LightingPage, Model};
use periphery_core::hid::{Api, Filter};
use periphery_core::{Device, DeviceInfo, DeviceKind, Driver, Group, Kind, Setting, Snapshot, Value, async_trait};

pub struct AulaDriver;

#[async_trait(?Send)]
impl Driver for AulaDriver {
    fn name(&self) -> &'static str {
        "aula"
    }

    fn filters(&self) -> Vec<Filter> {
        vec![models::FILTER]
    }

    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>> {
        Keyboard::open_all(api)
            .await
            .into_iter()
            .map(|r| r.map(|kb| Box::new(AulaDevice::new(kb)) as Box<dyn Device>))
            .collect()
    }
}

pub struct AulaDevice {
    kb: Keyboard,
    info: DeviceInfo,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn sleep_label(v: u8) -> String {
    let s = v as u32 * 30;
    if s.is_multiple_of(60) { format!("{} min", s / 60) } else { format!("{s} s") }
}

impl AulaDevice {
    fn new(kb: Keyboard) -> Self {
        let model = kb.model().map_or_else(|| "keyboard".to_string(), |m| m.name.to_string());
        let info =
            DeviceInfo { brand: "AULA".into(), model, kind: DeviceKind::Keyboard, vid: models::VENDOR_ID, pid: models::PRODUCT_ID };
        AulaDevice { kb, info }
    }

    fn require_model(&self) -> Result<&'static Model> {
        match self.kb.model() {
            Some(m) => Ok(m),
            None => bail!("this AULA model (identity {}) isn't supported yet", hex(&self.kb.identity)),
        }
    }

    /// Changes the live effect's settings.
    async fn edit_current(&self, edit: impl FnOnce(&mut EffectSettings)) -> Result<()> {
        let mut page = self.kb.lighting().await?;
        let effect = page.effect();
        let mut s = page.settings(effect)?;
        edit(&mut s);
        page.set_settings(effect, s)?;
        self.kb.set_lighting(&page).await
    }

    fn lighting_group(model: &Model, page: &LightingPage) -> Result<Group> {
        let mut g = Group::new("Lighting");
        let effect = page.effect();
        let mut choices: Vec<(i64, String)> = EFFECTS.iter().map(|e| (e.value as i64, e.name.to_string())).collect();
        if models::effect(effect).is_none() {
            choices.push((effect as i64, format!("Unknown ({effect})")));
        }
        g.push(Setting::new("effect", "Effect", Kind::Choice(choices), Value::Int(effect as i64)));
        let Some(fx) = models::effect(effect) else { return Ok(g) };
        let s = page.settings(effect)?;
        if fx.brightness {
            g.push(
                Setting::new(
                    "brightness",
                    "Brightness",
                    Kind::range(0, model.max_brightness as i64),
                    Value::Int(s.brightness.min(model.max_brightness) as i64),
                )
                .help("Every effect remembers its own brightness, speed and colour"),
            );
        }
        if fx.speed {
            g.push(Setting::new("speed", "Speed", Kind::range(0, 4), Value::Int(s.speed.min(4) as i64)));
        }
        if fx.color || fx.colorful {
            let mut colors: Vec<(i64, String)> = Vec::new();
            if fx.color {
                colors.extend(COLORS.iter().enumerate().map(|(i, c)| (i as i64, c.to_string())));
            }
            if fx.colorful {
                colors.push((COLORFUL as i64, "Colourful".into()));
            }
            if !colors.iter().any(|(v, _)| *v == s.color as i64) {
                colors.push((s.color as i64, format!("Unknown ({})", s.color)));
            }
            g.push(Setting::new("color", "Colour", Kind::Choice(colors), Value::Int(s.color as i64)));
        }
        Ok(g)
    }
}

#[async_trait(?Send)]
impl Device for AulaDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    async fn snapshot(&mut self) -> Result<Snapshot> {
        let r = self.read_snapshot().await;
        if let Err(e) = &r {
            log::warn!(target: "aula", "reading the keyboard failed: {e:#}");
        }
        r
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        log::info!(target: "aula", "set {id} = {value:?}");
        let r = self.apply(id, value).await;
        if let Err(e) = &r {
            log::warn!(target: "aula", "setting {id} to {value:?} failed: {e:#}");
        }
        r
    }
}

impl AulaDevice {
    async fn read_snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        out.status.push(("Connection".into(), "Wired".into()));
        out.status.push(("Identity".into(), hex(&self.kb.identity)));
        let Ok(model) = self.require_model() else {
            out.notice = Some(format!(
                "This AULA keyboard (identity {}) isn't in Periphery's list yet. Please report the identity together with the keyboard's name.",
                hex(&self.kb.identity)
            ));
            return Ok(out);
        };
        let page = self.kb.lighting().await?;
        out.groups.push(Self::lighting_group(model, &page)?);

        if model.sleep {
            let mut g = Group::new("Power");
            let v = page.sleep();
            let mut choices: Vec<(i64, String)> = SLEEP.iter().map(|(v, _)| (*v as i64, sleep_label(*v))).collect();
            if !SLEEP.iter().any(|(s, _)| *s == v) {
                choices.push((v as i64, format!("{} (current)", sleep_label(v))));
            }
            g.push(
                Setting::new("sleep", "Lighting sleep after", Kind::Choice(choices), Value::Int(v as i64))
                    .help("Idle time before the backlight turns off"),
            );
            out.groups.push(g);
        }

        let mut g = Group::new("Maintenance");
        g.push(Setting::action(
            "reset",
            "Factory reset",
            Some("Restore the keyboard's factory keymap and lighting?"),
        ));
        out.groups.push(g);
        Ok(out)
    }

    async fn apply(&mut self, id: &str, value: &Value) -> Result<()> {
        let model = self.require_model()?;
        match id {
            "effect" => {
                let v = value.int()?;
                let Some(fx) = EFFECTS.iter().find(|e| e.value as i64 == v) else { bail!("unknown effect {v}") };
                let mut page = self.kb.lighting().await?;
                page.set_effect(fx.value);
                self.kb.set_lighting(&page).await
            }
            "brightness" => {
                let v = value.int()?;
                if !(0..=model.max_brightness as i64).contains(&v) {
                    bail!("brightness must be 0..={}", model.max_brightness);
                }
                self.edit_current(|s| s.brightness = v as u8).await
            }
            "speed" => {
                let v = value.int()?;
                if !(0..=4).contains(&v) {
                    bail!("speed must be 0..=4");
                }
                self.edit_current(|s| s.speed = v as u8).await
            }
            "color" => {
                let v = value.int()?;
                if !(0..=COLORFUL as i64).contains(&v) {
                    bail!("invalid colour {v}");
                }
                self.edit_current(|s| s.color = v as u8).await
            }
            "sleep" => {
                if !model.sleep {
                    bail!("this keyboard has no sleep timer setting");
                }
                let v = value.int()?;
                let Some((v, _)) = SLEEP.iter().find(|(s, _)| *s as i64 == v) else { bail!("invalid sleep time {v}") };
                let mut page = self.kb.lighting().await?;
                page.set_sleep(*v);
                self.kb.set_lighting(&page).await
            }
            "reset" => self.kb.factory_reset().await,
            _ => bail!("unknown setting {id}"),
        }
    }
}

/// Read-only protocol details for `periphery-cli diag`.
pub async fn diag(api: &Api) -> Vec<String> {
    let mut out = Vec::new();
    for info in Keyboard::candidates(api) {
        let head = format!("aula {:04x}:{:04x} if={}", info.vendor_id, info.product_id, info.interface_number);
        let kb = match Keyboard::open(api, info).await {
            Ok(kb) => kb,
            Err(e) => {
                out.push(format!("{head}: {e:#}"));
                continue;
            }
        };
        out.push(format!("{head}: identity {} {:?}", hex(&kb.identity), kb.model().map(|m| m.name)));
        match kb.lighting().await {
            Ok(page) => {
                for (i, row) in page.bytes().chunks(16).enumerate() {
                    out.push(format!("  {:04x}  {}", i * 16, hex(row)));
                }
            }
            Err(e) => out.push(format!("  lighting   ERROR {e:#}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_labels() {
        assert_eq!(sleep_label(3), "90 s");
        assert_eq!(sleep_label(10), "5 min");
    }
}
