//! WebHID backend (Chromium browsers). Pages only see devices the user has
//! granted through [`Api::request`]; Chrome remembers those grants, so
//! [`Api::new`] lists them again on the next visit.
//!
//! One WebHID device covers all of its top-level collections. It is listed
//! once per collection (like `hidapi` on Windows), every listing opens the
//! same device, and the report id picks the collection on writes. Each opened
//! [`Device`] receives every input report of the physical device; protocol
//! code already skips replies that are not its own.

use std::cell::RefCell;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use futures::future::{Either, select};
use futures::{FutureExt, StreamExt};
use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HidCollectionInfo, HidDeviceFilter, HidDeviceRequestOptions, HidInputReportEvent};

use crate::{DeviceInfo, Filter};

#[derive(Clone, Debug)]
pub(crate) struct Handle {
    dev: web_sys::HidDevice,
}

fn js_err(e: JsValue) -> anyhow::Error {
    let text = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| format!("{e:?}"));
    anyhow!(text)
}

fn hid() -> Result<web_sys::Hid> {
    let window = web_sys::window().ok_or_else(|| anyhow!("no browser window"))?;
    let navigator = window.navigator();
    if !js_sys::Reflect::has(&navigator, &"hid".into()).unwrap_or(false) {
        bail!("this browser has no WebHID; use Chrome, Edge or Opera on a desktop");
    }
    Ok(navigator.hid())
}

pub struct Api {
    devices: Vec<DeviceInfo>,
}

impl Api {
    pub async fn new() -> Result<Api> {
        let mut a = Api { devices: Vec::new() };
        a.refresh().await?;
        Ok(a)
    }

    /// Lists the devices this site may use.
    pub async fn refresh(&mut self) -> Result<()> {
        let list: Array = JsFuture::from(hid()?.get_devices()).await.map_err(js_err)?.unchecked_into();
        self.devices = list.iter().flat_map(|d| infos(d.unchecked_into())).collect();
        Ok(())
    }

    pub fn devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter()
    }

    pub async fn open(&self, info: &DeviceInfo) -> Result<Device> {
        let dev = info.handle.dev.clone();
        if !dev.opened() {
            JsFuture::from(dev.open()).await.map_err(js_err).map_err(|e| e.context("opening the device"))?;
        }
        let (tx, rx) = unbounded();
        let on_report = Closure::<dyn FnMut(HidInputReportEvent)>::new(move |ev: HidInputReportEvent| {
            let id = ev.report_id();
            let data = ev.data();
            let mut report = Vec::with_capacity(data.byte_length() + 1);
            // hidapi layout: the id leads only for numbered reports.
            if id != 0 {
                report.push(id);
            }
            report.extend(Uint8Array::new_with_byte_offset_and_length(&data.buffer(), data.byte_offset() as u32, data.byte_length() as u32).to_vec());
            let _ = tx.unbounded_send(report);
        });
        dev.add_event_listener_with_callback("inputreport", on_report.as_ref().unchecked_ref()).map_err(js_err)?;
        Ok(Device { dev, rx: RefCell::new(rx), on_report })
    }

    /// Shows the browser's device picker (needs a recent click or key press)
    /// and refreshes the list. Cancelling the picker is not an error.
    pub async fn request(&mut self, filters: &[Filter]) -> Result<()> {
        let mut list = Vec::new();
        for f in filters {
            let js = HidDeviceFilter::new();
            if let Some(v) = f.vendor_id {
                js.set_vendor_id(v as u32);
            }
            if let Some(v) = f.product_id {
                js.set_product_id(v);
            }
            if let Some(v) = f.usage_page {
                js.set_usage_page(v);
            }
            if let Some(v) = f.usage {
                js.set_usage(v);
            }
            list.push(js);
        }
        let options = HidDeviceRequestOptions::new(&list);
        JsFuture::from(hid()?.request_device(&options)).await.map_err(js_err)?;
        self.refresh().await
    }
}

/// One entry per top-level collection.
fn infos(dev: web_sys::HidDevice) -> Vec<DeviceInfo> {
    let base = DeviceInfo {
        vendor_id: dev.vendor_id(),
        product_id: dev.product_id(),
        usage_page: 0,
        usage: 0,
        interface_number: -1,
        product: dev.product_name(),
        manufacturer: String::new(),
        handle: Handle { dev: dev.clone() },
    };
    let collections: Vec<HidCollectionInfo> = dev.collections().iter().map(JsCast::unchecked_into).collect();
    if collections.is_empty() {
        return vec![base];
    }
    collections
        .iter()
        .map(|c| DeviceInfo { usage_page: c.get_usage_page().unwrap_or(0), usage: c.get_usage().unwrap_or(0), ..base.clone() })
        .collect()
}

pub struct Device {
    dev: web_sys::HidDevice,
    rx: RefCell<UnboundedReceiver<Vec<u8>>>,
    on_report: Closure<dyn FnMut(HidInputReportEvent)>,
}

impl Drop for Device {
    fn drop(&mut self) {
        // The device stays open: other listings of it may still be in use.
        let _ = self.dev.remove_event_listener_with_callback("inputreport", self.on_report.as_ref().unchecked_ref());
    }
}

impl Device {
    /// `data[0]` is the report id.
    pub async fn write(&self, data: &[u8]) -> Result<usize> {
        let Some((&id, body)) = data.split_first() else { bail!("empty report") };
        let mut body = body.to_vec();
        JsFuture::from(self.dev.send_report_with_u8_slice(id, &mut body).map_err(js_err)?).await.map_err(js_err)?;
        Ok(data.len())
    }

    /// Next input report, or 0 bytes when none arrived within `timeout`.
    /// Reports are queued from the moment the device is opened.
    pub async fn read_timeout(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        // Calls on one device never overlap, so the borrow is never contended.
        let mut rx = self.rx.borrow_mut();
        let report = if timeout.is_zero() {
            rx.next().now_or_never().flatten()
        } else {
            match select(rx.next(), Box::pin(sleep(timeout))).await {
                Either::Left((r, _)) => r,
                Either::Right(_) => None,
            }
        };
        let Some(report) = report else { return Ok(0) };
        let n = report.len().min(buf.len());
        buf[..n].copy_from_slice(&report[..n]);
        Ok(n)
    }

    /// `data[0]` is the report id.
    pub async fn send_feature_report(&self, data: &[u8]) -> Result<()> {
        let Some((&id, body)) = data.split_first() else { bail!("empty report") };
        let mut body = body.to_vec();
        JsFuture::from(self.dev.send_feature_report_with_u8_slice(id, &mut body).map_err(js_err)?).await.map_err(js_err)?;
        Ok(())
    }

    /// `buf[0]` selects the report id; the reply is stored after it.
    pub async fn get_feature_report(&self, buf: &mut [u8]) -> Result<usize> {
        let Some(&id) = buf.first() else { bail!("empty buffer") };
        let view: js_sys::DataView = JsFuture::from(self.dev.receive_feature_report(id)).await.map_err(js_err)?.unchecked_into();
        let mut data = Uint8Array::new_with_byte_offset_and_length(&view.buffer(), view.byte_offset() as u32, view.byte_length() as u32).to_vec();
        // Chrome leads numbered feature reports with their id; hidapi too.
        if id != 0 && data.first() == Some(&id) {
            data.remove(0);
        }
        let n = (data.len() + 1).min(buf.len());
        buf[1..n].copy_from_slice(&data[..n - 1]);
        Ok(n)
    }
}

pub(crate) async fn sleep(d: Duration) {
    let ms = d.as_millis().min(i32::MAX as u128) as i32;
    let promise = Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .expect("browser window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}
