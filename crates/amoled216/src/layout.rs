//! Screen geometry: where the scaled game frame and the touch controls sit on
//! the 480x480 panel. Drawing (desktop sim, firmware) and hit-testing both
//! read these consts, so the two can't drift apart.
//!
//! Every rect is 2-px aligned (even x/y/w/h) because CO5300 partial-update
//! windows need even start and odd end coordinates. The const asserts at the
//! bottom enforce that, plus "on screen" and "no overlaps".

pub const SCREEN_W: u16 = 480;
pub const SCREEN_H: u16 = 480;

/// Game framebuffer size (`board::DISPLAY_WIDTH/HEIGHT` in core).
pub const SRC_W: u16 = 128;
pub const SRC_H: u16 = 64;
/// Integer upscale. Non-integer scaling of the pixel art looks bad.
pub const SCALE: u16 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> u16 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> u16 {
        self.y + self.h
    }

    pub const fn center(&self) -> (u16, u16) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub const fn contains(&self, px: u16, py: u16) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub const fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    pub const fn contains_rect(&self, o: &Rect) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }

    const fn is_even_aligned(&self) -> bool {
        self.x.is_multiple_of(2) && self.y.is_multiple_of(2) && self.w.is_multiple_of(2) && self.h.is_multiple_of(2)
    }
}

pub const SCREEN: Rect = Rect::new(0, 0, SCREEN_W, SCREEN_H);

/// Scaled game frame: 384x192, centered horizontally, near the top.
pub const VIEWPORT: Rect = Rect::new((SCREEN_W - SRC_W * SCALE) / 2, 24, SRC_W * SCALE, SRC_H * SCALE);

/// Logical button indices, same order and bit positions as
/// `catode32_core::input::Button::ALL` (bit `i` of the mask = index `i`).
pub mod button {
    pub const UP: u8 = 0;
    pub const DOWN: u8 = 1;
    pub const LEFT: u8 = 2;
    pub const RIGHT: u8 = 3;
    pub const A: u8 = 4;
    pub const B: u8 = 5;
    pub const MENU1: u8 = 6;
    pub const MENU2: u8 = 7;

    pub const fn bit(index: u8) -> u8 {
        1 << index
    }
}

/// D-pad: one square zone split into 8 direction sectors by `touch`.
/// `hit` is larger than the drawn cross so near-misses still register.
pub const DPAD_HIT: Rect = Rect::new(34, 256, 192, 192);
pub const DPAD_DRAW: Rect = Rect::new(46, 268, 168, 168);
/// Touches closer than this to the D-pad center report no direction.
pub const DPAD_DEADZONE: u16 = 16;

/// A single-button touch zone.
#[derive(Clone, Copy, Debug)]
pub struct Control {
    pub button: u8,
    pub hit: Rect,
    pub draw: Rect,
}

/// A/B on the right in the usual diagonal (B lower-left of A); Menu1/Menu2
/// as smaller buttons along the bottom right.
pub const CONTROLS: [Control; 4] = [
    Control { button: button::A, hit: Rect::new(370, 276, 80, 80), draw: Rect::new(380, 286, 60, 60) },
    Control { button: button::B, hit: Rect::new(282, 336, 80, 80), draw: Rect::new(292, 346, 60, 60) },
    Control { button: button::MENU1, hit: Rect::new(270, 418, 80, 60), draw: Rect::new(280, 432, 60, 32) },
    Control { button: button::MENU2, hit: Rect::new(370, 418, 80, 60), draw: Rect::new(380, 432, 60, 32) },
];

/// Full-width band holding every control. The firmware redraws only this
/// band when the pressed mask changes; the rest of the screen stays black.
pub const CONTROLS_AREA: Rect = Rect::new(0, 240, SCREEN_W, SCREEN_H - 240);

// --- compile-time checks -------------------------------------------------

/// Minimum hit-zone edge, about 7 mm on this panel.
const MIN_HIT: u16 = 60;

const fn check_rect(r: &Rect) {
    assert!(r.is_even_aligned(), "rect not 2-px aligned");
    assert!(SCREEN.contains_rect(r), "rect off screen");
}

const _: () = {
    check_rect(&VIEWPORT);
    check_rect(&DPAD_HIT);
    check_rect(&DPAD_DRAW);
    assert!(DPAD_HIT.contains_rect(&DPAD_DRAW));
    assert!(DPAD_HIT.w == DPAD_HIT.h, "D-pad sectors assume a square zone");
    assert!(!DPAD_HIT.intersects(&VIEWPORT));
    assert!(DPAD_DRAW.w == DPAD_DRAW.h && DPAD_DRAW.w.is_multiple_of(3), "D-pad cross needs 3 equal arms");
    check_rect(&CONTROLS_AREA);
    assert!(!CONTROLS_AREA.intersects(&VIEWPORT));
    assert!(CONTROLS_AREA.contains_rect(&DPAD_HIT));

    let mut i = 0;
    while i < CONTROLS.len() {
        let c = &CONTROLS[i];
        check_rect(&c.hit);
        check_rect(&c.draw);
        assert!(c.hit.contains_rect(&c.draw), "drawn shape outside its hit zone");
        assert!(c.hit.w >= MIN_HIT && c.hit.h >= MIN_HIT, "hit zone too small");
        assert!(!c.hit.intersects(&VIEWPORT));
        assert!(!c.hit.intersects(&DPAD_HIT));
        assert!(CONTROLS_AREA.contains_rect(&c.hit));
        let mut j = i + 1;
        while j < CONTROLS.len() {
            assert!(!c.hit.intersects(&CONTROLS[j].hit), "control hit zones overlap");
            j += 1;
        }
        i += 1;
    }
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_is_3x_centered() {
        assert_eq!(VIEWPORT, Rect::new(48, 24, 384, 192));
        assert_eq!(VIEWPORT.x, SCREEN_W - VIEWPORT.right());
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10, 20, 4, 6);
        assert!(r.contains(10, 20));
        assert!(r.contains(13, 25));
        assert!(!r.contains(14, 25));
        assert!(!r.contains(13, 26));
        assert!(!r.contains(9, 20));
    }

    #[test]
    fn every_button_has_exactly_one_zone() {
        // D-pad covers UP/DOWN/LEFT/RIGHT; CONTROLS covers the rest once each.
        let mut seen = 0u8;
        for c in CONTROLS {
            assert_eq!(seen & button::bit(c.button), 0);
            seen |= button::bit(c.button);
        }
        assert_eq!(seen, 0b1111_0000);
    }
}
