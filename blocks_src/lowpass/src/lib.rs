//! `lowpass`: exponential smoothing, y += dt / (tau + dt) * (x - y).
//! The filter is the one place where f32 is justified: the ports are `float`.

#![cfg_attr(not(test), no_std)]

/// State of one block instance.
pub struct Lowpass {
    y: f32,
}

impl Lowpass {
    pub const fn new() -> Self {
        Self { y: 0.0 }
    }

    /// One cycle: `dt_ms` since the previous step, `tau_ms` time constant; returns the output.
    pub fn step(&mut self, x: f32, tau_ms: f32, dt_ms: i32) -> f32 {
        let dt = dt_ms.max(0) as f32;
        let sum = tau_ms.max(0.0) + dt;
        if sum > 0.0 {
            self.y += dt / sum * (x - self.y);
        }
        self.y
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::Lowpass;

    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn input_f(index: i32) -> f32;
        fn param_f(index: i32) -> f32;
        fn output_f(index: i32, value: f32);
    }

    const I_IN: i32 = 0;
    const P_TAU: i32 = 0;
    const O_OUT: i32 = 0;

    static mut STATE: Lowpass = Lowpass::new();

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe { *core::ptr::addr_of_mut!(STATE) = Lowpass::new() };
    }

    #[no_mangle]
    pub extern "C" fn step(dt_ms: i32) {
        unsafe {
            let state = &mut *core::ptr::addr_of_mut!(STATE);
            output_f(O_OUT, state.step(input_f(I_IN), param_f(P_TAU), dt_ms));
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }
}

#[cfg(test)]
mod tests {
    use super::Lowpass;

    #[test]
    fn moves_towards_input_by_k() {
        let mut f = Lowpass::new();
        // k = 100 / (900 + 100) = 0.1
        assert!((f.step(10.0, 900.0, 100) - 1.0).abs() < 1e-6);
        assert!((f.step(10.0, 900.0, 100) - 1.9).abs() < 1e-5);
    }

    #[test]
    fn converges_and_zero_dt_holds() {
        let mut f = Lowpass::new();
        assert_eq!(f.step(5.0, 1000.0, 0), 0.0);
        for _ in 0..2000 {
            f.step(5.0, 100.0, 100);
        }
        assert!((f.step(5.0, 100.0, 100) - 5.0).abs() < 1e-3);
    }
}
