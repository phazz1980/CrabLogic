//! Restart policy of a failed module: stop it, wait, start it again from the stored image.
//!
//! Pure logic with no clock of its own: time comes in as `now_ms` (milliseconds since boot).
//! The pause doubles while failures keep coming, so a relay does not chatter.

/// First pause after a failure.
pub const FIRST_DELAY_MS: u64 = 1_000;
/// The pause never grows beyond this.
pub const MAX_DELAY_MS: u64 = 30_000;
/// A module that ran this long without a failure starts again from the first pause.
pub const CALM_MS: u64 = 60_000;

/// When a failed module may run again.
#[derive(Debug, Clone, Default)]
pub struct Restart {
    /// Pause for the next failure; 0 means "not set yet" (the first pause).
    next_delay_ms: u64,
    /// When the module was last started.
    started_at_ms: u64,
    /// Some(due time) while the module is stopped and waiting.
    due_at_ms: Option<u64>,
    restarts: u32,
}

impl Restart {
    pub fn new() -> Self {
        Self::default()
    }

    /// A step failed at `now_ms`: the module stops; returns the pause before its restart.
    /// A failure while already waiting keeps the old due time and does not grow the pause.
    pub fn failed(&mut self, now_ms: u64) -> u64 {
        if self.due_at_ms.is_some() {
            return self.left_ms(now_ms);
        }
        let calm = now_ms.saturating_sub(self.started_at_ms) >= CALM_MS;
        let delay = if calm || self.next_delay_ms == 0 {
            FIRST_DELAY_MS
        } else {
            self.next_delay_ms
        };
        self.next_delay_ms = delay.saturating_mul(2).min(MAX_DELAY_MS);
        self.due_at_ms = Some(now_ms.saturating_add(delay));
        delay
    }

    /// The module was restarted at `now_ms` by the policy (after `due`). Counts in `restarts()`
    /// when it ends a wait; the pause growth is kept. For uploads, `reset` and `swap` use
    /// [`Restart::started_by_hand`].
    pub fn started(&mut self, now_ms: u64) {
        if self.due_at_ms.take().is_some() {
            self.restarts = self.restarts.saturating_add(1);
        }
        self.started_at_ms = now_ms;
    }

    /// The module was started by hand at `now_ms` (upload of a fixed sheet, `reset`, `swap`):
    /// ends any wait and resets the pause growth, so the next failure waits the first pause again.
    /// Ending a wait counts in `restarts()` like `started`; a start while not waiting does not.
    pub fn started_by_hand(&mut self, now_ms: u64) {
        self.started(now_ms);
        self.next_delay_ms = 0;
    }

    /// Whether the module is stopped and waiting for a restart.
    pub fn waiting(&self) -> bool {
        self.due_at_ms.is_some()
    }

    /// Whether a waiting module is due at `now_ms`.
    pub fn due(&self, now_ms: u64) -> bool {
        self.due_at_ms.is_some_and(|due| now_ms >= due)
    }

    /// Milliseconds left until the restart (0 when not waiting or already due) — for the status line.
    pub fn left_ms(&self, now_ms: u64) -> u64 {
        self.due_at_ms.map_or(0, |due| due.saturating_sub(now_ms))
    }

    /// Restarts so far since the counter was last cleared (shown as `restarts=` in the status).
    pub fn restarts(&self) -> u32 {
        self.restarts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_idle() {
        let r = Restart::new();
        assert!(!r.waiting());
        assert!(!r.due(0));
        assert!(!r.due(u64::MAX));
        assert_eq!(r.left_ms(5), 0);
        assert_eq!(r.restarts(), 0);
    }

    #[test]
    fn pauses_double_up_to_the_cap() {
        let mut r = Restart::new();
        let mut now = 0;
        let mut got = Vec::new();
        for _ in 0..7 {
            let d = r.failed(now);
            got.push(d);
            now += d;
            r.started(now); // fails again at once
        }
        assert_eq!(got, [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
    }

    #[test]
    fn due_and_left() {
        let mut r = Restart::new();
        assert_eq!(r.failed(10_000), 1_000);
        assert!(r.waiting());
        assert!(!r.due(10_999));
        assert_eq!(r.left_ms(10_000), 1_000);
        assert_eq!(r.left_ms(10_400), 600);
        assert!(r.due(11_000));
        assert_eq!(r.left_ms(11_000), 0);
        assert!(r.due(50_000));
        assert_eq!(r.left_ms(50_000), 0);
    }

    #[test]
    fn calm_period_resets_growth() {
        let mut r = Restart::new();
        assert_eq!(r.failed(0), 1_000);
        r.started(1_000);
        assert_eq!(r.failed(1_000 + CALM_MS), 1_000);
        r.started(100_000);
        assert_eq!(r.failed(100_000 + CALM_MS - 1), 2_000);
    }

    #[test]
    fn failed_twice_keeps_due_time() {
        let mut r = Restart::new();
        assert_eq!(r.failed(0), 1_000);
        assert_eq!(r.failed(300), 700);
        assert!(r.due(1_000));
        r.started(1_000);
        // growth was not advanced by the second call
        assert_eq!(r.failed(1_000), 2_000);
    }

    #[test]
    fn by_hand_resets_growth() {
        let mut r = Restart::new();
        r.failed(0);
        r.started(1_000);
        assert_eq!(r.failed(1_000), 2_000);
        r.started_by_hand(1_500); // user uploaded a fixed sheet while waiting
        assert!(!r.waiting());
        assert_eq!(r.failed(1_600), 1_000);
    }

    #[test]
    fn restarts_counting() {
        let mut r = Restart::new();
        r.started_by_hand(0);
        r.started(10);
        assert_eq!(r.restarts(), 0); // not waiting: does not count
        r.failed(100);
        r.started(1_100);
        assert_eq!(r.restarts(), 1);
        r.failed(1_200);
        r.started_by_hand(1_300); // ends a wait: counts
        assert_eq!(r.restarts(), 2);
        r.started_by_hand(1_400);
        assert_eq!(r.restarts(), 2);
    }

    #[test]
    fn no_overflow_near_the_end_of_time() {
        let mut r = Restart::new();
        assert_eq!(r.failed(u64::MAX - 10), 1_000);
        assert!(!r.due(u64::MAX - 1));
        assert_eq!(r.left_ms(u64::MAX - 1), 1);
        assert!(r.due(u64::MAX));
        r.started(u64::MAX);
        r.failed(u64::MAX);
    }
}
