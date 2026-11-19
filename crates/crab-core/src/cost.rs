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

/// The block starts the network stack or needs it (Wi-Fi, Ethernet, MQTT, Sparkplug, NTP, Modbus TCP).
pub fn needs_network(kind: &str) -> bool {
    matches!(kind, "wifi" | "ethernet" | "ntp")
        || kind.starts_with("mqtt")
        || kind.starts_with("sparkplug")
        || kind.starts_with("modbus_tcp")
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
        | "hysteresis" | "pid" | "int_to_float" | "float_to_int" | "bytes_to_dword"
        | "bytes_to_word" | "dword_to_bytes" | "word_to_bytes" => Family::Float,
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
        "modbus_tag" | "modbus_device" | "modbus_rtu_master" | "modbus_rtu_slave"
        | "modbus_tcp_master" | "modbus_tcp_slave" | "mqtt" | "mqtt_publish" | "mqtt_subscribe"
        | "sparkplug_metric" | "sparkplug_node" | "wifi" | "ethernet" | "ntp" => Family::Net,
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

/// Heap the master keeps free for itself after a module loads (`HEAP_RESERVE` of the firmware):
/// a module that would leave less is refused.
pub const MASTER_RESERVE: u32 = 16 * 1024;

/// Heap a module takes on the board, bytes: what the master allocates to load it, not what the
/// image weighs in flash. From the `crablogic bench` stands (`specs/module-ram`): the module heap
/// of the reports `bench/esp32-0.1.0-2026-11-14-1746.md` and `bench/esp32c3-0.1.0-2026-11-14-1757.md`
/// (`из них код`: an empty stand, and the slope of the 10-, 100- and 500-block stands), and the
/// per-sheet heap steps of the "Листов" and "Проект на всю плату" stands of 15.11.2026
/// (`slot`, `first_load`). An ESTIMATE for the check panel: the linear memory of a module
/// (`crab.mem`, strings) is known exactly at build and is added on top, not guessed here.
#[derive(Debug, Clone, Copy)]
pub struct ChipHeap {
    /// Loading a module with no blocks (WAMR module and instance, execution environment, host).
    pub load: u32,
    /// What the master adds for every loaded module beyond the load itself (found by the heap
    /// steps of the sheet stands).
    pub slot: u32,
    /// Extra heap of the very first module after boot (not seen on the C3).
    pub first_load: u32,
    pub logic: u32,
    pub float: u32,
    pub timers: u32,
    pub text: u32,
    /// Digital pins and ADC.
    pub gpio: u32,
    pub i2c: u32,
    pub uart: u32,
    pub retain: u32,
    /// Wi-Fi, MQTT, Sparkplug, Ethernet, NTP (the heap of the stack itself comes on top at run time).
    pub net: u32,
    /// Modbus blocks.
    pub modbus: u32,
    /// Not measured: a conservative guess.
    pub other: u32,
    /// Free heap of a freshly booted master with no modules (bench 15.11.2026, "Листов" stands).
    pub fresh_free: u32,
    /// Heap the network stack takes while it runs (Wi-Fi, MQTT, Ethernet, NTP, Modbus TCP);
    /// 0 where it is not measured.
    pub net_stack: u32,
}

/// ESP32. Empty stand 11 540 B minus its generator and output pin; slope of `logic` 46.15 B per
/// block, `timers` 134.2; sheet of the mix 50/30/20 costs 13.97 KB, first module 24.58 KB.
pub const ESP32_HEAP: ChipHeap = ChipHeap {
    load: 11_370,
    slot: 1_780,
    first_load: 10_600,
    logic: 46,
    float: 46,
    timers: 134,
    text: 46,
    gpio: 40,
    i2c: 90,
    uart: 200,
    retain: 130,
    net: 360,
    modbus: 55,
    other: 100,
    fresh_free: 209_000,
    net_stack: 85_000,
};

/// ESP32-C3. Empty stand 11 464 B; sheet of the mix 13.67 KB. The Modbus TCP stand took 3.5 KB
/// more than on the ESP32 (bench 14.11.2026), hence its higher `modbus`.
pub const ESP32_C3_HEAP: ChipHeap = ChipHeap {
    load: 11_300,
    slot: 1_550,
    first_load: 0,
    logic: 46,
    float: 46,
    timers: 134,
    text: 46,
    gpio: 40,
    i2c: 88,
    uart: 206,
    retain: 131,
    net: 360,
    modbus: 350,
    other: 100,
    fresh_free: 88_000,
    net_stack: 0,
};

/// Heap costs for the chip of a project; an unknown chip gets the ESP32 table, S3 too (not measured).
pub fn heap_for(chip: Option<Chip>) -> &'static ChipHeap {
    match chip {
        Some(Chip::Esp32C3 | Chip::Esp32C3SuperMini) => &ESP32_C3_HEAP,
        _ => &ESP32_HEAP,
    }
}

