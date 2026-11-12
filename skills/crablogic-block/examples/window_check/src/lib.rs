//! `window_check`: pure logic block without memory (reference example for user blocks).
//! Compares `value` with the window [low, high] and sets three outputs.
//!
//! Layout to copy: the behaviour is a plain function (`classify`) tested on the PC with
//! `cargo test`; the WASM glue (`mod wasm`) is compiled only for wasm32.
//! ABI: skills/crablogic-block/reference/wasm-api.md, build flags: build.ps1.

#![cfg_attr(not(test), no_std)]

/// (inside, above, below); the edges belong to the window.
pub fn classify(value: f64, low: f64, high: f64) -> (bool, bool, bool) {
    let above = value > high;
    let below = value < low;
    (!above && !below, above, below)
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    // Imports of the master: numbers only (bool is 0.0 / 1.0).
    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn input(index: i32) -> f64;
        fn param(index: i32) -> f64;
        fn output(index: i32, value: f64);
    }

    // Input, parameter and output numbers follow the order of block.toml.
    const I_VALUE: i32 = 0;
    const P_LOW: i32 = 0;
    const P_HIGH: i32 = 1;
    const O_INSIDE: i32 = 0;
    const O_ABOVE: i32 = 1;
    const O_BELOW: i32 = 2;

    fn flag(b: bool) -> f64 {
        if b {
            1.0
        } else {
            0.0
        }
    }

    /// Called every cycle of the sheet; `dt_ms` is not needed here.
    #[no_mangle]
    pub extern "C" fn step(_dt_ms: i32) {
        unsafe {
            let (inside, above, below) =
                super::classify(input(I_VALUE), param(P_LOW), param(P_HIGH));
            output(O_INSIDE, flag(inside));
            output(O_ABOVE, flag(above));
            output(O_BELOW, flag(below));
        }
    }

    // no_std needs a panic handler; with panic = "abort" it is never reached in practice.
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn window_with_edges() {
        assert_eq!(classify(15.0, 10.0, 20.0), (true, false, false));
        assert_eq!(classify(10.0, 10.0, 20.0), (true, false, false));
        assert_eq!(classify(20.5, 10.0, 20.0), (false, true, false));
        assert_eq!(classify(9.0, 10.0, 20.0), (false, false, true));
    }
}
