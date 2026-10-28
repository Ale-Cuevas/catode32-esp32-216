//! CST9220 (or CST9217) capacitive touch controller over I2C, written against
//! `embedded_hal::i2c::I2c` so the caller decides how the bus is shared and
//! the exact bytes are unit-tested on the host.
//!
//! Protocol from SensorLib 0.3.3 `TouchDrvCST92xx` (MIT, Lewis He), which
//! Waveshare's demo uses. All registers are 16-bit, sent big-endian; the
//! values that come back are little-endian.
//! - Probe: reset, write `D101` (command mode), then read `D1FC` (checkcode,
//!   upper half `0xCACA`), `D1F8` (resolution), `D204` (project id, chip
//!   type) and `D208` (firmware version, checksum).
//! - Points: write `D000`, read 15 bytes, then always write `D000 AB` (ack).
//!   The read is only valid if byte 6 is `0xAB`. Byte 5 holds the point
//!   count (at most 2); finger 0 starts at byte 0, finger 1 at byte 7.
//! - INT pulses on touch instead of staying low while a finger is down, so
//!   `touch::TouchTracker` polls after the first edge until release.

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::i2c::I2c;

use crate::layout::{SCREEN_H, SCREEN_W};

pub const ADDRESS: u8 = 0x5A;
/// Fingers the controller reports (SensorLib `CST92XX_MAX_FINGER_NUM`).
pub const MAX_POINTS: usize = 2;

/// 16-bit registers.
pub mod reg {
    pub const READ_POINTS: u16 = 0xD000;
    pub const COMMAND_MODE: u16 = 0xD101;
    pub const SLEEP: u16 = 0xD105;
    pub const RESOLUTION: u16 = 0xD1F8;
    pub const CHECKCODE: u16 = 0xD1FC;
    pub const CHIP_TYPE: u16 = 0xD204;
    pub const FW_VERSION: u16 = 0xD208;
}

const ACK: u8 = 0xAB;
/// Finger event code for "pressed".
const EVT_PRESSED: u8 = 0x06;
const POINTS_LEN: usize = MAX_POINTS * 5 + 5;
const NO_FIRMWARE: u32 = 0xA5A5_A5A5;

const RESET_PULSE_MS: u32 = 10;
/// Time to leave the controller's boot mode after reset.
const BOOT_MS: u32 = 30;
const COMMAND_MODE_MS: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Cst9220,
    Cst9217,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub chip: Chip,
    pub project_id: u16,
    pub res_x: u16,
    pub res_y: u16,
    pub fw_version: u32,
    pub checksum: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<I, P> {
    I2c(I),
    Pin(P),
    /// Checkcode upper half isn't `0xCACA`: info registers didn't read.
    BadCheckcode(u32),
    /// The controller reports no firmware.
    NoFirmware,
    UnknownChip(u16),
}

pub struct Cst9220<I2C, RST> {
    i2c: I2C,
    rst: RST,
}

impl<I2C: I2c, RST: OutputPin> Cst9220<I2C, RST> {
    pub fn new(i2c: I2C, rst: RST) -> Self {
        Self { i2c, rst }
    }

    pub fn release(self) -> (I2C, RST) {
        (self.i2c, self.rst)
    }

    /// Hardware reset. Also wakes the controller from `sleep`.
    pub fn reset(&mut self, delay: &mut impl DelayNs) -> Result<(), Error<I2C::Error, RST::Error>> {
        self.rst.set_low().map_err(Error::Pin)?;
        delay.delay_ms(RESET_PULSE_MS);
        self.rst.set_high().map_err(Error::Pin)?;
        delay.delay_ms(BOOT_MS);
        Ok(())
    }

    /// Reset, then read the chip info (SensorLib `getAttribute`). Points can
    /// be read straight after.
    pub fn probe(&mut self, delay: &mut impl DelayNs) -> Result<Info, Error<I2C::Error, RST::Error>> {
        self.reset(delay)?;
        self.write(reg::COMMAND_MODE, &[])?;
        delay.delay_ms(COMMAND_MODE_MS);

        let mut b = [0u8; 8];
        self.read(reg::CHECKCODE, &mut b[..4])?;
        let checkcode = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        self.read(reg::RESOLUTION, &mut b[..4])?;
        let (res_x, res_y) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
        self.read(reg::CHIP_TYPE, &mut b[..4])?;
        let project_id = u16::from_le_bytes([b[0], b[1]]);
        let chip_type = u16::from_le_bytes([b[2], b[3]]);
        self.read(reg::FW_VERSION, &mut b)?;
        let fw_version = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let checksum = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);

