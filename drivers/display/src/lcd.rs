//! Character LCD side: widgets into a grid, mapping characters to the HD44780 ROM (with up to
//! 8 custom CGRAM glyphs for Cyrillic and icons) and the PCF8574 wire protocol.

use crate::font::{glyph, known, latin_twin, ICON_BASE};
use crate::safe::Safe;
use crate::text::{clamp, fmt_value, round, Grid, Txt};
use crab_ui_format::{DisplayKind, WidgetKindView, WidgetView};

/// Widest supported display (2004).
pub const LC: usize = 20;
pub const LR: usize = 4;
pub const SLOTS: usize = 8;

// PCF8574 pins: P0 RS, P1 RW, P2 EN, P3 backlight, P4..P7 data D4..D7.
const RS: u8 = 0x01;
const EN: u8 = 0x04;
const BL: u8 = 0x08;

/// Largest transfer: DDRAM address + 20 characters, 4 bytes each.
pub const MAX_TRANSFER: usize = 4 + LC * 4;

/// ROM code of the degree sign (ROM A00).
const ROM_DEGREE: u8 = 0xDF;
/// ROM code of the full block, used by bars and LEDs.
pub const ROM_BLOCK: u8 = 0xFF;

pub fn size(kind: DisplayKind) -> (usize, usize) {
    if kind == DisplayKind::Lcd2004 {
        (20, 4)
    } else {
        (16, 2)
    }
}

/// Writes a widget into the grid; coordinates and sizes are in character cells.
pub fn draw_widget(g: &mut Grid, w: &WidgetView, v: f64) {
    let (x, y) = (w.x as usize, w.y as usize);
    match w.kind {
        WidgetKindView::Text { text } => g.put_str(x, y, text),
        WidgetKindView::Value {
            format,
            decimals,
            unit,
        } => {
            let mut t = Txt::new();
            fmt_value(&mut t, v, format, decimals, unit);
            g.put(x, y, t.chars());
        }
        WidgetKindView::Bar { min, max } => {
            let cells = if w.w > 0 { w.w as usize } else { 8 };
            let span = (max - min) as f64;
            let frac = if span > 0.0 {
                (v - min as f64) / span
            } else {
                0.0
            };
            let frac = if frac.is_nan() {
                0.0
            } else {
                clamp(frac, 0.0, 1.0)
            };
            let fill = round(frac * cells as f64) as usize;
            for k in 0..cells {
                g.put(
                    x + k,
                    y,
                    &[if k < fill {
                        ROM_BLOCK as u16
                    } else {
                        b'-' as u16
                    }],
                );
            }
        }
        WidgetKindView::Led => g.put(
            x,
            y,
            &[if v > 0.5 {
                ROM_BLOCK as u16
            } else {
                b'o' as u16
            }],
        ),
        WidgetKindView::Icon { id } => g.put(x, y, &[ICON_BASE + id as u16]),
    }
}

/// What the next transfer does (so the state can be committed after it succeeded).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Slot(usize),
    Row(usize),
    Backlight,
}

/// Cell contents and what the display currently shows.
pub struct Lcd {
    pub cols: usize,
    pub rows: usize,
    /// Character code (Unicode-like) held by each CGRAM slot, 0 = free.
    pub cg: [u16; SLOTS],
    /// Bit s = slot s holds `cg[s]` on the display.
    cg_ok: u8,
    /// ROM or CGRAM codes to show.
    pub cells: [[u8; LC]; LR],
    sent: [[u8; LC]; LR],
    /// Bit r = row r on the display equals `sent[r]` (and `sent` is what we want, unless dirty).
    row_ok: u8,
    pub backlight: bool,
    bl_ok: bool,
}

impl Lcd {
    pub const fn new() -> Lcd {
        Lcd {
            cols: 0,
            rows: 0,
            cg: [0; SLOTS],
            cg_ok: 0,
            cells: [[0; LC]; LR],
            sent: [[0; LC]; LR],
            row_ok: 0,
            backlight: false,
            bl_ok: false,
        }
    }

    /// Forget what the display shows (after a (re)initialisation).
    pub fn invalidate(&mut self) {
        self.cg_ok = 0;
        self.row_ok = 0;
        self.bl_ok = false;
    }

    /// Native ROM code of `c`, or `None` when it needs a custom glyph.
    fn native(c: u16) -> Option<u8> {
        match c {
            0..=0x1F => Some(b' '),
            0x5C => Some(b'/'), // ROM A00 has a yen sign here
            0x7E => Some(b'-'), // and an arrow here
            0x20..=0x7E => Some(c as u8),
            0xB0 => Some(ROM_DEGREE),
            ROM_BLOCK_CODE => Some(ROM_BLOCK),
            _ => latin_twin(c),
        }
    }

