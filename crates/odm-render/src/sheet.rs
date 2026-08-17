//! Contact sheets: composite per-frame tiles into one captioned grid,
//! CPU-side. The GPU renders tiles; this arranges them and stamps each
//! frame's overrides under it with an embedded 8x8 bitmap font.

use crate::RenderError;
use crate::gpu::encode_png;

pub struct Tile {
    /// Tightly packed RGBA, tile-sized.
    pub rgba: Vec<u8>,
    /// Stamped in the strip under the tile; empty leaves the strip blank.
    pub caption: String,
}

/// Caption glyphs are the 8x8 font at 2x — chunky enough to survive the
/// downscaling an image reader applies to large sheets.
const SCALE: u32 = 2;
const GLYPH: u32 = 8 * SCALE;
const PAD: u32 = 3;
/// Height of the caption strip under every tile.
pub const STRIP_H: u32 = GLYPH + 2 * PAD;
/// Gap between cells.
const GUTTER: u32 = 2;

/// Sheet ground (gutters, strips, trailing empty cells), sRGB — darker than
/// the render background so tiles read as separate panes.
const FRAME_RGB: [u8; 3] = [16, 17, 20];
const TEXT_RGB: [u8; 3] = [219, 220, 224];

/// Near-square in *pixels*, not counts: the column count whose sheet is
/// closest to square given the cell aspect. Row-major fill; the last row
/// may be short.
pub fn grid_shape(n: usize, tile_w: u32, tile_h: u32) -> (u32, u32) {
    let n = n.max(1) as u32;
    let (cell_w, cell_h) = (tile_w as f64, (tile_h + STRIP_H) as f64);
    let mut best = (n, 1, f64::INFINITY);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let aspect = (cols as f64 * cell_w) / (rows as f64 * cell_h);
        let badness = aspect.max(1.0 / aspect);
        if badness < best.2 {
            best = (cols, rows, badness);
        }
    }
    (best.0, best.1)
}

pub fn sheet_size(tile_w: u32, tile_h: u32, cols: u32, rows: u32) -> (u32, u32) {
    (cols * tile_w + (cols - 1) * GUTTER, rows * (tile_h + STRIP_H) + (rows - 1) * GUTTER)
}

/// Composite `tiles` row-major into one PNG. Every tile must be
/// `tile_w` x `tile_h`.
pub fn sheet_png(tiles: &[Tile], tile_w: u32, tile_h: u32) -> Result<Vec<u8>, RenderError> {
    let (cols, rows) = grid_shape(tiles.len(), tile_w, tile_h);
    let (sw, sh) = sheet_size(tile_w, tile_h, cols, rows);
    let mut buf = vec![0u8; (sw as usize) * (sh as usize) * 4];
    for px in buf.chunks_exact_mut(4) {
        px[..3].copy_from_slice(&FRAME_RGB);
        px[3] = 255;
    }
    for (i, tile) in tiles.iter().enumerate() {
        let want = (tile_w as usize) * (tile_h as usize) * 4;
        if tile.rgba.len() != want {
            return Err(RenderError::BadOptions(format!(
                "tile {i} is {} bytes, expected {want}",
                tile.rgba.len()
            )));
        }
        let x0 = (i as u32 % cols) * (tile_w + GUTTER);
        let y0 = (i as u32 / cols) * (tile_h + STRIP_H + GUTTER);
        for row in 0..tile_h {
            let src = (row as usize) * (tile_w as usize) * 4;
            let dst = (((y0 + row) as usize) * (sw as usize) + x0 as usize) * 4;
            buf[dst..dst + (tile_w as usize) * 4]
                .copy_from_slice(&tile.rgba[src..src + (tile_w as usize) * 4]);
        }
        stamp(&tile.caption, &mut buf, sw, x0 + 2 * PAD, y0 + tile_h + PAD, tile_w - 4 * PAD);
    }
    encode_png(&buf, sw, sh)
}

