//! ROCCAT mouse protocol. Only the Kone Aimo family is implemented.
//!
//! Sources, none of them from ROCCAT:
//! - [phiph-s/roccat-kone-aimo-linux](https://github.com/phiph-s/roccat-kone-aimo-linux)
//!   (`src/aimo/device.py`, `buttons.py`): settings and button reports,
//!   checked on a Kone Aimo 16K (Remastered) by its author.
//! - [libratbag PR #1883](https://github.com/libratbag/libratbag/pull/1883)
//!   (`src/driver-roccat-kone-aimo.c`): the same reports as C structs, the
//!   ready-status values and the activate command captured from ROCCAT Swarm.
//! - [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB)
//!   (`RoccatKoneAimoController`): USB ids and the HID collection
//!   (interface 0, usage page `0x0B`).

pub mod keys;
pub mod kone_aimo;

pub use kone_aimo::{Buttons, KoneAimo, Model, Settings};
