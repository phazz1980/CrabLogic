//! UART bytes of the blocks (`uart(op, port, a, b)`), as the C++ master does them. Port 0 is the
//! console of the master (its protocol): a module may only send text or bytes there. Ports 1 and 2
//! (only 1 on the C3) are set up by codes and then read and written byte by byte.
//!
//! op 0 setting: a = code, b = value; code 0 starts the list, 63 starts the port (once).
//!   Codes: 1 speed, 2 data bits, 3 parity (0 N, 1 E, 2 O), 4 stop bits, 5 rx, 6 tx, 7 de (RS-485
//!   direction), 8 receive buffer.
//! op 1 next byte or -1; op 2 bytes waiting; op 3 write the low `1 << b` bytes of `a`, 1 if they
//! fitted; op 4 speed now; op 5 set speed `a`; op 6 write text (pointer `a`, length `b`), 1 if it
//! fitted; op 7 new frame format as text ("8E1"), 1 if it is a format.

use crab_kernel::host::Mem;
use esp_idf_svc::sys;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(target_arch = "riscv32")]
const PORTS: usize = 2;
#[cfg(not(target_arch = "riscv32"))]
const PORTS: usize = 3;

/// Pins of a port when the block leaves them at -1: those of the Arduino core 3 (never the flash
/// pins the UART1 of the ESP32 has by default in ESP-IDF).
#[cfg(target_arch = "riscv32")]
const DEFAULT_PINS: [(i32, i32); PORTS] = [(-1, -1), (18, 19)];
#[cfg(not(target_arch = "riscv32"))]
const DEFAULT_PINS: [(i32, i32); PORTS] = [(-1, -1), (26, 27), (16, 17)];

const TX_BUFFER: i32 = 256;

struct Port {
    codes: [i32; 16],
    on: bool,
}

static UART: Mutex<[Port; PORTS]> = Mutex::new(
    [const {
        Port {
            codes: [0; 16],
            on: false,
        }
    }; PORTS],
);

fn word_length(bits: i32) -> sys::uart_word_length_t {
    match bits {
        5 => sys::uart_word_length_t_UART_DATA_5_BITS,
        6 => sys::uart_word_length_t_UART_DATA_6_BITS,
        7 => sys::uart_word_length_t_UART_DATA_7_BITS,
        _ => sys::uart_word_length_t_UART_DATA_8_BITS,
    }
}

fn parity(code: i32) -> sys::uart_parity_t {
    match code {
        1 => sys::uart_parity_t_UART_PARITY_EVEN,
        2 => sys::uart_parity_t_UART_PARITY_ODD,
        _ => sys::uart_parity_t_UART_PARITY_DISABLE,
    }
}

fn stop_bits(n: i32) -> sys::uart_stop_bits_t {
    if n == 2 {
        sys::uart_stop_bits_t_UART_STOP_BITS_2
    } else {
        sys::uart_stop_bits_t_UART_STOP_BITS_1
    }
}

/// Starts port `n` with its codes; `false` if the driver could not be installed (another module
/// took the port through `crab.bus`).
fn start(n: usize, c: &[i32; 16]) -> bool {
    let port = n as sys::uart_port_t;
    let pin = |v: i32, default: i32| if v >= 0 { v } else { default };
    let (rx, tx) = (pin(c[5], DEFAULT_PINS[n].0), pin(c[6], DEFAULT_PINS[n].1));
    let rx_buffer = if c[8] > 0 { c[8] } else { 256 }.max(256);
    let cfg = sys::uart_config_t {
        baud_rate: if c[1] > 0 { c[1] } else { 9600 },
        data_bits: word_length(c[2]),
        parity: parity(c[3]),
        stop_bits: stop_bits(c[4]),
        flow_ctrl: sys::uart_hw_flowcontrol_t_UART_HW_FLOWCTRL_DISABLE,
        ..Default::default()
    };
    // SAFETY: a valid port; the driver is installed first, so a port in use is left alone.
    unsafe {
        if sys::uart_driver_install(port, rx_buffer, TX_BUFFER, 0, core::ptr::null_mut(), 0) != 0 {
            return false;
        }
        let rts = if c[7] >= 0 { c[7] } else { -1 };
        if sys::uart_param_config(port, &cfg) != 0 || sys::uart_set_pin(port, tx, rx, rts, -1) != 0
        {
            sys::uart_driver_delete(port);
            return false;
        }
        if c[7] >= 0 {
            sys::uart_set_mode(port, sys::uart_mode_t_UART_MODE_RS485_HALF_DUPLEX);
        }
    }
    true
}

