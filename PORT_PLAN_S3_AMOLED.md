# Porting Catode32 to the Waveshare ESP32-S3-Touch-AMOLED-2.16

Plan written against `moonbench/Catode32` at commit `e8ff889` (2026-10-23, crate version 0.13.0).
Nothing here has been compiled or run: the analysis sandbox could not download the Rust toolchain.
Items are tagged **[verified]** (read in the repo) or **[assumed]** (needs checking on your machine / against the board skill).

---

## 1. Verdict and a correction

**Feasible. Overall difficulty: moderate to hard**, dominated by one item: a no_std QSPI driver for the CO5300 AMOLED.

**Correction to my earlier answer:** I told you the project was MicroPython. That version is frozen. **[verified]** `micropython/README.md` says it is outdated, unmaintained, and stuck at v0.9.1. The active project is a **Rust workspace** (`crates/core`, `crates/firmware`, `crates/desktop`, `crates/i18n-macros`). This plan targets the Rust port.

That change helps you:
- No custom MicroPython firmware or C module is needed. `esp-hal` has quad-SPI plus DMA in Rust **[assumed, verify the API in the pinned esp-hal 1.2.2]**.
- The old "~85KB heap" and "frozen assets" constraints are gone. Assets are Rust consts in `crates/core/src/assets/`.
- The code already has a desktop twin of every hardware seam, which is the template for this port.

Remaining risk, in order:
1. CO5300 QSPI driver and board power/reset sequence (unknown until hardware bring-up).
2. Xtensa toolchain: the S3 is Xtensa, but the repo is configured for RISC-V (C6/C3) only.
3. Touch as the 8 buttons, with multi-touch for held directions.

---

## 2. What the repo looks like (verified)

| Area | File | Fact |
|---|---|---|
| Resolution | `core/src/board.rs` | `DISPLAY_WIDTH=128`, `DISPLAY_HEIGHT=64` consts, used in ~69 places in game code |
| Display | `core/src/render.rs` | `Renderer` wraps an `Ssd1306` in buffered mode. All drawing goes through embedded-graphics `DrawTarget<BinaryColor>`. The only calls the Renderer makes on the display object are `clear_buffer`, `flush`, `set_invert`, `set_display_on`, plus `draw_iter` |
| Desktop twin | `core/src/render.rs` (`desktop_fb`) | `DesktopFramebuffer` is a 1024-byte 1bpp `DrawTarget` with exactly that method set. It is the template for the new backend |
| Input | `core/src/input.rs` | `Buttons` holds `[ButtonPin; 8]` and only calls `is_low()`. On desktop, `ButtonPin` is an atomic-bool "virtual pin" set from keyboard events. Debounce is 50 ms |
| Input usage | whole crate | About 45 files use `was_just_pressed` / `is_pressed`, with ~177 refs to directional buttons. Games need real held state (platformer, zoomies, breakout), so touch must give held state and multi-touch |
| Board init | `core/src/board.rs` | `#[cfg(feature="c6"/"c3")]` pin maps, I2C for the OLED, WS2812 LED on GPIO8. `compile_error!` if neither feature is set |
| Frame loop | `core/src/game.rs` | ~12 FPS, paced by `FRAME_TIME_MS`. Input snapshot at the top of `tick()` (around line 178). `Game::new(renderer, buttons, rng, led, wifi, espnow)` |
| Deep sleep | `core/src/game.rs` `enter_deep_sleep` | C6-specific: LP-IO wake on GPIO0-3. Has a desktop stub |
| Soft sleep | `core/src/sleep_manager.rs` | Display off via `renderer.power_off()`, then polls for any button |
| LED | `core/src/led.rs` | WS2812 via RMT on GPIO8, with a no-op desktop twin |
| Storage | `core/src/storage.rs` | Save data in the ESP-IDF `nvs` partition via `esp-bootloader-esp-idf` + `esp-storage` |
| Radio | `core/src/platform/radio.rs`, `espnow_*` | Wi-Fi scan + ESP-NOW playdates via `esp-radio =1.0.0-beta.1`, `esp-rtos` |
| Firmware entry | `firmware/src/main.rs` | 64 KB heap, `esp_rtos::start`, `Renderer::new(board.i2c)` |
| Build config | `.cargo/config.toml`, `rust-toolchain.toml` | Default target `riscv32imac-unknown-none-elf`, stable toolchain with RISC-V targets only. No Xtensa target |
| Tests | `core/tests/*.rs` | Host tests behind the `desktop` feature, including a smoke test that ticks `Game` and checks pixels |