    /// Maps the grid to cell codes, giving custom characters stable CGRAM slots (at most 8
    /// distinct ones; the rest show `?`).
    pub fn map_grid(&mut self, g: &Grid) {
        let mut used = [false; SLOTS];
        for r in 0..self.rows {
            for c in 0..self.cols {
                let ch = g.at(r, c);
                if Self::native(ch).is_none() {
                    if let Some(s) = self.cg.iter().position(|&x| x == ch) {
                        used.wr(s, true);
                    }
                }
            }
        }
        for r in 0..self.rows {
            for c in 0..self.cols {
                let ch = g.at(r, c);
                let code = match Self::native(ch) {
                    Some(code) => code,
                    None if !known(ch) => b'?',
                    None => match self.cg.iter().position(|&x| x == ch) {
                        Some(s) => s as u8,
                        None => match used.iter().position(|&u| !u) {
                            Some(s) => {
                                self.cg.wr(s, ch);
                                used.wr(s, true);
                                self.cg_ok &= !(1 << s);
                                s as u8
                            }
                            None => b'?',
                        },
                    },
                };
                if let Some(row) = self.cells.get_mut(r) {
                    row.wr(c, code);
                }
            }
        }
    }

    /// Number of distinct custom glyphs the grid needs (for the 8-glyph limit).
    pub fn custom_needed(g: &Grid) -> usize {
        let mut seen = [0u16; 64];
        let mut n = 0;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let ch = g.at(r, c);
                if Self::native(ch).is_none()
                    && known(ch)
                    && !seen.part(n).contains(&ch)
                    && n < seen.len()
                {
                    seen.wr(n, ch);
                    n += 1;
                }
            }
        }
        n
    }

    /// Chooses the next piece of work: an unsent CGRAM glyph, then a changed row, then the
    /// backlight. Builds its bytes into `buf` and returns the action and the length.
    pub fn next(&self, buf: &mut [u8; MAX_TRANSFER]) -> Option<(Act, usize)> {
        let bl = if self.backlight { BL } else { 0 };
        for s in 0..SLOTS {
            if self.cg.rd(s) != 0 && self.cg_ok & (1 << s) == 0 {
                let mut n = 0;
                put_byte(buf, &mut n, 0x40 | ((s as u8) << 3), 0, bl);
                for r in glyph(self.cg.rd(s)) {
                    put_byte(buf, &mut n, r, RS, bl);
                }
                return Some((Act::Slot(s), n));
            }
        }
        for r in 0..self.rows {
            if self.row_ok & (1 << r) == 0
                || self.cells.rd(r).part(self.cols) != self.sent.rd(r).part(self.cols)
            {
                let base = [0x00u8, 0x40, 0x14, 0x54].rd(r);
                let mut n = 0;
                put_byte(buf, &mut n, 0x80 | base, 0, bl);
                for c in 0..self.cols {
                    put_byte(buf, &mut n, self.cells.rd(r).rd(c), RS, bl);
                }
                return Some((Act::Row(r), n));
            }
        }
        if !self.bl_ok {
            buf.wr(0, bl);
            return Some((Act::Backlight, 1));
        }
        None
    }

    /// Records that the transfer of `act` reached the display.
    pub fn commit(&mut self, act: Act) {
        match act {
            Act::Slot(s) => {
                self.cg_ok |= 1 << s;
                // Cells already showing this code must be redrawn with the new glyph.
                for r in 0..self.rows {
                    if self.sent.rd(r).part(self.cols).contains(&(s as u8)) {
                        self.row_ok &= !(1 << r);
                    }
                }
            }
            Act::Row(r) => {
                self.sent.wr(r, self.cells.rd(r));
                self.row_ok |= 1 << r;
            }
            Act::Backlight => {}
        }
        // Every transfer carries the current backlight bit.
        self.bl_ok = true;
    }

    /// Sets the backlight; the next transfer carries the new state.
    pub fn set_backlight(&mut self, on: bool) {
        if on != self.backlight {
            self.backlight = on;
            self.bl_ok = false;
        }
    }
}

impl Default for Lcd {
    fn default() -> Self {
        Lcd::new()
    }
}

/// Code of the full block as a grid character (so `ROM_BLOCK` survives the grid).
const ROM_BLOCK_CODE: u16 = ROM_BLOCK as u16;

/// One byte to the HD44780 in 4-bit mode as four PCF8574 bytes (high nibble, low nibble, each
/// with an EN pulse).
pub fn put_byte(buf: &mut [u8], n: &mut usize, byte: u8, rs: u8, bl: u8) {
    for nib in [byte >> 4, byte & 0x0F] {
        let b = (nib << 4) | bl | rs;
        buf.wr(*n, b | EN);
        buf.wr(*n + 1, b);
        *n += 2;
    }
}

/// A single nibble (initialisation, 8-bit-mode part).
pub fn put_nibble(buf: &mut [u8; 2], nib: u8, bl: bool) {
    let b = (nib << 4) | if bl { BL } else { 0 };
    buf.wr(0, b | EN);
    buf.wr(1, b);
}
