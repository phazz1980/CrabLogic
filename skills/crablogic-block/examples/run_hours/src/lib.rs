//! `run_hours`: block with memory (reference example for user blocks).
//! Counts running hours while `run` is true and the number of starts (rising edges of `run`).
//!
//! Layout to copy: the behaviour is plain Rust (`Counter`) tested on the PC with `cargo test`;
//! the WASM glue (`mod wasm`: imports, `init`, `step`) is compiled only for wasm32.
//! ABI: skills/crablogic-block/reference/wasm-api.md, flags: build.ps1.

#![cfg_attr(not(test), no_std)]

const MS_PER_HOUR: f32 = 3_600_000.0;

/// State of one block instance.
pub struct Counter {
    run_ms: u64,    // accumulated running time, ms (integer: no rounding drift)
    starts: u32,    // rising edges of `run`
    prev_run: bool, // `run` of the previous step, for edge detection
}

impl Counter {
    pub const fn new() -> Self {
        Self {
            run_ms: 0,
            starts: 0,
            prev_run: false,
        }
    }

    /// One cycle; returns (hours, starts). Time is summed in integer ms; the float appears
    /// only in the result, because the `hours` port is float.
    pub fn step(&mut self, run: bool, reset: bool, dt_ms: i32) -> (f32, u32) {
        if reset {
            // Reset wins over counting; prev_run = false, so a machine that still runs
            // after the reset is released counts as a new start.
            *self = Self::new();
        } else {
            if run {
                self.run_ms += dt_ms.max(0) as u64;
                if !self.prev_run {
                    self.starts += 1;
                }
            }
            self.prev_run = run;
        }
        (self.run_ms as f32 / MS_PER_HOUR, self.starts)
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::Counter;

    #[link(wasm_import_module = "crab")]
    extern "C" {
        // Interface v2 (no f64): `_i` for bool/int, `_f` (f32) for float ports.
        fn input_i(index: i32) -> i32;
        fn param_f(index: i32) -> f32;
        fn output_i(index: i32, value: i32);
        fn output_f(index: i32, value: f32);
    }

    // Numbers follow the order of block.toml.
    const I_RUN: i32 = 0;
    const I_RESET: i32 = 1;
    const P_LIMIT: i32 = 0;
    const O_HOURS: i32 = 0;
    const O_STARTS: i32 = 1;
    const O_SERVICE: i32 = 2;

    // Every block instance has its own copy of the module memory, so a static is its state.
    static mut STATE: Counter = Counter::new();

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe { *core::ptr::addr_of_mut!(STATE) = Counter::new() };
    }

    #[no_mangle]
    pub extern "C" fn step(dt_ms: i32) {
        unsafe {
            let state = &mut *core::ptr::addr_of_mut!(STATE);
            let (hours, starts) = state.step(input_i(I_RUN) != 0, input_i(I_RESET) != 0, dt_ms);
            output_f(O_HOURS, hours);
            output_i(O_STARTS, starts as i32);
            output_i(O_SERVICE, (hours >= param_f(P_LIMIT)) as i32);
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

    #[test]
    fn counts_time_and_starts() {
        let mut c = Counter::new();
        c.step(true, false, 1800_000); // half an hour running
        c.step(false, false, 1000);
        let (hours, starts) = c.step(true, false, 1800_000);
        assert_eq!(hours, 1.0);
        assert_eq!(starts, 2);
    }

    #[test]
    fn reset_clears_and_a_running_machine_starts_again() {
        let mut c = Counter::new();
        c.step(true, false, 1000);
        assert_eq!(c.step(true, true, 1000), (0.0, 0));
        assert_eq!(c.step(true, false, 0).1, 1);
    }
}
