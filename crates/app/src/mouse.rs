//! The mouse page. Every brand's DPI, polling rate, buttons, sensor and power
//! settings sit in the same place, whatever groups the driver put them in.
//! Drivers opt in by tagging settings with a [`Role`]; settings without one
//! are device-specific and keep the driver's grouping on a tab of their own.

use eframe::egui::{self, RichText, Ui};
use periphery_core::{Group, Kind, Role, Setting, Snapshot, Value};

use crate::ui::{setting_row, setting_widget};

/// Polling rates with at most this many choices show as a row of buttons.
const MAX_RATE_BUTTONS: usize = 8;
const MAX_WIDTH: f32 = 760.0;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MouseTab {
    #[default]
    Performance,
    Buttons,
    Sensor,
    Lighting,
    Device,
    Specific,
}

impl MouseTab {
    const ALL: [MouseTab; 6] =
        [MouseTab::Performance, MouseTab::Buttons, MouseTab::Sensor, MouseTab::Lighting, MouseTab::Device, MouseTab::Specific];

    /// The tab a setting shows on; `None` for the profile bar above the tabs.
    fn of(s: &Setting) -> Option<MouseTab> {
        let Some(role) = s.role else { return Some(MouseTab::Specific) };
        Some(match role {
            Role::Profile | Role::SettingsSource => return None,
            Role::PollingRate | Role::StageCount | Role::ActiveStage | Role::Dpi(_) | Role::StageColor(_) => MouseTab::Performance,
            Role::Button(_) | Role::ButtonOption(_) => MouseTab::Buttons,
            Role::LiftOff | Role::Debounce | Role::MotionSync | Role::AngleSnap | Role::Ripple => MouseTab::Sensor,
            Role::Lighting => MouseTab::Lighting,
            Role::Sleep | Role::Wireless | Role::FactoryReset => MouseTab::Device,
        })
    }

    /// For `--tab <name>`.
    pub fn from_name(name: &str) -> Option<MouseTab> {
        Some(match name.to_lowercase().as_str() {
            "dpi" => MouseTab::Performance,
            "buttons" => MouseTab::Buttons,
            "sensor" => MouseTab::Sensor,
            "lighting" => MouseTab::Lighting,
            "power" => MouseTab::Device,
            "specific" => MouseTab::Specific,
            _ => return None,
        })
    }

    fn title(self, brand: &str) -> String {
        match self {
            MouseTab::Performance => "DPI & polling".into(),
            MouseTab::Buttons => "Buttons".into(),
            MouseTab::Sensor => "Sensor".into(),
            MouseTab::Lighting => "Lighting".into(),
            MouseTab::Device => "Power & device".into(),
            MouseTab::Specific => format!("📁 {brand}-specific"),
        }
    }
}

/// What the user changed this frame.
#[derive(Default)]
pub struct Edits {
    pub changes: Vec<(String, Value)>,
    /// An action waiting for confirmation: id and question.
    pub confirm: Option<(String, String)>,
}

fn settings(groups: &[Group]) -> impl Iterator<Item = &Setting> {
    groups.iter().flat_map(|g| &g.settings)
}

fn settings_mut(groups: &mut [Group]) -> impl Iterator<Item = &mut Setting> {
    groups.iter_mut().flat_map(|g| &mut g.settings)
}

fn with_role(groups: &mut [Group], role: Role) -> Option<&mut Setting> {
    settings_mut(groups).find(|s| s.role == Some(role))
}

