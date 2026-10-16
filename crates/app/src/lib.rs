//! Shared setup for the Periphery GUI and CLI, and the GUI itself (used by the
//! desktop app and the web app).

use periphery_core::Driver;

mod mouse;
mod mouse_diagram;
mod platform;
pub mod request;
pub mod ui;
pub mod worker;

/// Every driver compiled into the app. Order decides listing order.
pub fn drivers() -> [&'static dyn Driver; 5] {
    [
        &periphery_scyrox::ScyroxDriver,
        &periphery_via::ViaDriver,
        &periphery_logitech::LogitechDriver,
        &periphery_rongyuan::RongyuanDriver,
        &periphery_roccat::RoccatDriver,
    ]
}
