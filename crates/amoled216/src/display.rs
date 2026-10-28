//! The display object core's `Renderer` draws into on the S3: a 128x64 1bpp
//! framebuffer (the same layout as core's `desktop_fb`) that `flush` scales
//! onto the panel. It has the method set the Renderer calls on the SSD1306
//! (`clear_buffer`, `flush`, `set_invert`, `set_display_on`, `draw_iter`).

use core::convert::Infallible;

use embedded_graphics::{
    draw_target::DrawTarget, geometry::Size, pixelcolor::BinaryColor, prelude::OriginDimensions, Pixel,
};
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;

use crate::co5300::{Error, QspiBus};
use crate::layout::{SRC_H, SRC_W};
use crate::panel::Amoled;
use crate::scale::SRC_BYTES;

const ROW_BYTES: usize = SRC_W as usize / 8;

pub struct AmoledDisplay<B, RST, D> {
    panel: Amoled<B, RST>,
    delay: D,
    buf: [u8; SRC_BYTES],
    invert: bool,
    on: bool,
    /// Buttons to highlight in the controls band on the next flush.
    pressed: u8,
}

impl<B: QspiBus, RST: OutputPin, D: DelayNs> AmoledDisplay<B, RST, D> {
    /// Takes an already initialized panel (`Amoled::init`).
    pub fn new(panel: Amoled<B, RST>, delay: D) -> Self {
        Self { panel, delay, buf: [0; SRC_BYTES], invert: false, on: true, pressed: 0 }
    }

    pub fn release(self) -> (Amoled<B, RST>, D) {
        (self.panel, self.delay)
    }

    pub fn panel(&mut self) -> &mut Amoled<B, RST> {
        &mut self.panel
    }

    pub fn clear_buffer(&mut self) {
        self.buf.fill(0);
    }

    /// Send the frame, then the controls if the highlight changed. Nothing
    /// is sent while the panel is off.
    pub fn flush(&mut self) -> Result<(), Error<B::Error, RST::Error>> {
        if !self.on {
            return Ok(());
        }
        self.panel.flush_frame(&self.buf, self.invert)?;
        self.panel.draw_controls(self.pressed)?;
        Ok(())
    }

    /// Applied by the scaler (fg/bg swap) on the next flush, not by the panel.
    pub fn set_invert(&mut self, invert: bool) -> Result<(), Error<B::Error, RST::Error>> {
        self.invert = invert;
        Ok(())
    }

    /// Panel sleep / wake (about 240 ms each). Waking clears the panel so the
    /// next flush redraws everything, controls included.
    pub fn set_display_on(&mut self, on: bool) -> Result<(), Error<B::Error, RST::Error>> {
        if on == self.on {
            return Ok(());
        }
        self.panel.lcd().set_display_on(on, &mut self.delay)?;
        self.on = on;
        if on {
            self.panel.clear()?;
        }
        Ok(())
    }

    pub fn set_pressed(&mut self, mask: u8) {
        self.pressed = mask;
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn invert(&self) -> bool {
        self.invert
    }

    pub fn frame(&self) -> &[u8; SRC_BYTES] {
        &self.buf
    }

    fn set_pixel(&mut self, x: i32, y: i32, on: bool) {
        if x < 0 || y < 0 || x >= SRC_W as i32 || y >= SRC_H as i32 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        let idx = y * ROW_BYTES + x / 8;
        let mask = 0x80u8 >> (x % 8);
        if on {
            self.buf[idx] |= mask;
        } else {
            self.buf[idx] &= !mask;
        }
    }
}

impl<B, RST, D> OriginDimensions for AmoledDisplay<B, RST, D> {
    fn size(&self) -> Size {
        Size::new(SRC_W as u32, SRC_H as u32)
    }
}

impl<B: QspiBus, RST: OutputPin, D: DelayNs> DrawTarget for AmoledDisplay<B, RST, D> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            self.set_pixel(p.x, p.y, c.is_on());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::co5300::mock::{lcd, reg, Bus, Delay, Pin, Txn};
    use crate::scale::{self, Palette};
    use embedded_graphics::prelude::*;

