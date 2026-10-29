//! AXP2101 PMIC over I2C: battery gauge, charger and USB state, and the PWR
//! key short-press flag. Written against `embedded_hal::i2c::I2c` like
//! `cst9220`, so the bytes are unit-tested on the host.
//!
//! Registers and bit positions are from XPowersLib 0.2.6 (MIT, Lewis He),
//! which Waveshare's demo uses: `REG/AXP2101Constants.h` for addresses,
//! `XPowersAXP2101.tpp` for the bits, `XPowersParams.hpp` for the IRQ and
//! charge-voltage enums. 8-bit registers, one byte each.
//!
//! The demo leaves every power rail at its power-on default and only sets
//! the 4.2 V charge target. This driver does the same and has no rail
//! controls on purpose: a wrong write could brown out the panel.
//!
//! The PMIC IRQ line isn't routed to a known ESP32 GPIO
//! (`docs/board-notes.md`), so the PWR key flag is polled.

use embedded_hal::i2c::I2c;

pub const ADDRESS: u8 = 0x34;
/// `XPOWERS_AXP2101_CHIP_ID`, read from `IC_TYPE`.
pub const CHIP_ID: u8 = 0x4A;

pub mod reg {
    pub const STATUS1: u8 = 0x00;
    pub const STATUS2: u8 = 0x01;
    pub const IC_TYPE: u8 = 0x03;
    pub const ADC_CHANNEL_CTRL: u8 = 0x30;
    /// Battery voltage, high 5 bits (mV).
    pub const ADC_VBAT_H: u8 = 0x34;
    pub const ADC_VBAT_L: u8 = 0x35;
    pub const INTEN2: u8 = 0x41;
    pub const INTSTS1: u8 = 0x48;
    pub const INTSTS2: u8 = 0x49;
    pub const INTSTS3: u8 = 0x4A;
    pub const CV_CHG_VOL_SET: u8 = 0x64;
    pub const BAT_DET_CTRL: u8 = 0x68;
    pub const BAT_PERCENT: u8 = 0xA4;
}

/// `STATUS1` bits.
const S1_VBUS_GOOD: u8 = 1 << 5;
const S1_BAT_PRESENT: u8 = 1 << 3;
/// `STATUS2` bit 3 set means VBUS is not in use (`isVbusIn` checks it is 0).
const S2_VBUS_OFF: u8 = 1 << 3;
/// `ADC_CHANNEL_CTRL` bits.
const ADC_VBAT: u8 = 1 << 0;
const ADC_TS: u8 = 1 << 1;
const BAT_DET_EN: u8 = 1 << 0;
/// `XPOWERS_AXP2101_CHG_VOL_4V2`, in the low 3 bits of `CV_CHG_VOL_SET`.
const CHG_VOL_4V2: u8 = 3;
const CHG_VOL_MASK: u8 = 0x07;
/// `XPOWERS_AXP2101_PKEY_SHORT_IRQ` (`_BV(11)`): bit 3 of the second
/// enable/status pair.
const IRQ2_PKEY_SHORT: u8 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Standby,
    Charging,
    Discharging,
    /// Any other `STATUS2 >> 5` value.
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// USB power is present and in use.
    pub vbus_in: bool,
    pub battery_present: bool,
    pub charge: Charge,
    /// Battery voltage in mV, `None` without a battery.
    pub battery_mv: Option<u16>,
    /// Fuel gauge, 0..=100, `None` without a battery.
    pub battery_pct: Option<u8>,
}

