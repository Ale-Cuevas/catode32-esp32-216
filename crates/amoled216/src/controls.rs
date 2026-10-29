//! On-screen touch controls, rasterized straight to RGB565 rows.
//!
//! No graphics library: every pixel is classified against the shapes in
//! `layout`, so the firmware can stream the controls area in strips (and
//! only when the pressed mask changes) and the desktop sim draws exactly the
//! same pixels.
//!
//! Idle controls are drawn as dim outlines (AMOLED burn-in: no large static
//! bright areas). A pressed control fills with the full foreground color.

use crate::layout::{button, Offset, Rect, CONTROLS, DPAD_DRAW, SCREEN_W};
use crate::scale::Palette;

/// Colors for the controls, in RGB565.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub idle: u16,
    pub active: u16,
    pub bg: u16,
}

impl Theme {
    /// Idle outlines at 40% of the game's foreground color.
    pub const fn from_palette(p: Palette) -> Self {
        Theme { idle: dim565(p.fg, 2, 5), active: p.fg, bg: p.bg }
    }

    /// After idle (`burnin`): outlines at 15%; a press still lights fully.
    pub const fn dimmed(p: Palette) -> Self {
        Theme { idle: dim565(p.fg, 3, 20), active: p.fg, bg: p.bg }
    }
}

/// Scale each RGB565 channel by `num / den`.
pub const fn dim565(c: u16, num: u16, den: u16) -> u16 {
    let r = ((c >> 11) & 0x1F) * num / den;
    let g = ((c >> 5) & 0x3F) * num / den;
    let b = (c & 0x1F) * num / den;
    (r << 11) | (g << 5) | b
}

/// Outline thickness in px.
const STROKE: i32 = 3;
/// D-pad arm width: a third of the drawn cross.
const ARM: i32 = DPAD_DRAW.w as i32 / 3;
/// Arrow triangle height inside each D-pad arm.
const ARROW_H: i32 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Outside,
    Inside,
    Outline,
    Label,
}

/// Render screen rows `y0..y0 + n_rows` (full 480 px width) as big-endian
/// RGB565 into `out`, with the controls moved by `offset` (burn-in shift;
/// hit zones don't move). Pixels outside every control are `theme.bg`.
/// Returns the number of bytes written.
pub fn render_rows(y0: u16, n_rows: u16, pressed: u8, theme: Theme, offset: Offset, out: &mut [u8]) -> usize {
    let len = n_rows as usize * SCREEN_W as usize * 2;
    assert!(out.len() >= len, "row buffer too small");
    for (i, px) in out[..len].as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let x = (i % SCREEN_W as usize) as i32 - offset.dx as i32;
        let y = (y0 as usize + i / SCREEN_W as usize) as i32 - offset.dy as i32;
        *px = match (u16::try_from(x), u16::try_from(y)) {
            (Ok(x), Ok(y)) => pixel(x, y, pressed, theme),
            _ => theme.bg,
        }
        .to_be_bytes();
    }
    len
}

/// Color of one screen pixel of the controls layer.
pub fn pixel(x: u16, y: u16, pressed: u8, theme: Theme) -> u16 {
    let (part, lit) = classify(x, y, pressed);
    match (part, lit) {
        (Part::Outside, _) => theme.bg,
        (Part::Outline, false) | (Part::Label, false) => theme.idle,
        (Part::Inside, false) | (Part::Label, true) => theme.bg,
        (Part::Outline, true) | (Part::Inside, true) => theme.active,
    }
}

/// Which part of which control (x, y) falls on, and whether that control
/// (or D-pad arm) is pressed.
fn classify(x: u16, y: u16, pressed: u8) -> (Part, bool) {
    if DPAD_DRAW.contains(x, y) {
        return dpad(x as i32 - DPAD_DRAW.x as i32, y as i32 - DPAD_DRAW.y as i32, pressed);
    }
    for c in CONTROLS {
        if c.draw.contains(x, y) {
            let lit = pressed & button::bit(c.button) != 0;
            let part = match rounded_rect(&c.draw, x, y) {
                Part::Inside if in_label(&c.draw, label(c.button), x, y) => Part::Label,
                p => p,
            };
            return (part, lit);
        }
    }
    (Part::Outside, false)
}

// --- D-pad ---------------------------------------------------------------

fn in_cross(lx: i32, ly: i32) -> bool {
    let size = 3 * ARM;
    let mid = |v: i32| (ARM..2 * ARM).contains(&v);
    let full = |v: i32| (0..size).contains(&v);
    (mid(lx) && full(ly)) || (mid(ly) && full(lx))
}

/// Direction bit of the arm (lx, ly) is in, or 0 for the center square.
fn arm(lx: i32, ly: i32) -> u8 {
    if ly < ARM {
        button::bit(button::UP)
    } else if ly >= 2 * ARM {
        button::bit(button::DOWN)
    } else if lx < ARM {
        button::bit(button::LEFT)
    } else if lx >= 2 * ARM {
        button::bit(button::RIGHT)
    } else {
        0
    }
}

