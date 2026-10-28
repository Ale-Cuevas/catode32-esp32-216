# Board notes: Waveshare ESP32-S3-Touch-AMOLED-2.16

Facts the Rust port relies on. They come from the `waveshare-amoled-216` skill, which was checked against the Waveshare demo repo at `225a62b` and the schematic. Primary sources:
- https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-2.16/tree/main
- https://spotpear.com/wiki/ESP32-S3-2.16-inch-AMOLED-Display-Capacitive-TouchScreen.html

## Chip
ESP32-S3R8 (Xtensa LX7, 240 MHz), 8 MB OPI PSRAM, 16 MB QIO flash, native USB CDC for flashing and logs.

## Pins

| Function | GPIO |
|---|---|
| LCD QSPI D0..D3 | 4, 5, 6, 7 |
| LCD SCLK / CS / RESET | 38 / 12 / 39 |
| I2C SDA / SCL (shared bus) | 15 / 14 |
| Touch INT / RST | 11 / 40 |
| IMU INT1 / INT2 | 17 / 21 |
| RTC INT | 13 |
| BOOT key | 0 |
| USER key | 18 |
| PWR key sense (SYS_OUT) | 16 (unverified) |
| I2S MCLK / BCLK / LRCK / DOUT / DIN | 42 / 9 / 45 / 8 / 10 |
| Speaker amp enable | 46 |
| SD MOSI / SCK / MISO / CS (SPI) | 1 / 2 / 3 / 41 |

There is **no I/O expander**. Display reset, touch reset and SD CS are plain GPIOs.

## I2C addresses (SDA 15 / SCL 14)

| Chip | Addr |
|---|---|
| CST9220 touch | 0x5A |
| AXP2101 PMIC | 0x34 |
| QMI8658 IMU | 0x6B |
| PCF85063 RTC | 0x51 |
| ES8311 codec | 0x18 |
| ES7210 mic ADC | 0x40 |

## Display (CO5300, 480x480, QSPI)
- Column/row offsets are 0/0. Write MADCTL `0x36 = 0xA0` right after init.
- Partial-update windows must be 2-px aligned: x1/y1 even, x2/y2 odd. Every rect in `crates/amoled216/src/layout.rs` is checked for this at compile time.
- Pixels are RGB565, sent big-endian (Arduino_GFX `MSB_32_16_16_SET` does the same). Not yet seen on the device.
- QSPI protocol (Arduino_GFX `Arduino_ESP32QSPI`): 40 MHz, mode 0, CS driven by hand. Register writes are single-line `0x02`, `cmd<<8` (24-bit), params. Pixels are `0x32` + `0x003C00` on one line, then data on four lines. CS stays low for the whole burst.
- Init (Arduino_GFX `Arduino_CO5300`): reset high 10 ms / low 200 ms / high 200 ms, `SLPOUT`, 120 ms, `FE=00 C4=80 3A=55 53=20 63=FF`, `DISPON`, `51=D0 58=00`, 10 ms, `INVOFF`, then the demo's `MADCTL=0xA0`. Display on/off: `DISPON`→`SLPOUT` / `DISPOFF`→`SLPIN`, 120 ms after each.
- The AXP2101 is left at its power-on defaults. The display needs no PMIC setup in the demo. If the panel stays dark with a clean init log, check this first.
- `setBrightness(0..255)`, display on/off commands are available. Black pixels are off.
- The display takes SPI2, so SD has to use SPI3 (HSPI).

## Touch (CST9220)
- I2C 0x5A, INT 11, RST 40. Max coordinates 480x480.
- For MADCTL 0xA0 the demo uses: swap X/Y = true, mirror X = true, mirror Y = false. Not yet confirmed on the device.
- Use INT to detect a touch start, then poll (~25 ms) until release. Reading only on INT is not proven to report drags or releases.
- The number of simultaneous touch points is still unknown (port plan, open question 5).

## Buttons
- BOOT (GPIO0) and USER (GPIO18) have external 10K pull-ups and read **LOW when pressed**. Both are RTC GPIOs, so both can wake from deep sleep. That gives two physical keys, not just BOOT as the port plan assumed.
- Holding BOOT during reset enters download mode.
- PWR goes to the AXP2101. A long press may cut power, so don't build a "hold PWR" gesture.

## PMIC (AXP2101)
The demo leaves the rails at their power-on defaults and only sets the charge target to 4.2 V. Do the same, and don't change rail voltages without a source.

## Port decisions
- The skill's "single target, replace the old board" rule does **not** apply here. Per CLAUDE.md, C6/C3 stay supported and the S3 becomes another cfg arm (`s3` feature) at the existing seams.
- There is no addressable RGB LED, so `Led` becomes a no-op on `s3`.

## Toolchain (verified 2026-10-28 by building `crates/s3-bringup`)
```bash
cargo install espup espflash --locked   # espflash 4.6.0
espup install --targets esp32s3          # adds the "esp" rustup toolchain + ~/export-esp.sh
. ~/export-esp.sh                         # every new shell (Xtensa GCC on PATH, LIBCLANG_PATH)
cd crates/s3-bringup && cargo build --release   # or `cargo run --release` to flash + monitor
```
`crates/s3-bringup` is its own workspace with `rust-toolchain.toml` (`channel = "esp"`) and a `.cargo/config.toml`. Together they set the target `xtensa-esp32s3-none-elf`, `build-std = ["core"]`, the link args `-nostartfiles -Wl,-Tlinkall.x`, and the runner `espflash flash --monitor --chip esp32s3 --flash-size 16mb`. The root workspace still uses stable and RISC-V, so the C6/C3 builds are unchanged. The same esp crate versions as the C6 firmware build for `esp32s3`: esp-hal 1.2.2 (with `unstable`, needed for half-duplex quad SPI), esp-println 0.18, esp-backtrace 0.20 and esp-bootloader-esp-idf 0.6. esp-radio, esp-rtos and esp-storage have not been tried on the S3 yet (port plan, open question 1).

## Still unverified on the device
Touch swap/mirror flags, number of touch points, RGB565 byte order, PWR-sense polarity on GPIO16, IMU axis orientation, and whether the RTC keeps time through a full power-off.
