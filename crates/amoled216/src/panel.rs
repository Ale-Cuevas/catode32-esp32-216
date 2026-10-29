//! The game's view of the AMOLED: push the 1bpp frame into the viewport, the
//! touch controls into their band and the battery into the status bar,
//! streaming through one strip buffer (no 450 KB framebuffer).

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;

use crate::co5300::{Co5300, Error, QspiBus};
use crate::controls::{self, Theme};
use crate::layout::{Offset, CONTROLS_AREA, FRAME_AREA, SCREEN, SCREEN_W, STATUS_AREA};
use crate::scale::{self, Palette, FRAME_ROW_BYTES, SRC_BYTES, STRIP_BYTES};
use crate::status::{self, Power};

/// Screen rows of the controls band that fit in one strip buffer.
const CONTROL_ROWS_PER_STRIP: u16 = (STRIP_BYTES / (SCREEN_W as usize * 2)) as u16;
/// Rows of `FRAME_AREA` that fit in one strip buffer.
const FRAME_ROWS_PER_STRIP: u16 = (STRIP_BYTES / FRAME_ROW_BYTES) as u16;

pub struct Amoled<B, RST> {
    lcd: Co5300<B, RST>,
    strip: [u8; STRIP_BYTES],
    palette: Palette,
    /// Burn-in shift and controls dimming (`burnin::Look`).
    offset: Offset,
    dim: bool,
    /// Pressed mask the controls band currently shows, `None` = not drawn
    /// (or the colors changed), so the next `draw_controls` redraws it.
    controls_shown: Option<u8>,
    power: Option<Power>,
    /// The status bar needs a redraw.
    status_dirty: bool,
}

impl<B: QspiBus, RST: OutputPin> Amoled<B, RST> {
    pub fn new(lcd: Co5300<B, RST>, palette: Palette) -> Self {
        Self {
            lcd,
            strip: [0; STRIP_BYTES],
            palette,
            offset: Offset::ZERO,
            dim: false,
            controls_shown: None,
            power: None,
            status_dirty: true,
        }
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
        self.status_dirty = true;
        self.lcd.fill(SCREEN, color, &mut self.strip)
    }

    pub fn set_palette(&mut self, palette: Palette) {
        if palette != self.palette {
            self.palette = palette;
            self.controls_shown = None;
            self.status_dirty = true;
        }
    }

    /// Burn-in shift and controls dimming. Takes effect on the next flush;
    /// the controls redraw if either changed.
    pub fn set_look(&mut self, offset: Offset, dim: bool) {
        if (offset, dim) != (self.offset, self.dim) {
            self.offset = offset;
            self.dim = dim;
            self.controls_shown = None;
            self.status_dirty = true;
        }
    }

    /// Battery state for the status bar; redrawn on the next
    /// `draw_status` if it changed.
    pub fn set_power(&mut self, power: Option<Power>) {
        if power != self.power {
            self.power = power;
            self.status_dirty = true;
        }
    }

    /// Redraw the status bar if anything it shows changed. Returns whether
    /// anything was sent.
    pub fn draw_status(&mut self) -> Result<bool, Error<B::Error, RST::Error>> {
        if !self.status_dirty {
            return Ok(false);
        }
        let (theme, offset, power) = (self.theme(), self.offset, self.power);
        let mut y = STATUS_AREA.y;
        let end = STATUS_AREA.bottom();
        self.lcd.write_area(STATUS_AREA, &mut self.strip, |buf| {
            if y >= end {
                return 0;
            }
            let rows = CONTROL_ROWS_PER_STRIP.min(end - y);
            let n = status::render_rows(y, rows, power, theme, offset, buf);
            y += rows;
            n
        })?;
        self.status_dirty = false;
        Ok(true)
    }

    fn theme(&self) -> Theme {
        if self.dim {
            Theme::dimmed(self.palette)
        } else {
            Theme::from_palette(self.palette)
        }
    }

    /// Scale the 128x64 frame 3x into the viewport, moved by the burn-in
    /// offset. Streams all of `FRAME_AREA`, so the margin is repainted too.
    pub fn flush_frame(&mut self, src: &[u8; SRC_BYTES], invert: bool) -> Result<(), Error<B::Error, RST::Error>> {
        let (palette, offset) = (self.palette, self.offset);
        let mut y = 0u16;
        self.lcd.write_area(FRAME_AREA, &mut self.strip, |buf| {
            if y >= FRAME_AREA.h {
                return 0;
            }
            let rows = FRAME_ROWS_PER_STRIP.min(FRAME_AREA.h - y);
            let n = scale::frame_rows(src, y, rows, offset, palette, invert, buf);
            y += rows;
            n
        })
    }

