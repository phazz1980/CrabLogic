//! Simulation of the network and Modbus blocks: the link, the broker and the slaves come
//! from the virtual bench, so the scheme sees an outage the way it would on the board.

use crate::{b, Behavior, Ctx};

/// Address the access point of the board has in the simulation.
const AP_ADDRESS: &str = "192.168.4.1";
/// Address a DHCP client gets in the simulation.
const DHCP_ADDRESS: &str = "192.168.1.100";

/// Wi-Fi client and/or access point. Outputs: `ready`, `ap_ready`, `address`, `status`, `error`.
pub struct Wifi;

impl Behavior for Wifi {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let mut text = vec![String::new(); out.len()];
        self.step_text(c, out, &mut text);
    }

    fn step_text(&mut self, c: &Ctx, out: &mut [f64], text: &mut [String]) {
        let mode = c.param(0) as i64;
        let client = mode != 1;
        let ap = mode != 0;
        // The `En` input is only there when it is switched on.
        let on = !c.active.first().copied().unwrap_or(false) || c.flag(0);
        let up = on && client && !c.bench.offline();
        let address = if up {
            if c.param(3) != 0.0 {
                DHCP_ADDRESS
            } else {
                c.param_text(4)
            }
        } else if on && ap && !client {
            AP_ADDRESS
        } else {
            ""
        };
        let status = if !on {
            0.0
        } else if up || (ap && !client) {
            3.0
        } else {
            2.0
        };
        let error = if on && client && c.bench.offline() {
            1.0
        } else {
            0.0
        };
        let values = [b(up), b(on && ap), 0.0, status, error];
        for (o, v) in out.iter_mut().zip(values) {
            *o = v;
        }
        if let Some(t) = text.get_mut(2) {
            *t = address.to_owned();
        }
    }
}

/// Ethernet: `ready` while the cable is in.
pub struct Ethernet;

impl Behavior for Ethernet {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        if let Some(o) = out.first_mut() {
            *o = b(!c.bench.offline());
        }
    }
}

/// MQTT client: connected while the network is up and the broker answers.
pub struct Mqtt;

impl Behavior for Mqtt {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let (connected, error) = if c.bench.offline() {
            (false, 15.0)
        } else if c.bench.broker_down() {
            (false, 7.0)
        } else {
            (true, 0.0)
        };
        if out.len() >= 2 {
            out[0] = b(connected);
            out[1] = error;
        }
    }
}

/// Read from a UART port: what was sent into the port on the bench comes as a line with a
/// `ready` pulse, or byte by byte (one per scan, `new` on each).
#[derive(Default)]
pub struct UartRead {
    seen: u64,
    pending: std::collections::VecDeque<u8>,
    bytes: bool,
}

impl Behavior for UartRead {
    // The simulation calls `step` and then `step_text`; the state moves in the latter only.
    fn step(&mut self, _: &Ctx, _: &mut [f64]) {}

    fn step_text(&mut self, c: &Ctx, out: &mut [f64], text: &mut [String]) {
        let bytes = c.param(1) != 0.0;
        let (serial, sent) = c.bench.uart_rx(c.param(0) as i64);
        if bytes != self.bytes {
            self.bytes = bytes;
            self.pending.clear();
        }
        let fresh = serial != self.seen;
        self.seen = serial;
        let ready = fresh && !bytes;
        if fresh && bytes {
            self.pending.extend(sent.bytes());
        }
        let next = if bytes {
            self.pending.pop_front()
        } else {
            None
        };
        // The outputs follow the mode: `line`, `ready`, `available` or `byte`, `new`,
        // `available`.
        if out.len() >= 3 {
            if bytes {
                out[1] = b(next.is_some());
                if let Some(byte) = next {
                    out[0] = f64::from(byte);
                }
            } else {
                out[1] = b(ready);
            }
            out[2] = self.pending.len() as f64;
        }
        if ready {
            if let Some(t) = text.first_mut() {
                *t = sent.to_owned();
            }
        }
    }
}

/// The device the Modbus slave blocks of the board belong to: its tables live on the bench
/// under this number, where the master (set by hand) and the program meet.
pub const OWN_SLAVE: i64 = 0;

/// Where a `modbus_tag` sits in the simulation, from its owner and the links of the tag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TagLink {
    /// Unit ID of the device on the bench, or [`OWN_SLAVE`] for the slave role of this board.
    pub unit: i64,
    /// The owner is a `modbus_device` polled by a master (it can be silent and sends `ok`).
    pub device: bool,
    /// An input is connected: the tag writes its value, else it reads the bench.
    pub write: bool,
}

/// One register or bit of a Modbus table (`modbus_tag`). Parameters, in block order: `device`,
/// `table`, `kind`, `reg`, `swap`; inputs `in`, `in_f`, `in_b`; outputs `out`, `out_f`, `out_b`, `ok`.
///
/// Read: the value from the bench; while a device is silent `ok` is off and the last value stays,
/// as in the firmware. Write: the value is handed to the bench register (see `drive`).
#[derive(Default)]
pub struct ModbusTag {
    link: TagLink,
    value: f64,
}

impl ModbusTag {
    pub fn new(link: TagLink) -> Self {
        Self { link, value: 0.0 }
    }

    /// The device answers (the slave role of this board always does).
    fn answers(&self, bench: &crate::Bench) -> bool {
        !self.link.device || !bench.silent(self.link.unit)
    }
}