The crate already follows the rule "core never uses esp_hal directly, go through `platform/` shims". Most of the port is adding a second backend behind seams that already exist.

---

## 3. Target hardware

From Waveshare's pages **[sourced]**: ESP32-S3R8 (dual-core LX7, 240 MHz, 8 MB PSRAM), 16 MB flash, 2.16" AMOLED 480x480 via **CO5300 over QSPI**, **CST9220** capacitive touch over **I2C**, AXP2101 PMIC, QMI8658 IMU, RTC, audio codec, battery header, **BOOT and PWR buttons**. Supported dev frameworks are Arduino and ESP-IDF, with no official Rust or MicroPython support.

Get these from your board skill / Waveshare demo before writing code **[assumed unknown]**:
- QSPI pins (CS, CLK, D0-D3), display reset pin, and whether reset or panel power runs through the AXP2101 rails or an I/O expander.
- I2C pins and addresses (CST9220, AXP2101, QMI8658, RTC), touch INT/RST pins.
- CO5300 init command sequence, window-address offsets (if any), and how the QSPI frame writes a pixel burst (the vendor demo shows it).
- How the PWR button is exposed (likely via AXP2101 IRQ over I2C, not a GPIO).
- Whether the board has an addressable RGB LED (assume **no**).

---

## 4. Architecture decision

**Keep the game 1-bit and 128x64. Scale at flush time. Draw the touch controls separately.**

```
 Game (unchanged) --draw--> 1bpp 128x64 buffer (1 KB)
                                   |
                       Renderer::flush()  (new backend)
                                   v
        scale 3x -> 384x192, tint fg/bg -> RGB565, stream in strips
                                   v
                      CO5300 over QSPI (DMA)

 CST9220 touch --poll once per frame--> hit-test -> 8-bit button mask
                                   v
        "virtual pins" read by Buttons::is_low() (same trick the desktop build uses)
```

Why:
- **Zero changes to scenes, sprites, minigames, i18n, save format.** The art is untouched at the source level.
- **No PSRAM or full framebuffer needed.** Output is streamed in strips (e.g. a few source rows = a dozen output rows, under 10 KB). The existing 64 KB heap stays as is.
- **Bandwidth is fine.** 384x192x2 B = 147 KB/frame, about 1.8 MB/s at 12 FPS, far below QSPI capability **[assumed, measure in WP3]**.
- AMOLED shows true black for "off" pixels, so the monochrome look is good. Foreground tint is configurable (the desktop sim defaults to `#23A5CC` on near-black).
- `set_invert` (used by lightning, `location_scene.rs`) becomes an fg/bg swap during scaling. A full-white flash on AMOLED is harsh, so consider a dimmed invert.

### Screen layout (480x480, constants to tune)
- **Game viewport:** 384x192 (3x), horizontally centered, near the top.
- **Controls area:** the remaining ~250 px below: D-pad at left, A/B at right, Menu1/Menu2 as small buttons. Hit zones should be larger than the drawn shapes.
- Controls are drawn once into the panel and only the pressed highlight is redrawn. They live in the display driver, so core's 1bpp `Renderer` stays unaware.
- Alternative layouts (e.g. rotate the viewport, put the pad on the sides) only change constants in the layout module.

### Buttons: 8 logical, 3 physical
- Touch zones for D-pad, A, B, Menu1, Menu2 (multi-touch, so Up+A works for the platformer).
- BOOT (GPIO0) can be a physical alias, e.g. B/back. PWR via AXP2101 is a later alias (e.g. Menu1). Reset is not usable in software.
- Suggested hook: add one `poll()` at the input snapshot in `Game::tick()`, so the I2C touch read happens once per frame rather than on every `is_low()`.

