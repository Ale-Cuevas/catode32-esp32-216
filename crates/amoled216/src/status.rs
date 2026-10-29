//! Status bar in `layout::STATUS_AREA`, rasterized straight to RGB565 rows
//! like `controls`: battery percentage and a battery icon at the top right,
//! always shown. Port chrome only; the game's 128x64 frame is untouched.
//!
//! - Battery: "87%" and the icon filled to match. At `LOW_PCT` or below,
//!   while not charging, both use the full foreground color as a warning.
//! - Charging: a bolt left of the percentage.
//! - No battery: "USB" on USB power, "--" if the PMIC can't be read.
//!
//! Drawn in the controls' idle color, so it dims with them (`burnin`).

use crate::controls::Theme;
use crate::layout::{Offset, SCREEN_W, STATUS_AREA};

/// What the indicator shows. `None` (no reading) draws "--".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Power {
    /// Fuel gauge 0..=100, `None` without a battery.
    pub battery_pct: Option<u8>,
    pub charging: bool,
    pub usb: bool,
}

/// At or below this, a discharging battery is drawn as a warning.
pub const LOW_PCT: u8 = 15;

/// Glyphs are 3x5 cells scaled by `CELL` (12x20 px).
const CELL: u16 = 4;
const GLYPH_W: u16 = 3 * CELL;
const GLYPH_H: u16 = 5 * CELL;
const GAP: u16 = CELL;
/// Content top edge: centered in the band.
const TOP: u16 = (STATUS_AREA.h - GLYPH_H) / 2;
const RIGHT_MARGIN: u16 = 12;

/// Battery icon: body outline plus a nub on the right.
const BODY_W: u16 = 40;
const STROKE: u16 = 3;
const NUB_W: u16 = 4;
const NUB_H: u16 = 10;
/// Gap between the outline and the level bar.
const PAD: u16 = 2;
const NUB_X: u16 = SCREEN_W - RIGHT_MARGIN - NUB_W;
const BODY_X: u16 = NUB_X - BODY_W;
const FILL_X: u16 = BODY_X + STROKE + PAD;
const FILL_W: u16 = BODY_W - 2 * (STROKE + PAD);
const TEXT_RIGHT: u16 = BODY_X - 2 * GAP;

const _: () = assert!(TOP >= crate::layout::SHIFT_MAX, "shifted content leaves the band");

/// 3x5 bitmaps, one row per entry, bit 2 = left column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    Digit(u8),
    Percent,
    Dash,
    U,
    S,
    B,
    Bolt,
}

impl Glyph {
    const fn rows(self) -> [u8; 5] {
        match self {
            Glyph::Digit(d) => DIGITS[d as usize],
            Glyph::Percent => [0b101, 0b001, 0b010, 0b100, 0b101],
            Glyph::Dash => [0b000, 0b000, 0b111, 0b000, 0b000],
            Glyph::U => [0b101, 0b101, 0b101, 0b101, 0b111],
            Glyph::S => DIGITS[5],
            Glyph::B => [0b110, 0b101, 0b110, 0b101, 0b110],
            Glyph::Bolt => [0b001, 0b010, 0b111, 0b010, 0b100],
        }
    }
}

const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Everything to draw for one `Power`, worked out once per redraw.
struct Content {
    glyphs: [Glyph; 5],
    n: usize,
    /// Left edge of the first glyph.
    text_x: u16,
    fill_w: u16,
    color: u16,
}

