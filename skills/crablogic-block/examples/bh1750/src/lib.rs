//! `bh1750`: I2C driver block for the BH1750 light sensor (reference example for driver blocks).
//! Never waits inside a step: the sensor measures continuously, the driver reads it every `period`.
//! Bus API and limits: skills/crablogic-block/reference/wasm-api.md; build flags: build.ps1.
//! The raw-to-lux formula is plain Rust tested on the PC (`cargo test`); the bus exchange is in `mod wasm`.

#![cfg_attr(not(test), no_std)]

/// Raw sensor value to lux in tenths, integer math: lux = raw / 1.2, so tenths = raw * 25 / 3.
fn lux_tenths(raw: u16) -> i32 {
    i32::from(raw) * 25 / 3
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::lux_tenths;

    // Interface v2 (no f64): `_i` for bool/int/duration/pin ports, `_f` (f32) for float ports.
    #[link(wasm_import_module = "crab")]
    extern "C" {
        fn param_i(index: i32) -> i32;
        fn output_i(index: i32, value: i32);
        fn output_f(index: i32, value: f32);
    }

    #[link(wasm_import_module = "bus")]
    extern "C" {
        fn i2c_open(sda: i32, scl: i32, hz: i32) -> i32;
        fn i2c_write(addr: i32, ptr: *const u8, len: i32) -> i32;
        fn i2c_read(addr: i32, ptr: *mut u8, len: i32) -> i32;
    }

    // Parameter and output numbers follow the order of block.toml.
    const P_ADDRESS: i32 = 0;
    const P_SDA: i32 = 1;
    const P_SCL: i32 = 2;
    const P_PERIOD: i32 = 3;
    const O_LUX: i32 = 0;
    const O_OK: i32 = 1;

    const CMD_POWER_ON: u8 = 0x01;
    /// Continuous high-resolution mode (1 lx, one measurement takes up to 180 ms).
    const CMD_CONT_HIRES: u8 = 0x10;
    const MEASURE_MS: i32 = 180;

    static mut STARTED: bool = false; // the sensor is switched on and measures
    static mut WAIT_MS: i32 = 0; // time left until the next action
    static mut BUF: [u8; 2] = [0; 2];

    fn addr() -> i32 {
        unsafe { param_i(P_ADDRESS) }
    }

    fn period() -> i32 {
        unsafe { param_i(P_PERIOD) }
    }

    fn command(cmd: u8) -> bool {
        let c = [cmd];
        unsafe { i2c_write(addr(), c.as_ptr(), 1) == 1 }
    }

    /// Powers the sensor on and starts continuous measuring.
    fn start() -> bool {
        command(CMD_POWER_ON) && command(CMD_CONT_HIRES)
    }

    #[no_mangle]
    pub extern "C" fn init() {
        unsafe {
            i2c_open(param_i(P_SDA), param_i(P_SCL), 100_000);
            STARTED = start();
            // The first result is ready after one measurement; no answer: retry in a period.
            WAIT_MS = if STARTED { MEASURE_MS } else { period() };
        }
    }

    #[no_mangle]
    pub extern "C" fn step(dt_ms: i32) {
        unsafe {
            WAIT_MS -= dt_ms;
            if WAIT_MS > 0 {
                return;
            }
            WAIT_MS = period();
            if !STARTED {
                // No chip yet, or the bus failed: try to start again, read on the next turn.
                STARTED = start();
                if STARTED {
                    WAIT_MS = MEASURE_MS;
                } else {
                    output_i(O_OK, 0);
                }
                return;
            }
            let buf = core::ptr::addr_of_mut!(BUF).cast::<u8>();
            if i2c_read(addr(), buf, 2) != 2 {
                STARTED = false; // start over (power on again) on the next turn
                output_i(O_OK, 0);
                return;
            }
            let raw = (u16::from(BUF[0]) << 8) | u16::from(BUF[1]); // big-endian
                                                                    // The float conversion happens only here, at the float output.
            output_f(O_LUX, lux_tenths(raw) as f32 / 10.0);
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
    use super::lux_tenths;

    #[test]
    fn raw_to_lux_in_tenths() {
        assert_eq!(lux_tenths(0), 0);
        assert_eq!(lux_tenths(120), 1000); // 120 / 1.2 = 100.0 lx
        assert_eq!(lux_tenths(65535), 546_125);
    }
}