/// Writes to the console of the master; gives up after 200 ms (nobody is reading the USB of a C3).
fn console(bytes: &[u8]) {
    let started = Instant::now();
    let mut out = std::io::stdout().lock();
    let mut rest = bytes;
    while !rest.is_empty() && started.elapsed() < Duration::from_millis(200) {
        match out.write(rest) {
            Ok(0) | Err(_) => std::thread::sleep(Duration::from_millis(1)),
            Ok(n) => rest = &rest[n..],
        }
    }
    let _ = out.flush();
}

fn tx_free(port: sys::uart_port_t) -> usize {
    let mut free = 0usize;
    // SAFETY: an installed port; plain out-parameter.
    unsafe { sys::uart_get_tx_buffer_free_size(port, &mut free) };
    free
}

/// Parses "8N1", "7e2"...: spaces around are skipped, the letter may be of either case.
pub fn parse_format(text: &[u8]) -> Option<(i32, i32, i32)> {
    let text = std::str::from_utf8(text).ok()?.trim_matches(' ').as_bytes();
    let [d, p, s] = text else { return None };
    let data = i32::from(d.wrapping_sub(b'0'));
    let stop = i32::from(s.wrapping_sub(b'0'));
    let parity = match p.to_ascii_uppercase() {
        b'N' => 0,
        b'E' => 1,
        b'O' => 2,
        _ => return None,
    };
    ((5..=8).contains(&data) && (stop == 1 || stop == 2)).then_some((data, parity, stop))
}

pub fn call(op: i32, port: i32, a: i32, b: i32, mem: &mut Mem<'_>) -> i32 {
    if port == 0 {
        return match op {
            3 => {
                let n = 1usize << b.clamp(0, 2);
                console(&a.to_le_bytes()[..n]);
                1
            }
            6 => match mem.read(a, b, 4096) {
                Some(text) => {
                    console(&text);
                    1
                }
                None => 0,
            },
            _ => 0,
        };
    }
    let Some(n) = usize::try_from(port).ok().filter(|&n| n > 0 && n < PORTS) else {
        return if op == 1 { -1 } else { 0 };
    };
    let mut ports = UART.lock().unwrap_or_else(|e| e.into_inner());
    let p = &mut ports[n];
    let id = n as sys::uart_port_t;
    if op == 0 {
        match a {
            0 => p.codes = [0; 16],
            63 => {
                if !p.on {
                    p.on = start(n, &p.codes);
                }
            }
            1..=15 => p.codes[a as usize] = b,
            _ => {}
        }
        return 0;
    }
    if !p.on {
        return if op == 1 { -1 } else { 0 };
    }
    // SAFETY (all calls below): the port's driver is installed (`p.on`).
    match op {
        1 => {
            let mut byte = 0u8;
            let got = unsafe { sys::uart_read_bytes(id, (&mut byte as *mut u8).cast(), 1, 0) };
            if got == 1 {
                i32::from(byte)
            } else {
                -1
            }
        }
        2 => {
            let mut waiting = 0usize;
            unsafe { sys::uart_get_buffered_data_len(id, &mut waiting) };
            waiting as i32
        }
        3 => {
            let size = 1usize << b.clamp(0, 2);
            if tx_free(id) < size {
                return 0;
            }
            let bytes = a.to_le_bytes();
            unsafe { sys::uart_write_bytes(id, bytes.as_ptr().cast(), size) };
            1
        }
        4 => {
            let mut baud = 0u32;
            unsafe { sys::uart_get_baudrate(id, &mut baud) };
            baud as i32
        }
        5 => {
            if a > 0 {
                unsafe { sys::uart_set_baudrate(id, a as u32) };
            }
            0
        }
        6 => {
            let Some(text) = mem.read(a, b, 4096) else {
                return 0;
            };
            if tx_free(id) < text.len().min(64) {
                return 0;
            }
            unsafe { sys::uart_write_bytes(id, text.as_ptr().cast(), text.len()) };
            1
        }
        7 => {
            let Some((data, par, stop)) = mem.read(a, b, 16).and_then(|t| parse_format(&t)) else {
                return 0;
            };
            p.codes[2] = data;
            p.codes[3] = par;
            p.codes[4] = stop;
            unsafe {
                sys::uart_set_word_length(id, word_length(data));
                sys::uart_set_parity(id, parity(par));
                sys::uart_set_stop_bits(id, stop_bits(stop));
            }
            1
        }
        _ => 0,
    }
}
