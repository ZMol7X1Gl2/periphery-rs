//! Periphery adapter for Scyrox mice (protocol in the `scyrox` crate).

use scyrox::{config, device, keys, protocol};

use std::time::Duration;

use anyhow::{Result, bail};
use periphery_core::hid::{self, Api, Filter};
use periphery_core::{
    Device, DeviceInfo, DeviceKind, ButtonPos, Driver, Group, Kind, Role, Setting, Snapshot, Value, async_trait,
};

use config::{ANGLE_TUNE_RANGE, Change, KEY_NAMES, MAX_DEBOUNCE, PROFILES, REPORT_RATES, SLEEP_OPTIONS, STAGES};
use device::Mouse;
use keys::{HidKey, KEY_TABLE, MEDIA_TABLE, MOUSE_BUTTONS};
use protocol::{FIRE_INTERVAL, FIRE_TIMES, KeyAction, Macro, MacroEvent, Sensor, Shortcut, cycle, pair};

pub struct ScyroxDriver;

#[async_trait(?Send)]
impl Driver for ScyroxDriver {
    fn name(&self) -> &'static str {
        "scyrox"
    }

    fn filters(&self) -> Vec<Filter> {
        protocol::PRODUCT_IDS
            .iter()
            .map(|pid| Filter { vendor_id: Some(protocol::VENDOR_ID), product_id: Some(*pid), ..Filter::default() })
            .collect()
    }

    async fn discover(&self, api: &Api) -> Vec<Result<Box<dyn Device>>> {
        let present = api.devices().any(|d| {
            d.vendor_id == protocol::VENDOR_ID && protocol::PRODUCT_IDS.contains(&d.product_id)
        });
        if !present {
            return Vec::new();
        }
        vec![Mouse::open(api).await.map(|m| Box::new(ScyroxDevice::new(m)) as Box<dyn Device>)]
    }
}

pub struct ScyroxDevice {
    mouse: Mouse,
    info: DeviceInfo,
    last: Option<config::Snapshot>,
}

/// Button choice values: indexes into `KeyAction::SIMPLE`, plus these.
/// Where each of [`KEY_NAMES`] sits on the mouse.
const KEY_POS: [ButtonPos; 6] =
    [ButtonPos::Left, ButtonPos::Right, ButtonPos::Middle, ButtonPos::Back, ButtonPos::Forward, ButtonPos::Bottom];
const CHOICE_FIRE: i64 = 100;
const CHOICE_COMBO: i64 = 101;
const CHOICE_MEDIA: i64 = 102;
const CHOICE_MACRO: i64 = 103;
const CHOICE_SPECIAL: i64 = 200;

impl ScyroxDevice {
    fn new(mouse: Mouse) -> Self {
        let model = if mouse.product.is_empty() {
            format!("mouse ({:04X})", mouse.product_id)
        } else {
            mouse.product.clone()
        };
        let info = DeviceInfo {
            brand: "Scyrox".into(),
            model,
            kind: DeviceKind::Mouse,
            vid: protocol::VENDOR_ID,
            pid: mouse.product_id,
        };
        ScyroxDevice { mouse, info, last: None }
    }

    fn settings(&self) -> Option<&config::Settings> {
        self.last.as_ref()?.mouse.as_ref().map(|o| &o.settings)
    }

    async fn pair(&self) -> Result<()> {
        let m = &self.mouse;
        if m.info.wired() {
            bail!("pairing only works through the wireless receiver");
        }
        m.enter_pair(if m.info.cid == 0 { 62 } else { m.info.cid }).await?;
        for _ in 0..20 {
            hid::sleep(Duration::from_secs(1)).await;
            match m.pair_state().await?.0 {
                pair::SUCCESS => return Ok(()),
                pair::FAILED => bail!("pairing failed"),
                _ => {}
            }
        }
        bail!("pairing timed out")
    }

