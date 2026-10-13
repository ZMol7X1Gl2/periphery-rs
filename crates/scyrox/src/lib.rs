//! Scyrox mouse protocol: HID transport, settings flash map, DPI and button
//! encodings, combo keys, macros, pairing and profiles.
//!
//! ```no_run
//! let api = hidapi::HidApi::new()?;
//! let mouse = scyrox::device::Mouse::open(&api)?;
//! println!("{:?}", mouse.snapshot()?);
//! # anyhow::Ok(())
//! ```

pub mod config;
pub mod device;
pub mod keys;
pub mod protocol;
