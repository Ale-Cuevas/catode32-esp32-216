//! Display and touch bring-up for the ESP32-S3-Touch-AMOLED-2.16 (port plan
//! WP3 + WP4).
//!
//! Runs the CO5300 and CST9220 drivers from `catode32-amoled216` on the real
//! board, one phase at a time, and says over USB what the screen should show:
//!
//! 0. I2C scan and a read-only AXP2101 status dump. Hold USER through reset
//!    to run QSPI at 10 MHz instead of 40.
//! 1. Reset + init, screen cleared to black.
//! 2. Full-screen RED, GREEN, BLUE, WHITE (1 s each), then a byte-swapped
//!    RED: color order and byte order.
//! 3. Orientation pattern in the 3x game viewport plus the touch controls.
//!    USER cycles MADCTL candidates, BOOT keeps the current one.
//! 4. Flush timing over the FIFO bus, then the same over DMA.
//! 5. Brightness ramp, display off for 2 s, back on.
//! 6. Touch probe.
//! 7. Endless 12 FPS animation: BOOT toggles invert, USER cycles brightness.
//!    The controls highlight one button per second until the first touch,
//!    then light up under the fingers; touches leave dots. Timing logged
//!    every 2 s.
//!
//! Each step logs its result and carries on after an error, so one bad
//! step doesn't hide the others. Checklist: `docs/DEVICE_TESTS.md`.

#![no_std]
#![no_main]

mod bus;

use core::{cell::RefCell, fmt::Debug};

use catode32_amoled216::{
    co5300::{Co5300, QspiBus, MADCTL_BOARD},
    cst9220::{Cst9220, BOARD_ORIENTATION, MAX_POINTS},
    layout::{Rect, SCREEN_H, SCREEN_W, SRC_H, SRC_W},
    panel::Amoled,
    scale::{Palette, SRC_BYTES, STRIP_BYTES},
    touch::TouchTracker,
};
use embedded_hal::{delay::DelayNs, digital::OutputPin};
use embedded_hal_bus::i2c::RefCellDevice;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    dma_rx_buffer, dma_tx_buffer,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::{Instant, Rate},
    Blocking,
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// Arduino_GFX's default QSPI clock (`ESP32QSPI_FREQUENCY`), which the
/// Waveshare demo runs at.
const SPI_MHZ: u32 = 40;
/// With USER held through reset: rules out signal integrity when 40 fails.
const SPI_MHZ_SLOW: u32 = 10;
const I2C_KHZ: u32 = 400;
const TIMING_RUNS: u32 = 20;
/// 12 FPS, like the game (`FRAME_TIME_MS`).
const FRAME_MS: u64 = 83;
const BRIGHTNESS_STEPS: [u8; 4] = [64, 128, 192, 255];

/// Every MY/MX/MV combination, board value first. Bit 3 (RGB/BGR) stays 0
/// as in `MADCTL_BOARD`.
const MADCTL_CANDIDATES: [u8; 8] = [MADCTL_BOARD, 0x00, 0x20, 0x40, 0x60, 0x80, 0xC0, 0xE0];
/// How long the MADCTL picker waits for a key before moving on.
const MADCTL_WAIT_MS: u64 = 8000;

/// AXP2101 registers dumped read-only at boot (XPowersLib 0.2.6
/// `REG/AXP2101Constants.h`).
const AXP2101_ADDR: u8 = 0x34;
const AXP2101_CHIP_ID: u8 = 0x4A;
const AXP2101_REGS: [(u8, &str); 6] = [
    (0x03, "IC_TYPE"),
    (0x00, "STATUS1"),
    (0x01, "STATUS2"),
    (0x80, "DC_ONOFF_DVM_CTRL"),
    (0x90, "LDO_ONOFF_CTRL0"),
    (0x91, "LDO_ONOFF_CTRL1"),
];
/// Addresses `docs/board-notes.md` lists on the shared bus.
const I2C_EXPECTED: [(u8, &str); 6] = [
    (0x18, "ES8311"),
    (0x34, "AXP2101"),
    (0x40, "ES7210"),
    (0x51, "PCF85063"),
    (0x5A, "CST9220"),
    (0x6B, "QMI8658"),
];

/// Touch dot size; even, so dots stay 2-px aligned for the CO5300.
const DOT: u16 = 6;
const DOT_COLOR: u16 = 0xF800;
/// How often touch coordinates are logged while a finger is down.
const TOUCH_LOG_MS: u64 = 250;
/// Without an INT edge, still poll this often to catch a dead INT line.
const IDLE_POLL_MS: u64 = 500;
/// INT low-time measurements to log (how INT behaves is still unknown).
const INT_PULSES_LOGGED: u32 = 10;