    fn change_for(&self, id: &str, value: &Value) -> Result<Option<Change>> {
        let (name, index) = match id.split_once('.') {
            Some((n, i)) => (n, i.parse::<u8>().ok()),
            None => (id, None),
        };
        let idx = || index.ok_or_else(|| anyhow::anyhow!("missing index in {id}"));
        let small = |v: &Value| -> Result<u8> { Ok(v.int()?.clamp(0, 255) as u8) };
        Ok(Some(match name {
            "profile" => Change::Profile(small(value)?),
            "rate" => Change::ReportRate(value.int()? as u32),
            "stages" => Change::StageCount(small(value)?),
            "stage" => Change::CurrentStage(small(value)?),
            "dpi" => Change::Dpi(idx()?, snap_dpi(self.mouse.sensor(), value.int()? as u32)),
            "color" => Change::DpiColor(idx()?, value.color()?),
            "lod" => Change::Lod(small(value)?),
            "debounce" => Change::Debounce(small(value)?),
            "motion_sync" => Change::MotionSync(value.bool()?),
            "angle_snap" => Change::AngleSnap(value.bool()?),
            "ripple" => Change::Ripple(value.bool()?),
            "competition" => Change::Competition(value.bool()?),
            "scan_20k" => Change::Scan20k(value.bool()?),
            "angle_tune" => Change::AngleTune(value.int()?.clamp(-128, 127) as i8),
            "sleep" => Change::SleepSecs(value.int()? as u32),
            "long_range" => Change::LongRange(value.bool()?),
            "key" => {
                let k = idx()?;
                let v = value.int()?;
                let current = self.settings().map(|s| s.keys[k as usize]);
                match v {
                    CHOICE_SPECIAL => return Ok(None),
                    CHOICE_COMBO => {
                        let existing = self.settings().and_then(|s| s.shortcuts[k as usize].clone()).filter(|s| !s.is_media());
                        Change::Shortcut(k, existing.unwrap_or_else(|| Shortcut::parse("LCtrl+C").unwrap()))
                    }
                    CHOICE_MEDIA => {
                        let existing = self.settings().and_then(|s| s.shortcuts[k as usize].clone()).filter(Shortcut::is_media);
                        Change::Shortcut(k, existing.unwrap_or_else(|| Shortcut::media(MEDIA_TABLE[1].1)))
                    }
                    CHOICE_MACRO => {
                        let (m, c) = self.current_macro(k);
                        Change::Macro(k, m, c)
                    }
                    CHOICE_FIRE => match current {
                        Some(a @ KeyAction::Fire { .. }) => Change::Key(k, a),
                        _ => Change::Key(k, KeyAction::DEFAULT_FIRE),
                    },
                    v => match KeyAction::SIMPLE.get(v as usize) {
                        Some(a) => Change::Key(k, *a),
                        None => bail!("unknown button action {v}"),
                    },
                }
            }
            "fire_interval" | "fire_times" => {
                let k = idx()?;
                let Some(KeyAction::Fire { mut interval, mut times }) =
                    self.settings().map(|s| s.keys[k as usize])
                else {
                    bail!("button {k} is not a fire button");
                };
                if name == "fire_interval" {
                    interval = small(value)?;
                } else {
                    times = small(value)?;
                }
                Change::Key(k, KeyAction::Fire { interval, times })
            }
            "combo" => {
                let Value::Text(t) = value else { bail!("combo needs text") };
                let Some(sc) = Shortcut::parse(&t.replace(' ', "")) else {
                    bail!("can't read the combo {t:?}: use up to 3 key names joined by +");
                };
                Change::Shortcut(idx()?, sc)
            }
            "media" => Change::Shortcut(idx()?, Shortcut::media(value.int()? as u16)),
            "macro" => {
                let Value::Text(t) = value else { bail!("macro needs text") };
                let k = idx()?;
                let (mut m, c) = self.current_macro(k);
                m.events = parse_events(t)?;
                Change::Macro(k, m, c)
            }
            "macro_repeat" => {
                let k = idx()?;
                let (m, _) = self.current_macro(k);
                Change::Macro(k, m, small(value)?)
            }
            _ => bail!("unknown setting {id}"),
        }))
    }

