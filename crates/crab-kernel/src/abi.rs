//! Host ABI v4: the one list of imports a module may use, with signatures and permissions.
//!
//! Values are 32 bit: `bool` and `int` travel as `i32`, `float` as `f32` (see `specs/numeric-model`).
//! Exports of a module: `abi_version() -> i32` (must be 4), `step(dt_ms: i32)`; optional
//! `init()`, `alloc(len: i32) -> i32` (where the host puts message bytes) and
//! `on_message(topic_ptr, topic_len, ptr, len)`.
//!
//! ABI 4 adds `crab.yield() -> i32`: a module counts its work (one unit per `step` and per loop
//! pass) and calls it when the count runs out; the host may suspend the module there until its
//! next quantum and returns the next budget. Modules of an older ABI must be rebuilt.
//!
//! Imports come from module `crab`. Those marked [`Perm::Always`] every module may use; the
//! others touch the hardware (or a costly service) and work only if the manifest `allow` list
//! names them. Both the PC simulator and the board master check modules against this list.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The ABI version this host speaks.
pub const ABI_VERSION: i32 = 4;

/// The module name all imports come from.
pub const MODULE: &str = "crab";

/// A value type of the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    I32,
    F32,
}

/// Who may use an import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perm {
    /// Every module.
    Always,
    /// Only a module whose manifest `allow` names the import.
    Allow,
}

/// One import of the ABI.
#[derive(Debug, Clone, Copy)]
pub struct Import {
    pub name: &'static str,
    pub params: &'static [Ty],
    pub results: &'static [Ty],
    pub perm: Perm,
}

use Perm::{Allow, Always};
use Ty::{F32, I32};

const fn imp(
    name: &'static str,
    params: &'static [Ty],
    results: &'static [Ty],
    perm: Perm,
) -> Import {
    Import {
        name,
        params,
        results,
        perm,
    }
}

/// Every import of ABI v4; the position in this list is the import id hosts dispatch on.
pub const IMPORTS: &[Import] = &[
    imp("input_i", &[I32], &[I32], Always),
    imp("input_f", &[I32], &[F32], Always),
    imp("param_i", &[I32], &[I32], Always),
    imp("param_f", &[I32], &[F32], Always),
    imp("output_i", &[I32, I32], &[I32], Always),
    imp("output_f", &[I32, F32], &[F32], Always),
    imp("log", &[I32, I32], &[], Always),
    imp("tag_get_i", &[I32], &[I32], Always),
    imp("tag_get_f", &[I32], &[F32], Always),
    imp("tag_set_i", &[I32, I32], &[], Always),
    imp("tag_set_f", &[I32, F32], &[], Always),
    imp("publish", &[I32, I32, I32, I32], &[], Always),
    imp("subscribe", &[I32, I32], &[], Always),
    imp("gpio_read", &[I32], &[I32], Allow),
    imp("gpio_write", &[I32, I32], &[], Allow),
    imp("adc_read", &[I32], &[I32], Allow),
    imp("pwm_write", &[I32, I32], &[], Allow),
    imp("millis", &[], &[I32], Allow),
    imp("sensor_read", &[I32, I32, I32], &[F32], Allow),
    imp("pcf_read", &[I32, I32, I32], &[I32], Allow),
    imp("pcf_write", &[I32, I32], &[I32], Allow),
    imp("modbus", &[I32, I32, I32, I32, I32], &[I32], Allow),
    imp("uart", &[I32, I32, I32, I32], &[I32], Allow),
    imp("text", &[I32, I32, I32, I32, I32, I32, I32], &[I32], Allow),
    imp("net", &[I32, I32, I32, I32, I32], &[I32], Allow),
    imp("native", &[I32, I32, I32, I32], &[I32], Allow),
    imp("math", &[I32, F32, F32], &[F32], Allow),
    imp("retain", &[I32, I32, I32], &[I32], Allow),
    imp("tag_text", &[I32, I32, I32, I32], &[I32], Allow),
    imp("retain_text", &[I32, I32, I32, I32], &[I32], Allow),
    imp("yield", &[], &[I32], Always),
    imp("bus", &[I32, I32, I32, I32, I32], &[I32], Allow),
];

