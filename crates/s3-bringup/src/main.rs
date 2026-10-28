//! Display bring-up for the ESP32-S3-Touch-AMOLED-2.16 (port plan WP3).
//!
//! Runs the CO5300 driver from `catode32-amoled216` on the real panel, one
//! phase at a time, and says over USB what the screen should show:
//!
//! 1. Reset + init, screen cleared to black.
//! 2. Full-screen RED, GREEN, BLUE, WHITE (1 s each): color order and byte order.
//! 3. Orientation pattern in the 3x game viewport plus the touch controls.
//! 4. Flush timing over the FIFO bus, then the same over DMA.
//! 5. Brightness ramp, display off for 2 s, back on.
//! 6. Endless 12 FPS animation: BOOT toggles invert, USER cycles brightness,
//!    the controls highlight one button per second. Timing logged every 2 s.
//!
//! Each step logs its result and carries on after an error, so one bad
//! step doesn't hide the others. Checklist: `docs/DEVICE_TESTS.md`.

#![no_std]
#![no_main]

mod bus;

use core::fmt::Debug;

use catode32_amoled216::{
    co5300::{Co5300, QspiBus},
    layout::{SRC_H, SRC_W},
    panel::Amoled,
    scale::{Palette, SRC_BYTES, STRIP_BYTES},
};
use embedded_hal::{delay::DelayNs, digital::OutputPin};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    dma_rx_buffer, dma_tx_buffer,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::{Instant, Rate},
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// Arduino_GFX's default QSPI clock (`ESP32QSPI_FREQUENCY`), which the
/// Waveshare demo runs at.
const SPI_MHZ: u32 = 40;
const TIMING_RUNS: u32 = 20;
/// 12 FPS, like the game (`FRAME_TIME_MS`).
const FRAME_MS: u64 = 83;
const BRIGHTNESS_STEPS: [u8; 4] = [64, 128, 192, 255];

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut delay = Delay::new();
    println!("[boot] catode32 s3-bringup (WP3 display), QSPI {} MHz", SPI_MHZ);

    // Pins from docs/board-notes.md. Keys have external pull-ups, LOW = pressed.
    let key_cfg = InputConfig::default().with_pull(Pull::None);
    let boot = Input::new(p.GPIO0, key_cfg);
    let user = Input::new(p.GPIO18, key_cfg);
    let cs = Output::new(p.GPIO12, Level::High, OutputConfig::default());
    let rst = Output::new(p.GPIO39, Level::High, OutputConfig::default());
    let spi = Spi::new(
        p.SPI2,
        Config::default().with_frequency(Rate::from_mhz(SPI_MHZ)).with_mode(Mode::_0),
    )
    .expect("SPI2 config")
    .with_sck(p.GPIO38)
    .with_sio0(p.GPIO4)
    .with_sio1(p.GPIO5)
    .with_sio2(p.GPIO6)
    .with_sio3(p.GPIO7);

    // --- FIFO bus (CPU feeds the 64-byte SPI FIFO) --------------------------
    let mut amoled = Amoled::new(Co5300::new(bus::EspQspi::new(spi, cs), rst), Palette::DEFAULT);
    let t = Instant::now();
    if step("init + clear", amoled.init(&mut delay)) {
        println!("[init] took {} ms; screen should now be black", t.elapsed().as_millis());
    }
    color_fills(&mut amoled, &mut delay);
    orientation(&mut amoled, &mut delay);
    time_flushes(&mut amoled, "fifo");

    // --- Same bus with DMA --------------------------------------------------
    let (bus, rst) = amoled.release().release();
    let (spi, cs) = bus.release();
    // Writes are copied into the TX buffer (one strip) and sent in one DMA
    // transfer; the RX buffer is unused but required.
    let spi = spi.with_dma(p.DMA_CH0).with_buffers(
        dma_rx_buffer!(32).expect("DMA rx buffer"),
        dma_tx_buffer!(STRIP_BYTES).expect("DMA tx buffer"),
    );
    let mut amoled = Amoled::new(Co5300::new(bus::EspQspi::new(spi, cs), rst), Palette::DEFAULT);
    orientation(&mut amoled, &mut delay);
    time_flushes(&mut amoled, "dma");

    power(&mut amoled, &mut delay);
    run(&mut amoled, &boot, &user)
}

