//! Small text helpers without `core::fmt`: a fixed text buffer of UTF-16-like codes, number
//! formatting, and the character grid shared by the menu and the LCD.

use crate::safe::Safe;

/// Grid size limits: 21 columns x 8 rows covers a 128x64 OLED in the 6x8 font and a 20x4 LCD.
pub const GC: usize = 21;
pub const GR: usize = 8;
/// Capacity of [`Txt`].
pub const TXT: usize = 32;

/// Code of a character in the driver (characters above U+FFFF are shown as `?`).
pub fn code(ch: char) -> u16 {
    let u = ch as u32;
    if u > 0xFFFF {
        b'?' as u16
    } else {
        u as u16
    }
}

/// Nearest integer, halves away from zero (no libm in `no_std`).
pub fn round(x: f64) -> f64 {
    if !(-9.0e15..=9.0e15).contains(&x) {
        x
    } else if x >= 0.0 {
        (x + 0.5) as i64 as f64
    } else {
        -((-x + 0.5) as i64 as f64)
    }
}

/// `v` limited to `lo..=hi` (`lo` for NaN or an empty range). Not `f64::clamp`: its panic message
/// drags float formatting (10 KB) into the module.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if hi < lo || v.is_nan() || v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// A short text of character codes.
#[derive(Clone, Copy)]
pub struct Txt {
    c: [u16; TXT],
    n: usize,
}

impl Default for Txt {
    fn default() -> Self {
        Txt::new()
    }
}

impl Txt {
    pub const fn new() -> Txt {
        Txt { c: [0; TXT], n: 0 }
    }
    pub fn push(&mut self, ch: u16) {
        if let Some(s) = self.c.get_mut(self.n) {
            *s = ch;
            self.n += 1;
        }
    }
    pub fn push_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.push(code(ch));
        }
    }
    pub fn chars(&self) -> &[u16] {
        self.c.get(..self.n).unwrap_or(&[])
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Unsigned integer with at least `min` digits.
    pub fn push_uint(&mut self, v: u64, min: usize) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut v = v;
        while (v > 0 || n < min.max(1)) && n < digits.len() {
            digits.wr(n, (v % 10) as u8);
            v /= 10;
            n += 1;
        }
        while n > 0 {
            n -= 1;
            self.push(b'0' as u16 + digits.rd(n) as u16);
        }
    }
    /// Number with exactly `dec` decimals (at most 6); `---` for NaN and huge values.
    pub fn push_num(&mut self, v: f64, dec: u8) {
        if !(-1.0e12..=1.0e12).contains(&v) {
            self.push_str("---");
            return;
        }
        let dec = dec.min(6) as u32;
        let mut scale = 1u64;
        for _ in 0..dec {
            scale *= 10;
        }
        let neg = v < 0.0;
        let a = if neg { -v } else { v };
        let n = (a * scale as f64 + 0.5) as u64;
        if neg && n != 0 {
            self.push(b'-' as u16);
        }
        self.push_uint(n / scale, 1);
        if dec > 0 {
            self.push(b'.' as u16);
            self.push_uint(n % scale, dec as usize);
        }
    }
    /// Whole numbers without decimals, others with two.
    pub fn push_auto(&mut self, v: f64) {
        let dec = if v == round(v) { 0 } else { 2 };
        self.push_num(v, dec);
    }
}

/// Text of a value widget. `format` 0: number with `decimals`, then `unit`. `format` 1: `unit` is a
/// template and `%v` is replaced by the number (with `decimals`).
pub fn fmt_value(t: &mut Txt, v: f64, format: u8, decimals: u8, unit: &str) {
    if format != 1 {
        t.push_num(v, decimals);
        t.push_str(unit);
        return;
    }
    // `%v` is replaced by the number (no str slicing: its panic path is large).
    let mut it = unit.chars().peekable();
    while let Some(ch) = it.next() {
        if ch == '%' && it.peek() == Some(&'v') {
            it.next();
            t.push_num(v, decimals);
        } else {
            t.push(code(ch));
        }
    }
}

/// A grid of characters (cell 0 means blank). Rows may be marked inverted (menu title).
#[derive(Clone, Copy, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cell: [[u16; GC]; GR],
    /// Bit r = row r is shown inverted.
    pub inv: u8,
}

impl Grid {
    pub const fn new() -> Grid {
        Grid {
            cols: 0,
            rows: 0,
            cell: [[0; GC]; GR],
            inv: 0,
        }
    }
    pub fn reset(&mut self, cols: usize, rows: usize) {
        self.cols = cols.min(GC);
        self.rows = rows.min(GR);
        self.cell = [[0; GC]; GR];
        self.inv = 0;
    }
    /// Character at (row, column), 0 outside.
    pub fn at(&self, row: usize, col: usize) -> u16 {
        self.cell.get(row).map_or(0, |r| r.rd(col))
    }
    pub fn put(&mut self, col: usize, row: usize, s: &[u16]) {
        if row >= self.rows {
            return;
        }
        let Some(line) = self.cell.get_mut(row) else {
            return;
        };
        for (k, &ch) in s.iter().enumerate() {
            if col + k < self.cols {
                line.wr(col + k, ch);
            }
        }
    }
    pub fn put_str(&mut self, col: usize, row: usize, s: &str) {
        for (k, ch) in s.chars().enumerate() {
            self.put(col + k, row, &[code(ch)]);
        }
    }
    /// The row as a `String`-like debug text (tests).
    #[cfg(test)]
    pub fn row_text(&self, row: usize) -> std::string::String {
        self.cell[row][..self.cols]
            .iter()
            .map(|&c| char::from_u32(if c == 0 { 0x20 } else { c as u32 }).unwrap_or('?'))
            .collect()
    }
}

impl Default for Grid {
    fn default() -> Self {
        Grid::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: f64, dec: u8) -> std::string::String {
        let mut t = Txt::new();
        t.push_num(v, dec);
        t.chars().iter().map(|&c| c as u8 as char).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(num(23.456, 1), "23.5");
        assert_eq!(num(-0.04, 1), "0.0");
        assert_eq!(num(-1.5, 0), "-2");
        assert_eq!(num(7.0, 2), "7.00");
        assert_eq!(num(f64::NAN, 1), "---");
        assert_eq!(num(0.05, 2), "0.05");
    }

    #[test]
    fn template() {
        let mut t = Txt::new();
        fmt_value(&mut t, 21.55, 1, 1, "T = %v °C");
        let s: std::string::String = t
            .chars()
            .iter()
            .map(|&c| char::from_u32(c as u32).unwrap())
            .collect();
        assert_eq!(s, "T = 21.6 °C");
        let mut t = Txt::new();
        fmt_value(&mut t, 5.0, 0, 0, "A");
        assert_eq!(t.len(), 2);
    }
}
