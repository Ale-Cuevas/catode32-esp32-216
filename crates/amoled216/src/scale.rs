//! 1bpp 128x64 framebuffer -> 3x RGB565 strips for the CO5300.
//!
//! The panel window is `layout::VIEWPORT`. A full frame is 384x192x2 =
//! 147 KB, so it is streamed in strips of `STRIP_SRC_ROWS` source rows
//! instead of being held in RAM.

use crate::layout::{SCALE, SRC_H, SRC_W};

/// Packed source frame: row-major, MSB-first within each byte. Same layout
/// as core's `render::desktop_fb::DesktopFramebuffer` and the SSD1306 sprites.
pub const SRC_BYTES: usize = SRC_W as usize * SRC_H as usize / 8;
const SRC_ROW_BYTES: usize = SRC_W as usize / 8;

pub const OUT_W: usize = (SRC_W * SCALE) as usize;
const OUT_ROW_BYTES: usize = OUT_W * 2;

/// Source rows per strip. 4 rows -> 12 output rows -> 9216 bytes.
pub const STRIP_SRC_ROWS: usize = 4;
pub const STRIP_BYTES: usize = strip_bytes(STRIP_SRC_ROWS);

const _: () = assert!((SRC_H as usize).is_multiple_of(STRIP_SRC_ROWS));

/// Output bytes for `src_rows` source rows.
pub const fn strip_bytes(src_rows: usize) -> usize {
    src_rows * SCALE as usize * OUT_ROW_BYTES
}

pub const fn rgb888_to_565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3)
}

/// Lit / unlit pixel colors in RGB565.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub fg: u16,
    pub bg: u16,
}

impl Palette {
    /// Same foreground as the desktop sim (#23A5CC), but true black for the
    /// background: unlit AMOLED pixels are off, which saves power and burn-in.
    pub const DEFAULT: Palette = Palette {
        fg: rgb888_to_565(0x23, 0xA5, 0xCC),
        bg: 0x0000,
    };

