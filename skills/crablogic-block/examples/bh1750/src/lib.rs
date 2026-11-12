//! `bh1750`: I2C driver block for the BH1750 light sensor (reference example for driver blocks).
//! Never waits inside a step: the sensor measures continuously, the driver reads it every `period`.
//! Bus API and limits: skills/crablogic-block/reference/wasm-api.md; build flags: build.ps1.
//! Only the bus exchange is here; for logic worth testing on the PC see the window_check layout.

#![no_std]

#[link(wasm_import_module = "crab")]
extern "C" {
    fn param(index: i32) -> f64;
    fn output(index: i32, value: f64);
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
    unsafe { param(P_ADDRESS) as i32 }
}

fn period() -> i32 {
    unsafe { param(P_PERIOD) as i32 }
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
        i2c_open(param(P_SDA) as i32, param(P_SCL) as i32, 100_000);
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
                output(O_OK, 0.0);
            }
            return;
        }
        let buf = core::ptr::addr_of_mut!(BUF).cast::<u8>();
        if i2c_read(addr(), buf, 2) != 2 {
            STARTED = false; // start over (power on again) on the next turn
            output(O_OK, 0.0);
            return;
        }
        let raw = (u16::from(BUF[0]) << 8) | u16::from(BUF[1]); // big-endian
        output(O_LUX, f64::from(raw) / 1.2);
        output(O_OK, 1.0);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
