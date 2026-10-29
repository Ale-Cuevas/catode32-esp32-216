# Pending device tests

One line per check that has only been compiled or host-tested. Steps are in `DEVICE_TESTS.md`. Replace `PENDING` with the result and the date.

- PENDING: WP3.0 I2C scan finds the expected devices; AXP2101 IC_TYPE = 0x4A; status lines: ____
- PENDING: WP3.1 s3-bringup boots and logs over native USB
- PENDING: WP3.2 CO5300 reset + init gives a clean black screen (40 MHz: ____, 10 MHz with USER held: ____)
- PENDING: WP3.3 full-screen RED/GREEN/BLUE/WHITE are correct, and the byte-order probe is NOT red (RGB565 big-endian, RGB order)
- PENDING: WP3.4 orientation: upright "F" at the top-left of the game area with MADCTL 0xA0 (kept value if not: ____)
- PENDING: WP3.5 FIFO viewport flush time: ____ us avg / ____ us worst; controls redraw: ____ us
- PENDING: WP3.6 DMA viewport flush time: ____ us avg / ____ us worst; controls redraw: ____ us
- PENDING: WP3.7 brightness ramp, display off/on keeps the picture
- PENDING: WP3.8 12 FPS animation is stable; BOOT inverts, USER steps brightness
- PENDING: WP4.1 CST9220 probe ok (chip ____, res ____, fw ____)
- PENDING: WP4.2 INT low time per touch: ____ us (pulse or level?)
- PENDING: WP4.3 corners map to ~(0,0) (479,0) (0,479) (479,479) with BOARD_ORIENTATION swap/mirrorX
- PENDING: WP4.4 two simultaneous points reported
- PENDING: WP4.5 on-screen buttons light under the finger, Up+A combo works
- PENDING: WP4.6 slow drag leaves a continuous trail
- PENDING: WP5.1 firmware-s3 boots to the pet; display and touch init lines clean
- PENDING: WP5.2 all on-screen controls work, BOOT = B, USER = Menu1, held direction + A combo works
- PENDING: WP5.3 soft sleep turns the screen off; touch and BOOT/USER wake it
- PENDING: WP5.4 save survives RESET
- PENDING: WP5.5 (only with `--features s3-sleep-standin`, if WP6.5 fails) deep-sleep stand-in: dark until BOOT/USER, then reboots into the game
- PENDING: WP5.6 Wi-Fi scan works on the S3
- PENDING: WP5.7 10 min run without reset or panic record
- PENDING: WP6.1 `[s3] PMU ok` at boot; battery_mv ____ vs meter ____ mV; battery_pct ____
- PENDING: WP6.2 status bar: `USB` without battery, bolt + % on USB with battery, bolt gone within ~5 s of unplugging
- PENDING: WP6.3 controls dim after ~30 s, panel after ~2 min; a tap restores both and still acts as a press
- PENDING: WP6.4 2 px shift every 2 min leaves no stray pixels; touch zones unaffected
- PENDING: WP6.5 deep sleep: touch doesn't wake; BOOT wakes ____, USER wakes ____; boot log wakeup cause: ____
- PENDING: WP6.6 current on battery: deep sleep ____ mA, basic sleep ____ mA, running ____ mA
- PENDING: WP6.7 (optional, nearly empty battery) basic sleep at <=5% on battery saves and deep sleeps; on USB it doesn't
- PENDING: WP6.8 deferred: GPIO16 reads ____ while PWR is held (expected HIGH)
- PENDING: WP6.9 deferred: AXP2101 PWR short press sets INTSTS2 bit 3 with polling only (yes/no): ____
- PENDING: WP6.10 deferred: decide on S3 light sleep for basic sleep from WP6.6 (and GPIO46 amp LOW before deep sleep if deep-sleep current is high)
