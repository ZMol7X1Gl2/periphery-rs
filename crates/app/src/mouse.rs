//! The mouse page. Every brand's DPI, polling rate, buttons, sensor and power
//! settings sit in the same place, whatever groups the driver put them in.
//! Drivers opt in by tagging settings with a [`Role`]; settings without one
//! are device-specific and keep the driver's grouping on a tab of their own.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText, Ui};
use periphery_core::{ButtonPos, Group, Kind, Role, Setting, Snapshot, Value};

use crate::mouse_diagram::{self, ButtonInfo};
use crate::theme::{self, Semibold, card, icon, size};
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
            MouseTab::Specific => format!("{brand}-specific"),
        }
    }
}

/// What the mouse page remembers between frames.
#[derive(Default)]
pub struct MouseState {
    pub tab: MouseTab,
    /// The button being edited on the Buttons tab.
    pub button: Option<ButtonPos>,
    pub picker: ActionPicker,
}

/// The action list of the selected button.
#[derive(Default)]
pub struct ActionPicker {
    /// `None` until the user picks one: then it follows the current binding.
    category: Option<Category>,
    filter: String,
}

/// Groups for button actions. Drivers only give a flat list of labels, so
/// the label decides the group; unknown wording lands in Other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Category {
    Mouse,
    Scroll,
    Dpi,
    Media,
    Keyboard,
    Macro,
    Other,
}

impl Category {
    const ALL: [Category; 7] =
        [Category::Mouse, Category::Scroll, Category::Dpi, Category::Media, Category::Keyboard, Category::Macro, Category::Other];

    fn of(label: &str) -> Category {
        let l = label.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
        if has(&["macro", "fire", "rapid"]) {
            Category::Macro
        } else if has(&["media", "volume", "play", "pause", "track", "mute", "stop"]) {
            Category::Media
        } else if has(&["dpi", "sniper", "sensitivity"]) {
            Category::Dpi
        } else if has(&["scroll", "wheel up", "wheel down", "tilt"]) {
            Category::Scroll
        } else if has(&["shortcut", "combo", "ctrl", "copy", "paste", "key"]) {
            Category::Keyboard
        } else if has(&["click", "back", "forward", "button"]) {
            Category::Mouse
        } else {
            Category::Other
        }
    }

    fn title(self) -> &'static str {
        match self {
            Category::Mouse => "Mouse",
            Category::Scroll => "Scroll",
            Category::Dpi => "DPI",
            Category::Media => "Media",
            Category::Keyboard => "Keyboard",
            Category::Macro => "Macro",
            Category::Other => "Other",
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

/// Draws the mouse page for `snap`; `state` is kept by the caller.
pub fn show(ui: &mut Ui, snap: &mut Snapshot, state: &mut MouseState, brand: &str, e: &mut Edits) {
    let tab = &mut state.tab;
    let groups = &mut snap.groups;
    profile_bar(ui, groups, e);

    let tabs: Vec<MouseTab> =
        MouseTab::ALL.into_iter().filter(|t| settings(groups).any(|s| MouseTab::of(s) == Some(*t))).collect();
    let Some(first) = tabs.first() else { return };
    // Keep the user's tab while it's missing (a sleeping mouse reports
    // little), so it comes back when the mouse wakes.
    let mut shown = if tabs.contains(tab) { *tab } else { *first };
    let items: Vec<(MouseTab, String)> = tabs.iter().map(|t| (*t, t.title(brand))).collect();
    if theme::segmented(ui, &mut shown, &items) {
        *tab = shown;
    }
    ui.add_space(10.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.set_max_width(MAX_WIDTH);
        match shown {
            MouseTab::Performance => performance(ui, groups, e),
            MouseTab::Buttons => buttons(ui, groups, &mut state.button, &mut state.picker, e),
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
            let label = ui.label(RichText::new(s.label.trim()).semibold());
            if let Some(h) = &s.help {
                label.on_hover_text(h);
            }
            setting_widget(ui, s, &mut e.changes, &mut e.confirm);
            ui.add_space(18.0);
        }
    });
    ui.add_space(4.0);
}

/// A label/editor grid of the settings whose role matches, in driver order.
fn rows(ui: &mut Ui, id: &str, groups: &mut [Group], e: &mut Edits, pick: impl Fn(Option<Role>) -> bool) {
    egui::Grid::new(id).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
        for s in settings_mut(groups).filter(|s| pick(s.role)) {
            setting_row(ui, s, &mut e.changes, &mut e.confirm);
        }
    });
}

/// The mouse drawing on the left, the selected button's actions on the right.
fn buttons(ui: &mut Ui, groups: &mut [Group], selected: &mut Option<ButtonPos>, picker: &mut ActionPicker, e: &mut Edits) {
    let infos: BTreeMap<ButtonPos, ButtonInfo> = settings(groups)
        .filter_map(|s| match s.role {
            Some(Role::Button(pos)) => Some((pos, ButtonInfo { name: s.label.trim().to_string(), binding: choice_label(s) })),
            _ => None,
        })
        .collect();
    // Start on the first button so the page always has something to edit.
    if selected.is_none_or(|b| !infos.contains_key(&b)) {
        *selected = infos.keys().next().copied();
    }
    let mut pick = None;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(290.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            card(ui, "", |ui| {
                ui.vertical_centered(|ui| pick = mouse_diagram::show(ui, &infos, *selected));
                ui.add_space(6.0);
                egui::Grid::new("button-list").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (pos, info) in &infos {
                        let on = *selected == Some(*pos);
                        let name = if on { RichText::new(&info.name) } else { RichText::new(&info.name).weak() };
                        if ui.selectable_label(on, name).clicked() | ui.selectable_label(on, &info.binding).clicked() {
                            pick = Some(*pos);
                        }
                        ui.end_row();
                    }
                });
            });
        });
        ui.vertical(|ui| {
            let Some(pos) = *selected else { return };
            card(ui, "", |ui| {
                if let Some(s) = with_role(groups, Role::Button(pos)) {
                    button_editor(ui, s, picker, e);
                }
                if settings(groups).any(|s| s.role == Some(Role::ButtonOption(pos))) {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Options").semibold());
                    ui.add_space(4.0);
                    rows(ui, "button-options", groups, e, |r| r == Some(Role::ButtonOption(pos)));
                }
            });
        });
    });
    if let Some(p) = pick {
        if *selected != Some(p) {
            *picker = ActionPicker::default();
        }
        *selected = Some(p);
    }
}

