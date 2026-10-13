//! Device-independent backups: every writable value a device reports, saved
//! as JSON. Restoring compares the backup with the device and writes only the
//! values that differ, so untouched settings are never rewritten.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{Device, KeyPair, Kind, Snapshot, Value};

const FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub format: u32,
    pub brand: String,
    pub model: String,
    pub vid: u16,
    pub pid: u16,
    pub settings: Vec<(String, Value)>,
    /// `keymap[layer][row * cols + col]`
    pub keymap: Option<Vec<Vec<u16>>>,
    pub cols: u8,
    pub key_properties: Vec<(String, Vec<Option<Value>>)>,
    pub pairs: Vec<(String, Vec<KeyPair>)>,
}

fn writable(kind: &Kind) -> bool {
    !matches!(kind, Kind::Info | Kind::Action { .. })
}

/// Profile selectors are restored first: other values belong to a profile.
fn is_profile(id: &str) -> bool {
    id == "profile" || id.ends_with(".profile")
}

impl Backup {
    pub fn capture(dev: &mut dyn Device) -> Result<Backup> {
        let snap = dev.snapshot()?;
        let settings: Vec<(String, Value)> = snap
            .groups
            .iter()
            .flat_map(|g| &g.settings)
            .filter(|s| writable(&s.kind))
            .map(|s| (s.id.clone(), s.value.clone()))
            .collect();
        if settings.is_empty() && snap.keymap.is_none() {
            bail!("{}", snap.notice.unwrap_or_else(|| "this device has nothing to back up".into()));
        }
        let info = dev.info();
        let km = snap.keymap.as_ref();
        Ok(Backup {
            format: FORMAT,
            brand: info.brand.clone(),
            model: info.model.clone(),
            vid: info.vid,
            pid: info.pid,
            settings,
            keymap: km.map(|k| k.codes.clone()),
            cols: km.map_or(0, |k| k.cols),
            key_properties: km
                .map(|k| {
                    k.properties
                        .iter()
                        .filter(|p| writable(&p.kind))
                        .map(|p| (p.id.clone(), p.values.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            pairs: km
                .map(|k| k.pair_lists.iter().map(|l| (l.id.clone(), l.pairs.clone())).collect())
                .unwrap_or_default(),
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("backup serializes")
    }

    pub fn from_json(text: &str) -> Result<Backup> {
        let b: Backup = serde_json::from_str(text).context("not a Periphery backup")?;
        if b.format != FORMAT {
            bail!("unsupported backup format {}", b.format);
        }
        Ok(b)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Setting(String, Value),
    Key { layer: u8, row: u8, col: u8, code: u16 },
    KeyProperty { id: String, keys: Vec<(u8, u8)>, value: Value },
    /// Put keys back on the property's default (e.g. global actuation).
    ResetKeyProperty { id: String, keys: Vec<(u8, u8)> },
    Pairs(String, Vec<KeyPair>),
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Setting(id, v) => write!(f, "set {id} = {v:?}"),
            Step::Key { layer, row, col, code } => write!(f, "key layer {layer} row {row} col {col} = {code:#06x}"),
            Step::KeyProperty { id, keys, value } => write!(f, "{id} = {value:?} for {} key(s) {keys:?}", keys.len()),
            Step::ResetKeyProperty { id, keys } => write!(f, "{id} = default for {} key(s) {keys:?}", keys.len()),
            Step::Pairs(id, p) => write!(f, "{id} = {} pair(s)", p.len()),
        }
    }
}

#[derive(Debug, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Differences that can't be restored (e.g. per-key values that can't be
    /// reset to the global default).
    pub skipped: Vec<String>,
}

/// Compares a backup with the device's current state.
pub fn plan(backup: &Backup, snap: &Snapshot) -> Plan {
    let mut out = Plan::default();
    for (id, v) in &backup.settings {
        match snap.find(id) {
            Some(s) if s.value == *v || !writable(&s.kind) => {}
            // Missing settings may appear once an earlier step applies (e.g.
            // a button switched to a combo shows its key field), so try them.
            _ => out.steps.push(Step::Setting(id.clone(), v.clone())),
        }
    }
    let Some(km) = &snap.keymap else { return out };
    let cols = km.cols as usize;
    let pos = |idx: usize| ((idx / cols) as u8, (idx % cols) as u8);
    if let Some(layers) = &backup.keymap {
        for (layer, codes) in layers.iter().enumerate() {
            for (idx, code) in codes.iter().enumerate() {
                let current = km.codes.get(layer).and_then(|l| l.get(idx));
                if current.is_some_and(|c| c != code) {
                    let (row, col) = pos(idx);
                    out.steps.push(Step::Key { layer: layer as u8, row, col, code: *code });
                }
            }
        }
    }
    for (id, values) in &backup.key_properties {
        let Some(prop) = km.properties.iter().find(|p| p.id == *id) else { continue };
        // Group keys by target value so each value is one write.
        let mut groups: BTreeMap<String, (Value, Vec<(u8, u8)>)> = BTreeMap::new();
        let mut reset = Vec::new();
        for (idx, want) in values.iter().enumerate() {
            let current = prop.values.get(idx).cloned().flatten();
            if *want == current {
                continue;
            }
            let (row, col) = pos(idx);
            match want {
                Some(v) => groups.entry(format!("{v:?}")).or_insert_with(|| (v.clone(), Vec::new())).1.push((row, col)),
                None if prop.resettable => reset.push((row, col)),
                None => out.skipped.push(format!("{id}: key row {row} col {col} can't be reset to {}", prop.default_label)),
            }
        }
        if !reset.is_empty() {
            out.steps.push(Step::ResetKeyProperty { id: id.clone(), keys: reset });
        }
        for (_, (value, keys)) in groups {
            out.steps.push(Step::KeyProperty { id: id.clone(), keys, value });
        }
    }
    for (id, pairs) in &backup.pairs {
        if km.pair_lists.iter().any(|l| l.id == *id && l.pairs != *pairs) {
            out.steps.push(Step::Pairs(id.clone(), pairs.clone()));
        }
    }
    out
}

fn apply(dev: &mut dyn Device, step: &Step) -> Result<()> {
    match step {
        Step::Setting(id, v) => dev.set(id, v),
        Step::Key { layer, row, col, code } => dev.set_key(*layer, *row, *col, *code),
        Step::KeyProperty { id, keys, value } => dev.set_key_property(id, keys, Some(value)),
        Step::ResetKeyProperty { id, keys } => dev.set_key_property(id, keys, None),
        Step::Pairs(id, p) => dev.set_key_pairs(id, p),
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

/// Restores `backup`. With `dry_run`, only reports what would change.
pub fn restore(dev: &mut dyn Device, backup: &Backup, dry_run: bool) -> Result<Report> {
    let info = dev.info();
    if (info.vid, info.pid) != (backup.vid, backup.pid) {
        bail!("this backup is for {} {} ({:04x}:{:04x})", backup.brand, backup.model, backup.vid, backup.pid);
    }
    let mut report = Report::default();
    let mut snap = dev.snapshot()?;

    // Switch to the backed-up profile first; the rest belongs to it.
    for (id, v) in backup.settings.iter().filter(|(id, _)| is_profile(id)) {
        if snap.find(id).is_some_and(|s| s.value != *v) {
            let step = Step::Setting(id.clone(), v.clone());
            report.applied.push(step.to_string());
            if !dry_run {
                apply(dev, &step)?;
                snap = dev.snapshot()?;
            }
        }
    }

    let plan = plan(backup, &snap);
    report.skipped = plan.skipped;
    let steps: Vec<Step> = plan.steps.into_iter().filter(|s| !matches!(s, Step::Setting(id, _) if is_profile(id))).collect();
    if dry_run {
        report.applied.extend(steps.iter().map(Step::to_string));
        return Ok(report);
    }
    // Settings can depend on each other (e.g. sleep timer ordering), so retry
    // failures once after everything else has been applied.
    let mut failed = Vec::new();
    for step in steps {
        match apply(dev, &step) {
            Ok(()) => report.applied.push(step.to_string()),
            Err(e) => failed.push((step, e)),
        }
    }
    for (step, _) in std::mem::take(&mut failed) {
        match apply(dev, &step) {
            Ok(()) => report.applied.push(step.to_string()),
            Err(e) => report.errors.push(format!("{step}: {e:#}")),
        }
    }
    Ok(report)
}