---

## 5. Work packages

Ordered so most of the work is testable before hardware arrives at your bench.

### WP0: Baseline on your machine
- Run the desktop sim and `cargo test -p catode32-core --features desktop,lang-en`; confirm they pass.
- Install `espup` (Xtensa Rust toolchain) and `espflash`; confirm a "hello world" `xtensa-esp32s3-none-elf` build **[assumed: how the pinned esp-hal 1.2.2 expects this]**.
- Collect the pin map and init sequence from the board skill into a short `docs/board-notes.md`.
- **Done when:** desktop tests pass, S3 hello-world flashes, pin notes are written.

### WP1: Layout + scaler + hit-testing (host-only, unit-tested)
- New crate, e.g. `crates/amoled216`, with a `no_std`-compatible, hardware-free module: layout constants, 1bpp to RGB565 strip scaler (3x, fg/bg/invert), touch-point to button-mask hit-test (with diagonals and multi-touch).
- Unit tests for scaling correctness, edge pixels, and hit zones.
- **Done when:** tests pass on the host.

### WP2: Desktop sim "AMOLED mode"
- Add a flag to `crates/desktop` that opens a 480x480 window using the WP1 scaler and layout, with the mouse acting as touch (`embedded-graphics-simulator` exposes mouse events **[assumed, verify]**).
- This lets you tune layout and feel before touching hardware.
- **Done when:** you can play the game by clicking the on-screen controls.

### WP3: Hardware bring-up (standalone, no Catode32 yet)
- In `crates/amoled216` add a bring-up example: enable required power rails (AXP2101 if needed), reset and init the CO5300, fill the screen, show the scaled test pattern.
- Implement the strip streaming over esp-hal quad SPI + DMA. Measure full-flush time; target well under 83 ms (the 12 FPS frame budget).
- Add brightness control and display on/off (maps to `power_on` / `power_off`).
- **Done when:** a stable image, correct orientation/offsets, flush time measured and logged.
- **This is the highest-uncertainty step.** Expect iteration on init values, DMA buffer sizing, and clocks.

### WP4: Touch driver
- CST9220 over I2C: read touch points (confirm multi-touch count), handle INT/RST, map raw coords to the same orientation as the display.
- Share the I2C bus with the PMIC/IMU/RTC (e.g. via `embedded-hal-bus`), since those live on the same bus **[assumed]**.
- **Done when:** a test screen draws dots under two fingers simultaneously.

### WP5: Integrate into core (keep the diff small)
Target a minimal patch to upstream files, since upstream is active and you may want to rebase:
- `core/Cargo.toml`, `firmware/Cargo.toml`: add an `s3` feature mirroring the `c6`/`c3` blocks (esp-hal, esp-println, esp-storage, esp-bootloader-esp-idf, esp-radio, esp-rtos with `esp32s3`; confirm each crate version supports it **[assumed]**).
- `core/src/board.rs`: `s3` init (shared I2C, SPI + DMA, GPIO0, no LED). Keep the `compile_error!` logic consistent.
- `core/src/render.rs`: add `#[cfg(feature="s3")] type Display = amoled216::Display`, plus `Renderer::new(...)`. Reuse the `desktop_fb` method set.
- `core/src/input.rs`: an `s3` `ButtonPin` that reads a shared mask (updated by touch plus BOOT).
- `core/src/led.rs`: no-op `Led` for `s3` (or a small on-screen dot).
- `core/src/game.rs`: input `poll()` hook; `s3` branch for `enter_deep_sleep` (see WP6).
- `firmware/src/main.rs`: wire it all together.
- Build setup: `.cargo/config.toml` target block for `xtensa-esp32s3-none-elf`, and use the `esp` toolchain via `cargo +esp ...` **[assumed: exact flags such as build-std]** rather than changing the default `rust-toolchain.toml`, so C6/C3 builds still work.
- Flash/partitions: confirm the default partition table includes `nvs` and that `espflash` is told the 16 MB flash size.
- **Done when:** the game boots to the pet, responds to touch, saves and reloads.