    /// Redraw the controls band if `pressed` differs from what it shows.
    /// Returns whether anything was sent.
    pub fn draw_controls(&mut self, pressed: u8) -> Result<bool, Error<B::Error, RST::Error>> {
        if self.controls_shown == Some(pressed) {
            return Ok(false);
        }
        let (theme, offset) = (self.theme(), self.offset);
        let mut y = CONTROLS_AREA.y;
        let end = CONTROLS_AREA.bottom();
        self.lcd.write_area(CONTROLS_AREA, &mut self.strip, |buf| {
            if y >= end {
                return 0;
            }
            let rows = CONTROL_ROWS_PER_STRIP.min(end - y);
            let n = controls::render_rows(y, rows, pressed, theme, offset, buf);
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
        a.set_look(Offset::new(-2, 4), false);
        a.flush_frame(&src, false).unwrap();

        let mut expected = vec![0u8; FRAME_AREA.h as usize * FRAME_ROW_BYTES];
        scale::frame_rows(&src, 0, FRAME_AREA.h, Offset::new(-2, 4), Palette::DEFAULT, false, &mut expected);
        assert_eq!(log.borrow()[0], reg(0x2A, &[0, 44, 1, 179]));
        assert_eq!(log.borrow()[1], reg(0x2B, &[0, 30, 0, 229]));
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
        controls::render_rows(
            CONTROLS_AREA.y,
            CONTROLS_AREA.h,
            0b0001_0001,
            Theme::from_palette(Palette::DEFAULT),
            Offset::ZERO,
            &mut expected,
        );
        assert_eq!(pixels(a), expected);
    }

    #[test]
    fn look_change_redraws_controls_dimmed_and_shifted() {
        let (mut a, _) = amoled();
        a.draw_controls(0).unwrap();
        a.set_look(Offset::ZERO, false);
        assert!(!a.draw_controls(0).unwrap(), "same look: no redraw");

        let (mut a, _) = amoled();
        a.draw_controls(0).unwrap();
        a.lcd().bus_mut().pixels.clear();
        a.set_look(Offset::new(4, -4), true);
        assert!(a.draw_controls(0).unwrap());
        let mut expected = vec![0u8; CONTROLS_AREA.w as usize * CONTROLS_AREA.h as usize * 2];
        let theme = Theme::dimmed(Palette::DEFAULT);
        controls::render_rows(CONTROLS_AREA.y, CONTROLS_AREA.h, 0, theme, Offset::new(4, -4), &mut expected);
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
    fn status_redraws_only_on_change() {
        let (mut a, log) = amoled();
        assert!(a.draw_status().unwrap(), "first draw");
        assert_eq!(log.borrow()[0], reg(0x2A, &[0, 0, 1, 0xDF]));
        assert_eq!(log.borrow()[1], reg(0x2B, &[0, 0, 0, 29]));
        assert!(!a.draw_status().unwrap());

        let p = Some(Power { battery_pct: Some(50), charging: false, usb: false });
        a.set_power(p);
        assert!(a.draw_status().unwrap());
        a.set_power(p);
        assert!(!a.draw_status().unwrap());
        a.set_look(Offset::new(2, 2), false);
        assert!(a.draw_status().unwrap(), "shift redraws");
        a.set_look(Offset::new(2, 2), true);
        assert!(a.draw_status().unwrap(), "dimming redraws");
        a.clear().unwrap();
        assert!(a.draw_status().unwrap(), "clear redraws");
    }

    #[test]
    fn status_matches_rasterizer() {
        let (mut a, _) = amoled();
        let p = Some(Power { battery_pct: Some(73), charging: true, usb: true });
        a.set_power(p);
        a.draw_status().unwrap();
        let mut expected = vec![0u8; STATUS_AREA.w as usize * STATUS_AREA.h as usize * 2];
        status::render_rows(0, STATUS_AREA.h, p, Theme::from_palette(Palette::DEFAULT), Offset::ZERO, &mut expected);
        assert_eq!(pixels(a), expected);
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
