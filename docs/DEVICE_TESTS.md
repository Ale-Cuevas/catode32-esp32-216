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

## WP5: the game on the S3 (`crates/firmware-s3`)

Run this once WP3 and WP4 pass. If either of them changed `MADCTL_BOARD`, the byte order or `BOARD_ORIENTATION`, those fixes carry over, since the game uses the same `amoled216` code.

```bash
. ~/export-esp.sh
cd crates/firmware-s3
cargo run --release     # build, flash, open the serial monitor
```

Controls: the on-screen D-pad, A, B, M1 and M2. **BOOT** is B (back) and **USER** is Menu1.

| # | What the log says | What to do / check | If it's wrong |
|---|---|---|---|
| 15 | `[s3] touch Cst9220, fw 0x...`, then `catode32 v0.13.0 ... (ESP32-S3 AMOLED)` | The pet appears in the game area, 3x size, with the controls below it. | `[s3] display init failed` / `[s3] touch probe failed`: copy the line. A blank screen with a clean log: compare with WP3 step 2. |
| 16 | (none) | Play: walk the pet with the D-pad, open the menu with M1 and with **USER**, go back with B and with **BOOT**. Hold a direction and press A at the same time (e.g. in the platformer). The pressed control lights up. | Note any button that does nothing or acts as another one. `[s3] touch read failed` appears once if the touch chip stops answering. |
| 17 | `[Sleep] Entering basic sleep` after the idle timeout (15 min) | Leave it alone until the screen turns off. Then tap the screen, or press BOOT. | It should wake on the touch and on BOOT/USER. If only the keys wake it, note it. (Touch wakes basic sleep only, never deep sleep.) |
| 18 | (save on its own schedule, or from the debug menu) | Change something visible (feed the pet, rename it), wait for a save or trigger one, then tap RESET. | After the reboot the change is still there. If not, copy the boot log (storage lines). |
| 19 | (replaced by step 26) | Real deep sleep landed in WP6. The old stand-in is only built with `--features s3-sleep-standin`; run this step only if step 26 fails and you fall back to it: the log says `[Power] Deep sleep (S3 stand-in ...)`, the screen goes dark, touch does nothing, BOOT or USER reboots into the game. | |
| 20 | Wi-Fi scan log lines | Trigger a Wi-Fi scan from the debug menu. | Networks are found and the game keeps running. Copy any radio error. |
| 21 | (none) | Leave the game running for 10 min. | No reset, no `[panic]` record on the Power debug screen, and the animation keeps going. |

## WP6: power, sleep and burn-in (`crates/firmware-s3`)

Same build and flash as WP5. A USB power meter (or a multimeter on the battery lead) helps with steps 22 and 26. Battery = a 3.7 V LiPo on the MX1.25 header.

| # | What the log says | What to do / check | If it's wrong |
|---|---|---|---|
| 22 | `[s3] PMU ok: Status { vbus_in: .., battery_present: .., charge: .., battery_mv: .., battery_pct: .. }` | Boot once on USB without a battery, once with a battery. Compare `battery_mv` with a multimeter on the battery. | `[s3] PMU init failed: WrongChip(..)` or an I2C error: copy it and the WP3.0 I2C scan. mV off by more than ~50: note both values. |
| 23 | (none) | Status bar at the top right. Check: no battery + USB = `USB`; battery on USB = bolt + `NN%` with the icon filled to match; unplug USB = the bolt disappears within ~5 s. | Note what it shows for each case. `--` means the PMIC can't be read (see step 22). |
| 24 | (none) | Leave it alone. After ~30 s the controls and status bar get dimmer; after ~2 min the whole panel dims. Tap a control: everything is bright again **and** the tap acts as a press. | Note the times, and whether dim is still readable or too bright. |
| 25 | (none) | Watch the frame edges and the controls for about 5 min. Every 2 min everything moves by 2 px. | No leftover lit pixels at the frame border or around the controls after a move. Touch zones don't move, so taps near a control's edge still work. |
| 26 | `[Power] Entering deep sleep (wake: BOOT/USER)` | Power debug menu, Deep Sleep. The screen goes dark. Touch the screen: nothing happens. Press **BOOT**: it reboots into the game, and the boot log shows `[Power] reset .., wakeup ..` naming ext1. Repeat with **USER**. | Wakes by itself at once: a wake pad isn't held high, so note it and try the stand-in (`cargo run --release --features s3-sleep-standin`). Never wakes: press RESET and note it. |
| 27 | (none) | On battery only, in deep sleep (step 26): measure the current. Then in basic sleep (step 17) and while running. | Note the three values. This decides whether basic sleep needs S3 light sleep (see the list below). |
| 28 | `[Sleep] Battery low, leaving basic sleep for deep sleep` | Optional, needs a nearly empty battery: on battery, let it reach basic sleep with the status bar at 5% or less. | It saves and deep sleeps. On USB it must stay in basic sleep. |

### Deferred until the board is here

Known unknowns, not built or not settled without hardware. Each one has a line in `PENDING_TESTS.md`.

- **PWR key sense on GPIO16** (SYS_OUT via a BSS138): read the pin while holding PWR briefly. The schematic suggests HIGH while held. Nothing uses it yet.
- **AXP2101 PWR short-press flag**: the driver has `take_power_key_press()` (INTSTS2 bit 3, polled, since the PMIC IRQ line goes to no known GPIO), but the game doesn't call it. Check that a short press sets the flag. If it does, it could toggle sleep. Never build a long-press gesture: a long press may cut power.
- **S3 light sleep during basic sleep**: basic sleep still busy-waits at 2 FPS. Light sleep with a timer wake would save the most power, but esp-rtos and the radio have to survive it. Decide from the step 27 currents.
- **Speaker amp enable (GPIO46)**: nothing drives it yet, so it isn't driven LOW before deep sleep. Revisit if step 27's deep-sleep current is high, or once audio exists.
