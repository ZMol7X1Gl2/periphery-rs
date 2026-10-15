//! What differs between the desktop app and the web app: how the worker
//! runs, where backups go and how files are picked.

use std::rc::Rc;
use std::sync::mpsc::Sender;

use anyhow::Result;
use eframe::egui;

use periphery_core::unknown::UnknownDevice;

use crate::request;
use crate::worker::{Msg, Req};

/// Sends a request to the worker.
pub type ReqSender = Rc<dyn Fn(Req)>;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::thread;

    use super::*;
    use crate::worker::{POLL_CHANGES, Worker};

    /// The desktop sees every device; nothing to ask for.
    pub const ASKS_FOR_DEVICES: bool = false;
    pub const NO_DEVICES: &str = "No supported devices found.";
    /// Every HID device shows up, so only list ones that look configurable.
    pub const LIST_ALL_UNKNOWN: bool = false;

    /// Where Periphery runs, for support requests.
    pub fn name() -> String {
        format!("desktop, {}", std::env::consts::OS)
    }

    /// Looks for an issue already asking for `d` and posts
    /// [`Msg::RequestLookup`].
    pub fn lookup_request(d: &UnknownDevice, msg: Sender<Msg>, ctx: egui::Context) {
        let (id, url) = (d.id(), request::search_url(d));
        thread::spawn(move || {
            let result = ureq::get(&url)
                .set("User-Agent", concat!("periphery/", env!("CARGO_PKG_VERSION")))
                .set("Accept", "application/vnd.github+json")
                .timeout(std::time::Duration::from_secs(10))
                .call()
                .map_err(anyhow::Error::from)
                .and_then(|r| Ok(r.into_string()?))
                .and_then(|text| request::parse_search(&text))
                .map_err(|e| format!("{e:#}"));
            let _ = msg.send(Msg::RequestLookup(id, result));
            ctx.request_repaint();
        });
    }

    /// Runs the worker on its own thread; HID calls block there.
    pub fn spawn_worker(msg: Sender<Msg>, ctx: egui::Context) -> ReqSender {
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut worker = Worker::new(msg, ctx);
            loop {
                match rx.recv_timeout(POLL_CHANGES) {
                    Ok(req) => pollster::block_on(worker.handle(req)),
                    Err(RecvTimeoutError::Timeout) => pollster::block_on(worker.tick()),
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Rc::new(move |r| {
            let _ = tx.send(r);
        })
    }

    /// Asks where to save a backup; returns the path.
    pub fn pick_backup_target(name: &str) -> Option<String> {
        let path = rfd::FileDialog::new().add_filter("Periphery backup", &["json"]).set_file_name(name).save_file()?;
        Some(path.display().to_string())
    }

    pub fn save_backup(target: &str, json: &str) -> Result<String> {
        std::fs::write(target, json)?;
        Ok(format!("Backup saved to {target}"))
    }

    /// Asks for a backup file and posts its contents as [`Msg::RestoreFile`].
    pub fn pick_restore(msg: Sender<Msg>, _ctx: egui::Context) {
        let Some(path) = rfd::FileDialog::new().add_filter("Periphery backup", &["json"]).pick_file() else { return };
        let source = path.display().to_string();
        let _ = msg.send(match std::fs::read_to_string(&path) {
            Ok(json) => Msg::RestoreFile { source, json },
            Err(e) => Msg::Error(format!("Reading {source}: {e}")),
        });
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;

    use futures::StreamExt;
    use futures::channel::mpsc::unbounded;
    use futures::future::{Either, select};
    use periphery_core::hid;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::spawn_local;

    use super::*;
    use crate::worker::{POLL_CHANGES, Worker};

    /// Pages only see devices the user picked in the browser's dialog.
    pub const ASKS_FOR_DEVICES: bool = true;
    pub const NO_DEVICES: &str = "No devices connected yet. Use “Connect a device…” to pick one.";
    /// The page only sees devices the user picked, so list every one.
    pub const LIST_ALL_UNKNOWN: bool = true;

    /// Where Periphery runs, for support requests.
    pub fn name() -> String {
        "web app".into()
    }

    /// Looks for an issue already asking for `d` and posts
    /// [`Msg::RequestLookup`].
    pub fn lookup_request(d: &UnknownDevice, msg: Sender<Msg>, ctx: egui::Context) {
        let (id, url) = (d.id(), request::search_url(d));
        spawn_local(async move {
            let result = fetch_text(&url).await.and_then(|text| request::parse_search(&text)).map_err(|e| format!("{e:#}"));
            let _ = msg.send(Msg::RequestLookup(id, result));
            ctx.request_repaint();
        });
    }

    async fn fetch_text(url: &str) -> Result<String> {
        use wasm_bindgen_futures::JsFuture;

        let err = |e: wasm_bindgen::JsValue| anyhow::anyhow!("{e:?}");
        let window = web_sys::window().ok_or_else(|| anyhow::anyhow!("no browser window"))?;
        let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await.map_err(err)?.unchecked_into();
        let text = JsFuture::from(resp.text().map_err(err)?).await.map_err(err)?;
        text.as_string().ok_or_else(|| anyhow::anyhow!("empty reply"))
    }

    /// Runs the worker as a task next to the UI.
    pub fn spawn_worker(msg: Sender<Msg>, ctx: egui::Context) -> ReqSender {
        let (tx, mut rx) = unbounded::<Req>();
        spawn_local(async move {
            let mut worker = Worker::new(msg, ctx);
            // Keep one pending `next()` so no request is dropped by a timeout.
            let mut next = rx.next();
            loop {
                match select(next, Box::pin(hid::sleep(POLL_CHANGES))).await {
                    Either::Left((Some(req), _)) => {
                        worker.handle(req).await;
                        next = rx.next();
                    }
                    Either::Left((None, _)) => return,
                    Either::Right((_, pending)) => {
                        next = pending;
                        worker.tick().await;
                    }
                }
            }
        });
        Rc::new(move |r| {
            let _ = tx.unbounded_send(r);
        })
    }

    /// Backups are downloaded under this file name.
    pub fn pick_backup_target(name: &str) -> Option<String> {
        Some(name.to_string())
    }

    pub fn save_backup(target: &str, json: &str) -> Result<String> {
        download(target, json).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(format!("Backup downloaded as {target}"))
    }

    fn download(name: &str, text: &str) -> Result<(), wasm_bindgen::JsValue> {
        let parts = js_sys::Array::of1(&text.into());
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("application/json");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let a: web_sys::HtmlAnchorElement = document.create_element("a")?.unchecked_into();
        a.set_href(&url);
        a.set_download(name);
        a.click();
        web_sys::Url::revoke_object_url(&url)
    }

    thread_local! {
        static PICKING: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Asks for a backup file and posts its contents as [`Msg::RestoreFile`].
    pub fn pick_restore(msg: Sender<Msg>, ctx: egui::Context) {
        if PICKING.with(|p| p.replace(true)) {
            return;
        }
        spawn_local(async move {
            if let Some(file) = rfd::AsyncFileDialog::new().add_filter("Periphery backup", &["json"]).pick_file().await {
                let json = String::from_utf8_lossy(&file.read().await).into_owned();
                let _ = msg.send(Msg::RestoreFile { source: file.file_name(), json });
                ctx.request_repaint();
            }
            PICKING.with(|p| *p.borrow_mut() = false);
        });
    }
}
