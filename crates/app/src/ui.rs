//! The Periphery window, shared by the desktop app and the web app. The UI
//! knows nothing about brands; it renders what each driver describes. All HID
//! traffic happens in the [`Worker`](crate::worker::Worker).

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Ui};
use periphery_core::hid::Instant;
use periphery_core::unknown::UnknownDevice;
use periphery_core::{DeviceInfo, DeviceKind, KeyPair, KeyPairList, KeyProperty, Keymap, Kind, Setting, Snapshot, Value};

use crate::mouse::{self, MouseState, MouseTab};
use crate::platform::{self, ReqSender};
use crate::request::{self, Existing};
use crate::theme::{self, Semibold, icon, radius, size};
use crate::worker::{Msg, Req};

const REFRESH_NOTICE: Duration = Duration::from_secs(3);
const REFRESH_IDLE: Duration = Duration::from_secs(30);

/// What to show first (desktop: `--device <name>`, `--tab keymap[:<setting>]`
/// or a mouse tab: `--tab buttons`), and `--test-devices`.
#[derive(Clone, Debug, Default)]
pub struct StartOptions {
    /// Preselect the first device whose title contains this (lowercase).
    pub device: Option<String>,
    pub keymap: bool,
    /// Per-key setting or pair list to open on the keymap tab (lowercase).
    pub km: Option<String>,
    pub mouse_tab: Option<MouseTab>,
    /// Add a made-up unsupported device and a failed one to every scan, to
    /// try Request support and Report a bug without such hardware.
    pub test_devices: bool,
}

impl StartOptions {
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let args: Vec<String> = args.into_iter().collect();
        let mut o = StartOptions { test_devices: args.iter().any(|a| a == "--test-devices"), ..Default::default() };
        for w in args.windows(2) {
            match w[0].as_str() {
                "--device" => o.device = Some(w[1].to_lowercase()),
                "--tab" => {
                    o.keymap = w[1].starts_with("keymap");
                    o.km = w[1].strip_prefix("keymap:").map(str::to_lowercase);
                    o.mouse_tab = MouseTab::from_name(&w[1]);
                }
                _ => {}
            }
        }
        o
    }
}

pub struct App {
    tx: ReqSender,
    rx: Receiver<Msg>,
    /// Lets platform code (file pickers) post messages to the UI.
    local: Sender<Msg>,
    devices: Vec<DeviceInfo>,
    discover_errors: Vec<String>,
    /// Devices no driver handles, offered for a support request.
    unknown: Vec<UnknownDevice>,
    /// Support-request lookups by device id.
    lookups: std::collections::HashMap<String, Lookup>,
    selected: Option<usize>,
    snap: Option<Snapshot>,
    error: Option<String>,
    busy: usize,
    last_refresh: Instant,
    /// Coalesced edits, flushed when the pointer is released.
    pending: Vec<(String, Value)>,
    confirm: Option<(String, String)>,
    info: Option<String>,
    /// Dry-run result: source, backup JSON, planned changes.
    restore_plan: Option<(String, String, Vec<String>)>,
    tab: Tab,
    mouse: MouseState,
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
    want_mouse_tab: Option<MouseTab>,
    test_devices: bool,
}

/// A made-up device no driver handles, for `--test-devices`.
fn test_unknown() -> UnknownDevice {
    UnknownDevice {
        vid: 0xffff,
        pid: 0x0001,
        product: "Periphery test device (not real)".into(),
        manufacturer: String::new(),
        collections: vec![(0x0001, 0x0002), (0xff00, 0x0001)],
        interfaces: Vec::new(),
    }
}

/// A made-up scan error, for `--test-devices`.
const TEST_ERROR: &str = "qmk-via: Periphery test keyboard (ffff:0002): no reply to command 0x01 (test error, not a real device)";