fn dpad(lx: i32, ly: i32, pressed: u8) -> (Part, bool) {
    if !in_cross(lx, ly) {
        return (Part::Outside, false);
    }
    let dir = arm(lx, ly);
    let lit = pressed & dir != 0;
    // All 8 neighbors, so the inner corners of the cross close up.
    let edge = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
        .iter()
        .any(|&(dx, dy)| !in_cross(lx + dx * STROKE, ly + dy * STROKE));
    if edge {
        return (Part::Outline, lit);
    }
    // Arrow pointing outward, centered in the arm.
    let c = ARM + ARM / 2;
    let (along, across) = match dir {
        d if d == button::bit(button::UP) => (ly - ARM / 2, lx - c),
        d if d == button::bit(button::DOWN) => (ARM / 2 - (ly - 2 * ARM), lx - c),
        d if d == button::bit(button::LEFT) => (lx - ARM / 2, ly - c),
        d if d == button::bit(button::RIGHT) => (ARM / 2 - (lx - 2 * ARM), ly - c),
        _ => return (Part::Inside, lit),
    };
    let t = along + ARROW_H / 2;
    if (0..ARROW_H).contains(&t) && across.abs() <= t {
        (Part::Label, lit)
    } else {
        (Part::Inside, lit)
    }
}

// --- Round buttons ---------------------------------------------------------

/// Pill / circle: a rounded rect whose corner radius is half its short side.
/// Works in doubled coordinates so pixel centers land on integers.
fn rounded_rect(r: &Rect, x: u16, y: u16) -> Part {
    let px = 2 * (x as i32 - r.x as i32) + 1 - r.w as i32;
    let py = 2 * (y as i32 - r.y as i32) + 1 - r.h as i32;
    let (hw, hh) = (r.w as i32, r.h as i32);
    let rad = hw.min(hh);
    let inside = |shrink: i32| {
        let (hw, hh, rad) = (hw - shrink, hh - shrink, rad - shrink);
        let (ax, ay) = (px.abs(), py.abs());
        let ex = (ax - (hw - rad)).max(0);
        let ey = (ay - (hh - rad)).max(0);
        ax < hw && ay < hh && ex * ex + ey * ey < rad * rad
    };
    if !inside(0) {
        Part::Outside
    } else if !inside(2 * STROKE) {
        Part::Outline
    } else {
        Part::Inside
    }
}

// --- Labels ----------------------------------------------------------------

const GLYPH_W: i32 = 5;
const GLYPH_H: i32 = 7;

/// 5x7 glyphs, one byte per row, bit 4 = leftmost column.
fn glyph(ch: u8) -> [u8; 7] {
    match ch {
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        _ => [0; 7],
    }
}

fn label(b: u8) -> &'static [u8] {
    match b {
        button::A => b"A",
        button::B => b"B",
        button::MENU1 => b"M1",
        button::MENU2 => b"M2",
        _ => b"",
    }
}

