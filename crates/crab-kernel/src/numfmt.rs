//! Number to text without heap allocations or floating-point math, for the string host
//! (`text_op`). The output is byte-for-byte what `format!("{:.p$}", f64::from(x))` and
//! `i32::to_string` give; the tests check that.

/// Longest text of [`fixed`]: sign, 39 integer digits, point and 9 decimals.
pub const FIXED_MAX: usize = 50;

/// Writes `x` with `prec` decimals (0..=9, larger is clamped) into `buf`, returns the length.
/// Rounds the exact binary value half to even, like the standard formatter.
pub fn fixed(x: f32, prec: usize, buf: &mut [u8; FIXED_MAX]) -> usize {
    let prec = prec.min(9);
    let bits = x.to_bits();
    let neg = bits >> 31 != 0;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let frac = bits & 0x7F_FFFF;
    if exp == 0xFF {
        let s: &[u8] = match (frac != 0, neg) {
            (true, _) => b"NaN",
            (false, true) => b"-inf",
            (false, false) => b"inf",
        };
        buf[..s.len()].copy_from_slice(s);
        return s.len();
    }
    // x = m * 2^e
    let (m, e) = if exp == 0 {
        (frac, -149)
    } else {
        (frac | 0x80_0000, exp - 150)
    };
    let pow = 10u64.pow(prec as u32);
    let mut n = 0;
    if neg {
        buf[0] = b'-';
        n = 1;
    }
    // The scaled value round(x * 10^prec) as integer part and decimals. 32-bit RISC-V has no
    // 64/128-bit division, so the common path stays in u64 with as few divisions as possible.
    let (int, dec) = if e >= 0 {
        if e > 39 {
            // At least 2^63: rare, the slow 128-bit path (f32::MAX is (2^24 - 1) * 2^104).
            n += digits128((m as u128) << e, &mut buf[n..]);
            return n + zeros(prec, &mut buf[n..]);
        }
        ((m as u64) << e, 0)
    } else {
        let k = -e as u32;
        // m * 10^9 < 2^54, so for k >= 64 it is below half of 2^k and rounds to 0.
        let q = if k >= 64 {
            0
        } else {
            let num = m as u64 * pow;
            let mut q = num >> k;
            let r = num & ((1u64 << k) - 1);
            let half = 1u64 << (k - 1);
            if r > half || (r == half && q & 1 == 1) {
                q += 1;
            }
            q
        };
        if q < 1 << 32 {
            let (q, pow) = (q as u32, pow as u32);
            (u64::from(q / pow), q % pow)
        } else {
            (q / pow, (q % pow) as u32)
        }
    };
    n += digits(int, &mut buf[n..]);
    if prec > 0 {
        buf[n] = b'.';
        n += 1;
        let mut d = dec;
        for i in (0..prec).rev() {
            buf[n + i] = b'0' + (d % 10) as u8;
            d /= 10;
        }
        n += prec;
    }
    n
}

/// Writes `v` in decimal into `buf` (at least 11 bytes), returns the length.
pub fn int(v: i32, buf: &mut [u8]) -> usize {
    if v < 0 {
        buf[0] = b'-';
        1 + digits32(v.unsigned_abs(), &mut buf[1..])
    } else {
        digits32(v as u32, buf)
    }
}

/// `.` and `prec` zeros (nothing for 0).
fn zeros(prec: usize, out: &mut [u8]) -> usize {
    if prec == 0 {
        return 0;
    }
    out[0] = b'.';
    out[1..=prec].fill(b'0');
    prec + 1
}

/// Decimal digits of `v` (no leading zeros, "0" for zero) into the start of `out`.
fn digits32(mut v: u32, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 10];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let len = tmp.len() - i;
    out[..len].copy_from_slice(&tmp[i..]);
    len
}

/// Like [`digits32`] for u64: one 64-bit division per 9 digits.
fn digits(v: u64, out: &mut [u8]) -> usize {
    if v < 1 << 32 {
        return digits32(v as u32, out);
    }
    let (hi, lo) = (v / 1_000_000_000, (v % 1_000_000_000) as u32);
    let n = digits(hi, out);
    pad9(lo, &mut out[n..]);
    n + 9
}

/// `v` (below 10^9) as exactly nine digits.
fn pad9(mut v: u32, out: &mut [u8]) {
    for i in (0..9).rev() {
        out[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
}

/// Like [`digits32`] for u128 (slow on 32-bit targets; only for values of 2^63 and more).
fn digits128(mut v: u128, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 39];
    let mut i = tmp.len();
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let len = tmp.len() - i;
    out[..len].copy_from_slice(&tmp[i..]);
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;

    fn check(x: f32) {
        let mut buf = [0u8; FIXED_MAX];
        for p in 0..=9 {
            let n = fixed(x, p, &mut buf);
            let want = format!("{:.*}", p, f64::from(x));
            assert_eq!(
                core::str::from_utf8(&buf[..n]).unwrap(),
                want,
                "x={x:e} bits={:08x} p={p}",
                x.to_bits()
            );
        }
    }

    #[test]
    fn fixed_matches_std_on_edges() {
        let edges = [
            0.0f32,
            -0.0,
            0.5,
            1.5,
            2.5,
            -2.5,
            0.25,
            0.125,
            0.375,
            1e-10,
            -1e-10,
            123.456,
            0.1,
            0.05,
            9.999_999,
            99.995,
            1e9,
            16_777_216.0,
            f32::MAX,
            f32::MIN,
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::EPSILON,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for x in edges {
            check(x);
        }
    }

    #[test]
    fn fixed_matches_std_on_random_values() {
        // Random bit patterns cover all exponents; values near 0..1e6 are the common case.
        let mut s = 0x2545_f491_4f6c_dd1du64;
        for i in 0..60_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = if i % 2 == 0 {
                f32::from_bits(s as u32)
            } else {
                (s % 100_000_000) as f32 / 100.0 - 500_000.0
            };
            check(x);
        }
    }

    #[test]
    fn int_matches_std() {
        let mut buf = [0u8; 11];
        for v in [0, 7, -7, 10, -10, 123_456, i32::MAX, i32::MIN] {
            let n = int(v, &mut buf);
            assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), v.to_string());
        }
    }
}
