//! Frame of the star network: `header(1) | len(1) | data | CRC16(2)`, big-endian CRC.
//!
//! * header: bit 7 — type (0 request of the centre, 1 reply of a slave), bits 6..3 — node
//!   address 0..15 (the slave the frame is for or from), bits 2..0 — exchange number 0..7;
//! * len: number of data bytes; 0 = no values (a poll, or a reply without values);
//! * data: the values in port order, the types are known to both ends from the settings, so the
//!   frame carries none: first all `bool` as a bit mask (8 per byte, LSB first), then every `int`
//!   as a zigzag varint, then every `float` as 4 bytes little-endian;
//! * CRC16 (CCITT, poly 0x1021) over `header | len | data` starts from the fingerprint of the
//!   type lists of both ends, so a frame from a node with other types fails the check.
//!
//! The values are raw 32-bit cells: `bool` 0/1, `int` as `i32` bits, `float` as `f32` bits.
//! Nothing here allocates; every external byte is checked.

use crate::safe::{copy, Safe};

/// Most values in one exchange.
pub const MAX_VALUES: usize = 10;
/// Longest data field: ten `int` of five bytes (a mask byte never exceeds it).
pub const MAX_DATA: usize = MAX_VALUES * 5;
/// Longest frame.
pub const MAX_FRAME: usize = 2 + MAX_DATA + 2;
/// Shortest frame: header, len, CRC.
pub const MIN_FRAME: usize = 4;
/// Exchange numbers are 3 bits.
pub const SEQ_MASK: u8 = 7;
/// Node addresses are 4 bits.
pub const NODE_MAX: u8 = 15;

/// Type of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Bool,
    Int,
    Float,
}

/// What the header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// A reply of a slave (otherwise a request of the centre).
    pub reply: bool,
    /// Address of the slave.
    pub node: u8,
    /// Exchange number, 0..7.
    pub seq: u8,
}

/// Why a frame was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Shorter than the smallest frame or than its length field says.
    Short,
    /// The length field does not match the frame (or exceeds the limit).
    Length,
    /// Wrong checksum: damaged in the air, or the sender's types differ (fingerprint).
    Crc,
    /// The data field does not fit the types (bad varint, wrong size).
    Data,
}

/// What came out of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    pub header: Header,
    /// The frame carried values (they are in `out`); false: a poll without values.
    pub values: bool,
}

/// Fingerprint of the two type lists: the centre→slave values and the slave→centre values.
/// Both ends compute it from their settings (FNV-1a folded to 16 bits).
pub fn fingerprint(request: &[Ty], reply: &[Ty]) -> u16 {
    let mut h: u32 = 0x811C_9DC5;
    for list in [request, reply] {
        h = (h ^ list.len() as u32).wrapping_mul(0x0100_0193);
        for t in list {
            let code = match t {
                Ty::Bool => 1,
                Ty::Int => 2,
                Ty::Float => 3,
            };
            h = (h ^ code).wrapping_mul(0x0100_0193);
        }
    }
    (h ^ (h >> 16)) as u16
}

