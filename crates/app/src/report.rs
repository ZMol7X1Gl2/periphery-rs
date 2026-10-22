//! "Report an issue": a short history of what the user asked devices to do,
//! and a window that turns the actions they pick into a prefilled GitHub
//! issue plus a full log (with the raw device traffic) to paste into it.
//! The history lives in memory only and is gone when the app closes.

use std::collections::{BTreeSet, VecDeque};

use eframe::egui::{self, RichText, Ui};
use periphery_core::hid::{SystemTime, UNIX_EPOCH};

use crate::request;
use crate::theme::{self, Semibold, icon};

/// Actions kept in the history.
const KEEP: usize = 50;
/// Actions put in the issue link; the full log has them all.
const IN_LINK: usize = 15;

/// One request sent to the worker and what came of it.
pub struct Entry {
    id: u64,
    /// Seconds since the Unix epoch.
    at: u64,
    /// Device title and vid:pid, when the request was about one.
    device: Option<String>,
    action: String,
    errors: Vec<String>,
    traffic: Vec<String>,
    /// Sent by the app itself (a timed refresh): kept only when it failed.
    quiet: bool,
}

impl Entry {
    fn failed(&self) -> bool {
        !self.errors.is_empty()
    }

    fn summary(&self) -> String {
        let device = self.device.as_deref().map(|d| format!("{d}: ")).unwrap_or_default();
        let result = if self.failed() { format!("failed: {}", self.errors.join("; ")) } else { "OK".into() };
        format!("{} {device}{} — {result}", clock(self.at), self.action)
    }
}