impl ChipHeap {
    /// Estimated heap of one block of `kind`, bytes (the linear memory of text blocks is separate).
    pub fn block_bytes(&self, kind: &str) -> u32 {
        if kind.starts_with("modbus_") {
            return self.modbus;
        }
        match family_of(kind) {
            Family::Logic => self.logic,
            Family::Float => self.float,
            Family::Timers => self.timers,
            Family::Text => self.text,
            Family::GpioIn | Family::GpioOut | Family::Adc => self.gpio,
            Family::I2c => self.i2c,
            Family::Uart => self.uart,
            Family::Retain => self.retain,
            Family::Net => self.net,
            Family::Free => 0,
            Family::Other => self.other,
        }
    }

    /// Heap a module takes: its load, the master's per-module part, its blocks and `memory`
    /// (`crab.mem` of the built module, 0 if it has none).
    pub fn module_bytes<'a>(&self, kinds: impl IntoIterator<Item = &'a str>, memory: u32) -> u32 {
        let blocks: u32 = kinds.into_iter().map(|k| self.block_bytes(k)).sum();
        self.load + self.slot + blocks + memory
    }

    /// Heap a project may use: the fresh heap less the network stack (when the project has
    /// network blocks) and the master's reserve.
    pub fn budget(&self, network: bool) -> u32 {
        let stack = if network { self.net_stack } else { 0 };
        self.fresh_free
            .saturating_sub(stack)
            .saturating_sub(MASTER_RESERVE)
    }

    /// Heap of a whole project from the heap of its modules ([`ChipHeap::module_bytes`] of each).
    pub fn project_bytes(&self, modules: &[u32]) -> u32 {
        self.first_load + modules.iter().sum::<u32>()
    }

    /// How many more blocks of `kind` fit into `free` bytes of heap without a new module, keeping
    /// the master's reserve. A kind with no heap of its own (web widgets) fits without limit.
    pub fn blocks_fitting(&self, free: u32, kind: &str) -> u32 {
        match self.block_bytes(kind) {
            0 => u32::MAX,
            each => free.saturating_sub(MASTER_RESERVE) / each,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_the_fresh_heap_less_the_network_and_the_reserve() {
        let h = &ESP32_HEAP;
        assert_eq!(h.budget(false), h.fresh_free - MASTER_RESERVE);
        assert_eq!(h.budget(true), h.fresh_free - h.net_stack - MASTER_RESERVE);
        assert!(needs_network("mqtt_publish") && needs_network("wifi"));
        assert!(!needs_network("and") && !needs_network("modbus_rtu_master"));
    }

    #[test]
    fn heap_of_a_module_is_load_slot_blocks_and_memory() {
        let h = &ESP32_HEAP;
        assert_eq!(h.module_bytes([], 0), h.load + h.slot);
        assert_eq!(
            h.module_bytes(["and", "timer_on", "num_to_text"], 600),
            h.load + h.slot + h.logic + h.timers + h.text + 600
        );
        assert_eq!(h.block_bytes("web_label"), 0);
        assert_eq!(h.block_bytes("modbus_tag"), h.modbus);
        assert!(h.block_bytes("no_such_block") >= h.logic);
        assert_eq!(h.project_bytes(&[100, 200]), h.first_load + 300);
        assert!(heap_for(Some(Chip::Esp32C3)).modbus > h.modbus);
    }

    #[test]
    fn blocks_fitting_keeps_the_master_reserve() {
        let h = &ESP32_HEAP;
        assert_eq!(h.blocks_fitting(MASTER_RESERVE, "and"), 0);
        assert_eq!(h.blocks_fitting(MASTER_RESERVE + 460, "and"), 10);
        assert_eq!(h.blocks_fitting(0, "and"), 0);
        assert_eq!(h.blocks_fitting(1000, "web_label"), u32::MAX);
    }

    #[test]
    fn chips_differ_and_unknown_is_conservative() {
        assert!(ESP32.block_us("and") < ESP32_C3.block_us("and"));
        assert!(ESP32.block_us("timer_on") > ESP32.block_us("and"));
        assert!(ESP32.block_us("no_such_block") >= ESP32.block_us("and"));
        assert_eq!(ESP32.block_us("web_label"), 0.0);
    }
}
