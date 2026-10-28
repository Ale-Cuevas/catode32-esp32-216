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

/// Game button each physical key stands for: BOOT is back, USER opens the menu.
pub const BOOT_BUTTON: u8 = button::B;
pub const USER_BUTTON: u8 = button::MENU1;

/// Button bits for the two physical keys (true = pressed).
pub const fn key_mask(boot: bool, user: bool) -> u8 {
    (if boot { button::bit(BOOT_BUTTON) } else { 0 }) | (if user { button::bit(USER_BUTTON) } else { 0 })
}

/// Empty reads in a row before a touch counts as released. One dropped read
/// mid-hold must not let go of a held direction.
pub const RELEASE_READS: u8 = 2;

/// The "INT edge starts a touch, then poll until release" pattern, without
/// the I/O. The controller's INT only pulses, so it can't tell whether a
/// finger is still down; the tracker keeps polling until the reads come
/// back empty.
///
/// Per frame: call `on_int_edge` if INT fell since the last frame, then, if
/// `should_poll`, read the points and pass them to `update` (an empty slice
/// for no points or a failed read).
#[derive(Debug, Default)]
pub struct TouchTracker {
    active: bool,
    empty_reads: u8,
    mask: u8,
}

impl TouchTracker {
    pub const fn new() -> Self {
        Self { active: false, empty_reads: 0, mask: 0 }
    }

    pub fn on_int_edge(&mut self) {
        self.active = true;
        self.empty_reads = 0;
    }

    pub fn should_poll(&self) -> bool {
        self.active
    }

    /// Feed one read (panel coordinates) and get the button mask. An empty
    /// read keeps the last mask until `RELEASE_READS` of them in a row.
    pub fn update(&mut self, points: &[(u16, u16)]) -> u8 {
        if points.is_empty() {
            self.empty_reads = self.empty_reads.saturating_add(1);
            if self.empty_reads >= RELEASE_READS {
                self.active = false;
                self.mask = 0;
            }
        } else {
            self.empty_reads = 0;
            self.mask = hit_test(points);
        }
        self.mask
    }

    /// Current mask without a new read.
    pub fn mask(&self) -> u8 {
        self.mask
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

    #[test]
    fn keys_map_to_b_and_menu1_and_combine_with_touch() {
        assert_eq!(key_mask(false, false), 0);
        assert_eq!(key_mask(true, false), bit(B));
        assert_eq!(key_mask(false, true), bit(MENU1));
        assert_eq!(key_mask(true, true), bit(B) | bit(MENU1));
        assert_eq!(hit_test(&[center_of(A)]) | key_mask(true, false), bit(A) | bit(B));
    }

    fn center_of(b: u8) -> (u16, u16) {
        CONTROLS.iter().find(|c| c.button == b).unwrap().hit.center()
    }

    #[test]
    fn tracker_idle_until_int() {
        let mut t = TouchTracker::new();
        assert!(!t.should_poll());
        assert_eq!(t.mask(), 0);
        t.on_int_edge();
        assert!(t.should_poll());
    }

    #[test]
    fn tracker_holds_through_one_dropped_read_then_releases() {
        let mut t = TouchTracker::new();
        t.on_int_edge();
        assert_eq!(t.update(&[center_of(A)]), bit(A));
        assert_eq!(t.update(&[]), bit(A)); // one empty read: still held
        assert!(t.should_poll());
        assert_eq!(t.update(&[center_of(A)]), bit(A)); // empties reset
        assert_eq!(t.update(&[]), bit(A));
        assert_eq!(t.update(&[]), 0); // second empty in a row: released
        assert!(!t.should_poll());
    }

    #[test]
    fn tracker_rearms_on_next_int() {
        let mut t = TouchTracker::new();
        t.on_int_edge();
        t.update(&[]);
        t.update(&[]);
        assert!(!t.should_poll());
        t.on_int_edge();
        assert!(t.should_poll());
        assert_eq!(t.update(&[center_of(B)]), bit(B));
    }

    #[test]
    fn raw_report_to_button_mask() {
        use crate::cst9220::{mock, BOARD_ORIENTATION};
        // Raw coords whose board mapping lands on D-pad up and on A. The
        // mapping swaps then mirrors x: panel (px, py) <- raw (py, 480 - px).
        let (cx, cy) = DPAD_HIT.center();
        let up = (cx, cy - 60);
        let a = center_of(A);
        let raw = |(px, py): (u16, u16)| (py, 480 - px);
        let reply = mock::report(
            &[mock::finger(0, 6, raw(up).0, raw(up).1), mock::finger(1, 6, raw(a).0, raw(a).1)],
            0xAB,
        );
        let (mut touch, _, _) = mock::touch(vec![Some(reply)]);
        let mut pts = [(0, 0); 2];
        let n = touch.read_points(&mut pts).unwrap();
        let mapped: Vec<_> = pts[..n].iter().map(|&p| BOARD_ORIENTATION.map(p)).collect();
        assert_eq!(mapped, vec![up, a]);

        let mut t = TouchTracker::new();
        t.on_int_edge();
        assert_eq!(t.update(&mapped), bit(UP) | bit(A));
    }
}