fn crc16(init: u16, data: &[u8]) -> u16 {
    let mut crc = init;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

fn unzigzag(z: u32) -> i32 {
    ((z >> 1) as i32) ^ -((z & 1) as i32)
}

/// Writes the data field of `cells` (one cell per type) to `out`; the length, or `None` when it
/// does not fit or the counts differ.
fn put_data(types: &[Ty], cells: &[u32], out: &mut [u8]) -> Option<usize> {
    if types.len() != cells.len() || types.len() > MAX_VALUES {
        return None;
    }
    let mut n = 0;
    let bools = types.iter().filter(|t| **t == Ty::Bool).count();
    for chunk in 0..bools.div_ceil(8) {
        let mut byte = 0u8;
        let mut k = 0;
        for (t, c) in types.iter().zip(cells) {
            if *t != Ty::Bool {
                continue;
            }
            if k / 8 == chunk && *c != 0 {
                byte |= 1 << (k % 8);
            }
            k += 1;
        }
        *out.get_mut(n)? = byte;
        n += 1;
    }
    for (t, c) in types.iter().zip(cells) {
        if *t != Ty::Int {
            continue;
        }
        let mut z = zigzag(*c as i32);
        loop {
            let low = (z & 0x7F) as u8;
            z >>= 7;
            *out.get_mut(n)? = if z == 0 { low } else { low | 0x80 };
            n += 1;
            if z == 0 {
                break;
            }
        }
    }
    for (t, c) in types.iter().zip(cells) {
        if *t == Ty::Float {
            copy(out.get_mut(n..n + 4)?, &c.to_le_bytes());
            n += 4;
        }
    }
    Some(n)
}

/// Reads the data field into `out` (one cell per type); `None` if it does not fit the types.
fn get_data(types: &[Ty], data: &[u8], out: &mut [u32]) -> Option<()> {
    if types.len() > MAX_VALUES || out.len() < types.len() {
        return None;
    }
    let bools = types.iter().filter(|t| **t == Ty::Bool).count();
    let mask = data.get(..bools.div_ceil(8))?;
    let mut at = mask.len();
    let mut k = 0;
    for (i, t) in types.iter().enumerate() {
        if *t == Ty::Bool {
            out.wr(i, u32::from((mask.rd(k / 8) >> (k % 8)) & 1));
            k += 1;
        }
    }
    for (i, t) in types.iter().enumerate() {
        if *t != Ty::Int {
            continue;
        }
        let mut z: u32 = 0;
        for shift in (0..35).step_by(7) {
            let b = *data.get(at)?;
            at += 1;
            let part = u32::from(b & 0x7F);
            // The fifth byte holds the last four bits.
            if shift == 28 && (b & 0x80 != 0 || part > 0x0F) {
                return None;
            }
            z |= part << shift;
            if b & 0x80 == 0 {
                break;
            }
        }
        out.wr(i, unzigzag(z) as u32);
    }
    for (i, t) in types.iter().enumerate() {
        if *t == Ty::Float {
            let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
            out.wr(i, u32::from_le_bytes(bytes));
            at += 4;
        }
    }
    (at == data.len()).then_some(())
}

/// Builds a frame into `buf`; `cells` empty = a frame without values (a poll). Returns its
/// length, or `None` if `buf` is too small, the header is out of range or the counts differ.
pub fn encode(
    buf: &mut [u8],
    header: Header,
    types: &[Ty],
    cells: &[u32],
    fingerprint: u16,
) -> Option<usize> {
    if header.node > NODE_MAX || header.seq > SEQ_MASK {
        return None;
    }
    let data_len = if cells.is_empty() {
        0
    } else {
        put_data(types, cells, buf.get_mut(2..)?)?
    };
    *buf.get_mut(0)? = (u8::from(header.reply) << 7) | (header.node << 3) | header.seq;
    *buf.get_mut(1)? = data_len as u8;
    let body = 2 + data_len;
    let crc = crc16(fingerprint, buf.get(..body)?);
    copy(buf.get_mut(body..body + 2)?, &crc.to_be_bytes());
    Some(body + 2)
}

/// Checks a received frame and reads its values into `out`. Only the length, the checksum and the
/// shape of the data are checked here; the node, the number and the type are the caller's business.
pub fn decode(
    frame: &[u8],
    types: &[Ty],
    fingerprint: u16,
    out: &mut [u32],
) -> Result<Decoded, Error> {
    if frame.len() < MIN_FRAME {
        return Err(Error::Short);
    }
    let data_len = usize::from(frame.rd(1));
    if data_len > MAX_DATA {
        return Err(Error::Length);
    }
    if frame.len() != 4 + data_len {
        return Err(if frame.len() < 4 + data_len {
            Error::Short
        } else {
            Error::Length
        });
    }
    let body = 2 + data_len;
    let crc = u16::from_be_bytes([frame.rd(body), frame.rd(body + 1)]);
    if crc16(fingerprint, frame.part(body)) != crc {
        return Err(Error::Crc);
    }
    let h = frame.rd(0);
    let header = Header {
        reply: h & 0x80 != 0,
        node: (h >> 3) & 0x0F,
        seq: h & SEQ_MASK,
    };
    if data_len == 0 {
        return Ok(Decoded {
            header,
            values: false,
        });
    }
    get_data(types, frame.span(2, body), out).ok_or(Error::Data)?;
    Ok(Decoded {
        header,
        values: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPES: [Ty; 5] = [Ty::Int, Ty::Bool, Ty::Float, Ty::Bool, Ty::Int];

    fn fp() -> u16 {
        fingerprint(&TYPES, &[Ty::Float])
    }

    fn header() -> Header {
        Header {
            reply: false,
            node: 5,
            seq: 3,
        }
    }

    fn round(types: &[Ty], cells: &[u32]) -> ([u32; MAX_VALUES], usize) {
        let fp = fingerprint(types, &[]);
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, header(), types, cells, fp).unwrap();
        let mut out = [0u32; MAX_VALUES];
        let d = decode(&buf[..n], types, fp, &mut out).unwrap();
        assert_eq!(d.header, header());
        assert!(d.values);
        (out, n)
    }

    #[test]
    fn round_trip_mixed_types() {
        let cells = [(-5i32) as u32, 1, 1.5f32.to_bits(), 0, 100_000i32 as u32];
        let (out, n) = round(&TYPES, &cells);
        assert_eq!(&out[..5], &cells);
        // header, len, mask, -5 (1), 100000 (3), float (4), crc (2)
        assert_eq!(n, 2 + 1 + 1 + 3 + 4 + 2);
    }

    #[test]
    fn int_limits_and_small_values() {
        for v in [0, 1, -1, 63, -64, 64, 8191, i32::MAX, i32::MIN] {
            let (out, _) = round(&[Ty::Int], &[v as u32]);
            assert_eq!(out[0] as i32, v, "{v}");
        }
        // Small values take one byte: header, len, 1 byte, crc.
        assert_eq!(round(&[Ty::Int], &[(-3i32) as u32]).1, 5);
    }

    #[test]
    fn many_bools_use_a_bit_mask() {
        let types = [Ty::Bool; 10];
        let cells = [1, 0, 1, 1, 0, 0, 0, 1, 1, 0];
        let (out, n) = round(&types, &cells);
        assert_eq!(&out[..10], &cells);
        assert_eq!(n, 2 + 2 + 2);
    }

    #[test]
    fn worst_case_fits_the_limit() {
        let types = [Ty::Int; MAX_VALUES];
        let cells = [i32::MIN as u32; MAX_VALUES];
        assert_eq!(round(&types, &cells).1, MAX_FRAME);
    }

    #[test]
    fn poll_without_values() {
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, header(), &TYPES, &[], fp()).unwrap();
        assert_eq!(n, MIN_FRAME);
        let mut out = [7u32; MAX_VALUES];
        let d = decode(&buf[..n], &TYPES, fp(), &mut out).unwrap();
        assert!(!d.values);
        assert_eq!(out, [7; MAX_VALUES], "cells stay untouched");
    }

    #[test]
    fn header_fields() {
        let h = Header {
            reply: true,
            node: 15,
            seq: 7,
        };
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, h, &[], &[], 0).unwrap();
        assert_eq!(buf[0], 0xFF);
        assert_eq!(decode(&buf[..n], &[], 0, &mut []).unwrap().header, h);
        assert!(encode(&mut buf, Header { node: 16, ..h }, &[], &[], 0).is_none());
        assert!(encode(&mut buf, Header { seq: 8, ..h }, &[], &[], 0).is_none());
    }

    #[test]
    fn damaged_frame_is_refused() {
        let cells = [1, 1, 2.0f32.to_bits(), 0, 7];
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, header(), &TYPES, &cells, fp()).unwrap();
        let mut out = [0u32; MAX_VALUES];
        for i in 0..n {
            for bit in 0..8 {
                let mut bad = buf;
                bad[i] ^= 1 << bit;
                assert!(
                    decode(&bad[..n], &TYPES, fp(), &mut out).is_err(),
                    "byte {i} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn short_and_long_frames() {
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, header(), &TYPES, &[0; 5], fp()).unwrap();
        let mut out = [0u32; MAX_VALUES];
        assert_eq!(decode(&[], &TYPES, fp(), &mut out), Err(Error::Short));
        assert_eq!(decode(&buf[..3], &TYPES, fp(), &mut out), Err(Error::Short));
        assert_eq!(
            decode(&buf[..n - 1], &TYPES, fp(), &mut out),
            Err(Error::Short)
        );
        buf[n] = 0;
        assert_eq!(
            decode(&buf[..n + 1], &TYPES, fp(), &mut out),
            Err(Error::Length)
        );
        let mut huge = [0u8; 8];
        huge[1] = 200;
        assert_eq!(decode(&huge, &TYPES, fp(), &mut out), Err(Error::Length));
    }

    #[test]
    fn other_fingerprint_fails_the_checksum() {
        let mut buf = [0u8; MAX_FRAME];
        let n = encode(&mut buf, header(), &TYPES, &[0; 5], fp()).unwrap();
        let other = fingerprint(&[Ty::Int; 5], &[Ty::Float]);
        assert_ne!(other, fp());
        let mut out = [0u32; MAX_VALUES];
        assert_eq!(decode(&buf[..n], &TYPES, other, &mut out), Err(Error::Crc));
    }

    #[test]
    fn data_that_does_not_fit_the_types_is_refused() {
        // A valid frame (right checksum) of one type list read with another count of values.
        let mut buf = [0u8; MAX_FRAME];
        let sender = [Ty::Int, Ty::Int];
        let n = encode(&mut buf, header(), &sender, &[1, 2], 9).unwrap();
        let mut out = [0u32; MAX_VALUES];
        assert_eq!(decode(&buf[..n], &[Ty::Int], 9, &mut out), Err(Error::Data));
        assert_eq!(
            decode(&buf[..n], &[Ty::Int, Ty::Int, Ty::Int], 9, &mut out),
            Err(Error::Data)
        );
    }

    #[test]
    fn overlong_varint_is_refused() {
        // Six continuation bytes and a fifth byte with too many bits, under a valid checksum.
        for data in [
            &[0x80u8, 0x80, 0x80, 0x80, 0x80, 0x01][..],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0x1F][..],
        ] {
            let mut frame = [0u8; 16];
            frame[1] = data.len() as u8;
            frame[2..2 + data.len()].copy_from_slice(data);
            let body = 2 + data.len();
            let crc = crc16(0, &frame[..body]);
            frame[body..body + 2].copy_from_slice(&crc.to_be_bytes());
            let mut out = [0u32; MAX_VALUES];
            assert_eq!(
                decode(&frame[..body + 2], &[Ty::Int], 0, &mut out),
                Err(Error::Data)
            );
        }
    }

    #[test]
    fn encode_checks_its_arguments() {
        let mut small = [0u8; 4];
        assert!(encode(&mut small, header(), &TYPES, &[0; 5], fp()).is_none());
        let mut buf = [0u8; MAX_FRAME];
        assert!(encode(&mut buf, header(), &TYPES, &[0; 4], fp()).is_none());
        let many = [Ty::Bool; MAX_VALUES + 1];
        assert!(encode(&mut buf, header(), &many, &[0; MAX_VALUES + 1], 0).is_none());
    }

    #[test]
    fn fingerprint_depends_on_both_lists_and_order() {
        let a = fingerprint(&[Ty::Int, Ty::Bool], &[Ty::Float]);
        assert_ne!(a, fingerprint(&[Ty::Bool, Ty::Int], &[Ty::Float]));
        assert_ne!(a, fingerprint(&[Ty::Int, Ty::Bool], &[Ty::Int]));
        assert_ne!(a, fingerprint(&[Ty::Int], &[Ty::Bool, Ty::Float]));
    }
}
