//! BMP280 driver block (`specs/wasm-drivers`): temperature and pressure over I²C through the
//! master's bus functions. ABI: help/12-drivers.md.
//!
//! Build with `..\build.ps1 rust` (the flags there keep the module small and without bulk memory).

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
    fn i2c_write_read(addr: i32, wptr: *const u8, wlen: i32, rptr: *mut u8, rlen: i32) -> i32;
}

// Parameter and output numbers, in the order of block.toml.
const P_ADDRESS: i32 = 0;
const P_SDA: i32 = 1;
const P_SCL: i32 = 2;
const P_PERIOD: i32 = 3;
const O_TEMPERATURE: i32 = 0;
const O_PRESSURE: i32 = 1;
const O_OK: i32 = 2;

const REG_CALIB: u8 = 0x88;
const REG_ID: u8 = 0xD0;
const REG_CTRL_MEAS: u8 = 0xF4;
const REG_DATA: u8 = 0xF7;
const CHIP_ID: u8 = 0x58;
/// Temperature and pressure oversampling x1, forced mode: one measurement, then sleep.
const FORCED_X1: u8 = 0b001_001_01;
/// A forced measurement with x1 oversampling takes up to 6.4 ms.
const MEASURE_MS: i32 = 10;

#[derive(Clone, Copy)]
struct Calib {
    t1: i32,
    t2: i32,
    t3: i32,
    p: [i64; 9],
}

static mut CALIB: Option<Calib> = None;
static mut WAIT_MS: i32 = 0;
static mut MEASURING: bool = false;
static mut BUF: [u8; 24] = [0; 24];

fn addr() -> i32 {
    unsafe { param(P_ADDRESS) as i32 }
}

/// Reads `n` bytes from register `reg` into `BUF`.
fn read(reg: u8, n: usize) -> bool {
    let r = [reg];
    unsafe {
        i2c_write_read(addr(), r.as_ptr(), 1, core::ptr::addr_of_mut!(BUF).cast(), n as i32)
            == n as i32
    }
}

fn write(reg: u8, value: u8) -> bool {
    let w = [reg, value];
    unsafe { i2c_write(addr(), w.as_ptr(), 2) == 2 }
}

fn le_u16(b: &[u8], at: usize) -> i32 {
    i32::from(b[at]) | i32::from(b[at + 1]) << 8
}

fn le_i16(b: &[u8], at: usize) -> i32 {
    i32::from(i16::from_le_bytes([b[at], b[at + 1]]))
}

/// The chip answers with its id; then the calibration words are read once.
fn connect() -> Option<Calib> {
    if !read(REG_ID, 1) || unsafe { BUF[0] } != CHIP_ID || !read(REG_CALIB, 24) {
        return None;
    }
    let b = unsafe { &*core::ptr::addr_of!(BUF) };
    let mut p = [0i64; 9];
    p[0] = le_u16(b, 6) as i64;
    for (k, v) in p.iter_mut().enumerate().skip(1) {
        *v = le_i16(b, 6 + 2 * k) as i64;
    }
    Some(Calib {
        t1: le_u16(b, 0),
        t2: le_i16(b, 2),
        t3: le_i16(b, 4),
        p,
    })
}

/// Compensation formulas of the BMP280 datasheet (integer versions): °C and Pa.
fn compensate(c: &Calib, adc_t: i32, adc_p: i32) -> (f64, f64) {
    let var1 = (((adc_t >> 3) - (c.t1 << 1)) * c.t2) >> 11;
    let d = (adc_t >> 4) - c.t1;
    let var2 = (((d * d) >> 12) * c.t3) >> 14;
    let t_fine = var1 + var2;
    let t = ((t_fine * 5 + 128) >> 8) as f64 / 100.0;

    let p = &c.p;
    let mut v1 = t_fine as i64 - 128_000;
    let mut v2 = v1 * v1 * p[5];
    v2 += (v1 * p[4]) << 17;
    v2 += p[3] << 35;
    v1 = ((v1 * v1 * p[2]) >> 8) + ((v1 * p[1]) << 12);
    v1 = (((1i64 << 47) + v1) * p[0]) >> 33;
    if v1 == 0 {
        return (t, 0.0);
    }
    let mut pr = 1_048_576 - adc_p as i64;
    pr = (((pr << 31) - v2) * 3125) / v1;
    let v1 = (p[8] * (pr >> 13) * (pr >> 13)) >> 25;
    let v2 = (p[7] * pr) >> 19;
    pr = ((pr + v1 + v2) >> 8) + (p[6] << 4);
    (t, pr as f64 / 256.0)
}

#[no_mangle]
pub extern "C" fn init() {
    unsafe {
        i2c_open(param(P_SDA) as i32, param(P_SCL) as i32, 400_000);
        CALIB = connect();
        WAIT_MS = 0;
        MEASURING = false;
    }
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    unsafe {
        WAIT_MS -= dt_ms;
        if WAIT_MS > 0 {
            return;
        }
        if (*core::ptr::addr_of!(CALIB)).is_none() {
            // Not answering yet (no chip, wrong address): try again a period later.
            CALIB = connect();
            if (*core::ptr::addr_of!(CALIB)).is_none() {
                output(O_OK, 0.0);
                WAIT_MS = param(P_PERIOD) as i32;
                return;
            }
        }
        if !MEASURING {
            // Start a measurement and come back when it is done: no waiting inside a step.
            MEASURING = write(REG_CTRL_MEAS, FORCED_X1);
            WAIT_MS = if MEASURING { MEASURE_MS } else { param(P_PERIOD) as i32 };
            if !MEASURING {
                output(O_OK, 0.0);
            }
            return;
        }
        MEASURING = false;
        WAIT_MS = param(P_PERIOD) as i32 - MEASURE_MS;
        if !read(REG_DATA, 6) {
            output(O_OK, 0.0);
            return;
        }
        let b = &*core::ptr::addr_of!(BUF);
        let raw = |at: usize| {
            (i32::from(b[at]) << 12) | (i32::from(b[at + 1]) << 4) | (i32::from(b[at + 2]) >> 4)
        };
        let (t, p) = compensate((*core::ptr::addr_of!(CALIB)).as_ref().unwrap_unchecked(), raw(3), raw(0));
        output(O_TEMPERATURE, t);
        output(O_PRESSURE, p / 100.0);
        output(O_OK, 1.0);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