/// Log a step's result; true on success.
fn step<T, E: Debug>(what: &str, r: Result<T, E>) -> bool {
    match r {
        Ok(_) => {
            println!("[ok] {}", what);
            true
        }
        Err(e) => {
            println!("[FAIL] {}: {:?}", what, e);
            false
        }
    }
}

fn color_fills<B, R>(amoled: &mut Amoled<B, R>, delay: &mut Delay)
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    for (name, color) in [("RED", 0xF800), ("GREEN", 0x07E0), ("BLUE", 0x001F), ("WHITE", 0xFFFF)] {
        println!("[color] whole screen should be {}", name);
        step("fill", amoled.fill_screen(color));
        delay.delay_ms(1000);
    }
    println!("[color] if RED/BLUE are swapped or colors look noisy, check byte order / MADCTL RGB bit");
    step("clear", amoled.clear());
}

fn orientation<B, R>(amoled: &mut Amoled<B, R>, delay: &mut Delay)
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    let mut frame = [0u8; SRC_BYTES];
    test_pattern(&mut frame, 0);
    step("clear", amoled.clear());
    step("viewport", amoled.flush_frame(&frame, false));
    step("controls", amoled.draw_controls(0));
    println!("[orient] game area (top, 384x192): 1-px border, an upright 'F' at its top-left,");
    println!("[orient]   a filled square at its top-right, a diagonal from the top-left corner.");
    println!("[orient] below it: D-pad left, A/B right, M1/M2 bottom right, dim outlines.");
    println!("[orient] mirrored or rotated 'F' => MADCTL (0x36) needs another value");
    delay.delay_ms(3000);
}

fn time_flushes<B, R>(amoled: &mut Amoled<B, R>, label: &str)
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    let mut frame = [0u8; SRC_BYTES];
    let (mut total, mut worst) = (0u64, 0u64);
    for i in 0..TIMING_RUNS {
        test_pattern(&mut frame, i);
        let t = Instant::now();
        if !step_quiet(amoled.flush_frame(&frame, false)) {
            println!("[{}] flush failed, skipping timing", label);
            return;
        }
        let us = t.elapsed().as_micros();
        total += us;
        worst = worst.max(us);
    }
    println!(
        "[{}] viewport flush (384x192, 147 KB): avg {} us, worst {} us; budget {} us per frame",
        label,
        total / TIMING_RUNS as u64,
        worst,
        FRAME_MS * 1000
    );

    let t = Instant::now();
    let ok = step_quiet(amoled.draw_controls(0xFF)).then(|| t.elapsed().as_micros());
    step_quiet(amoled.draw_controls(0));
    if let Some(us) = ok {
        println!("[{}] controls band redraw (480x240, 230 KB): {} us", label, us);
    }
}

fn power<B, R>(amoled: &mut Amoled<B, R>, delay: &mut Delay)
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    println!("[power] brightness ramp up from dark");
    for level in (0..=255u8).step_by(15) {
        step_quiet(amoled.lcd().set_brightness(level));
        delay.delay_ms(60);
    }
    println!("[power] display OFF for 2 s");
    step("display off", amoled.lcd().set_display_on(false, delay));
    delay.delay_ms(2000);
    step("display on", amoled.lcd().set_display_on(true, delay));
    println!("[power] display ON again: the same picture should be back");
}

