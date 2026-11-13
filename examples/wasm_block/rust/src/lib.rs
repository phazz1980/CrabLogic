//! Simulation module of the `lowpass` block in Rust: exponential smoothing.
//! ABI: see help/10-block-editor.md ("Модуль симуляции").

#![no_std]

#[link(wasm_import_module = "crab")]
extern "C" {
    fn input_f(index: i32) -> f32;
    fn param_f(index: i32) -> f32;
    fn output_f(index: i32, value: f32);
}

static mut Y: f32 = 0.0;

#[no_mangle]
pub extern "C" fn init() {
    unsafe { Y = 0.0 };
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    unsafe {
        let tau = param_f(0);
        let dt = dt_ms as f32;
        Y += dt / (tau + dt) * (input_f(0) - Y);
        output_f(0, Y);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
