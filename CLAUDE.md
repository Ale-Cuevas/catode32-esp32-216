# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Catode32 is a virtual pet game for ESP32 (1-bit 128x64 SSD1306 OLED, 8 buttons, WS2812 LED, Wi-Fi/ESP-NOW). The active codebase is the **Rust workspace** under `crates/`. `micropython/` is a frozen, unmaintained older version (v0.9.1). Don't change it unless someone asks.

## Goal of this fork

**The goal is to port this digital pet to the Waveshare ESP32-S3-Touch-AMOLED-2.16** (2.16" 480x480 CO5300 QSPI AMOLED, CST9220 touch, AXP2101 PMIC). The plan is in `PORT_PLAN_S3_AMOLED.md`, organized as work packages WP0–WP7.

**Use the `waveshare-amoled-216` skill whenever you need board-specific information.** That includes pins, display and touch drivers, init sequences, power rails, the PMIC, IMU, RTC and audio, toolchain and flashing setup, bring-up sketches, and device test plans. Load it before writing or changing any code that touches the S3 hardware, and don't guess board details that the skill covers.

For anything the skill doesn't cover, check these board resources (Arduino/ESP-IDF, not Rust):

- Waveshare demo repo (examples, pin config, schematic, CO5300 init): <https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-2.16/tree/main>
- Spotpear wiki (specs, pinout, setup guides): <https://spotpear.com/wiki/ESP32-S3-2.16-inch-AMOLED-Display-Capacitive-TouchScreen.html>

`docs/board-notes.md` summarizes the pins and display/touch facts the Rust port relies on. Checks that need the board go in `docs/DEVICE_TESTS.md` (steps) and `docs/PENDING_TESTS.md` (one line each, with results). `crates/amoled216` holds the hardware-free 480x480 layout, the 1bpp→RGB565 scaler, the touch hit-testing and `TouchTracker`, the CO5300 driver (written against a `QspiBus` trait, host-tested with a mock bus) and the CST9220 touch driver (written against `embedded_hal::i2c::I2c`, host-tested with a mock). `crates/s3-bringup` implements the bus on esp-hal and runs the display and touch bring-up on the board. The game itself runs on the S3 through core's `s3` feature (`crates/core/src/s3/`), built by `crates/firmware-s3`. Like `s3-bringup`, that crate is its own workspace on the `esp` toolchain, so the root workspace stays on stable/RISC-V. `docs/board-setup.md` is the first-run checklist for the board.

## Commands

The workspace's default build target is `riscv32imac-unknown-none-elf` (`.cargo/config.toml`), and its default member is `crates/firmware`. Host builds and tests therefore need an explicit `--target`.

```bash
# Firmware (ESP32-C6 by default): build, flash via espflash, open serial monitor
cargo run --release
# ESP32-C3 instead of C6
cargo run --release --no-default-features --features c3,lang-en
# Different language (en, de, es, fr, it, nl; enable exactly one)
cargo run --release --no-default-features --features c6,lang-de

# Desktop simulator (SDL2 window; uses the same core crate)
cargo run -p catode32-desktop --target x86_64-unknown-linux-gnu
cargo run -p catode32-desktop --target x86_64-unknown-linux-gnu -- --fg FFAA00 --bg 000000
# 480x480 AMOLED port preview: mouse clicks act as touch on the on-screen controls
cargo run -p catode32-desktop --target x86_64-unknown-linux-gnu -- --amoled

# AMOLED layout/scaler/touch crate tests (host)
cargo test -p catode32-amoled216 --target x86_64-unknown-linux-gnu

# ESP32-S3 display bring-up (standalone crate, Xtensa `esp` toolchain from `espup install`)
cd crates/s3-bringup && . ~/export-esp.sh && cargo build --release   # `cargo run --release` flashes + monitors
# The game on the ESP32-S3 AMOLED board (same toolchain; --no-default-features --features lang-de etc. for other languages)
cd crates/firmware-s3 && . ~/export-esp.sh && cargo build --release  # `cargo run --release` flashes + monitors

# Host tests (core crate, desktop feature)
cargo test -p catode32-core --no-default-features --features desktop,lang-en --target x86_64-unknown-linux-gnu
# Single test file / single test
cargo test -p catode32-core --no-default-features --features desktop,lang-en --target x86_64-unknown-linux-gnu --test game_smoke
cargo test -p catode32-core --no-default-features --features desktop,lang-en --target x86_64-unknown-linux-gnu --test plant_pluck <test_name>
```

Desktop keys: arrows = D-pad, A/S = A/B, Q/W = Menu1/Menu2, ESC saves and quits. The desktop save file is `./catode32-save.json` in the working directory.

Use release builds for on-device testing. Debug builds are much larger and run slower.

If Python is needed for anything (helper scripts, asset or level conversion, tooling), use `uv`: `uv run script.py`, `uv run --with <pkg> ...`, or `uvx <tool>`. Don't use bare `python`/`pip` or create virtualenvs by hand.

More detailed docs live in `docs/`: `docs/architecture.md` (code architecture) and `docs/game-logic.md` (game loop, pet systems, UI). The screenshots in `docs/images/` are real frames from the game. After UI changes, regenerate them with `cargo run -p catode32-desktop --example screenshots --target x86_64-unknown-linux-gnu -- docs/images`. That example boots the game headless into each scene through the crash-resume intent.

## Architecture

### Crates

- `crates/core` (`catode32-core`): all game logic. It is `no_std` on firmware and `std` under the `desktop` feature. A chip feature (`c6` or `c3`) or `desktop` selects the backend. `board.rs` raises a `compile_error!` unless exactly one chip feature is enabled on firmware.
- `crates/firmware`: a thin `no_std` binary. It sets up the heap and `esp_rtos`, calls `board::init`, builds `Renderer`/`Game`, and runs the loop. `panic.rs` records panics to RTC RAM and resets.
- `crates/desktop`: an SDL simulator through `embedded-graphics-simulator`. It maps keys to virtual button pins, calls `Game::tick()`, and blits the 1bpp framebuffer, scaled.
- `crates/i18n-macros`: a proc macro. `t!("key")` resolves to a `&'static str` at compile time from `translations/{lang}.json`, falling back to `en.json` and then to the key itself. Add new UI strings to `en.json`, plus the other languages where possible.

**Platform seam (important).** Code in `core` must never `use esp_hal::…` or `esp_println::…` directly. It has to go through `core/src/platform/` (`persist`, `power`, `radio`, `rng`, `time`, `watchdog`) or use the crate-level `println!` macro in `lib.rs`. These modules swap in std-backed shims under `desktop`. Each hardware-facing module (`render.rs`, `input.rs`, `led.rs`, `storage.rs`, `board.rs`, deep sleep in `game.rs`) has `#[cfg(feature = "desktop")]` twins. A new hardware backend, such as the S3 port, should add another cfg arm at these same seams rather than spreading chip checks through game code.

**Rendering.** `render::Renderer` wraps a buffered `Ssd1306` (or `DesktopFramebuffer` on desktop). All drawing goes through embedded-graphics `DrawTarget<BinaryColor>` at 128x64 (`board::DISPLAY_WIDTH/HEIGHT`, which are referenced throughout game code). The display object must provide `clear_buffer`, `flush`, `set_invert`, `set_display_on` and `draw_iter`. Sprites and art are Rust consts in `core/src/assets/`. Per the LICENSE, the artwork is CC BY-NC-ND, so don't modify it.

**Game loop.** `game::Game::tick()` runs at about 12 FPS (`FRAME_TIME_MS`). It snapshots input, calls `update` on the active scene (or the menu overlay, while the scene gets `tick_background`), then draws. It also handles transitions, sleep (`sleep_manager.rs`), Wi-Fi scans and periodic saves. `context::GameContext` holds all persistent pet and world state: stats, inventory, plants, Wi-Fi familiarity, and so on.

**Scenes.** Each screen or minigame implements `scene::Scene` (`enter`/`exit`/`update`/`draw`/`tick_background`). `update` returns `Some(SceneId)` to switch scenes. Adding a scene takes three steps:

1. Add a variant to `SceneId` in `scene.rs`, and append it to `SceneId::ALL` in declaration order. A const assert enforces the order, and the numeric ID is persisted for crash-resume.
2. Add the module and an entry in the `dispatch_enum!` block for `ActiveScene` in `scenes/mod.rs`.
3. Wire up navigation, usually through menus or `location_scene.rs`.

Locations (inside, outside, bedroom, kitchen, treehouse, vacations) share `location_scene.rs` and `scenes/vacation_base.rs`.

**Behaviors.** The pet's AI lives in `behavior.rs` (selection and transitions) plus one module per behavior in `behaviors/`, with shared helpers in `behaviors/common.rs`. The next behavior is chosen from stats, recent history and randomness.

**Input.** `input::Buttons` holds 8 `ButtonPin`s in this order: UP, DOWN, LEFT, RIGHT, A, B, MENU1, MENU2. It only calls `is_low()`, debounces at 50 ms, and exposes `was_just_pressed` / `is_pressed`. On desktop, the pins are atomic "virtual pins" set by `set_desktop_button`.

**Persistence.** `save.rs` serializes `GameContext` to JSON (`serde-json-core`, fixed-size `heapless` buffers sized to `storage::MAX_PAYLOAD`). On firmware, `storage.rs` writes it to the ESP-IDF `nvs` partition. On desktop, it writes to a file. Changing the save shape must stay backward compatible or bump `SCHEMA_VERSION`. Collections use `heapless` with fixed caps, so check the `*_MAX` consts in `context.rs`.

**Radio.** Wi-Fi scanning (`wifi_tracker.rs`, the "home comfort" mechanic) and ESP-NOW playdates (`espnow_manager.rs`, `espnow_msg.rs`, `scenes/social_scene.rs`) run over `esp-radio` (pinned `=1.0.0-beta.1`) and `esp-rtos`.

**Macros.** `core/src/macros.rs` provides `enum_str_method!`, `enum_key_pair_option!`, `enum_key_pair_default!` and `dispatch_enum!` to generate enum↔string and enum-dispatch boilerplate. Use them instead of hand-written `match` tables.