type SharedI2c<'a> = RefCellDevice<'a, I2c<'a, Blocking>>;

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut delay = Delay::new();

    // Pins from docs/board-notes.md. Keys have external pull-ups, LOW = pressed.
    let key_cfg = InputConfig::default().with_pull(Pull::None);
    let boot = Input::new(p.GPIO0, key_cfg);
    let user = Input::new(p.GPIO18, key_cfg);
    // USER, not BOOT: holding BOOT through reset enters download mode.
    let spi_mhz = if user.is_low() { SPI_MHZ_SLOW } else { SPI_MHZ };
    println!("[boot] catode32 s3-bringup (WP3 display + WP4 touch), QSPI {} MHz", spi_mhz);
    if spi_mhz != SPI_MHZ {
        println!("[boot] USER held at reset: slow QSPI clock");
    }

    // --- Shared I2C bus: scan + PMIC status, touch later ---------------------
    let i2c = I2c::new(p.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(I2C_KHZ)))
        .expect("I2C0 config")
        .with_sda(p.GPIO15)
        .with_scl(p.GPIO14);
    let i2c = RefCell::new(i2c);
    i2c_scan(&mut i2c.borrow_mut());
    axp2101_status(&mut i2c.borrow_mut());

    let cs = Output::new(p.GPIO12, Level::High, OutputConfig::default());
    let rst = Output::new(p.GPIO39, Level::High, OutputConfig::default());
    let spi = Spi::new(
        p.SPI2,
        Config::default().with_frequency(Rate::from_mhz(spi_mhz)).with_mode(Mode::_0),
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
    let madctl = pick_madctl(&mut amoled, &boot, &user);
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
    // Same controller, still configured; only the bus driver changed.
    step_quiet(amoled.lcd().set_madctl(madctl));
    orientation(&mut amoled, &mut delay);
    time_flushes(&mut amoled, "dma");

    power(&mut amoled, &mut delay);

    // --- Touch on the shared I2C bus ----------------------------------------
    let tp_rst = Output::new(p.GPIO40, Level::High, OutputConfig::default());
    let tp_int = Input::new(p.GPIO11, InputConfig::default().with_pull(Pull::Up));
    let mut drv = Cst9220::new(RefCellDevice::new(&i2c), tp_rst);
    let touch = match drv.probe(&mut delay) {
        Ok(info) => {
            println!(
                "[touch] {:?} ok, res {}x{}, project 0x{:04X}, fw 0x{:08X}, checksum 0x{:08X}",
                info.chip, info.res_x, info.res_y, info.project_id, info.fw_version, info.checksum
            );
            if madctl != MADCTL_BOARD {
                println!("[touch] MADCTL is not 0x{:02X}: dots will be off until BOARD_ORIENTATION is changed", MADCTL_BOARD);
            }
            Some(Touch::new(drv, tp_int))
        }
        Err(e) => {
            println!("[FAIL] touch probe: {:?}", e);
            None
        }
    };
    run(&mut amoled, &boot, &user, touch)
}

fn i2c_scan(i2c: &mut I2c<'_, Blocking>) {
    let mut found = 0;
    for addr in 0x08u8..0x78 {
        // A 1-byte read: esp-hal rejects zero-length writes.
        if i2c.read(addr, &mut [0u8; 1]).is_ok() {
            let name = I2C_EXPECTED.iter().find(|(a, _)| *a == addr).map_or("unexpected", |(_, n)| *n);
            println!("[i2c] 0x{:02X} answers ({})", addr, name);
            found += 1;
        }
    }
    for (addr, name) in I2C_EXPECTED {
        // The touch chip may only answer after its reset (done at the touch step).
        if i2c.read(addr, &mut [0u8; 1]).is_err() {
            println!("[i2c] 0x{:02X} ({}) did not answer", addr, name);
        }
    }
    println!("[i2c] scan done, {} devices", found);
}

