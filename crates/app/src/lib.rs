//! Shared setup for the Periphery GUI and CLI.

use periphery_core::Driver;

/// Every driver compiled into the app. Order decides listing order.
pub fn drivers() -> [&'static dyn Driver; 4] {
    [
        &periphery_scyrox::ScyroxDriver,
        &periphery_via::ViaDriver,
        &periphery_logitech::LogitechDriver,
        &periphery_rongyuan::RongyuanDriver,
    ]
}