        // Same checks, in the same order, as SensorLib.
        if fw_version == NO_FIRMWARE {
            return Err(Error::NoFirmware);
        }
        if checkcode & 0xFFFF_0000 != 0xCACA_0000 {
            return Err(Error::BadCheckcode(checkcode));
        }
        let chip = match chip_type {
            0x9220 => Chip::Cst9220,
            0x9217 => Chip::Cst9217,
            other => return Err(Error::UnknownChip(other)),
        };
        Ok(Info { chip, project_id, res_x, res_y, fw_version, checksum })
    }

    /// Read the current touch points in raw controller coordinates (use
    /// `Orientation::map` for panel coordinates). Returns how many of `out`
    /// were filled; 0 when nothing is touched, on a release, or when the
    /// controller didn't ack the read.
    pub fn read_points(
        &mut self,
        out: &mut [(u16, u16); MAX_POINTS],
    ) -> Result<usize, Error<I2C::Error, RST::Error>> {
        let mut b = [0u8; POINTS_LEN];
        let r = self.read(reg::READ_POINTS, &mut b);
        // The ack goes out even after a failed read, like SensorLib.
        let ack = self.write(reg::READ_POINTS, &[ACK]);
        r?;
        ack?;
        if b[6] != ACK {
            return Ok(0);
        }
        let count = (b[5] & 0x7F) as usize;
        if count == 0 || count > MAX_POINTS {
            return Ok(0);
        }

        let mut n = 0;
        for i in 0..count {
            let d = &b[i * 5 + if i == 0 { 0 } else { 2 }..];
            // SensorLib keeps a not-pressed finger as (0, 0); skip it instead.
            if d[0] & 0x0F != EVT_PRESSED || (d[0] >> 4) as usize >= MAX_POINTS {
                continue;
            }
            let x = (d[1] as u16) << 4 | (d[3] >> 4) as u16;
            let y = (d[2] as u16) << 4 | (d[3] & 0x0F) as u16;
            out[n] = (x, y);
            n += 1;
        }
        Ok(n)
    }

    /// Low-power sleep. Wake with `reset`.
    pub fn sleep(&mut self) -> Result<(), Error<I2C::Error, RST::Error>> {
        self.write(reg::SLEEP, &[])
    }

    fn write(&mut self, r: u16, data: &[u8]) -> Result<(), Error<I2C::Error, RST::Error>> {
        let mut buf = [0u8; 3];
        let n = 2 + data.len();
        buf[..2].copy_from_slice(&r.to_be_bytes());
        buf[2..n].copy_from_slice(data);
        self.i2c.write(ADDRESS, &buf[..n]).map_err(Error::I2c)
    }

    fn read(&mut self, r: u16, out: &mut [u8]) -> Result<(), Error<I2C::Error, RST::Error>> {
        self.i2c.write_read(ADDRESS, &r.to_be_bytes(), out).map_err(Error::I2c)
    }
}

/// Raw controller coordinates to panel coordinates: swap, then mirror
/// (SensorLib `updateXY` order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orientation {
    pub swap_xy: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
}

/// What Waveshare's demo uses with MADCTL 0xA0 (`docs/board-notes.md`).
/// Not yet confirmed on the device; if touches land mirrored or rotated,
/// change only this.
pub const BOARD_ORIENTATION: Orientation = Orientation { swap_xy: true, mirror_x: true, mirror_y: false };

impl Orientation {
    /// Map one raw point. Mirroring is `max - v` with max = 480, as in
    /// SensorLib, so results are clamped to 0..=479.
    pub const fn map(&self, (x, y): (u16, u16)) -> (u16, u16) {
        let (mut x, mut y) = if self.swap_xy { (y, x) } else { (x, y) };
        if self.mirror_x {
            x = SCREEN_W.saturating_sub(x);
        }
        if self.mirror_y {
            y = SCREEN_H.saturating_sub(y);
        }
        (clamp(x, SCREEN_W), clamp(y, SCREEN_H))
    }
}