impl Content {
    fn new(power: Option<Power>, theme: Theme) -> Self {
        let mut glyphs = [Glyph::Dash; 5];
        let mut n = 0;
        let mut push = |g| {
            glyphs[n] = g;
            n += 1;
        };
        let p = power.unwrap_or_default();
        if p.charging {
            push(Glyph::Bolt);
        }
        match (power, p.battery_pct) {
            (Some(_), Some(pct)) => {
                let pct = pct.min(100);
                if pct >= 100 {
                    push(Glyph::Digit(1));
                }
                if pct >= 10 {
                    push(Glyph::Digit(pct / 10 % 10));
                }
                push(Glyph::Digit(pct % 10));
                push(Glyph::Percent);
            }
            (Some(_), None) if p.usb => {
                push(Glyph::U);
                push(Glyph::S);
                push(Glyph::B);
            }
            _ => {
                push(Glyph::Dash);
                push(Glyph::Dash);
            }
        }
        let width = n as u16 * GLYPH_W + (n as u16 - 1) * GAP;
        let pct = p.battery_pct.map(|v| v.min(100) as u16);
        let low = power.is_some() && pct.is_some_and(|v| v <= LOW_PCT as u16) && !p.charging;
        Content {
            glyphs,
            n,
            text_x: TEXT_RIGHT - width,
            // Round up so any charge shows at least 1 px.
            fill_w: pct.map_or(0, |v| (v * FILL_W).div_ceil(100)),
            color: if low { theme.active } else { theme.idle },
        }
    }

    fn lit(&self, x: u16, y: u16) -> bool {
        if !(TOP..TOP + GLYPH_H).contains(&y) {
            return false;
        }
        let ly = y - TOP;
        // Battery icon.
        if (BODY_X..NUB_X).contains(&x) {
            let lx = x - BODY_X;
            let outline = lx < STROKE || lx >= BODY_W - STROKE || ly < STROKE || ly >= GLYPH_H - STROKE;
            let level = (FILL_X..FILL_X + self.fill_w).contains(&x) && (STROKE + PAD..GLYPH_H - STROKE - PAD).contains(&ly);
            return outline || level;
        }
        if (NUB_X..NUB_X + NUB_W).contains(&x) {
            return ((GLYPH_H - NUB_H) / 2..(GLYPH_H + NUB_H) / 2).contains(&ly);
        }
        // Text.
        if x < self.text_x {
            return false;
        }
        let i = ((x - self.text_x) / (GLYPH_W + GAP)) as usize;
        let gx = (x - self.text_x) % (GLYPH_W + GAP);
        if i >= self.n || gx >= GLYPH_W {
            return false;
        }
        self.glyphs[i].rows()[(ly / CELL) as usize] & (0b100 >> (gx / CELL)) != 0
    }
}

