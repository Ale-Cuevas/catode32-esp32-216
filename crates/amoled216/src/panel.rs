//! The game's view of the AMOLED: push the 1bpp frame into the viewport and
//! the touch controls into their band, streaming through one strip buffer
//! (no 450 KB framebuffer).

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;

use crate::co5300::{Co5300, Error, QspiBus};
use crate::controls::{self, Theme};
use crate::layout::{CONTROLS_AREA, SCREEN, SCREEN_W, SRC_H, VIEWPORT};
use crate::scale::{self, Palette, SRC_BYTES, STRIP_BYTES, STRIP_SRC_ROWS};

/// Screen rows of the controls band that fit in one strip buffer.
const CONTROL_ROWS_PER_STRIP: u16 = (STRIP_BYTES / (SCREEN_W as usize * 2)) as u16;

pub struct Amoled<B, RST> {
    lcd: Co5300<B, RST>,
    strip: [u8; STRIP_BYTES],
    palette: Palette,
    /// Pressed mask the controls band currently shows, `None` = not drawn
    /// (or the colors changed), so the next `draw_controls` redraws it.
    controls_shown: Option<u8>,
}

impl<B: QspiBus, RST: OutputPin> Amoled<B, RST> {
    pub fn new(lcd: Co5300<B, RST>, palette: Palette) -> Self {
        Self { lcd, strip: [0; STRIP_BYTES], palette, controls_shown: None }
    }

    pub fn release(self) -> Co5300<B, RST> {
        self.lcd
    }

    pub fn lcd(&mut self) -> &mut Co5300<B, RST> {
        &mut self.lcd
    }

    /// Reset + init the panel, then clear it to the background color.
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), Error<B::Error, RST::Error>> {
        self.lcd.init(delay)?;
        self.clear()
    }

    /// Whole screen to the background color; controls must be redrawn.
    pub fn clear(&mut self) -> Result<(), Error<B::Error, RST::Error>> {
        self.fill_screen(self.palette.bg)
    }

    /// Whole screen to one RGB565 color (bring-up color tests).
    pub fn fill_screen(&mut self, color: u16) -> Result<(), Error<B::Error, RST::Error>> {
        self.controls_shown = None;
        self.lcd.fill(SCREEN, color, &mut self.strip)
    }

    pub fn set_palette(&mut self, palette: Palette) {
        if palette != self.palette {
            self.palette = palette;
            self.controls_shown = None;
        }
    }

    /// Scale the 128x64 frame 3x into the viewport.
    pub fn flush_frame(&mut self, src: &[u8; SRC_BYTES], invert: bool) -> Result<(), Error<B::Error, RST::Error>> {
        let palette = self.palette;
        let mut row = 0usize;
        self.lcd.write_area(VIEWPORT, &mut self.strip, |buf| {
            if row >= SRC_H as usize {
                return 0;
            }
            let n = scale::scale_strip(src, row, STRIP_SRC_ROWS, palette, invert, buf);
            row += STRIP_SRC_ROWS;
            n
        })
    }

    /// Redraw the controls band if `pressed` differs from what it shows.
    /// Returns whether anything was sent.
    pub fn draw_controls(&mut self, pressed: u8) -> Result<bool, Error<B::Error, RST::Error>> {
        if self.controls_shown == Some(pressed) {
            return Ok(false);
        }
        let theme = Theme::from_palette(self.palette);
        let mut y = CONTROLS_AREA.y;
        let end = CONTROLS_AREA.bottom();
        self.lcd.write_area(CONTROLS_AREA, &mut self.strip, |buf| {
            if y >= end {
                return 0;
            }
            let rows = CONTROL_ROWS_PER_STRIP.min(end - y);
            let n = controls::render_rows(y, rows, pressed, theme, buf);
            y += rows;
            n
        })?;
        self.controls_shown = Some(pressed);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co5300::mock::{lcd, reg, Bus, Pin, Txn};
    use crate::layout::SCREEN_H;

    fn amoled() -> (Amoled<Bus, Pin>, crate::co5300::mock::Log) {
        let (lcd, _, log) = lcd();
        (Amoled::new(lcd, Palette::DEFAULT), log)
    }

    fn pixels(a: Amoled<Bus, Pin>) -> Vec<u8> {
        a.release().release().0.pixels
    }

    #[test]
    fn flush_frame_sends_exactly_the_scaled_frame() {
        let (mut a, log) = amoled();
        let mut src = [0u8; SRC_BYTES];
        src[0] = 0x80;
        src[SRC_BYTES - 1] = 0x01;
        a.flush_frame(&src, false).unwrap();

        let mut expected = vec![0u8; scale::strip_bytes(SRC_H as usize)];
        scale::scale_strip(&src, 0, SRC_H as usize, Palette::DEFAULT, false, &mut expected);
        assert_eq!(log.borrow()[0], reg(0x2A, &[0, 48, 1, 175]));
        assert_eq!(pixels(a), expected);
    }

    #[test]
    fn controls_match_rasterizer_and_skip_when_unchanged() {
        let (mut a, log) = amoled();
        assert!(a.draw_controls(0b0001_0001).unwrap());
        let n = log.borrow().len();
        assert!(!a.draw_controls(0b0001_0001).unwrap());
        assert_eq!(log.borrow().len(), n, "unchanged mask must not touch the bus");

        let mut expected = vec![0u8; CONTROLS_AREA.w as usize * CONTROLS_AREA.h as usize * 2];
        controls::render_rows(CONTROLS_AREA.y, CONTROLS_AREA.h, 0b0001_0001, Theme::from_palette(Palette::DEFAULT), &mut expected);
        assert_eq!(pixels(a), expected);
    }

    #[test]
    fn clear_and_palette_change_force_controls_redraw() {
        let (mut a, _) = amoled();
        a.draw_controls(0).unwrap();
        a.clear().unwrap();
        assert!(a.draw_controls(0).unwrap());
        a.set_palette(Palette::DEFAULT);
        assert!(!a.draw_controls(0).unwrap());
        a.set_palette(Palette { fg: 0xFFFF, bg: 0 });
        assert!(a.draw_controls(0).unwrap());
    }

    #[test]
    fn clear_covers_the_whole_screen() {
        let (mut a, log) = amoled();
        a.clear().unwrap();
        assert_eq!(log.borrow()[0], reg(0x2A, &[0, 0, 1, 0xDF]));
        assert_eq!(log.borrow()[1], reg(0x2B, &[0, 0, 1, 0xDF]));
        assert!(matches!(log.borrow()[3], Txn::QuadBegin(0x32, 0x3C00, _)));
        assert_eq!(pixels(a).len(), SCREEN_W as usize * SCREEN_H as usize * 2);
    }
}