/// State of a "Request support" lookup.
enum Lookup {
    Checking,
    /// The existing request, if there is one.
    Done(Option<Existing>),
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Settings,
    Keymap,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, start: StartOptions) -> Self {
        theme::apply(&cc.egui_ctx);
        let (local, rx) = channel();
        let tx = platform::spawn_worker(local.clone(), cc.egui_ctx.clone());
        let mut app = App {
            tx,
            rx,
            local,
            devices: Vec::new(),
            discover_errors: Vec::new(),
            unknown: Vec::new(),
            lookups: Default::default(),
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
            mouse: MouseState::default(),
            layer: 0,
            selected_key: None,
            key_group: 0,
            km_mode: KmMode::Keymap,
            multi: Default::default(),
            prop_value: None,
            pair_pick: Vec::new(),
            pair_mode: None,
            want_device: start.device,
            want_keymap: start.keymap,
            want_km: start.km,
            want_mouse_tab: start.mouse_tab,
            test_devices: start.test_devices,
        };
        app.send(Req::Discover);
        app
    }

    fn send(&mut self, r: Req) {
        self.busy += 1;
        self.last_refresh = Instant::now();
        (self.tx)(r);
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
        self.mouse = MouseState::default();
        self.send(Req::Select(i));
    }

    fn queue(&mut self, id: &str, v: Value) {
        self.pending.retain(|(p, _)| p != id);
        self.pending.push((id.to_string(), v));
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Devices(list, mut errors, mut unknown) => {
                    if self.test_devices {
                        unknown.push(test_unknown());
                        errors.push(TEST_ERROR.into());
                    }
                    self.unknown = unknown;
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
                        if let Some(t) = self.want_mouse_tab.take() {
                            self.mouse.tab = t;
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
                Msg::BackupReady(target, json) => match platform::save_backup(&target, &json) {
                    Ok(info) => self.info = Some(info),
                    Err(e) => self.error = Some(format!("Backup failed: {e:#}")),
                },
                Msg::RestoreFile { source, json } => {
                    if let Some(device) = self.selected {
                        self.send(Req::Restore { device, source, json, dry_run: true });
                    }
                }
                Msg::RestorePlan { source, json, lines } => self.restore_plan = Some((source, json, lines)),
                Msg::RequestLookup(id, result) => {
                    // A failed lookup still lets the user open the form.
                    self.lookups.insert(id, Lookup::Done(result.ok().flatten()));
                }
                Msg::Done => self.busy = self.busy.saturating_sub(1),
            }
        }
    }
}

fn kind_icon(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => icon::MOUSE,
        DeviceKind::Keyboard => icon::KEYBOARD,
        DeviceKind::Headset => icon::HEADPHONES,
        DeviceKind::Receiver => icon::USB,
        DeviceKind::Other => icon::PLUG,
    }
}

fn kind_name(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Headset => "Headset",
        DeviceKind::Receiver => "Receiver",
        DeviceKind::Other => "Device",
    }
}

