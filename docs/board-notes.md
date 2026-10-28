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
- Pixels are RGB565. The scaler sends them big-endian. **[assumed, confirm in WP3]**
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

## Toolchain (not installed yet; stage 2)
```bash
cargo install espup espflash
espup install            # Xtensa Rust toolchain ("esp") + export-esp.sh
. ~/export-esp.sh
# S3 builds use the esp toolchain explicitly; rust-toolchain.toml stays on stable for C6/C3:
cargo +esp build --release --target xtensa-esp32s3-none-elf -Zbuild-std=core,alloc ...
```
The exact flags for esp-hal 1.2.2 still need confirming. Flash with `espflash flash --flash-size 16mb`.

## Still unverified on the device
Touch swap/mirror flags, number of touch points, RGB565 byte order, PWR-sense polarity on GPIO16, IMU axis orientation, and whether the RTC keeps time through a full power-off.
