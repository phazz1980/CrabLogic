//! Fast counter driver block: counts the pulses of a pin with a counter of the master (hardware
//! PCNT on the ESP32), 32 bit wrapping, optional direction pin (`specs/pcnt`).
//! Build: `drivers\build.ps1 fast_counter`.

#![cfg_attr(not(test), no_std)]

/// The counter is opened again this often (ms) while it is not open.
const RETRY_MS: i32 = 1000;

/// Counter number, or a negative error while it is not open; time since the last try.
struct State {
    n: i32,
    retry_ms: i32,
}

impl State {
    const fn new() -> Self {
        State { n: -1, retry_ms: 0 }
    }

    /// True when the step has to try to open the counter now.
    fn retry_due(&mut self, dt_ms: i32) -> bool {
        self.retry_ms = self.retry_ms.saturating_add(dt_ms);
        if self.retry_ms >= RETRY_MS {
            self.retry_ms = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod glue {
    use super::State;

    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn param_i(index: i32) -> i32;
        fn input_i(index: i32) -> i32;
        fn output_i(index: i32, value: i32);
    }

    #[link(wasm_import_module = "bus")]
    extern "C" {
        fn count_open(pin_a: i32, pin_b: i32, mode: i32, filter_us: i32) -> i32;
        fn count_read(n: i32) -> i32;
        fn count_clear(n: i32) -> i32;
    }

    const P_PIN: i32 = 0;
    const P_EDGE: i32 = 1;
    const P_DIRECTION: i32 = 2;
    const P_DIR_PIN: i32 = 3;
    const P_FILTER: i32 = 4;
    const I_RESET: i32 = 0;
    const O_COUNT: i32 = 0;
    const O_OK: i32 = 1;

    static mut STATE: State = State::new();

    unsafe fn open() -> i32 {
        let pin_b = if param_i(P_DIRECTION) != 0 {
            param_i(P_DIR_PIN)
        } else {
            -1
        };
        count_open(param_i(P_PIN), pin_b, param_i(P_EDGE), param_i(P_FILTER))
    }

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe {
            let s = &mut *core::ptr::addr_of_mut!(STATE);
            *s = State::new();
            s.n = open();
        }
    }

    #[no_mangle]
    pub extern "C" fn step(dt_ms: i32) {
        unsafe {
            let s = &mut *core::ptr::addr_of_mut!(STATE);
            if s.n < 0 {
                if s.retry_due(dt_ms) {
                    s.n = open();
                }
                if s.n < 0 {
                    output_i(O_OK, 0);
                    return;
                }
            }
            if input_i(I_RESET) != 0 {
                count_clear(s.n);
            }
            output_i(O_COUNT, count_read(s.n));
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

    #[test]
    fn retries_once_a_second() {
        let mut s = State::new();
        assert!(!s.retry_due(400));
        assert!(!s.retry_due(500));
        assert!(s.retry_due(100));
        assert!(!s.retry_due(10));
    }
}