/// Render rows `y0..y0 + n_rows` of the status band (full 480 px width),
/// moved by `offset`, as big-endian RGB565. Returns the bytes written.
pub fn render_rows(y0: u16, n_rows: u16, power: Option<Power>, theme: Theme, offset: Offset, out: &mut [u8]) -> usize {
    assert!(y0 + n_rows <= STATUS_AREA.bottom(), "rows past the status band");
    let len = n_rows as usize * SCREEN_W as usize * 2;
    assert!(out.len() >= len, "row buffer too small");
    let content = Content::new(power, theme);
    let (fg, bg) = (content.color.to_be_bytes(), theme.bg.to_be_bytes());
    for (i, px) in out[..len].as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let x = (i % SCREEN_W as usize) as i32 - offset.dx as i32;
        let y = (y0 as usize + i / SCREEN_W as usize) as i32 - offset.dy as i32;
        let lit = matches!((u16::try_from(x), u16::try_from(y)), (Ok(x), Ok(y)) if content.lit(x, y));
        *px = if lit { fg } else { bg };
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: Theme = Theme { idle: 0x1111, active: 0xFFFF, bg: 0x0000 };

    fn band(power: Option<Power>, offset: Offset) -> Vec<u16> {
        let mut out = vec![0u8; STATUS_AREA.h as usize * SCREEN_W as usize * 2];
        render_rows(0, STATUS_AREA.h, power, THEME, offset, &mut out);
        out.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect()
    }

    fn at(b: &[u16], x: u16, y: u16) -> u16 {
        b[y as usize * SCREEN_W as usize + x as usize]
    }

    /// Text the band shows, decoded back from its pixels.
    fn text(power: Option<Power>) -> String {
        let c = Content::new(power, THEME);
        c.glyphs[..c.n]
            .iter()
            .map(|g| match g {
                Glyph::Digit(d) => char::from(b'0' + d),
                Glyph::Percent => '%',
                Glyph::Dash => '-',
                Glyph::U => 'U',
                Glyph::S => 'S',
                Glyph::B => 'B',
                Glyph::Bolt => '+',
            })
            .collect()
    }

    fn batt(pct: u8, charging: bool) -> Option<Power> {
        Some(Power { battery_pct: Some(pct), charging, usb: charging })
    }

    #[test]
    fn text_for_each_state() {
        assert_eq!(text(batt(87, false)), "87%");
        assert_eq!(text(batt(5, false)), "5%");
        assert_eq!(text(batt(100, true)), "+100%");
        assert_eq!(text(batt(200, false)), "100%");
        assert_eq!(text(Some(Power { battery_pct: None, charging: false, usb: true })), "USB");
        assert_eq!(text(Some(Power::default())), "--");
        assert_eq!(text(None), "--");
    }

    #[test]
    fn text_is_right_aligned_before_the_icon() {
        for p in [batt(5, false), batt(100, true), None] {
            let c = Content::new(p, THEME);
            let end = c.text_x + c.n as u16 * GLYPH_W + (c.n as u16 - 1) * GAP;
            assert_eq!(end, TEXT_RIGHT);
        }
    }

    #[test]
    fn level_bar_tracks_percentage() {
        let y = TOP + GLYPH_H / 2;
        let full = band(batt(100, false), Offset::ZERO);
        // Charging, so no warning color.
        let empty = band(batt(0, true), Offset::ZERO);
        let half = band(batt(50, false), Offset::ZERO);
        for x in FILL_X..FILL_X + FILL_W {
            assert_eq!(at(&full, x, y), THEME.idle, "x {x}");
            assert_eq!(at(&empty, x, y), THEME.bg, "x {x}");
            assert_eq!(at(&half, x, y), if x < FILL_X + FILL_W / 2 { THEME.idle } else { THEME.bg });
        }
        // 1% still shows (charging, so not in the warning color).
        assert_eq!(at(&band(batt(1, true), Offset::ZERO), FILL_X, y), THEME.idle);
        // Outline and nub are always there.
        assert_eq!(at(&empty, BODY_X, y), THEME.idle);
        assert_eq!(at(&empty, NUB_X, y), THEME.idle);
        assert_eq!(at(&empty, NUB_X + NUB_W, y), THEME.bg);
    }

    #[test]
    fn low_battery_warns_unless_charging() {
        let y = TOP;
        assert_eq!(at(&band(batt(LOW_PCT, false), Offset::ZERO), BODY_X, y), THEME.active);
        assert_eq!(at(&band(batt(LOW_PCT, true), Offset::ZERO), BODY_X, y), THEME.idle);
        assert_eq!(at(&band(batt(LOW_PCT + 1, false), Offset::ZERO), BODY_X, y), THEME.idle);
    }

    #[test]
    fn offset_moves_everything() {
        let plain = band(batt(42, true), Offset::ZERO);
        let moved = band(batt(42, true), Offset::new(-4, 4));
        for y in 4..STATUS_AREA.h {
            for x in 0..SCREEN_W - 4 {
                assert_eq!(at(&moved, x, y), at(&plain, x + 4, y - 4), "({x},{y})");
            }
        }
    }

    #[test]
    fn nothing_outside_the_content_rows() {
        let b = band(batt(88, true), Offset::ZERO);
        for x in 0..SCREEN_W {
            assert_eq!(at(&b, x, TOP - 1), THEME.bg);
            assert_eq!(at(&b, x, TOP + GLYPH_H), THEME.bg);
        }
        assert!(b.iter().any(|&c| c == THEME.idle));
    }
}
