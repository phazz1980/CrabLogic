//! Firmware code of the simple built-in blocks with hardware: pins, sensors, flash and the
//! PCF8574 expanders. Those that are plain logic (`dms`, `decoder`, `encoder`, `flash_store`)
//! are simulated; the rest have no simulation.

use crate::{b, Behavior, Ctx};

/// Behaviour of a block of this file, or `None` when it has no simulation.
pub fn behavior(kind: &str) -> Option<Box<dyn Behavior>> {
    match kind {
        "dms" => Some(Box::new(Dms)),
        "decoder" => Some(Box::new(Decoder)),
        "encoder" => Some(Box::new(Encoder)),
        "flash_store" => Some(Box::<FlashStore>::default()),
        "cycle_report" => Some(Box::new(CycleReport)),
        "digital_in" => Some(Box::new(DigitalIn)),
        "analog_in" => Some(Box::new(AnalogIn)),
        "digital_out" => Some(Box::<DigitalOut>::default()),
        "pwm_out" => Some(Box::<PwmOut>::default()),
        "dht22" => Some(Box::<Dht22>::default()),
        "ds18b20" => Some(Box::<Ds18b20>::default()),
        "wifi" => Some(Box::new(crate::netsim::Wifi)),
        "ethernet" => Some(Box::new(crate::netsim::Ethernet)),
        "mqtt" => Some(Box::new(crate::netsim::Mqtt)),
        "modbus_master_read" => Some(Box::new(crate::netsim::ModbusMasterRead)),
        "uart_read" => Some(Box::<crate::netsim::UartRead>::default()),
        "modbus_read" => Some(Box::new(crate::netsim::ModbusRead)),
        "modbus_write" => Some(Box::<crate::netsim::ModbusWrite>::default()),
        _ => None,
    }
}

/// Demultiplexer: `in` goes to the output chosen by `sel`, the others are zero (empty).
struct Dms;

impl Behavior for Dms {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let idx = c.input(0) as i64;
        let count = (c.param(0) as usize).clamp(0, 16).min(out.len());
        for (i, o) in out.iter_mut().take(count).enumerate() {
            *o = if idx == i as i64 { c.input(1) } else { 0.0 };
        }
    }

    fn step_text(&mut self, c: &Ctx, _: &mut [f64], text: &mut [String]) {
        let idx = c.input(0) as i64;
        let count = (c.param(0) as usize).clamp(0, 16).min(text.len());
        for (i, t) in text.iter_mut().take(count).enumerate() {
            *t = if idx == i as i64 {
                c.text(1).to_owned()
            } else {
                String::new()
            };
        }
    }
}

/// Decoder: output `o<n>` is set while `in` equals `n`.
struct Decoder;

impl Behavior for Decoder {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let count = (c.param(0) as usize).clamp(0, 16).min(out.len());
        for (i, o) in out.iter_mut().take(count).enumerate() {
            *o = b(c.input(0) == i as f64);
        }
    }
}

/// Encoder: the lowest set input gives `out`, `any` tells that some input is set.
struct Encoder;

impl Behavior for Encoder {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let count = (c.param(0) as usize).clamp(0, 16);
        let first = (0..count).find(|i| c.flag(*i));
        out[0] = first.map_or(0.0, |i| i as f64);
        out[1] = b(first.is_some());
    }
}

/// Value kept in flash: loaded at start, saved on the rising edge of `save`.
#[derive(Default)]
struct FlashStore {
    started: bool,
    prev: bool,
}

impl Behavior for FlashStore {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        if !self.started {
            self.started = true;
            out[0] = c.param(0);
        }
        if c.flag(1) && !self.prev && c.input(0) != out[0] {
            out[0] = c.input(0);
        }
        self.prev = c.flag(1);
    }
}

/// Only reports to the debugger; nothing to compute.
struct CycleReport;

impl Behavior for CycleReport {
    fn step(&mut self, _: &Ctx, _: &mut [f64]) {}
}

/// Digital input: the level of the pin on the bench (a floating pin reads low, or high
/// with the pull-up), inverted when asked. Params: pin, pullup, inverted.
struct DigitalIn;