/// The label of a choice setting's current value.
fn choice_label(s: &Setting) -> String {
    match (&s.kind, &s.value) {
        (Kind::Choice(opts), Value::Int(v)) => opts.iter().find(|(o, _)| o == v).map_or_else(|| v.to_string(), |(_, l)| l.clone()),
        _ => String::new(),
    }
}

/// Every action the button can take, as buttons in category tabs. Typing
/// in the filter searches all categories.
fn button_editor(ui: &mut Ui, s: &mut Setting, picker: &mut ActionPicker, e: &mut Edits) {
    ui.label(RichText::new(s.label.trim()).size(size::HEADING).semibold());
    ui.label(RichText::new(format!("Currently: {}", choice_label(s))).weak());
    if let Some(h) = &s.help {
        ui.label(RichText::new(h).weak().small());
    }
    ui.add_space(6.0);
    let (Kind::Choice(opts), Value::Int(v)) = (&s.kind, &mut s.value) else {
        setting_widget(ui, s, &mut e.changes, &mut e.confirm);
        return;
    };
    let current = opts.iter().find(|(o, _)| o == v).map(|(_, l)| Category::of(l));
    let present: Vec<Category> = Category::ALL.into_iter().filter(|c| opts.iter().any(|(_, l)| Category::of(l) == *c)).collect();
    let mut shown = picker.category.or(current).filter(|c| present.contains(c)).or(present.first().copied());

    ui.horizontal_wrapped(|ui| {
        let searching = !picker.filter.is_empty();
        for c in &present {
            let mut text = RichText::new(c.title());
            if current == Some(*c) {
                text = text.semibold();
            }
            let tab = egui::SelectableLabel::new(!searching && shown == Some(*c), text);
            if ui.add(tab).on_hover_text(if current == Some(*c) { "Holds the current action" } else { "" }).clicked() {
                picker.category = Some(*c);
                picker.filter.clear();
                shown = Some(*c);
            }
        }
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut picker.filter).hint_text(format!("{}  Search all", icon::MAGNIFYING_GLASS)).desired_width(140.0));
    });
    ui.separator();

    let needle = picker.filter.to_lowercase();
    let visible = |l: &str| if needle.is_empty() { Some(Category::of(l)) == shown } else { l.to_lowercase().contains(&needle) };
    egui::ScrollArea::vertical().id_salt("button-actions").max_height(320.0).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (o, l) in opts.iter().filter(|(_, l)| visible(l)) {
                let button = theme::choice_button(l, *v == *o).min_size(egui::vec2(104.0, 30.0));
                if ui.add(button).clicked() && *v != *o {
                    *v = *o;
                    e.changes.push((s.id.clone(), Value::Int(*o)));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::Category;

    #[test]
    fn actions_from_every_driver_land_in_a_sensible_category() {
        let cases = [
            ("Left Click", Category::Mouse),
            ("Backward", Category::Mouse),
            ("Browser forward", Category::Mouse),
            ("Double click", Category::Mouse),
            ("Scroll Up", Category::Scroll),
            ("Scroll left", Category::Scroll),
            ("DPI +", Category::Dpi),
            ("DPI shift (hold)", Category::Dpi),
            ("Play / pause", Category::Media),
            ("Media key", Category::Media),
            ("Next track", Category::Media),
            ("Combo key", Category::Keyboard),
            ("Keyboard shortcut", Category::Keyboard),
            ("Copy (Ctrl+C)", Category::Keyboard),
            ("Fire button", Category::Macro),
            ("Macro", Category::Macro),
            ("Profile cycle", Category::Other),
            ("Polling Rate Switch", Category::Other),
            ("Disabled", Category::Other),
            ("Easy-Shift", Category::Other),
        ];
        for (label, want) in cases {
            assert_eq!(Category::of(label), want, "{label}");
        }
    }
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
                    let button = theme::choice_button(l, *v == *o).min_size(egui::vec2(72.0, 30.0));
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
                if ui.add_enabled(*n > *min, egui::Button::new(icon::MINUS)).clicked() {
                    *n -= 1;
                }
                ui.label(RichText::new(n.to_string()).size(size::SECTION).semibold());
                if ui.add_enabled(*n < *max, egui::Button::new(icon::PLUS)).clicked() {
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

    // Some mice only report the active stage (it changes with the DPI button).
    if let Some(s) = with_role(groups, Role::ActiveStage).filter(|s| matches!(s.kind, Kind::Info)) {
        egui::Grid::new("dpi-active").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
            setting_row(ui, s, &mut e.changes, &mut e.confirm);
        });
        ui.add_space(4.0);
    }
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
            let button = egui::Button::new(RichText::new(format!("{}…", s.label.trim())).color(egui::Color32::WHITE))
                .fill(theme::DANGER);
            if ui.add(button).clicked() {
                match &s.kind {
                    Kind::Action { confirm: Some(q) } => e.confirm = Some((s.id.clone(), q.clone())),
                    _ => e.changes.push((s.id.clone(), Value::Unit)),
                }
            }
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
