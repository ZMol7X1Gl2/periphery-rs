#![windows_subsystem = "windows"]

//! Periphery: one app for mice, keyboards and headsets from different brands.
//! The UI knows nothing about brands; it renders what each driver describes.
//! All HID traffic happens on a worker thread.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Ui};
use std::path::PathBuf;

use periphery_core::backup::{self, Backup};
use periphery_core::hidapi::HidApi;
use periphery_core::{
    Device, DeviceInfo, DeviceKind, KeyPair, KeyPairList, KeyProperty, Keymap, Kind, Setting, Snapshot, Value,
    discover_all,
};

const POLL_CHANGES: Duration = Duration::from_millis(250);
const REFRESH_NOTICE: Duration = Duration::from_secs(3);
const REFRESH_IDLE: Duration = Duration::from_secs(30);
const RED: Color32 = Color32::from_rgb(220, 70, 60);

enum Req {
    Discover,
    Select(usize),
    Refresh,
    Apply(usize, Vec<(String, Value)>),
    SetKey(usize, u8, u8, u8, u16),
    SetKeyProperty(usize, String, Vec<(u8, u8)>, Option<Value>),
    SetPairs(usize, String, Vec<KeyPair>),
    Backup(usize, PathBuf),
    /// Restore; `true` = dry run that only reports the plan.
    Restore(usize, PathBuf, bool),
}

enum Msg {
    Devices(Vec<DeviceInfo>, Vec<String>),
    Snapshot(usize, Box<Snapshot>),
    Error(String),
    Info(String),
    /// Dry-run result awaiting confirmation: path and planned changes.
    RestorePlan(PathBuf, Vec<String>),
    Done,
}

// ------------------------------------------------------------------ worker

struct Worker {
    api: Option<HidApi>,
    devices: Vec<Box<dyn Device>>,
    selected: Option<usize>,
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Worker {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }

    fn snapshot(&mut self, i: usize) {
        let Some(d) = self.devices.get_mut(i) else { return };
        let msg = match d.snapshot() {
            Ok(s) => Msg::Snapshot(i, Box::new(s)),
            Err(e) => Msg::Error(format!("{}: {e:#}", d.info().title())),
        };
        self.send(msg);
    }

    fn discover(&mut self) {
        self.devices.clear(); // release handles before reopening
        let api = match self.api.as_mut() {
            Some(api) => api.refresh_devices().map(|_| ()),
            None => HidApi::new().map(|a| self.api = Some(a)),
        };
        if let Err(e) = api {
            self.send(Msg::Devices(Vec::new(), vec![format!("HID: {e}")]));
            return;
        }
        let (devices, errors) = discover_all(self.api.as_ref().unwrap(), &periphery::drivers());
        self.devices = devices;
        let infos = self.devices.iter().map(|d| d.info().clone()).collect();
        self.send(Msg::Devices(infos, errors));
    }