impl Behavior for ModbusTag {
    fn step(&mut self, c: &Ctx, out: &mut [f64]) {
        let ok = self.answers(c.bench);
        if self.link.write {
            // The inputs are `in`, `in_f` and `in_b`: the one of the kind counts.
            self.value = match c.param_text(2) {
                "bool" => b(c.flag(2)),
                "float" => c.input(1),
                _ => c.input(0).round(),
            };
        } else if ok && out.len() >= 3 {
            let v = c
                .bench
                .register(self.link.unit, c.param_text(1), c.param(3) as i64);
            out[0] = v.round();
            out[1] = v;
            out[2] = b(v != 0.0);
        }
        if self.link.device && out.len() >= 4 {
            out[3] = b(ok);
        }
    }

    fn drive(&self) -> Option<f64> {
        self.link.write.then_some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bench;

    fn ctx<'a>(
        bench: &'a Bench,
        params: &'a [f64],
        param_texts: &'a [String],
        active: &'a [bool],
        inputs: &'a [f64],
    ) -> Ctx<'a> {
        Ctx {
            now_ms: 0,
            inputs,
            active,
            params,
            texts: &[],
            param_texts,
            bench,
        }
    }

    #[test]
    fn wifi_follows_the_link() {
        let mut bench = Bench::new();
        let (mut out, mut text) = ([0.0; 5], vec![String::new(); 5]);
        // Client with DHCP, no `En` input.
        let params = [0.0, 0.0, 0.0, 1.0, 0.0];
        let texts = vec![String::new(); 5];
        let c = ctx(&bench, &params, &texts, &[], &[]);
        Wifi.step(&c, &mut out);
        Wifi.step_text(&c, &mut out, &mut text);
        assert_eq!((out[0], out[3], out[4]), (1.0, 3.0, 0.0));
        assert_eq!(text[2], DHCP_ADDRESS);
        bench.set_offline(true);
        let c = ctx(&bench, &params, &texts, &[], &[]);
        Wifi.step(&c, &mut out);
        Wifi.step_text(&c, &mut out, &mut text);
        assert_eq!((out[0], out[3], out[4]), (0.0, 2.0, 1.0));
        assert_eq!(text[2], "");
    }

    #[test]
    fn wifi_en_input_switches_the_radio_off() {
        let bench = Bench::new();
        let mut out = [0.0; 5];
        let params = [0.0, 0.0, 0.0, 1.0, 0.0];
        let texts = vec![String::new(); 5];
        let c = ctx(&bench, &params, &texts, &[true], &[0.0]);
        Wifi.step(&c, &mut out);
        assert_eq!((out[0], out[3]), (0.0, 0.0));
    }

    #[test]
    fn mqtt_needs_the_link_and_the_broker() {
        let mut bench = Bench::new();
        let mut out = [0.0; 2];
        Mqtt.step(&ctx(&bench, &[], &[], &[], &[]), &mut out);
        assert_eq!(out, [1.0, 0.0]);
        bench.set_broker_down(true);
        Mqtt.step(&ctx(&bench, &[], &[], &[], &[]), &mut out);
        assert_eq!(out, [0.0, 7.0]);
        bench.set_offline(true);
        Mqtt.step(&ctx(&bench, &[], &[], &[], &[]), &mut out);
        assert_eq!(out, [0.0, 15.0]);
    }

    #[test]
    fn modbus_tag_read_gives_the_register_and_keeps_it_when_silent() {
        let mut bench = Bench::new();
        bench.set_register(3, "input", 7, 21.5);
        // device, table, kind, reg, swap
        let params = [0.0, 0.0, 0.0, 7.0, 0.0];
        let texts = vec![
            String::new(),
            "input".to_owned(),
            String::new(),
            String::new(),
            String::new(),
        ];
        let link = TagLink {
            unit: 3,
            device: true,
            write: false,
        };
        let mut tag = ModbusTag::new(link);
        let mut out = [0.0; 4];
        tag.step(&ctx(&bench, &params, &texts, &[], &[]), &mut out);
        assert_eq!(out, [22.0, 21.5, 1.0, 1.0]);
        assert_eq!(tag.drive(), None);
        bench.set_silent(3, true);
        bench.set_register(3, "input", 7, 99.0);
        tag.step(&ctx(&bench, &params, &texts, &[], &[]), &mut out);
        assert_eq!(out, [22.0, 21.5, 1.0, 0.0]);
    }

    #[test]
    fn modbus_tag_write_drives_the_value_of_its_kind() {
        let bench = Bench::new();
        let params = [0.0, 0.0, 0.0, 7.0, 0.0];
        let texts = vec![
            String::new(),
            "holding".to_owned(),
            "float".to_owned(),
            String::new(),
            String::new(),
        ];
        let link = TagLink {
            unit: OWN_SLAVE,
            device: false,
            write: true,
        };
        let mut tag = ModbusTag::new(link);
        let mut out = [0.0; 4];
        // `in`, `in_f`, `in_b`: the float input is the one that counts.
        tag.step(
            &ctx(&bench, &params, &texts, &[], &[9.0, 2.5, 1.0]),
            &mut out,
        );
        assert_eq!(tag.drive(), Some(2.5));
        assert_eq!(out, [0.0; 4], "a write leaves the outputs alone");
    }
}
