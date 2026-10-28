//! Hardware-free pieces of the ESP32-S3-Touch-AMOLED-2.16 backend.
//!
//! The game keeps drawing into its 1bpp 128x64 framebuffer. This crate
//! decides where that frame and the touch controls go on the 480x480 panel
//! (`layout`), turns the frame into RGB565 strips for the CO5300 (`scale`),
//! and turns touch points into the 8-button mask `input::Buttons` expects
//! (`touch`). `controls` rasterizes the on-screen buttons. No esp-hal here, so it is unit-tested on the host and shared by
//! the desktop sim and the firmware.
#![cfg_attr(not(test), no_std)]

pub mod controls;
pub mod layout;
pub mod scale;
pub mod touch;

use layout::{SCREEN_H, SCREEN_W, VIEWPORT};
use scale::{Palette, SRC_BYTES};

/// Bytes in a full 480x480 RGB565 frame.
pub const SCREEN_BYTES: usize = SCREEN_W as usize * SCREEN_H as usize * 2;

/// Render the whole panel (controls plus the scaled game frame) as
/// big-endian RGB565. For the desktop sim and screenshots. The firmware
/// streams the same pixels in strips instead of holding a 450 KB frame.
pub fn compose_screen(src: &[u8; SRC_BYTES], pressed: u8, palette: Palette, invert: bool, out: &mut [u8]) {
    assert!(out.len() >= SCREEN_BYTES, "screen buffer too small");
    controls::render_rows(0, SCREEN_H, pressed, controls::Theme::from_palette(palette), out);

    let row_bytes = VIEWPORT.w as usize * 2;
    let mut strip = [0u8; scale::STRIP_BYTES];
    for first in (0..layout::SRC_H as usize).step_by(scale::STRIP_SRC_ROWS) {
        let n = scale::scale_strip(src, first, scale::STRIP_SRC_ROWS, palette, invert, &mut strip);
        for (r, row) in strip[..n].chunks_exact(row_bytes).enumerate() {
            let y = VIEWPORT.y as usize + first * layout::SCALE as usize + r;
            let start = (y * SCREEN_W as usize + VIEWPORT.x as usize) * 2;
            out[start..start + row_bytes].copy_from_slice(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_places_frame_in_viewport() {
        let mut src = [0u8; SRC_BYTES];
        src[0] = 0x80; // pixel (0, 0)
        let pal = Palette { fg: 0xF800, bg: 0x0000 };
        let mut out = vec![0u8; SCREEN_BYTES];
        compose_screen(&src, 0, pal, false, &mut out);
        let px = |x: usize, y: usize| u16::from_be_bytes([out[(y * 480 + x) * 2], out[(y * 480 + x) * 2 + 1]]);
        let (vx, vy) = (VIEWPORT.x as usize, VIEWPORT.y as usize);
        assert_eq!(px(vx, vy), 0xF800);
        assert_eq!(px(vx + 2, vy + 2), 0xF800);
        assert_eq!(px(vx + 3, vy), 0x0000);
        assert_eq!(px(vx - 1, vy), 0x0000);
        assert_eq!(px(vx, vy - 1), 0x0000);
    }
}
