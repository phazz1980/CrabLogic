//! Frequency driver block: measures the frequency of the pulses of a pin. The master counts the
//! edges and times them (below ~1 kHz); the driver turns that into hertz once per window
//! (`specs/pcnt`). Integers on the board, `f32` only at the end and for the output port.
//! Build: `drivers\build.ps1 frequency`.

#![cfg_attr(not(test), no_std)]

#[cfg(target_arch = "wasm32")]
const RETRY_MS: i32 = 1000;

/// What `COUNT_PERIOD` reported for a window: `n` timed edges whose intervals sum to `sum_us`.
#[derive(Clone, Copy)]
struct Period {
    n: i32,
    sum_us: i32,
}

struct Freq {
    /// Time in the current window, ms.
    elapsed_ms: u32,
    /// Time since the last window that saw an edge, ms.
    idle_ms: u32,
    /// The last measured period, ms (at least 1).
    last_period_ms: u32,
    hz: f32,
}

impl Freq {
    const fn new() -> Self {
        Freq {
            elapsed_ms: 0,
            idle_ms: 0,
            last_period_ms: 0,
            hz: 0.0,
        }
    }

    /// Advances the window by `dt_ms`. Returns true when the window ended: the caller then reads
    /// the count delta and the period and calls [`Freq::finish`].
    fn tick(&mut self, dt_ms: u32, window_ms: u32) -> bool {
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
        self.elapsed_ms >= window_ms
    }

    /// Closes the window: `dc` edges were counted (wrapping delta), `period` is the timing of
    /// those that could be timed. The frequency comes from the period (exact at low frequency)
    /// or from the count over the window.
    fn finish(&mut self, window_ms: u32, dc: u32, period: Option<Period>) {
        let elapsed = self.elapsed_ms.max(1);
        self.elapsed_ms = 0;
        match period {
            Some(p) if p.n > 0 && p.sum_us > 0 => {
                let (n, t) = (p.n as u32, p.sum_us as u32);
                self.hz = n as f32 * 1_000_000.0 / t as f32;
                self.last_period_ms = (t / n / 1000).max(1);
                self.idle_ms = 0;
            }
            _ if dc > 0 => {
                self.hz = dc as f32 * 1000.0 / elapsed as f32;
                self.last_period_ms = (elapsed / dc).max(1);
                self.idle_ms = 0;
            }
            _ => {
                self.idle_ms = self.idle_ms.saturating_add(elapsed);
                if self.idle_ms > window_ms.max(self.last_period_ms.saturating_mul(2)) {
                    self.hz = 0.0;
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod glue {
    use super::{Freq, Period, RETRY_MS};

    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn param_i(index: i32) -> i32;
        fn output_i(index: i32, value: i32);
        fn output_f(index: i32, value: f32);
    }

    #[link(wasm_import_module = "bus")]
    extern "C" {
        fn count_open(pin_a: i32, pin_b: i32, mode: i32, filter_us: i32) -> i32;
        fn count_read(n: i32) -> i32;
        fn count_period(n: i32, ptr: *mut i32) -> i32;
    }

    const P_PIN: i32 = 0;
    const P_EDGE: i32 = 1;
    const P_WINDOW: i32 = 2;
    const P_FILTER: i32 = 3;
    const O_HZ: i32 = 0;
    const O_COUNT: i32 = 1;
    const O_OK: i32 = 2;
    /// Keep the edge times for `count_period`.
    const PERIOD_FLAG: i32 = 0x100;

    static mut N: i32 = -1;
    static mut RETRY_LEFT: i32 = 0;
    static mut LAST: i32 = 0;
    static mut SUM_US: i32 = 0;
    static mut FREQ: Freq = Freq::new();

    unsafe fn open() -> i32 {
        let n = count_open(
            param_i(P_PIN),
            -1,
            param_i(P_EDGE) | PERIOD_FLAG,
            param_i(P_FILTER),
        );
        if n >= 0 {
            LAST = count_read(n);
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe {
            FREQ = Freq::new();
            RETRY_LEFT = 0;
            N = open();
        }
    }

    #[no_mangle]
    pub extern "C" fn step(dt_ms: i32) {
        unsafe {
            if N < 0 {
                RETRY_LEFT = RETRY_LEFT.saturating_add(dt_ms);
                if RETRY_LEFT >= RETRY_MS {
                    RETRY_LEFT = 0;
                    N = open();
                }
                if N < 0 {
                    output_i(O_OK, 0);
                    return;
                }
            }
            let freq = &mut *core::ptr::addr_of_mut!(FREQ);
            let window = param_i(P_WINDOW).max(1) as u32;
            let count = count_read(N);
            if freq.tick(dt_ms.max(0) as u32, window) {
                let dc = count.wrapping_sub(LAST) as u32;
                LAST = count;
                let n = count_period(N, core::ptr::addr_of_mut!(SUM_US));
                let period = Period { n, sum_us: SUM_US };
                freq.finish(window, dc, Some(period));
            }
            output_f(O_HZ, freq.hz);
            output_i(O_COUNT, count);
            output_i(O_OK, 1);
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(f: &mut Freq, window: u32, dc: u32, p: Option<Period>) {
        assert!(f.tick(window, window));
        f.finish(window, dc, p);
    }

    #[test]
    fn period_beats_count() {
        let mut f = Freq::new();
        // 3 timed edges of 10 ms each: 100 Hz, whatever the count says.
        let p = Period {
            n: 3,
            sum_us: 30_000,
        };
        close(&mut f, 1000, 3, Some(p));
        assert!((f.hz - 100.0).abs() < 0.01);
    }

    #[test]
    fn count_over_window_when_edges_are_fast() {
        let mut f = Freq::new();
        close(&mut f, 1000, 20_000, Some(Period { n: 0, sum_us: 0 }));
        assert!((f.hz - 20_000.0).abs() < 0.01);
        close(&mut f, 500, 10_000, None);
        assert!((f.hz - 20_000.0).abs() < 0.01);
    }

    #[test]
    fn count_delta_wraps() {
        let mut f = Freq::new();
        // The counter wrapped from i32::MAX to i32::MIN + 4: 5 edges.
        let dc = i32::MIN.wrapping_add(4).wrapping_sub(i32::MAX) as u32;
        assert_eq!(dc, 5);
        close(&mut f, 1000, dc, None);
        assert!((f.hz - 5.0).abs() < 0.01);
    }

    #[test]
    fn zero_after_two_periods_without_edges() {
        let mut f = Freq::new();
        // 0.5 Hz: one period of 2 s, window 100 ms.
        let p = Period {
            n: 1,
            sum_us: 2_000_000,
        };
        close(&mut f, 100, 1, Some(p));
        assert!((f.hz - 0.5).abs() < 1e-3);
        // Silent windows hold the value until 2 periods (4 s) have passed.
        for _ in 0..40 {
            close(&mut f, 100, 0, None);
        }
        assert!(f.hz > 0.0);
        close(&mut f, 100, 0, None);
        assert_eq!(f.hz, 0.0);
    }

    #[test]
    fn window_waits_for_its_length() {
        let mut f = Freq::new();
        assert!(!f.tick(400, 1000));
        assert!(!f.tick(500, 1000));
        assert!(f.tick(100, 1000));
    }
}
