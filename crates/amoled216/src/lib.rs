//! Hardware-free pieces of the ESP32-S3-Touch-AMOLED-2.16 backend.
//!
//! The game keeps drawing into its 1bpp 128x64 framebuffer. This crate
//! decides where that frame and the touch controls go on the 480x480 panel
//! (`layout`), turns the frame into RGB565 strips for the CO5300 (`scale`),
//! and turns touch points into the 8-button mask `input::Buttons` expects
//! (`touch`). `controls` rasterizes the on-screen buttons, `co5300` drives
//! the panel through a small `QspiBus` trait, and `panel` streams frames and
//! controls to it. `display` is the 1bpp `DrawTarget` core renders into on
//! the S3; `burnin` decides when it dims and how far everything is shifted.
//! `cst9220` reads touch points and `axp2101` the battery and USB
//! state over `embedded_hal::i2c`.
//! No esp-hal here, so it is unit-tested on the host and shared by
//! the desktop sim and the firmware.
#![cfg_attr(not(test), no_std)]

pub mod axp2101;
pub mod burnin;
pub mod co5300;
pub mod controls;
pub mod cst9220;
pub mod display;
pub mod layout;
pub mod panel;
pub mod scale;
pub mod touch;

use burnin::Look;
use controls::Theme;
use layout::{FRAME_AREA, SCREEN_H, SCREEN_W};
use scale::{Palette, FRAME_ROW_BYTES, SRC_BYTES};

/// Bytes in a full 480x480 RGB565 frame.
pub const SCREEN_BYTES: usize = SCREEN_W as usize * SCREEN_H as usize * 2;

/// Render the whole panel (controls plus the scaled game frame) as
/// big-endian RGB565, with `look`'s shift and controls dimming (panel
/// brightness is the caller's to emulate). For the desktop sim and
/// screenshots. The firmware streams the same pixels in strips instead of
/// holding a 450 KB frame.
pub fn compose_screen(src: &[u8; SRC_BYTES], pressed: u8, palette: Palette, invert: bool, look: Look, out: &mut [u8]) {
    assert!(out.len() >= SCREEN_BYTES, "screen buffer too small");
    let theme = if look.controls_dim { Theme::dimmed(palette) } else { Theme::from_palette(palette) };
    controls::render_rows(0, SCREEN_H, pressed, theme, look.offset, out);

    let mut frame = [0u8; FRAME_ROW_BYTES];
    for y in 0..FRAME_AREA.h {
        scale::frame_rows(src, y, 1, look.offset, palette, invert, &mut frame);
        let start = ((FRAME_AREA.y + y) as usize * SCREEN_W as usize + FRAME_AREA.x as usize) * 2;
        out[start..start + FRAME_ROW_BYTES].copy_from_slice(&frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::VIEWPORT;

    #[test]
    fn compose_places_frame_in_viewport() {
        let mut src = [0u8; SRC_BYTES];
        src[0] = 0x80; // pixel (0, 0)
        let pal = Palette { fg: 0xF800, bg: 0x0000 };
        let mut out = vec![0u8; SCREEN_BYTES];
        compose_screen(&src, 0, pal, false, Look::PLAIN, &mut out);
        let px = |x: usize, y: usize| u16::from_be_bytes([out[(y * 480 + x) * 2], out[(y * 480 + x) * 2 + 1]]);
        let (vx, vy) = (VIEWPORT.x as usize, VIEWPORT.y as usize);
        assert_eq!(px(vx, vy), 0xF800);
        assert_eq!(px(vx + 2, vy + 2), 0xF800);
        assert_eq!(px(vx + 3, vy), 0x0000);
        assert_eq!(px(vx - 1, vy), 0x0000);
        assert_eq!(px(vx, vy - 1), 0x0000);
    }
}