impl Behavior for DigitalIn {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let level = c.bench.level(c.param(0) as i64).unwrap_or(c.param(1));
        out[0] = b((level != 0.0) != (c.param(2) != 0.0));
    }
}

/// Analog input: the counts set on the pin, kept inside the 12-bit range. Params: pin.
struct AnalogIn;

impl Behavior for AnalogIn {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let counts = c.bench.level(c.param(0) as i64).unwrap_or(0.0);
        out[0] = counts.round().clamp(0.0, 4095.0);
    }
}

/// Whether an output block may write now: its `en` input exists and is off means hold.
fn may_write(c: &Ctx) -> bool {
    !c.active.get(1).copied().unwrap_or(false) || c.flag(1)
}

/// Digital output. Inputs: in, en. Params: pin, inverted.
#[derive(Default)]
struct DigitalOut {
    logic: bool,
    inverted: bool,
}

impl Behavior for DigitalOut {
    fn step(&mut self, c: &Ctx, _: &mut [f64]) {
        self.inverted = c.param(1) != 0.0;
        if may_write(c) {
            self.logic = c.flag(0);
        }
    }

    fn drive(&self) -> Option<f64> {
        Some(b(self.logic != self.inverted))
    }
}

/// PWM output: the duty in percent, kept in 0..100. Inputs: in, en. Params: pin.
#[derive(Default)]
struct PwmOut {
    duty: f64,
}

impl Behavior for PwmOut {
    fn step(&mut self, c: &Ctx, _: &mut [f64]) {
        if may_write(c) {
            self.duty = c.input(0).clamp(0.0, 100.0);
        }
    }

    fn drive(&self) -> Option<f64> {
        Some(self.duty)
    }
}

/// The sensor is read once per period; a failed read keeps the last values and drops `ok`.
#[derive(Default)]
struct Reader {
    last_ms: Option<u64>,
}

impl Reader {
    fn due(&mut self, now_ms: u64, period_ms: f64) -> bool {
        let due = self
            .last_ms
            .is_none_or(|t| now_ms.saturating_sub(t) as f64 >= period_ms);
        if due {
            self.last_ms = Some(now_ms);
        }
        due
    }
}

/// DHT22. Outputs: temp, humidity, ok. Params: pin, period.
#[derive(Default)]
struct Dht22 {
    reader: Reader,
}

impl Behavior for Dht22 {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        if !self.reader.due(c.now_ms, c.param(1)) {
            return;
        }
        let pin = c.param(0) as i64;
        let (temp, bad_t) = c.bench.read(pin, "temp", c.now_ms);
        let (humidity, bad_h) = c.bench.read(pin, "humidity", c.now_ms);
        let ok = !(bad_t || bad_h);
        if ok {
            out[0] = temp;
            out[1] = humidity;
        }
        out[2] = b(ok);
    }
}

/// DS18B20; the channel is `temp<index>` because several sensors share a bus pin.
/// Outputs: temp, ok. Params: pin, index, period.
#[derive(Default)]
struct Ds18b20 {
    reader: Reader,
}

/// Name of the bench channel of the DS18B20 with this index on its bus.
pub fn ds18b20_channel(index: i64) -> String {
    format!("temp{index}")
}

impl Behavior for Ds18b20 {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        if !self.reader.due(c.now_ms, c.param(2)) {
            return;
        }
        let channel = ds18b20_channel(c.param(1) as i64);
        let (temp, bad) = c.bench.read(c.param(0) as i64, &channel, c.now_ms);
        if !bad {
            out[0] = temp;
        }
        out[1] = b(!bad);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(b: &mut dyn Behavior, inputs: &[f64], initial: f64, out: &mut [f64]) {
        let ctx = Ctx {
            now_ms: 0,
            inputs,
            active: &[true, true],
            params: &[initial],
            texts: &[],
            param_texts: &[],
            bench: &crate::NO_BENCH,
        };
        b.step(&ctx, out);
    }