    fn handle(&mut self, req: Req) {
        match req {
            Req::Discover => self.discover(),
            Req::Select(i) => {
                self.selected = Some(i);
                self.snapshot(i);
            }
            Req::Refresh => {
                if let Some(i) = self.selected {
                    self.snapshot(i);
                }
            }
            Req::Apply(i, changes) => {
                if let Some(d) = self.devices.get_mut(i) {
                    for (id, v) in &changes {
                        if let Err(e) = d.set(id, v) {
                            self.send(Msg::Error(format!("{e:#}")));
                            break;
                        }
                    }
                }
                self.snapshot(i);
            }
            Req::SetKeyProperty(i, id, keys, value) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key_property(&id, &keys, value.as_ref()) {
                        self.send(Msg::Error(format!("{e:#}")));
                    }
                }
                self.snapshot(i);
            }
            Req::Backup(i, path) => {
                let Some(d) = self.devices.get_mut(i) else { self.send(Msg::Done); return };
                let msg = match Backup::capture(d.as_mut()).and_then(|b| Ok(std::fs::write(&path, b.to_json())?)) {
                    Ok(()) => Msg::Info(format!("Backup saved to {}", path.display())),
                    Err(e) => Msg::Error(format!("Backup failed: {e:#}")),
                };
                self.send(msg);
            }
            Req::Restore(i, path, dry_run) => {
                let Some(d) = self.devices.get_mut(i) else { self.send(Msg::Done); return };
                let result = std::fs::read_to_string(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|t| Backup::from_json(&t))
                    .and_then(|b| backup::restore(d.as_mut(), &b, dry_run));
                let msg = match result {
                    Ok(r) if dry_run => {
                        let mut lines = r.applied;
                        lines.extend(r.skipped.into_iter().map(|s| format!("(skipped) {s}")));
                        Msg::RestorePlan(path, lines)
                    }
                    Ok(r) if r.errors.is_empty() => Msg::Info(format!("Restored {} value(s)", r.applied.len())),
                    Ok(r) => Msg::Error(format!("Restored {} value(s); failed: {}", r.applied.len(), r.errors.join("; "))),
                    Err(e) => Msg::Error(format!("Restore failed: {e:#}")),
                };
                self.send(msg);
                if !dry_run {
                    self.snapshot(i);
                }
            }
            Req::SetPairs(i, id, pairs) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key_pairs(&id, &pairs) {
                        self.send(Msg::Error(format!("{e:#}")));
                    }
                }
                self.snapshot(i);
            }
            Req::SetKey(i, layer, row, col, code) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key(layer, row, col, code) {
                        self.send(Msg::Error(format!("{e:#}")));
                        self.snapshot(i);
                    }
                }
            }
        }
        self.send(Msg::Done);
    }

    fn run(mut self, rx: Receiver<Req>) {
        loop {
            match rx.recv_timeout(POLL_CHANGES) {
                Ok(req) => self.handle(req),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    let Some(i) = self.selected else { continue };
                    let changed = self.devices.get_mut(i).map(|d| d.poll_changes());
                    if let Some(Ok(true)) = changed {
                        self.snapshot(i);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ app

struct App {
    tx: Sender<Req>,
    rx: Receiver<Msg>,
    devices: Vec<DeviceInfo>,
    discover_errors: Vec<String>,
    selected: Option<usize>,
    snap: Option<Snapshot>,
    error: Option<String>,
    busy: usize,
    last_refresh: Instant,
    /// Coalesced edits, flushed when the pointer is released.
    pending: Vec<(String, Value)>,
    confirm: Option<(String, String)>,
    info: Option<String>,
    restore_plan: Option<(PathBuf, Vec<String>)>,
    tab: Tab,
    layer: usize,
    selected_key: Option<(u8, u8)>,
    key_group: usize,
    km_mode: KmMode,
    /// Keys selected for a per-key property.
    multi: std::collections::BTreeSet<(u8, u8)>,
    prop_value: Option<Value>,
    pair_pick: Vec<(u8, u8)>,
    pair_mode: Option<i64>,
    /// From `--device <name>`: preselect the first device whose title contains it.
    want_device: Option<String>,
    want_keymap: bool,
    /// From `--tab keymap:<name>`: per-key setting or pair list to open.
    want_km: Option<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Settings,
    Keymap,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let (tx, worker_rx) = channel();
        let (worker_tx, rx) = channel();
        let worker = Worker { api: None, devices: Vec::new(), selected: None, tx: worker_tx, ctx: cc.egui_ctx.clone() };
        thread::spawn(move || worker.run(worker_rx));
        let mut app = App {
            tx,
            rx,
            devices: Vec::new(),
            discover_errors: Vec::new(),
            selected: None,
            snap: None,
            error: None,
            busy: 0,
            last_refresh: Instant::now(),
            pending: Vec::new(),
            confirm: None,
            info: None,
            restore_plan: None,
            tab: Tab::Settings,
            layer: 0,
            selected_key: None,
            key_group: 0,
            km_mode: KmMode::Keymap,
            multi: Default::default(),
            prop_value: None,
            pair_pick: Vec::new(),
            pair_mode: None,
            want_device: None,
            want_keymap: false,
            want_km: None,
        };
        let args: Vec<String> = std::env::args().collect();
        for w in args.windows(2) {
            match w[0].as_str() {
                "--device" => app.want_device = Some(w[1].to_lowercase()),
                "--tab" => {
                    app.want_keymap = w[1].starts_with("keymap");
                    app.want_km = w[1].strip_prefix("keymap:").map(str::to_lowercase);
                }
                _ => {}
            }
        }
        app.send(Req::Discover);
        app
    }

    fn send(&mut self, r: Req) {
        self.busy += 1;
        self.last_refresh = Instant::now();
        let _ = self.tx.send(r);
    }

    fn select(&mut self, i: usize) {
        self.selected = Some(i);
        self.snap = None;
        self.pending.clear();
        self.selected_key = None;
        self.layer = 0;
        self.km_mode = KmMode::Keymap;
        self.multi.clear();
        self.pair_pick.clear();
        self.prop_value = None;
        self.tab = Tab::Settings;
        self.send(Req::Select(i));
    }

    fn queue(&mut self, id: &str, v: Value) {
        self.pending.retain(|(p, _)| p != id);
        self.pending.push((id.to_string(), v));
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Devices(list, errors) => {
                    self.devices = list;
                    self.discover_errors = errors;
                    self.snap = None;
                    self.selected = None;
                    let wanted = self.want_device.as_ref().and_then(|w| {
                        self.devices.iter().position(|d| d.title().to_lowercase().contains(w))
                    });
                    if let Some(i) = wanted.or((!self.devices.is_empty()).then_some(0)) {
                        self.select(i);
                        if self.want_keymap {
                            self.tab = Tab::Keymap;
                        }
                    }
                }
                Msg::Snapshot(i, s) => {
                    if Some(i) == self.selected {
                        if let (Some(want), Some(km)) = (self.want_km.take(), s.keymap.as_ref()) {
                            let props = km.properties.iter().map(|p| &p.label).position(|l| l.to_lowercase().contains(&want));
                            let pairs = km.pair_lists.iter().map(|p| &p.label).position(|l| l.to_lowercase().contains(&want));
                            self.km_mode = match (props, pairs) {
                                (Some(p), _) => KmMode::Property(p),
                                (_, Some(p)) => KmMode::Pairs(p),
                                _ => KmMode::Keymap,
                            };
                        }
                        self.snap = Some(*s);
                    }
                }
                Msg::Error(e) => self.error = Some(e),
                Msg::Info(i) => self.info = Some(i),
                Msg::RestorePlan(p, lines) => self.restore_plan = Some((p, lines)),
                Msg::Done => self.busy = self.busy.saturating_sub(1),
            }
        }
    }
}

fn kind_icon(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => "🖱",
        DeviceKind::Keyboard => "⌨",
        DeviceKind::Headset => "🎧",
        DeviceKind::Other => "🔌",
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();

        let interacting = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        if !self.pending.is_empty() && !interacting {
            if let Some(i) = self.selected {
                let changes = std::mem::take(&mut self.pending);
                self.send(Req::Apply(i, changes));
            }
        }
        let interval = if self.snap.as_ref().is_some_and(|s| s.notice.is_some() && s.groups.len() < 2) {
            REFRESH_NOTICE
        } else {
            REFRESH_IDLE
        };
        if self.selected.is_some() && self.busy == 0 && !interacting && self.last_refresh.elapsed() >= interval {
            self.send(Req::Refresh);
        }
        ctx.request_repaint_after(Duration::from_millis(500));

        egui::SidePanel::left("devices").resizable(false).exact_width(220.0).show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Periphery");
            ui.add_space(8.0);
            let mut pick = None;
            for (i, d) in self.devices.iter().enumerate() {
                let text = format!("{}  {}", kind_icon(d.kind), d.title());
                if ui.selectable_label(self.selected == Some(i), text).clicked() {
                    pick = Some(i);
                }
            }
            if self.devices.is_empty() {
                ui.label(RichText::new("No supported devices found.").weak());
            }
            if let Some(i) = pick {
                self.select(i);
            }
            ui.add_space(8.0);
            if ui.button("⟳ Rescan").clicked() {
                self.send(Req::Discover);
            }
            for e in &self.discover_errors {
                ui.label(RichText::new(e).small().color(RED));
            }
        });

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            self.header(ui);
            ui.add_space(6.0);
        });

        self.confirm_window(ctx);
        self.restore_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(snap) = &self.snap else {
                if self.selected.is_some() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.spinner();
                    });
                }
                return;
            };
            if let Some(n) = &snap.notice {
                ui.label(RichText::new(n).color(Color32::from_rgb(220, 160, 40)));
                ui.add_space(6.0);
            }
            if snap.keymap.is_some() {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
                    ui.selectable_value(&mut self.tab, Tab::Keymap, "Keymap");
                });
                ui.separator();
            }
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Keymap if self.snap.as_ref().is_some_and(|s| s.keymap.is_some()) => self.keymap_view(ui),
                _ => self.settings_view(ui),
            });
        });
    }
}

