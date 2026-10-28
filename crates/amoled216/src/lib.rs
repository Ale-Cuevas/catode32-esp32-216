//! Hardware-free pieces of the ESP32-S3-Touch-AMOLED-2.16 backend.
//!
//! The game keeps drawing into its 1bpp 128x64 framebuffer. This crate
//! decides where that frame and the touch controls go on the 480x480 panel
//! (`layout`), turns the frame into RGB565 strips for the CO5300 (`scale`),
//! and turns touch points into the 8-button mask `input::Buttons` expects
//! (`touch`). No esp-hal here, so it is unit-tested on the host and shared by
//! the desktop sim and the firmware.
#![cfg_attr(not(test), no_std)]

pub mod layout;
pub mod scale;
pub mod touch;
