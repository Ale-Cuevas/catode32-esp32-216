//! CO5300 AMOLED controller over QSPI, written against a tiny bus trait so
//! the exact bytes it sends are unit-tested on the host.
//!
//! Wire protocol (from Arduino_GFX `Arduino_ESP32QSPI`, which Waveshare's
//! demo uses):
//! - Register writes go out on one line, in one CS frame:
//!   `0x02`, 24-bit address `cmd << 8`, then the parameter bytes.
//! - Pixel data starts with opcode `0x32` and address `0x003C00` (memory
//!   write continue) on one line, then the pixels on four lines (QIO),
//!   big-endian RGB565. CS stays low across chunks until the area is full.
//!
//! Init sequence and delays: Arduino_GFX `Arduino_CO5300` (`tftInit` +
//! `co5300_init_operations`), plus MADCTL 0xA0 that Waveshare's demo writes
//! after `begin()`.

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;

use crate::layout::{Rect, SCREEN};

/// Commands (CO5300 datasheet names, as in Arduino_GFX).
pub mod cmd {
    pub const SLPIN: u8 = 0x10;
    pub const SLPOUT: u8 = 0x11;
    pub const INVOFF: u8 = 0x20;
    pub const DISPOFF: u8 = 0x28;
    pub const DISPON: u8 = 0x29;
    pub const CASET: u8 = 0x2A;
    pub const PASET: u8 = 0x2B;
    pub const RAMWR: u8 = 0x2C;
    pub const MADCTL: u8 = 0x36;
    pub const PIXFMT: u8 = 0x3A;
    pub const WRMC: u8 = 0x3C;
    pub const BRIGHTNESS: u8 = 0x51;
    pub const WCTRLD1: u8 = 0x53;
    pub const WCE: u8 = 0x58;
    pub const BRIGHTNESS_HBM: u8 = 0x63;
    pub const SPIMODECTL: u8 = 0xC4;
    /// Page select (manufacturer command set); 0x00 = user commands.
    pub const PAGE: u8 = 0xFE;
}

/// Opcode for single-line register writes.
const WRITE_REG: u8 = 0x02;
/// Opcode + address that start a quad pixel burst (memory write continue).
pub const PIXEL_OPCODE: u8 = 0x32;
pub const PIXEL_ADDRESS: u32 = (cmd::WRMC as u32) << 8;

/// Orientation the Waveshare demo sets on this board. The touch mapping
/// in `docs/board-notes.md` assumes it.
pub const MADCTL_BOARD: u8 = 0xA0;
/// Brightness `init` leaves the panel at (Arduino_GFX default).
pub const DEFAULT_BRIGHTNESS: u8 = 0xD0;

const RESET_PULSE_MS: u32 = 10;
const RESET_DELAY_MS: u32 = 200;
const SLEEP_DELAY_MS: u32 = 120;
const INIT_SETTLE_MS: u32 = 10;

