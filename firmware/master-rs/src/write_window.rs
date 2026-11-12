//! Counts flash writes: a total since boot and the writes of the last hour, kept in a ring of
//! 60 one-minute slots. Pure logic (time is passed in), so it is tested on the desktop.

const SLOTS: usize = 60;
const MINUTE_MS: u64 = 60_000;

#[derive(Debug, Clone)]
pub struct WriteWindow {
    total: u32,
    slots: [u32; SLOTS],
    /// The minute number (`ms / 60000`) the ring was last advanced to.
    minute: u64,
}

impl WriteWindow {
    pub const fn new() -> Self {
        Self {
            total: 0,
            slots: [0; SLOTS],
            minute: 0,
        }
    }

    /// Move the ring to `now_ms`, clearing the slots of the minutes that passed.
    fn advance(&mut self, now_ms: u64) {
        let m = now_ms / MINUTE_MS;
        if m <= self.minute {
            return; // the clock never goes back; ignore a stale caller
        }
        let gap = m - self.minute;
        if gap >= SLOTS as u64 {
            self.slots = [0; SLOTS];
        } else {
            for i in 1..=gap {
                self.slots[((self.minute + i) % SLOTS as u64) as usize] = 0;
            }
        }
        self.minute = m;
    }

    /// Record one write made at `now_ms` (uptime).
    pub fn record(&mut self, now_ms: u64) {
        self.advance(now_ms);
        self.total = self.total.wrapping_add(1);
        let i = (self.minute % SLOTS as u64) as usize;
        self.slots[i] = self.slots[i].saturating_add(1);
    }

    /// Writes since boot.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// Writes in the last hour (60 minute slots) as of `now_ms`.
    pub fn last_hour(&mut self, now_ms: u64) -> u32 {
        self.advance(now_ms);
        self.slots.iter().fold(0u32, |a, &b| a.saturating_add(b))
    }
}

impl Default for WriteWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_within_hour() {
        let mut w = WriteWindow::new();
        w.record(1_000);
        w.record(61_000);
        w.record(61_500);
        assert_eq!(w.total(), 3);
        assert_eq!(w.last_hour(120_000), 3);
    }

    #[test]
    fn expires_after_an_hour() {
        let mut w = WriteWindow::new();
        w.record(0);
        w.record(30 * MINUTE_MS);
        assert_eq!(w.last_hour(59 * MINUTE_MS), 2);
        // the minute-0 write leaves the window at minute 60
        assert_eq!(w.last_hour(60 * MINUTE_MS), 1);
        assert_eq!(w.last_hour(89 * MINUTE_MS), 1);
        assert_eq!(w.last_hour(90 * MINUTE_MS), 0);
        assert_eq!(w.total(), 2);
    }

    #[test]
    fn long_gap_clears_all() {
        let mut w = WriteWindow::new();
        for i in 0..10 {
            w.record(i * MINUTE_MS);
        }
        assert_eq!(w.last_hour(10 * 3600_000), 0);
        w.record(10 * 3600_000);
        assert_eq!(w.last_hour(10 * 3600_000), 1);
        assert_eq!(w.total(), 11);
    }

    #[test]
    fn ring_wraps_over_many_hours() {
        let mut w = WriteWindow::new();
        // one write per minute for 5 hours: always 60 in the window
        for m in 0..300u64 {
            w.record(m * MINUTE_MS + 5);
        }
        assert_eq!(w.last_hour(299 * MINUTE_MS + 5), 60);
        assert_eq!(w.total(), 300);
    }

    #[test]
    fn stale_time_is_ignored() {
        let mut w = WriteWindow::new();
        w.record(10 * MINUTE_MS);
        w.record(5 * MINUTE_MS); // earlier clock: counted in the current slot, no panic
        assert_eq!(w.last_hour(10 * MINUTE_MS), 2);
    }
}