const fn clamp(v: u16, max: u16) -> u16 {
    if v >= max {
        max - 1
    } else {
        v
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use super::*;
    use core::convert::Infallible;
    use embedded_hal::i2c::{ErrorKind, ErrorType, Operation};
    use std::collections::VecDeque;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Txn {
        Write(u8, Vec<u8>),
        Read(u8, usize),
        Reset(bool),
        DelayMs(u32),
    }

    pub type Log = std::rc::Rc<std::cell::RefCell<Vec<Txn>>>;

    #[derive(Debug, PartialEq, Eq)]
    pub struct Nack;
    impl embedded_hal::i2c::Error for Nack {
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    /// Records every operation and answers reads from `replies`, in order.
    /// A read with no reply left (or a `None` reply) fails.
    pub struct Bus {
        pub log: Log,
        pub replies: VecDeque<Option<Vec<u8>>>,
    }

    impl ErrorType for Bus {
        type Error = Nack;
    }

    impl I2c for Bus {
        fn transaction(&mut self, addr: u8, ops: &mut [Operation<'_>]) -> Result<(), Nack> {
            for op in ops {
                match op {
                    Operation::Write(b) => self.log.borrow_mut().push(Txn::Write(addr, b.to_vec())),
                    Operation::Read(out) => {
                        self.log.borrow_mut().push(Txn::Read(addr, out.len()));
                        let reply = self.replies.pop_front().flatten().ok_or(Nack)?;
                        assert_eq!(reply.len(), out.len(), "scripted reply length");
                        out.copy_from_slice(&reply);
                    }
                }
            }
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

    pub fn touch(replies: Vec<Option<Vec<u8>>>) -> (Cst9220<Bus, Pin>, Delay, Log) {
        let log = Log::default();
        let bus = Bus { log: log.clone(), replies: replies.into() };
        (Cst9220::new(bus, Pin(log.clone())), Delay(log.clone()), log)
    }

    pub fn w(bytes: &[u8]) -> Txn {
        Txn::Write(ADDRESS, bytes.to_vec())
    }

    /// One finger record: id, event, 12-bit x/y packed as the chip sends them.
    pub fn finger(id: u8, evt: u8, x: u16, y: u16) -> [u8; 5] {
        [id << 4 | evt, (x >> 4) as u8, (y >> 4) as u8, ((x & 0xF) << 4 | (y & 0xF)) as u8, 0]
    }

    /// A 15-byte point report.
    pub fn report(fingers: &[[u8; 5]], ack: u8) -> Vec<u8> {
        let mut b = vec![0u8; POINTS_LEN];
        if let Some(f) = fingers.first() {
            b[..5].copy_from_slice(f);
        }
        if let Some(f) = fingers.get(1) {
            b[7..12].copy_from_slice(f);
        }
        b[5] = fingers.len() as u8;
        b[6] = ack;
        b
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{finger, report, touch, w, Nack, Txn};
    use super::*;

    fn info_replies(checkcode: u32, chip: u16, fw: u32) -> Vec<Option<Vec<u8>>> {
        let mut fw_bytes = fw.to_le_bytes().to_vec();
        fw_bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        let mut chip_bytes = 0x0042u16.to_le_bytes().to_vec();
        chip_bytes.extend_from_slice(&chip.to_le_bytes());
        vec![
            Some(checkcode.to_le_bytes().to_vec()),
            Some(vec![0xE0, 0x01, 0xE0, 0x01]), // 480 x 480
            Some(chip_bytes),
            Some(fw_bytes),
        ]
    }

    #[test]
    fn probe_matches_sensorlib() {
        let (mut t, mut delay, log) = touch(info_replies(0xCACA_0001, 0x9220, 0x0102_0304));
        let info = t.probe(&mut delay).unwrap();
        assert_eq!(
            info,
            Info {
                chip: Chip::Cst9220,
                project_id: 0x0042,
                res_x: 480,
                res_y: 480,
                fw_version: 0x0102_0304,
                checksum: 0x1234_5678
            }
        );
        assert_eq!(
            *log.borrow(),
            vec![
                Txn::Reset(false),
                Txn::DelayMs(10),
                Txn::Reset(true),
                Txn::DelayMs(30),
                w(&[0xD1, 0x01]),
                Txn::DelayMs(10),
                w(&[0xD1, 0xFC]),
                Txn::Read(ADDRESS, 4),
                w(&[0xD1, 0xF8]),
                Txn::Read(ADDRESS, 4),
                w(&[0xD2, 0x04]),
                Txn::Read(ADDRESS, 4),
                w(&[0xD2, 0x08]),
                Txn::Read(ADDRESS, 8),
            ]
        );
    }

    #[test]
    fn probe_accepts_cst9217() {
        let (mut t, mut delay, _) = touch(info_replies(0xCACA_0000, 0x9217, 1));
        assert_eq!(t.probe(&mut delay).unwrap().chip, Chip::Cst9217);
    }

    #[test]
    fn probe_failures() {
        let (mut t, mut d, _) = touch(info_replies(0xCACA_0000, 0x9220, NO_FIRMWARE));
        assert_eq!(t.probe(&mut d), Err(Error::NoFirmware));
        let (mut t, mut d, _) = touch(info_replies(0x1234_0000, 0x9220, 1));
        assert_eq!(t.probe(&mut d), Err(Error::BadCheckcode(0x1234_0000)));
        let (mut t, mut d, _) = touch(info_replies(0xCACA_0000, 0x1234, 1));
        assert_eq!(t.probe(&mut d), Err(Error::UnknownChip(0x1234)));
        let (mut t, mut d, _) = touch(vec![None]);
        assert_eq!(t.probe(&mut d), Err(Error::I2c(Nack)));
    }

    fn read(reply: Option<Vec<u8>>) -> (Result<usize, Error<Nack, core::convert::Infallible>>, [(u16, u16); 2], Vec<Txn>) {
        let (mut t, _, log) = touch(vec![reply]);
        let mut pts = [(0, 0); 2];
        let r = t.read_points(&mut pts);
        let log = log.borrow().clone();
        (r, pts, log)
    }

    #[test]
    fn one_finger_parses_and_acks() {
        let (r, pts, log) = read(Some(report(&[finger(0, 6, 0x123, 0x1DF)], ACK)));
        assert_eq!(r, Ok(1));
        assert_eq!(pts[0], (0x123, 0x1DF));
        assert_eq!(log, vec![w(&[0xD0, 0x00]), Txn::Read(ADDRESS, 15), w(&[0xD0, 0x00, 0xAB])]);
    }

    #[test]
    fn two_fingers_use_the_offset_second_record() {
        let (r, pts, _) = read(Some(report(&[finger(0, 6, 10, 20), finger(1, 6, 470, 300)], ACK)));
        assert_eq!(r, Ok(2));
        assert_eq!(pts, [(10, 20), (470, 300)]);
    }

    #[test]
    fn release_bad_ack_and_bad_counts_give_no_points() {
        assert_eq!(read(Some(report(&[finger(0, 0, 10, 20)], ACK))).0, Ok(0));
        assert_eq!(read(Some(report(&[finger(0, 6, 10, 20)], 0x00))).0, Ok(0));
        assert_eq!(read(Some(report(&[], ACK))).0, Ok(0));
        let mut three = report(&[finger(0, 6, 10, 20)], ACK);
        three[5] = 3;
        assert_eq!(read(Some(three)).0, Ok(0));
    }

    #[test]
    fn second_finger_not_pressed_is_skipped() {
        let (r, pts, _) = read(Some(report(&[finger(0, 6, 10, 20), finger(1, 0, 0, 0)], ACK)));
        assert_eq!(r, Ok(1));
        assert_eq!(pts[0], (10, 20));
    }

    #[test]
    fn failed_read_still_sends_the_ack() {
        let (r, _, log) = read(None);
        assert_eq!(r, Err(Error::I2c(Nack)));
        assert_eq!(*log.last().unwrap(), w(&[0xD0, 0x00, 0xAB]));
    }

    #[test]
    fn sleep_command() {
        let (mut t, _, log) = touch(vec![]);
        t.sleep().unwrap();
        assert_eq!(*log.borrow(), vec![w(&[0xD1, 0x05])]);
    }

    #[test]
    fn board_orientation_corners() {
        let o = BOARD_ORIENTATION;
        // swap, then x = 480 - x (clamped).
        assert_eq!(o.map((0, 0)), (479, 0));
        assert_eq!(o.map((479, 0)), (479, 479));
        assert_eq!(o.map((0, 479)), (1, 0));
        assert_eq!(o.map((479, 479)), (1, 479));
        assert_eq!(o.map((240, 100)), (380, 240));
    }

    #[test]
    fn each_flag_alone() {
        let none = Orientation { swap_xy: false, mirror_x: false, mirror_y: false };
        assert_eq!(none.map((10, 20)), (10, 20));
        assert_eq!(none.map((600, 600)), (479, 479));
        assert_eq!(Orientation { swap_xy: true, ..none }.map((10, 20)), (20, 10));
        assert_eq!(Orientation { mirror_x: true, ..none }.map((10, 20)), (470, 20));
        assert_eq!(Orientation { mirror_y: true, ..none }.map((10, 20)), (10, 460));
    }
}
