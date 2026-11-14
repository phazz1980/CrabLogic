//! Login of a network client of the master (`specs/ota`, phase 1): the parts that need no ESP-IDF,
//! tested on the desktop. The board computes the HMAC itself (mbedtls) and keeps the key in NVS.
//!
//! - The key is `SHA-256("crablogic" || project password)`, 32 bytes, written over USB only
//!   (`netkey <64 hex>`); neither the password nor the key goes over the network.
//! - On connect the board sends `auth <32 hex>` (a fresh 16-byte nonce); the client answers
//!   `auth <64 hex>` = HMAC-SHA256(key, nonce); `ok` or `err auth`.
//! - Wrong answers are slowed down (one per second) and after [`MAX_FAILURES`] in a row the port
//!   refuses everyone for [`LOCK_MS`].

/// TCP port of the command server.
pub const PORT: u16 = 4040;
/// Failed logins in a row before the lock.
pub const MAX_FAILURES: u32 = 5;
/// How long the port refuses after too many failures, ms.
pub const LOCK_MS: u64 = 60_000;
/// Pause after a failed login, ms.
pub const FAIL_PAUSE_MS: u64 = 1_000;

/// `n` bytes from lowercase or uppercase hex, exactly `2 * N` digits.
pub fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != 2 * N {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = digit(text[2 * i])? << 4 | digit(text[2 * i + 1])?;
    }
    Some(out)
}

/// Lowercase hex of `bytes` into `out` (two digits per byte); returns the written part.
pub fn to_hex<'a>(bytes: &[u8], out: &'a mut [u8]) -> &'a str {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let n = bytes.len().min(out.len() / 2);
    for (i, b) in bytes[..n].iter().enumerate() {
        out[2 * i] = DIGITS[usize::from(b >> 4)];
        out[2 * i + 1] = DIGITS[usize::from(b & 15)];
    }
    // Only ASCII digits were written.
    core::str::from_utf8(&out[..2 * n]).unwrap_or("")
}

/// Equal without an early exit (the time does not tell how many bytes matched).
pub fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The answer line of a client: `auth <64 hex>`.
pub fn parse_answer(line: &str) -> Option<[u8; 32]> {
    from_hex(line.trim().strip_prefix("auth ")?.trim())
}

/// Failed logins in a row and the lock after too many.
#[derive(Debug, Default, Clone, Copy)]
pub struct Limiter {
    failures: u32,
    /// Uptime (ms) until which the port refuses, when locked.
    locked_until: Option<u64>,
}

impl Limiter {
    /// Whether a client may try now; ends a lock that is over.
    pub fn allows(&mut self, now_ms: u64) -> bool {
        match self.locked_until {
            Some(until) if now_ms < until => false,
            Some(_) => {
                self.locked_until = None;
                self.failures = 0;
                true
            }
            None => true,
        }
    }

    pub fn failed(&mut self, now_ms: u64) {
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.locked_until = Some(now_ms + LOCK_MS);
        }
    }

    pub fn succeeded(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let bytes = [0x00, 0x7f, 0xab, 0xff];
        let mut buf = [0u8; 8];
        let text = to_hex(&bytes, &mut buf);
        assert_eq!(text, "007fabff");
        assert_eq!(from_hex::<4>(text), Some(bytes));
        assert_eq!(from_hex::<4>("007FABFF"), Some(bytes));
        assert_eq!(from_hex::<4>("007fab"), None);
        assert_eq!(from_hex::<4>("007fabfg"), None);
    }

    #[test]
    fn answers_and_compare() {
        let line = format!("auth {}", "ab".repeat(32));
        assert_eq!(parse_answer(&line), Some([0xab; 32]));
        assert_eq!(parse_answer("auth 12"), None);
        assert_eq!(parse_answer("netkey 00"), None);
        assert!(same(&[1, 2, 3], &[1, 2, 3]));
        assert!(!same(&[1, 2, 3], &[1, 2, 4]));
        assert!(!same(&[1, 2], &[1, 2, 3]));
    }

    #[test]
    fn locks_after_five_failures_for_a_minute() {
        let mut l = Limiter::default();
        for i in 0..MAX_FAILURES {
            assert!(l.allows(i as u64));
            l.failed(i as u64);
        }
        assert!(!l.allows(10));
        assert!(!l.allows(4 + LOCK_MS - 1));
        assert!(l.allows(4 + LOCK_MS));
        l.failed(4 + LOCK_MS);
        assert!(
            l.allows(4 + LOCK_MS + 1),
            "the count starts afresh after a lock"
        );
        l.succeeded();
        for _ in 0..MAX_FAILURES - 1 {
            l.failed(0);
        }
        assert!(l.allows(0));
    }
}