fn run<B, R>(amoled: &mut Amoled<B, R>, boot: &Input, user: &Input) -> !
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    println!("[run] animation at 12 FPS. BOOT = invert, USER = brightness step");
    let mut frame = [0u8; SRC_BYTES];
    let mut invert = false;
    let mut level = BRIGHTNESS_STEPS.len() - 1;
    let (mut boot_was, mut user_was) = (false, false);
    let (mut n, mut flush_total, mut flush_worst) = (0u32, 0u64, 0u64);
    let mut report = Instant::now();
    step_quiet(amoled.lcd().set_brightness(BRIGHTNESS_STEPS[level]));

    for tick in 0u32.. {
        let start = Instant::now();

        let (boot_now, user_now) = (boot.is_low(), user.is_low());
        if boot_now && !boot_was {
            invert = !invert;
            println!("[run] invert {}", invert);
        }
        if user_now && !user_was {
            level = (level + 1) % BRIGHTNESS_STEPS.len();
            step_quiet(amoled.lcd().set_brightness(BRIGHTNESS_STEPS[level]));
            println!("[run] brightness {}", BRIGHTNESS_STEPS[level]);
        }
        (boot_was, user_was) = (boot_now, user_now);

        test_pattern(&mut frame, tick);
        let t = Instant::now();
        if !step_quiet(amoled.flush_frame(&frame, invert)) {
            println!("[run] flush failed");
        }
        let us = t.elapsed().as_micros();
        (flush_total, flush_worst, n) = (flush_total + us, flush_worst.max(us), n + 1);
        // One button lit per second (12 frames): exercises partial redraws.
        step_quiet(amoled.draw_controls(1 << ((tick / 12) % 8)));

        if report.elapsed().as_millis() >= 2000 {
            println!("[run] flush avg {} us, worst {} us over {} frames", flush_total / n as u64, flush_worst, n);
            (n, flush_total, flush_worst) = (0, 0, 0);
            report = Instant::now();
        }
        while start.elapsed().as_millis() < FRAME_MS {}
    }
    unreachable!()
}

fn step_quiet<T, E: Debug>(r: Result<T, E>) -> bool {
    match r {
        Ok(_) => true,
        Err(e) => {
            println!("[FAIL] {:?}", e);
            false
        }
    }
}

// --- 128x64 1bpp test pattern ----------------------------------------------

fn set(frame: &mut [u8; SRC_BYTES], x: i32, y: i32) {
    if (0..SRC_W as i32).contains(&x) && (0..SRC_H as i32).contains(&y) {
        let (x, y) = (x as usize, y as usize);
        frame[y * SRC_W as usize / 8 + x / 8] |= 0x80 >> (x % 8);
    }
}

fn fill(frame: &mut [u8; SRC_BYTES], x0: i32, y0: i32, w: i32, h: i32) {
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            set(frame, x, y);
        }
    }
}

/// Border, an upright "F" top-left, a square top-right, a diagonal, and an
/// 8x8 ball bouncing with `t` (0 = parked bottom-right).
fn test_pattern(frame: &mut [u8; SRC_BYTES], t: u32) {
    frame.fill(0);
    let (w, h) = (SRC_W as i32, SRC_H as i32);
    fill(frame, 0, 0, w, 1);
    fill(frame, 0, h - 1, w, 1);
    fill(frame, 0, 0, 1, h);
    fill(frame, w - 1, 0, 1, h);
    // "F": stem, top bar, middle bar.
    fill(frame, 4, 4, 3, 16);
    fill(frame, 4, 4, 12, 3);
    fill(frame, 4, 11, 9, 3);
    fill(frame, w - 14, 4, 10, 10);
    for i in 0..h {
        set(frame, i, i);
    }
    // Ball bouncing inside the border.
    let span_x = (w - 2 - 8) as u32;
    let span_y = (h - 2 - 8) as u32;
    let bounce = |v: u32, span: u32| {
        let p = v % (2 * span);
        (if p < span { p } else { 2 * span - p }) as i32
    };
    let (bx, by) = if t == 0 {
        (w - 10, h - 10)
    } else {
        (1 + bounce(t * 3, span_x), 1 + bounce(t * 2, span_y))
    };
    fill(frame, bx, by, 8, 8);
}
