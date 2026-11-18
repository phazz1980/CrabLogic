//! `crab-netfind`: wire format of the LAN board search (`specs/net-find`).
//!
//! The PC sends one broadcast datagram ([`request`]) to [`PORT`]. Every board with a network
//! address answers with one text record ([`encode_reply`]) sent back to the sender only. A reply
//! carries no secrets: no keys, passwords or project contents.
//!
//! No dependencies; `no_std` + `alloc`, so the board firmware and the PC crates share one format.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// UDP port of the search (separate from the command port 4040).
pub const PORT: u16 = 4041;
/// Protocol version; a request or reply with another version is rejected.
pub const PROTOCOL: u8 = 1;
/// Length of the request datagram.
pub const REQUEST_LEN: usize = 9;
/// Longest reply record in bytes.
pub const MAX_REPLY: usize = 200;
/// Shortest allowed gap between two replies of one board, in milliseconds.
pub const MIN_INTERVAL_MS: u32 = 200;

const REQUEST_TAG: &[u8; 8] = b"CRABFIND";
const REPLY_TAG: &str = "CRABHERE";
const SEP: u8 = b'\t';
const NAME_MAX: usize = 40;
const ID_LEN: usize = 12;
const HASH_LEN: usize = 8;
const NO_HASH: &str = "-";

/// One board as seen by the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    /// 12 lowercase hex digits (the MAC address).
    pub id: String,
    /// Chip name, for example `esp32` or `esp32c3`.
    pub chip: String,
    /// Master firmware version, for example `2.0.1`.
    pub master: String,
    /// Short project hash: 8 lowercase hex digits, or `None` without a project.
    pub hash: Option<String>,
    /// Whether the board has a network key (the key itself is never sent).
    pub has_key: bool,
    /// TCP command port.
    pub port: u16,
    /// Project name; may be empty.
    pub name: String,
}

/// The search request: `CRABFIND` followed by [`PROTOCOL`].
pub fn request() -> [u8; REQUEST_LEN] {
    let mut out = [0u8; REQUEST_LEN];
    out[..REQUEST_TAG.len()].copy_from_slice(REQUEST_TAG);
    out[REQUEST_LEN - 1] = PROTOCOL;
    out
}

/// True only for exactly the 9-byte request with a known protocol version.
pub fn is_request(packet: &[u8]) -> bool {
    packet.len() == REQUEST_LEN
        && packet[..REQUEST_TAG.len()] == REQUEST_TAG[..]
        && packet[REQUEST_LEN - 1] == PROTOCOL
}

/// Writes one reply record for `board` into `out`.
///
/// The record is `CRABHERE`, protocol, id, chip, master, hash (`-` if none), has_key (`1`/`0`),
/// port and name, separated by single TABs. Control characters in the name become spaces and the
/// name is cut to 40 bytes on a char boundary. Returns `false` and leaves `out` unchanged if a
/// field is invalid or the record would be longer than [`MAX_REPLY`].
pub fn encode_reply(board: &Board, out: &mut Vec<u8>) -> bool {
    if !is_id(&board.id) || !is_token(&board.chip) || !is_token(&board.master) {
        return false;
    }
    let hash = match board.hash.as_deref() {
        None => None,
        Some(h) if is_hash(h) => Some(h),
        Some(_) => return false,
    };

    let start = out.len();
    out.extend_from_slice(REPLY_TAG.as_bytes());
    out.push(SEP);
    push_decimal(out, u32::from(PROTOCOL));
    out.push(SEP);
    out.extend_from_slice(board.id.as_bytes());
    out.push(SEP);
    out.extend_from_slice(board.chip.as_bytes());
    out.push(SEP);
    out.extend_from_slice(board.master.as_bytes());
    out.push(SEP);
    match hash {
        Some(h) => out.extend_from_slice(h.as_bytes()),
        None => out.extend_from_slice(NO_HASH.as_bytes()),
    }
    out.push(SEP);
    out.push(if board.has_key { b'1' } else { b'0' });
    out.push(SEP);
    push_decimal(out, u32::from(board.port));
    out.push(SEP);
    push_name(out, &board.name);

    if out.len() - start > MAX_REPLY {
        out.truncate(start);
        return false;
    }
    true
}