    type Display = AmoledDisplay<Bus, Pin, Delay>;

    fn display() -> (Display, crate::co5300::mock::Log) {
        let (lcd, delay, log) = lcd();
        (AmoledDisplay::new(Amoled::new(lcd, Palette::DEFAULT), delay), log)
    }

    /// Pixel bytes the bus received since the last call.
    fn take_pixels(d: &mut Display) -> Vec<u8> {
        core::mem::take(&mut d.panel().lcd().bus_mut().pixels)
    }

    fn expected_frame(src: &[u8; SRC_BYTES], invert: bool) -> Vec<u8> {
        let mut out = vec![0u8; scale::strip_bytes(SRC_H as usize)];
        scale::scale_strip(src, 0, SRC_H as usize, Palette::DEFAULT, invert, &mut out);
        out
    }

    #[test]
    fn drawn_pixels_reach_the_panel_scaled() {
        let (mut d, _) = display();
        Pixel(Point::new(0, 0), BinaryColor::On).draw(&mut d).unwrap();
        Pixel(Point::new(127, 63), BinaryColor::On).draw(&mut d).unwrap();
        Pixel(Point::new(128, 0), BinaryColor::On).draw(&mut d).unwrap(); // off-screen: ignored
        assert_eq!(d.frame()[0], 0x80);
        assert_eq!(d.frame()[SRC_BYTES - 1], 0x01);

        d.flush().unwrap();
        let mut frame = [0u8; SRC_BYTES];
        frame[0] = 0x80;
        frame[SRC_BYTES - 1] = 0x01;
        let sent = take_pixels(&mut d);
        let viewport = expected_frame(&frame, false);
        assert_eq!(&sent[..viewport.len()], &viewport[..]);
        // The controls band follows on the first flush.
        assert!(sent.len() > viewport.len());
    }

    #[test]
    fn invert_is_applied_by_the_scaler() {
        let (mut d, _) = display();
        d.set_invert(true).unwrap();
        d.flush().unwrap();
        let sent = take_pixels(&mut d);
        let viewport = expected_frame(&[0u8; SRC_BYTES], true);
        assert_eq!(&sent[..viewport.len()], &viewport[..]);
    }

    #[test]
    fn controls_only_redraw_when_pressed_changes() {
        let (mut d, _) = display();
        let frame_len = expected_frame(&[0u8; SRC_BYTES], false).len();
        d.flush().unwrap();
        assert!(take_pixels(&mut d).len() > frame_len);
        d.flush().unwrap();
        assert_eq!(take_pixels(&mut d).len(), frame_len);
        d.set_pressed(0b0001_0000);
        d.flush().unwrap();
        assert!(take_pixels(&mut d).len() > frame_len);
    }

    #[test]
    fn off_sends_nothing_and_on_redraws_everything() {
        let (mut d, log) = display();
        d.flush().unwrap();
        take_pixels(&mut d);

        d.set_display_on(false).unwrap();
        assert!(!d.is_on());
        log.borrow_mut().clear();
        d.flush().unwrap();
        assert!(log.borrow().is_empty());
        // Already off: no second panel command.
        d.set_display_on(false).unwrap();
        assert!(log.borrow().is_empty());

        d.set_display_on(true).unwrap();
        assert_eq!(log.borrow()[0], reg(0x29, &[]));
        assert_eq!(log.borrow()[1], Txn::DelayMs(120));
        assert_eq!(log.borrow()[2], reg(0x11, &[]));
        take_pixels(&mut d); // the clear
        let frame_len = expected_frame(&[0u8; SRC_BYTES], false).len();
        d.flush().unwrap();
        assert!(take_pixels(&mut d).len() > frame_len, "controls redrawn after wake");
    }

    #[test]
    fn clear_buffer_blanks_the_frame() {
        let (mut d, _) = display();
        Pixel(Point::new(5, 5), BinaryColor::On).draw(&mut d).unwrap();
        d.clear_buffer();
        assert!(d.frame().iter().all(|&b| b == 0));
    }
}