/// One device in the sidebar: icon, name, and kind · brand underneath.
fn device_row(ui: &mut Ui, d: &DeviceInfo, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::click());
    let v = ui.visuals().clone();
    let (accent, _) = theme::accent(&v);
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, radius::MD, v.widgets.inactive.bg_fill);
        let bar = egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 12.0), egui::vec2(3.0, rect.height() - 24.0));
        painter.rect_filled(bar, radius::SM, accent);
    } else if resp.hovered() {
        painter.rect_filled(rect, radius::MD, v.widgets.hovered.weak_bg_fill);
    }
    let text = v.text_color();
    let icon_color = if selected { accent } else { v.weak_text_color() };
    painter.text(rect.left_center() + egui::vec2(22.0, 0.0), egui::Align2::CENTER_CENTER, kind_icon(d.kind), egui::FontId::proportional(20.0), icon_color);
    let x = rect.left() + 44.0;
    let width = rect.right() - x - 6.0;
    let title = ui.fonts(|f| f.layout(d.title(), egui::FontId::new(size::BODY, theme::semibold()), text, width));
    let sub = format!("{} · {}", kind_name(d.kind), d.brand);
    let sub = ui.fonts(|f| f.layout(sub, egui::FontId::proportional(size::SMALL), v.weak_text_color(), width));
    let title_h = title.size().y;
    let top = rect.center().y - (title_h + sub.size().y + 2.0) / 2.0;
    painter.galley(egui::pos2(x, top), title, text);
    painter.galley(egui::pos2(x, top + title_h + 2.0), sub, text);
    resp.on_hover_text(format!("{} ({:04x}:{:04x})", d.title(), d.vid, d.pid))
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

        let side = egui::Frame::none().fill(ctx.style().visuals.extreme_bg_color).inner_margin(egui::Margin::same(12.0));
        egui::SidePanel::left("devices").resizable(false).exact_width(240.0).frame(side).show(ctx, |ui| {
            ui.horizontal(|ui| {
                theme::logo(ui, 24.0);
                ui.label(RichText::new("Periphery").size(size::HEADING).semibold());
            });
            ui.add_space(14.0);
            ui.label(RichText::new("DEVICES").small().weak());
            ui.add_space(2.0);
            let mut pick = None;
            for (i, d) in self.devices.iter().enumerate() {
                if device_row(ui, d, self.selected == Some(i)).clicked() {
                    pick = Some(i);
                }
            }
            if self.devices.is_empty() {
                ui.label(RichText::new(platform::NO_DEVICES).weak());
            }
            if let Some(i) = pick {
                self.select(i);
            }
            ui.add_space(10.0);
            let full = egui::vec2(ui.available_width(), 32.0);
            if platform::ASKS_FOR_DEVICES
                && ui.add(egui::Button::new(format!("{}  Connect a device…", icon::PLUS)).min_size(full)).on_hover_text("Choose devices to give this page access to").clicked()
            {
                self.send(Req::Request(false));
            }
            if platform::ASKS_FOR_DEVICES
                && ui.add(egui::Button::new(RichText::new("Connect another device…").small().weak()).frame(false)).on_hover_text("Pick a device Periphery doesn't support yet, to request support for it").clicked()
            {
                self.send(Req::Request(true));
            }
            if ui.add(egui::Button::new(format!("{}  Rescan", icon::ARROWS_CLOCKWISE)).min_size(full)).clicked() {
                self.send(Req::Discover);
            }
            for e in &self.discover_errors {
                ui.label(RichText::new(e).small().color(theme::DANGER));
                let report = request::bug_report_url(None, e, "While scanning for devices\n", &platform::name());
                ui.hyperlink_to(RichText::new(format!("Report a bug {}", icon::ARROW_SQUARE_OUT)).small(), report)
                    .on_hover_text("Opens a prefilled GitHub issue; you can edit it before submitting");
                ui.add_space(4.0);
            }
            self.unknown_list(ui);
        });

        let header = egui::Frame::none().fill(ctx.style().visuals.panel_fill).inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 14.0, bottom: 10.0 });
        egui::TopBottomPanel::top("header").frame(header).show_separator_line(false).show(ctx, |ui| {
            self.header(ui);
        });

        self.confirm_window(ctx);
        self.restore_window(ctx);

        let central = egui::Frame::none().fill(ctx.style().visuals.panel_fill).inner_margin(egui::Margin { left: 20.0, right: 16.0, top: 4.0, bottom: 8.0 });
        egui::CentralPanel::default().frame(central).show(ctx, |ui| {
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
                theme::banner(ui, theme::WARN, n, false, None);
                ui.add_space(8.0);
            }
            let is_mouse = self.selected.and_then(|i| self.devices.get(i)).is_some_and(|d| d.kind == DeviceKind::Mouse);
            if is_mouse && snap.has_roles() {
                self.mouse_page(ui);
                return;
            }
            if snap.keymap.is_some() {
                theme::segmented(ui, &mut self.tab, &[(Tab::Settings, "Settings".into()), (Tab::Keymap, "Keymap".into())]);
                ui.add_space(8.0);
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
                ui.label(RichText::new(d.title()).size(size::TITLE).semibold());
                if self.busy > 0 || !self.pending.is_empty() {
                    ui.spinner().on_hover_text("Talking to the device…");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dev = self.selected;
                if ui.add_enabled(dev.is_some(), egui::Button::new(format!("{}  Restore…", icon::UPLOAD_SIMPLE))).on_hover_text("Load a backup; shows the changes before writing").clicked() {
                    platform::pick_restore(self.local.clone(), ui.ctx().clone());
                }
                if ui.add_enabled(dev.is_some(), egui::Button::new(format!("{}  Backup…", icon::DOWNLOAD_SIMPLE))).on_hover_text("Save every setting, the keymap and per-key settings to a file").clicked() {
                    let name = self.selected.and_then(|i| self.devices.get(i)).map_or("device".to_string(), |d| d.title().replace(' ', "-"));
                    if let Some(target) = platform::pick_backup_target(&format!("{name}.json")) {
                        self.send(Req::Backup(dev.unwrap(), target));
                    }
                }
                if ui.add(egui::Button::new(icon::ARROWS_CLOCKWISE)).on_hover_text("Read the device again").clicked() {
                    self.send(Req::Refresh);
                }
            });
        });
        if let Some(s) = &self.snap {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (k, v) in &s.status {
                    theme::chip(ui, k, v);
                }
            });
        }
        if let Some(i) = self.info.clone() {
            ui.add_space(6.0);
            if theme::banner(ui, theme::GOOD, &format!("{}  {i}", icon::CHECK_CIRCLE), true, None) {
                self.info = None;
            }
        }
        if let Some(e) = self.error.clone() {
            ui.add_space(6.0);
            let report = self.bug_report_url(&e);
            if theme::banner(ui, theme::DANGER, &format!("{}  {e}", icon::WARNING_CIRCLE), true, Some(("Report a bug", &report))) {
                self.error = None;
            }
        }
    }

    /// A prefilled bug report about `error`, naming the selected device and
    /// what it reported (connection, firmware…).
    fn bug_report_url(&self, error: &str) -> String {
        let device = self.selected.and_then(|i| self.devices.get(i));
        let mut details = String::new();
        if let Some(d) = device {
            details.push_str(&format!("Device: {} ({:04x}:{:04x})\n", d.title(), d.vid, d.pid));
        }
        for (k, v) in self.snap.iter().flat_map(|s| &s.status) {
            details.push_str(&format!("{k}: {v}\n"));
        }
        request::bug_report_url(device.map(|d| d.title()).as_deref(), error, &details, &platform::name())
    }

    /// Devices no driver handles, each with a way to ask for support.
    fn unknown_list(&mut self, ui: &mut Ui) {
        if self.unknown.is_empty() {
            return;
        }
        ui.add_space(16.0);
        ui.label(RichText::new("NOT SUPPORTED YET").small().weak());
        let mut lookup = None;
        for d in &self.unknown {
            ui.add_space(4.0);
            ui.label(format!("{}  {}", icon::PLUG, d.name())).on_hover_text(d.report());
            ui.label(RichText::new(d.id()).small().weak());
            let new_issue = request::new_issue_url(d, &platform::name());
            match self.lookups.get(&d.id()) {
                None => {
                    if ui.small_button("Request support").on_hover_text("Ask for this device on GitHub (you review the request before it's sent)").clicked() {
                        lookup = Some(d.clone());
                    }
                }
                Some(Lookup::Checking) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Checking existing requests…").small());
                    });
                }
                Some(Lookup::Done(Some(e))) => {
                    let state = if e.open { "open" } else { "closed" };
                    ui.hyperlink_to(format!("Already requested: #{} ({state}) {}", e.number, icon::ARROW_SQUARE_OUT), &e.url)
                        .on_hover_text("Add a thumbs-up or details there");
                    ui.hyperlink_to(RichText::new(format!("New request anyway {}", icon::ARROW_SQUARE_OUT)).small(), new_issue);
                }
                Some(Lookup::Done(None)) => {
                    ui.hyperlink_to(format!("Open the request form {}", icon::ARROW_SQUARE_OUT), new_issue).on_hover_text("Opens a prefilled GitHub issue; you can edit it before submitting");
                }
            }
        }
        if let Some(d) = lookup {
            self.lookups.insert(d.id(), Lookup::Checking);
            platform::lookup_request(&d, self.local.clone(), ui.ctx().clone());
        }
    }

    fn restore_window(&mut self, ctx: &egui::Context) {
        let Some((source, json, lines)) = self.restore_plan.clone() else { return };
        let mut close = false;
        egui::Window::new("Restore backup").collapsible(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(RichText::new(&source).weak());
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
                    ui.label(RichText::new(l).monospace().size(size::SMALL));
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Restore").clicked() {
                    if let Some(device) = self.selected {
                        self.send(Req::Restore { device, source: source.clone(), json: json.clone(), dry_run: false });
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
    fn mouse_page(&mut self, ui: &mut Ui) {
        let brand = self.selected.and_then(|i| self.devices.get(i)).map_or_else(String::new, |d| d.brand.clone());
        let Some(mut snap) = self.snap.take() else { return };
        let mut edits = mouse::Edits::default();
        mouse::show(ui, &mut snap, &mut self.mouse, &brand, &mut edits);
        self.snap = Some(snap);
        for (id, v) in edits.changes {
            self.queue(&id, v);
        }
        if edits.confirm.is_some() {
            self.confirm = edits.confirm;
        }
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        let mut snap = self.snap.take().unwrap();
        let mut changes = Vec::new();
        let mut confirm = None;
        let groups = std::mem::take(&mut snap.groups);
        let mut groups = groups;
        ui.columns(2, |cols| {
            for (gi, g) in groups.iter_mut().enumerate() {
                let ui = &mut cols[gi % 2];
                // Leave room for the label and the value box so cards don't overlap.
                ui.spacing_mut().slider_width = (ui.available_width() - 300.0).clamp(80.0, 220.0);
                theme::card(ui, &g.label, |ui| {
                    egui::Grid::new(("group", gi)).num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                        for s in &mut g.settings {
                            setting_row(ui, s, &mut changes, &mut confirm);
                        }
                    });
                });
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

pub(crate) fn setting_row(ui: &mut Ui, s: &mut Setting, changes: &mut Vec<(String, Value)>, confirm: &mut Option<(String, String)>) {
    let label = ui.label(s.label.trim());
    if let Some(h) = &s.help {
        label.on_hover_text(h);
    }
    setting_widget(ui, s, changes, confirm);
    ui.end_row();
}

/// The editor for one setting, without its label. Edits go to `changes`;
/// actions that need confirming go to `confirm`.
pub(crate) fn setting_widget(ui: &mut Ui, s: &mut Setting, changes: &mut Vec<(String, Value)>, confirm: &mut Option<(String, String)>) {
    let resp = match (&s.kind, &mut s.value) {
        (Kind::Toggle, Value::Bool(b)) => Some(toggle_switch(ui, b)),
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
            // The row's label already names the action.
            let text = if c.is_some() { "Start…" } else { "Run" };
            if ui.button(text).clicked() {
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
}

/// An on/off switch (the egui demo's toggle).
pub(crate) fn toggle_switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact(&response);
        let (accent, on_accent) = theme::accent(ui.visuals());
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();
        let (track, knob) = if *on { (accent, on_accent) } else { (visuals.bg_fill, visuals.fg_stroke.color) };
        ui.painter().rect(rect, radius, track, visuals.bg_stroke);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 0.7 * radius, knob);
    }
    response
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
        (Kind::Toggle, Value::Bool(b)) => toggle_switch(ui, b).changed(),
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
            let text_size = if label.chars().count() > 5 { size::TINY } else { size::MONO };
            let text = RichText::new(&label).size(text_size);
            let mut button = egui::Button::new(text).wrap();
            if let Some(c) = fill {
                button = button.fill(c);
            }
            if selected {
                let (accent, on_accent) = theme::accent(ui.visuals());
                button = button.stroke(egui::Stroke::new(2.5_f32, accent));
                if fill.is_none() {
                    button = egui::Button::new(RichText::new(&label).size(text_size).color(on_accent)).wrap().fill(accent);
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
        ui.label(RichText::new(format!("Key at row {row}, col {col} on {} — currently “{current}”", km.layer_names[layer])).semibold());
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
            ui.label(RichText::new(format!("{} key(s) selected", self.multi.len())).semibold());
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
                ui.label(RichText::new(format!("#{}  {} + {}", i + 1, name(p.a), name(p.b))).semibold());
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