// ------------------------------------------------------------------ header

impl App {
    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(d) = self.selected.and_then(|i| self.devices.get(i)) {
                ui.heading(d.title());
                ui.label(RichText::new(format!("{:04x}:{:04x}", d.vid, d.pid)).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dev = self.selected;
                if ui.add_enabled(dev.is_some(), egui::Button::new("Restore…")).on_hover_text("Load a backup; shows the changes before writing").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Periphery backup", &["json"]).pick_file() {
                        self.send(Req::Restore(dev.unwrap(), path, true));
                    }
                }
                if ui.add_enabled(dev.is_some(), egui::Button::new("Backup…")).on_hover_text("Save every setting, the keymap and per-key settings to a file").clicked() {
                    let name = self.selected.and_then(|i| self.devices.get(i)).map_or("device".to_string(), |d| d.title().replace(' ', "-"));
                    if let Some(path) = rfd::FileDialog::new().add_filter("Periphery backup", &["json"]).set_file_name(format!("{name}.json")).save_file() {
                        self.send(Req::Backup(dev.unwrap(), path));
                    }
                }
                if ui.button("⟳ Refresh").clicked() {
                    self.send(Req::Refresh);
                }
                if self.busy > 0 || !self.pending.is_empty() {
                    ui.spinner();
                }
            });
        });
        if let Some(s) = &self.snap {
            ui.horizontal_wrapped(|ui| {
                for (i, (k, v)) in s.status.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.label(RichText::new(format!("{k}:")).weak());
                    ui.label(v);
                }
            });
        }
        if let Some(i) = self.info.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(70, 170, 90), i);
                if ui.small_button("✕").clicked() {
                    self.info = None;
                }
            });
        }
        if let Some(e) = self.error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(RED, format!("⚠ {e}"));
                if ui.small_button("✕").clicked() {
                    self.error = None;
                }
            });
        }
    }

    fn restore_window(&mut self, ctx: &egui::Context) {
        let Some((path, lines)) = self.restore_plan.clone() else { return };
        let mut close = false;
        egui::Window::new("Restore backup").collapsible(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(RichText::new(path.display().to_string()).weak());
            if lines.is_empty() {
                ui.label("The device already matches this backup.");
                if ui.button("Close").clicked() {
                    close = true;
                }
                return;
            }
            ui.label(format!("{} change(s) will be written:", lines.len()));
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for l in &lines {
                    ui.label(RichText::new(l).monospace().size(12.0));
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Restore").clicked() {
                    if let Some(i) = self.selected {
                        self.send(Req::Restore(i, path.clone(), false));
                    }
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.restore_plan = None;
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some((id, text)) = self.confirm.clone() else { return };
        let mut close = false;
        egui::Window::new("Confirm").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(&text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Continue").clicked() {
                    if let Some(i) = self.selected {
                        self.send(Req::Apply(i, vec![(id.clone(), Value::Unit)]));
                    }
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.confirm = None;
        }
    }
}

// ------------------------------------------------------------------ settings

impl App {
    fn settings_view(&mut self, ui: &mut Ui) {
        let mut snap = self.snap.take().unwrap();
        let mut changes = Vec::new();
        let mut confirm = None;
        let groups = std::mem::take(&mut snap.groups);
        let mut groups = groups;
        ui.columns(2, |cols| {
            for (gi, g) in groups.iter_mut().enumerate() {
                let ui = &mut cols[gi % 2];
                egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&g.label).strong().size(15.0));
                    ui.add_space(6.0);
                    egui::Grid::new(("group", gi)).num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                        for s in &mut g.settings {
                            setting_row(ui, s, &mut changes, &mut confirm);
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
        snap.groups = groups;
        self.snap = Some(snap);
        for (id, v) in changes {
            self.queue(&id, v);
        }
        if confirm.is_some() {
            self.confirm = confirm;
        }
    }
}

fn setting_row(ui: &mut Ui, s: &mut Setting, changes: &mut Vec<(String, Value)>, confirm: &mut Option<(String, String)>) {
    let label = ui.label(&s.label);
    if let Some(h) = &s.help {
        label.on_hover_text(h);
    }
    let resp = match (&s.kind, &mut s.value) {
        (Kind::Toggle, Value::Bool(b)) => Some(ui.checkbox(b, "")),
        (Kind::Range { min, max, step, scale, unit }, Value::Int(v)) => {
            let (scale, unit) = (*scale, unit.clone());
            let slider = egui::Slider::new(v, *min..=*max).step_by(*step as f64).custom_formatter(move |n, _| {
                if scale == 1.0 { format!("{n}{unit}") } else { format!("{:.1}{unit}", n * scale) }
            });
            let slider = if scale != 1.0 {
                slider.custom_parser(move |t| t.trim().trim_end_matches(|c: char| c.is_alphabetic()).trim().parse::<f64>().ok().map(|x| (x / scale).round()))
            } else {
                slider
            };
            Some(ui.add(slider))
        }
        (Kind::Choice(opts), Value::Int(v)) => {
            let text = opts.iter().find(|(o, _)| o == v).map_or(v.to_string(), |(_, l)| l.clone());
            let mut changed = false;
            let r = egui::ComboBox::from_id_salt(&s.id).selected_text(text).width(180.0).show_ui(ui, |ui| {
                for (o, l) in opts {
                    if ui.selectable_value(v, *o, l).changed() {
                        changed = true;
                    }
                }
            });
            let mut r = r.response;
            if changed {
                r.mark_changed();
            }
            Some(r)
        }
        (Kind::Color, Value::Color(c)) => Some(ui.color_edit_button_srgb(c)),
        (Kind::Text { max_len }, Value::Text(t)) => {
            let r = ui.add(egui::TextEdit::singleline(t).char_limit(*max_len));
            if r.lost_focus() {
                changes.push((s.id.clone(), Value::Text(t.clone())));
            }
            None
        }
        (Kind::Action { confirm: c }, _) => {
            if ui.button(&s.label).clicked() {
                match c {
                    Some(text) => *confirm = Some((s.id.clone(), text.clone())),
                    None => changes.push((s.id.clone(), Value::Unit)),
                }
            }
            None
        }
        (_, Value::Text(t)) => {
            ui.label(t.as_str());
            None
        }
        (_, v) => {
            ui.label(format!("{v:?}"));
            None
        }
    };
    if let Some(r) = resp {
        let r = match &s.help {
            Some(h) => r.on_hover_text(h),
            None => r,
        };
        if r.changed() {
            changes.push((s.id.clone(), s.value.clone()));
        }
    }
    ui.end_row();
}

// ------------------------------------------------------------------ keymap

/// What the keyboard drawing edits.
#[derive(Clone, Copy, PartialEq)]
enum KmMode {
    Keymap,
    Property(usize),
    Pairs(usize),
}

fn fmt_value(kind: &Kind, v: &Value) -> String {
    match (kind, v) {
        (Kind::Range { scale, unit, .. }, Value::Int(x)) => {
            if *scale == 1.0 { format!("{x}{}", unit.trim()) } else { format!("{:.1}", *x as f64 * scale) }
        }
        (Kind::Choice(o), Value::Int(x)) => o.iter().find(|(a, _)| a == x).map_or(x.to_string(), |(_, l)| l.clone()),
        (_, Value::Text(t)) => t.clone(),
        (_, Value::Bool(b)) => (if *b { "on" } else { "off" }).into(),
        _ => String::new(),
    }
}

/// Editor widget for one value of `kind`; returns true when changed.
fn value_editor(ui: &mut Ui, id: &str, kind: &Kind, v: &mut Value) -> bool {
    match (kind, v) {
        (Kind::Range { min, max, step, scale, unit }, Value::Int(x)) => {
            let (scale, unit) = (*scale, unit.clone());
            ui.add(egui::Slider::new(x, *min..=*max).step_by(*step as f64).custom_formatter(move |n, _| {
                if scale == 1.0 { format!("{n}{unit}") } else { format!("{:.1}{unit}", n * scale) }
            }))
            .changed()
        }
        (Kind::Choice(opts), Value::Int(x)) => {
            let mut changed = false;
            let text = opts.iter().find(|(o, _)| o == x).map_or(x.to_string(), |(_, l)| l.clone());
            egui::ComboBox::from_id_salt(id).selected_text(text).show_ui(ui, |ui| {
                for (o, l) in opts {
                    changed |= ui.selectable_value(x, *o, l).changed();
                }
            });
            changed
        }
        (Kind::Color, Value::Color(c)) => ui.color_edit_button_srgb(c).changed(),
        (Kind::Toggle, Value::Bool(b)) => ui.checkbox(b, "").changed(),
        _ => false,
    }
}

fn default_value(kind: &Kind) -> Value {
    match kind {
        Kind::Range { min, max, .. } => Value::Int((min + max) / 2),
        Kind::Choice(o) => Value::Int(o.first().map_or(0, |(v, _)| *v)),
        Kind::Color => Value::Color([255, 255, 255]),
        Kind::Toggle => Value::Bool(false),
        _ => Value::Unit,
    }
}

impl App {
    fn keymap_view(&mut self, ui: &mut Ui) {
        let Some(km) = self.snap.as_ref().and_then(|s| s.keymap.clone()) else { return };
        let Some(dev) = self.selected else { return };

        ui.horizontal_wrapped(|ui| {
            ui.label("Edit:");
            let mut set = |ui: &mut Ui, m: KmMode, label: &str| {
                if ui.selectable_label(self.km_mode == m, label).clicked() && self.km_mode != m {
                    self.km_mode = m;
                    self.multi.clear();
                    self.pair_pick.clear();
                    self.prop_value = None;
                }
            };
            set(ui, KmMode::Keymap, "Keymap");
            for (i, p) in km.properties.iter().enumerate() {
                set(ui, KmMode::Property(i), &p.label);
            }
            for (i, l) in km.pair_lists.iter().enumerate() {
                set(ui, KmMode::Pairs(i), &l.label);
            }
        });
        if self.km_mode == KmMode::Keymap {
            ui.horizontal_wrapped(|ui| {
                ui.label("Layer:");
                for (i, n) in km.layer_names.iter().enumerate() {
                    if ui.selectable_label(self.layer == i, n).clicked() {
                        self.layer = i;
                    }
                }
            });
        }
        let prop = match self.km_mode {
            KmMode::Property(i) => km.properties.get(i),
            _ => None,
        };
        let pairs = match self.km_mode {
            KmMode::Pairs(i) => km.pair_lists.get(i),
            _ => None,
        };
        if let Some(h) = prop.and_then(|p| p.help.as_ref()).or(pairs.and_then(|p| p.help.as_ref())) {
            ui.label(RichText::new(h).weak());
        }
        ui.add_space(8.0);
        let layer = self.layer.min(km.codes.len().saturating_sub(1));
        let cols = km.cols as usize;

        // Draw the board scaled to the available width.
        let width = km.layout.iter().map(|k| k.x + k.w).fold(1.0, f32::max);
        let height = km.layout.iter().map(|k| k.y + k.h).fold(1.0, f32::max);
        let unit = (ui.available_width() / width).clamp(28.0, 64.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width * unit, height * unit), egui::Sense::hover());
        let mut clicked = None;
        for k in &km.layout {
            let pos = (k.row, k.col);
            let idx = k.row as usize * cols + k.col as usize;
            let keycap = km.labels.get(layer).and_then(|l| l.get(idx)).cloned().unwrap_or_default();
            let (label, fill, selected) = match self.km_mode {
                KmMode::Keymap => (keycap.clone(), None, self.selected_key == Some(pos)),
                KmMode::Property(_) => {
                    let p = prop.unwrap();
                    match p.values.get(idx).and_then(Option::as_ref) {
                        Some(Value::Color([r, g, b])) => (String::new(), Some(Color32::from_rgb(*r, *g, *b)), self.multi.contains(&pos)),
                        Some(v) => (fmt_value(&p.kind, v), None, self.multi.contains(&pos)),
                        None => (p.default_label.clone(), None, self.multi.contains(&pos)),
                    }
                }
                KmMode::Pairs(_) => {
                    let list = pairs.unwrap();
                    let n = list.pairs.iter().position(|p| p.a == pos || p.b == pos);
                    let text = match n {
                        Some(n) => format!("{keycap}\n#{}", n + 1),
                        None => keycap.clone(),
                    };
                    (text, None, self.pair_pick.contains(&pos) || n.is_some())
                }
            };
            let r = egui::Rect::from_min_size(
                rect.min + egui::vec2(k.x * unit + 2.0, k.y * unit + 2.0),
                egui::vec2(k.w * unit - 4.0, k.h * unit - 4.0),
            );
            let text = RichText::new(&label).size(if label.chars().count() > 5 { 10.0 } else { 13.0 });
            let mut button = egui::Button::new(text).wrap();
            if let Some(c) = fill {
                button = button.fill(c);
            }
            if selected {
                button = button.stroke(egui::Stroke::new(2.5_f32, ui.visuals().selection.stroke.color));
                if fill.is_none() {
                    button = button.fill(ui.visuals().selection.bg_fill);
                }
            }
            if ui.put(r, button).on_hover_text(format!("{keycap}  (row {} col {})", k.row, k.col)).clicked() {
                clicked = Some(pos);
            }
        }
        if let Some(pos) = clicked {
            match self.km_mode {
                KmMode::Keymap => self.selected_key = Some(pos),
                KmMode::Property(_) if !matches!(prop.map(|p| &p.kind), Some(Kind::Info)) => {
                    if !self.multi.remove(&pos) {
                        self.multi.insert(pos);
                    }
                    if self.prop_value.is_none() {
                        let p = prop.unwrap();
                        let idx = pos.0 as usize * cols + pos.1 as usize;
                        self.prop_value = Some(p.values.get(idx).cloned().flatten().unwrap_or_else(|| default_value(&p.kind)));
                    }
                }
                KmMode::Pairs(_) => {
                    if let Some(i) = self.pair_pick.iter().position(|p| *p == pos) {
                        self.pair_pick.remove(i);
                    } else if self.pair_pick.len() < 2 {
                        self.pair_pick.push(pos);
                    }
                }
                _ => {}
            }
        }
        ui.add_space(12.0);

        match self.km_mode {
            KmMode::Keymap => self.keycode_picker(ui, &km, dev, layer),
            KmMode::Property(_) => self.property_editor(ui, &km, prop.unwrap(), dev),
            KmMode::Pairs(_) => self.pair_editor(ui, &km, pairs.unwrap(), dev),
        }
    }

    fn keycode_picker(&mut self, ui: &mut Ui, km: &Keymap, dev: usize, layer: usize) {
        let Some((row, col)) = self.selected_key else {
            ui.label(RichText::new("Click a key, then pick what it should do.").weak());
            return;
        };
        let idx = row as usize * km.cols as usize + col as usize;
        let current = km.labels.get(layer).and_then(|l| l.get(idx)).cloned().unwrap_or_default();
        ui.label(RichText::new(format!("Key at row {row}, col {col} on {} — currently “{current}”", km.layer_names[layer])).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (i, g) in km.catalog.iter().enumerate() {
                if ui.selectable_label(self.key_group == i, &g.label).clicked() {
                    self.key_group = i;
                }
            }
        });
        ui.separator();
        let mut assign = None;
        if let Some(g) = km.catalog.get(self.key_group) {
            ui.horizontal_wrapped(|ui| {
                for (code, label) in &g.codes {
                    let text = if label.is_empty() { "(none)" } else { label.as_str() };
                    if ui.add(egui::Button::new(text).min_size(egui::vec2(48.0, 28.0))).on_hover_text(format!("{code:#06x}")).clicked() {
                        assign = Some((*code, label.clone()));
                    }
                }
            });
        }
        if let Some((code, label)) = assign {
            if let Some(k) = self.snap.as_mut().and_then(|s| s.keymap.as_mut()) {
                k.codes[layer][idx] = code;
                k.labels[layer][idx] = label;
            }
            self.send(Req::SetKey(dev, layer as u8, row, col, code));
        }
    }

    fn property_editor(&mut self, ui: &mut Ui, km: &Keymap, prop: &KeyProperty, dev: usize) {
        if matches!(prop.kind, Kind::Info) {
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} key(s) selected", self.multi.len())).strong());
            if ui.button("Select all").clicked() {
                self.multi = km.layout.iter().map(|k| (k.row, k.col)).collect();
            }
            if ui.button("Clear").clicked() {
                self.multi.clear();
            }
        });
        if self.multi.is_empty() {
            ui.label(RichText::new("Click keys to select them.").weak());
            return;
        }
        let mut value = self.prop_value.clone().unwrap_or_else(|| default_value(&prop.kind));
        let mut apply = false;
        ui.horizontal(|ui| {
            ui.label(&prop.label);
            value_editor(ui, &prop.id, &prop.kind, &mut value);
            apply = ui.button(format!("Apply to {} key(s)", self.multi.len())).clicked();
            if prop.resettable && ui.button(format!("Reset to {}", prop.default_label)).clicked() {
                let keys: Vec<(u8, u8)> = self.multi.iter().copied().collect();
                self.send(Req::SetKeyProperty(dev, prop.id.clone(), keys, None));
            }
        });
        self.prop_value = Some(value.clone());
        if apply {
            let keys: Vec<(u8, u8)> = self.multi.iter().copied().collect();
            self.send(Req::SetKeyProperty(dev, prop.id.clone(), keys, Some(value)));
        }
    }

    fn pair_editor(&mut self, ui: &mut Ui, km: &Keymap, list: &KeyPairList, dev: usize) {
        let name = |(r, c): (u8, u8)| km.labels[0].get(r as usize * km.cols as usize + c as usize).cloned().unwrap_or_default();
        let mut new_list = list.pairs.clone();
        let mut changed = false;
        let mut remove = None;
        for (i, p) in new_list.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("#{}  {} + {}", i + 1, name(p.a), name(p.b))).strong());
                let kind = Kind::Choice(list.modes.clone());
                let mut v = Value::Int(p.mode);
                if value_editor(ui, &format!("pairmode{i}"), &kind, &mut v) {
                    p.mode = v.as_int().unwrap_or(p.mode);
                    changed = true;
                }
                if ui.button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            new_list.remove(i);
            changed = true;
        }
        ui.separator();
        if new_list.len() < list.max {
            ui.horizontal(|ui| {
                let picked: Vec<String> = self.pair_pick.iter().map(|p| name(*p)).collect();
                ui.label(if picked.is_empty() {
                    "New pair: click two keys on the board.".to_string()
                } else {
                    format!("New pair: {}", picked.join(" + "))
                });
                if self.pair_pick.len() == 2 {
                    let kind = Kind::Choice(list.modes.clone());
                    let mut v = Value::Int(self.pair_mode.unwrap_or(list.modes[0].0));
                    value_editor(ui, "newpairmode", &kind, &mut v);
                    self.pair_mode = v.as_int();
                    if ui.button("Add pair").clicked() {
                        new_list.push(KeyPair { a: self.pair_pick[0], b: self.pair_pick[1], mode: v.as_int().unwrap_or(0) });
                        self.pair_pick.clear();
                        changed = true;
                    }
                }
            });
        } else {
            ui.label(RichText::new(format!("Maximum of {} pairs reached.", list.max)).weak());
        }
        if changed {
            self.send(Req::SetPairs(dev, list.id.clone(), new_list));
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 820.0]).with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("Periphery", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
