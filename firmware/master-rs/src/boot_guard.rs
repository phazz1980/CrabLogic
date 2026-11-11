//! Boot loop guard: a stored module that hangs or crashes the board right at the start would
//! reset it again and again before any command could remove it. After [`SAFE_AFTER`] resets in a
//! row by a watchdog or a panic the stored modules are not started; `put` and `del` still work.
//!
//! Pure logic: the counter lives in memory that survives a reset (RTC) on the board, the reason of
//! the reset comes from ESP-IDF. A power-on or an ordinary reset clears the counter.

/// Bad resets in a row after which the master starts no stored module.
pub const SAFE_AFTER: u32 = 3;

/// Marks the RTC counter as set by this firmware: after a power-on the RTC memory holds garbage.
pub const MAGIC: u32 = 0xB007_B007;

/// Why the board started, as far as the guard cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// Power was switched on: the RTC memory is not valid.
    PowerOn,
    /// A watchdog or a panic reset the board.
    Bad,
    /// Anything else: the reset button, a software restart, a reset over USB.
    Other,
}

/// The counter kept in RTC memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counter {
    pub magic: u32,
    pub bad: u32,
}

impl Counter {
    /// Counts this boot and returns the number of bad resets in a row (0 for a good start).
    pub fn boot(&mut self, why: Reset) -> u32 {
        if why == Reset::PowerOn || self.magic != MAGIC {
            self.bad = 0;
        }
        self.magic = MAGIC;
        self.bad = if why == Reset::Bad {
            self.bad.saturating_add(1)
        } else {
            0
        };
        self.bad
    }
}

/// Whether the stored modules stay stopped after `bad` resets in a row.
pub fn safe(bad: u32) -> bool {
    bad >= SAFE_AFTER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_bad_resets_in_a_row_make_safe_mode() {
        let mut c = Counter::default();
        assert_eq!(c.boot(Reset::PowerOn), 0);
        assert!(!safe(c.boot(Reset::Bad)));
        assert!(!safe(c.boot(Reset::Bad)));
        assert!(safe(c.boot(Reset::Bad)));
        assert_eq!(c.boot(Reset::Bad), 4);
    }

    #[test]
    fn ordinary_reset_or_power_on_clears() {
        let mut c = Counter::default();
        c.boot(Reset::PowerOn);
        c.boot(Reset::Bad);
        c.boot(Reset::Bad);
        assert_eq!(c.boot(Reset::Other), 0);
        assert_eq!(c.boot(Reset::Bad), 1);
        c.boot(Reset::Bad);
        assert_eq!(c.boot(Reset::PowerOn), 0);
    }

    #[test]
    fn garbage_in_rtc_memory_is_not_counted() {
        let mut c = Counter {
            magic: 0x1234,
            bad: 99,
        };
        assert_eq!(c.boot(Reset::Bad), 1);
    }
}