    #[test]
    fn flash_store_loads_initial_and_saves_on_the_edge() {
        let mut store = behavior("flash_store").unwrap();
        let mut out = [0.0];
        step(&mut *store, &[5.0, 0.0], 2.5, &mut out);
        assert_eq!(out[0], 2.5);
        step(&mut *store, &[5.0, 1.0], 2.5, &mut out);
        assert_eq!(out[0], 5.0);
        // Held `save` does not write again.
        step(&mut *store, &[7.0, 1.0], 2.5, &mut out);
        assert_eq!(out[0], 5.0);
        step(&mut *store, &[7.0, 0.0], 2.5, &mut out);
        step(&mut *store, &[7.0, 1.0], 2.5, &mut out);
        assert_eq!(out[0], 7.0);
    }

    #[test]
    fn expander_blocks_have_no_simulation() {
        for kind in ["pcf8574_in", "pcf8574_out"] {
            assert!(behavior(kind).is_none(), "{kind}");
        }
    }

    /// One scan of a block at `now_ms` on `bench`.
    fn scan(
        b: &mut dyn Behavior,
        bench: &crate::Bench,
        now_ms: u64,
        inputs: &[f64],
        params: &[f64],
        out: &mut [f64],
    ) {
        let ctx = Ctx {
            now_ms,
            inputs,
            active: &[true, false],
            params,
            texts: &[],
            param_texts: &[],
            bench,
        };
        b.step(&ctx, out);
    }

    #[test]
    fn digital_input_follows_pullup_and_inversion() {
        let mut bench = crate::Bench::new();
        let mut b = behavior("digital_in").unwrap();
        let mut out = [0.0];
        // Floating, no pull-up: low. With the pull-up: high; inverted: low.
        scan(&mut *b, &bench, 0, &[], &[9.0, 0.0, 0.0], &mut out);
        assert_eq!(out[0], 0.0);
        scan(&mut *b, &bench, 0, &[], &[9.0, 1.0, 1.0], &mut out);
        assert_eq!(out[0], 0.0);
        // The button pulls the pin to ground.
        bench.set_level(9, 0.0);
        scan(&mut *b, &bench, 0, &[], &[9.0, 1.0, 1.0], &mut out);
        assert_eq!(out[0], 1.0);
    }

    #[test]
    fn analog_input_is_counts_within_twelve_bits() {
        let mut bench = crate::Bench::new();
        let mut b = behavior("analog_in").unwrap();
        let mut out = [0.0];
        bench.set_level(34, 5000.0);
        scan(&mut *b, &bench, 0, &[], &[34.0], &mut out);
        assert_eq!(out[0], 4095.0);
        bench.set_level(34, 1234.4);
        scan(&mut *b, &bench, 0, &[], &[34.0], &mut out);
        assert_eq!(out[0], 1234.0);
    }

    #[test]
    fn sensor_reads_once_per_period_and_keeps_values_on_fault() {
        let mut bench = crate::Bench::new();
        bench.set_signal(
            4,
            "temp0",
            crate::Signal {
                from: 10.0,
                to: 20.0,
                duration_ms: 10_000,
                start_ms: 0,
                fault: false,
            },
        );
        let mut b = behavior("ds18b20").unwrap();
        let mut out = [0.0; 2];
        let params = [4.0, 0.0, 1000.0];
        scan(&mut *b, &bench, 0, &[], &params, &mut out);
        assert_eq!(out, [10.0, 1.0]);
        // Within the period nothing is read.
        scan(&mut *b, &bench, 500, &[], &params, &mut out);
        assert_eq!(out[0], 10.0);
        scan(&mut *b, &bench, 5000, &[], &params, &mut out);
        assert_eq!(out[0], 15.0);
        let mut broken = bench.signal(4, "temp0");
        broken.fault = true;
        bench.set_signal(4, "temp0", broken);
        scan(&mut *b, &bench, 6000, &[], &params, &mut out);
        assert_eq!(out, [15.0, 0.0]);
    }

    #[test]
    fn outputs_leave_their_pin_state() {
        let bench = crate::Bench::new();
        let mut out = [];
        let mut d = behavior("digital_out").unwrap();
        scan(&mut *d, &bench, 0, &[1.0, 0.0], &[2.0, 1.0], &mut out);
        assert_eq!(d.drive(), Some(0.0));
        let mut p = behavior("pwm_out").unwrap();
        scan(&mut *p, &bench, 0, &[150.0, 0.0], &[2.0], &mut out);
        assert_eq!(p.drive(), Some(100.0));
    }
}