    /// The button's macro and repeat mode, or a starter macro.
    fn current_macro(&self, k: u8) -> (Macro, u8) {
        let s = self.settings();
        let cycle = match s.map(|s| s.keys[k as usize]) {
            Some(KeyAction::Macro { cycle }) => cycle,
            _ => 1,
        };
        let m = s.and_then(|s| s.macros[k as usize].clone()).unwrap_or_else(|| Macro {
            name: format!("Button {}", k + 1),
            events: vec![
                MacroEvent { press: true, key: HidKey::new(1, 0x04), delay_ms: 20 },
                MacroEvent { press: false, key: HidKey::new(1, 0x04), delay_ms: 20 },
            ],
        });
        (m, cycle)
    }
}

/// Space-free token for a key, accepted back by [`HidKey::parse`].
fn key_token(k: HidKey) -> String {
    match k.kind {
        4 => {
            let names = ["left", "right", "middle", "back", "forward"];
            MOUSE_BUTTONS
                .iter()
                .position(|(_, v)| *v == k.value)
                .map_or_else(|| k.label(), |i| format!("mouse-{}", names[i]))
        }
        2 => format!("media:{}", k.label().to_lowercase().replace(' ', "_")),
        _ => KEY_TABLE
            .iter()
            .find(|(_, _, kind, v)| *kind == k.kind && *v == k.value)
            .map_or_else(|| k.label(), |(name, ..)| name.to_string()),
    }
}