/// Import ids by name, for hosts that dispatch with `match`.
pub mod id {
    pub const INPUT_I: usize = 0;
    pub const INPUT_F: usize = 1;
    pub const PARAM_I: usize = 2;
    pub const PARAM_F: usize = 3;
    pub const OUTPUT_I: usize = 4;
    pub const OUTPUT_F: usize = 5;
    pub const LOG: usize = 6;
    pub const TAG_GET_I: usize = 7;
    pub const TAG_GET_F: usize = 8;
    pub const TAG_SET_I: usize = 9;
    pub const TAG_SET_F: usize = 10;
    pub const PUBLISH: usize = 11;
    pub const SUBSCRIBE: usize = 12;
    pub const GPIO_READ: usize = 13;
    pub const GPIO_WRITE: usize = 14;
    pub const ADC_READ: usize = 15;
    pub const PWM_WRITE: usize = 16;
    pub const MILLIS: usize = 17;
    pub const SENSOR_READ: usize = 18;
    pub const PCF_READ: usize = 19;
    pub const PCF_WRITE: usize = 20;
    pub const MODBUS: usize = 21;
    pub const UART: usize = 22;
    pub const TEXT: usize = 23;
    pub const NET: usize = 24;
    pub const NATIVE: usize = 25;
    pub const MATH: usize = 26;
    pub const RETAIN: usize = 27;
    pub const TAG_TEXT: usize = 28;
    pub const RETAIN_TEXT: usize = 29;
    pub const YIELD: usize = 30;
    pub const BUS: usize = 31;
}

/// Operation codes and error codes of `crab.bus(op, a, b, c, d)`: GPIO, I²C and UART of the board
/// for driver blocks. Resources must be claimed (`CLAIM`) before `LOCK`; every other call is
/// checked against the claims of the calling module.
pub mod bus_op {
    /// `CLAIM(kind, value)`: before `LOCK` only.
    pub const CLAIM: i32 = 0;
    /// Ends the claims of the module.
    pub const LOCK: i32 = 1;
    /// `GPIO_MODE(pin, mode)`; modes: 0 input, 1 output, 2 input pull-up, 3 input pull-down,
    /// 4 open-drain output.
    pub const GPIO_MODE: i32 = 10;
    pub const GPIO_READ: i32 = 11;
    pub const GPIO_WRITE: i32 = 12;
    /// `I2C_OPEN(sda, scl, hz)`.
    pub const I2C_OPEN: i32 = 20;
    /// `I2C_WRITE(addr, ptr, len)`.
    pub const I2C_WRITE: i32 = 21;
    /// `I2C_READ(addr, ptr, len)`.
    pub const I2C_READ: i32 = 22;
    /// `I2C_WRITE_READ(addr, wptr, wlen | (rlen << 16), rptr)`.
    pub const I2C_WRITE_READ: i32 = 23;
    /// `UART_OPEN(port, tx, rx, baud)`, 8N1.
    pub const UART_OPEN: i32 = 30;
    /// `UART_WRITE(port, ptr, len)`.
    pub const UART_WRITE: i32 = 31;
    /// `UART_READ(port, ptr, cap)`: non-blocking.
    pub const UART_READ: i32 = 32;
    pub const UART_AVAILABLE: i32 = 33;
    /// Microseconds since boot, wrapping.
    pub const MICROS: i32 = 40;
    /// `DELAY_US(us)`: busy wait, `us` clamped to 0..=100.
    pub const DELAY_US: i32 = 41;
    /// `SPI_OPEN(sck, mosi, miso, hz)`: `mosi` or `miso` may be -1 (not used); `hz` 0 means 1 MHz,
    /// clamped to 80 MHz. One SPI controller per board for drivers (classic ESP32: SPI3, C3: SPI2,
    /// which is not available when the Ethernet W5500 uses it). Same pins again: 0, other pins: -4.
    pub const SPI_OPEN: i32 = 50;
    /// `SPI_TRANSFER(cs, ptr, len, mode)`: full duplex in place, `mode` 0..=3 (CPOL/CPHA). `cs` is a
    /// claimed GPIO the master holds low during the transfer (-1: no chip select). Returns `len`.
    pub const SPI_TRANSFER: i32 = 51;
    /// `OW_RESET(pin)`: 1-Wire reset; 1 if a presence pulse was seen, 0 if not.
    pub const OW_RESET: i32 = 60;
    /// `OW_WRITE(pin, ptr, len)`: bytes, least significant bit first. Returns `len`.
    pub const OW_WRITE: i32 = 61;
    /// `OW_READ(pin, ptr, len)`: bytes into the module memory. Returns `len`.
    pub const OW_READ: i32 = 62;
    /// `OW_READ_BIT(pin)`: 0 or 1.
    pub const OW_READ_BIT: i32 = 63;
    /// `OW_WRITE_BIT(pin, bit)`: returns 0.
    pub const OW_WRITE_BIT: i32 = 64;