/// Draws the mouse page for `snap`; `tab` is kept by the caller.
pub fn show(ui: &mut Ui, snap: &mut Snapshot, tab: &mut MouseTab, brand: &str, e: &mut Edits) {
    let groups = &mut snap.groups;
    profile_bar(ui, groups, e);

    let tabs: Vec<MouseTab> =
        MouseTab::ALL.into_iter().filter(|t| settings(groups).any(|s| MouseTab::of(s) == Some(*t))).collect();
    let Some(first) = tabs.first() else { return };
    // Keep the user's tab while it's missing (a sleeping mouse reports
    // little), so it comes back when the mouse wakes.
    let mut shown = if tabs.contains(tab) { *tab } else { *first };
    ui.horizontal_wrapped(|ui| {
        for t in &tabs {
            let label = RichText::new(t.title(brand)).size(15.0);
            if ui.add(egui::SelectableLabel::new(shown == *t, label)).clicked() {
                shown = *t;
                *tab = *t;
            }
        }
    });
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.set_max_width(MAX_WIDTH);
        match shown {
            MouseTab::Performance => performance(ui, groups, e),
            MouseTab::Buttons => card(ui, "Buttons", |ui| {
                rows(ui, "buttons", groups, e, |r| matches!(r, Some(Role::Button(_) | Role::ButtonOption(_))));
            }),
            MouseTab::Sensor => card(ui, "Sensor", |ui| {
                const ORDER: [Role; 5] = [Role::LiftOff, Role::MotionSync, Role::AngleSnap, Role::Ripple, Role::Debounce];
                egui::Grid::new("sensor").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    for role in ORDER {
                        if let Some(s) = with_role(groups, role) {
                            setting_row(ui, s, &mut e.changes, &mut e.confirm);
                        }
                    }
                });
            }),
            MouseTab::Lighting => card(ui, "Lighting", |ui| {
                rows(ui, "lighting", groups, e, |r| r == Some(Role::Lighting));
            }),
            MouseTab::Device => device(ui, groups, e),
            MouseTab::Specific => specific(ui, groups, brand, e),
        }
    });
}

/// Profile and settings source sit above the tabs: they decide what every
/// tab below is editing.
fn profile_bar(ui: &mut Ui, groups: &mut [Group], e: &mut Edits) {
    let roles = [Role::SettingsSource, Role::Profile];
    if !roles.iter().any(|r| settings(groups).any(|s| s.role == Some(*r))) {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for role in roles {
            let Some(s) = with_role(groups, role) else { continue };
            let label = ui.label(RichText::new(s.label.trim()).strong());
            if let Some(h) = &s.help {
                label.on_hover_text(h);
            }
            setting_widget(ui, s, &mut e.changes, &mut e.confirm);
            ui.add_space(18.0);
        }
    });
    ui.add_space(4.0);
}

fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style()).inner_margin(12.0).rounding(6.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(10.0);
}

/// A label/editor grid of the settings whose role matches, in driver order.
fn rows(ui: &mut Ui, id: &str, groups: &mut [Group], e: &mut Edits, pick: impl Fn(Option<Role>) -> bool) {
    egui::Grid::new(id).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        for s in settings_mut(groups).filter(|s| pick(s.role)) {
            setting_row(ui, s, &mut e.changes, &mut e.confirm);
        }
    });
}

fn performance(ui: &mut Ui, groups: &mut [Group], e: &mut Edits) {
    if let Some(s) = with_role(groups, Role::PollingRate) {
        card(ui, "Polling rate", |ui| polling_rate(ui, s, e));
    }
    let has_dpi = settings(groups).any(|s| matches!(s.role, Some(Role::Dpi(_) | Role::StageCount | Role::ActiveStage)));
    if has_dpi {
        card(ui, "DPI", |ui| dpi(ui, groups, e));
    }
}

fn polling_rate(ui: &mut Ui, s: &mut Setting, e: &mut Edits) {
    match (&s.kind, &mut s.value) {
        (Kind::Choice(opts), Value::Int(v)) if opts.len() <= MAX_RATE_BUTTONS => {
            ui.horizontal_wrapped(|ui| {
                for (o, l) in opts {
                    let button = egui::Button::new(RichText::new(l).size(14.0)).selected(*v == *o).min_size(egui::vec2(72.0, 30.0));
                    if ui.add(button).clicked() && *v != *o {
                        *v = *o;
                        e.changes.push((s.id.clone(), Value::Int(*o)));
                    }
                }
            });
        }
        _ => setting_widget(ui, s, &mut e.changes, &mut e.confirm),
    }
    if let Some(h) = &s.help {
        ui.label(RichText::new(h).weak());
    }
}