/// The QSPI transfers the CO5300 needs. Implemented on esp-hal by the S3
/// backend, and by a recording mock in the tests.
pub trait QspiBus {
    type Error;
    /// Send `bytes` on one data line inside a single CS frame.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
    /// Assert CS, send `opcode` and the 24-bit `address` on one line, then
    /// `data` on four lines. CS stays asserted.
    fn quad_begin(&mut self, opcode: u8, address: u32, data: &[u8]) -> Result<(), Self::Error>;
    /// More quad data in the same CS frame.
    fn quad_continue(&mut self, data: &[u8]) -> Result<(), Self::Error>;
    /// Release CS.
    fn quad_end(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<B, P> {
    Bus(B),
    Pin(P),
    /// Window not 2-px aligned or not on screen.
    BadArea,
    /// The pixel source produced a different byte count than the area holds.
    Length { expected: usize, got: usize },
}

pub struct Co5300<B, RST> {
    bus: B,
    rst: RST,
}

impl<B: QspiBus, RST: OutputPin> Co5300<B, RST> {
    pub fn new(bus: B, rst: RST) -> Self {
        Self { bus, rst }
    }

    pub fn release(self) -> (B, RST) {
        (self.bus, self.rst)
    }

    #[cfg(test)]
    pub(crate) fn bus_mut(&mut self) -> &mut B {
        &mut self.bus
    }

    /// Hardware reset, then the init sequence. Leaves the panel on, at
    /// `DEFAULT_BRIGHTNESS`, in the board orientation. Takes about 540 ms.
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), Error<B::Error, RST::Error>> {
        self.rst.set_high().map_err(Error::Pin)?;
        delay.delay_ms(RESET_PULSE_MS);
        self.rst.set_low().map_err(Error::Pin)?;
        delay.delay_ms(RESET_DELAY_MS);
        self.rst.set_high().map_err(Error::Pin)?;
        delay.delay_ms(RESET_DELAY_MS);

        self.command(cmd::SLPOUT, &[])?;
        delay.delay_ms(SLEEP_DELAY_MS);

        self.command(cmd::PAGE, &[0x00])?;
        self.command(cmd::SPIMODECTL, &[0x80])?;
        self.command(cmd::PIXFMT, &[0x55])?; // 16 bpp
        self.command(cmd::WCTRLD1, &[0x20])?;
        self.command(cmd::BRIGHTNESS_HBM, &[0xFF])?;
        self.command(cmd::DISPON, &[])?;
        self.command(cmd::BRIGHTNESS, &[DEFAULT_BRIGHTNESS])?;
        self.command(cmd::WCE, &[0x00])?; // sunlight contrast enhancement off
        delay.delay_ms(INIT_SETTLE_MS);

        self.command(cmd::INVOFF, &[])?;
        self.command(cmd::MADCTL, &[MADCTL_BOARD])
    }

    /// One register write: command plus up to 4 parameter bytes.
    pub fn command(&mut self, c: u8, params: &[u8]) -> Result<(), Error<B::Error, RST::Error>> {
        let mut buf = [0u8; 8];
        let n = 4 + params.len();
        assert!(params.len() <= 4, "CO5300 commands take at most 4 parameter bytes");
        buf[..4].copy_from_slice(&[WRITE_REG, 0x00, c, 0x00]);
        buf[4..n].copy_from_slice(params);
        self.bus.write(&buf[..n]).map_err(Error::Bus)
    }

    pub fn set_brightness(&mut self, level: u8) -> Result<(), Error<B::Error, RST::Error>> {
        self.command(cmd::BRIGHTNESS, &[level])
    }

    /// Memory access control (scan direction / RGB order). `init` sets
    /// `MADCTL_BOARD`; bring-up uses this to try other values.
    pub fn set_madctl(&mut self, value: u8) -> Result<(), Error<B::Error, RST::Error>> {
        self.command(cmd::MADCTL, &[value])
    }

    /// Same sequences as Arduino_GFX `displayOn` / `displayOff`; each takes
    /// about 240 ms.
    pub fn set_display_on(
        &mut self,
        on: bool,
        delay: &mut impl DelayNs,
    ) -> Result<(), Error<B::Error, RST::Error>> {
        let (first, second) = if on {
            (cmd::DISPON, cmd::SLPOUT)
        } else {
            (cmd::DISPOFF, cmd::SLPIN)
        };
        self.command(first, &[])?;
        delay.delay_ms(SLEEP_DELAY_MS);
        self.command(second, &[])?;
        delay.delay_ms(SLEEP_DELAY_MS);
        Ok(())
    }

    /// Set the RAM window to `area` and start a memory write.
    fn set_window(&mut self, area: Rect) -> Result<(), Error<B::Error, RST::Error>> {
        if !area.is_even_aligned() || area.w == 0 || area.h == 0 || !SCREEN.contains_rect(&area) {
            return Err(Error::BadArea);
        }
        let (x0, x1) = (area.x, area.right() - 1);
        let (y0, y1) = (area.y, area.bottom() - 1);
        let be = |a: u16, b: u16| {
            let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
            [a[0], a[1], b[0], b[1]]
        };
        self.command(cmd::CASET, &be(x0, x1))?;
        self.command(cmd::PASET, &be(y0, y1))?;
        self.command(cmd::RAMWR, &[])
    }

    /// Fill `area` with pixels from `next`, which writes the next chunk of
    /// big-endian RGB565 into `buf` and returns its length (0 = no more).
    /// The chunks must add up to exactly `area.w * area.h * 2` bytes.
    pub fn write_area<F>(
        &mut self,
        area: Rect,
        buf: &mut [u8],
        mut next: F,
    ) -> Result<(), Error<B::Error, RST::Error>>
    where
        F: FnMut(&mut [u8]) -> usize,
    {
        self.set_window(area)?;
        let expected = area.w as usize * area.h as usize * 2;
        let mut sent = 0usize;
        let mut result = Ok(());
        loop {
            let n = next(buf);
            if n == 0 || sent + n > expected {
                if n != 0 {
                    sent += n;
                }
                break;
            }
            let r = if sent == 0 {
                self.bus.quad_begin(PIXEL_OPCODE, PIXEL_ADDRESS, &buf[..n])
            } else {
                self.bus.quad_continue(&buf[..n])
            };
            if let Err(e) = r {
                result = Err(Error::Bus(e));
                break;
            }
            sent += n;
        }
        // Always release CS, even after an error.
        let end = self.bus.quad_end().map_err(Error::Bus);
        result?;
        end?;
        if sent != expected {
            return Err(Error::Length { expected, got: sent });
        }
        Ok(())
    }

    /// Fill `area` with one color, using `buf` as the chunk buffer.
    pub fn fill(&mut self, area: Rect, color: u16, buf: &mut [u8]) -> Result<(), Error<B::Error, RST::Error>> {
        let c = color.to_be_bytes();
        let chunk = buf.len() / 2 * 2;
        for px in buf.as_chunks_mut::<2>().0 {
            *px = c;
        }
        let mut left = area.w as usize * area.h as usize * 2;
        self.write_area(area, buf, |_| {
            let n = left.min(chunk);
            left -= n;
            n
        })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use core::convert::Infallible;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Txn {
        Write(Vec<u8>),
        QuadBegin(u8, u32, usize),
        QuadContinue(usize),
        QuadEnd,
        Reset(bool),
        DelayMs(u32),
    }

    /// Records everything in one shared log so pin, bus and delay order is
    /// checked together.
    pub type Log = std::rc::Rc<std::cell::RefCell<Vec<Txn>>>;

    pub struct Bus {
        pub log: Log,
        /// Every quad byte, concatenated.
        pub pixels: Vec<u8>,
        /// Fail the Nth quad transfer (0-based), for error-path tests.
        pub fail_at: Option<usize>,
        quads: usize,
    }

    impl Bus {
        pub fn new(log: Log) -> Self {
            Self { log, pixels: Vec::new(), fail_at: None, quads: 0 }
        }
        fn quad(&mut self, data: &[u8]) -> Result<(), &'static str> {
            let n = self.quads;
            self.quads += 1;
            if self.fail_at == Some(n) {
                return Err("bus error");
            }
            self.pixels.extend_from_slice(data);
            Ok(())
        }
    }

    impl QspiBus for Bus {
        type Error = &'static str;
        fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
            self.log.borrow_mut().push(Txn::Write(bytes.to_vec()));
            Ok(())
        }
        fn quad_begin(&mut self, opcode: u8, address: u32, data: &[u8]) -> Result<(), Self::Error> {
            self.log.borrow_mut().push(Txn::QuadBegin(opcode, address, data.len()));
            self.quad(data)
        }
        fn quad_continue(&mut self, data: &[u8]) -> Result<(), Self::Error> {
            self.log.borrow_mut().push(Txn::QuadContinue(data.len()));
            self.quad(data)
        }
        fn quad_end(&mut self) -> Result<(), Self::Error> {
            self.log.borrow_mut().push(Txn::QuadEnd);
            Ok(())
        }
    }

