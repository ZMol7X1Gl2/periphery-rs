//! Command-line access to every supported device.

use anyhow::{Context, Result, bail};
use periphery_core::backup::{self, Backup};
use periphery::request;
use periphery_core::hid::Api;
use periphery_core::unknown::unknown_devices;
use periphery_core::{Device, Kind, Snapshot, Value, discover_all};

const USAGE: &str = "usage:
  periphery-cli list
  periphery-cli show <device>
  periphery-cli set <device> <setting-id> <value>     (value: number, on/off, #rrggbb)
  periphery-cli keymap <device> [layer]
  periphery-cli setkey <device> <layer> <row> <col> <keycode-hex>
  periphery-cli setprop <device> <property-id> <value|default> <row,col>...
  periphery-cli backup <device> <file.json>
  periphery-cli restore <device> <file.json> [--dry-run]
  periphery-cli diag                                   (read-only protocol details for bug reports)";

fn main() -> Result<()> {
    pollster::block_on(run())
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api = Api::new().await?;
    if args.first().map(String::as_str) == Some("diag") {
        return diag(&api).await;
    }
    let (mut devices, errors) = discover_all(&api, &periphery::drivers()).await;
    for e in &errors {
        let report = request::bug_report_url(None, e, "While scanning for devices\n", &format!("cli, {}", std::env::consts::OS));
        eprintln!("warning: {e}\n  report a bug: {report}");
    }
    let pick = |devices: &mut Vec<Box<dyn Device>>, s: Option<&String>| -> Result<usize> {
        let i: usize = s.context("missing device number")?.parse()?;
        if i == 0 || i > devices.len() {
            bail!("no device {i}; run `list`");
        }
        Ok(i - 1)
    };
    match args.first().map(String::as_str) {
        Some("list") | None => {
            for (i, d) in devices.iter().enumerate() {
                let info = d.info();
                println!("{}. {} [{:?}] {:04x}:{:04x}", i + 1, info.title(), info.kind, info.vid, info.pid);
            }
            if devices.is_empty() {
                println!("no supported devices found");
            }
            let unknown: Vec<_> = unknown_devices(&api, &periphery::drivers()).into_iter().filter(|u| u.is_configurable()).collect();
            if !unknown.is_empty() {
                println!("
not supported yet (request support with the link):");
                for u in &unknown {
                    println!("  {} {}
    {}", u.id(), u.name(), request::new_issue_url(u, &format!("cli, {}", std::env::consts::OS)));
                }
            }
        }
        Some("show") => {
            let i = pick(&mut devices, args.get(1))?;
            print_snapshot(&devices[i].snapshot().await?);
        }
        Some("set") => {
            let i = pick(&mut devices, args.get(1))?;
            let (Some(id), Some(raw)) = (args.get(2), args.get(3)) else { bail!(USAGE) };
            let snap = devices[i].snapshot().await?;
            let Some(setting) = snap.find(id) else { bail!("unknown setting {id}") };
            let value = parse_value(&setting.kind, raw)?;
            setting.check(&value)?;
            devices[i].set(id, &value).await?;
            println!("ok");
        }
        Some("keymap") => {
            let i = pick(&mut devices, args.get(1))?;
            let snap = devices[i].snapshot().await?;
            let Some(km) = snap.keymap else { bail!("this device has no keymap") };
            let layer: usize = args.get(2).map_or(Ok(0), |s| s.parse())?;
            let Some(labels) = km.labels.get(layer) else { bail!("no layer {layer}") };
            println!("{}", km.layer_names[layer]);
            for r in 0..km.rows as usize {
                let row: Vec<String> = (0..km.cols as usize)
                    .map(|c| format!("{:>7}", labels[r * km.cols as usize + c]))
                    .collect();
                println!("{}", row.join(""));
            }
        }
        Some("setkey") => {
            let i = pick(&mut devices, args.get(1))?;
            let nums: Vec<u8> = args[2..5.min(args.len())].iter().map(|s| s.parse()).collect::<Result<_, _>>()?;
            let [layer, row, col] = nums[..] else { bail!(USAGE) };
            let code = u16::from_str_radix(args.get(5).context(USAGE)?.trim_start_matches("0x"), 16)?;
            devices[i].set_key(layer, row, col, code).await?;
            println!("ok");
        }
        Some("setprop") => {
            let i = pick(&mut devices, args.get(1))?;
            let (Some(id), Some(raw)) = (args.get(2), args.get(3)) else { bail!(USAGE) };
            let keys: Vec<(u8, u8)> = args[4..]
                .iter()
                .map(|k| {
                    let (r, c) = k.split_once(',').context("keys are row,col")?;
                    Ok((r.parse()?, c.parse()?))
                })
                .collect::<Result<_>>()?;
            if keys.is_empty() {
                bail!("give at least one row,col");
            }
            let snap = devices[i].snapshot().await?;
            let Some(prop) = snap.keymap.as_ref().and_then(|k| k.properties.iter().find(|p| p.id == *id)) else {
                bail!("unknown per-key setting {id}");
            };
            let value = if raw == "default" { None } else { Some(parse_value(&prop.kind, raw)?) };
            devices[i].set_key_property(id, &keys, value.as_ref()).await?;
            println!("ok");
        }
        Some("backup") => {
            let i = pick(&mut devices, args.get(1))?;
            let file = args.get(2).context(USAGE)?;
            let b = Backup::capture(devices[i].as_mut()).await?;
            std::fs::write(file, b.to_json())?;
            println!("saved {} settings, {} keymap layers, {} per-key settings, {} pair lists to {file}", b.settings.len(), b.keymap.as_ref().map_or(0, Vec::len), b.key_properties.len(), b.pairs.len());
        }
        Some("restore") => {
            let i = pick(&mut devices, args.get(1))?;
            let file = args.get(2).context(USAGE)?;
            let dry = args.iter().any(|a| a == "--dry-run");
            let b = Backup::from_json(&std::fs::read_to_string(file)?)?;
            let r = backup::restore(devices[i].as_mut(), &b, dry).await?;
            let verb = if dry { "would change" } else { "changed" };
            println!("{verb} {} value(s)", r.applied.len());
            for a in &r.applied {
                println!("  {a}");
            }
            for s in &r.skipped {
                println!("  skipped: {s}");
            }
            for e in &r.errors {
                println!("  error: {e}");
            }
        }
        _ => bail!(USAGE),
    }
    Ok(())
}

fn parse_value(kind: &Kind, s: &str) -> Result<Value> {
    Ok(match kind {
        Kind::Toggle => Value::Bool(matches!(s, "on" | "1" | "true" | "yes")),
        Kind::Color => {
            let c = u32::from_str_radix(s.trim_start_matches('#'), 16)?;
            Value::Color([(c >> 16) as u8, (c >> 8) as u8, c as u8])
        }
        Kind::Text { .. } => Value::Text(s.into()),
        Kind::Action { .. } => Value::Unit,
        Kind::Range { scale, .. } if *scale != 1.0 => Value::Int((s.parse::<f64>()? / scale).round() as i64),
        _ => Value::Int(s.parse()?),
    })
}

fn print_snapshot(s: &Snapshot) {
    for (k, v) in &s.status {
        println!("{k:<20} {v}");
    }
    if let Some(n) = &s.notice {
        println!("\n! {n}");
    }
    for g in &s.groups {
        println!("\n[{}]", g.label);
        for st in &g.settings {
            let v = match (&st.kind, &st.value) {
                (Kind::Choice(opts), Value::Int(x)) => {
                    opts.iter().find(|(o, _)| o == x).map_or(x.to_string(), |(_, l)| l.clone())
                }
                (Kind::Range { scale, unit, .. }, Value::Int(x)) => {
                    if *scale == 1.0 { format!("{x}{unit}") } else { format!("{:.1}{unit}", *x as f64 * scale) }
                }
                (_, Value::Bool(b)) => (if *b { "on" } else { "off" }).into(),
                (_, Value::Color([r, g, b])) => format!("#{r:02x}{g:02x}{b:02x}"),
                (_, Value::Text(t)) => t.clone(),
                (Kind::Action { .. }, _) => "(action)".into(),
                (_, v) => format!("{v:?}"),
            };
            println!("  {:<28} {:<26} {}", st.label, v, st.id);
        }
    }
    if let Some(km) = &s.keymap {
        println!("\n[Keymap] {} layers, {}x{} matrix, {} keys", km.codes.len(), km.rows, km.cols, km.layout.len());
        let name = |r: u8, c: u8| km.labels[0][r as usize * km.cols as usize + c as usize].clone();
        for p in &km.properties {
            let set: Vec<String> = km
                .layout
                .iter()
                .filter_map(|k| {
                    let v = p.values.get(k.row as usize * km.cols as usize + k.col as usize)?.as_ref()?;
                    let v = match (&p.kind, v) {
                        (Kind::Range { scale, unit, .. }, Value::Int(x)) => format!("{:.1}{unit}", *x as f64 * scale),
                        (Kind::Choice(o), Value::Int(x)) => o.iter().find(|(a, _)| a == x).map_or(x.to_string(), |(_, l)| l.clone()),
                        (_, Value::Color([r, g, b])) => format!("#{r:02x}{g:02x}{b:02x}"),
                        (_, Value::Text(t)) => t.clone(),
                        (_, v) => format!("{v:?}"),
                    };
                    Some(format!("{}={v}", name(k.row, k.col)))
                })
                .collect();
            let shown = if set.len() > 12 { format!("{} … ({} keys)", set[..12].join(" "), set.len()) } else { set.join(" ") };
            println!("  {:<24} {}", p.label, if shown.is_empty() { format!("all {}", p.default_label) } else { shown });
        }
        for l in &km.pair_lists {
            let pairs: Vec<String> = l
                .pairs
                .iter()
                .map(|p| {
                    let m = l.modes.iter().find(|(m, _)| *m == p.mode).map_or("?", |(_, s)| s.as_str());
                    format!("{}+{} ({m})", name(p.a.0, p.a.1), name(p.b.0, p.b.1))
                })
                .collect();
            println!("  {:<24} {}", l.label, if pairs.is_empty() { "none".into() } else { pairs.join(", ") });
        }
    }
}

/// Read-only protocol details to paste into bug reports.
async fn diag(api: &Api) -> Result<()> {
    println!("periphery {}", env!("CARGO_PKG_VERSION"));
    for d in api.devices() {
        if matches!(d.vendor_id, 0x046D | 0x1E7D | 0x3151 | 0x3434 | 0x3554) {
            println!(
                "hid {:04x}:{:04x} if={} page={:04x} usage={:04x} {:?}",
                d.vendor_id, d.product_id, d.interface_number, d.usage_page, d.usage, d.product
            );
        }
    }
    for dev in logitech_hidpp::discover(api).await {
        let mut dev = match dev {
            Ok(d) => d,
            Err(e) => {
                println!("logitech: {e:#}");
                continue;
            }
        };
        println!("
logitech index {:#04x} on {:04x}, HID++ {}.{}", dev.index, dev.product_id, dev.protocol.0, dev.protocol.1);
        let show = |label: &str, r: anyhow::Result<String>| match r {
            Ok(v) => println!("  {label:<14} {v}"),
            Err(e) => println!("  {label:<14} ERROR {e:#}"),
        };
        show("name", dev.name().await);
        show("firmware", dev.firmware().await.map(|f| format!("{f:?}")));
        show("battery", dev.battery().await.map(|b| format!("{b:?}")));
        show("dpi range", dev.dpi_info().await.map(|d| format!("{d:?}")));
        show("dpi", dev.dpi().await.map(|d| d.to_string()));
        show("rates (Hz)", dev.report_rates().await.map(|r| format!("{r:?}")));
        show("rate (Hz)", dev.report_rate().await.map(|r| r.to_string()));
        show("profile ms", dev.report_intervals().await.map(|r| format!("{r:?}")));
        match dev.onboard_info().await {
            Ok(Some(info)) => {
                show("onboard", Ok(format!("{info:?}")));
                show("mode", dev.mode().await.map(|m| format!("{m:?}")));
                show("current", dev.current_profile().await.map(|p| format!("{p:#06x}")));
                show("directory", dev.profile_directory(&info).await.map(|d| format!("{d:x?}")));
            }
            Ok(None) => println!("  onboard        none"),
            Err(e) => println!("  onboard        ERROR {e:#}"),
        }
        match dev.feature_list().await {
            Ok(list) => {
                println!("  features:");
                for (i, id, flags, ver) in list {
                    println!("    [{i:2}] {id:#06x} flags={flags:#04x} v{ver}");
                }
            }
            Err(e) => println!("  features       ERROR {e:#}"),
        }
    }
    for line in periphery_rongyuan::diag(api).await.into_iter().chain(periphery_roccat::diag(api).await) {
        println!("{line}");
    }
    Ok(())
}
