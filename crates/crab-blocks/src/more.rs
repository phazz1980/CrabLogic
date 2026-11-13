//! Numbers, bits and words: stateless blocks and the few with a little state.
//!
//! Integer ports are 32-bit as in the firmware; floats are computed in 32 bits, as `sinf` and
//! friends do on the board.

use crate::{b, Behavior, Ctx, Plain};

/// Deterministic generator shared by the simulation and the PC tests: the same
/// sequence comes out of both (the module prelude of `random` uses the same constants).
pub const RANDOM_SEED: u32 = 2_463_534_242;

pub fn next_random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn int(v: f64) -> i32 {
    v as i32
}

/// The angle in radians; parameter 0 is the unit (0 — degrees).
fn radians(c: &Ctx) -> f32 {
    let x = c.input(0) as f32;
    if c.param(0) == 0.0 {
        x * 0.017_453_292
    } else {
        x
    }
}

pub fn behavior(kind: &str) -> Option<Box<dyn Behavior>> {
    let plain = |f: fn(&Ctx, &mut [f64])| -> Option<Box<dyn Behavior>> { Some(Box::new(Plain(f))) };
    match kind {
        "abs" => plain(|c, o| o[0] = c.input(0).abs()),
        "sq" => plain(|c, o| o[0] = c.input(0) * c.input(0)),
        "sqrt" => plain(|c, o| {
            let x = c.input(0) as f32;
            o[0] = if x > 0.0 { f64::from(x.sqrt()) } else { 0.0 };
        }),
        "pow" => plain(|c, o| o[0] = f64::from((c.input(0) as f32).powf(c.input(1) as f32))),
        "sin" => plain(|c, o| o[0] = f64::from(radians(c).sin())),
        "cos" => plain(|c, o| o[0] = f64::from(radians(c).cos())),
        "tan" => plain(|c, o| o[0] = f64::from(radians(c).tan())),
        "float_to_int" => plain(|c, o| {
            let x = c.input(0);
            o[0] = f64::from(match c.param(0) as i64 {
                0 => x.round() as i32,
                1 => x.floor() as i32,
                2 => x.ceil() as i32,
                _ => x as i32,
            });
        }),
        "int_to_float" => plain(|c, o| o[0] = c.input(0)),
        "random" => Some(Box::new(Random {
            prev: false,
            state: RANDOM_SEED,
        })),
        "bit_and" => plain(|c, o| o[0] = f64::from(int(c.input(0)) & int(c.input(1)))),
        "bit_or" => plain(|c, o| o[0] = f64::from(int(c.input(0)) | int(c.input(1)))),
        "bit_xor" => plain(|c, o| o[0] = f64::from(int(c.input(0)) ^ int(c.input(1)))),
        "bit_not" => plain(|c, o| o[0] = f64::from(!int(c.input(0)))),
        "bit_read" => plain(|c, o| o[0] = b(bit(c))),
        "bit_write" => plain(|c, o| {
            let (x, mask) = (int(c.input(0)), 1_i32.wrapping_shl(c.param(0) as u32));
            o[0] = f64::from(if c.flag(1) { x | mask } else { x & !mask });
        }),
        "bit_change" => Some(Box::<BitChange>::default()),
        "bits_to_byte" => plain(|c, o| {
            o[0] = f64::from(
                (0..8)
                    .map(|i| if c.flag(i) { 1 << i } else { 0 })
                    .sum::<i32>(),
            );
        }),
        "byte_to_bits" => plain(|c, o| {
            for (i, v) in o.iter_mut().enumerate().take(8) {
                *v = b((int(c.input(0)) >> i) & 1 != 0);
            }
        }),
        "bytes_to_word" => plain(|c, o| {
            o[0] = f64::from(((int(c.input(1)) & 0xFF) << 8) | (int(c.input(0)) & 0xFF));
        }),
        "word_to_bytes" => plain(|c, o| {
            o[0] = f64::from(int(c.input(0)) & 0xFF);
            o[1] = f64::from((int(c.input(0)) >> 8) & 0xFF);
        }),
        "bytes_to_dword" => plain(|c, o| {
            let v = (0..4).fold(0_u32, |acc, i| {
                acc | (((int(c.input(i)) & 0xFF) as u32) << (8 * i))
            });
            o[0] = f64::from(v as i32);
        }),
        "dword_to_bytes" => plain(|c, o| {
            for (i, v) in o.iter_mut().enumerate().take(4) {
                *v = f64::from(((int(c.input(0)) as u32) >> (8 * i)) & 0xFF);
            }
        }),
        "number_change" => Some(Box::<NumberChange>::default()),
        "string_change" => Some(Box::<StringChange>::default()),
        "pid" => Some(Box::<Pid>::default()),
        "scale_table" => Some(Box::<ScaleTable>::default()),
        _ => None,
    }
}

