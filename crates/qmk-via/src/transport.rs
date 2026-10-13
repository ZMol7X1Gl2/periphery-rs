//! VIA raw-HID transport: 32-byte reports without a report ID on usage page
//! 0xFF60 / usage 0x61. Every request is answered by a report that echoes the
//! command byte (and, for grouped commands, the sub-command).

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use hidapi::HidDevice;

pub const REPORT_LEN: usize = 32;
const TIMEOUT: Duration = Duration::from_millis(300);
/// Firmware reply to a command it does not implement.
const UNHANDLED: u8 = 0xFF;

pub type Report = [u8; REPORT_LEN];

pub struct Raw {
    dev: HidDevice,
}

impl Raw {
    pub fn new(dev: HidDevice) -> Self {
        Raw { dev }
    }

    /// How many leading bytes of a reply must equal the request.
    fn echo_len(req: &[u8]) -> usize {
        match req[0] {
            // custom value get/set: command, channel, value id
            0x07 | 0x08 => 3,
            // keymap buffer read: command, offset hi, offset lo
            0x12 => 3,
            // keycode set: command, layer, row, col
            0x05 | 0x04 => 4,
            // Keychron grouped commands
            0xA7 | 0xA8 | 0xA9 | 0xAB => 2,
            _ => 1,
        }
        .min(req.len())
    }

    fn write(&self, req: &[u8]) -> Result<()> {
        if req.len() > REPORT_LEN {
            bail!("request too long");
        }
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..1 + req.len()].copy_from_slice(req);
        self.dev.write(&buf).context("HID write")?;
        Ok(())
    }

    /// Sends a request and waits for its reply. `None` means the firmware does
    /// not support it (no reply, or the "unhandled" marker).
    pub fn try_query(&self, req: &[u8]) -> Result<Option<Report>> {
        let echo = Self::echo_len(req);
        self.write(req)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let mut r = [0u8; 64];
            let n = self.dev.read_timeout(&mut r, left.as_millis() as i32).context("HID read")?;
            if n < REPORT_LEN {
                continue;
            }
            // QMK overwrites only the command byte when it doesn't know a command.
            if r[0] == UNHANDLED {
                return Ok(None);
            }
            if r[..echo] == req[..echo] {
                return Ok(Some(r[..REPORT_LEN].try_into().unwrap()));
            }
        }
    }

    pub fn query(&self, req: &[u8]) -> Result<Report> {
        for _ in 0..3 {
            if let Some(r) = self.try_query(req)? {
                return Ok(r);
            }
        }
        bail!("no reply to command {:#04x}", req[0])
    }

    /// Discards any pending input reports.
    pub fn drain(&self) {
        let mut r = [0u8; 64];
        while matches!(self.dev.read_timeout(&mut r, 0), Ok(n) if n > 0) {}
    }
}
