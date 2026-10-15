use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use hid_transport::{Api, Device, Instant, SystemTime, UNIX_EPOCH};

use crate::protocol::{self, CHUNK, PACKET_LEN, REPORT_ID, Sensor, cmd};

const REPLY_TIMEOUT: Duration = Duration::from_millis(200);
const RETRIES: usize = 5;

/// Handshake reply (`EncryptionData`): bytes 9..12 of the response.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub cid: u8,
    pub mid: u8,
    pub kind: u8,
}

impl Info {
    pub fn wired(&self) -> bool {
        matches!(self.kind, 2 | 3)
    }

    pub fn max_report_rate(&self) -> u32 {
        match self.kind {
            1 => 4000,
            4 => 2000,
            3 | 5 => 8000,
            _ => 1000,
        }
    }
}

pub struct Mouse {
    dev: Device,
    pub info: Info,
    pub product_id: u16,
    pub product: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok([u8; PACKET_LEN]),
    /// Device answered with status 1 (unsupported / rejected).
    Rejected,
}

impl Mouse {
    /// Finds the vendor collection that answers the handshake. On Windows each
    /// HID collection is its own path, so we probe every candidate the same way
    /// the web driver filters for "the collection whose output report ID is 8".
    pub async fn open(api: &Api) -> Result<Self> {
        let mut candidates: Vec<_> = api
            .devices()
            .filter(|d| {
                d.vendor_id == protocol::VENDOR_ID
                    && protocol::PRODUCT_IDS.contains(&d.product_id)
            })
            .collect();
        if candidates.is_empty() {
            bail!("no Scyrox receiver or wired mouse found (VID 0x3554)");
        }
        // Vendor-defined usage pages first; they are the likely config channel.
        candidates.sort_by_key(|d| d.usage_page < 0xFF00);

        let mut last_err = None;
        for c in candidates {
            let dev = match api.open(c).await {
                Ok(d) => d,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            let mut m = Mouse {
                dev,
                info: Info { cid: 0, mid: 0, kind: 0 },
                product_id: c.product_id,
                product: c.product.clone(),
            };
            match m.handshake().await {
                Ok(info) => {
                    m.info = info;
                    return Ok(m);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err
            .unwrap_or_else(|| anyhow!("no interface answered"))
            .context("found a Scyrox device but no HID interface answered the handshake"))
    }

    pub fn sensor(&self) -> Sensor {
        Sensor::from_mid(self.info.mid)
    }

    async fn send(&self, p: &[u8; PACKET_LEN]) -> Result<()> {
        let mut buf = [0u8; PACKET_LEN + 1];
        buf[0] = REPORT_ID;
        buf[1..].copy_from_slice(p);
        self.dev.write(&buf).await.context("HID write")?;
        Ok(())
    }

    async fn recv(&self, deadline: Instant) -> Result<Option<[u8; PACKET_LEN]>> {
        let mut buf = [0u8; 64];
        let left = deadline.saturating_duration_since(Instant::now());
        let n = self.dev.read_timeout(&mut buf, left).await?;
        let data = match n {
            0 => return Ok(None),
            n if n > PACKET_LEN && buf[0] == REPORT_ID => &buf[1..=PACKET_LEN],
            n if n >= PACKET_LEN => &buf[..PACKET_LEN],
            _ => return Ok(Some([0xFF; PACKET_LEN])), // foreign report; ignore
        };
        Ok(Some(data.try_into().unwrap()))
    }

    /// Send a packet and wait for its echo. Matching mirrors `Send_HID_Buffer`:
    /// the first 3 header bytes (5 for flash reads) must match, or status == 1.
    pub async fn transact(&self, p: &[u8; PACKET_LEN]) -> Result<Reply> {
        let header = if p[0] == cmd::READ_FLASH { 5 } else { 3 };
        for _ in 0..RETRIES {
            self.send(p).await?;
            let deadline = Instant::now() + REPLY_TIMEOUT;
            while Instant::now() < deadline {
                let Some(r) = self.recv(deadline).await? else { break };
                if r[0] != p[0] {
                    continue; // unsolicited (e.g. StatusChanged) or stale reply
                }
                if r[1] == 1 {
                    return Ok(Reply::Rejected);
                }
                if r[..header] == p[..header] {
                    return Ok(Reply::Ok(r));
                }
            }
        }
        bail!("no reply to command {} after {RETRIES} attempts", p[0])
    }

    pub async fn command(&self, c: u8, data: &[u8]) -> Result<Reply> {
        self.transact(&protocol::packet(c, 0, data)).await
    }

    async fn command_ok(&self, c: u8, data: &[u8]) -> Result<[u8; PACKET_LEN]> {
        match self.command(c, data).await? {
            Reply::Ok(r) => Ok(r),
            Reply::Rejected => bail!("device rejected command {c}"),
        }
    }

    async fn handshake(&self) -> Result<Info> {
        // The web driver sends 4 random bytes + 4 zeros; content is not checked.
        let seed = SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos();
        let mut data = [0u8; 8];
        data[..4].copy_from_slice(&seed.to_le_bytes());
        let r = self.command_ok(cmd::ENCRYPTION_DATA, &data).await?;
        Ok(Info { cid: r[9], mid: r[10], kind: r[11] })
    }

    /// Whether the mouse itself is reachable (always true when wired).
    pub async fn online(&self) -> Result<bool> {
        Ok(self.command_ok(cmd::DEVICE_ONLINE, &[]).await?[5] == 1)
    }

    pub async fn require_online(&self) -> Result<()> {
        if !self.online().await? {
            bail!("receiver found but the mouse is offline (switched off or asleep — move it)");
        }
        Ok(())
    }

    /// (raw level %, charging, millivolts)
    pub async fn battery(&self) -> Result<(u8, bool, u16)> {
        let r = self.command_ok(cmd::BATTERY_LEVEL, &[]).await?;
        Ok((r[5], r[6] == 1, u16::from_be_bytes([r[7], r[8]])))
    }

    pub async fn version(&self) -> Result<String> {
        let r = self.command_ok(cmd::READ_VERSION, &[]).await?;
        Ok(format!("v{}.{:02x}", r[5], r[6]))
    }

    pub async fn dongle_version(&self) -> Result<Option<String>> {
        Ok(match self.command(cmd::GET_DONGLE_VERSION, &[]).await? {
            Reply::Ok(r) => Some(format!("v{}.{:02x}", r[5], r[6])),
            Reply::Rejected => None,
        })
    }

    pub async fn profile(&self) -> Result<Option<u8>> {
        Ok(match self.command(cmd::GET_CURRENT_CONFIG, &[]).await? {
            Reply::Ok(r) => Some(r[5]),
            Reply::Rejected => None,
        })
    }

    pub async fn set_profile(&self, p: u8) -> Result<()> {
        self.command_ok(cmd::SET_CURRENT_CONFIG, &[p]).await.map(drop)
    }

    pub async fn long_range(&self) -> Result<Option<bool>> {
        Ok(match self.command(cmd::GET_LONG_RANGE, &[]).await? {
            Reply::Ok(r) => Some(r[5] == 1),
            Reply::Rejected => None,
        })
    }

    pub async fn set_long_range(&self, on: bool) -> Result<()> {
        let mut data = [0u8; CHUNK];
        data[0] = on as u8;
        self.command_ok(cmd::SET_LONG_RANGE, &data).await.map(drop)
    }

    pub async fn factory_reset(&self) -> Result<()> {
        // The device acknowledges once the reset completes, which can take seconds.
        self.send(&protocol::packet(cmd::CLEAR_SETTING, 0, &[])).await?;
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            if let Some(r) = self.recv(deadline).await? {
                if r[0] == cmd::CLEAR_SETTING {
                    return Ok(());
                }
            }
        }
        bail!("no reset acknowledgement")
    }

    pub async fn read_flash(&self, start: u16, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let address = start + out.len() as u16;
            let n = (len - out.len()).min(CHUNK);
            match self.transact(&protocol::read_packet(address, n as u8)).await? {
                Reply::Ok(r) => out.extend_from_slice(&r[5..5 + n]),
                Reply::Rejected => bail!("flash read rejected at {address}"),
            }
        }
        Ok(out)
    }

    pub async fn write_flash(&self, start: u16, data: &[u8]) -> Result<()> {
        for (i, chunk) in data.chunks(CHUNK).enumerate() {
            let address = start + (i * CHUNK) as u16;
            if self.transact(&protocol::packet(cmd::WRITE_FLASH, address, chunk)).await?
                == Reply::Rejected
            {
                bail!("flash write rejected at {address}");
            }
        }
        Ok(())
    }

    /// Writes only the 10-byte chunks that differ from what is on the device.
    pub async fn write_diff(&self, start: u16, data: &[u8]) -> Result<()> {
        let current = self.read_flash(start, data.len()).await?;
        for (i, (new, old)) in data.chunks(CHUNK).zip(current.chunks(CHUNK)).enumerate() {
            if new != old {
                self.write_flash(start + (i * CHUNK) as u16, new).await?;
            }
        }
        Ok(())
    }

    /// Puts the receiver in pairing mode for mice with customer id `cid`.
    pub async fn enter_pair(&self, cid: u8) -> Result<()> {
        match self.transact(&protocol::pair_packet(cid)).await? {
            Reply::Ok(_) => Ok(()),
            Reply::Rejected => bail!("receiver refused pairing mode"),
        }
    }

    /// (status, seconds left); status is one of [`protocol::pair`].
    pub async fn pair_state(&self) -> Result<(u8, u8)> {
        let r = self.command_ok(cmd::GET_PAIR_STATE, &[]).await?;
        Ok((r[5], r[6]))
    }

    /// Non-blocking check for a `StatusChanged` notification (sent when settings
    /// change on the mouse itself, e.g. the DPI or profile button). Returns the
    /// flag byte: 1 DPI, 2 polling rate, 4 profile, 64 battery.
    pub async fn poll_status(&self) -> Result<Option<u8>> {
        let mut flags = None;
        while let Some(r) = self.recv(Instant::now()).await? {
            if r[0] == cmd::STATUS_CHANGED && r[1] == 0 {
                flags = Some(flags.unwrap_or(0) | r[5]);
            }
        }
        Ok(flags)
    }

    /// Single-byte setting stored as `[v, 0x55 - v]`.
    pub async fn write_setting(&self, address: u16, v: u8) -> Result<()> {
        self.write_flash(address, &[v, 0x55u8.wrapping_sub(v)]).await
    }
}
