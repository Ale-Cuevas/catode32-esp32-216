//! AMOLED burn-in mitigation: what the panel should look like right now.
//!
//! The touch controls (and later the status bar) are the only static
//! pixels, so they dim after a short idle time and the whole panel dims
//! after a longer one. Any press restores both, and the press still acts as
//! a press: dimmed controls stay visible, so swallowing it would feel like
//! a missed input. Independently of input, everything drawn orbits a ring
//! of offsets up to `layout::SHIFT_MAX` px, one step every
//! `SHIFT_STEP_MS`, so no pixel shows the same edge for long.
//!
//! Pure logic on a millisecond clock the caller supplies; `display` applies
//! it on the S3 and the desktop preview calls it directly.

use crate::co5300::DEFAULT_BRIGHTNESS;
use crate::layout::{Offset, SHIFT_MAX};

/// Idle time before the controls dim.
pub const CONTROLS_DIM_MS: u64 = 30_000;
/// Idle time before the whole panel dims.
pub const PANEL_DIM_MS: u64 = 120_000;
/// Panel brightness (CO5300 `0x51`) while idle.
pub const DIM_BRIGHTNESS: u8 = 0x60;
/// Time on each offset of `PATH`.
pub const SHIFT_STEP_MS: u64 = 120_000;

const S: i8 = SHIFT_MAX as i8;
const H: i8 = S / 2;

/// A ring around the centered position, 2 px per step (32 min per lap).
const PATH: [Offset; 16] = [
    Offset::new(S, 0),
    Offset::new(S, H),
    Offset::new(S, S),
    Offset::new(H, S),
    Offset::new(0, S),
    Offset::new(-H, S),
    Offset::new(-S, S),
    Offset::new(-S, H),
    Offset::new(-S, 0),
    Offset::new(-S, -H),
    Offset::new(-S, -S),
    Offset::new(-H, -S),
    Offset::new(0, -S),
    Offset::new(H, -S),
    Offset::new(S, -S),
    Offset::new(S, -H),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub offset: Offset,
    pub controls_dim: bool,
    pub brightness: u8,
}

impl Look {
    /// No mitigation: centered, bright (bring-up, screenshots).
    pub const PLAIN: Look = Look { offset: Offset::ZERO, controls_dim: false, brightness: DEFAULT_BRIGHTNESS };
}

/// Offset for a point in time.
pub fn offset_at(now_ms: u64) -> Offset {
    PATH[(now_ms / SHIFT_STEP_MS) as usize % PATH.len()]
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BurnIn {
    last_activity_ms: Option<u64>,
}

impl BurnIn {
    pub const fn new() -> Self {
        Self { last_activity_ms: None }
    }

    /// Call once per frame with the clock and the buttons held right now.
    /// The first call counts as activity.
    pub fn update(&mut self, now_ms: u64, pressed: u8) -> Look {
        if pressed != 0 || self.last_activity_ms.is_none() {
            self.last_activity_ms = Some(now_ms);
        }
        let idle = now_ms.saturating_sub(self.last_activity_ms.unwrap_or(now_ms));
        Look {
            offset: offset_at(now_ms),
            controls_dim: idle >= CONTROLS_DIM_MS,
            brightness: if idle >= PANEL_DIM_MS { DIM_BRIGHTNESS } else { DEFAULT_BRIGHTNESS },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_in_two_stages_and_any_press_restores() {
        let mut b = BurnIn::new();
        let t0 = 1_000;
        let look = b.update(t0, 0);
        assert!(!look.controls_dim);
        assert_eq!(look.brightness, DEFAULT_BRIGHTNESS);

        let look = b.update(t0 + CONTROLS_DIM_MS, 0);
        assert!(look.controls_dim);
        assert_eq!(look.brightness, DEFAULT_BRIGHTNESS);

        let look = b.update(t0 + PANEL_DIM_MS, 0);
        assert!(look.controls_dim);
        assert_eq!(look.brightness, DIM_BRIGHTNESS);

        let look = b.update(t0 + PANEL_DIM_MS + 5, 0b0001_0000);
        assert!(!look.controls_dim);
        assert_eq!(look.brightness, DEFAULT_BRIGHTNESS);
        // Holding a button keeps it bright.
        let look = b.update(t0 + 10 * PANEL_DIM_MS, 0b0000_0001);
        assert!(!look.controls_dim);
    }

    #[test]
    fn offset_walks_a_closed_ring_inside_the_margin() {
        for (i, o) in PATH.iter().enumerate() {
            assert!(o.dx.unsigned_abs() as u16 <= SHIFT_MAX && o.dy.unsigned_abs() as u16 <= SHIFT_MAX);
            let n = PATH[(i + 1) % PATH.len()];
            // One small step at a time, including the wrap-around.
            assert!((n.dx - o.dx).abs() <= H && (n.dy - o.dy).abs() <= H, "{o:?} -> {n:?}");
            assert_ne!(*o, n);
        }
        assert_eq!(offset_at(0), PATH[0]);
        assert_eq!(offset_at(SHIFT_STEP_MS - 1), PATH[0]);
        assert_eq!(offset_at(SHIFT_STEP_MS), PATH[1]);
        assert_eq!(offset_at(SHIFT_STEP_MS * PATH.len() as u64), PATH[0]);
    }

    #[test]
    fn dim_brightness_is_dimmer() {
        assert!(DIM_BRIGHTNESS < DEFAULT_BRIGHTNESS);
        assert_eq!(Look::PLAIN.brightness, DEFAULT_BRIGHTNESS);
    }
}
