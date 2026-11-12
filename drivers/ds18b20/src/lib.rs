//! DS18B20 driver block: temperature over 1-Wire through the master's bus functions
//! (`help/12-drivers.md`). Several sensors may share the pin: `index` picks one by the order of the
//! ROM search. A step never waits: it starts a conversion and reads it 750 ms later.
//!
//! Build: `drivers\build.ps1 ds18b20`.

#![no_std]

#[link(wasm_import_module = "crab")]
extern "C" {
    fn param(index: i32) -> f64;
    fn output(index: i32, value: f64);
}

#[link(wasm_import_module = "bus")]
extern "C" {
    fn ow_reset(pin: i32) -> i32;
    fn ow_write(pin: i32, ptr: *const u8, len: i32) -> i32;
    fn ow_read(pin: i32, ptr: *mut u8, len: i32) -> i32;
    fn ow_read_bit(pin: i32) -> i32;
    fn ow_write_bit(pin: i32, bit: i32) -> i32;
}

// Parameters and outputs in the order of block.toml.
const P_PIN: i32 = 0;
const P_INDEX: i32 = 1;
const P_PERIOD: i32 = 2;
const O_TEMP: i32 = 0;
const O_OK: i32 = 1;

const SKIP_ROM: u8 = 0xCC;
const MATCH_ROM: u8 = 0x55;
const SEARCH_ROM: u8 = 0xF0;
const CONVERT: u8 = 0x44;
const READ_SCRATCHPAD: u8 = 0xBE;
/// A 12-bit conversion takes up to 750 ms.
const CONVERT_MS: i32 = 750;
/// Most sensors one search walks through.
const MAX_INDEX: i32 = 16;

static mut WAIT_MS: i32 = 0;
static mut CONVERTING: bool = false;
static mut ROM: [u8; 8] = [0; 8];
static mut HAVE_ROM: bool = false;
static mut BUF: [u8; 9] = [0; 9];

fn pin() -> i32 {
    unsafe { param(P_PIN) as i32 }
}

fn period() -> i32 {
    unsafe { param(P_PERIOD) as i32 }
}

fn write(bytes: &[u8]) {
    unsafe {
        ow_write(pin(), bytes.as_ptr(), bytes.len() as i32);
    }
}

/// Dallas CRC-8 (polynomial x^8 + x^5 + x^4 + 1, reflected).
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            b >>= 1;
        }
    }
    crc
}

/// ROM search (Maxim application note 187): the ROM of the `index`-th device found, if any.
fn find(index: i32) -> Option<[u8; 8]> {
    let pin = pin();
    let mut rom = [0u8; 8];
    let mut last_discrepancy = 0i32;
    for found in 0..=index.clamp(0, MAX_INDEX) {
        if found > 0 && last_discrepancy == 0 {
            return None;
        }
        if unsafe { ow_reset(pin) } != 1 {
            return None;
        }
        write(&[SEARCH_ROM]);
        let mut discrepancy = 0i32;
        for bit in 1..=64i32 {
            let (a, b) = unsafe { (ow_read_bit(pin), ow_read_bit(pin)) };
            let byte = ((bit - 1) / 8) as usize;
            let mask = 1u8 << ((bit - 1) % 8);
            let take = match (a, b) {
                (1, 1) => return None,
                (0, 0) => {
                    let take = if bit < last_discrepancy {
                        rom[byte] & mask != 0
                    } else {
                        bit == last_discrepancy
                    };
                    if !take {
                        discrepancy = bit;
                    }
                    take
                }
                _ => a == 1,
            };
            if take {
                rom[byte] |= mask;
            } else {
                rom[byte] &= !mask;
            }
            unsafe { ow_write_bit(pin, take as i32) };
        }
        last_discrepancy = discrepancy;
        if crc8(&rom[..7]) != rom[7] {
            return None;
        }
    }
    Some(rom)
}

fn fail() {
    unsafe { output(O_OK, 0.0) };
}

#[no_mangle]
pub extern "C" fn init() {
    unsafe {
        WAIT_MS = 0;
        CONVERTING = false;
        HAVE_ROM = false;
    }
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    unsafe {
        WAIT_MS -= dt_ms;
        if WAIT_MS > 0 {
            return;
        }
        if !CONVERTING {
            // All sensors on the pin convert at once.
            if ow_reset(pin()) != 1 {
                HAVE_ROM = false;
                fail();
                WAIT_MS = period();
                return;
            }
            write(&[SKIP_ROM, CONVERT]);
            CONVERTING = true;
            WAIT_MS = CONVERT_MS;
            return;
        }
        CONVERTING = false;
        WAIT_MS = (period() - CONVERT_MS).max(0);
        if !HAVE_ROM {
            match find(param(P_INDEX) as i32) {
                Some(rom) => {
                    ROM = rom;
                    HAVE_ROM = true;
                }
                None => return fail(),
            }
        }
        if ow_reset(pin()) != 1 {
            HAVE_ROM = false;
            return fail();
        }
        let rom = *core::ptr::addr_of!(ROM);
        let mut select = [MATCH_ROM, 0, 0, 0, 0, 0, 0, 0, 0, READ_SCRATCHPAD];
        select[1..9].copy_from_slice(&rom);
        write(&select);
        let buf = &mut *core::ptr::addr_of_mut!(BUF);
        ow_read(pin(), buf.as_mut_ptr(), 9);
        if crc8(&buf[..8]) != buf[8] {
            // Wrong device or a broken line: search again next time.
            HAVE_ROM = false;
            return fail();
        }
        let raw = i16::from_le_bytes([buf[0], buf[1]]);
        output(O_TEMP, f64::from(raw) / 16.0);
        output(O_OK, 1.0);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