/// Stage count, then one row per stage: active marker, DPI and colour.
fn dpi(ui: &mut Ui, groups: &mut [Group], e: &mut Edits) {
    if let Some(s) = with_role(groups, Role::StageCount) {
        if let (Kind::Range { min, max, .. }, Value::Int(n)) = (&s.kind, &mut s.value) {
            ui.horizontal(|ui| {
                ui.label(s.label.trim());
                let before = *n;
                if ui.add_enabled(*n > *min, egui::Button::new(" − ")).clicked() {
                    *n -= 1;
                }
                ui.label(RichText::new(n.to_string()).strong().monospace().size(15.0));
                if ui.add_enabled(*n < *max, egui::Button::new(" + ")).clicked() {
                    *n += 1;
                }
                if *n != before {
                    e.changes.push((s.id.clone(), Value::Int(*n)));
                }
            });
            ui.add_space(6.0);
        }
    }

    // (id, current, stages it can select, help)
    let active = with_role(groups, Role::ActiveStage).and_then(|s| match (&s.kind, &s.value) {
        (Kind::Choice(o), Value::Int(v)) => Some((s.id.clone(), *v, o.iter().map(|(x, _)| *x).collect::<Vec<_>>(), s.help.clone())),
        _ => None,
    });
    let mut stages: Vec<u8> = settings(groups).filter_map(|s| if let Some(Role::Dpi(n)) = s.role { Some(n) } else { None }).collect();
    stages.sort_unstable();
    stages.dedup();
    let single = stages.len() == 1 && active.is_none();
    let mut picked = None;

    egui::Grid::new("dpi").num_columns(4).spacing([10.0, 10.0]).show(ui, |ui| {
        for n in stages {
            if let Some((id, current, valid, help)) = &active {
                if valid.contains(&(n as i64)) {
                    let hover = help.clone().unwrap_or_else(|| "Make this the active stage".into());
                    if ui.radio(*current == n as i64, "").on_hover_text(hover).clicked() && *current != n as i64 {
                        e.changes.push((id.clone(), Value::Int(n as i64)));
                        picked = Some(n as i64);
                    }
                } else {
                    ui.label("");
                }
            }
            ui.label(if single { "DPI".to_string() } else { format!("Stage {}", n + 1) });
            if let Some(s) = with_role(groups, Role::Dpi(n)) {
                ui.spacing_mut().slider_width = 320.0;
                setting_widget(ui, s, &mut e.changes, &mut e.confirm);
            }
            match with_role(groups, Role::StageColor(n)) {
                Some(s) => setting_widget(ui, s, &mut e.changes, &mut e.confirm),
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
    // Show the new active stage before the device confirms it.
    if let (Some(p), Some(s)) = (picked, with_role(groups, Role::ActiveStage)) {
        s.value = Value::Int(p);
    }
}

fn device(ui: &mut Ui, groups: &mut [Group], e: &mut Edits) {
    if settings(groups).any(|s| matches!(s.role, Some(Role::Sleep | Role::Wireless))) {
        card(ui, "Power & wireless", |ui| {
            rows(ui, "power", groups, e, |r| matches!(r, Some(Role::Sleep | Role::Wireless)));
        });
    }
    if let Some(s) = with_role(groups, Role::FactoryReset) {
        card(ui, "Reset", |ui| {
            ui.label(RichText::new("Puts every setting on the mouse back to how it left the factory.").weak());
            ui.add_space(4.0);
            setting_widget(ui, s, &mut e.changes, &mut e.confirm);
        });
    }
}

/// Settings only this brand has, in the driver's own groups.
fn specific(ui: &mut Ui, groups: &mut [Group], brand: &str, e: &mut Edits) {
    ui.label(RichText::new(format!("Settings only {brand} mice have, grouped the way the driver reports them.")).weak());
    ui.add_space(6.0);
    for (gi, g) in groups.iter_mut().enumerate() {
        if !g.settings.iter().any(|s| s.role.is_none()) {
            continue;
        }
        card(ui, &g.label, |ui| {
            egui::Grid::new(("specific", gi)).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                for s in g.settings.iter_mut().filter(|s| s.role.is_none()) {
                    setting_row(ui, s, &mut e.changes, &mut e.confirm);
                }
            });
        });
    }
}
