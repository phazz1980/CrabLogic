//! Small text helpers without `core::fmt`: a fixed text buffer of UTF-16-like codes, number
//! formatting, and the character grid shared by the menu and the LCD.
//!
//! Numbers are integers in units of `10^-decimals` (fixed point); `f32` appears only where a
//! `float` cell is turned into such an integer ([`scaled_f32`]) and back ([`unscaled_bits`]).

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

/// Marker of a value that cannot be shown (NaN, infinity or beyond `i32` after scaling): `---`.
pub const INVALID: i32 = i32::MIN;

/// `10^dec` (`dec` at most 6, as the format allows).
pub fn pow10(dec: u8) -> u32 {
    let mut p = 1u32;
    for _ in 0..dec.min(6) {
        p *= 10;
    }
    p
}

/// An `f32` as sign, mantissa and binary exponent: value = `m * 2^e`. `None` for NaN and infinity.
fn split_f32(bits: u32) -> Option<(bool, u32, i32)> {
    let exp = ((bits >> 23) & 0xFF) as i32;
    let frac = bits & 0x007F_FFFF;
    if exp == 0xFF {
        None
    } else if exp == 0 {
        Some((bits >> 31 != 0, frac, -149))
    } else {
        Some((bits >> 31 != 0, frac | 0x0080_0000, exp - 150))
    }
}

/// The `f32` with bits `bits` times `10^dec`, rounded to the nearest integer (halves away from
/// zero), exactly: the whole calculation is integer, there is no rounding of the product.
/// [`INVALID`] for NaN, infinity and results that do not fit `i32`.
pub fn scaled_f32(bits: u32, dec: u8) -> i32 {
    let Some((neg, m, e)) = split_f32(bits) else {
        return INVALID;
    };
    // m < 2^24 and 10^6 < 2^20: the product fits 44 bits.
    let p = u64::from(m) * u64::from(pow10(dec));
    let n: u64 = if e >= 0 {
        if p == 0 {
            0
        } else if e > 19 {
            return INVALID;
        } else {
            p << e
        }
    } else if -e >= 64 {
        0
    } else {
        (p + (1u64 << (-e - 1))) >> -e
    };
    if n > i32::MAX as u64 {
        return INVALID;
    }
    if neg {
        -(n as i32)
    } else {
        n as i32
    }
}

/// An `i32` times `10^dec`; [`INVALID`] on overflow and for [`INVALID`] itself.
pub fn scaled_i32(v: i32, dec: u8) -> i32 {
    if v == INVALID {
        return INVALID;
    }
    match v.checked_mul(pow10(dec) as i32) {
        Some(n) if n != INVALID => n,
        _ => INVALID,
    }
}

/// The value of a variable cell (`bits`: `f32` bits for a `float` variable, an `i32` otherwise)
/// in units of `10^-dec`.
pub fn scaled_cell(bits: u32, float: bool, dec: u8) -> i32 {
    if float {
        scaled_f32(bits, dec)
    } else {
        scaled_i32(bits as i32, dec)
    }
}

/// The value of a cell rounded to a whole number (choice index), 0 for NaN.
pub fn whole_cell(bits: u32, float: bool) -> i32 {
    match scaled_cell(bits, float, 0) {
        INVALID => 0,
        n => n,
    }
}

/// The cell has no fraction (an integer value; NaN and infinity do not count as whole).
pub fn is_whole_cell(bits: u32, float: bool) -> bool {
    if !float {
        return true;
    }
    match split_f32(bits) {
        None => false,
        Some((_, m, e)) => m == 0 || e >= 0 || (-e < 32 && m & ((1u32 << -e) - 1) == 0),
    }
}

