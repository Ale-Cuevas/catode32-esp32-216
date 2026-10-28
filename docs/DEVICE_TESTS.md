# Device tests: ESP32-S3-Touch-AMOLED-2.16

Checks that need the real board. Everything else is covered by host tests (`cargo test -p catode32-amoled216 --target x86_64-unknown-linux-gnu`). Record each outcome in `PENDING_TESTS.md`. When something fails, paste the serial log.

## Setup (once per shell)

```bash
. ~/export-esp.sh
cd crates/s3-bringup
cargo run --release     # build, flash, open the serial monitor (native USB)
```

If flashing can't connect, hold **BOOT**, tap **RESET**, release BOOT, then run it again.

## WP3: display bring-up (`crates/s3-bringup`)

The firmware runs the steps below in order and logs `[ok]` / `[FAIL]` for each. A failed step doesn't stop the rest.

| # | What the log says | What to check on the screen | If it's wrong |
|---|---|---|---|
| 1 | `[boot] catode32 s3-bringup ...` | The USB log shows up at all. | Check the USB cable or port. The S3 uses native USB CDC. |
| 2 | `[ok] init + clear`, `[init] took ~540 ms` | The screen goes black, with no noise or stripes. | `[FAIL]` means a bus error, so note it. A dark screen with `[ok]` points at reset, the rails, or the init sequence. Compare with the Waveshare demo. |
| 3 | `[color] whole screen should be RED/GREEN/BLUE/WHITE` | Each color fills the whole screen for 1 s. | Red and blue swapped means the RGB/BGR bit in MADCTL. Noise or wrong colors means byte order (`scale.rs` sends big-endian). A partly filled screen means window or offsets. |
| 4 | `[orient] ...` | In the top area (384x192): a 1-px border, an **upright, readable "F"** at top-left, a filled square at top-right, and a diagonal from the top-left corner. Below it: D-pad on the left, A/B on the right, M1/M2 at bottom right, as dim outlines. | A mirrored or rotated "F" means `MADCTL_BOARD` (0xA0) in `co5300.rs` needs another value. If you change it, the touch mapping changes too (WP4). |
| 5 | `[fifo] viewport flush ... avg N us` and `[fifo] controls band redraw ...` | The picture stays steady while the timing runs. | Note the numbers. The budget is 83 000 us per frame. |
| 6 | `[dma] viewport flush ...` (after the orientation pattern is redrawn) | Same picture, redrawn over DMA. | `[FAIL]` here with FIFO working means a DMA issue, so note the error. |
| 7 | `[power] brightness ramp ...`, `display OFF for 2 s`, `display ON again` | The screen goes from dim to bright, turns fully off for 2 s, then comes back with the same picture. | Note anything that sticks or doesn't come back. |
| 8 | `[run] animation at 12 FPS`, then `[run] flush avg ...` every 2 s | A ball bounces in the game area with no tearing or flicker of the border. One control lights up per second (cycling through all 8). **BOOT** inverts the game area. **USER** steps the brightness (64/128/192/255). | Tearing or flicker: note it (TE sync is not used yet). If a key does nothing, check its log line. |

Things to record:
- Steps 3 and 4 pass or fail. These settle the byte order and MADCTL.
- The FIFO and DMA flush times (avg/worst) and the controls redraw time.
- Any `[FAIL]` lines, copied verbatim.
- Whether brightness and on/off behave.