/// Parses one reply record. The inverse of [`encode_reply`]: anything not in exactly that format
/// gives `None`.
pub fn decode_reply(packet: &[u8]) -> Option<Board> {
    if packet.len() > MAX_REPLY {
        return None;
    }
    let text = core::str::from_utf8(packet).ok()?;
    // The name is the last field: splitting into at most 9 parts keeps any extra TAB inside the
    // name, where it is a control character and gets rejected below.
    let mut fields = text.splitn(9, '\t');

    if fields.next()? != REPLY_TAG {
        return None;
    }
    if parse_decimal(fields.next()?)? != u32::from(PROTOCOL) {
        return None;
    }
    let id = fields.next()?;
    if !is_id(id) {
        return None;
    }
    let chip = fields.next()?;
    if !is_token(chip) {
        return None;
    }
    let master = fields.next()?;
    if !is_token(master) {
        return None;
    }
    let hash = match fields.next()? {
        NO_HASH => None,
        h if is_hash(h) => Some(String::from(h)),
        _ => return None,
    };
    let has_key = match fields.next()? {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    let port = u16::try_from(parse_decimal(fields.next()?)?).ok()?;
    let name = fields.next()?;
    if name.len() > NAME_MAX || name.chars().any(char::is_control) {
        return None;
    }

    Some(Board {
        id: String::from(id),
        chip: String::from(chip),
        master: String::from(master),
        hash,
        has_key,
        port,
        name: String::from(name),
    })
}

/// Allows at most one reply per `min_interval_ms`, so a flood of requests cannot make the board
/// send a flood of replies. Works with wrapping `u32` milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyLimiter {
    min_interval_ms: u32,
    last_ms: Option<u32>,
}

impl ReplyLimiter {
    /// A limiter that allows one reply per `min_interval_ms`.
    pub const fn new(min_interval_ms: u32) -> Self {
        Self {
            min_interval_ms,
            last_ms: None,
        }
    }

    /// True if a reply may be sent at `now_ms`; the first call is always allowed. A denied call
    /// does not move the window.
    pub fn allow(&mut self, now_ms: u32) -> bool {
        let allowed = match self.last_ms {
            None => true,
            Some(last) => now_ms.wrapping_sub(last) >= self.min_interval_ms,
        };
        if allowed {
            self.last_ms = Some(now_ms);
        }
        allowed
    }
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

fn is_id(s: &str) -> bool {
    s.len() == ID_LEN && s.bytes().all(is_lower_hex)
}

fn is_hash(s: &str) -> bool {
    s.len() == HASH_LEN && s.bytes().all(is_lower_hex)
}

/// Non-empty printable ASCII without spaces.
fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// Writes the name with control characters replaced, cut to [`NAME_MAX`] bytes.
fn push_name(out: &mut Vec<u8>, name: &str) {
    let mut used = 0;
    for c in name.chars() {
        let c = if c.is_control() { ' ' } else { c };
        let len = c.len_utf8();
        if used + len > NAME_MAX {
            break;
        }
        used += len;
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
}

/// Writes `value` in decimal without allocating.
fn push_decimal(out: &mut Vec<u8>, value: u32) {
    let mut buf = [0u8; 10];
    let mut rest = value;
    let mut at = buf.len();
    loop {
        at -= 1;
        buf[at] = b'0' + (rest % 10) as u8;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[at..]);
}

/// Parses a canonical decimal number: digits only, no sign, no leading zeros.
fn parse_decimal(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 10 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return None;
    }
    s.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        Board {
            id: "a1b2c3d4e5f6".into(),
            chip: "esp32".into(),
            master: "2.0.1".into(),
            hash: Some("0badf00d".into()),
            has_key: true,
            port: 4040,
            name: "Pump station".into(),
        }
    }