/// HH:MM:SS in UTC.
fn clock(secs: u64) -> String {
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Requests in flight (the worker answers them in order) and the finished
/// ones, newest last.
#[derive(Default)]
pub struct Activity {
    pending: VecDeque<Entry>,
    done: VecDeque<Entry>,
    next_id: u64,
}

impl Activity {
    /// Notes a request just sent to the worker.
    pub fn begin(&mut self, action: String, device: Option<String>, quiet: bool) {
        self.next_id += 1;
        let entry = Entry { id: self.next_id, at: now(), device, action, errors: Vec::new(), traffic: Vec::new(), quiet };
        self.pending.push_back(entry);
    }

    /// An error the worker reported for the request it is running.
    pub fn error(&mut self, e: &str) {
        if let Some(entry) = self.pending.front_mut() {
            entry.errors.push(e.to_string());
        }
    }

    /// The request the worker was running finished, with this traffic.
    pub fn finish(&mut self, traffic: Vec<String>) {
        let Some(mut entry) = self.pending.pop_front() else { return };
        if entry.quiet && !entry.failed() {
            return;
        }
        entry.traffic = traffic;
        self.done.push_back(entry);
        while self.done.len() > KEEP {
            self.done.pop_front();
        }
    }

    /// The newest finished action that failed with `error`, else the newest
    /// failed one.
    fn matching(&self, error: Option<&str>) -> Option<u64> {
        let failed = || self.done.iter().rev().filter(|e| e.failed());
        error
            .and_then(|err| failed().find(|e| e.errors.iter().any(|x| x.contains(err) || err.contains(x.as_str()))))
            .or_else(|| failed().next())
            .map(|e| e.id)
    }
}

/// The open "Report an issue" window.
pub struct ReportWindow {
    ticked: BTreeSet<u64>,
    text: String,
    copied: bool,
}

/// What the app knows about the selected device, for the report.
pub struct DeviceFacts {
    pub title: String,
    pub id: String,
    /// Status chips: connection, firmware, battery…
    pub status: Vec<(String, String)>,
}

impl ReportWindow {
    /// Opens the window with the action that failed with `error` ticked (or
    /// the newest failed one), plus the newest action.
    pub fn open(activity: &Activity, error: Option<&str>) -> Self {
        let mut ticked: BTreeSet<u64> = activity.matching(error).into_iter().collect();
        ticked.extend(activity.done.back().map(|e| e.id));
        ReportWindow { ticked, text: String::new(), copied: false }
    }

    /// Draws the window; returns false once it should close.
    pub fn show(&mut self, ctx: &egui::Context, activity: &Activity, device: Option<&DeviceFacts>) -> bool {
        let mut open = true;
        let mut close = false;
        egui::Window::new("Report an issue")
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(RichText::new("Tick what went wrong. The report opens on GitHub, where you can read and edit it before sending.").weak());
                ui.add_space(8.0);
                ui.label(RichText::new("Recent actions (UTC)").semibold());
                self.list(ui, activity);
                ui.add_space(8.0);
                ui.label(RichText::new("What happened?").semibold());
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .hint_text("What did you expect, and what happened instead?")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let picked = self.picked(activity);
                    let ready = !picked.is_empty() || !self.text.trim().is_empty();
                    let (fill, on) = theme::accent(ui.visuals());
                    let open_issue = egui::Button::new(RichText::new(format!("{}  Open GitHub issue", icon::ARROW_SQUARE_OUT)).color(on)).fill(fill);
                    if ui.add_enabled(ready, open_issue).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(self.issue_url(&picked, device)));
                        close = true;
                    }
                    if ui.add_enabled(ready, egui::Button::new(format!("{}  Copy full log", icon::COPY))).clicked() {
                        ui.ctx().copy_text(self.full_log(&picked, device));
                        self.copied = true;
                    }
                    if self.copied {
                        ui.label(RichText::new(format!("{}  Copied", icon::CHECK_CIRCLE)).color(theme::GOOD));
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The issue gets the ticked actions and the device details. The full log adds the raw device \
                         traffic; paste it into the issue's Details box.",
                    )
                    .small()
                    .weak(),
                );
            });
        open && !close
    }

    fn list(&mut self, ui: &mut Ui, activity: &Activity) {
        if activity.done.is_empty() {
            ui.label(RichText::new("Nothing has happened yet. Use the device, then come back here.").weak());
            return;
        }
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for e in activity.done.iter().rev() {
                let mut on = self.ticked.contains(&e.id);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut on, "");
                    ui.label(RichText::new(clock(e.at)).monospace().weak());
                    ui.vertical(|ui| {
                        ui.label(&e.action);
                        if let Some(d) = &e.device {
                            ui.label(RichText::new(d).small().weak());
                        }
                        if e.failed() {
                            ui.label(RichText::new(e.errors.join("; ")).small().color(theme::DANGER));
                        }
                    });
                });
                if on {
                    self.ticked.insert(e.id);
                } else {
                    self.ticked.remove(&e.id);
                }
            }
        });
    }

    /// The ticked actions, oldest first.
    fn picked<'a>(&self, activity: &'a Activity) -> Vec<&'a Entry> {
        activity.done.iter().filter(|e| self.ticked.contains(&e.id)).collect()
    }

    fn what_happened(&self, picked: &[&Entry]) -> String {
        let mut out = self.text.trim().to_string();
        let failures: Vec<String> = picked.iter().flat_map(|e| e.errors.iter().cloned()).collect();
        if !failures.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("Errors:\n");
            for f in failures {
                out.push_str(&format!("- {f}\n"));
            }
        }
        out
    }

    fn details(&self, picked: &[&Entry], device: Option<&DeviceFacts>, limit: usize) -> String {
        let mut out = String::new();
        if let Some(d) = device {
            out.push_str(&format!("Device: {} ({})\n", d.title, d.id));
            for (k, v) in &d.status {
                out.push_str(&format!("{k}: {v}\n"));
            }
        }
        if !picked.is_empty() {
            out.push_str("Actions (UTC):\n");
            let skip = picked.len().saturating_sub(limit);
            if skip > 0 {
                out.push_str(&format!("({skip} earlier ones are in the full log)\n"));
            }
            for e in &picked[skip..] {
                out.push_str(&format!("{}\n", e.summary()));
            }
        }
        out
    }

    fn issue_url(&self, picked: &[&Entry], device: Option<&DeviceFacts>) -> String {
        let named = device.map(|d| d.title.clone()).or_else(|| picked.iter().rev().find_map(|e| e.device.clone()));
        let what = self.what_happened(picked);
        let what = if what.is_empty() { "(describe what went wrong)".to_string() } else { what };
        request::bug_report_url(named.as_deref(), &what, &self.details(picked, device, IN_LINK), &crate::platform::name())
    }

    fn full_log(&self, picked: &[&Entry], device: Option<&DeviceFacts>) -> String {
        let mut out = format!("Periphery {} ({})\n", env!("CARGO_PKG_VERSION"), crate::platform::name());
        let what = self.what_happened(picked);
        if !what.is_empty() {
            out.push_str(&format!("\n{what}\n"));
        }
        out.push('\n');
        out.push_str(&self.details(picked, device, usize::MAX));
        for e in picked.iter().filter(|e| !e.traffic.is_empty()) {
            out.push_str(&format!("\nTraffic for {} {}:\n", clock(e.at), e.action));
            for line in &e.traffic {
                out.push_str(&format!("  {line}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity() -> Activity {
        let mut a = Activity::default();
        a.begin("Scan for devices".into(), None, false);
        a.finish(vec![]);
        a.begin("Read the device".into(), Some("Mouse (1234:5678)".into()), true);
        a.finish(vec!["1234:5678 -> 10 ff".into()]);
        a.begin("Set Polling rate to 1000 Hz".into(), Some("Mouse (1234:5678)".into()), false);
        a.error("no reply to command 0x0a");
        a.finish(vec!["1234:5678 -> 10 ff 0a".into()]);
        a
    }

    #[test]
    fn quiet_actions_are_kept_only_when_they_fail() {
        let a = activity();
        let actions: Vec<&str> = a.done.iter().map(|e| e.action.as_str()).collect();
        assert_eq!(actions, ["Scan for devices", "Set Polling rate to 1000 Hz"]);
    }

    #[test]
    fn report_starts_with_the_failed_action_and_carries_its_traffic() {
        let a = activity();
        let w = ReportWindow::open(&a, Some("no reply to command 0x0a"));
        let picked = w.picked(&a);
        assert_eq!(picked.len(), 1);
        let log = w.full_log(&picked, None);
        assert!(log.contains("failed: no reply to command 0x0a"));
        assert!(log.contains("  1234:5678 -> 10 ff 0a"));
        let url = w.issue_url(&picked, None);
        assert!(url.contains("template=bug-report.yml"));
        // Traffic stays out of the link.
        assert!(!url.contains("10%20ff%200a"));
    }

    #[test]
    fn clock_is_utc() {
        assert_eq!(clock(86_400 + 3600 * 13 + 60 * 5 + 9), "13:05:09");
    }
}
