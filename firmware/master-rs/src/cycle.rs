//! Timing of the scan (`specs/fast-cycle`): the step time `dt_ms` of the sheets from microsecond
//! clock readings, overruns and late starts. Plain arithmetic, so it is tested on the PC; the
//! board wakes the scan with a periodic timer and passes `esp_timer_get_time`.

/// Late start of a step beyond which it is counted in [`Cycle::late50`].
pub const LATE_SMALL_US: u32 = 50;
/// Late start of a step beyond which it is counted in [`Cycle::late100`].
pub const LATE_BIG_US: u32 = 100;

/// Timing of the scan of all sheets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    period_us: u32,
    /// Start of the last step.
    last_start: Option<u64>,
    /// Microseconds not yet given to the sheets as whole milliseconds.
    carry_us: u32,
    /// Steps since the statistics were reset.
    pub steps: u32,
    /// Steps that ended after the next one was due.
    pub overruns: u32,
    /// Starts more than [`LATE_SMALL_US`] / [`LATE_BIG_US`] after the period.
    pub late50: u32,
    pub late100: u32,
    /// Longest pause between the starts of two steps.
    pub gap_max_us: u32,
    /// Longest work of the master between two steps (commands, upload, notes).
    pub between_max_us: u32,
}

impl Cycle {
    pub fn new(period_ms: u32) -> Self {
        Self {
            period_us: period_ms.max(1) * 1000,
            last_start: None,
            carry_us: 0,
            steps: 0,
            overruns: 0,
            late50: 0,
            late100: 0,
            gap_max_us: 0,
            between_max_us: 0,
        }
    }

    pub fn period_ms(&self) -> u32 {
        self.period_us / 1000
    }

    pub fn period_us(&self) -> u32 {
        self.period_us
    }

    /// A new period from the next step on; the time already counted is kept.
    pub fn set_period(&mut self, period_ms: u32) {
        self.period_us = period_ms.max(1) * 1000;
    }

    /// A step starts at `now_us`: the milliseconds the sheets get. The first step gets the
    /// period; then the real time since the last start, the rest of a millisecond carried over,
    /// so the timers of the scheme stay exact whatever the jitter.
    pub fn begin(&mut self, now_us: u64) -> u32 {
        let Some(last) = self.last_start.replace(now_us) else {
            self.steps = self.steps.wrapping_add(1);
            return self.period_ms();
        };
        let gap = now_us.saturating_sub(last).min(u32::MAX as u64) as u32;
        self.steps = self.steps.wrapping_add(1);
        self.gap_max_us = self.gap_max_us.max(gap);
        let late = gap.saturating_sub(self.period_us);
        if late > LATE_SMALL_US {
            self.late50 = self.late50.wrapping_add(1);
        }
        if late > LATE_BIG_US {
            self.late100 = self.late100.wrapping_add(1);
        }
        let total = self.carry_us as u64 + gap as u64;
        self.carry_us = (total % 1000) as u32;
        (total / 1000).min(u32::MAX as u64) as u32
    }

    /// The step that began at the last [`Cycle::begin`] ended at `now_us`. True when it took
    /// longer than the period: as in a PLC, the next step waits for the next mark of the timer
    /// grid, the periods missed are not made up (the sheets get the real time in `dt_ms`).
    pub fn end(&mut self, now_us: u64) -> bool {
        let Some(start) = self.last_start else {
            return false;
        };
        let over = now_us.saturating_sub(start) > self.period_us as u64;
        if over {
            self.overruns = self.overruns.wrapping_add(1);
        }
        over
    }

    /// Clears the statistics (`resetmax`); the timing goes on.
    pub fn reset_stats(&mut self) {
        self.steps = 0;
        self.overruns = 0;
        self.late50 = 0;
        self.late100 = 0;
        self.gap_max_us = 0;
        self.between_max_us = 0;
    }

    /// The work between two steps took `us`.
    pub fn between(&mut self, us: u32) {
        self.between_max_us = self.between_max_us.max(us);
    }
}

/// The period the master runs at: the shortest any running sheet asks for, 10 ms without sheets.
pub fn period_of(asked: impl IntoIterator<Item = u32>) -> u32 {
    asked.into_iter().min().unwrap_or(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_one_ms_gives_one_ms_each_step() {
        let mut c = Cycle::new(1);
        assert_eq!(c.begin(5_000), 1);
        for k in 1..=100u64 {
            assert_eq!(c.begin(5_000 + k * 1000), 1);
            assert!(!c.end(5_000 + k * 1000 + 300));
        }
        assert_eq!((c.overruns, c.late50, c.late100), (0, 0, 0));
        assert_eq!(c.gap_max_us, 1000);
        assert_eq!(c.steps, 101);
    }

    #[test]
    fn jitter_is_carried_so_the_sum_is_the_real_time() {
        let mut c = Cycle::new(1);
        c.begin(0);
        let starts = [950u64, 2_080, 2_990, 4_010, 5_000, 6_400, 6_900, 8_000];
        let total: u32 = starts.iter().map(|&t| c.begin(t)).sum();
        assert_eq!(total, 8);
        // gaps 1130, 1400 and 1100 µs; 1100 is exactly 100 late
        assert_eq!(c.late50, 3);
        assert_eq!(c.late100, 2);
        assert_eq!(c.gap_max_us, 1_400);
    }

    #[test]
    fn overrun_is_counted_and_the_long_gap_is_given_to_the_sheets() {
        let mut c = Cycle::new(1);
        c.begin(0);
        assert!(c.end(2_500));
        // The marks at 1 and 2 ms are dropped, the next step starts on the 3 ms mark and gets
        // all the time; no burst of steps to catch up.
        assert_eq!(c.begin(3_000), 3);
        assert!(!c.end(3_200));
        assert_eq!(c.begin(4_000), 1);
        assert_eq!(c.overruns, 1);
    }

    #[test]
    fn ten_ms_period_matches_the_old_quantum() {
        let mut c = Cycle::new(10);
        assert_eq!(c.begin(0), 10);
        assert_eq!(c.begin(10_000), 10);
        assert_eq!(c.begin(20_040), 10);
        assert_eq!(c.late50, 0);
        assert_eq!(c.begin(30_100), 10);
        assert_eq!(c.late50, 1);
        assert_eq!(c.late100, 0);
    }

    #[test]
    fn reset_keeps_the_timing() {
        let mut c = Cycle::new(1);
        c.begin(0);
        c.begin(1_600);
        c.reset_stats();
        assert_eq!((c.steps, c.late50, c.gap_max_us), (0, 0, 0));
        // 600 µs carried from before the reset
        assert_eq!(c.begin(2_000), 1);
    }

    #[test]
    fn master_period_is_the_shortest() {
        assert_eq!(period_of([10, 2, 5]), 2);
        assert_eq!(period_of([]), 10);
    }
}
