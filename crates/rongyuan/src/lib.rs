//! Rongyuan (RY) device protocol, as used by Ajazz mice and many other brands
//! that ship Rongyuan firmware. Recovered from the qmk.top web driver:
//! the current site sends unknown devices to `qmk.top/v4/`, whose bundle
//! (`js/index.f6b25fd8.js`) holds the device list and transport, and whose
//! chunk `js/21aea354.js` (`CommonMsPan1080`) holds the PAN1080 mouse
//! commands.
//!
//! Only the PAN1080 mouse family is implemented. Its 2.4 GHz receivers relay
//! the same commands (see [`transport`]).

pub mod models;
pub mod pan1080;
pub mod transport;

pub use models::{Model, Sensor};
pub use pan1080::Mouse;
