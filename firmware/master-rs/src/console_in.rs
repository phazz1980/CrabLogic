//! Console input (`tin <hex>`): bytes the editor types in the debug console wait here until a
//! `uart_read` block on USB takes them. A fixed ring, nothing is allocated; a line that does not
//! fit is refused as a whole (the editor sends it again), so the module never sees half a line.
//! The board side is `board/uart.rs` (port 0, ops 1 and 2).

/// Bytes the ring holds.
pub const CAPACITY: usize = 256;

/// Most bytes in one `tin` line (the line stays far below the 512-byte limit of the parser).
pub const MAX_CHUNK: usize = 128;

pub struct ConsoleIn {
    data: [u8; CAPACITY],
    head: usize,
    len: usize,
}

impl ConsoleIn {
    pub const fn new() -> Self {
        Self {
            data: [0; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    /// Bytes waiting.
    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Takes all of `bytes` or none: `false` when they do not fit.
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > CAPACITY - self.len {
            return false;
        }
        for &b in bytes {
            self.data[(self.head + self.len) % CAPACITY] = b;
            self.len += 1;
        }
        true
    }

    /// The oldest byte.
    pub fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.data[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(b)
    }

    /// Drops everything (a new schema starts with an empty input).
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

impl Default for ConsoleIn {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_order_and_wraps() {
        let mut ring = ConsoleIn::new();
        assert!(ring.push(&[1, 2, 3]));
        assert_eq!(ring.pop(), Some(1));
        let more: Vec<u8> = (10..10 + (CAPACITY - 2) as u32).map(|b| b as u8).collect();
        assert!(ring.push(&more), "the ring wraps around its end");
        assert_eq!(ring.len(), CAPACITY);
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        for b in &more {
            assert_eq!(ring.pop(), Some(*b));
        }
        assert!(ring.is_empty());
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn a_line_that_does_not_fit_is_refused_whole() {
        let mut ring = ConsoleIn::new();
        assert!(ring.push(&[7; CAPACITY - 1]));
        assert!(!ring.push(&[1, 2]));
        assert_eq!(ring.len(), CAPACITY - 1);
        assert!(ring.push(&[1]));
        assert!(!ring.push(&[1]));
        ring.clear();
        assert!(ring.is_empty());
    }
}
