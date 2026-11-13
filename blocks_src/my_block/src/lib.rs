//! `my_block`: copies a bool input to a bool output (integers only).

#![cfg_attr(not(test), no_std)]

/// Output for the input value (0/1).
pub fn copy(input: i32) -> i32 {
    (input != 0) as i32
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn input_i(index: i32) -> i32;
        fn output_i(index: i32, value: i32);
    }

    const I_IN: i32 = 0;
    const O_OUT: i32 = 0;

    #[no_mangle]
    pub extern "C" fn step(_dt_ms: i32) {
        unsafe { output_i(O_OUT, super::copy(input_i(I_IN))) };
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }
}

#[cfg(test)]
mod tests {
    use super::copy;

    #[test]
    fn copies_as_bool() {
        assert_eq!(copy(0), 0);
        assert_eq!(copy(1), 1);
        assert_eq!(copy(5), 1);
    }
}