    pub struct Pin(pub Log);
    impl embedded_hal::digital::ErrorType for Pin {
        type Error = Infallible;
    }
    impl OutputPin for Pin {
        fn set_low(&mut self) -> Result<(), Infallible> {
            self.0.borrow_mut().push(Txn::Reset(false));
            Ok(())
        }
        fn set_high(&mut self) -> Result<(), Infallible> {
            self.0.borrow_mut().push(Txn::Reset(true));
            Ok(())
        }
    }

    pub struct Delay(pub Log);
    impl DelayNs for Delay {
        fn delay_ns(&mut self, ns: u32) {
            self.0.borrow_mut().push(Txn::DelayMs(ns / 1_000_000));
        }
        fn delay_ms(&mut self, ms: u32) {
            self.0.borrow_mut().push(Txn::DelayMs(ms));
        }
    }

    pub fn lcd() -> (Co5300<Bus, Pin>, Delay, Log) {
        let log = Log::default();
        (Co5300::new(Bus::new(log.clone()), Pin(log.clone())), Delay(log.clone()), log)
    }

    pub fn reg(c: u8, params: &[u8]) -> Txn {
        let mut v = vec![0x02, 0x00, c, 0x00];
        v.extend_from_slice(params);
        Txn::Write(v)
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{lcd, reg, Txn};
    use super::*;

    #[test]
    fn init_matches_arduino_gfx_plus_demo_madctl() {
        let (mut lcd, mut delay, log) = lcd();
        lcd.init(&mut delay).unwrap();
        let expected = vec![
            Txn::Reset(true),
            Txn::DelayMs(10),
            Txn::Reset(false),
            Txn::DelayMs(200),
            Txn::Reset(true),
            Txn::DelayMs(200),
            reg(0x11, &[]),
            Txn::DelayMs(120),
            reg(0xFE, &[0x00]),
            reg(0xC4, &[0x80]),
            reg(0x3A, &[0x55]),
            reg(0x53, &[0x20]),
            reg(0x63, &[0xFF]),
            reg(0x29, &[]),
            reg(0x51, &[0xD0]),
            reg(0x58, &[0x00]),
            Txn::DelayMs(10),
            reg(0x20, &[]),
            reg(0x36, &[0xA0]),
        ];
        assert_eq!(*log.borrow(), expected);
    }

    #[test]
    fn brightness_and_power() {
        let (mut lcd, mut delay, log) = lcd();
        lcd.set_brightness(0x40).unwrap();
        lcd.set_madctl(0x60).unwrap();
        lcd.set_display_on(false, &mut delay).unwrap();
        lcd.set_display_on(true, &mut delay).unwrap();
        assert_eq!(
            *log.borrow(),
            vec![
                reg(0x51, &[0x40]),
                reg(0x36, &[0x60]),
                reg(0x28, &[]),
                Txn::DelayMs(120),
                reg(0x10, &[]),
                Txn::DelayMs(120),
                reg(0x29, &[]),
                Txn::DelayMs(120),
                reg(0x11, &[]),
                Txn::DelayMs(120),
            ]
        );
    }

    #[test]
    fn write_area_sets_window_then_streams_quad() {
        let (mut lcd, _, log) = lcd();
        let area = Rect::new(48, 24, 384, 192);
        let mut buf = [0u8; 1000];
        let total = 384 * 192 * 2;
        let mut left = total;
        lcd.write_area(area, &mut buf, |b| {
            let n = left.min(b.len());
            b[..n].fill(0xAB);
            left -= n;
            n
        })
        .unwrap();

        let log = log.borrow();
        assert_eq!(log[0], reg(0x2A, &[0, 48, 1, 0xAF])); // 48..=431
        assert_eq!(log[1], reg(0x2B, &[0, 24, 0, 215])); // 24..=215
        assert_eq!(log[2], reg(0x2C, &[]));
        assert_eq!(log[3], Txn::QuadBegin(0x32, 0x003C00, 1000));
        assert!(log[4..log.len() - 1].iter().all(|t| matches!(t, Txn::QuadContinue(_))));
        assert_eq!(*log.last().unwrap(), Txn::QuadEnd);
        let quad_bytes: usize = log
            .iter()
            .map(|t| match t {
                Txn::QuadBegin(_, _, n) | Txn::QuadContinue(n) => *n,
                _ => 0,
            })
            .sum();
        assert_eq!(quad_bytes, total);
    }

    #[test]
    fn unaligned_or_offscreen_area_is_rejected_without_bus_traffic() {
        let (mut lcd, _, log) = lcd();
        let mut buf = [0u8; 16];
        for area in [
            Rect::new(1, 0, 2, 2),
            Rect::new(0, 0, 3, 2),
            Rect::new(0, 0, 0, 2),
            Rect::new(478, 0, 4, 2),
        ] {
            assert_eq!(lcd.write_area(area, &mut buf, |_| 0), Err(Error::BadArea));
        }
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn wrong_length_is_reported_and_cs_released() {
        let (mut lcd, _, log) = lcd();
        let mut buf = [0u8; 4];
        let mut calls = 0;
        let r = lcd.write_area(Rect::new(0, 0, 2, 2), &mut buf, |_| {
            calls += 1;
            if calls == 1 { 4 } else { 0 }
        });
        assert_eq!(r, Err(Error::Length { expected: 8, got: 4 }));
        assert_eq!(*log.borrow().last().unwrap(), Txn::QuadEnd);
    }

    #[test]
    fn bus_error_mid_burst_still_releases_cs() {
        let (lcd, _, log) = lcd();
        let (mut bus, pin) = lcd.release();
        bus.fail_at = Some(1);
        let mut lcd = Co5300::new(bus, pin);
        let mut buf = [0u8; 4];
        let r = lcd.write_area(Rect::new(0, 0, 2, 2), &mut buf, |b| b.len());
        assert_eq!(r, Err(Error::Bus("bus error")));
        assert_eq!(*log.borrow().last().unwrap(), Txn::QuadEnd);
    }

    #[test]
    fn fill_sends_big_endian_color() {
        let (mut lcd, _, _) = lcd();
        let mut buf = [0u8; 7]; // odd size: last byte unused
        lcd.fill(Rect::new(0, 0, 4, 2), 0xF800, &mut buf).unwrap();
        let (bus, _) = lcd.release();
        assert_eq!(bus.pixels.len(), 16);
        assert!(bus.pixels.chunks(2).all(|p| p == [0xF8, 0x00]));
    }
}