### WP6: Power and sleep
- Phase 1: skip deep sleep on `s3`; use the existing soft sleep (display off, poll for touch/BOOT). **Done in WP5.**
- Phase 2: deep sleep with wake on BOOT (GPIO0) and USER (GPIO18). **Built, compile-only:** esp-hal 1.2.2 uses the same `listen` + `apply_wakeup_config(low_power_path)` API as the C6, backed by ext1 any-low on the S3. Touch does **not** wake it (decided: standby current and accidental touches). Fallback: `--features s3-sleep-standin`.
- Battery level via AXP2101: **built, host-tested.** `amoled216::axp2101` + `core/src/s3/pmu.rs` behind `platform::power::status()`; an always-visible status bar (`amoled216::status`). Power policy: basic sleep turns into deep sleep only on battery at ≤5% (the pet's clock stops in deep sleep without an RTC). PWR button: driver flag only, not used yet.
- Burn-in mitigation: **built, host-tested** (`amoled216::burnin`). Controls and status bar dim after 30 s, the panel after 2 min, and a ±4 px orbit runs every 2 min.
- Remaining (needs the board): `docs/DEVICE_TESTS.md` WP6 steps 22–28 and the deferred list there (GPIO16 PWR sense, PWR short-press flag, S3 light sleep for basic sleep, GPIO46 amp).
- **Done when:** idle display turns off, wakes reliably, battery use is sane.

### WP7: Optional polish
- Fg color themes (e.g. by time of day), brightness setting in the menu.
- Skip flushing unchanged frames (compare the 1 KB buffer) to save power.
- IMU shake/tilt as extra input; RTC for wall-clock features.
- ESP-NOW playdates between the S3 and original C6/C3 devices (protocol is chip-agnostic; test it).

---

## 6. Effort estimate (rough, assumes Claude Code does most of the typing)
- WP0-WP2: about a day.
- WP3: half a day if the vendor demo's init sequence ports cleanly, several days if QSPI/DMA or power sequencing fights back. **Dominates the schedule.**
- WP4: half a day.
- WP5: about a day.
- WP6-WP7: optional, open-ended.
Total for a playable MVP: roughly 3-6 working days.

## 7. Open questions to resolve early
1. Do all pinned esp-rs crates (`esp-radio =1.0.0-beta.1`, `esp-rtos 0.4.0`, `esp-storage`, `esp-bootloader-esp-idf`) build for `esp32s3` at these versions? **Answer:** yes, `crates/firmware-s3` builds with all of them (compile only; Wi-Fi not yet run on the device, `PENDING_TESTS.md` WP5.6).
2. Exact Xtensa toolchain flow with esp-hal 1.2.2 (target, `build-std`, linker args).
3. Is there an existing Rust CO5300 driver worth reusing, or is it written from the vendor demo? **Answer:** written in-house from Arduino_GFX + the demo (`crates/amoled216/src/co5300.rs`); the CST9220 driver likewise from SensorLib (`cst9220.rs`).
4. Does display power/reset go through the AXP2101 or an I/O expander?
5. How many simultaneous touch points does the CST9220 report? **Library answer:** 2 (SensorLib `CST92XX_MAX_FINGER_NUM`); confirm on the device (`PENDING_TESTS.md` WP4.4).

## 8. Licensing
- Code is MIT. **Artwork is CC BY-NC-ND 4.0** (no derivatives), and the "Catode32" name is trademarked. Personal and educational builds are allowed per the README; commercial use is not.
- Scaling at display time for your own device is fine, but don't edit or redistribute modified art. Keep your changes to code, and read the full `LICENSE` before sharing the port.

---

## 9. Suggested kickoff prompt for Claude Code
> Read `PORT_PLAN_S3_AMOLED.md`. Start with WP0 and WP1: confirm the desktop build and tests pass, then create `crates/amoled216` with the layout constants, the 1bpp-to-RGB565 3x strip scaler, and the touch hit-testing, with unit tests. Don't modify `crates/core` yet. Use my board skill for pin and init details once we reach WP3.