    fn encode(b: &Board) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(encode_reply(b, &mut out));
        out
    }

    fn round_trip(b: &Board) -> Board {
        decode_reply(&encode(b)).expect("decodes")
    }

    #[test]
    fn request_round_trip() {
        let r = request();
        assert_eq!(&r[..8], b"CRABFIND");
        assert_eq!(r[8], PROTOCOL);
        assert!(is_request(&r));
    }

    #[test]
    fn request_rejects_other_packets() {
        assert!(!is_request(&[]));
        assert!(!is_request(b"CRABFIND"));
        assert!(!is_request(b"CRABFIND\x01\x00"));
        assert!(!is_request(b"hello"));
        assert!(!is_request(b"CRABFINX\x01"));
        let mut other_version = request();
        other_version[8] = PROTOCOL + 1;
        assert!(!is_request(&other_version));
    }

    #[test]
    fn reply_text_is_as_specified() {
        let bytes = encode(&board());
        assert_eq!(
            core::str::from_utf8(&bytes).unwrap(),
            "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d\t1\t4040\tPump station"
        );
    }

    #[test]
    fn reply_round_trip_normal_board() {
        assert_eq!(round_trip(&board()), board());
    }

    #[test]
    fn reply_round_trip_without_hash() {
        let mut b = board();
        b.hash = None;
        assert_eq!(round_trip(&b), b);
        assert!(core::str::from_utf8(&encode(&b)).unwrap().contains("\t-\t"));
    }

    #[test]
    fn reply_round_trip_without_key() {
        let mut b = board();
        b.has_key = false;
        assert_eq!(round_trip(&b), b);
    }

    #[test]
    fn reply_round_trip_empty_name() {
        let mut b = board();
        b.name = String::new();
        assert_eq!(round_trip(&b), b);
    }

    #[test]
    fn name_control_characters_become_spaces() {
        let mut b = board();
        b.name = "a\tb\nc\rd".into();
        let decoded = round_trip(&b);
        assert_eq!(decoded.name, "a b c d");
        assert_eq!(decoded.port, b.port);
    }

    #[test]
    fn name_is_cut_on_char_boundary() {
        let mut b = board();
        // 20 Cyrillic letters are exactly 40 bytes.
        b.name = "Ж".repeat(25);
        let decoded = round_trip(&b);
        assert_eq!(decoded.name, "Ж".repeat(20));
        assert_eq!(decoded.name.len(), 40);

        // 'a' + 19 letters = 39 bytes; the 20th letter would not fit in 40 bytes.
        b.name = alloc::format!("a{}", "Ж".repeat(30));
        let decoded = round_trip(&b);
        assert_eq!(decoded.name, alloc::format!("a{}", "Ж".repeat(19)));
    }

    #[test]
    fn encode_rejects_bad_id_and_chip() {
        let bad_ids = [
            "A1B2C3D4E5F6",
            "a1b2c3d4e5f",
            "a1b2c3d4e5f67",
            "a1b2c3d4e5 6",
            "",
        ];
        for id in bad_ids {
            let mut b = board();
            b.id = id.into();
            let mut out = alloc::vec![9u8];
            assert!(!encode_reply(&b, &mut out), "id {id:?}");
            assert_eq!(out, [9u8]);
        }

        let bad_chips = ["esp 32", "", "esp32\t", "esp32\n"];
        for chip in bad_chips {
            let mut b = board();
            b.chip = chip.into();
            let mut out = Vec::new();
            assert!(!encode_reply(&b, &mut out), "chip {chip:?}");
            assert!(out.is_empty());
        }
    }

    #[test]
    fn encode_rejects_bad_master_and_hash() {
        let mut b = board();
        b.master = "2 0".into();
        assert!(!encode_reply(&b, &mut Vec::new()));

        for hash in ["0badf00", "0badf00da", "0BADF00D", "0badf00g"] {
            let mut b = board();
            b.hash = Some(hash.into());
            assert!(!encode_reply(&b, &mut Vec::new()), "hash {hash:?}");
        }
    }

    #[test]
    fn encode_rejects_too_long_record_and_keeps_out() {
        let mut b = board();
        b.master = "x".repeat(150);
        let mut out = alloc::vec![7u8, 7u8];
        assert!(!encode_reply(&b, &mut out));
        assert_eq!(out, [7u8, 7u8]);
    }

    #[test]
    fn encode_appends_to_existing_content() {
        let mut out = alloc::vec![b'>'];
        assert!(encode_reply(&board(), &mut out));
        assert_eq!(out[0], b'>');
        assert_eq!(decode_reply(&out[1..]), Some(board()));
    }

    #[test]
    fn decode_rejects_truncated() {
        assert_eq!(decode_reply(b""), None);
        assert_eq!(
            decode_reply(b"CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d\t1\t4040"),
            None
        );
        assert_eq!(
            decode_reply(b"CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d\t1"),
            None
        );
    }

    #[test]
    fn decode_rejects_extra_fields() {
        assert_eq!(
            decode_reply(
                b"CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d\t1\t4040\tname\textra"
            ),
            None
        );
    }

    #[test]
    fn decode_rejects_wrong_tag_and_protocol() {
        let rest = "\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d\t1\t4040\tn";
        for head in ["CRABHEREX\t1", "CRABFIND\t1", "CRABHERE\t2", "CRABHERE\t01"] {
            let text = alloc::format!("{head}{rest}");
            assert_eq!(decode_reply(text.as_bytes()), None, "{head:?}");
        }
    }

    #[test]
    fn decode_rejects_non_utf8() {
        let mut bytes = encode(&board());
        let last = bytes.len() - 1;
        bytes[last] = 0xff;
        assert_eq!(decode_reply(&bytes), None);
    }

    #[test]
    fn decode_rejects_longer_than_max() {
        let mut bytes = encode(&board());
        bytes.resize(MAX_REPLY + 1, b'x');
        assert_eq!(decode_reply(&bytes), None);
    }

    #[test]
    fn decode_rejects_bad_flag_and_port() {
        let base = "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00d";
        for tail in [
            "\t2\t4040\tn",
            "\t\t4040\tn",
            "\tyes\t4040\tn",
            "\t1\t70000\tn",
            "\t1\t+80\tn",
            "\t1\t080\tn",
            "\t1\t\tn",
            "\t1\tabc\tn",
            "\t1\t-1\tn",
        ] {
            let text = alloc::format!("{base}{tail}");
            assert_eq!(decode_reply(text.as_bytes()), None, "{tail:?}");
        }
    }

    #[test]
    fn decode_rejects_bad_id_and_hash() {
        let upper = "CRABHERE\t1\tA1B2C3D4E5F6\tesp32\t2.0.1\t0badf00d\t1\t4040\tn";
        assert_eq!(decode_reply(upper.as_bytes()), None);
        let short_hash = "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t0badf00\t1\t4040\tn";
        assert_eq!(decode_reply(short_hash.as_bytes()), None);
    }

    #[test]
    fn decode_rejects_control_chars_and_long_name() {
        let ctrl = "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t-\t0\t4040\tPump\x07";
        assert_eq!(decode_reply(ctrl.as_bytes()), None);
        let tab_in_name = "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t-\t0\t4040\tPump\tx";
        assert_eq!(decode_reply(tab_in_name.as_bytes()), None);
        let long = alloc::format!(
            "CRABHERE\t1\ta1b2c3d4e5f6\tesp32\t2.0.1\t-\t0\t4040\t{}",
            "x".repeat(41)
        );
        assert_eq!(decode_reply(long.as_bytes()), None);
    }

    #[test]
    fn limiter_first_allowed_then_interval() {
        let mut l = ReplyLimiter::new(100);
        assert!(l.allow(1000));
        assert!(!l.allow(1050));
        assert!(!l.allow(1099));
        assert!(l.allow(1100));
        assert!(!l.allow(1150));
    }

    #[test]
    fn limiter_works_across_u32_wrap() {
        let mut l = ReplyLimiter::new(MIN_INTERVAL_MS);
        let start = u32::MAX - 50;
        assert!(l.allow(start));
        assert!(!l.allow(start.wrapping_add(10)));
        // 40 is 91 ms after `start` across the wrap: still inside the interval.
        assert!(!l.allow(40));
        // 149 is exactly 200 ms after `start` across the wrap.
        assert!(l.allow(start.wrapping_add(200)));
        assert!(!l.allow(start.wrapping_add(250)));
    }
}
