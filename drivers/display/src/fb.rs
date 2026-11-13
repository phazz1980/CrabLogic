//! OLED framebuffer: 128 columns, pages of 8 rows (SSD1306 layout: byte = column, bit = row in page).

use crate::font::glyph;
use crate::safe::Safe;
use crate::text::code;

pub const WIDTH: usize = 128;
pub const PAGE_BYTES: usize = WIDTH;
pub const MAX_PAGES: usize = 8;

pub struct Fb {
    pub buf: [u8; WIDTH * MAX_PAGES],
    pub pages: usize,
}

impl Fb {
    pub const fn new() -> Fb {
        Fb {
            buf: [0; WIDTH * MAX_PAGES],
            pages: 0,
        }
    }

    pub fn clear(&mut self) {
        self.buf = [0; WIDTH * MAX_PAGES];
    }

    pub fn page(&self, p: usize) -> &[u8] {
        self.buf.span(p * PAGE_BYTES, (p + 1) * PAGE_BYTES)
    }

    pub fn pixel(&mut self, x: i32, y: i32, on: bool) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= (self.pages * 8) as i32 {
            return;
        }
        let i = (y as usize / 8) * WIDTH + x as usize;
        let bit = 1u8 << (y % 8);
        if let Some(b) = self.buf.get_mut(i) {
            if on {
                *b |= bit;
            } else {
                *b &= !bit;
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, on: bool) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.pixel(xx, yy, on);
            }
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w < 1 || h < 1 {
            return;
        }
        for xx in x..x + w {
            self.pixel(xx, y, true);
            self.pixel(xx, y + h - 1, true);
        }
        for yy in y..y + h {
            self.pixel(x, yy, true);
            self.pixel(x + w - 1, yy, true);
        }
    }

    /// Draws the set pixels of a glyph with `on`, magnified by `scale`.
    pub fn glyph(&mut self, x: i32, y: i32, rows: &[u8; 8], scale: i32, on: bool) {
        for (r, &bits) in rows.iter().enumerate() {
            for c in 0..5 {
                if bits & (0x10 >> c) != 0 {
                    self.fill_rect(x + c * scale, y + r as i32 * scale, scale, scale, on);
                }
            }
        }
    }

    /// Text from character codes; the advance is 6 pixels per scale step. Returns the end x.
    pub fn chars(&mut self, x: i32, y: i32, s: &[u16], scale: i32) -> i32 {
        let mut x = x;
        for &ch in s {
            self.glyph(x, y, &glyph(ch), scale, true);
            x += 6 * scale;
        }
        x
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, scale: i32) -> i32 {
        let mut x = x;
        for ch in s.chars() {
            self.glyph(x, y, &glyph(code(ch)), scale, true);
            x += 6 * scale;
        }
        x
    }

    /// The region as ASCII art, `#` for a lit pixel (tests).
    #[cfg(test)]
    pub fn ascii(
        &self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
    ) -> std::vec::Vec<std::string::String> {
        (y0..y0 + h)
            .map(|y| {
                (x0..x0 + w)
                    .map(|x| {
                        if self.buf[(y / 8) * WIDTH + x] & (1 << (y % 8)) != 0 {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

impl Default for Fb {
    fn default() -> Self {
        Fb::new()
    }
}