/// Read-only: the demo leaves the PMIC at its defaults, so nothing is written.
fn axp2101_status(i2c: &mut I2c<'_, Blocking>) {
    for (reg, name) in AXP2101_REGS {
        let mut v = [0u8; 1];
        match i2c.write_read(AXP2101_ADDR, &[reg], &mut v) {
            Ok(()) => println!("[pmic] AXP2101 0x{:02X} {} = 0x{:02X} ({:08b})", reg, name, v[0], v[0]),
            Err(e) => {
                println!("[FAIL] AXP2101 read 0x{:02X}: {:?}", reg, e);
                return;
            }
        }
        if reg == 0x03 && v[0] != AXP2101_CHIP_ID {
            println!("[pmic] IC_TYPE is not 0x{:02X}: not an AXP2101?", AXP2101_CHIP_ID);
        }
    }
    println!("[pmic] bit meanings: XPowersLib REG/AXP2101Constants.h; a dark panel with a clean init log starts here");
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
    // RED with its bytes swapped. A big-endian panel shows 0x00F8, a
    // blue-green; only a little-endian one shows RED.
    println!("[color] byte-order probe: RED here means the panel wants little-endian (scale.rs sends big-endian)");
    step("fill", amoled.fill_screen(0xF800u16.swap_bytes()));
    delay.delay_ms(1500);
    step("clear", amoled.clear());
}

/// USER steps through `MADCTL_CANDIDATES` and redraws the pattern, BOOT
/// keeps the current value. Moves on after `MADCTL_WAIT_MS` without a key.
/// Returns the value in use.
fn pick_madctl<B, R>(amoled: &mut Amoled<B, R>, boot: &Input, user: &Input) -> u8
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    println!(
        "[orient] MADCTL 0x{:02X}. USER = try the next value, BOOT = keep this one ({} s timeout)",
        MADCTL_BOARD,
        MADCTL_WAIT_MS / 1000
    );
    let mut frame = [0u8; SRC_BYTES];
    test_pattern(&mut frame, 0);
    let mut i = 0;
    let (mut boot_was, mut user_was) = (boot.is_low(), user.is_low());
    let mut last_key = Instant::now();
    while last_key.elapsed().as_millis() < MADCTL_WAIT_MS {
        let (boot_now, user_now) = (boot.is_low(), user.is_low());
        if boot_now && !boot_was {
            break;
        }
        if user_now && !user_was {
            i = (i + 1) % MADCTL_CANDIDATES.len();
            step_quiet(amoled.lcd().set_madctl(MADCTL_CANDIDATES[i]));
            // RAM isn't remapped: redraw everything in the new orientation.
            step_quiet(amoled.clear());
            step_quiet(amoled.flush_frame(&frame, false));
            step_quiet(amoled.draw_controls(0));
            println!("[orient] MADCTL 0x{:02X}", MADCTL_CANDIDATES[i]);
            last_key = Instant::now();
        }
        (boot_was, user_was) = (boot_now, user_now);
    }
    let v = MADCTL_CANDIDATES[i];
    if v == MADCTL_BOARD {
        println!("[orient] keeping MADCTL 0x{:02X}", v);
    } else {
        println!("[orient] keeping MADCTL 0x{:02X}: if this is the upright one, set MADCTL_BOARD in co5300.rs to it", v);
    }
    v
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

/// CST9220 plus the INT-edge / poll bookkeeping the game loop will need
/// (`TouchTracker`), and the INT timing this bring-up measures.
struct Touch<'a> {
    drv: Cst9220<SharedI2c<'a>, Output<'a>>,
    int: Input<'a>,
    tracker: TouchTracker,
    /// Falling edge seen since the last frame.
    edge: bool,
    int_was_low: bool,
    int_fell_at: Instant,
    pulses_logged: u32,
    last_idle_poll: Instant,
    warned_no_int: bool,
    /// The demo mode (one button lit per second) ends at the first touch.
    seen: bool,
    last_log: Instant,
}

impl<'a> Touch<'a> {
    fn new(drv: Cst9220<SharedI2c<'a>, Output<'a>>, int: Input<'a>) -> Self {
        let int_was_low = int.is_low();
        Self {
            drv,
            int,
            tracker: TouchTracker::new(),
            edge: false,
            int_was_low,
            int_fell_at: Instant::now(),
            pulses_logged: 0,
            last_idle_poll: Instant::now(),
            warned_no_int: false,
            seen: false,
            last_log: Instant::now(),
        }
    }

    /// Sample INT; called in a tight loop while the frame waits. Latches a
    /// falling edge and logs how long INT stays low.
    fn sample_int(&mut self) {
        let low = self.int.is_low();
        if low && !self.int_was_low {
            self.edge = true;
            self.int_fell_at = Instant::now();
        } else if !low && self.int_was_low && self.pulses_logged < INT_PULSES_LOGGED {
            self.pulses_logged += 1;
            println!("[touch] INT was low for {} us", self.int_fell_at.elapsed().as_micros());
        }
        self.int_was_low = low;
    }

