//! Quadrature encoder driver block: position and direction from the A/B channels, counted by a
//! counter of the master (hardware PCNT on the ESP32), 32 bit wrapping (`specs/pcnt`).
//! Build: `drivers\build.ps1 quad_encoder`.

#![cfg_attr(not(test), no_std)]

#[cfg(target_arch = "wasm32")]
const RETRY_MS: i32 = 1000;
/// No step for this long (ms) means the knob stands still.
const HOLD_MS: u32 = 200;

/// Direction from the position changes: the sign of the last change, 0 after `HOLD_MS` of rest.
struct Dir {
    last: i32,
    since_ms: u32,
    dir: i32,
}

impl Dir {
    const fn new() -> Self {
        Dir {
            last: 0,
            since_ms: HOLD_MS,
            dir: 0,
        }
    }

    fn update(&mut self, pos: i32, dt_ms: u32) -> i32 {
        let delta = pos.wrapping_sub(self.last);
        self.last = pos;
        if delta != 0 {
            self.dir = if delta > 0 { 1 } else { -1 };
            self.since_ms = 0;
        } else {
            self.since_ms = self.since_ms.saturating_add(dt_ms);
            if self.since_ms >= HOLD_MS {
                self.dir = 0;
            }
        }
        self.dir
    }

    /// The position was cleared: no direction.
    #[cfg(any(test, target_arch = "wasm32"))]
    fn reset(&mut self) {
        *self = Dir::new();
    }
}

#[cfg(target_arch = "wasm32")]
mod glue {
    use super::{Dir, RETRY_MS};

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

    const P_PIN_A: i32 = 0;
    const P_PIN_B: i32 = 1;
    const P_MODE: i32 = 2;
    const P_FILTER: i32 = 3;
    const I_RESET: i32 = 0;
    const O_POS: i32 = 0;
    const O_DIR: i32 = 1;
    const O_OK: i32 = 2;
    /// `COUNT_QUAD_X1`: the parameter 0/1/2 is x1/x2/x4.
    const QUAD_X1: i32 = 3;

    static mut N: i32 = -1;
    static mut RETRY_LEFT: i32 = 0;
    static mut DIR: Dir = Dir::new();

    unsafe fn open() -> i32 {
        count_open(
            param_i(P_PIN_A),
            param_i(P_PIN_B),
            QUAD_X1 + param_i(P_MODE).clamp(0, 2),
            param_i(P_FILTER),
        )
    }

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe {
            DIR = Dir::new();
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
            let dir = &mut *core::ptr::addr_of_mut!(DIR);
            if input_i(I_RESET) != 0 {
                count_clear(N);
                dir.reset();
                output_i(O_POS, 0);
                output_i(O_DIR, 0);
            } else {
                let pos = count_read(N);
                output_i(O_POS, pos);
                output_i(O_DIR, dir.update(pos, dt_ms.max(0) as u32));
            }
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
    fn direction_follows_the_last_change_and_rests() {
        let mut d = Dir::new();
        assert_eq!(d.update(0, 10), 0);
        assert_eq!(d.update(1, 10), 1);
        assert_eq!(d.update(1, 100), 1);
        assert_eq!(d.update(1, 100), 0);
        assert_eq!(d.update(-3, 10), -1);
    }

    #[test]
    fn wrap_is_a_step_forward() {
        let mut d = Dir::new();
        d.update(i32::MAX, 10);
        assert_eq!(d.update(i32::MIN, 10), 1);
    }

    #[test]
    fn reset_clears_direction() {
        let mut d = Dir::new();
        d.update(5, 10);
        d.reset();
        assert_eq!(d.update(0, 10), 0);
    }
}
