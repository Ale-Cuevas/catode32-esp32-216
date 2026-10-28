//! S3 board init: AMOLED over QSPI + DMA, touch on the shared I2C bus,
//! BOOT/USER keys. Hands the rest of the firmware the same `Board` fields as
//! the C6/C3, except that the display arrives ready-made (`display`) instead
//! of as an I2C bus.

use core::cell::RefCell;

use catode32_amoled216::{
    co5300::Co5300,
    cst9220::Cst9220,
    display::AmoledDisplay,
    panel::Amoled,
    scale::{Palette, STRIP_BYTES},
};
use critical_section::Mutex;
use embedded_hal_bus::i2c::CriticalSectionDevice;
use esp_hal::{
    delay::Delay,
    dma_rx_buffer, dma_tx_buffer,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    peripherals::{Peripherals, FLASH, FROM_CPU_INTR0, TIMG0, WIFI},
    rng::Rng,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::Rate,
    Blocking,
};
use static_cell::StaticCell;

use super::bus::EspQspi;
use super::input::{self, ButtonPin};
use crate::board::I2C_FREQ_KHZ;
use crate::{input::Buttons, led::Led};

/// What core's `Renderer` draws into.
pub type Display = AmoledDisplay<EspQspi<'static>, Output<'static>, Delay>;
/// The I2C bus is shared: touch now, PMIC/IMU/RTC later. A critical-section
/// device so users can sit in statics (the touch state does).
pub type SharedI2c = CriticalSectionDevice<'static, I2c<'static, Blocking>>;

/// Arduino_GFX's default QSPI clock, which the Waveshare demo runs at.
const SPI_MHZ: u32 = 40;

static I2C: StaticCell<Mutex<RefCell<I2c<'static, Blocking>>>> = StaticCell::new();

pub struct Board {
    pub display: Display,
    pub buttons: Buttons,
    pub rng: Rng,
    pub led: Led,
    pub flash: FLASH<'static>,
    pub timg0: TIMG0<'static>,
    pub sw_int0: FROM_CPU_INTR0<'static>,
    pub wifi: WIFI<'static>,
}

pub fn init(p: Peripherals) -> Board {
    // Pins from docs/board-notes.md.
    let i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(I2C_FREQ_KHZ)))
        .unwrap()
        .with_sda(p.GPIO15)
        .with_scl(p.GPIO14);
    let i2c: &'static Mutex<RefCell<_>> = I2C.init(Mutex::new(RefCell::new(i2c)));

    // Writes are copied into the TX buffer (one strip) and sent in one DMA
    // transfer; the RX buffer is unused but required.
    let spi = Spi::new(p.SPI2, Config::default().with_frequency(Rate::from_mhz(SPI_MHZ)).with_mode(Mode::_0))
        .unwrap()
        .with_sck(p.GPIO38)
        .with_sio0(p.GPIO4)
        .with_sio1(p.GPIO5)
        .with_sio2(p.GPIO6)
        .with_sio3(p.GPIO7)
        .with_dma(p.DMA_CH0)
        .with_buffers(dma_rx_buffer!(32).unwrap(), dma_tx_buffer!(STRIP_BYTES).unwrap());
    let cs = Output::new(p.GPIO12, Level::High, OutputConfig::default());
    let lcd_rst = Output::new(p.GPIO39, Level::High, OutputConfig::default());

    let mut delay = Delay::new();
    let mut panel = Amoled::new(Co5300::new(EspQspi::new(spi, cs), lcd_rst), Palette::DEFAULT);
    if let Err(e) = panel.init(&mut delay) {
        println!("[s3] display init failed: {:?}", e);
    }
    let display = AmoledDisplay::new(panel, delay);

    let tp_rst = Output::new(p.GPIO40, Level::High, OutputConfig::default());
    let mut touch = Cst9220::new(CriticalSectionDevice::new(i2c), tp_rst);
    match touch.probe(&mut delay) {
        Ok(info) => println!("[s3] touch {:?}, fw 0x{:08X}", info.chip, info.fw_version),
        Err(e) => println!("[s3] touch probe failed: {:?}", e),
    }
    // Keys have external pull-ups and read LOW when pressed.
    let key_cfg = InputConfig::default().with_pull(Pull::None);
    input::install(
        p.IO_MUX,
        touch,
        Input::new(p.GPIO11, InputConfig::default().with_pull(Pull::Up)),
        Input::new(p.GPIO0, key_cfg),
        Input::new(p.GPIO18, key_cfg),
    );

    Board {
        display,
        buttons: Buttons::new(core::array::from_fn(ButtonPin::new)),
        rng: Rng::new(),
        led: Led::new(),
        flash: p.FLASH,
        timg0: p.TIMG0,
        sw_int0: p.FROM_CPU_INTR0,
        wifi: p.WIFI,
    }
}
