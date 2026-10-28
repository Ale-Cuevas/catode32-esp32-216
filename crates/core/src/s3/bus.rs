//! `QspiBus` on esp-hal: SPI2 + DMA in half-duplex mode with CS driven by
//! hand, like Arduino_GFX `Arduino_ESP32QSPI`, so one pixel burst can span
//! many transfers. Same as `crates/s3-bringup/src/bus.rs`, DMA only.

use catode32_amoled216::co5300::QspiBus;
use esp_hal::{
    gpio::Output,
    spi::{
        master::{Address, Command, DataMode, SpiDma},
        Error,
    },
    Blocking,
};

pub struct EspQspi<'d> {
    spi: SpiDma<'d, Blocking>,
    cs: Output<'d>,
}

impl<'d> EspQspi<'d> {
    pub fn new(spi: SpiDma<'d, Blocking>, cs: Output<'d>) -> Self {
        Self { spi, cs }
    }
}

impl QspiBus for EspQspi<'_> {
    type Error = Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.cs.set_low();
        let r = self.spi.half_duplex_write(DataMode::Single, Command::None, Address::None, 0, bytes);
        self.cs.set_high();
        r
    }

    fn quad_begin(&mut self, opcode: u8, address: u32, data: &[u8]) -> Result<(), Error> {
        self.cs.set_low();
        self.spi.half_duplex_write(
            DataMode::Quad,
            Command::_8Bit(opcode as u16, DataMode::Single),
            Address::_24Bit(address, DataMode::Single),
            0,
            data,
        )
    }

    fn quad_continue(&mut self, data: &[u8]) -> Result<(), Error> {
        self.spi.half_duplex_write(DataMode::Quad, Command::None, Address::None, 0, data)
    }

    fn quad_end(&mut self) -> Result<(), Error> {
        self.cs.set_high();
        Ok(())
    }
}