fn events_to_text(events: &[MacroEvent]) -> String {
    events
        .iter()
        .map(|e| format!("{}{}@{}", if e.press { "+" } else { "-" }, key_token(e.key), e.delay_ms))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses "+Key", "-Key" or "Key" (tap) tokens, each with an optional "@ms".
fn parse_events(text: &str) -> Result<Vec<MacroEvent>> {
    let mut out = Vec::new();
    for t in text.split_whitespace() {
        let (body, delay) = match t.rsplit_once('@') {
            Some((b, d)) => (b, d.parse::<u16>().map_err(|_| anyhow::anyhow!("bad delay in {t:?}"))?),
            None => (t, 20),
        };
        let (press, release, name) = match body.as_bytes().first() {
            Some(b'+') => (true, false, &body[1..]),
            Some(b'-') if body.len() > 1 => (false, true, &body[1..]),
            _ => (true, true, body),
        };
        let name = if name.starts_with("media:") { name.replace('_', " ") } else { name.to_string() };
        let Some(key) = HidKey::parse(&name) else { bail!("unknown key {name:?}") };
        if press {
            out.push(MacroEvent { press: true, key, delay_ms: if release { 20 } else { delay } });
        }
        if release {
            out.push(MacroEvent { press: false, key, delay_ms: delay });
        }
    }
    if out.is_empty() {
        bail!("the macro is empty");
    }
    if out.len() > Macro::MAX_EVENTS {
        bail!("a macro holds at most {} events", Macro::MAX_EVENTS);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_text_roundtrip() {
        let events = parse_events("+ShiftLeft H -ShiftLeft i@50 mouse-left media:play/pause").unwrap();
        assert_eq!(events.len(), 10);
        let text = events_to_text(&events);
        assert_eq!(parse_events(&text).unwrap(), events);
    }
}

/// Rounds to the nearest DPI the sensor can represent.
fn snap_dpi(sensor: Sensor, dpi: u32) -> u32 {
    let step = match sensor {
        Sensor::Paw3950 => 50,
        Sensor::Paw3955 if dpi > 42000 => 2,
        Sensor::Paw3955 => 1,
    };
    ((dpi + step / 2) / step * step).clamp(step, sensor.max_dpi())
}

fn fmt_secs(secs: u32) -> String {
    if secs < 60 { format!("{secs} s") } else { format!("{} min", secs / 60) }
}

fn button_choice(action: KeyAction, s: &config::Settings, k: usize) -> (i64, Option<String>) {
    if let Some(i) = KeyAction::SIMPLE.iter().position(|a| *a == action) {
        return (i as i64, None);
    }
    match action {
        KeyAction::Fire { .. } => (CHOICE_FIRE, None),
        KeyAction::Shortcut if s.shortcuts[k].as_ref().is_some_and(Shortcut::is_media) => (CHOICE_MEDIA, None),
        KeyAction::Shortcut => (CHOICE_COMBO, None),
        KeyAction::Macro { .. } => (CHOICE_MACRO, None),
        a => (CHOICE_SPECIAL, Some(a.label())),
    }
}

#[async_trait(?Send)]
impl Device for ScyroxDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    async fn snapshot(&mut self) -> Result<Snapshot> {
        let snap = self.mouse.snapshot().await?;
        let mut out = Snapshot::default();
        let wired = snap.info.wired();
        let conn = if wired { "Wired".to_string() } else { "Wireless".to_string() };
        out.status.push(("Connection".into(), conn));
        if let Some(v) = &snap.dongle_version {
            out.status.push(("Receiver firmware".into(), v.clone()));
        }
        let Some(online) = &snap.mouse else {
            out.notice = Some("The mouse is asleep or switched off — move it to wake it up.".into());
            if !wired {
                let mut g = Group::new("Wireless");
                g.push_as(Role::Wireless, Setting::action(
                    "pair",
                    "Pair a mouse",
                    Some("Hold left + right + middle on the mouse for 3 s until the light flashes, keep it close to the receiver, then continue."),
                ));
                out.groups.push(g);
            }
            self.last = Some(snap);
            return Ok(out);
        };
        let s = &online.settings;
        let (level, charging, mv) = online.battery;
        out.status.push((
            "Battery".into(),
            format!("{level}%{} ({mv} mV)", if charging { " charging" } else { "" }),
        ));
        out.status.push(("Firmware".into(), online.firmware.clone()));
        out.status.push(("Sensor".into(), format!("{:?}", snap.sensor)));

        if let Some(p) = online.profile {
            let mut g = Group::new("Profile");
            g.push_as(Role::Profile, Setting::new(
                "profile",
                "Onboard profile",
                Kind::choice((0..PROFILES as i64).map(|i| (i, format!("Profile {}", i + 1)))),
                Value::Int(p as i64),
            ));
            out.groups.push(g);
        }

        let mut perf = Group::new("Performance");
        let max_rate = snap.info.max_report_rate();
        perf.push_as(Role::PollingRate, Setting::new(
            "rate",
            "Polling rate",
            Kind::choice(
                REPORT_RATES.iter().filter(|r| **r <= max_rate).map(|r| (*r as i64, format!("{r} Hz"))),
            ),
            Value::Int(s.report_rate as i64),
        ));
        perf.push_as(Role::StageCount, Setting::new("stages", "DPI stages", Kind::range(1, STAGES as i64), Value::Int(s.stage_count as i64)));
        perf.push_as(Role::ActiveStage, Setting::new(
            "stage",
            "Active stage",
            Kind::choice((0..s.stage_count as i64).map(|i| (i, format!("Stage {}", i + 1)))),
            Value::Int(s.current_stage as i64),
        ));
        out.groups.push(perf);

        let mut dpi = Group::new("DPI");
        let step = if snap.sensor == Sensor::Paw3950 { 50 } else { 1 };
        for i in 0..s.stage_count as usize {
            dpi.push_as(Role::Dpi(i as u8), Setting::new(
                format!("dpi.{i}"),
                format!("Stage {} DPI", i + 1),
                Kind::range_unit(step, snap.sensor.max_dpi() as i64, step, 1.0, " DPI"),
                Value::Int(s.dpis[i].0 as i64),
            ));
            dpi.push_as(Role::StageColor(i as u8), Setting::new(
                format!("color.{i}"),
                format!("Stage {} colour", i + 1),
                Kind::Color,
                Value::Color(s.colors[i]),
            ));
        }
        out.groups.push(dpi);

        let mut sensor = Group::new("Sensor");
        sensor.push_as(Role::LiftOff, Setting::new(
            "lod",
            "Lift-off distance",
            Kind::choice(config::lod_options(snap.sensor).iter().map(|(v, l)| (*v as i64, *l))),
            Value::Int(s.lod as i64),
        ));
        sensor.push_as(Role::MotionSync, Setting::toggle("motion_sync", "Motion sync", s.motion_sync));
        sensor.push_as(
            Role::AngleSnap,
            Setting::toggle("angle_snap", "Angle snapping", s.angle_snap)
                .help("Straightens slightly diagonal movements"),
        );
        sensor.push_as(Role::Ripple, Setting::toggle("ripple", "Ripple control", s.ripple));
        if let Some(v) = s.competition {
            sensor.push(
                Setting::toggle("competition", "Competition mode", v)
                    .help("Highest-performance sensor mode; sensor LED at full brightness"),
            );
        }
        if let Some(v) = s.scan_20k {
            sensor.push(
                Setting::toggle("scan_20k", "20K FPS scanning", v)
                    .help("Requires competition mode"),
            );
        }
        if let Some(v) = s.angle_tune {
            sensor.push(Setting::new(
                "angle_tune",
                "Angle tune",
                Kind::range_unit(*ANGLE_TUNE_RANGE.start() as i64, *ANGLE_TUNE_RANGE.end() as i64, 1, 1.0, "°"),
                Value::Int(v as i64),
            ));
        }
        sensor.push_as(
            Role::Debounce,
            Setting::new("debounce", "Debounce", Kind::range_unit(0, MAX_DEBOUNCE as i64, 1, 1.0, " ms"), Value::Int(s.debounce as i64))
                .help("Raise it if a button double-clicks"),
        );
        out.groups.push(sensor);

        let mut buttons = Group::new("Buttons");
        for (k, name) in KEY_NAMES.iter().enumerate() {
            let (value, special) = button_choice(s.keys[k], s, k);
            let mut options: Vec<(i64, String)> =
                KeyAction::SIMPLE.iter().enumerate().map(|(i, a)| (i as i64, a.label())).collect();
            options.push((CHOICE_FIRE, "Fire button".into()));
            options.push((CHOICE_COMBO, "Combo key".into()));
            options.push((CHOICE_MEDIA, "Media key".into()));
            options.push((CHOICE_MACRO, "Macro".into()));
            if let Some(text) = special {
                options.push((CHOICE_SPECIAL, text));
            }
            let mut label = name.to_string();
            label[..1].make_ascii_uppercase();
            let pos = KEY_POS[k];
            let opt = Role::ButtonOption(pos);
            buttons.push_as(Role::Button(pos), Setting::new(format!("key.{k}"), label, Kind::Choice(options), Value::Int(value)));
            if let KeyAction::Fire { interval, times } = s.keys[k] {
                buttons.push_as(
                    opt,
                    Setting::new(
                        format!("fire_interval.{k}"),
                        "  Fire interval",
                        Kind::range_unit(*FIRE_INTERVAL.start() as i64, *FIRE_INTERVAL.end() as i64, 1, 1.0, " ms"),
                        Value::Int(interval as i64),
                    ),
                );
                buttons.push_as(
                    opt,
                    Setting::new(
                        format!("fire_times.{k}"),
                        "  Fire clicks",
                        Kind::range(*FIRE_TIMES.start() as i64, *FIRE_TIMES.end() as i64),
                        Value::Int(times as i64),
                    )
                    .help("0 = keep clicking while held"),
                );
            }
            match s.keys[k] {
                KeyAction::Shortcut => match &s.shortcuts[k] {
                    Some(sc) if sc.is_media() => buttons.push_as(
                        opt,
                        Setting::new(
                            format!("media.{k}"),
                            "  Media key",
                            Kind::choice(MEDIA_TABLE.iter().map(|(l, v)| (*v as i64, *l))),
                            Value::Int(sc.0[0].value as i64),
                        ),
                    ),
                    sc => buttons.push_as(
                        opt,
                        Setting::new(
                            format!("combo.{k}"),
                            "  Keys",
                            Kind::Text { max_len: 60 },
                            Value::Text(sc.as_ref().map(Shortcut::label).unwrap_or_default()),
                        )
                        .help("Up to 3 keys joined by +, e.g. LCtrl+LShift+S"),
                    ),
                },
                KeyAction::Macro { cycle: c } => {
                    let events = s.macros[k].as_ref().map(|m| events_to_text(&m.events)).unwrap_or_default();
                    buttons.push_as(
                        opt,
                        Setting::new(format!("macro.{k}"), "  Macro", Kind::Text { max_len: 2000 }, Value::Text(events))
                            .help("Events separated by spaces: +Key press, -Key release, Key tap, @ms delay after, e.g. +ShiftLeft H -ShiftLeft I@50. Mouse buttons: mouse-left …"),
                    );
                    let mut repeats: Vec<(i64, String)> = [1u8, 2, 3, 5, 10]
                        .iter()
                        .map(|n| (*n as i64, cycle::label(*n)))
                        .collect();
                    for m in [cycle::WHILE_HELD, cycle::UNTIL_ANY_KEY, cycle::UNTIL_PRESSED_AGAIN] {
                        repeats.push((m as i64, cycle::label(m)));
                    }
                    if !repeats.iter().any(|(v, _)| *v == c as i64) {
                        repeats.push((c as i64, cycle::label(c)));
                    }
                    buttons.push_as(opt, Setting::new(format!("macro_repeat.{k}"), "  Repeat", Kind::Choice(repeats), Value::Int(c as i64)));
                }
                _ => {}
            }
        }
        out.groups.push(buttons);

        let mut power = Group::new("Power & wireless");
        power.push_as(Role::Sleep, Setting::new(
            "sleep",
            "Sleep after",
            Kind::choice(SLEEP_OPTIONS.iter().map(|s| (*s as i64, fmt_secs(*s)))),
            Value::Int(s.sleep_secs as i64),
        ));
        if !wired {
            if let Some(lr) = online.long_range {
                power.push_as(
                    Role::Wireless,
                    Setting::toggle("long_range", "Long-range mode", lr)
                        .help("Stronger signal at distance; shortens battery life"),
                );
            }
            power.push_as(Role::Wireless, Setting::action(
                "pair",
                "Pair a mouse",
                Some("Hold left + right + middle on the mouse for 3 s until the light flashes, keep it close to the receiver, then continue."),
            ));
        }
        power.push_as(Role::FactoryReset, Setting::action("reset", "Factory reset", Some("Reset every setting to factory defaults?")));
        out.groups.push(power);

        self.last = Some(snap);
        Ok(out)
    }

    async fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        match id {
            "pair" => return self.pair().await,
            "reset" => {
                self.mouse.require_online().await?;
                return self.mouse.factory_reset().await;
            }
            _ => {}
        }
        let Some(change) = self.change_for(id, value)? else { return Ok(()) };
        self.mouse.require_online().await?;
        self.mouse.apply(&change).await
    }

    async fn poll_changes(&mut self) -> Result<bool> {
        Ok(self.mouse.poll_status().await?.is_some())
    }
}
