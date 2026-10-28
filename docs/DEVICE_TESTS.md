# Device tests: ESP32-S3-Touch-AMOLED-2.16

Checks that need the real board. Everything else is covered by host tests (`cargo test -p catode32-amoled216 --target x86_64-unknown-linux-gnu`). Record each outcome in `PENDING_TESTS.md`. When something fails, paste the serial log.

## Setup (once per shell)

```bash
. ~/export-esp.sh
cd crates/s3-bringup
cargo run --release     # build, flash, open the serial monitor (native USB)
```

If flashing can't connect, hold **BOOT**, tap **RESET**, release BOOT, then run it again.

**Slow clock:** hold **USER** while tapping RESET (keep holding until the `[boot]` line) to run QSPI at 10 MHz instead of 40. Use it when the screen shows noise or garbage at 40 MHz: if 10 MHz is clean, the problem is signal integrity or timing, not the protocol. Don't use BOOT for this, because holding BOOT through a reset enters download mode.

## WP3: display bring-up (`crates/s3-bringup`)

The firmware runs the steps below in order and logs `[ok]` / `[FAIL]` for each. A failed step doesn't stop the rest.

| # | What the log says | What to check on the screen | If it's wrong |
|---|---|---|---|
| 0 | `[i2c] 0x.. answers (...)` for each device, `[i2c] scan done`, then six `[pmic] AXP2101 0x.. ... = 0x..` lines | Nothing on screen yet. Expect 0x18, 0x34, 0x40, 0x51, 0x6B. 0x5A (touch) may only answer after its reset at the touch step. `IC_TYPE` should be 0x4A. | Any `unexpected` or missing address: copy the lines. No devices at all: check SDA 15 / SCL 14. Keep the `[pmic]` lines: if the panel stays dark in step 2, compare them with the demo's PMIC defaults. |
| 1 | `[boot] catode32 s3-bringup ...` | The USB log shows up at all. | Check the USB cable or port. The S3 uses native USB CDC. |
| 2 | `[ok] init + clear`, `[init] took ~540 ms` | The screen goes black, with no noise or stripes. | `[FAIL]` means a bus error, so note it. A dark screen with `[ok]` points at reset, the rails (see the step 0 `[pmic]` lines), or the init sequence. Compare with the Waveshare demo. Noise: try the slow clock. |
| 3 | `[color] whole screen should be RED/GREEN/BLUE/WHITE`, then `[color] byte-order probe ...` | Each color fills the whole screen for 1 s. Then the **byte-order probe** shows for 1.5 s: it should be a **blue-green, not red**. | Red and blue swapped means the RGB/BGR bit in MADCTL. If the probe screen is **red** (and the RED fill above wasn't), the panel wants little-endian: change `scale.rs`/`co5300.rs` to send little-endian. A partly filled screen means window or offsets. |
| 4 | `[orient] ...`, then `[orient] MADCTL 0xA0. USER = try the next value, BOOT = keep this one` | In the top area (384x192): a 1-px border, an **upright, readable "F"** at top-left, a filled square at top-right, and a diagonal from the top-left corner. Below it: D-pad on the left, A/B on the right, M1/M2 at bottom right, as dim outlines. | If the "F" is mirrored or rotated, press **USER** to step through the 8 MADCTL values (each one is logged and redrawn) until the picture is upright, then press **BOOT** (or wait 8 s). Note the value from `[orient] keeping MADCTL 0x..`. If it isn't 0xA0, set `MADCTL_BOARD` in `co5300.rs` to it; the touch mapping (WP4) changes too. |
| 5 | `[fifo] viewport flush ... avg N us` and `[fifo] controls band redraw ...` | The picture stays steady while the timing runs. | Note the numbers. The budget is 83 000 us per frame. |
| 6 | `[dma] viewport flush ...` (after the orientation pattern is redrawn) | Same picture, redrawn over DMA, in the MADCTL kept in step 4. | `[FAIL]` here with FIFO working means a DMA issue, so note the error. |
| 7 | `[power] brightness ramp ...`, `display OFF for 2 s`, `display ON again` | The screen goes from dim to bright, turns fully off for 2 s, then comes back with the same picture. | Note anything that sticks or doesn't come back. |
| 8 | `[run] animation at 12 FPS`, then `[run] flush avg ...` every 2 s | A ball bounces in the game area with no tearing or flicker of the border. Until the first touch, one control lights up per second (cycling through all 8). **BOOT** inverts the game area. **USER** steps the brightness (64/128/192/255). | Tearing or flicker: note it (TE sync is not used yet). If a key does nothing, check its log line. |

Things to record:
- Steps 3 and 4 pass or fail. These settle the byte order and MADCTL (and which MADCTL value you kept).
- The FIFO and DMA flush times (avg/worst) and the controls redraw time.
- The step 0 `[i2c]` / `[pmic]` lines.
- Any `[FAIL]` lines, copied verbatim.
- Whether brightness and on/off behave.

## WP4: touch (`crates/s3-bringup`, same run)

The touch step runs after step 7 and before the animation. In the animation, each finger draws a red dot and the on-screen button under it lights up. Dots in the controls band stay as a trail until the band redraws (tap any button to clear them); in the game area the next frame covers them.

| # | What the log says | What to do / check | If it's wrong |
|---|---|---|---|
| 9 | `[touch] Cst9220 ok, res 480x480, project 0x...., fw 0x........` (the chip may say `Cst9217`) | Nothing to do. | `[FAIL] touch probe: ...`: copy it. `I2c(AcknowledgeCheckFailed(Address))` means no answer at 0x5A after the reset (check RST GPIO40). `BadCheckcode` / `UnknownChip` / `NoFirmware`: copy the value. |
| 10 | `[touch] INT was low for N us` (the first 10 touches) | Tap the screen a few times, then hold a finger still for 2 s. | Record the times. Short pulses (well under a frame) confirm that INT only pulses. If INT stays low for the whole hold, INT is level-triggered and reading could be gated on it. `[touch] got points without an INT edge` means INT (GPIO11) isn't seen. |
| 11 | `[touch] #0 raw (..) -> panel (..)` every 250 ms while touched | Touch near each screen corner: top-left, top-right, bottom-left, bottom-right. The dot should appear **under the finger**, and `panel` should read about (0,0), (479,0), (0,479), (479,479). | Copy the `raw` and `panel` values for each corner. If the dot is mirrored or rotated, only `BOARD_ORIENTATION` in `cst9220.rs` needs to change (the raw values tell which way). If step 4 kept a MADCTL other than 0xA0, expect this to be off. |
| 12 | `[touch] 2 point(s), ...` with `#0` and `#1` | Put two fingers down at once, far apart. | Two dots should appear. If you only ever see `1 point(s)`, record it: the controller reports one point. |
| 13 | `buttons 0b........` | Press each on-screen control: the D-pad (4 directions and the diagonals), A, B, M1, M2. Then hold D-pad up and press A together. | The button under the finger should light up, and the log mask should change. Up+A should light both. A control that lights up away from the finger means a mapping or layout offset; copy the `panel` coordinates. |
| 14 | (none) | Slowly drag a finger across the empty space in the controls band, between the D-pad and A/B. | The red trail should be continuous. Gaps mean reads were missed while held; note how big they are. |

Things to record:
- The step 9 line (chip, resolution, firmware).
- The INT low times from step 10.
- The raw and panel values for the four corners (step 11).
- Whether two points work (step 12) and whether the trail is continuous (step 14).