/// `v` limited to `lo..=hi` (`lo` for an empty range).
pub fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo || v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `n / 10^dec` as `f32` bits (the value of a `float` variable that was edited in units of
/// `10^-dec`); the division of two exact integers is rounded once, to the nearest `f32`.
pub fn unscaled_bits(n: i32, dec: u8) -> u32 {
    (n as f32 / pow10(dec) as f32).to_bits()
}

/// Filled part of a bar `cells` wide for the value `v` between `min` and `max` (an `f32` calculation
/// because the bar input is a `float`); 0 for NaN and an empty range, halves round up.
pub fn bar_fill(v: f32, min: f32, max: f32, cells: i32) -> i32 {
    let span = max - min;
    if span.is_nan() || span <= 0.0 {
        return 0;
    }
    let t = (v - min) / span;
    let t = if t.is_nan() || t < 0.0 {
        0.0
    } else if t > 1.0 {
        1.0
    } else {
        t
    };
    (t * cells as f32 + 0.5) as i32
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
    pub fn push_uint(&mut self, v: u32, min: usize) {
        let mut digits = [0u8; 10];
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
    /// A number given in units of `10^-dec` (at most 6 decimals); `---` for [`INVALID`].
    pub fn push_scaled(&mut self, v: i32, dec: u8) {
        if v == INVALID {
            self.push_str("---");
            return;
        }
        let dec = dec.min(6);
        let scale = pow10(dec);
        let a = v.unsigned_abs();
        if v < 0 {
            self.push(b'-' as u16);
        }
        self.push_uint(a / scale, 1);
        if dec > 0 {
            self.push(b'.' as u16);
            self.push_uint(a % scale, dec as usize);
        }
    }
    /// Whole numbers without decimals, others with two (a view of a variable cell).
    pub fn push_auto(&mut self, bits: u32, float: bool) {
        let dec = if is_whole_cell(bits, float) { 0 } else { 2 };
        self.push_scaled(scaled_cell(bits, float, dec), dec);
    }
}

/// Text of a value widget; `v` is in units of `10^-decimals` ([`scaled_f32`]). `format` 0: number
/// with `decimals`, then `unit`. `format` 1: `unit` is a template and `%v` is replaced by the
/// number (with `decimals`).
pub fn fmt_value(t: &mut Txt, v: i32, format: u8, decimals: u8, unit: &str) {
    if format != 1 {
        t.push_scaled(v, decimals);
        t.push_str(unit);
        return;
    }
    // `%v` is replaced by the number (no str slicing: its panic path is large).
    let mut it = unit.chars().peekable();
    while let Some(ch) = it.next() {
        if ch == '%' && it.peek() == Some(&'v') {
            it.next();
            t.push_scaled(v, decimals);
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

    fn num(v: f32, dec: u8) -> std::string::String {
        let mut t = Txt::new();
        t.push_scaled(scaled_f32(v.to_bits(), dec), dec);
        t.chars().iter().map(|&c| c as u8 as char).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(num(23.456, 1), "23.5");
        assert_eq!(num(-0.04, 1), "0.0");
        assert_eq!(num(-1.5, 0), "-2");
        assert_eq!(num(7.0, 2), "7.00");
        assert_eq!(num(f32::NAN, 1), "---");
        assert_eq!(num(f32::INFINITY, 1), "---");
        assert_eq!(num(3.0e9, 0), "---");
        assert_eq!(num(0.05, 2), "0.05");
        assert_eq!(num(0.0, 3), "0.000");
        assert_eq!(num(-12345.678, 3), "-12345.678");
        assert_eq!(num(1.0e-30, 6), "0.000000");
    }

    #[test]
    fn scaling_is_exact() {
        // 21.55 as f32 is 21.549999..., so one decimal is 21.5 (an f32 product 215.5 would lie).
        assert_eq!(scaled_f32(21.55f32.to_bits(), 1), 215);
        assert_eq!(scaled_f32(2.5f32.to_bits(), 0), 3);
        assert_eq!(scaled_f32((-2.5f32).to_bits(), 0), -3);
        assert_eq!(scaled_f32(2.0e9f32.to_bits(), 0), 2_000_000_000);
        assert_eq!(scaled_i32(5, 2), 500);
        assert_eq!(scaled_i32(i32::MAX, 1), INVALID);
        assert_eq!(unscaled_bits(215, 1), 21.5f32.to_bits());
    }

    #[test]
    fn whole_cells() {
        assert!(is_whole_cell(5.0f32.to_bits(), true));
        assert!(is_whole_cell(0.0f32.to_bits(), true));
        assert!(is_whole_cell(1.0e20f32.to_bits(), true));
        assert!(!is_whole_cell(5.004f32.to_bits(), true));
        assert!(!is_whole_cell(0.5f32.to_bits(), true));
        assert!(!is_whole_cell(f32::NAN.to_bits(), true));
        assert!(is_whole_cell(7, false));
    }

    #[test]
    fn template() {
        let mut t = Txt::new();
        fmt_value(&mut t, scaled_f32(21.6f32.to_bits(), 1), 1, 1, "T = %v °C");
        let s: std::string::String = t
            .chars()
            .iter()
            .map(|&c| char::from_u32(c as u32).unwrap())
            .collect();
        assert_eq!(s, "T = 21.6 °C");
        let mut t = Txt::new();
        fmt_value(&mut t, 5, 0, 0, "A");
        assert_eq!(t.len(), 2);
    }
}