/// Bit number `param 0` of input 0.
fn bit(c: &Ctx) -> bool {
    int(c.input(0)).wrapping_shr(c.param(0) as u32) & 1 != 0
}

struct Random {
    prev: bool,
    state: u32,
}

impl Behavior for Random {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if ctx.flag(0) && !self.prev {
            let r = (next_random(&mut self.state) % 10_001) as f32 / 10_000.0;
            let (lo, hi) = (ctx.param(0) as f32, ctx.param(1) as f32);
            out[0] = f64::from(lo + (hi - lo) * r);
        }
        self.prev = ctx.flag(0);
    }
}

#[derive(Default)]
struct BitChange {
    prev: bool,
    init: bool,
}

impl Behavior for BitChange {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let now = bit(ctx);
        out[0] = b(self.init && now != self.prev);
        self.prev = now;
        self.init = true;
    }
}

#[derive(Default)]
struct NumberChange {
    prev: f64,
    init: bool,
}

impl Behavior for NumberChange {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let x = ctx.input(0);
        if self.init {
            out[0] = b((x - self.prev).abs() > ctx.param(0));
            if out[0] != 0.0 {
                self.prev = x;
            }
        } else {
            self.init = true;
            self.prev = x;
            out[0] = 0.0;
        }
    }
}

#[derive(Default)]
struct StringChange {
    prev: String,
    init: bool,
}

impl Behavior for StringChange {
    fn step(&mut self, _: &Ctx, _: &mut [f64]) {}

    fn step_text(&mut self, ctx: &Ctx, out: &mut [f64], _: &mut [String]) {
        out[0] = b(self.init && ctx.text(0) != self.prev);
        self.prev = ctx.text(0).to_owned();
        self.init = true;
    }
}

/// `constrain` (clamp) on 32 bit floats.
fn constrain32(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

/// The sketch's PID in 32 bit floats, in the same order of operations (the module does the same).
#[derive(Default)]
struct Pid {
    integral: f32,
    prev_err: f32,
    last_ms: u32,
    running: bool,
}

impl Behavior for Pid {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        let (kp, ki, kd) = (
            ctx.param(0) as f32,
            ctx.param(1) as f32,
            ctx.param(2) as f32,
        );
        let (lo, hi) = (ctx.param(3) as f32, ctx.param(4) as f32);
        if !ctx.flag(2) {
            self.running = false;
            self.integral = 0.0;
            out[0] = f64::from(lo);
            return;
        }
        let err = ctx.input(0) as f32 - ctx.input(1) as f32;
        let now = ctx.now_ms as u32;
        if self.running {
            let dt = now.wrapping_sub(self.last_ms) as f32 / 1000.0;
            if dt > 0.0 {
                self.integral = constrain32(self.integral + err * dt * ki, lo, hi);
                let d = (err - self.prev_err) / dt;
                out[0] = f64::from(constrain32(kp * err + self.integral + kd * d, lo, hi));
                self.prev_err = err;
                self.last_ms = now;
            }
        } else {
            self.running = true;
            self.last_ms = now;
            self.prev_err = err;
            out[0] = f64::from(constrain32(kp * err, lo, hi));
        }
    }
}