/// Draw `text` at (x0, y0), truncated with ".." to fit `max_w` pixels.
/// ASCII only — anything else prints as '?'.
fn stamp(text: &str, buf: &mut [u8], sheet_w: u32, x0: u32, y0: u32, max_w: u32) {
    let max_chars = (max_w / GLYPH) as usize;
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() > max_chars {
        chars.truncate(max_chars.saturating_sub(2));
        chars.extend(['.', '.']);
    }
    for (i, ch) in chars.iter().enumerate() {
        let code = if ch.is_ascii() { *ch as usize } else { b'?' as usize };
        let glyph = font8x8::legacy::BASIC_LEGACY[code];
        for (gy, bits) in glyph.iter().enumerate() {
            for gx in 0..8u32 {
                if bits & (1 << gx) == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        let x = x0 + i as u32 * GLYPH + gx * SCALE + dx;
                        let y = y0 + gy as u32 * SCALE + dy;
                        let at = ((y as usize) * (sheet_w as usize) + x as usize) * 4;
                        buf[at..at + 3].copy_from_slice(&TEXT_RGB);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(tile_w: u32, tile_h: u32, rgb: [u8; 3], caption: &str) -> Tile {
        let mut rgba = Vec::with_capacity((tile_w * tile_h * 4) as usize);
        for _ in 0..tile_w * tile_h {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        Tile { rgba, caption: caption.into() }
    }

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * w + x) * 4) as usize;
        [buf[at], buf[at + 1], buf[at + 2]]
    }

    #[test]
    fn grids_are_pixel_near_square() {
        // 4:3 tiles: two stack vertically (closer to square than 2 wide),
        // three take a 2x2 with a hole, six take 2x3.
        assert_eq!(grid_shape(1, 512, 384), (1, 1));
        assert_eq!(grid_shape(2, 512, 384), (1, 2));
        assert_eq!(grid_shape(3, 512, 384), (2, 2));
        assert_eq!(grid_shape(4, 512, 384), (2, 2));
        assert_eq!(grid_shape(6, 512, 384), (2, 3));
        // Wide tiles pack in a single column longer.
        assert_eq!(grid_shape(2, 1024, 256), (1, 2));
        // Tall tiles go side by side.
        assert_eq!(grid_shape(2, 256, 1024), (2, 1));
    }

    #[test]
    fn composite_places_tiles_and_captions() {
        let (tw, th) = (64, 48);
        let tiles = [
            solid(tw, th, [200, 0, 0], "t=0"),
            solid(tw, th, [0, 200, 0], ""),
            solid(tw, th, [0, 0, 200], "t=1"),
        ];
        let (cols, rows) = grid_shape(3, tw, th);
        assert_eq!((cols, rows), (2, 2));
        let png = sheet_png(&tiles, tw, th).unwrap();
        let (w, h, buf) = decode(&png);
        assert_eq!((w, h), sheet_size(tw, th, cols, rows));

        // Tile interiors, row-major.
        assert_eq!(px(&buf, w, 10, 10), [200, 0, 0]);
        assert_eq!(px(&buf, w, tw + GUTTER + 10, 10), [0, 200, 0]);
        assert_eq!(px(&buf, w, 10, th + STRIP_H + GUTTER + 10), [0, 0, 200]);
        // The unfilled fourth cell is ground color.
        assert_eq!(px(&buf, w, tw + GUTTER + 10, th + STRIP_H + GUTTER + 10), FRAME_RGB);
        // The gutter separates tiles.
        assert_eq!(px(&buf, w, tw, 10), FRAME_RGB);
        // Tile 0's caption strip holds text pixels; tile 1's stays blank.
        let strip = |x0: u32, y0: u32| {
            let mut text = 0;
            for y in y0 + th..y0 + th + STRIP_H {
                for x in x0..x0 + tw {
                    if px(&buf, w, x, y) == TEXT_RGB {
                        text += 1;
                    }
                }
            }
            text
        };
        assert!(strip(0, 0) > 0);
        assert_eq!(strip(tw + GUTTER, 0), 0);
    }

    #[test]
    fn long_captions_truncate() {
        let (tw, th) = (64, 16);
        let long: String = "x".repeat(100);
        let tiles = [solid(tw, th, [9, 9, 9], &long)];
        // Must not panic / draw out of bounds; text stays inside the tile.
        let png = sheet_png(&tiles, tw, th).unwrap();
        let (w, _h, buf) = decode(&png);
        for y in th..th + STRIP_H {
            assert_eq!(px(&buf, w, tw - 1, y), FRAME_RGB, "text ran into the right edge");
        }
    }

    #[test]
    fn tile_size_mismatch_is_an_error() {
        let tiles = [Tile { rgba: vec![0; 16], caption: String::new() }];
        assert!(sheet_png(&tiles, 8, 8).is_err());
    }
}
