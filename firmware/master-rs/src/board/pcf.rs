//! PCF8574 / PCF8575 expanders of the project (`pcf_read`, `pcf_write`), as the C++ master does
//! them: modules only read the last word and set the wanted one; a task of the master talks to the
//! chips every 20 ms over the I²C bus.
//!
//! `key` = address (bits 0–6) | SDA << 8 | SCL << 16 | 1 << 24 for a 16-bit PCF8575; SDA or SCL
//! 0xFF is the default pin of the board.

use super::bus;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Chips one board serves.
const CHIPS: usize = 16;
const POLL: Duration = Duration::from_millis(20);
/// A writer whose chip does not answer retries this often, ms.
const RETRY_MS: u64 = 500;

/// Default I²C pins of the Arduino core (`Wire.begin()`).
#[cfg(target_arch = "riscv32")]
const DEFAULT_SDA_SCL: (i32, i32) = (8, 9);
#[cfg(not(target_arch = "riscv32"))]
const DEFAULT_SDA_SCL: (i32, i32) = (21, 22);

#[derive(Clone, Copy)]
struct Chip {
    key: i32,
    bytes: usize,
    writer: bool,
    reader: bool,
    want: u16,
    sent: Option<u16>,
    value: u16,
    ok: bool,
    period_ms: u64,
    last: Option<Instant>,
    last_read: Option<Instant>,
    ready: bool,
}

static TABLE: Mutex<Vec<Chip>> = Mutex::new(Vec::new());

/// The chip of `key`, registered at its first use; the poll task starts with the first chip.
fn with_chip<R>(
    key: i32,
    writer: bool,
    period: i32,
    first: u16,
    f: impl FnOnce(&mut Chip) -> R,
) -> Option<R> {
    let mut table = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let period = u64::try_from(period.max(5)).unwrap_or(5);
    let index = match table.iter().position(|c| c.key == key) {
        Some(i) => {
            let c = &mut table[i];
            // A chip with outputs and inputs: the first writer's word goes out as the module wants it.
            if writer && !c.writer {
                c.want = first;
                c.writer = true;
            }
            if !writer {
                c.reader = true;
                c.period_ms = c.period_ms.min(period);
            }
            i
        }
        None => {
            if table.len() >= CHIPS {
                return None;
            }
            if table.is_empty() {
                start_task();
            }
            table.push(Chip {
                key,
                bytes: if key >> 24 & 1 != 0 { 2 } else { 1 },
                writer,
                reader: !writer,
                want: first,
                sent: None,
                value: 0,
                ok: false,
                period_ms: period,
                last: None,
                last_read: None,
                ready: false,
            });
            table.len() - 1
        }
    };
    Some(f(&mut table[index]))
}

/// `pcf_read(key, period, field)`: field 0 the last word read, 1 whether the chip answered.
pub fn read(key: i32, period: i32, field: i32) -> i32 {
    with_chip(key, false, period, 0xFFFF, |c| {
        if field == 0 {
            i32::from(c.value)
        } else {
            i32::from(c.ok)
        }
    })
    .unwrap_or(0)
}

/// `pcf_write(key, value)`: sets the wanted word; 1 if the chip answered the last write.
pub fn write(key: i32, value: i32) -> i32 {
    let word = (value & 0xFFFF) as u16;
    with_chip(key, true, 0, word, |c| {
        c.want = word;
        i32::from(c.ok)
    })
    .unwrap_or(0)
}

fn start_task() {
    let spawned = std::thread::Builder::new()
        .name("pcf".into())
        .stack_size(4096)
        .spawn(|| loop {
            poll_all();
            std::thread::sleep(POLL);
        });
    if spawned.is_err() {
        super::notes::push("pcf", "задача опроса расширителей не запущена".into());
    }
}

fn poll_all() {
    // Copies out, so modules are not held up by a slow bus.
    let chips: Vec<Chip> = TABLE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for chip in chips {
        let polled = poll(chip, Instant::now());
        let mut table = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = table.iter_mut().find(|c| c.key == chip.key) {
            // Keep what modules changed meanwhile (the wanted word, roles, period).
            c.sent = polled.sent;
            c.value = polled.value;
            c.ok = polled.ok;
            c.last = polled.last;
            c.last_read = polled.last_read;
            c.ready = polled.ready;
        }
    }
}

fn elapsed_ms(since: Option<Instant>, now: Instant) -> u64 {
    since.map_or(u64::MAX, |t| now.duration_since(t).as_millis() as u64)
}

fn send(c: &Chip, word: u16) -> bool {
    let bytes = word.to_le_bytes();
    bus::system_i2c_write((c.key & 0x7F) as u8, &bytes[..c.bytes])
}

fn poll(mut c: Chip, now: Instant) -> Chip {
    let pin = |v: i32, default: i32| if v == 0xFF { default } else { v };
    let sda = pin(c.key >> 8 & 0xFF, DEFAULT_SDA_SCL.0);
    let scl = pin(c.key >> 16 & 0xFF, DEFAULT_SDA_SCL.1);
    if !bus::system_i2c(sda, scl) {
        c.ok = false;
        return c;
    }
    if !c.ready {
        // A pin reads as an input while the chip holds it high.
        let first = if c.writer { c.want } else { 0xFFFF };
        c.ok = send(&c, first);
        if c.writer && c.ok {
            c.sent = Some(first);
        }
        c.last = Some(now);
        c.last_read = Some(now);
        c.ready = true;
        return c;
    }
    if c.writer {
        // Write on change; while the chip does not answer, retry twice a second.
        let due = if c.ok {
            c.sent != Some(c.want)
        } else {
            elapsed_ms(c.last, now) >= RETRY_MS
        };
        if due {
            c.ok = send(&c, c.want);
            c.last = Some(now);
            if c.ok {
                c.sent = Some(c.want);
            }
        }
    }
    if c.reader && elapsed_ms(c.last_read, now) >= c.period_ms {
        c.last_read = Some(now);
        let mut buf = [0u8; 2];
        c.ok = bus::system_i2c_read((c.key & 0x7F) as u8, &mut buf[..c.bytes]);
        if c.ok {
            c.value = u16::from_le_bytes(buf);
        }
    }
    c
}
