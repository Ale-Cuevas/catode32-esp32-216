//! `QspiBus` on esp-hal: SPI2 in half-duplex mode with CS driven by hand,
//! like Arduino_GFX `Arduino_ESP32QSPI`, so one pixel burst can span many
//! transfers.

use catode32_amoled216::co5300::QspiBus;
use esp_hal::{
    gpio::Output,
    spi::{
        master::{Address, Command, DataMode, Spi, SpiDma},
        Error,
    },
    Blocking,
};

/// The half-duplex write that both the FIFO and the DMA SPI drivers have.
pub trait HalfDuplexWrite {
    fn hd_write(&mut self, mode: DataMode, cmd: Command, addr: Address, data: &[u8]) -> Result<(), Error>;
}

impl HalfDuplexWrite for Spi<'_, Blocking> {
    fn hd_write(&mut self, mode: DataMode, cmd: Command, addr: Address, data: &[u8]) -> Result<(), Error> {
        self.half_duplex_write(mode, cmd, addr, 0, data)
    }
}

impl HalfDuplexWrite for SpiDma<'_, Blocking> {
    fn hd_write(&mut self, mode: DataMode, cmd: Command, addr: Address, data: &[u8]) -> Result<(), Error> {
        self.half_duplex_write(mode, cmd, addr, 0, data)
    }
}

pub struct EspQspi<'d, S> {
    spi: S,
    cs: Output<'d>,
}

impl<'d, S: HalfDuplexWrite> EspQspi<'d, S> {
    pub fn new(spi: S, cs: Output<'d>) -> Self {
        Self { spi, cs }
    }

    pub fn release(self) -> (S, Output<'d>) {
        (self.spi, self.cs)
    }
}

impl<S: HalfDuplexWrite> QspiBus for EspQspi<'_, S> {
    type Error = Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.cs.set_low();
        let r = self.spi.hd_write(DataMode::Single, Command::None, Address::None, bytes);
        self.cs.set_high();
        r
    }

    fn quad_begin(&mut self, opcode: u8, address: u32, data: &[u8]) -> Result<(), Error> {
        self.cs.set_low();
        self.spi.hd_write(
            DataMode::Quad,
            Command::_8Bit(opcode as u16, DataMode::Single),
            Address::_24Bit(address, DataMode::Single),
            data,
        )
    }

    fn quad_continue(&mut self, data: &[u8]) -> Result<(), Error> {
        self.spi.hd_write(DataMode::Quad, Command::None, Address::None, data)
    }

    fn quad_end(&mut self) -> Result<(), Error> {
        self.cs.set_high();
        Ok(())
    }
}