    /// Claim kinds.
    pub const KIND_PIN: i32 = 1;
    pub const KIND_I2C_ADDR: i32 = 2;
    pub const KIND_UART: i32 = 3;

    /// Resource not claimed (or claims not locked yet, or the bus not opened by the module).
    pub const ERR_NOT_CLAIMED: i32 = -1;
    /// Bus error, NACK.
    pub const ERR_BUS: i32 = -2;
    pub const ERR_TIMEOUT: i32 = -3;
    /// Bad arguments: pin not usable on the chip, length over `MAX_LEN`, pointer outside the memory.
    pub const ERR_ARGS: i32 = -4;
    /// Largest transfer of one call, bytes.
    pub const MAX_LEN: i32 = 256;
}

/// The id of an import by name.
pub fn find(name: &str) -> Option<usize> {
    IMPORTS.iter().position(|i| i.name == name)
}

/// The names a manifest `allow` list may contain, in table order.
pub fn permissions() -> impl Iterator<Item = &'static str> {
    IMPORTS
        .iter()
        .filter(|i| i.perm == Perm::Allow)
        .map(|i| i.name)
}

/// Checks the names of a manifest `allow` list.
pub fn check_allow(allow: &[String]) -> Result<(), String> {
    for name in allow {
        if !permissions().any(|p| p == name) {
            return Err(format!(
                "в allow манифеста неизвестное право «{name}» (допустимы: {})",
                permissions().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(())
}

/// Checks the imports of a module (`(module, name)` pairs) against the list and `allow`.
pub fn check_imports<'a>(
    imports: impl IntoIterator<Item = (&'a str, &'a str)>,
    allow: &[String],
) -> Result<(), String> {
    for (m, n) in imports {
        let known = (m == MODULE).then(|| find(n)).flatten();
        let Some(id) = known else {
            return Err(format!("неизвестный импорт {m}.{n}"));
        };
        if IMPORTS[id].perm == Perm::Allow && !allow.iter().any(|a| a == n) {
            return Err(format!(
                "модулю не разрешён импорт crab.{n}: добавьте «{n}» в allow манифеста"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::ToOwned;
    use alloc::vec;

    #[test]
    fn ids_match_the_table() {
        let named = [
            (id::INPUT_I, "input_i"),
            (id::SUBSCRIBE, "subscribe"),
            (id::GPIO_READ, "gpio_read"),
            (id::MILLIS, "millis"),
            (id::TEXT, "text"),
            (id::RETAIN_TEXT, "retain_text"),
            (id::YIELD, "yield"),
            (id::BUS, "bus"),
        ];
        for (i, name) in named {
            assert_eq!(IMPORTS[i].name, name);
        }
        assert_eq!(IMPORTS.len(), id::BUS + 1);
    }

    #[test]
    fn hardware_import_needs_permission() {
        let allow = vec!["gpio_read".to_owned()];
        assert!(check_imports([("crab", "log"), ("crab", "gpio_read")], &allow).is_ok());
        let err = check_imports([("crab", "gpio_write")], &allow).unwrap_err();
        assert!(err.contains("allow"), "{err}");
        assert!(check_imports([("env", "log")], &allow).is_err());
        assert!(
            check_allow(&["log".to_owned()]).is_err(),
            "log is not a permission"
        );
    }
}
