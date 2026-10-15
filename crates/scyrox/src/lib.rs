//! Scyrox mouse protocol: HID transport, settings flash map, DPI and button
//! encodings, combo keys, macros, pairing and profiles.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let api = hid_transport::Api::new().await?;
//! let mouse = scyrox::device::Mouse::open(&api).await?;
//! println!("{:?}", mouse.snapshot().await?);
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod device;
pub mod keys;
pub mod protocol;
