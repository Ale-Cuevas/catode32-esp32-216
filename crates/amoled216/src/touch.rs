//! Touch points -> 8-button mask, in the bit order `input::Buttons` uses.
//!
//! Every point is tested on its own and the results are OR-ed, so multi-touch
//! gives combos such as Up+A (needed by the platformer).

use crate::layout::{button, Rect, CONTROLS, DPAD_DEADZONE, DPAD_HIT};

/// Bit mask for all touch points in panel coordinates (0..480).
pub fn hit_test(points: &[(u16, u16)]) -> u8 {
    points.iter().fold(0, |mask, &(x, y)| mask | hit_point(x, y))
}

/// Bit mask for a single touch point.
pub fn hit_point(x: u16, y: u16) -> u8 {
    if DPAD_HIT.contains(x, y) {
        return dpad_direction(&DPAD_HIT, x, y);
    }
    CONTROLS
        .iter()
        .find(|c| c.hit.contains(x, y))
        .map_or(0, |c| button::bit(c.button))
}

/// tan(22.5°) as a fraction. Splits the circle into 8 equal 45° sectors.
const TAN_22_5_NUM: i32 = 41_421;
const TAN_22_5_DEN: i32 = 100_000;

/// Direction bits for a point in the D-pad zone: a cardinal direction, or
/// two bits for a diagonal. Nothing inside the dead zone.
fn dpad_direction(zone: &Rect, x: u16, y: u16) -> u8 {
    let (cx, cy) = zone.center();
    let dx = x as i32 - cx as i32;
    let dy = y as i32 - cy as i32;
    let dead = DPAD_DEADZONE as i32;
    if dx * dx + dy * dy < dead * dead {
        return 0;
    }

    let (ax, ay) = (dx.abs(), dy.abs());
    let horizontal = if dx < 0 { button::bit(button::LEFT) } else { button::bit(button::RIGHT) };
    let vertical = if dy < 0 { button::bit(button::UP) } else { button::bit(button::DOWN) };
    if ay * TAN_22_5_DEN < ax * TAN_22_5_NUM {
        horizontal
    } else if ax * TAN_22_5_DEN < ay * TAN_22_5_NUM {
        vertical
    } else {
        horizontal | vertical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{button::*, VIEWPORT};

    fn dpad_at(dx: i32, dy: i32) -> u8 {
        let (cx, cy) = DPAD_HIT.center();
        hit_point((cx as i32 + dx) as u16, (cy as i32 + dy) as u16)
    }

    #[test]
    fn dpad_cardinals() {
        assert_eq!(dpad_at(0, -60), bit(UP));
        assert_eq!(dpad_at(0, 60), bit(DOWN));
        assert_eq!(dpad_at(-60, 0), bit(LEFT));
        assert_eq!(dpad_at(60, 0), bit(RIGHT));
        // Slightly off-axis still reads as a cardinal (inside ±22.5°).
        assert_eq!(dpad_at(60, -20), bit(RIGHT));
        assert_eq!(dpad_at(20, 60), bit(DOWN));
    }

    #[test]
    fn dpad_diagonals() {
        assert_eq!(dpad_at(-50, -50), bit(UP) | bit(LEFT));
        assert_eq!(dpad_at(50, -50), bit(UP) | bit(RIGHT));
        assert_eq!(dpad_at(-50, 50), bit(DOWN) | bit(LEFT));
        assert_eq!(dpad_at(50, 50), bit(DOWN) | bit(RIGHT));
        assert_eq!(dpad_at(60, -30), bit(UP) | bit(RIGHT));
    }

    #[test]
    fn dpad_deadzone() {
        assert_eq!(dpad_at(0, 0), 0);
        assert_eq!(dpad_at(10, -10), 0);
        assert_eq!(dpad_at(0, -(DPAD_DEADZONE as i32)), bit(UP));
    }

    #[test]
    fn dpad_zone_corners_still_register() {
        let r = DPAD_HIT;
        assert_eq!(hit_point(r.x, r.y), bit(UP) | bit(LEFT));
        assert_eq!(hit_point(r.right() - 1, r.bottom() - 1), bit(DOWN) | bit(RIGHT));
    }

    #[test]
    fn control_centers_and_edges() {
        for c in CONTROLS {
            let (x, y) = c.hit.center();
            assert_eq!(hit_point(x, y), bit(c.button));
            assert_eq!(hit_point(c.hit.x, c.hit.y), bit(c.button));
            assert_eq!(hit_point(c.hit.right() - 1, c.hit.bottom() - 1), bit(c.button));
        }
    }

    #[test]
    fn outside_every_zone_is_zero() {
        let (vx, vy) = VIEWPORT.center();
        assert_eq!(hit_point(vx, vy), 0);
        assert_eq!(hit_point(0, 0), 0);
        assert_eq!(hit_point(479, 0), 0);
        assert_eq!(hit_point(479, 479), 0);
        assert_eq!(hit_point(DPAD_HIT.right() + 4, DPAD_HIT.y), 0);
    }

    #[test]
    fn multi_touch_combines() {
        let (cx, cy) = DPAD_HIT.center();
        let a = CONTROLS.iter().find(|c| c.button == A).unwrap().hit.center();
        let b = CONTROLS.iter().find(|c| c.button == B).unwrap().hit.center();
        assert_eq!(hit_test(&[(cx, cy - 60), a]), bit(UP) | bit(A));
        assert_eq!(hit_test(&[a, b]), bit(A) | bit(B));
        assert_eq!(hit_test(&[]), 0);
        assert_eq!(hit_test(&[(cx, cy)]), 0);
    }
}