impl Status {
    pub fn charging(&self) -> bool {
        self.charge == Charge::Charging
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<I> {
    I2c(I),
    WrongChip(u8),
}

pub struct Axp2101<I2C> {
    i2c: I2C,
}

impl<I2C: I2c> Axp2101<I2C> {
    pub fn new(i2c: I2C) -> Self {
        Self { i2c }
    }

    pub fn release(self) -> I2C {
        self.i2c
    }

    /// Check the chip ID, then apply the demo's settings (`initImpl` plus
    /// `setChargeTargetVoltage(4V2)`, `enableBattDetection`,
    /// `enableBattVoltageMeasure`). Also enables and clears the PWR key
    /// short-press flag.
    pub fn init(&mut self) -> Result<(), Error<I2C::Error>> {
        let id = self.read(reg::IC_TYPE)?;
        if id != CHIP_ID {
            return Err(Error::WrongChip(id));
        }
        // initImpl: the NTC pin isn't fitted, so don't measure it.
        self.modify(reg::ADC_CHANNEL_CTRL, ADC_TS, ADC_VBAT)?;
        self.modify(reg::CV_CHG_VOL_SET, CHG_VOL_MASK, CHG_VOL_4V2)?;
        self.modify(reg::BAT_DET_CTRL, 0, BAT_DET_EN)?;
        self.modify(reg::INTEN2, 0, IRQ2_PKEY_SHORT)?;
        // clearIrqStatus: write-1-to-clear.
        for r in [reg::INTSTS1, reg::INTSTS2, reg::INTSTS3] {
            self.write(r, 0xFF)?;
        }
        Ok(())
    }

    pub fn status(&mut self) -> Result<Status, Error<I2C::Error>> {
        let mut s = [0u8; 2];
        self.read_into(reg::STATUS1, &mut s)?;
        let [s1, s2] = s;
        let battery_present = s1 & S1_BAT_PRESENT != 0;
        let charge = match s2 >> 5 {
            0 => Charge::Standby,
            1 => Charge::Charging,
            2 => Charge::Discharging,
            other => Charge::Other(other),
        };
        let (battery_mv, battery_pct) = if battery_present {
            let mut v = [0u8; 2];
            self.read_into(reg::ADC_VBAT_H, &mut v)?;
            let mv = ((v[0] & 0x1F) as u16) << 8 | v[1] as u16;
            let pct = self.read(reg::BAT_PERCENT)?.min(100);
            (Some(mv), Some(pct))
        } else {
            (None, None)
        };
        Ok(Status {
            vbus_in: s1 & S1_VBUS_GOOD != 0 && s2 & S2_VBUS_OFF == 0,
            battery_present,
            charge,
            battery_mv,
            battery_pct,
        })
    }

    /// True once per PWR key short press since the last call (clears the
    /// flag).
    pub fn take_power_key_press(&mut self) -> Result<bool, Error<I2C::Error>> {
        let pressed = self.read(reg::INTSTS2)? & IRQ2_PKEY_SHORT != 0;
        if pressed {
            self.write(reg::INTSTS2, IRQ2_PKEY_SHORT)?;
        }
        Ok(pressed)
    }

    fn read(&mut self, r: u8) -> Result<u8, Error<I2C::Error>> {
        let mut b = [0u8];
        self.read_into(r, &mut b)?;
        Ok(b[0])
    }

    fn read_into(&mut self, r: u8, out: &mut [u8]) -> Result<(), Error<I2C::Error>> {
        self.i2c.write_read(ADDRESS, &[r], out).map_err(Error::I2c)
    }

    fn write(&mut self, r: u8, v: u8) -> Result<(), Error<I2C::Error>> {
        self.i2c.write(ADDRESS, &[r, v]).map_err(Error::I2c)
    }

    /// Read-modify-write: clear `clear`, then set `set`.
    fn modify(&mut self, r: u8, clear: u8, set: u8) -> Result<(), Error<I2C::Error>> {
        let v = self.read(r)?;
        self.write(r, (v & !clear) | set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cst9220::mock::Nack;
    use embedded_hal::i2c::{ErrorType, Operation};

    /// A register file. Reads auto-increment like the chip; writes are
    /// logged and applied, except INTSTS*, which clear the bits written.
    struct Regs {
        r: [u8; 256],
        writes: Vec<(u8, u8)>,
        fail: bool,
    }

    impl Regs {
        fn new() -> Self {
            let mut r = [0u8; 256];
            r[reg::IC_TYPE as usize] = CHIP_ID;
            Self { r, writes: Vec::new(), fail: false }
        }
    }

    impl ErrorType for Regs {
        type Error = Nack;
    }

    impl I2c for Regs {
        fn transaction(&mut self, addr: u8, ops: &mut [Operation<'_>]) -> Result<(), Nack> {
            assert_eq!(addr, ADDRESS);
            if self.fail {
                return Err(Nack);
            }
            let mut ptr = 0usize;
            for op in ops {
                match op {
                    Operation::Write(b) => {
                        ptr = b[0] as usize;
                        for &v in &b[1..] {
                            self.writes.push((ptr as u8, v));
                            let is_sts = (reg::INTSTS1..=reg::INTSTS3).contains(&(ptr as u8));
                            self.r[ptr] = if is_sts { self.r[ptr] & !v } else { v };
                            ptr += 1;
                        }
                    }
                    Operation::Read(out) => {
                        for o in out.iter_mut() {
                            *o = self.r[ptr];
                            ptr += 1;
                        }
                    }
                }
            }
            Ok(())
        }
    }

    #[test]
    fn init_applies_demo_settings() {
        let mut regs = Regs::new();
        regs.r[reg::ADC_CHANNEL_CTRL as usize] = 0b1000_0010; // TS on
        regs.r[reg::CV_CHG_VOL_SET as usize] = 0b1010_0001; // 4.0 V
        regs.r[reg::INTEN2 as usize] = 0x40;
        regs.r[reg::INTSTS2 as usize] = 0xFF;
        let mut pmu = Axp2101::new(regs);
        pmu.init().unwrap();
        let regs = pmu.release();
        assert_eq!(regs.r[reg::ADC_CHANNEL_CTRL as usize], 0b1000_0001);
        assert_eq!(regs.r[reg::CV_CHG_VOL_SET as usize], 0b1010_0011);
        assert_eq!(regs.r[reg::BAT_DET_CTRL as usize], 0x01);
        assert_eq!(regs.r[reg::INTEN2 as usize], 0x48);
        assert_eq!(regs.r[reg::INTSTS2 as usize], 0);
        // Nothing outside these registers is written (no rail changes).
        let allowed = [
            reg::ADC_CHANNEL_CTRL,
            reg::CV_CHG_VOL_SET,
            reg::BAT_DET_CTRL,
            reg::INTEN2,
            reg::INTSTS1,
            reg::INTSTS2,
            reg::INTSTS3,
        ];
        assert!(regs.writes.iter().all(|(r, _)| allowed.contains(r)), "{:?}", regs.writes);
    }

    #[test]
    fn init_rejects_wrong_chip() {
        let mut regs = Regs::new();
        regs.r[reg::IC_TYPE as usize] = 0x03;
        let mut pmu = Axp2101::new(regs);
        assert_eq!(pmu.init(), Err(Error::WrongChip(0x03)));
        assert!(pmu.release().writes.is_empty());
    }

    #[test]
    fn status_on_usb_charging() {
        let mut regs = Regs::new();
        regs.r[reg::STATUS1 as usize] = S1_VBUS_GOOD | S1_BAT_PRESENT;
        regs.r[reg::STATUS2 as usize] = 1 << 5;
        // 0x1F upper bits of the high byte are ignored: 0x0F_A0 = 4000 mV.
        regs.r[reg::ADC_VBAT_H as usize] = 0xEF;
        regs.r[reg::ADC_VBAT_L as usize] = 0xA0;
        regs.r[reg::BAT_PERCENT as usize] = 87;
        let s = Axp2101::new(regs).status().unwrap();
        assert_eq!(
            s,
            Status {
                vbus_in: true,
                battery_present: true,
                charge: Charge::Charging,
                battery_mv: Some(4000),
                battery_pct: Some(87),
            }
        );
        assert!(s.charging());
    }

    #[test]
    fn status_without_battery_skips_gauge() {
        let mut regs = Regs::new();
        regs.r[reg::STATUS1 as usize] = S1_VBUS_GOOD;
        regs.r[reg::BAT_PERCENT as usize] = 50;
        let s = Axp2101::new(regs).status().unwrap();
        assert!(s.vbus_in);
        assert_eq!((s.battery_mv, s.battery_pct), (None, None));
    }

    #[test]
    fn status_vbus_good_but_not_in_use() {
        let mut regs = Regs::new();
        regs.r[reg::STATUS1 as usize] = S1_VBUS_GOOD | S1_BAT_PRESENT;
        regs.r[reg::STATUS2 as usize] = (2 << 5) | S2_VBUS_OFF;
        regs.r[reg::BAT_PERCENT as usize] = 140; // out of range: clamped
        let s = Axp2101::new(regs).status().unwrap();
        assert!(!s.vbus_in);
        assert_eq!(s.charge, Charge::Discharging);
        assert_eq!(s.battery_pct, Some(100));
    }

    #[test]
    fn power_key_press_is_taken_once() {
        let mut regs = Regs::new();
        regs.r[reg::INTSTS2 as usize] = IRQ2_PKEY_SHORT | 0x80; // + VBUS insert
        let mut pmu = Axp2101::new(regs);
        assert!(pmu.take_power_key_press().unwrap());
        assert!(!pmu.take_power_key_press().unwrap());
        let regs = pmu.release();
        // Only the PWR bit was cleared.
        assert_eq!(regs.r[reg::INTSTS2 as usize], 0x80);
        assert_eq!(regs.writes, vec![(reg::INTSTS2, IRQ2_PKEY_SHORT)]);
    }

    #[test]
    fn bus_error_propagates() {
        let mut regs = Regs::new();
        regs.fail = true;
        assert_eq!(Axp2101::new(regs).status(), Err(Error::I2c(Nack)));
    }
}
