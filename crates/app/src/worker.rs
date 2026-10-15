//! Owns the open devices and runs every HID exchange. The UI talks to it with
//! [`Req`] and gets [`Msg`] back; on the desktop it runs on its own thread, in
//! the browser as a task next to the UI.

use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;
use periphery_core::backup::{self, Backup};
use periphery_core::hid::Api;
use periphery_core::unknown::{UnknownDevice, unknown_devices};
use periphery_core::{Device, DeviceInfo, KeyPair, Snapshot, Value, discover_all};

/// How often the selected device is asked about on-device changes.
pub const POLL_CHANGES: Duration = Duration::from_millis(250);

pub enum Req {
    Discover,
    /// Ask the browser for access to more devices, then discover. `true`
    /// lists every device in the picker, not only supported ones.
    Request(bool),
    Select(usize),
    Refresh,
    Apply(usize, Vec<(String, Value)>),
    SetKey(usize, u8, u8, u8, u16),
    SetKeyProperty(usize, String, Vec<(u8, u8)>, Option<Value>),
    SetPairs(usize, String, Vec<KeyPair>),
    /// Capture a backup; the target (a path or a file name) comes back with it.
    Backup(usize, String),
    /// Restore backup JSON loaded from `source`; `true` = dry run that only
    /// reports the plan.
    Restore { device: usize, source: String, json: String, dry_run: bool },
}

pub enum Msg {
    /// Opened devices, discovery errors, devices no driver handles.
    Devices(Vec<DeviceInfo>, Vec<String>, Vec<UnknownDevice>),
    Snapshot(usize, Box<Snapshot>),
    Error(String),
    Info(String),
    /// Backup JSON for the target given in [`Req::Backup`].
    BackupReady(String, String),
    /// A backup file the user picked (sent by the UI's file picker).
    RestoreFile { source: String, json: String },
    /// Dry-run result awaiting confirmation.
    RestorePlan { source: String, json: String, lines: Vec<String> },
    /// Existing support request for an unknown device (by its id), sent by
    /// the UI's lookup.
    RequestLookup(String, Result<Option<crate::request::Existing>, String>),
    Done,
}

pub struct Worker {
    api: Option<Api>,
    devices: Vec<Box<dyn Device>>,
    selected: Option<usize>,
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Worker {
    pub fn new(tx: Sender<Msg>, ctx: egui::Context) -> Self {
        Worker { api: None, devices: Vec::new(), selected: None, tx, ctx }
    }

    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }

    async fn snapshot(&mut self, i: usize) {
        let Some(d) = self.devices.get_mut(i) else { return };
        let msg = match d.snapshot().await {
            Ok(s) => Msg::Snapshot(i, Box::new(s)),
            Err(e) => Msg::Error(format!("{}: {e:#}", d.info().title())),
        };
        self.send(msg);
    }

    async fn discover(&mut self, request: Option<bool>) {
        self.devices.clear(); // release handles before reopening
        let api = match self.api.as_mut() {
            Some(api) => api.refresh().await,
            None => Api::new().await.map(|a| self.api = Some(a)),
        };
        if let Err(e) = api {
            self.send(Msg::Devices(Vec::new(), vec![format!("HID: {e:#}")], Vec::new()));
            return;
        }
        let api = self.api.as_mut().unwrap();
        let drivers = crate::drivers();
        let mut request_error = None;
        if let Some(any) = request {
            let filters = if any { Vec::new() } else { periphery_core::filters(&drivers) };
            if let Err(e) = api.request(&filters).await {
                request_error = Some(format!("{e:#}"));
            }
        }
        let (devices, mut errors) = discover_all(api, &drivers).await;
        errors.extend(request_error);
        self.devices = devices;
        let infos: Vec<DeviceInfo> = self.devices.iter().map(|d| d.info().clone()).collect();
        let unknown = unknown_devices(api, &drivers)
            .into_iter()
            .filter(|u| crate::platform::LIST_ALL_UNKNOWN || u.is_configurable())
            .collect();
        self.send(Msg::Devices(infos, errors, unknown));
    }

    pub async fn handle(&mut self, req: Req) {
        match req {
            Req::Discover => self.discover(None).await,
            Req::Request(any) => self.discover(Some(any)).await,
            Req::Select(i) => {
                self.selected = Some(i);
                self.snapshot(i).await;
            }
            Req::Refresh => {
                if let Some(i) = self.selected {
                    self.snapshot(i).await;
                }
            }
            Req::Apply(i, changes) => {
                if let Some(d) = self.devices.get_mut(i) {
                    for (id, v) in &changes {
                        if let Err(e) = d.set(id, v).await {
                            self.send(Msg::Error(format!("{e:#}")));
                            break;
                        }
                    }
                }
                self.snapshot(i).await;
            }
            Req::SetKeyProperty(i, id, keys, value) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key_property(&id, &keys, value.as_ref()).await {
                        self.send(Msg::Error(format!("{e:#}")));
                    }
                }
                self.snapshot(i).await;
            }
            Req::Backup(i, target) => {
                let Some(d) = self.devices.get_mut(i) else { self.send(Msg::Done); return };
                let msg = match Backup::capture(d.as_mut()).await {
                    Ok(b) => Msg::BackupReady(target, b.to_json()),
                    Err(e) => Msg::Error(format!("Backup failed: {e:#}")),
                };
                self.send(msg);
            }
            Req::Restore { device: i, source, json, dry_run } => {
                let Some(d) = self.devices.get_mut(i) else { self.send(Msg::Done); return };
                let result = match Backup::from_json(&json) {
                    Ok(b) => backup::restore(d.as_mut(), &b, dry_run).await,
                    Err(e) => Err(e),
                };
                let msg = match result {
                    Ok(r) if dry_run => {
                        let mut lines = r.applied;
                        lines.extend(r.skipped.into_iter().map(|s| format!("(skipped) {s}")));
                        Msg::RestorePlan { source, json, lines }
                    }
                    Ok(r) if r.errors.is_empty() => Msg::Info(format!("Restored {} value(s)", r.applied.len())),
                    Ok(r) => Msg::Error(format!("Restored {} value(s); failed: {}", r.applied.len(), r.errors.join("; "))),
                    Err(e) => Msg::Error(format!("Restore failed: {e:#}")),
                };
                self.send(msg);
                if !dry_run {
                    self.snapshot(i).await;
                }
            }
            Req::SetPairs(i, id, pairs) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key_pairs(&id, &pairs).await {
                        self.send(Msg::Error(format!("{e:#}")));
                    }
                }
                self.snapshot(i).await;
            }
            Req::SetKey(i, layer, row, col, code) => {
                if let Some(d) = self.devices.get_mut(i) {
                    if let Err(e) = d.set_key(layer, row, col, code).await {
                        self.send(Msg::Error(format!("{e:#}")));
                        self.snapshot(i).await;
                    }
                }
            }
        }
        self.send(Msg::Done);
    }

    /// Refreshes the selected device when it reports on-device changes. Call
    /// every [`POLL_CHANGES`] while idle.
    pub async fn tick(&mut self) {
        let Some(i) = self.selected else { return };
        let Some(d) = self.devices.get_mut(i) else { return };
        if let Ok(true) = d.poll_changes().await {
            self.snapshot(i).await;
        }
    }
}
