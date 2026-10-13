//! QMK/VIA keyboards over raw HID (usage page 0xFF60, usage 0x61), plus the
//! Keychron firmware extensions (commands 0xA0-0xAC).
//!
//! - [`transport`]: 32-byte request/reply framing
//! - [`keycodes`]: QMK keycode labels (VIA protocol 12+ numbering)
//! - [`definition`]: keyboard layout definitions (Keychron's are downloaded)
//! - [`menus`]: VIA custom menus (lighting) incl. `showIf`
//! - [`macros`]: dynamic macro buffer encoding
//! - [`keychron`]: Keychron extensions: magnetic switches, per-key RGB, sleep…

pub mod definition;
pub mod keychron;
pub mod keycodes;
pub mod macros;
pub mod menus;
pub mod transport;

pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE: u16 = 0x61;

/// VIA command ids.
pub mod cmd {
    pub const GET_PROTOCOL_VERSION: u8 = 0x01;
    pub const GET_KEYBOARD_VALUE: u8 = 0x02;
    pub const SET_KEYCODE: u8 = 0x05;
    pub const CUSTOM_SET_VALUE: u8 = 0x07;
    pub const CUSTOM_GET_VALUE: u8 = 0x08;
    pub const CUSTOM_SAVE: u8 = 0x09;
    pub const MACRO_GET_COUNT: u8 = 0x0C;
    pub const MACRO_GET_BUFFER_SIZE: u8 = 0x0D;
    pub const MACRO_GET_BUFFER: u8 = 0x0E;
    pub const MACRO_SET_BUFFER: u8 = 0x0F;
    pub const GET_LAYER_COUNT: u8 = 0x11;
    pub const KEYMAP_GET_BUFFER: u8 = 0x12;
}