    /// `invert` swaps fg and bg, like `set_invert` on the SSD1306.
    pub const fn resolve(self, invert: bool) -> Palette {
        if invert {
            Palette { fg: self.bg, bg: self.fg }
        } else {
            self
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Scale source rows `first_src_row..first_src_row + n_rows` into `out` as
/// big-endian RGB565 (`n_rows * 3` rows of 384 px), ready to stream to the
/// panel. Returns the number of bytes written.
///
/// Panics if the rows run past the frame or `out` is shorter than
/// `strip_bytes(n_rows)`.
pub fn scale_strip(
    src: &[u8; SRC_BYTES],
    first_src_row: usize,
    n_rows: usize,
    palette: Palette,
    invert: bool,
    out: &mut [u8],
) -> usize {
    assert!(first_src_row + n_rows <= SRC_H as usize, "rows past end of frame");
    let len = strip_bytes(n_rows);
    assert!(out.len() >= len, "strip buffer too small");

    let p = palette.resolve(invert);
    let (fg, bg) = (p.fg.to_be_bytes(), p.bg.to_be_bytes());
    let scale = SCALE as usize;

    for r in 0..n_rows {
        let src_row = &src[(first_src_row + r) * SRC_ROW_BYTES..][..SRC_ROW_BYTES];
        let base = r * scale * OUT_ROW_BYTES;
        let row = &mut out[base..base + OUT_ROW_BYTES];
        for (bx, &byte) in src_row.iter().enumerate() {
            for bit in 0..8 {
                let color = if byte & (0x80 >> bit) != 0 { fg } else { bg };
                let x = (bx * 8 + bit) * scale;
                for dx in 0..scale {
                    row[(x + dx) * 2..][..2].copy_from_slice(&color);
                }
            }
        }
        // Vertical scaling: repeat the finished row.
        for dy in 1..scale {
            out.copy_within(base..base + OUT_ROW_BYTES, base + dy * OUT_ROW_BYTES);
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    const FG: u16 = 0xF81F;
    const BG: u16 = 0x0841;
    const PAL: Palette = Palette { fg: FG, bg: BG };

    fn px(out: &[u8], x: usize, y: usize) -> u16 {
        let i = y * OUT_ROW_BYTES + x * 2;
        u16::from_be_bytes([out[i], out[i + 1]])
    }

    fn set(src: &mut [u8; SRC_BYTES], x: usize, y: usize) {
        src[y * SRC_ROW_BYTES + x / 8] |= 0x80 >> (x % 8);
    }

    /// Scale the whole frame strip by strip, as the driver will.
    fn full_frame(src: &[u8; SRC_BYTES], invert: bool) -> Vec<u8> {
        let mut out = vec![0u8; strip_bytes(SRC_H as usize)];
        let mut strip = [0u8; STRIP_BYTES];
        for (i, first) in (0..SRC_H as usize).step_by(STRIP_SRC_ROWS).enumerate() {
            let n = scale_strip(src, first, STRIP_SRC_ROWS, PAL, invert, &mut strip);
            out[i * STRIP_BYTES..][..n].copy_from_slice(&strip[..n]);
        }
        out
    }

    #[test]
    fn sizes() {
        assert_eq!(OUT_W, 384);
        assert_eq!(STRIP_BYTES, 4 * 3 * 384 * 2);
        assert_eq!(strip_bytes(SRC_H as usize), 384 * 192 * 2);
    }

    #[test]
    fn rgb565_conversion() {
        assert_eq!(rgb888_to_565(0, 0, 0), 0x0000);
        assert_eq!(rgb888_to_565(0xFF, 0xFF, 0xFF), 0xFFFF);
        assert_eq!(rgb888_to_565(0xFF, 0, 0), 0xF800);
        assert_eq!(rgb888_to_565(0, 0xFF, 0), 0x07E0);
        assert_eq!(rgb888_to_565(0, 0, 0xFF), 0x001F);
    }

    #[test]
    fn blank_and_full_frames() {
        let out = full_frame(&[0u8; SRC_BYTES], false);
        assert!(out.chunks(2).all(|c| c == BG.to_be_bytes()));
        let out = full_frame(&[0xFF; SRC_BYTES], false);
        assert!(out.chunks(2).all(|c| c == FG.to_be_bytes()));
    }

    #[test]
    fn invert_swaps_colors() {
        let out = full_frame(&[0u8; SRC_BYTES], true);
        assert!(out.chunks(2).all(|c| c == FG.to_be_bytes()));
    }

    #[test]
    fn corner_pixels_become_exact_3x3_blocks() {
        for (sx, sy) in [(0usize, 0usize), (127, 63), (8, 1), (7, 62)] {
            let mut src = [0u8; SRC_BYTES];
            set(&mut src, sx, sy);
            let out = full_frame(&src, false);
            for y in 0..192 {
                for x in 0..384 {
                    let lit = x / 3 == sx && y / 3 == sy;
                    assert_eq!(px(&out, x, y), if lit { FG } else { BG }, "({x},{y}) for src ({sx},{sy})");
                }
            }
        }
    }

    #[test]
    fn output_is_big_endian() {
        let mut src = [0u8; SRC_BYTES];
        set(&mut src, 0, 0);
        let mut strip = [0u8; STRIP_BYTES];
        scale_strip(&src, 0, 1, PAL, false, &mut strip);
        assert_eq!(&strip[..2], &[0xF8, 0x1F]);
    }

    #[test]
    fn partial_strip_at_end_of_frame() {
        let mut src = [0u8; SRC_BYTES];
        set(&mut src, 5, 63);
        let mut strip = [0xAAu8; STRIP_BYTES];
        let n = scale_strip(&src, 63, 1, PAL, false, &mut strip);
        assert_eq!(n, strip_bytes(1));
        for y in 0..3 {
            assert_eq!(px(&strip, 15, y), FG);
            assert_eq!(px(&strip, 14, y), BG);
        }
        // Bytes past the returned length are untouched.
        assert!(strip[n..].iter().all(|&b| b == 0xAA));
    }

    #[test]
    #[should_panic(expected = "rows past end of frame")]
    fn rejects_rows_past_frame() {
        let mut strip = [0u8; STRIP_BYTES];
        scale_strip(&[0u8; SRC_BYTES], 62, 4, PAL, false, &mut strip);
    }
}
