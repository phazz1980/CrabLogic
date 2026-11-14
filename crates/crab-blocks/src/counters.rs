//! Pulse counters on the bench: the user sets a pulse rate (Hz) on the counter's pin and the
//! block counts `rate * dt`, as the hardware counter on the board would. The rate is kept in
//! the bench pin level; a pin used by a counter is used by nobody else (the check forbids it).

use crate::{b, Behavior, Ctx};

/// Counts of `rate` events per second over `dt_ms`, with the fraction carried to the next scan.
#[derive(Default)]
struct Accumulator {
    count: i32,
    frac: f64,
    last_ms: u64,
}

impl Accumulator {
    /// Time since the previous scan in seconds.
    fn tick(&mut self, now_ms: u64) -> f64 {
        let dt = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        dt as f64 / 1000.0
    }

    fn add(&mut self, events_per_s: f64, dt_s: f64) {
        let total = events_per_s * dt_s + self.frac;
        let whole = total.trunc();
        self.frac = total - whole;
        // The count is 32 bits and wraps, like the counter on the board.
        self.count = self.count.wrapping_add(whole as i64 as i32);
    }

    fn clear(&mut self) {
        self.count = 0;
        self.frac = 0.0;
    }
}

/// Counted edges per pulse for the `edge` parameter: both edges count twice.
fn edge_factor(edge: f64) -> f64 {
    if edge == 2.0 {
        2.0
    } else {
        1.0
    }
}

/// Fast counter. Input: reset. Params: pin, edge, direction, dir_pin, filter.
/// The rate is the pulses per second; with `direction` its sign is the direction
/// (negative = counting down), the real dir pin is not read on the bench.
#[derive(Default)]
pub struct FastCounter(Accumulator);

impl Behavior for FastCounter {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let dt = self.0.tick(c.now_ms);
        let rate = c.bench.level(c.param(0) as i64).unwrap_or(0.0);
        let rate = if c.param(2) != 0.0 { rate } else { rate.abs() };
        if c.flag(0) {
            self.0.clear();
        } else {
            self.0.add(rate * edge_factor(c.param(1)), dt);
        }
        out[0] = f64::from(self.0.count);
        out[1] = b(true);
    }
}

/// Frequency meter. Params: pin, edge, window, filter. `hz` is the rate of the pin,
/// `count` counts the edges up.
#[derive(Default)]
pub struct Frequency(Accumulator);

impl Behavior for Frequency {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let dt = self.0.tick(c.now_ms);
        let rate = c.bench.level(c.param(0) as i64).unwrap_or(0.0).abs();
        self.0.add(rate * edge_factor(c.param(1)), dt);
        out[0] = rate;
        out[1] = f64::from(self.0.count);
        out[2] = b(true);
    }
}

/// Quadrature encoder. Input: reset. Params: pin_a, pin_b, mode, filter. The rate set on
/// pin A is in steps per second already (so the mode factor is not applied); its sign is
/// the direction.
#[derive(Default)]
pub struct QuadEncoder(Accumulator);

impl Behavior for QuadEncoder {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let dt = self.0.tick(c.now_ms);
        let rate = c.bench.level(c.param(0) as i64).unwrap_or(0.0);
        if c.flag(0) {
            self.0.clear();
        } else {
            self.0.add(rate, dt);
        }
        out[0] = f64::from(self.0.count);
        out[1] = if rate > 0.0 {
            1.0
        } else if rate < 0.0 {
            -1.0
        } else {
            0.0
        };
        out[2] = b(true);
    }
}
