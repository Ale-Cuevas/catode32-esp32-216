# Board setup: first run on the ESP32-S3-Touch-AMOLED-2.16

What to do when the board arrives. It runs the display and touch bring-up firmware (`crates/s3-bringup`) and records the results. The detailed per-step checks are in [`DEVICE_TESTS.md`](DEVICE_TESTS.md).

## 1. Toolchain (once per machine)

Skip this if `crates/s3-bringup` already builds on this machine.

```bash
cargo install espup espflash --locked
espup install --targets esp32s3      # adds the "esp" toolchain + ~/export-esp.sh
```

## 2. Build, flash and open the log

Connect the board over USB-C, then:

```bash
. ~/export-esp.sh                    # every new shell
cd crates/s3-bringup
cargo run --release                  # build, flash, open the serial monitor
```

- If flashing can't connect, hold **BOOT**, tap **RESET**, release BOOT, then run `cargo run --release` again.
- To watch the log again without reflashing: `espflash monitor`.
- Keep the whole serial log. Most results are read from it.

## 3. Watch the run

The firmware runs every step by itself and logs `[ok]` / `[FAIL]`. Things you have to do during the run:

| When the log says | Do |
|---|---|
| `[color] byte-order probe ...` | Look at the screen: it should be blue-green, **not red**. |
| `[orient] MADCTL 0xA0. USER = try the next value ...` | If the "F" in the game area is upright and readable, press **BOOT** (or wait 8 s). If not, press **USER** until it is, then press **BOOT**, and note the `keeping MADCTL 0x..` value. |
| `[touch] ... ok` and `[run] animation at 12 FPS` | Touch tests: tap a few times, then hold still for 2 s. Touch each of the four corners. Put two fingers down at once. Press every on-screen button, and D-pad up + A together. Slowly drag across the empty space between the D-pad and A/B. |
| `[run] ...` | Press **BOOT** (inverts the game area) and **USER** (steps the brightness). |

If the screen shows noise or stays garbled, retry with the slow clock: hold **USER** while tapping RESET, and keep holding until the `[boot]` line appears (QSPI at 10 MHz instead of 40). Don't use BOOT for this, because holding BOOT through reset enters download mode.

## 4. Record the results

- Fill in each line of [`PENDING_TESTS.md`](PENDING_TESTS.md) (WP3.0–WP3.8, WP4.1–WP4.6): replace `PENDING` with the result and the date, and fill in the `____` blanks.
- Save the full serial log, for example as `docs/logs/bringup-YYYY-MM-DD.txt`, or paste it into the chat.
- Copy every `[FAIL]` line verbatim.

## 5. Values that may need changing

| Symptom | Change |
|---|---|
| A MADCTL other than 0xA0 gave the upright picture | `MADCTL_BOARD` in `crates/amoled216/src/co5300.rs` |
| The byte-order probe was red | Byte order in `crates/amoled216/src/scale.rs` / `co5300.rs` (sends big-endian now) |
| Touch dots are mirrored or rotated | `BOARD_ORIENTATION` in `crates/amoled216/src/cst9220.rs` |
| The screen stays dark even though init logged `[ok]` | Compare the `[pmic]` lines with the demo's AXP2101 defaults (`board-notes.md`) |

After a change, run the host tests (`cargo test -p catode32-amoled216 --target x86_64-unknown-linux-gnu`) and update the tests that expect the old value. Then reflash and repeat steps 2–4.

## 6. Next

Once WP3 and WP4 pass, the next step is WP5 (integrating the S3 into `crates/core`), as described in `PORT_PLAN_S3_AMOLED.md`.