/// True if (x, y) is a lit pixel of `text`, centered in `r` at the largest
/// integer scale (max 3) that leaves room for the outline.
fn in_label(r: &Rect, text: &[u8], x: u16, y: u16) -> bool {
    if text.is_empty() {
        return false;
    }
    let n = text.len() as i32;
    let room_w = r.w as i32 - 4 * STROKE;
    let room_h = r.h as i32 - 4 * STROKE;
    let scale = (room_w / (n * (GLYPH_W + 1) - 1)).min(room_h / GLYPH_H).clamp(1, 3);
    let tw = (n * (GLYPH_W + 1) - 1) * scale;
    let th = GLYPH_H * scale;
    let (cx, cy) = r.center();
    let lx = x as i32 - (cx as i32 - tw / 2);
    let ly = y as i32 - (cy as i32 - th / 2);
    if lx < 0 || ly < 0 || lx >= tw || ly >= th {
        return false;
    }
    let (gx, gy) = (lx / scale, ly / scale);
    let (idx, col) = (gx / (GLYPH_W + 1), gx % (GLYPH_W + 1));
    col < GLYPH_W && glyph(text[idx as usize])[gy as usize] & (0x10 >> col) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{button::*, CONTROLS_AREA, SCREEN_H, VIEWPORT};

    const THEME: Theme = Theme { idle: 0x1111, active: 0xFFFF, bg: 0x0000 };

    fn ctrl(b: u8) -> Rect {
        CONTROLS.iter().find(|c| c.button == b).unwrap().draw
    }

    /// Center of a D-pad arm's arrow, and a point in the arm away from the arrow.
    fn arm_points(dir: u8) -> ((u16, u16), (u16, u16)) {
        let (x, y) = (DPAD_DRAW.x as i32, DPAD_DRAW.y as i32);
        let c = ARM + ARM / 2;
        let (ax, ay) = match dir {
            UP => (c, ARM / 2),
            DOWN => (c, 2 * ARM + ARM / 2),
            LEFT => (ARM / 2, c),
            _ => (2 * ARM + ARM / 2, c),
        };
        // Arrow tip area vs. a spot near the inner end of the arm, off the arrow.
        let (ox, oy) = match dir {
            UP => (ARM + 8, ARM - 8),
            DOWN => (ARM + 8, 2 * ARM + 8),
            LEFT => (ARM - 8, ARM + 8),
            _ => (2 * ARM + 8, ARM + 8),
        };
        (((x + ax) as u16, (y + ay) as u16), ((x + ox) as u16, (y + oy) as u16))
    }

    #[test]
    fn dim_halves_channels() {
        assert_eq!(dim565(0xFFFF, 1, 2), (15 << 11) | (31 << 5) | 15);
        assert_eq!(dim565(0xFFFF, 0, 5), 0);
    }

    #[test]
    fn nothing_drawn_outside_controls_area() {
        for y in (0..SCREEN_H).step_by(3) {
            for x in (0..SCREEN_W).step_by(3) {
                if !CONTROLS_AREA.contains(x, y) || VIEWPORT.contains(x, y) {
                    assert_eq!(pixel(x, y, 0xFF, THEME), THEME.bg, "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn round_button_parts() {
        let r = ctrl(A);
        let (cx, cy) = r.center();
        // Rect corner is outside the circle.
        assert_eq!(classify(r.x, r.y, 0), (Part::Outside, false));
        // Leftmost point on the horizontal midline is the ring.
        assert_eq!(classify(r.x, cy, 0).0, Part::Outline);
        // Between the ring and the glyph; the glyph's crossbar runs through the center.
        assert_eq!(classify(r.x + 9, cy, 0).0, Part::Inside);
        assert_eq!(classify(cx, cy, 0).0, Part::Label);
        assert_eq!(pixel(r.x, cy, 0, THEME), THEME.idle);
        assert_eq!(pixel(r.x, cy, bit(A), THEME), THEME.active);
        assert_eq!(pixel(r.x, cy, bit(B), THEME), THEME.idle);
    }

    #[test]
    fn labels_invert_when_pressed() {
        for c in CONTROLS {
            let label_px = (c.draw.y..c.draw.bottom())
                .flat_map(|y| (c.draw.x..c.draw.right()).map(move |x| (x, y)))
                .find(|&(x, y)| classify(x, y, 0).0 == Part::Label)
                .expect("every control has a visible label");
            let (x, y) = label_px;
            assert_eq!(pixel(x, y, 0, THEME), THEME.idle);
            assert_eq!(pixel(x, y, bit(c.button), THEME), THEME.bg);
        }
    }

    #[test]
    fn dpad_arms_light_independently() {
        for dir in [UP, DOWN, LEFT, RIGHT] {
            let (arrow, body) = arm_points(dir);
            assert_eq!(classify(arrow.0, arrow.1, 0).0, Part::Label, "arrow of {dir}");
            assert_eq!(pixel(body.0, body.1, 0, THEME), THEME.bg);
            assert_eq!(pixel(body.0, body.1, bit(dir), THEME), THEME.active);
            for other in [UP, DOWN, LEFT, RIGHT] {
                if other != dir {
                    assert_eq!(pixel(body.0, body.1, bit(other), THEME), THEME.bg);
                }
            }
        }
        // A diagonal lights two arms.
        let mask = bit(UP) | bit(LEFT);
        assert_eq!(pixel(arm_points(UP).1 .0, arm_points(UP).1 .1, mask, THEME), THEME.active);
        assert_eq!(pixel(arm_points(LEFT).1 .0, arm_points(LEFT).1 .1, mask, THEME), THEME.active);
    }

    #[test]
    fn dpad_inner_corner_is_closed() {
        let (x, y) = (DPAD_DRAW.x + ARM as u16, DPAD_DRAW.y + ARM as u16);
        assert_eq!(classify(x, y, 0).0, Part::Outline);
        assert_eq!(classify(x - 1, y - 1, 0).0, Part::Outside);
    }

    #[test]
    fn render_rows_matches_pixel() {
        let y0 = CONTROLS_AREA.y + 100;
        let mut out = vec![0u8; 4 * SCREEN_W as usize * 2];
        let n = render_rows(y0, 4, bit(A), THEME, Offset::ZERO, &mut out);
        assert_eq!(n, out.len());
        for (i, px) in out.chunks(2).enumerate() {
            let (x, y) = ((i % 480) as u16, y0 + (i / 480) as u16);
            assert_eq!(u16::from_be_bytes([px[0], px[1]]), pixel(x, y, bit(A), THEME));
        }
    }

    #[test]
    fn render_rows_shifts_the_drawing() {
        let y0 = CONTROLS_AREA.y + 100;
        let off = Offset::new(-4, 2);
        let mut out = vec![0u8; 4 * SCREEN_W as usize * 2];
        render_rows(y0, 4, bit(A), THEME, off, &mut out);
        for (i, px) in out.chunks(2).enumerate() {
            let (x, y) = ((i % 480) as u16, y0 + (i / 480) as u16);
            let want = if x + 4 >= SCREEN_W { THEME.bg } else { pixel(x + 4, y - 2, bit(A), THEME) };
            assert_eq!(u16::from_be_bytes([px[0], px[1]]), want, "({x},{y})");
        }
    }
}
