//! Catode32 on the Waveshare ESP32-S3-Touch-AMOLED-2.16. Same flow as
//! `crates/firmware/src/main.rs`; the board differences (AMOLED, touch, no
//! LED) live behind core's `s3` feature.

#![no_std]
#![no_main]

// Shared with the C6/C3 firmware: log, record to RTC RAM, reset.
#[path = "../../firmware/src/panic.rs"]
mod panic;

use catode32_core::{
    board, espnow_manager::EspNowManager, game::Game, platform::watchdog, render::Renderer,
    storage,
};
use esp_hal::{clock::CpuClock, main, timer::timg::TimerGroup};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    // Heap for the wifi stack, as on the C6 (see `crates/firmware`). Internal
    // RAM; the 8 MB PSRAM isn't needed since frames are streamed in strips.
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    // Arm the watchdog before anything that could hang (flash, display,
    // touch probe, radio scheduler); the game loop feeds it.
    watchdog::start();
    let board = board::init(peripherals);
    storage::init(board.flash);
    let renderer = Renderer::new(board.display);

    println!("catode32 v0.13.0 behavior framework (ESP32-S3 AMOLED)");

    let timg0 = TimerGroup::new(board.timg0);
    esp_rtos::start(timg0.timer0, board.sw_int0);

    Game::new(
        renderer,
        board.buttons,
        board.rng,
        board.led,
        board.wifi,
        EspNowManager::new(),
    )
    .run();
}
