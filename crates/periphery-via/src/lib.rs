//! Periphery adapter for QMK/VIA keyboards (any brand), with Keychron
//! extensions when present. Protocol code lives in the `qmk-via` crate.


use anyhow::{Result, bail};
use periphery_core::hidapi::{DeviceInfo as HidInfo, HidApi};
use periphery_core::{
    Device, DeviceInfo, DeviceKind, Driver, Group, KeyPair, KeyPairList, KeyProperty, Keymap, Kind,
    LayoutKey, Setting, Snapshot, Value,
};

use definition::{Definition, KEYCHRON_VID};
use keychron::{Debounce, HeProfile, HeState, KeychronInfo, Nkro, Sleep, Travel};
use menus::{ItemKind, MenuItem};
use qmk_via::transport::Raw;

use qmk_via::{USAGE, USAGE_PAGE, cmd, definition, keychron, keycodes, macros, menus};

pub struct ViaDriver;

impl Driver for ViaDriver {
    fn name(&self) -> &'static str {
        "qmk-via"
    }

    fn discover(&self, api: &HidApi) -> Vec<Result<Box<dyn Device>>> {
        api.device_list()
            .filter(|d| d.usage_page() == USAGE_PAGE && d.usage() == USAGE)
            .map(|d| ViaKeyboard::open(api, d).map(|k| Box::new(k) as Box<dyn Device>))
            .collect()
    }
}

pub struct ViaKeyboard {
    raw: Raw,
    info: DeviceInfo,
    protocol: u16,
    layers: u8,
    macros: u8,
    def: Option<Definition>,
    def_error: Option<String>,
    menu: Vec<MenuItem>,
    kc: Option<KeychronInfo>,
    // Last values read, needed for writes that carry several fields.
    sleep: Option<Sleep>,
    debounce: Option<Debounce>,
    perf: Option<(u8, u8)>,
    he: Option<HeState>,
    he_profile: Option<HeProfile>,
    /// Matrix position → LED index, read once.
    led_map: Option<Vec<Option<u8>>>,
    macro_size: u16,
    macro_bytes: Vec<Vec<u8>>,
}

const SLEEP_BACKLIGHT: [u16; 10] = [5, 10, 30, 60, 120, 300, 600, 1200, 1800, 3600];
const SLEEP_MAGNET: [u16; 8] = [60, 120, 300, 600, 1200, 1800, 3600, 7200];
const SLEEP_DEVICE: [u16; 8] = [60, 300, 600, 1200, 1800, 3600, 7200, 14400];

fn fmt_secs(s: u16) -> String {
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h", s as f32 / 3600.0),
    }
}

fn secs_choice(options: &[u16], current: u16) -> Kind {
    let mut v: Vec<u16> = options.to_vec();
    if !v.contains(&current) {
        v.push(current);
        v.sort();
    }
    Kind::choice(v.into_iter().map(|s| (s as i64, fmt_secs(s))))
}