    /// Once per frame: read the points if a touch is in progress (or for an
    /// occasional idle poll), returning them in panel coordinates.
    fn poll(&mut self, out: &mut [(u16, u16); MAX_POINTS]) -> usize {
        self.sample_int();
        if core::mem::take(&mut self.edge) {
            self.tracker.on_int_edge();
        }
        let idle_poll = !self.tracker.should_poll() && self.last_idle_poll.elapsed().as_millis() >= IDLE_POLL_MS;
        if !self.tracker.should_poll() && !idle_poll {
            return 0;
        }
        if idle_poll {
            self.last_idle_poll = Instant::now();
        }

        let mut raw = [(0u16, 0u16); MAX_POINTS];
        let n = match self.drv.read_points(&mut raw) {
            Ok(n) => n,
            Err(e) => {
                println!("[FAIL] touch read: {:?}", e);
                0
            }
        };
        if idle_poll {
            if n == 0 {
                return 0;
            }
            if !self.warned_no_int {
                self.warned_no_int = true;
                println!("[touch] got points without an INT edge: check INT (GPIO11)");
            }
            self.tracker.on_int_edge();
        }
        for i in 0..n {
            out[i] = BOARD_ORIENTATION.map(raw[i]);
        }
        let mask = self.tracker.update(&out[..n]);
        if n > 0 {
            self.seen = true;
            if self.last_log.elapsed().as_millis() >= TOUCH_LOG_MS {
                self.last_log = Instant::now();
                for i in 0..n {
                    println!("[touch] #{} raw {:?} -> panel {:?}", i, raw[i], out[i]);
                }
                println!("[touch] {} point(s), buttons 0b{:08b}", n, mask);
            }
        }
        n
    }
}

/// Small square centered on `p`, 2-px aligned and kept on screen.
fn dot_at((x, y): (u16, u16)) -> Rect {
    let clamp = |v: u16, max: u16| (v.saturating_sub(DOT / 2) & !1).min(max - DOT);
    Rect::new(clamp(x, SCREEN_W), clamp(y, SCREEN_H), DOT, DOT)
}

fn run<B, R>(amoled: &mut Amoled<B, R>, boot: &Input, user: &Input, mut touch: Option<Touch>) -> !
where
    B: QspiBus,
    B::Error: Debug,
    R: OutputPin,
    R::Error: Debug,
{
    println!("[run] animation at 12 FPS. BOOT = invert, USER = brightness step");
    if touch.is_some() {
        println!("[run] touch: dots under each finger, buttons light under the finger; tapping a button clears old dots");
    }
    let mut frame = [0u8; SRC_BYTES];
    let mut invert = false;
    let mut level = BRIGHTNESS_STEPS.len() - 1;
    let (mut boot_was, mut user_was) = (false, false);
    let (mut n, mut flush_total, mut flush_worst) = (0u32, 0u64, 0u64);
    let mut report = Instant::now();
    let mut points = [(0u16, 0u16); MAX_POINTS];
    let mut dot_buf = [0u8; DOT as usize * DOT as usize * 2];
    step_quiet(amoled.lcd().set_brightness(BRIGHTNESS_STEPS[level]));

    for tick in 0u32.. {
        let start = Instant::now();
        let touched = touch.as_mut().map_or(0, |t| t.poll(&mut points));

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
        let mask = match &touch {
            Some(t) if t.seen => t.tracker.mask(),
            // One button lit per second (12 frames): exercises partial redraws.
            _ => 1 << ((tick / 12) % 8),
        };
        step_quiet(amoled.draw_controls(mask));
        // After the controls, so a redraw doesn't hide the new dots. Dots in
        // the controls band stay until it redraws (a trail for drag tests);
        // in the game area the next frame covers them.
        for &p in &points[..touched] {
            step_quiet(amoled.lcd().fill(dot_at(p), DOT_COLOR, &mut dot_buf));
        }

        if report.elapsed().as_millis() >= 2000 {
            println!("[run] flush avg {} us, worst {} us over {} frames", flush_total / n as u64, flush_worst, n);
            (n, flush_total, flush_worst) = (0, 0, 0);
            report = Instant::now();
        }
        while start.elapsed().as_millis() < FRAME_MS {
            if let Some(t) = touch.as_mut() {
                t.sample_int();
            }
        }
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
