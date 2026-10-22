//! Async HID access with the same shape as `hidapi`, backed by `hidapi` on
//! desktop platforms and by WebHID in the browser.
//!
//! Byte layouts follow `hidapi` everywhere, so protocol code is identical on
//! both: the first byte of a written report (output or feature) is the report
//! id (0 for devices without numbered reports), and a read input report starts
//! with its report id only when the device uses numbered reports.
//!
//! The native backend blocks inside its `async fn`s; drive it with a simple
//! executor such as `pollster::block_on` on a thread that may block.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let api = hid_transport::Api::new().await?;
//! for info in api.devices() {
//!     println!("{:04x}:{:04x} page {:04x}", info.vendor_id, info.product_id, info.usage_page);
//! }
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
use native as backend;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
use web as backend;

pub use backend::{Api, Device};

pub mod traffic;

/// One HID top-level collection (on Windows, one openable device path).
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// USB interface number; -1 when unknown (always in the browser).
    pub interface_number: i32,
    pub product: String,
    /// Empty when unknown (always in the browser).
    pub manufacturer: String,
    pub(crate) handle: backend::Handle,
}

/// Devices a driver can talk to, for the browser's device picker. Every set
/// field must match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
}

impl Filter {
    pub fn matches(&self, d: &DeviceInfo) -> bool {
        self.vendor_id.is_none_or(|v| v == d.vendor_id)
            && self.product_id.is_none_or(|v| v == d.product_id)
            && self.usage_page.is_none_or(|v| v == d.usage_page)
            && self.usage.is_none_or(|v| v == d.usage)
    }
}

/// Waits without blocking the browser; sleeps the thread on desktop.
pub async fn sleep(d: Duration) {
    backend::sleep(d).await
}
