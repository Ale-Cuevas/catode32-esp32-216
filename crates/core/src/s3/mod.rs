//! Waveshare ESP32-S3-Touch-AMOLED-2.16 backend (`s3` feature).
//!
//! The game still renders 1bpp 128x64. `catode32-amoled216` scales it onto
//! the 480x480 CO5300 panel and turns CST9220 touches into the 8 buttons.
//! This module only wires that crate to esp-hal: the QSPI bus (`bus`), pin
//! and peripheral setup (`board`), the touch/key "virtual pins"
//! (`input`), and the AXP2101 battery state (`pmu`). The generic seams (`board.rs`, `input.rs`, `render.rs`,
//! `led.rs`, `game.rs`) pick these up behind `#[cfg(feature = "s3")]`.
//! Pins and protocol facts: `docs/board-notes.md`.

pub mod board;
mod bus;
pub mod input;
pub mod pmu;
