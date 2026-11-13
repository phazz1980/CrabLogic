//! WASM entry points: `init`, `step`, and the imports of the master. Built with
//! `--features oled` (library/oled) or `--features lcd` (library/lcd), see drivers\build.ps1.

use crate::driver::{Driver, Io};
use crab_ui_format::DisplayKind;

#[link(wasm_import_module = "crab")]
extern "C" {
    fn input_i(index: i32) -> i32;
    fn input_f(index: i32) -> f32;
    fn input_text(index: i32, ptr: *mut u8, cap: i32) -> i32;
    fn param_i(index: i32) -> i32;
    fn output_i(index: i32, value: i32);
    fn output_f(index: i32, value: f32);
    fn config(ptr: *mut u8, cap: i32) -> i32;
}

#[link(wasm_import_module = "bus")]
extern "C" {
    fn i2c_open(sda: i32, scl: i32, hz: i32) -> i32;
    fn i2c_write(addr: i32, ptr: *const u8, len: i32) -> i32;
}

// Parameter numbers follow block.toml: model, address, sda, scl.
const P_ADDRESS: i32 = 1;
const P_SDA: i32 = 2;
const P_SCL: i32 = 3;

/// Own input ports: `oled` has `screen`; `lcd` has `screen`, `backlight`. One output: `ok`.
const OWN_IN: usize = if cfg!(feature = "lcd") { 2 } else { 1 };
const OWN_OUT: usize = 1;
/// Largest config the master hands over (help/12-drivers.md: 4 KB per instance).
const CFG_CAP: usize = 4096;
/// Fast mode: a whole OLED page or LCD row goes out in about 2-3 ms.
const I2C_HZ: i32 = 400_000;

static mut CFG: [u8; CFG_CAP] = [0; CFG_CAP];
static mut DRV: Driver<'static> = Driver::new();
static mut ADDR: i32 = 0;

struct Master;

impl Io for Master {
    fn input_i(&self, i: usize) -> i32 {
        unsafe { input_i(i as i32) }
    }
    fn input_f(&self, i: usize) -> f32 {
        unsafe { input_f(i as i32) }
    }
    fn input_text(&self, i: usize, buf: &mut [u8]) -> usize {
        let n = unsafe { input_text(i as i32, buf.as_mut_ptr(), buf.len() as i32) };
        (n.max(0) as usize).min(buf.len())
    }
    fn output_i(&mut self, i: usize, v: i32) {
        unsafe { output_i(i as i32, v) }
    }
    fn output_f(&mut self, i: usize, v: f32) {
        unsafe { output_f(i as i32, v) }
    }
    fn write(&mut self, data: &[u8]) -> bool {
        unsafe { i2c_write(ADDR, data.as_ptr(), data.len() as i32) == data.len() as i32 }
    }
}

/// The block type must match the config (an `oled` module cannot drive an LCD).
fn matches_block(kind: DisplayKind) -> bool {
    let lcd = matches!(kind, DisplayKind::Lcd1602 | DisplayKind::Lcd2004);
    lcd == cfg!(feature = "lcd")
}

#[no_mangle]
pub extern "C" fn init() {
    unsafe {
        ADDR = param_i(P_ADDRESS);
        i2c_open(param_i(P_SDA), param_i(P_SCL), I2C_HZ);
        let cfg = core::ptr::addr_of_mut!(CFG).cast::<u8>();
        let n = config(cfg, CFG_CAP as i32);
        if n <= 0 || n as usize > CFG_CAP {
            return;
        }
        let blob: &'static [u8] = core::slice::from_raw_parts(cfg, n as usize);
        let drv = &mut *core::ptr::addr_of_mut!(DRV);
        if drv.init(blob, OWN_IN, OWN_OUT) && !matches_block(blob_kind(blob)) {
            // Wrong block type for this display: stay idle (`ok` stays false).
            drv.init(&[], OWN_IN, OWN_OUT);
        }
    }
}

fn blob_kind(blob: &[u8]) -> DisplayKind {
    blob.get(1)
        .and_then(|&b| DisplayKind::from_u8(b))
        .unwrap_or(DisplayKind::Ssd1306_128x64)
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    unsafe {
        (*core::ptr::addr_of_mut!(DRV)).step(&mut Master, dt_ms);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
