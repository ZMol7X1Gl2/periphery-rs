//! Feature-report transport (web driver classes `H4` for a wired device and
//! `PF` for a receiver).
//!
//! Every command is one 64-byte feature report without a report ID, sent on
//! the vendor collection (usage page `0xFFFF`, usage 2, interface 2). The
//! answer is fetched with a get-feature-report on the same collection about
//! 10 ms later. Commands carry a checksum in byte 7:
//! `0xFF - (bytes[0..7].sum() & 0xFF)` (the driver's "Bit7" mode). Replies to
//! reads echo the command in byte 0 and carry their data from byte 8.
//!
//! A 2.4 GHz receiver relays commands to the mouse. Its own commands carry no
//! checksum:
//!
//! | send          | meaning                                            |
//! |---------------|----------------------------------------------------|
//! | `F7`          | status (reply below)                               |
//! | `F6 05`       | route the next commands to the mouse (`0A` = keyboard) |
//! | `FC`          | fetch the reply the mouse has sent                 |
//!
//! `F7` reply: byte 0 = a reply is waiting, 1 = keyboard battery %, 2 = mouse
//! battery %, 3 / 4 = keyboard / mouse offline (0 = online), 5 = ready to
//! send, 6 = which of the two the reply describes (bit 0 keyboard, bit 1
//! mouse), 8 = receiver in bootloader.
//!
//! Relayed command: poll `F7` (100 ms apart, up to 5 times) until "ready to
//! send", route with `F6 05` once, then send the command. Relayed reply: poll
//! `F7` until "reply waiting", send `FC`, then get the feature report.

use std::cell::Cell;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hidapi::HidDevice;

pub const REPORT_LEN: usize = 64;
pub type Report = [u8; REPORT_LEN];

const SEND_DELAY: Duration = Duration::from_millis(10);
const READ_DELAY: Duration = Duration::from_millis(10);
const POLL_DELAY: Duration = Duration::from_millis(100);
const POLLS: usize = 5;

mod receiver_cmd {
    pub const STATUS: u8 = 0xF7;
    pub const ROUTE: u8 = 0xF6;
    pub const ROUTE_MOUSE: u8 = 0x05;
    pub const FETCH: u8 = 0xFC;
}

/// Pads `cmd` to a report and stores the byte-7 checksum.
pub fn frame(cmd: &[u8]) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[..cmd.len()].copy_from_slice(cmd);
    r[7] = checksum(&r);
    r
}

pub fn checksum(r: &Report) -> u8 {
    let sum = r[..7].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    0xFF - sum
}

/// What a receiver reports about its mouse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseStatus {
    /// The receiver knows a mouse (its status byte describes one).
    pub paired: bool,
    pub online: bool,
    pub battery: u8,
    can_send: bool,
    can_read: bool,
}

impl MouseStatus {
    pub fn parse(r: &[u8]) -> Self {
        if r[6] & 2 == 0 {
            return MouseStatus::default();
        }
        MouseStatus {
            paired: true,
            online: r[4] == 0,
            battery: r[2],
            can_send: r[5] == 1,
            can_read: r[0] == 1,
        }
    }
}

pub struct Link {
    dev: HidDevice,
    receiver: bool,
    routed: Cell<bool>,
}

impl Link {
    pub fn new(dev: HidDevice, receiver: bool) -> Self {
        Link { dev, receiver, routed: Cell::new(false) }
    }

    pub fn is_receiver(&self) -> bool {
        self.receiver
    }

    fn set_feature(&self, data: &[u8]) -> Result<()> {
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..1 + data.len()].copy_from_slice(data);
        self.dev.send_feature_report(&buf).context("HID set feature report")
    }

    fn get_feature(&self) -> Result<Report> {
        let mut buf = [0u8; REPORT_LEN + 1];
        let n = self.dev.get_feature_report(&mut buf).context("HID get feature report")?;
        if n < 2 {
            bail!("empty feature report");
        }
        let mut r = [0u8; REPORT_LEN];
        let data = &buf[1..n.min(buf.len())];
        r[..data.len()].copy_from_slice(data);
        Ok(r)
    }

    /// Receiver status (`F7`). Only valid on a receiver.
    pub fn receiver_status(&self) -> Result<MouseStatus> {
        sleep(SEND_DELAY);
        self.set_feature(&[receiver_cmd::STATUS])?;
        sleep(READ_DELAY);
        Ok(MouseStatus::parse(&self.get_feature()?))
    }

    fn wait_receiver(&self, ready: impl Fn(&MouseStatus) -> bool, what: &str) -> Result<()> {
        for _ in 0..POLLS {
            sleep(POLL_DELAY);
            let s = self.receiver_status()?;
            if !s.online {
                bail!("the mouse is asleep or switched off — move it to wake it up");
            }
            if ready(&s) {
                return Ok(());
            }
        }
        bail!("the receiver did not get {what}")
    }

    /// Sends a framed command (to the mouse, through the receiver if needed).
    pub fn send(&self, report: &Report) -> Result<()> {
        if self.receiver {
            self.wait_receiver(|s| s.can_send, "ready to send")?;
            if !self.routed.get() {
                sleep(SEND_DELAY);
                self.set_feature(&[receiver_cmd::ROUTE, receiver_cmd::ROUTE_MOUSE])?;
                self.routed.set(true);
            }
        }
        sleep(SEND_DELAY);
        self.set_feature(report)
    }

    /// Fetches the reply to the last command.
    pub fn read(&self) -> Result<Report> {
        if self.receiver {
            self.wait_receiver(|s| s.can_read, "a reply from the mouse")?;
            sleep(SEND_DELAY);
            self.set_feature(&[receiver_cmd::FETCH])?;
        }
        sleep(READ_DELAY);
        self.get_feature()
    }

    /// Sends `cmd` (framed with its checksum) and returns the reply.
    pub fn query(&self, cmd: &[u8]) -> Result<Report> {
        self.send(&frame(cmd))?;
        self.read()
    }

    /// Like [`Link::query`], but requires the reply to echo the command byte.
    pub fn query_echo(&self, cmd: &[u8]) -> Result<Report> {
        for _ in 0..3 {
            let r = self.query(cmd)?;
            if r[0] == cmd[0] {
                return Ok(r);
            }
        }
        bail!("no reply to command {:#04x}", cmd[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_checksum() {
        // GET_INFO as the web driver sends it: 8F 00 .. 00 70
        let r = frame(&[0x8F]);
        assert_eq!(r[7], 0x70);
        assert!(r[8..].iter().all(|b| *b == 0));
        let r = frame(&[0xD4, 2]);
        assert_eq!(r[7], 0xFF - 0xD6);
    }

    #[test]
    fn receiver_status_bits() {
        let mut r = [0u8; REPORT_LEN];
        r[2] = 87;
        r[5] = 1;
        r[6] = 2;
        let s = MouseStatus::parse(&r);
        assert!(s.paired && s.online && s.can_send && !s.can_read);
        assert_eq!(s.battery, 87);
        r[6] = 1; // keyboard only
        assert!(!MouseStatus::parse(&r).paired);
    }
}
