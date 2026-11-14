//! Estimated cost of one sheet step on the board, microseconds per block kind.
//!
//! The numbers come from the MCU bench (`crablogic bench`, reports
//! `bench/esp32-0.1.0-2026-11-13-1917.md` and `bench/esp32c3-0.1.0-2026-11-13-1928.md`, AOT mode,
//! master with the hot path of the step in IRAM):
//! per-block cost of a family = (step of the 500-block stand - step of the 10-block stand) / 490.
//! The calls of the master (pins, ADC, I2C, UART, flash) are taken from the table
//! "Цена вызовов мастера". This is an ESTIMATE for a warning, not a guarantee: refresh it from the
//! bench when the master or the code generator gets faster or slower.

use crate::pins::Chip;
use crate::project::{is_screen_kind, is_web_kind};

/// What a block costs per step, by what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Bool and integer logic, triggers, edges, switches, constants.
    Logic,
    /// Float and numeric math, scaling, comparison, PID.
    Float,
    /// Timers, generator, pulse, counter, debounce.
    Timers,
    /// Text blocks.
    Text,
    GpioIn,
    GpioOut,
    Adc,
    I2c,
    Uart,
    Retain,
    /// Network blocks: Modbus, MQTT, Sparkplug and their settings, Wi-Fi, Ethernet, NTP.
    Net,
    /// Not on the board as step code: web widgets and screen widgets.
    Free,
    /// Not measured (displays, sensors, user blocks): a conservative guess.
    Other,
}

/// Costs of one chip, microseconds.
#[derive(Debug, Clone, Copy)]
pub struct ChipCost {
    /// An empty sheet: the call of the module and the loop of the master.
    pub overhead: f32,
    pub logic: f32,
    pub float: f32,
    pub timers: f32,
    pub text: f32,
    pub gpio_in: f32,
    pub gpio_out: f32,
    pub adc: f32,
    pub i2c: f32,
    pub uart: f32,
    pub retain: f32,
    /// From the `cycle_1ms` stand: its sheet minus logic, ADC and the empty sheet, divided by its
    /// 14 network blocks. Bench 13.11.2026 21:35/21:45, after the step stopped taking the master's
    /// locks for network calls (ESP32 30 -> 11 us, C3 40 -> 23 us).
    pub net: f32,
    pub other: f32,
}

/// ESP32 (Xtensa, AOT). Bench 13.11.2026 19:17: logic 0.15, float 0.79, timers 3.18, text 3.52 us/block.
pub const ESP32: ChipCost = ChipCost {
    overhead: 30.0,
    logic: 0.15,
    float: 0.79,
    timers: 3.2,
    text: 3.5,
    gpio_in: 4.7,
    gpio_out: 4.6,
    adc: 37.0,
    i2c: 22.9,
    uart: 30.8,
    retain: 1.0,
    net: 11.0,
    other: 5.0,
};

/// ESP32-C3 (RISC-V, AOT). Bench 13.11.2026 19:28: logic 1.33, float 2.19, timers 2.70, text 4.63 us/block.
pub const ESP32_C3: ChipCost = ChipCost {
    overhead: 24.0,
    logic: 1.33,
    float: 2.2,
    timers: 2.7,
    text: 4.6,
    gpio_in: 3.8,
    gpio_out: 3.9,
    adc: 16.1,
    i2c: 15.9,
    uart: 27.4,
    retain: 1.0,
    net: 23.0,
    other: 6.0,
};

/// Costs for the chip of a project; an unknown chip gets the ESP32 table, S3 too (not measured).
pub fn costs_for(chip: Option<Chip>) -> &'static ChipCost {
    match chip {
        Some(Chip::Esp32C3 | Chip::Esp32C3SuperMini) => &ESP32_C3,
        _ => &ESP32,
    }
}

/// Family of a block kind (library name); an unknown kind is [`Family::Other`].
pub fn family_of(kind: &str) -> Family {
    if is_web_kind(kind) || is_screen_kind(kind) {
        return Family::Free;
    }
    match kind {
        "and" | "or" | "xor" | "not" | "constant" | "bit_and" | "bit_or" | "bit_xor"
        | "bit_not" | "bit_read" | "bit_write" | "bits_to_byte" | "byte_to_bits" | "decoder"
        | "encoder" | "rising_edge" | "falling_edge" | "rs_trigger" | "sr_trigger"
        | "t_trigger" | "bit_change" | "number_change" | "mux" | "switch" | "dms" => Family::Logic,
        "add" | "sub" | "mul" | "div" | "abs" | "cos" | "sin" | "tan" | "limit" | "max" | "min"
        | "mod" | "pow" | "random" | "sq" | "sqrt" | "scale" | "scale_table" | "compare"
        | "greater" | "less" | "hysteresis" | "pid" | "int_to_float" | "float_to_int"
        | "bytes_to_dword" | "bytes_to_word" | "dword_to_bytes" | "word_to_bytes" => Family::Float,
        "generator" | "pulse" | "timer_on" | "timer_off" | "counter" | "debounce" => Family::Timers,
        "num_to_text" | "text_add_char" | "text_char" | "text_compare" | "text_concat"
        | "text_const" | "text_find" | "text_length" | "text_substring" | "text_to_num"
        | "string_change" | "ip_to_text" | "text_to_ip" => Family::Text,
        "digital_in" => Family::GpioIn,
        "digital_out" | "pwm_out" | "analog_out" => Family::GpioOut,
        "analog_in" => Family::Adc,
        "pcf8574_in" | "pcf8574_out" => Family::I2c,
        "uart_read" | "uart_send" | "uart_config" => Family::Uart,
        "flash_store" => Family::Retain,
        "modbus_read"
        | "modbus_write"
        | "modbus_master_read"
        | "modbus_master_write"
        | "modbus_rtu_master"
        | "modbus_rtu_slave"
        | "modbus_tcp_master"
        | "modbus_tcp_slave"
        | "mqtt"
        | "mqtt_publish"
        | "mqtt_subscribe"
        | "sparkplug_metric"
        | "sparkplug_node"
        | "wifi"
        | "ethernet"
        | "ntp" => Family::Net,
        _ => Family::Other,
    }
}

impl ChipCost {
    /// Estimated microseconds one block of `kind` adds to a step.
    pub fn block_us(&self, kind: &str) -> f32 {
        match family_of(kind) {
            Family::Logic => self.logic,
            Family::Float => self.float,
            Family::Timers => self.timers,
            Family::Text => self.text,
            Family::GpioIn => self.gpio_in,
            Family::GpioOut => self.gpio_out,
            Family::Adc => self.adc,
            Family::I2c => self.i2c,
            Family::Uart => self.uart,
            Family::Retain => self.retain,
            Family::Net => self.net,
            Family::Free => 0.0,
            Family::Other => self.other,
        }
    }

    /// Estimated step of a sheet with the given block kinds, microseconds.
    pub fn sheet_us<'a>(&self, kinds: impl IntoIterator<Item = &'a str>) -> f32 {
        self.overhead + kinds.into_iter().map(|k| self.block_us(k)).sum::<f32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_differ_and_unknown_is_conservative() {
        assert!(ESP32.block_us("and") < ESP32_C3.block_us("and"));
        assert!(ESP32.block_us("timer_on") > ESP32.block_us("and"));
        assert!(ESP32.block_us("no_such_block") >= ESP32.block_us("and"));
        assert_eq!(ESP32.block_us("web_label"), 0.0);
    }
}
