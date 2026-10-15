//! AULA keyboard protocol for the wired F75 and its siblings (F87, F99,
//! H108…), which all enumerate as USB `258A:010C` and are told apart by a
//! six-byte identity.
//!
//! The vendor's Windows driver (`OemDrv.exe`, class `CDevG5KB`) talks to them
//! with 520-byte feature reports on the vendor collection (usage page
//! `0xFF00`). The wire format and the lighting page layout come from two
//! independent re-implementations that were checked on an F75:
//! [AulaSofwareLinux](https://github.com/leisiji/AulaSofwareLinux) (MIT,
//! recovered from `OemDrv.exe`) and
//! [aula-f75](https://github.com/RustamSheoran/aula-f75) (WebHID).
//!
//! The 2.4 GHz dongle (`3554:FA09`) and Bluetooth speak a different,
//! 20-byte transport and are not implemented.

pub mod lighting;
pub mod models;
pub mod transport;

pub use lighting::LightingPage;
pub use models::{Effect, Model};
pub use transport::Keyboard;
