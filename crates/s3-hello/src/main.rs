//! First thing to flash on the ESP32-S3-Touch-AMOLED-2.16: prints a counter
//! and the BOOT / USER key states over native USB every 500 ms. Proves the
//! Xtensa toolchain, espflash and the log path before any display work.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Pull},
    main,
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Both keys have external 10K pull-ups and read LOW when pressed.
    let cfg = InputConfig::default().with_pull(Pull::None);
    let boot = Input::new(p.GPIO0, cfg);
    let user = Input::new(p.GPIO18, cfg);

    let delay = Delay::new();
    let mut n: u32 = 0;
    loop {
        println!(
            "catode32 s3-hello #{}: BOOT={} USER={}",
            n,
            if boot.is_low() { "down" } else { "up" },
            if user.is_low() { "down" } else { "up" },
        );
        n = n.wrapping_add(1);
        delay.delay_millis(500);
    }
}