/// `strtof` of the C library, without `inf`, `nan` and hex: the value (0 when there is no
/// number) and how many bytes were used.
fn parse_float(s: &str) -> (f32, usize) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let start = i;
    if matches!(bytes.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    let int_end = digits(i);
    let mut end = int_end;
    let mut has_digits = int_end > i;
    if bytes.get(end) == Some(&b'.') {
        let frac_end = digits(end + 1);
        has_digits |= frac_end > end + 1;
        if has_digits {
            end = frac_end;
        }
    }
    if !has_digits {
        return (0.0, 0);
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exp = end + 1;
        if matches!(bytes.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        let exp_end = digits(exp);
        if exp_end > exp {
            end = exp_end;
        }
    }
    (s[start..end].parse().unwrap_or(0.0), end)
}

/// Points of `"x:y;x:y"`, parsed the way the firmware does it (up to 32 of them).
pub(crate) fn parse_points(text: &str) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    // The default of the block is a C string literal (`"0:0;50:20"`): its quotes are not points.
    let mut rest = text.trim().trim_matches('"');
    while !rest.is_empty() && points.len() < 32 {
        let (x, used) = parse_float(rest);
        if rest.as_bytes().get(used) != Some(&b':') {
            break;
        }
        rest = &rest[used + 1..];
        let (y, used) = parse_float(rest);
        points.push((x, y));
        rest = match rest.as_bytes().get(used) {
            Some(b';') => &rest[used + 1..],
            _ => &rest[used..],
        };
    }
    if points.is_empty() {
        points.push((0.0, 0.0));
    }
    points
}

#[derive(Default)]
struct ScaleTable {
    source: String,
    points: Vec<(f32, f32)>,
}

impl Behavior for ScaleTable {
    fn step(&mut self, ctx: &Ctx, out: &mut [f64]) {
        if self.points.is_empty() || self.source != ctx.param_text(0) {
            self.source = ctx.param_text(0).to_owned();
            self.points = parse_points(&self.source);
        }
        let p = &self.points;
        let v = ctx.input(0) as f32;
        let y = if v <= p[0].0 {
            p[0].1
        } else if v >= p[p.len() - 1].0 {
            p[p.len() - 1].1
        } else {
            let i = p
                .iter()
                .position(|q| v <= q.0)
                .unwrap_or(p.len() - 1)
                .max(1);
            p[i - 1].1 + (p[i].1 - p[i - 1].1) * (v - p[i - 1].0) / (p[i].0 - p[i - 1].0)
        };
        out[0] = f64::from(y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(b: &mut dyn Behavior, inputs: &[f64], texts: &[&str], ptexts: &[&str]) -> f64 {
        let texts: Vec<String> = texts.iter().map(|s| (*s).to_owned()).collect();
        let ptexts: Vec<String> = ptexts.iter().map(|s| (*s).to_owned()).collect();
        let active = vec![true; inputs.len().max(texts.len())];
        let ctx = Ctx {
            now_ms: 0,
            inputs,
            active: &active,
            params: &[],
            texts: &texts,
            param_texts: &ptexts,
            bench: &crate::NO_BENCH,
        };
        let mut out = [0.0];
        b.step(&ctx, &mut out);
        b.step_text(&ctx, &mut out, &mut []);
        out[0]
    }

    #[test]
    fn string_change_ignores_the_first_scan() {
        let mut b = behavior("string_change").unwrap();
        let mut s = |t: &str| scan(&mut *b, &[], &[t], &[]);
        assert_eq!(s("a"), 0.0, "first scan only remembers");
        assert_eq!(s("a"), 0.0);
        assert_eq!(s("b"), 1.0);
        assert_eq!(s("b"), 0.0);
        assert_eq!(s(""), 1.0);
    }

    #[test]
    fn scale_table_interpolates_and_clamps() {
        let mut b = behavior("scale_table").unwrap();
        let mut y = |x: f64| scan(&mut *b, &[x], &[], &["0:0;50:20;100:100"]);
        assert_eq!(y(-10.0), 0.0);
        assert_eq!(y(0.0), 0.0);
        assert_eq!(y(25.0), 10.0);
        assert_eq!(y(50.0), 20.0);
        assert_eq!(y(75.0), 60.0);
        assert_eq!(y(100.0), 100.0);
        assert_eq!(y(250.0), 100.0);
    }

    #[test]
    fn scale_table_reparses_when_the_points_change() {
        let mut b = behavior("scale_table").unwrap();
        assert_eq!(scan(&mut *b, &[5.0], &[], &["0:0;10:100"]), 50.0);
        assert_eq!(scan(&mut *b, &[5.0], &[], &["0:0;10:10"]), 5.0);
        // Broken text falls back to a single point at zero.
        assert_eq!(scan(&mut *b, &[5.0], &[], &["junk"]), 0.0);
    }
}