impl ViaKeyboard {
    fn open(api: &HidApi, d: &HidInfo) -> Result<Self> {
        let raw = Raw::new(d.open_device(api)?);
        raw.drain();
        let p = raw.query(&[cmd::GET_PROTOCOL_VERSION])?;
        let protocol = u16::from_be_bytes([p[1], p[2]]);
        let json_version = raw
            .try_query(&[cmd::GET_KEYBOARD_VALUE, 4])?
            .map(|r| u32::from_be_bytes([r[2], r[3], r[4], r[5]]))
            .filter(|v| *v <= 100)
            .unwrap_or(0);
        let layers = raw.query(&[cmd::GET_LAYER_COUNT])?[1];
        let macros = raw.try_query(&[cmd::MACRO_GET_COUNT])?.map_or(0, |r| r[1]);
        let kc = keychron::info(&raw)?;

        let (vid, pid) = (d.vendor_id(), d.product_id());
        let (def, def_error) = match definition::load(vid, pid, protocol, json_version) {
            Ok(def) => (def, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let menu = def.as_ref().map(|d| menus::parse(&d.menus)).unwrap_or_default();
        let product = d.product_string().unwrap_or("Keyboard").to_string();
        let brand = if vid == KEYCHRON_VID {
            "Keychron".to_string()
        } else {
            d.manufacturer_string().unwrap_or("QMK").to_string()
        };
        let model = def.as_ref().map_or(product, |d| d.name.clone());
        Ok(ViaKeyboard {
            raw,
            info: DeviceInfo { brand, model, kind: DeviceKind::Keyboard, vid, pid },
            protocol,
            layers,
            macros,
            def,
            def_error,
            menu,
            kc,
            sleep: None,
            debounce: None,
            perf: None,
            he: None,
            he_profile: None,
            led_map: None,
            macro_size: 0,
            macro_bytes: Vec::new(),
        })
    }

    fn lighting(&self) -> Result<Vec<Group>> {
        // Read every value first: `showIf` expressions refer to other values.
        let mut read: Vec<(&MenuItem, Kind, Value, i64)> = Vec::new();
        for item in &self.menu {
            let Some(r) = self.raw.try_query(&[cmd::CUSTOM_GET_VALUE, item.channel, item.value_id])? else {
                continue;
            };
            let data = &r[3..];
            let raw = if item.width() == 2 { u16::from_be_bytes([data[0], data[1]]) as i64 } else { data[0] as i64 };
            let (kind, value) = match &item.kind {
                ItemKind::Range { min, max } => (Kind::range(*min, *max), Value::Int(raw)),
                ItemKind::Dropdown(opts) => (Kind::Choice(opts.clone()), Value::Int(raw)),
                ItemKind::Toggle => (Kind::Toggle, Value::Bool(raw != 0)),
                ItemKind::Color => (Kind::Color, Value::Color(menus::hs_to_rgb(data[0], data[1]))),
            };
            read.push((item, kind, value, raw));
        }
        let lookup = |name: &str| read.iter().find(|(i, ..)| i.name == name).map(|(.., raw)| *raw);
        let mut groups: Vec<Group> = Vec::new();
        for (item, kind, value, _) in &read {
            if let Some(expr) = &item.show_if {
                if !menus::show_if(expr, &lookup) {
                    continue;
                }
            }
            let group = match item.section.as_str() {
                "" | "Backlight" => "Lighting",
                s => s,
            };
            let setting = Setting::new(item.id(), item.label.clone(), kind.clone(), value.clone());
            match groups.iter_mut().find(|g| g.label == group) {
                Some(g) => g.push(setting),
                None => {
                    let mut g = Group::new(group);
                    g.push(setting);
                    groups.push(g);
                }
            }
        }
        Ok(groups)
    }

    fn he_group(&mut self) -> Result<Option<Group>> {
        self.he = keychron::he_read(&self.raw)?;
        let Some(he) = &self.he else { return Ok(None) };
        let mut g = Group::new("Magnetic switches");
        if he.version != 4 {
            g.push(Setting::new(
                "he.unsupported",
                "Status",
                Kind::Info,
                Value::Text(format!("Hall-effect command version {} is not supported yet", he.version)),
            ));
            return Ok(Some(g));
        }
        let t = he.global;
        g.push(Setting::new(
            "he.profile",
            "Profile",
            Kind::choice(he.names.iter().enumerate().map(|(i, n)| (i as i64, n.clone()))),
            Value::Int(he.current as i64),
        ));
        g.push(
            Setting::new(
                "he.mode",
                "Mode",
                Kind::choice([(keychron::HE_STATIC as i64, "Fixed actuation"), (keychron::HE_RAPID_TRIGGER as i64, "Rapid trigger")]),
                Value::Int(t.mode as i64),
            )
            .help("Rapid trigger resets a key as soon as it starts moving up"),
        );
        let range = |min| Kind::range_unit(min, keychron::HE_MAX as i64, 1, 0.1, " mm");
        g.push(
            Setting::new("he.actuation", "Actuation point", range(keychron::HE_MIN as i64), Value::Int(t.actuation as i64))
                .help("How far a key travels before it registers (applies to all keys)"),
        );
        g.push(Setting::new("he.rt_press", "Rapid trigger press", range(1), Value::Int(t.rt_press as i64)));
        g.push(Setting::new("he.rt_release", "Rapid trigger release", range(1), Value::Int(t.rt_release as i64)));
        Ok(Some(g))
    }

    fn keyboard_group(&mut self) -> Result<Option<Group>> {
        let mut g = Group::new("Keyboard");
        self.debounce = keychron::debounce_get(&self.raw)?;
        match self.debounce {
            Some(Debounce::Qmk { mode, ms }) => {
                g.push(Setting::new(
                    "debounce.mode",
                    "Debounce algorithm",
                    Kind::choice([
                        (0, "Defer (global)"),
                        (2, "Defer (per key)"),
                        (4, "Eager (per key)"),
                        (5, "Eager-defer (per key)"),
                        (6, "None"),
                    ]),
                    Value::Int(mode as i64),
                ));
                g.push(Setting::new("debounce.ms", "Debounce time", Kind::range_unit(0, 50, 1, 1.0, " ms"), Value::Int(ms as i64)));
            }
            Some(Debounce::Telink { press, release }) => {
                g.push(Setting::new("debounce.press", "Press debounce", Kind::range_unit(0, 50, 1, 1.0, " ms"), Value::Int(press as i64)));
                g.push(Setting::new("debounce.release", "Release debounce", Kind::range_unit(1, 50, 1, 1.0, " ms"), Value::Int(release as i64)));
            }
            None => {}
        }
        if let Some((hz, supported)) = keychron::poll_get(&self.raw)? {
            g.push(Setting::new(
                "poll",
                "Polling rate",
                Kind::choice(supported.iter().map(|r| (*r as i64, format!("{r} Hz")))),
                Value::Int(hz as i64),
            ));
        }
        let performance = self.kc.as_ref().is_some_and(|k| k.features.performance);
        self.perf = if performance { keychron::perf_get(&self.raw)? } else { None };
        if let Some((_, mode)) = self.perf {
            g.push(Setting::new(
                "perf",
                "Performance mode",
                Kind::choice([(0, "Battery saver"), (1, "Normal"), (2, "High performance")]),
                Value::Int(mode as i64),
            ));
        }
        if !performance {
            let version = self.kc.as_ref().map_or(0, |k| k.version);
            if let Some(Nkro { switchable: true, on }) = keychron::nkro_get(&self.raw, version)? {
                g.push(Setting::toggle("nkro", "N-key rollover", on));
            }
        }
        if self.def.as_ref().is_some_and(|d| d.has_feature("DIP")) {
            if let Some(func) = keychron::dip_get(&self.raw)? {
                g.push(Setting::new(
                    "dip",
                    "DIP switch controls",
                    Kind::choice([(1, "Polling rate"), (2, "Profile")]),
                    Value::Int(func as i64),
                ));
            }
        }
        Ok((!g.settings.is_empty()).then_some(g))
    }

    fn power_group(&mut self) -> Result<Option<Group>> {
        self.sleep = keychron::sleep_get(&self.raw)?;
        let Some(s) = self.sleep else { return Ok(None) };
        let mut g = Group::new("Power");
        if s.backlight > 0 {
            g.push(Setting::new("sleep.backlight", "Backlight off after", secs_choice(&SLEEP_BACKLIGHT, s.backlight), Value::Int(s.backlight as i64)));
        }
        if s.magnet_scan > 0 {
            g.push(
                Setting::new("sleep.magnet", "Switch scanning idle after", secs_choice(&SLEEP_MAGNET, s.magnet_scan), Value::Int(s.magnet_scan as i64))
                    .help("Must be between the backlight and sleep times"),
            );
        }
        if s.sleep > 0 {
            g.push(Setting::new("sleep.sleep", "Sleep after", secs_choice(&SLEEP_DEVICE, s.sleep), Value::Int(s.sleep as i64)));
        }
        Ok(Some(g))
    }

    fn keymap(&mut self) -> Result<Option<Keymap>> {
        let Some(def) = self.def.clone() else { return Ok(None) };
        let (rows, cols) = (def.matrix.rows, def.matrix.cols);
        let total = self.layers as usize * rows as usize * cols as usize * 2;
        let mut bytes = Vec::with_capacity(total);
        while bytes.len() < total {
            let off = bytes.len() as u16;
            let n = (total - bytes.len()).min(28) as u8;
            let [hi, lo] = off.to_be_bytes();
            let r = self.raw.query(&[cmd::KEYMAP_GET_BUFFER, hi, lo, n])?;
            bytes.extend_from_slice(&r[4..4 + n as usize]);
        }
        let custom = def.custom_labels();
        let per_layer = rows as usize * cols as usize;
        let codes: Vec<Vec<u16>> = bytes
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect::<Vec<_>>()
            .chunks(per_layer)
            .map(<[u16]>::to_vec)
            .collect();
        let labels = codes.iter().map(|l| l.iter().map(|c| keycodes::label(*c, &custom)).collect()).collect();
        let layer_names = (0..self.layers as usize)
            .map(|i| match def.layers.get(i) {
                // Keychron repeats the OS name for its Fn layers ("Mac", "Mac", ...).
                Some(n) if i > 0 && def.layers.get(i - 1) == Some(n) => format!("{i}: {n} Fn"),
                Some(n) => format!("{i}: {n}"),
                None => format!("Layer {i}"),
            })
            .collect();
        Ok(Some(Keymap {
            rows,
            cols,
            layout: def
                .layouts
                .keys
                .iter()
                .map(|k| LayoutKey { row: k.row, col: k.col, x: k.x, y: k.y, w: k.w, h: k.h })
                .collect(),
            layer_names,
            codes,
            labels,
            catalog: keycodes::catalog(self.layers, self.macros, &custom)
                .into_iter()
                .map(|g| periphery_core::KeycodeGroup { label: g.label, codes: g.codes })
                .collect(),
            properties: self.key_properties(rows, cols)?,
            pair_lists: self.pair_lists(),
        }))
    }

    fn key_properties(&mut self, rows: u8, cols: u8) -> Result<Vec<KeyProperty>> {
        let n = rows as usize * cols as usize;
        let mut props = Vec::new();
        self.he_profile = None;
        if let Some(he) = self.he.as_ref().filter(|h| h.version == 4) {
            let p = keychron::he_read_profile(&self.raw, he, rows, cols)?;
            let per_key = |f: &dyn Fn(&Travel) -> i64| -> Vec<Option<Value>> {
                p.keys.iter().map(|k| (k.travel.mode != 0).then(|| Value::Int(f(&k.travel)))).collect()
            };
            let mm = |min: u8| Kind::range_unit(min as i64, keychron::HE_MAX as i64, 1, 0.1, " mm");
            let prop = |id: &str, label: &str, help: &str, kind: Kind, values| KeyProperty {
                id: id.into(),
                label: label.into(),
                help: Some(help.into()),
                kind,
                values,
                default_label: "global".into(),
                resettable: true,
            };
            props.push(prop(
                "he.key.actuation",
                "Actuation point",
                "How far each key travels before it registers. Keys showing “global” follow the setting on the Settings tab.",
                mm(keychron::HE_MIN),
                per_key(&|t| t.actuation as i64),
            ));
            props.push(prop(
                "he.key.mode",
                "Rapid trigger",
                "Fixed actuation or rapid trigger, per key.",
                Kind::choice([(keychron::HE_STATIC as i64, "Fixed"), (keychron::HE_RAPID_TRIGGER as i64, "Rapid")]),
                per_key(&|t| t.mode as i64),
            ));
            props.push(prop("he.key.rt_press", "RT press sensitivity", "Rapid-trigger press distance per key.", mm(1), per_key(&|t| t.rt_press as i64)));
            props.push(prop("he.key.rt_release", "RT release sensitivity", "Rapid-trigger release distance per key.", mm(1), per_key(&|t| t.rt_release as i64)));
            props.push(KeyProperty {
                id: "he.key.advanced".into(),
                label: "Advanced keys".into(),
                help: Some("Keys with DKS, toggle, turbo or similar functions (read-only here).".into()),
                kind: Kind::Info,
                values: p
                    .keys
                    .iter()
                    .map(|k| keychron::advanced_label(k.advanced).map(|l| Value::Text(l.into())))
                    .collect(),
                default_label: String::new(),
                resettable: false,
            });
            self.he_profile = Some(p);
        }
        if self.kc.as_ref().is_some_and(|k| k.features.keychron_rgb) {
            if self.led_map.is_none() {
                self.led_map = Some(keychron::led_map(&self.raw, rows, cols)?);
            }
            if let Some(count) = keychron::led_count(&self.raw)? {
                let hsv = keychron::led_colors(&self.raw, count)?;
                let map = self.led_map.as_ref().unwrap();
                props.push(KeyProperty {
                    id: "rgb.key.color".into(),
                    label: "Key colour".into(),
                    help: Some("Per-key colours show when the lighting effect is “Per Key RGB”.".into()),
                    kind: Kind::Color,
                    values: (0..n)
                        .map(|i| {
                            map.get(i).copied().flatten().and_then(|l| hsv.get(l as usize)).map(|c| Value::Color(menus::hsv_to_rgb(*c)))
                        })
                        .collect(),
                    default_label: String::new(),
                    resettable: false,
                });
            }
        }
        Ok(props)
    }

    fn pair_lists(&self) -> Vec<KeyPairList> {
        let Some(p) = &self.he_profile else { return Vec::new() };
        vec![KeyPairList {
            id: "he.snap".into(),
            label: "Snap tap (SOCD)".into(),
            help: Some("When both keys of a pair are held, decides which one counts — e.g. A/D for counter-strafing.".into()),
            max: keychron::SNAP_SLOTS.count(),
            modes: keychron::SNAP_MODES.iter().map(|(m, l)| (*m as i64, l.to_string())).collect(),
            pairs: p.snap.iter().map(|s| KeyPair { a: s.a, b: s.b, mode: s.mode as i64 }).collect(),
        }]
    }

    fn macros_group(&mut self) -> Result<Option<Group>> {
        if self.macros == 0 {
            return Ok(None);
        }
        let Some(r) = self.raw.try_query(&[cmd::MACRO_GET_BUFFER_SIZE])? else { return Ok(None) };
        self.macro_size = u16::from_be_bytes([r[1], r[2]]);
        let mut buf = Vec::with_capacity(self.macro_size as usize);
        while buf.len() < self.macro_size as usize {
            let n = (self.macro_size as usize - buf.len()).min(28) as u8;
            let [hi, lo] = (buf.len() as u16).to_be_bytes();
            let r = self.raw.query(&[cmd::MACRO_GET_BUFFER, hi, lo, n])?;
            buf.extend_from_slice(&r[4..4 + n as usize]);
        }
        self.macro_bytes = macros::split(&buf, self.macros as usize);
        let custom = self.def.as_ref().map(Definition::custom_labels).unwrap_or_default();
        let label = |c: u16| keycodes::label(c, &custom);
        let mut g = Group::new("Macros");
        for (i, m) in self.macro_bytes.iter().enumerate() {
            g.push(
                Setting::new(
                    format!("macro.{i}"),
                    format!("M{i}"),
                    Kind::Text { max_len: self.macro_size as usize },
                    Value::Text(macros::to_text(m, &label)),
                )
                .help(format!(
                    "Typed when a key mapped to M{i} is pressed. Plain text plus {{Enter}}, {{+LCtrl}} hold, {{-LCtrl}} release, {{250}} wait ms."
                )),
            );
        }
        Ok(Some(g))
    }

    fn set_macro(&mut self, index: usize, text: &str) -> Result<()> {
        if index >= self.macro_bytes.len() {
            bail!("no macro M{index}");
        }
        let custom = self.def.as_ref().map(Definition::custom_labels).unwrap_or_default();
        let code_for = |name: &str| {
            (0u16..=0xFF).find(|c| keycodes::label(*c, &custom).eq_ignore_ascii_case(name))
        };
        let mut all = self.macro_bytes.clone();
        all[index] = macros::from_text(text, &code_for)?;
        let buf = macros::join(&all, self.macro_size as usize)?;
        for (i, chunk) in buf.chunks(28).enumerate() {
            let [hi, lo] = ((i * 28) as u16).to_be_bytes();
            let mut req = vec![cmd::MACRO_SET_BUFFER, hi, lo, chunk.len() as u8];
            req.extend(chunk);
            self.raw.query(&req)?;
        }
        self.macro_bytes = all;
        Ok(())
    }

    fn set_key_travel(&mut self, field: &str, keys: &[(u8, u8)], v: i64) -> Result<()> {
        let (Some(he), Some(p), Some(def)) = (&self.he, &self.he_profile, &self.def) else {
            bail!("no magnetic-switch data");
        };
        let cols = def.matrix.cols as usize;
        let v = v.clamp(0, 63) as u8;
        let mut groups: std::collections::HashMap<Travel, Vec<(u8, u8)>> = Default::default();
        for &(row, col) in keys {
            let Some(k) = p.keys.get(row as usize * cols + col as usize) else { continue };
            let mut t = if k.travel.mode != 0 { k.travel } else { he.global };
            match field {
                "actuation" => t.actuation = v,
                "mode" => t.mode = v,
                "rt_press" => t.rt_press = v,
                "rt_release" => t.rt_release = v,
                _ => bail!("unknown per-key setting {field}"),
            }
            groups.entry(t).or_default().push((row, col));
        }
        for (t, keys) in groups {
            keychron::he_set_keys(&self.raw, he.current, &keys, t)?;
        }
        Ok(())
    }

    /// Puts keys back on the global travel setting (per-key mode 0, "Global"
    /// in the launcher's mode enum).
    fn reset_key_travel(&mut self, keys: &[(u8, u8)]) -> Result<()> {
        let Some(he) = &self.he else { bail!("no magnetic-switch data") };
        let t = Travel { mode: 0, ..he.global };
        keychron::he_set_keys(&self.raw, he.current, keys, t)
    }

    fn set_key_colors(&mut self, keys: &[(u8, u8)], rgb: [u8; 3]) -> Result<()> {
        let (Some(map), Some(def)) = (&self.led_map, &self.def) else { bail!("no per-key lighting") };
        let cols = def.matrix.cols as usize;
        let hsv = menus::rgb_to_hsv(rgb);
        for &(row, col) in keys {
            if let Some(Some(led)) = map.get(row as usize * cols + col as usize) {
                keychron::led_set(&self.raw, *led, &[hsv])?;
            }
        }
        keychron::led_save(&self.raw)
    }

    fn set_snap(&mut self, pairs: &[KeyPair]) -> Result<()> {
        let (Some(he), Some(p)) = (&self.he, &self.he_profile) else { bail!("no magnetic-switch data") };
        let profile = he.current;
        for pair in pairs {
            if pair.a == pair.b {
                bail!("a snap tap pair needs two different keys");
            }
            if !keychron::SNAP_MODES.iter().any(|(m, _)| *m as i64 == pair.mode) {
                bail!("unknown snap tap mode {}", pair.mode);
            }
        }
        let same = |s: &keychron::SnapPair, n: &KeyPair| s.a == n.a && s.b == n.b && s.mode as i64 == n.mode;
        let kept: Vec<&keychron::SnapPair> = p.snap.iter().filter(|s| pairs.iter().any(|n| same(s, n))).collect();
        for s in p.snap.iter().filter(|s| !kept.contains(s)) {
            keychron::he_set_snap(&self.raw, profile, s.slot, (0, 0), (0, 0), 0)?;
        }
        let mut free = keychron::SNAP_SLOTS.filter(|slot| !kept.iter().any(|k| k.slot == *slot));
        for n in pairs.iter().filter(|n| !kept.iter().any(|s| same(s, n))) {
            let Some(slot) = free.next() else { bail!("at most {} snap tap pairs", keychron::SNAP_SLOTS.count()) };
            keychron::he_set_snap(&self.raw, profile, slot, n.a, n.b, n.mode as u8)?;
        }
        keychron::he_save(&self.raw, profile)
    }

    fn set_he(&mut self, field: &str, v: i64) -> Result<()> {
        let Some(he) = self.he.as_mut() else { bail!("no magnetic-switch data") };
        if field == "profile" {
            keychron::he_select_profile(&self.raw, v as u8)?;
            he.current = v as u8;
            return Ok(());
        }
        let mut t: Travel = he.global;
        let v = v.clamp(0, 63) as u8;
        match field {
            "mode" => t.mode = v,
            "actuation" => t.actuation = v,
            "rt_press" => t.rt_press = v,
            "rt_release" => t.rt_release = v,
            _ => bail!("unknown setting he.{field}"),
        }
        keychron::he_set_global(&self.raw, he.current, t)?;
        he.global = t;
        Ok(())
    }

    fn set_sleep(&mut self, field: &str, v: i64) -> Result<()> {
        let Some(mut s) = self.sleep else { bail!("sleep timers not supported") };
        let v = v.clamp(0, u16::MAX as i64) as u16;
        match field {
            "backlight" => s.backlight = v,
            "magnet" => s.magnet_scan = v,
            "sleep" => s.sleep = v,
            _ => bail!("unknown setting sleep.{field}"),
        }
        if s.sleep > 0 && s.backlight >= s.sleep {
            bail!("the backlight must turn off before the keyboard sleeps");
        }
        if s.magnet_scan > 0 && (s.magnet_scan < s.backlight || (s.sleep > 0 && s.magnet_scan > s.sleep)) {
            bail!("switch scanning idle time must be between the backlight and sleep times");
        }
        keychron::sleep_set(&self.raw, s)?;
        self.sleep = Some(s);
        Ok(())
    }

    fn set_custom_value(&mut self, id: &str, value: &Value) -> Result<()> {
        let Some(item) = self.menu.iter().find(|m| m.id() == id) else { bail!("unknown setting {id}") };
        let mut req = vec![cmd::CUSTOM_SET_VALUE, item.channel, item.value_id];
        match item.kind {
            ItemKind::Color => {
                let (h, s) = menus::rgb_to_hs(value.color()?);
                req.extend([h, s]);
            }
            _ if item.width() == 2 => req.extend((value.int()?.clamp(0, 0xFFFF) as u16).to_be_bytes()),
            _ => req.push(value.int()?.clamp(0, 255) as u8),
        }
        self.raw.query(&req)?;
        self.raw.try_query(&[cmd::CUSTOM_SAVE, item.channel])?;
        Ok(())
    }
}

impl Device for ViaKeyboard {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        let mut out = Snapshot::default();
        if let Some(kc) = &self.kc {
            if let Some(fw) = &kc.firmware {
                out.status.push(("Firmware".into(), fw.clone()));
            }
            if let Some(t) = kc.transport {
                out.status.push(("Connection".into(), t.into()));
            }
        }
        out.status.push(("VIA protocol".into(), self.protocol.to_string()));
        out.status.push(("Layers".into(), self.layers.to_string()));
        if self.def.is_none() {
            let dir = definition::cache_dir();
            out.notice = Some(match &self.def_error {
                Some(e) => format!("Keymap unavailable: {e}"),
                None => format!(
                    "No layout definition for {:04x}:{:04x}. Put one in {} as {:04x}_{:04x}.json to edit the keymap.",
                    self.info.vid,
                    self.info.pid,
                    dir.display(),
                    self.info.vid,
                    self.info.pid
                ),
            });
        }
        if let Some(g) = self.he_group()? {
            out.groups.push(g);
        }
        out.groups.extend(self.lighting()?);
        if self.kc.is_some() {
            if let Some(g) = self.keyboard_group()? {
                out.groups.push(g);
            }
            if let Some(g) = self.power_group()? {
                out.groups.push(g);
            }
        }
        if let Some(g) = self.macros_group()? {
            out.groups.push(g);
        }
        out.keymap = self.keymap()?;
        Ok(out)
    }

    fn set(&mut self, id: &str, value: &Value) -> Result<()> {
        let (head, tail) = id.split_once('.').unwrap_or((id, ""));
        match head {
            "via" => self.set_custom_value(id, value),
            "he" => self.set_he(tail, value.int()?),
            "sleep" => self.set_sleep(tail, value.int()?),
            "debounce" => {
                let v = value.int()?.clamp(0, 255) as u8;
                let d = match (self.debounce, tail) {
                    (Some(Debounce::Qmk { ms, .. }), "mode") => Debounce::Qmk { mode: v, ms },
                    (Some(Debounce::Qmk { mode, .. }), "ms") => Debounce::Qmk { mode, ms: v },
                    (Some(Debounce::Telink { release, .. }), "press") => Debounce::Telink { press: v, release },
                    (Some(Debounce::Telink { press, .. }), "release") => Debounce::Telink { press, release: v },
                    _ => bail!("unknown setting {id}"),
                };
                keychron::debounce_set(&self.raw, d)?;
                self.debounce = Some(d);
                Ok(())
            }
            "poll" => keychron::poll_set(&self.raw, value.int()? as u32),
            "perf" => {
                let Some((version, _)) = self.perf else { bail!("performance mode not supported") };
                keychron::perf_set(&self.raw, version, value.int()? as u8)
            }
            "nkro" => keychron::nkro_set(&self.raw, value.bool()?),
            "dip" => keychron::dip_set(&self.raw, value.int()? as u8),
            "macro" => match value {
                Value::Text(t) => self.set_macro(tail.parse()?, t),
                _ => bail!("a macro needs text"),
            },
            _ => bail!("unknown setting {id}"),
        }
    }

    fn set_key_property(&mut self, id: &str, keys: &[(u8, u8)], value: Option<&Value>) -> Result<()> {
        let Some(value) = value else {
            if id.starts_with("he.key.") {
                return self.reset_key_travel(keys);
            }
            bail!("this setting can't be reset per key");
        };
        match id.strip_prefix("he.key.") {
            Some(field) => self.set_key_travel(field, keys, value.int()?),
            None if id == "rgb.key.color" => self.set_key_colors(keys, value.color()?),
            None => bail!("unknown per-key setting {id}"),
        }
    }

    fn set_key_pairs(&mut self, id: &str, pairs: &[KeyPair]) -> Result<()> {
        match id {
            "he.snap" => self.set_snap(pairs),
            _ => bail!("unknown pair list {id}"),
        }
    }

    fn set_key(&mut self, layer: u8, row: u8, col: u8, code: u16) -> Result<()> {
        let [hi, lo] = code.to_be_bytes();
        let r = self.raw.query(&[cmd::SET_KEYCODE, layer, row, col, hi, lo])?;
        if r[4] != hi || r[5] != lo {
            bail!("keyboard did not accept the keycode");
        }
        Ok(())
    }
}
