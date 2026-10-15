//! Feature-report transport.
//!
//! Every transfer is one 520-byte feature report with id 6 (`OemDrv.exe`
//! uses 6 when the firmware id is `0x18`, which every shipped model has):
//!
//! | byte | meaning                                            |
//! |------|----------------------------------------------------|
//! | 0    | report id `06`                                     |
//! | 1    | command; reads have bit 7 set                      |
//! | 2    | parameter (layer, sub-selector)                    |
//! | 3    | 0                                                  |
//! | 4    | number of packages, `ceil(len / 512)`              |
//! | 5    | package index                                      |
//! | 6-7  | payload length, little endian, ≤ 512              |
//! | 8-   | payload, zero padded to 512 bytes                  |
//!
//! A read sends the request header (with the wanted length) and then gets
//! feature report 6, whose header repeats the command and whose payload
//! starts at byte 8. A write sends header and data; the keyboard stores it
//! without a reply.
//!
//! The keyboard has several `0xFF00` collections; only one declares report 6.
//! On Windows each is its own device path, so [`Keyboard::open`] tries them
//! in turn.
//!
//! Everything is logged under the `aula` target (the browser console in the
//! web app): problems as warnings, opened devices and changes as info.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use hid_transport::{Api, DeviceInfo, sleep};
use log::{info, warn};

use crate::lighting::{LightingPage, PAGE_LEN};
use crate::models::{self, FILTER, Model};

pub const REPORT_ID: u8 = 6;
pub const HEADER: usize = 8;
pub const PAGE: usize = 512;
pub const REPORT_LEN: usize = HEADER + PAGE;

/// The vendor driver waits this long around every transfer.
const DELAY: Duration = Duration::from_millis(20);
const RETRY_DELAY: Duration = Duration::from_millis(70);
const RETRIES: usize = 3;

pub mod cmd {
    /// Lighting / settings page.
    pub const SET_LIGHTING: u8 = 0x04;
    pub const GET_LIGHTING: u8 = 0x84;
    /// Restores the factory configuration (payload `01`).
    pub const RESET: u8 = 0x11;
    /// Identity; parameter [`INFO_IDENTITY`] returns six bytes.
    pub const GET_INFO: u8 = 0x82;
    pub const INFO_IDENTITY: u8 = 0x01;
}

/// One package of a transfer of `total` bytes.
pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn describe(d: &DeviceInfo) -> String {
    format!(
        "{:04x}:{:04x} page {:04x} usage {:04x} interface {} {:?}",
        d.vendor_id, d.product_id, d.usage_page, d.usage, d.interface_number, d.product
    )
}

pub fn frame(command: u8, parameter: u8, total: usize, index: usize, len: usize, payload: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; REPORT_LEN];
    buf[0] = REPORT_ID;
    buf[1] = command;
    buf[2] = parameter;
    buf[4] = total.div_ceil(PAGE) as u8;
    buf[5] = index as u8;
    buf[6..8].copy_from_slice(&(len as u16).to_le_bytes());
    buf[HEADER..HEADER + payload.len()].copy_from_slice(payload);
    buf
}

pub struct Keyboard {
    dev: hid_transport::Device,
    pub identity: [u8; 6],
}

impl Keyboard {
    /// The vendor collections of every attached keyboard of the family.
    pub fn candidates(api: &Api) -> impl Iterator<Item = &DeviceInfo> {
        api.devices().filter(|d| FILTER.matches(d))
    }

    /// Opens every keyboard: tries each candidate collection of an interface
    /// until one answers the identity request.
    pub async fn open_all(api: &Api) -> Vec<Result<Keyboard>> {
        let mut groups: Vec<(i32, Vec<&DeviceInfo>)> = Vec::new();
        for info in Self::candidates(api) {
            info!(target: "aula", "candidate {}", describe(info));
            match groups.iter_mut().find(|(i, _)| *i == info.interface_number) {
                Some((_, list)) => list.push(info),
                None => groups.push((info.interface_number, vec![info])),
            }
        }
        let mut out = Vec::new();
        for (_, infos) in groups {
            let mut last = None;
            for info in infos {
                match Self::open(api, info).await {
                    Ok(kb) => {
                        last = Some(Ok(kb));
                        break;
                    }
                    Err(e) => {
                        // Expected on Windows for the collections without report 6.
                        info!(target: "aula", "{} did not answer: {e:#}", describe(info));
                        last = Some(Err(e));
                    }
                }
            }
            if let Some(Err(e)) = &last {
                warn!(target: "aula", "no collection of the keyboard answered; last error: {e:#}");
            }
            out.extend(last);
        }
        out
    }

    pub async fn open(api: &Api, info: &DeviceInfo) -> Result<Keyboard> {
        let dev = api.open(info).await?;
        let mut kb = Keyboard { dev, identity: [0; 6] };
        let id = kb.read(cmd::GET_INFO, cmd::INFO_IDENTITY, 6).await.context("reading the keyboard identity")?;
        kb.identity.copy_from_slice(&id);
        match kb.model() {
            Some(m) => info!(target: "aula", "opened AULA {} (identity {}) on {}", m.name, hex(&kb.identity), describe(info)),
            None => warn!(
                target: "aula",
                "unknown AULA model, identity {} on {}; please report it with the keyboard's name",
                hex(&kb.identity),
                describe(info)
            ),
        }
        Ok(kb)
    }

