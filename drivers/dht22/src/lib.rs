//! DHT22 (AM2302) driver block: temperature and humidity on one data pin. The master measures the
//! answer of the sensor (`bus.pulse_read`: start pulse, then the lengths of the levels); the driver
//! decodes the 40 bits.
//! Interface v2 (`specs/no-f64`): integers everywhere, `f32` only for the `float` outputs. Build: `drivers\build.ps1 dht22`.

#![no_std]

#[link(wasm_import_module = "crab")]
extern "C" {
    fn param_i(index: i32) -> i32;
    fn output_i(index: i32, value: i32);
    fn output_f(index: i32, value: f32);
}

#[link(wasm_import_module = "bus")]
extern "C" {
    fn pulse_read(pin: i32, start_us: i32, ptr: *mut u16, count: i32) -> i32;
}

const P_PIN: i32 = 0;
const P_PERIOD: i32 = 1;
const O_TEMP: i32 = 0;
const O_HUMIDITY: i32 = 1;
const O_OK: i32 = 2;

/// The host holds the line low at least 1 ms to wake the sensor.
const START_US: i32 = 1100;
/// High after the release, the 80 µs low and high of the answer, 40 bits of low and high, the
/// final low; a little slack.
const LEVELS: usize = 86;
/// A high longer than this is a 1 (26–28 µs for 0, 70 µs for 1).
const ONE_US: u16 = 45;

static mut WAIT_MS: i32 = 0;
static mut LENGTHS: [u16; LEVELS] = [0; LEVELS];

/// The five bytes of the answer, if its levels make 40 bits with a good checksum.
fn decode(levels: &[u16]) -> Option<[u8; 5]> {
    // The answer starts with a low and a high of about 80 µs each; the level before them is the
    // released line (20–40 µs), which may be missed.
    let start = (0..3).find(|&k| {
        levels.len() > k + 1 && levels[k] > 60 && levels[k] < 110 && levels[k + 1] > 60
    })?;
    let bits = &levels[start + 2..];
    if bits.len() < 80 {
        return None;
    }
    let mut data = [0u8; 5];
    for i in 0..40 {
        if bits[2 * i + 1] > ONE_US {
            data[i / 8] |= 0x80 >> (i % 8);
        }
    }
    let sum = data[..4].iter().fold(0u8, |s, &b| s.wrapping_add(b));
    (sum == data[4]).then_some(data)
}

#[no_mangle]
pub extern "C" fn init() {
    unsafe { WAIT_MS = 0 };
}

#[no_mangle]
pub extern "C" fn step(dt_ms: i32) {
    unsafe {
        WAIT_MS -= dt_ms;
        if WAIT_MS > 0 {
            return;
        }
        WAIT_MS = param_i(P_PERIOD);
        let lengths = &mut *core::ptr::addr_of_mut!(LENGTHS);
        let n = pulse_read(
            param_i(P_PIN),
            START_US,
            lengths.as_mut_ptr(),
            LEVELS as i32,
        );
        let Some(d) = decode(&lengths[..n.max(0) as usize]) else {
            output_i(O_OK, 0);
            return;
        };
        // Tenths of %RH and of degree C in integers; floats only at the output ports.
        let humidity = i32::from(u16::from_be_bytes([d[0], d[1]]));
        let magnitude = i32::from(u16::from_be_bytes([d[2] & 0x7F, d[3]]));
        let temp = if d[2] & 0x80 != 0 {
            -magnitude
        } else {
            magnitude
        };
        output_f(O_TEMP, temp as f32 / 10.0);
        output_f(O_HUMIDITY, humidity as f32 / 10.0);
        output_i(O_OK, 1);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
