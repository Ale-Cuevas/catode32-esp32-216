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