    pub fn model(&self) -> Option<&'static Model> {
        models::model(&self.identity)
    }

    async fn set_feature(&self, buf: &[u8]) -> Result<()> {
        let mut last = None;
        for attempt in 1..=RETRIES {
            match self.dev.send_feature_report(buf).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(target: "aula", "set feature report (header {}) failed, attempt {attempt}/{RETRIES}: {e:#}", hex(&buf[..HEADER]));
                    last = Some(e);
                }
            }
            sleep(RETRY_DELAY).await;
        }
        Err(last.unwrap()).context("HID set feature report")
    }

    /// Reads `len` bytes with `command`.
    pub async fn read(&self, command: u8, parameter: u8, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        for index in 0..len.div_ceil(PAGE) {
            let chunk = (len - out.len()).min(PAGE);
            sleep(DELAY).await;
            self.set_feature(&frame(command, parameter, len, index, chunk, &[])).await?;
            sleep(DELAY).await;
            let mut reply = vec![0u8; REPORT_LEN];
            reply[0] = REPORT_ID;
            let n = match self.dev.get_feature_report(&mut reply).await {
                Ok(n) => n,
                Err(e) => {
                    warn!(target: "aula", "get feature report for command {command:#04x} failed: {e:#}");
                    return Err(e).context("HID get feature report");
                }
            };
            if n < HEADER + chunk || reply[1] != command {
                warn!(
                    target: "aula",
                    "unexpected reply to command {command:#04x} ({n} bytes, wanted {}): {}",
                    HEADER + chunk,
                    hex(&reply[..n.min(HEADER + 16)])
                );
                bail!("unexpected reply to command {command:#04x}: {:02x?}", &reply[..n.min(HEADER)]);
            }
            out.extend_from_slice(&reply[HEADER..HEADER + chunk]);
        }
        Ok(out)
    }

    /// Writes `data` with `command`, split into 512-byte packages.
    pub async fn write(&self, command: u8, parameter: u8, data: &[u8]) -> Result<()> {
        for (index, chunk) in data.chunks(PAGE).enumerate() {
            sleep(DELAY).await;
            self.set_feature(&frame(command, parameter, data.len(), index, chunk.len(), chunk)).await?;
        }
        Ok(())
    }

    pub async fn lighting(&self) -> Result<LightingPage> {
        let bytes = self.read(cmd::GET_LIGHTING, 0, PAGE_LEN).await?;
        match LightingPage::new(bytes.clone()) {
            Ok(page) => {
                info!(target: "aula", "lighting page: effect {}, sleep {}", page.effect(), page.sleep());
                Ok(page)
            }
            Err(e) => {
                warn!(target: "aula", "{e:#}: {}", hex(&bytes));
                Err(e)
            }
        }
    }

    /// Writes the page and checks that it reads back unchanged.
    pub async fn set_lighting(&self, page: &LightingPage) -> Result<()> {
        page.check()?;
        info!(target: "aula", "writing lighting page: {}", page.diff(&self.lighting().await?));
        self.write(cmd::SET_LIGHTING, 0, page.bytes()).await?;
        let back = self.lighting().await?;
        if back.bytes() != page.bytes() {
            warn!(target: "aula", "lighting page read back differently: {}", back.diff(page));
            bail!("the keyboard did not store the lighting change");
        }
        Ok(())
    }

    pub async fn factory_reset(&self) -> Result<()> {
        info!(target: "aula", "factory reset");
        self.write(cmd::RESET, 0, &[1]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_request_matches_the_vendor_driver() {
        // OemDrv.exe sends 06 82 01 00 01 00 06 00 for the identity.
        let f = frame(cmd::GET_INFO, cmd::INFO_IDENTITY, 6, 0, 6, &[]);
        assert_eq!(f.len(), 520);
        assert_eq!(&f[..8], &[0x06, 0x82, 0x01, 0x00, 0x01, 0x00, 0x06, 0x00]);
        assert!(f[8..].iter().all(|b| *b == 0));
    }

    #[test]
    fn lighting_read_matches_the_web_driver() {
        // aula-f75 (WebHID) reads the page with 06 84 00 00 01 00 80 00.
        let f = frame(cmd::GET_LIGHTING, 0, 128, 0, 128, &[]);
        assert_eq!(&f[..8], &[0x06, 0x84, 0x00, 0x00, 0x01, 0x00, 0x80, 0x00]);
    }

    #[test]
    fn long_writes_are_split() {
        let data = vec![0xAB; 600];
        let second = frame(0x03, 1, data.len(), 1, 88, &data[512..]);
        assert_eq!(&second[..8], &[0x06, 0x03, 0x01, 0x00, 0x02, 0x01, 88, 0x00]);
        assert_eq!(second[8 + 87], 0xAB);
        assert_eq!(second[8 + 88], 0);
    }
}
