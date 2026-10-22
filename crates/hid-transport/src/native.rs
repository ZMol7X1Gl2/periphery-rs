//! `hidapi` backend. Every call blocks the current thread.

use std::ffi::CString;
use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::traffic::{self, Dir};
use crate::{DeviceInfo, Filter};

#[derive(Clone, Debug)]
pub(crate) struct Handle {
    path: CString,
}

pub struct Api {
    api: hidapi::HidApi,
    devices: Vec<DeviceInfo>,
}

impl Api {
    pub async fn new() -> Result<Api> {
        let api = hidapi::HidApi::new()?;
        let mut a = Api { api, devices: Vec::new() };
        a.collect();
        Ok(a)
    }

    /// Re-enumerates connected devices.
    pub async fn refresh(&mut self) -> Result<()> {
        self.api.refresh_devices()?;
        self.collect();
        Ok(())
    }

    fn collect(&mut self) {
        self.devices = self
            .api
            .device_list()
            .map(|d| DeviceInfo {
                vendor_id: d.vendor_id(),
                product_id: d.product_id(),
                usage_page: d.usage_page(),
                usage: d.usage(),
                interface_number: d.interface_number(),
                product: d.product_string().unwrap_or_default().to_string(),
                manufacturer: d.manufacturer_string().unwrap_or_default().to_string(),
                handle: Handle { path: d.path().to_owned() },
            })
            .collect();
    }

    pub fn devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter()
    }

    pub async fn open(&self, info: &DeviceInfo) -> Result<Device> {
        let dev = self.api.open_path(&info.handle.path).map_err(|e| anyhow!(e))?;
        Ok(Device { dev, id: format!("{:04x}:{:04x}", info.vendor_id, info.product_id) })
    }

    /// Desktop apps see every device already; nothing to ask for.
    pub async fn request(&mut self, _filters: &[Filter]) -> Result<()> {
        Ok(())
    }
}

pub struct Device {
    dev: hidapi::HidDevice,
    /// vid:pid, for [`traffic`].
    id: String,
}

impl Device {
    pub async fn write(&self, data: &[u8]) -> Result<usize> {
        traffic::note(&self.id, Dir::Write, data);
        Ok(self.dev.write(data)?)
    }

    /// Reads one input report into `buf`; 0 when nothing arrived in time.
    pub async fn read_timeout(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let n = self.dev.read_timeout(buf, ms)?;
        if n > 0 {
            traffic::note(&self.id, Dir::Read, &buf[..n]);
        }
        Ok(n)
    }

    pub async fn send_feature_report(&self, data: &[u8]) -> Result<()> {
        traffic::note(&self.id, Dir::FeatureSet, data);
        Ok(self.dev.send_feature_report(data)?)
    }

    /// `buf[0]` selects the report id; the reply is stored after it.
    pub async fn get_feature_report(&self, buf: &mut [u8]) -> Result<usize> {
        let n = self.dev.get_feature_report(buf)?;
        traffic::note(&self.id, Dir::FeatureGet, &buf[..n.min(buf.len())]);
        Ok(n)
    }
}

pub(crate) async fn sleep(d: Duration) {
    std::thread::sleep(d);
}
