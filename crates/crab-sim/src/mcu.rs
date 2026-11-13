//! Host ABI v4: modules that run on the PC in wasmi and on the board in the master firmware.
//!
//! Values are 32 bit: `bool` and `int` travel as `i32`, `float` as `f32` (see `specs/numeric-model`).
//! Exports of a module: `abi_version() -> i32` (must be 4), `step(dt_ms: i32)`; optional
//! `init()`, `alloc(len: i32) -> i32` (where the host puts message bytes) and
//! `on_message(topic_ptr, topic_len, ptr, len)`.
//!
//! The imports, their signatures and permissions are one list in the kernel
//! ([`crab_kernel::abi::IMPORTS`]); the kernel runs the module ([`crab_kernel::wasmi`]) and does
//! everything that does not touch the hardware. On the PC the hardware is a stub ([`StubIo`])
//! that tests and the editor fill and read.

use std::collections::HashMap;

use crab_kernel::abi::{self, id};
use crab_kernel::engine::{Engine, Instance, Limits, Run};
use crab_kernel::host::{Devices, Mem, ModuleHost, Val};
use crab_kernel::kernel::{deliver_message, step_to_end};
use crab_kernel::lease::NoHal;
use crab_kernel::wasmi::{Wasmi, WasmiInstance};
use serde::Deserialize;

pub use crab_kernel::abi::ABI_VERSION;
pub use crab_kernel::bus::{TopicDecl, TopicKind};
pub use crab_kernel::Word;

/// `module.toml` next to the `.wasm`: name and the hardware imports the module may use.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_abi")]
    pub abi: i32,
    #[serde(default)]
    pub allow: Vec<String>,
    /// Linear memory the module needs, in 64 KiB pages (0 when it has none or declares it itself).
    #[serde(default)]
    pub memory_pages: u32,
    /// Topics the module declares: kind and size limit (`specs/pubsub-bus`).
    #[serde(default)]
    pub topics: Vec<TopicDecl>,
    /// How many event messages may wait for the module; 0 means the default.
    #[serde(default)]
    pub queue: usize,
}

fn default_abi() -> i32 {
    ABI_VERSION
}

impl Manifest {
    /// Parses `module.toml`; unknown names in `allow` are errors.
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self =
            toml::from_str(text).map_err(|e| format!("манифест модуля не разобран: {e}"))?;
        if manifest.abi != ABI_VERSION {
            return Err(format!(
                "манифест требует ABI {}, поддерживается {ABI_VERSION}: пересоберите модуль",
                manifest.abi
            ));
        }
        abi::check_allow(&manifest.allow)?;
        Ok(manifest)
    }
}

/// The stub hardware of the PC: what the module reads is set by the caller, what it writes
/// is kept for the caller to read.
#[derive(Debug, Clone, Default)]
pub struct StubIo {
    /// `crab.bus` calls of driver blocks, `[op, a, b, c, d]` in order.
    pub bus_calls: Vec<[i32; 5]>,
    /// Bytes an I²C read (`bus` op 22) puts into the module memory; the answer is their count.
    pub i2c_in: Vec<u8>,
    /// Registers of an I²C chip for `bus` op 23 (write the register number, read from it on):
    /// the read gives `i2c_regs[reg..]`. Writes (op 21) succeed while this is not empty.
    pub i2c_regs: Vec<u8>,
    /// Level lengths, µs, a pulse measurement (`bus` op 65) gives; the answer is their count.
    pub pulse_in: Vec<u16>,
    /// Display chips on the I²C bus: writes (op 21) to their address go to the model, which keeps
    /// the picture (see [`crate::display`]); the write answers with the byte count, a missing
    /// address with -1 (no acknowledge).
    pub displays: Vec<crate::display::DisplayDevice>,
    pub gpio_in: HashMap<i32, i32>,
    pub adc_in: HashMap<i32, i32>,
    pub gpio_out: HashMap<i32, i32>,
    pub pwm_out: HashMap<i32, i32>,
    /// What the master would do with an output pin when the module fails, by pin, from bits 9-10
    /// of the pin number of `gpio_write` / `pwm_write`: 0 low (PWM off), 1 high (full), 2 hold.
    pub on_fault: HashMap<i32, i32>,
    /// What the master reports for a sensor: `(key, field)` as the module passes them
    /// (`kind | index << 4 | pin << 8`; field 0 temperature, 1 humidity, 2 read ok).
    pub sensors: HashMap<(i32, i32), f32>,
    /// What the master reports for a PCF8574: `(key, field)` as the module passes them
    /// (`address | sda << 8 | scl << 16`; field 0 the byte read, 1 the chip answered).
    pub pcf_in: HashMap<(i32, i32), i32>,
    /// The last byte the module handed over for a chip, by key.
    pub pcf_out: HashMap<i32, i32>,
    /// Words kept "in flash" by `retain`, by key.
    pub retain: HashMap<i32, i32>,
    /// Texts kept "in flash" by `retain_text`, by key.
    pub retain_text: HashMap<i32, Vec<u8>>,
    /// How many times the module wrote to "flash" (`retain` and `retain_text`, op 1).
    pub retain_writes: u32,
    /// Modbus settings the module handed over, as `(code, value)` in order (the apply call is code 63).
    pub modbus_cfg: Vec<(i32, i32)>,
    /// What a Modbus read returns, by `(slave, table | kind << 4 | master << 8, register)`; the bits of an int or float.
    pub modbus_in: HashMap<(i32, i32, i32), i32>,
    /// What a module wrote, by the same key.
    pub modbus_out: HashMap<(i32, i32, i32), i32>,
    /// Status of a master's slave by address (-1: error of our slave).
    pub modbus_status: HashMap<i32, i32>,
    /// UART settings the module handed over, as `(port, code, value)` in order (code 63 opens).
    pub uart_cfg: Vec<(i32, i32, i32)>,
    /// The last frame format the module asked for, by port (`8E1`).
    pub uart_format: HashMap<i32, String>,
    /// Bytes waiting in a port's receive buffer, by port.
    pub uart_rx: HashMap<i32, std::collections::VecDeque<u8>>,
    /// What the module sent, by port, as `(value, size code)`.
    pub uart_tx: HashMap<i32, Vec<(i32, i32)>>,
    /// Speed changes the module asked for, by port.
    pub uart_speed: HashMap<i32, i32>,
    /// Text the module sent (op 6), by port.
    pub uart_text: HashMap<i32, Vec<u8>>,
    /// Words of the inputs and parameters the module handed to a native part, by `(kind, inst)`
    /// (the last call); `native_words` says how many to read for a kind.
    pub native_in: HashMap<(i32, i32), Vec<i32>>,
    /// Number of input words (inputs and parameters) a native part of `kind` takes.
    pub native_words: HashMap<i32, usize>,
    /// What a native part answers, by `(kind, inst)`: the words of its outputs. Without an
    /// entry the call answers 0 (the master cannot run the part) and writes nothing.
    pub native_out: HashMap<(i32, i32), Vec<i32>>,
    /// Network numbers the module set (`net` op 0), by code.
    pub net_num: HashMap<i32, i32>,
    /// Network text settings (`net` op 1), by code.
    pub net_text: HashMap<i32, String>,
    /// The radio request of the last step: 0 off, 1 client, 2 access point, 3 both.
    pub net_radio: i32,
    /// What the master reports for Wi-Fi (`net` op 3), by field: 0 client ready, 1 AP ready, 2 status, 3 error, 4 address.
    pub net_wifi: HashMap<i32, i32>,
    /// Ethernet numbers the module set (`net` op 6), by code: 1 dhcp, 2 sck, 3 miso, 4 mosi, 5 cs, 6 MAC byte, 63 complete.
    pub eth_num: HashMap<i32, i32>,
    /// Ethernet text settings (`net` op 7), by code: 1 ip, 2 gateway, 3 subnet.
    pub eth_text: HashMap<i32, String>,
    /// What the master reports for Ethernet (`net` op 8), by field: 0 ready, 1 address.
    pub eth_state: HashMap<i32, i32>,
    /// MQTT numbers the module set (`net` op 9), by code: 1 port, 2 keep-alive, 3 network, 4 max subscriptions, 63 complete.
    pub mqtt_num: HashMap<i32, i32>,
    /// MQTT text settings (`net` op 10), by code: 1 broker, 2 client id, 3 user, 4 password.
    pub mqtt_text: HashMap<i32, String>,
    /// What the master reports for the client (`net` op 11), by field: 0 connected, 1 error.
    pub mqtt_state: HashMap<i32, i32>,
    /// Publications the module queued (ops 12, 13): topic, payload, retain.
    pub mqtt_published: Vec<(String, String, bool)>,
    /// Sparkplug setup (op 30): texts by code (1 group, 2 node), enable flag, setup complete.
    pub sp_text: HashMap<i32, String>,
    pub sp_enable: i32,
    pub sp_complete: bool,
    /// Sparkplug metrics asked with op 31: name, type (0 bool, 1 int, 2 float), last value bits (op 32).
    pub sp_metrics: Vec<(String, i32, u32)>,
    /// Filters the module subscribed to (op 14); the index is the slot.
    pub mqtt_filters: Vec<String>,
    /// Last message and counter of each slot (ops 15, 16).
    pub mqtt_slots: Vec<(String, i32)>,
    /// Layout JSON the module gave the web page (`net` op 17, code 1).
    pub web_layout: String,
    /// Widgets of the page (op 17, code 2); the module writes values (ops 19, 20) and reads what
    /// the page set (ops 21, 22). Tests play the page by changing `value` and `text`.
    pub web_items: Vec<WebStubItem>,
    /// Interfaces the page is served on (op 17, code 3): 1 Wi-Fi, 2 Ethernet.
    pub web_mask: i32,
    /// Password of the page (op 17, code 4).
    pub web_pass: String,
    /// The module finished the setup (op 17, code 63).
    pub web_complete: bool,
    /// Network form (op 17, code 5): the ten fields (ssid, password, dhcp, ip, gateway, subnet, ap ssid,
    /// ap password, mode, ap delay) and the widget of the form; empty until the module sets it up.
    pub web_net: Vec<String>,
    pub web_net_item: i32,
    /// The page saved new settings; read (and cleared) by op 23, code 10.
    pub web_net_changed: bool,
    /// Unix time (UTC) the NTP service knows; `None` until the first sync.
    pub net_unix: Option<u32>,
    /// Seconds since the last sync.
    pub net_age: i32,
    /// Milliseconds since the module started; advances with every step.
    pub now_ms: u32,
}

type Host = ModuleHost<StubIo>;

/// Yields one step of a module alone may make: with the default budget that is over a million
/// loop passes, far beyond any real step.
pub const MAX_YIELDS: u32 = 100;

/// A loaded ABI v4 module with its state: a kernel instance on the stub hardware.
pub struct McuModule {
    inner: WasmiInstance<Host>,
}

impl Instance<Host> for McuModule {
    fn host(&self) -> &Host {
        self.inner.host()
    }

    fn host_mut(&mut self) -> &mut Host {
        self.inner.host_mut()
    }

    fn call_step(&mut self, dt_ms: u32) -> Result<Run, String> {
        self.inner.call_step(dt_ms)
    }

    fn resume(&mut self) -> Result<Run, String> {
        self.inner.resume()
    }

    fn suspended(&self) -> bool {
        self.inner.suspended()
    }

    fn call_on_message(&mut self, topic: &str, data: &[u8]) -> Result<(), String> {
        self.inner.call_on_message(topic, data)
    }
}

impl McuModule {
    /// Checks imports against the manifest, loads the module, checks `abi_version` and runs `init`.
    pub fn load(wasm: &[u8], manifest: &Manifest, n_outputs: usize) -> Result<Self, String> {
        Self::load_with(wasm, manifest, n_outputs, StubIo::default())
    }

    /// Like [`McuModule::load`], with the stub hardware already set up (what the master reports,
    /// values kept in flash) before the module's `init` runs.
    pub fn load_with(
        wasm: &[u8],
        manifest: &Manifest,
        n_outputs: usize,
        io: StubIo,
    ) -> Result<Self, String> {
        let inner = Wasmi::new().load(
            wasm,
            &manifest.allow,
            Limits::default(),
            ModuleHost::new(io, n_outputs),
        )?;
        Ok(Self { inner })
    }

    /// One whole step of `dt_ms` milliseconds; the stub clock advances first. A module alone
    /// is resumed at once when it yields; after [`MAX_YIELDS`] yields it counts as hung.
    pub fn step(&mut self, dt_ms: u32) -> Result<(), String> {
        step_to_end(self, dt_ms, MAX_YIELDS)
    }

    /// Hands a message to the module if it subscribed to `topic`; returns whether it was delivered.
    pub fn deliver(&mut self, topic: &str, data: &[u8]) -> Result<bool, String> {
        deliver_message(self, topic, data)
    }

    pub fn set_inputs(&mut self, inputs: &[Word]) {
        self.host_mut().inputs = inputs.to_vec();
    }

    pub fn set_params(&mut self, params: &[Word]) {
        self.host_mut().params = params.to_vec();
    }

    pub fn outputs(&self) -> &[Word] {
        &self.host().outputs
    }

    pub fn tag(&self, id: i32) -> Option<Word> {
        self.host().tags.get(&id).copied()
    }

    pub fn set_tag(&mut self, id: i32, value: Word) {
        self.host_mut().tags.insert(id, value);
    }

    /// The texts of the string tags the module has written, by tag number.
    pub fn text_tags(&self) -> &std::collections::BTreeMap<i32, Vec<u8>> {
        &self.host().text_tags
    }

    pub fn set_text_tag(&mut self, id: i32, value: Vec<u8>) {
        self.host_mut().text_tags.insert(id, value);
    }

    /// The pictures of the display chips on the stub bus.
    pub fn displays(&self) -> Vec<crate::display::DisplayView> {
        self.io().displays.iter().map(|d| d.view()).collect()
    }

    pub fn io(&self) -> &StubIo {
        &self.host().devices
    }

    pub fn io_mut(&mut self) -> &mut StubIo {
        &mut self.host_mut().devices
    }

    /// Topics the module subscribed to.
    pub fn subscriptions(&self) -> &[String] {
        &self.host().subscriptions
    }

    /// Messages the module published since the last call.
    pub fn take_published(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.host_mut().published)
    }

    /// Lines the module wrote with `log` since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.host_mut().log)
    }
}

impl Devices for StubIo {
    type Hal = NoHal;

    fn hal(&self) -> NoHal {
        NoHal
    }

    fn tick(&mut self, dt_ms: u32) {
        self.now_ms = self.now_ms.wrapping_add(dt_ms);
    }

    fn call(&mut self, import: usize, args: &[Val], mut mem: Mem<'_>) -> Option<Val> {
        let a = |i: usize| args.get(i).map_or(0, |v| v.i());
        let int = |v: i32| Some(Val::I32(v));
        match import {
            // Bit 8 of the pin asks the master for the pull-up; the stub has no resistors.
            id::GPIO_READ => int(self.gpio_in.get(&(a(0) & 0xFF)).copied().unwrap_or(0)),
            id::GPIO_WRITE | id::PWM_WRITE => {
                let (pin, v) = (a(0), a(1));
                self.on_fault.insert(pin & 0xFF, pin >> 9 & 3);
                let out = if import == id::GPIO_WRITE {
                    &mut self.gpio_out
                } else {
                    &mut self.pwm_out
                };
                out.insert(pin & 0xFF, v);
                None
            }
            id::ADC_READ => int(self.adc_in.get(&a(0)).copied().unwrap_or(0)),
            id::MILLIS => int(self.now_ms as i32),
            id::SENSOR_READ => Some(Val::F32(
                self.sensors.get(&(a(0), a(2))).copied().unwrap_or(0.0),
            )),
            id::PCF_READ => int(self.pcf_in.get(&(a(0), a(2))).copied().unwrap_or(0)),
            id::PCF_WRITE => {
                self.pcf_out.insert(a(0), a(1));
                int(1)
            }
            // `retain(op, key, value)`: 0 reads (the value is the default), 1 writes.
            id::RETAIN => int(if a(0) == 0 {
                self.retain.get(&a(1)).copied().unwrap_or(a(2))
            } else {
                self.retain.insert(a(1), a(2));
                self.retain_writes += 1;
                1
            }),
            // `retain_text(op, key, ptr, n)`: 0 reads (the length, or -1 when nothing is kept), 1 writes.
            id::RETAIN_TEXT => int(self.retain_text(a(0), a(1), a(2), a(3), mem)),
            id::MODBUS => int(self.modbus(a(0), a(1), a(2), a(3), a(4))),
            id::UART => int(self.uart(a(0), a(1), a(2), a(3), mem)),
            id::NATIVE => int(self.native(a(0), a(1), a(2), a(3), mem)),
            id::NET => int(self.net([a(0), a(1), a(2), a(3), a(4)], &mut mem)),
            id::BUS => int(self.bus([a(0), a(1), a(2), a(3), a(4)], &mut mem)),
            _ => None,
        }
    }
}

impl StubIo {
    /// `crab.bus` on the PC: claims and the lock succeed, an I²C read gives `i2c_in`, the rest
    /// fails with "resource not claimed" (-1). Every call is kept in `bus_calls`.
    fn bus(&mut self, call: [i32; 5], mem: &mut Mem<'_>) -> i32 {
        self.bus_calls.push(call);
        match call[0] {
            0 | 1 => 0,
            22 => {
                let n = (call[3].max(0) as usize).min(self.i2c_in.len());
                if mem.write(call[2], &self.i2c_in[..n]) {
                    n as i32
                } else {
                    -4
                }
            }
            65 => {
                let n = (call[4].max(0) as usize).min(self.pulse_in.len());
                let bytes: Vec<u8> = self.pulse_in[..n]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect();
                if mem.write(call[3], &bytes) {
                    n as i32
                } else {
                    -4
                }
            }
            20 if !self.displays.is_empty() => 0,
            21 if self
                .displays
                .iter()
                .any(|d| i32::from(d.address) == call[1]) =>
            {
                let Some(bytes) = mem.read(call[2], call[3], 4096) else {
                    return -4;
                };
                for d in self
                    .displays
                    .iter_mut()
                    .filter(|d| i32::from(d.address) == call[1])
                {
                    d.write(&bytes);
                }
                bytes.len() as i32
            }
            21 if !self.i2c_regs.is_empty() => call[3],
            23 if !self.i2c_regs.is_empty() => {
                let Some(reg) = mem.read(call[2], 1, 1) else {
                    return -4;
                };
                let rlen = (call[3] >> 16) as usize;
                let from = usize::from(reg[0]).min(self.i2c_regs.len());
                let to = (from + rlen).min(self.i2c_regs.len());
                if mem.write(call[4], &self.i2c_regs[from..to]) {
                    (to - from) as i32
                } else {
                    -4
                }
            }
            _ => -1,
        }
    }

    fn retain_text(&mut self, op: i32, key: i32, ptr: i32, n: i32, mut mem: Mem<'_>) -> i32 {
        if op == 0 {
            let Some(text) = self.retain_text.get(&key) else {
                return -1;
            };
            let len = text.len().min(n.max(0) as usize);
            if !mem.write(ptr, &text[..len]) {
                return -1;
            }
            len as i32
        } else {
            let Some(bytes) = mem.read(ptr, n, 128) else {
                return -1;
            };
            self.retain_text.insert(key, bytes);
            self.retain_writes += 1;
            1
        }
    }

    fn modbus(&mut self, op: i32, a: i32, b: i32, reg: i32, d: i32) -> i32 {
        match op {
            0 => {
                self.modbus_cfg.push((a, b));
                0
            }
            1 => self.modbus_in.get(&(a, b, reg)).copied().unwrap_or(0),
            2 => {
                self.modbus_out.insert((a, b, reg), d);
                0
            }
            _ => self.modbus_status.get(&a).copied().unwrap_or(0),
        }
    }

    fn uart(&mut self, op: i32, port: i32, a: i32, b: i32, mem: Mem<'_>) -> i32 {
        let text = if op == 6 || op == 7 {
            mem.read(a, b, 1024)
        } else {
            None
        };
        match op {
            6 => match text {
                Some(t) => {
                    self.uart_text.entry(port).or_default().extend(t);
                    1
                }
                None => 0,
            },
            7 => {
                let f = String::from_utf8_lossy(&text.unwrap_or_default())
                    .trim()
                    .to_uppercase();
                let ok = matches!(f.as_bytes(), [d, p, s]
                    if (b'5'..=b'8').contains(d) && matches!(p, b'N' | b'E' | b'O')
                        && matches!(s, b'1' | b'2'));
                if ok {
                    self.uart_format.insert(port, f);
                }
                i32::from(ok)
            }
            0 => {
                self.uart_cfg.push((port, a, b));
                0
            }
            1 => self
                .uart_rx
                .get_mut(&port)
                .and_then(|q| q.pop_front())
                .map_or(-1, i32::from),
            2 => self.uart_rx.get(&port).map_or(0, |q| q.len() as i32),
            3 => {
                self.uart_tx.entry(port).or_default().push((a, b));
                1
            }
            4 => self.uart_speed.get(&port).copied().unwrap_or(0),
            _ => {
                self.uart_speed.insert(port, a);
                0
            }
        }
    }

    fn native(&mut self, kind: i32, inst: i32, input: i32, output: i32, mut mem: Mem<'_>) -> i32 {
        let words = self.native_words.get(&kind).copied().unwrap_or(0);
        let Some(bytes) = mem.read(input, (words * 4) as i32, 4096) else {
            return 0;
        };
        let given: Vec<i32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| i32::from_le_bytes(*w))
            .collect();
        self.native_in.insert((kind, inst), given);
        let Some(answer) = self.native_out.get(&(kind, inst)) else {
            return 0;
        };
        let raw: Vec<u8> = answer.iter().flat_map(|w| w.to_le_bytes()).collect();
        i32::from(mem.write(output, &raw))
    }

    fn net(&mut self, [op, a, b, len, d]: [i32; 5], mem: &mut Mem<'_>) -> i32 {
        let text = if op == 1 || op == 7 || op == 10 {
            mem.read(b, len, 128)
        } else {
            None
        };
        match op {
            30 => {
                match a {
                    1 | 2 => {
                        let t = mem.read(b, len, 64);
                        if let Some(t) = t {
                            self.sp_text
                                .insert(a, String::from_utf8_lossy(&t).into_owned());
                        }
                    }
                    4 => self.sp_enable = b,
                    63 => self.sp_complete = true,
                    _ => {}
                }
                return 0;
            }
            31 => {
                let name = mem.read(a, b, 32);
                let io = &mut *self;
                return match name {
                    Some(n) if !n.is_empty() => {
                        let n = String::from_utf8_lossy(&n).into_owned();
                        match io.sp_metrics.iter().position(|m| m.0 == n) {
                            Some(i) => i as i32,
                            None if io.sp_metrics.len() < 12 => {
                                io.sp_metrics.push((n, len, 0));
                                io.sp_metrics.len() as i32 - 1
                            }
                            None => -1,
                        }
                    }
                    _ => -1,
                };
            }
            32 => {
                if let Some(m) = self.sp_metrics.get_mut(a as usize) {
                    m.2 = b as u32;
                }
                return 0;
            }
            33 => return 0, // no broker on the PC: the node is never online
            _ => {}
        }
        match op {
            12 | 13 => {
                let topic = mem.read(a, b, 128);
                let payload = mem.read(len, d, 128);
                let io = &mut *self;
                return match (topic, payload) {
                    (Some(t), Some(p)) if b > 0 && io.mqtt_state.get(&0) == Some(&1) => {
                        io.mqtt_published.push((
                            String::from_utf8_lossy(&t).into_owned(),
                            String::from_utf8_lossy(&p).into_owned(),
                            op == 13,
                        ));
                        1
                    }
                    _ => 0,
                };
            }
            17 => {
                match a {
                    1 => {
                        let t = mem.read(b, len, 8191);
                        if let Some(t) = t {
                            self.web_layout = String::from_utf8_lossy(&t).into_owned();
                        }
                    }
                    2 => {
                        let t = mem.read(b, len * 16, 32 * 16);
                        if let Some(t) = t {
                            let f = |r: &[u8]| f32::from_le_bytes(r.try_into().unwrap());
                            self.web_items = t
                                .as_chunks::<16>()
                                .0
                                .iter()
                                .map(|r| WebStubItem {
                                    kind: r[0],
                                    rw: r[1] != 0,
                                    locked: false,
                                    dec: r[2],
                                    points: r[3],
                                    lo: f(&r[4..8]),
                                    hi: f(&r[8..12]),
                                    value: f(&r[12..16]),
                                    text: String::new(),
                                })
                                .collect();
                        }
                    }
                    3 => self.web_mask = b,
                    4 => {
                        let t = mem.read(b, len, 64);
                        if let Some(t) = t {
                            self.web_pass = String::from_utf8_lossy(&t).into_owned();
                        }
                    }
                    5 => {
                        let t = mem.read(b, len, 700);
                        if let Some(t) = t {
                            let io = &mut *self;
                            io.web_net = String::from_utf8_lossy(&t)
                                .split('\n')
                                .map(str::to_string)
                                .collect();
                            io.web_net_item = d;
                        }
                    }
                    63 => self.web_complete = true,
                    _ => {}
                }
                return 0;
            }
            19 => {
                if let Some(it) = self.web_items.get_mut(a as usize) {
                    it.value = f32::from_bits(b as u32);
                }
                return 0;
            }
            20 => {
                let t = mem.read(b, len, 128);
                if let (Some(t), Some(it)) = (t, self.web_items.get_mut(a as usize)) {
                    it.text = String::from_utf8_lossy(&t).into_owned();
                }
                return 0;
            }
            21 => {
                return self
                    .web_items
                    .get(a as usize)
                    .map_or(0, |it| it.value.to_bits() as i32);
            }
            22 => {
                let Some(text) = self.web_items.get(a as usize).map(|it| it.text.clone()) else {
                    return 0;
                };
                let Some(data) = mem.bytes() else {
                    return 0;
                };
                let (p, cap) = (b as usize, len.max(0) as usize);
                if cap == 0 || p + cap > data.len() {
                    return 0;
                }
                let n = text.len().min(cap - 1);
                data[p..p + n].copy_from_slice(&text.as_bytes()[..n]);
                data[p + n] = 0;
                return n as i32;
            }
            24 => {
                if let Some(it) = self.web_items.get_mut(a as usize) {
                    it.locked = b == 0;
                }
                return 0;
            }
            23 => {
                let io = &mut *self;
                return match a {
                    10 => std::mem::take(&mut io.web_net_changed) as i32,
                    11 => io.web_net.get(2).is_some_and(|v| v != "0") as i32,
                    12 => io.web_net.get(8).and_then(|v| v.parse().ok()).unwrap_or(0),
                    0..=9 => {
                        let text = io.web_net.get(a as usize).cloned().unwrap_or_default();
                        let Some(data) = mem.bytes() else {
                            return 0;
                        };
                        let (p, cap) = (b as usize, len.max(0) as usize);
                        if cap == 0 || p + cap > data.len() {
                            return 0;
                        }
                        let n = text.len().min(cap - 1);
                        data[p..p + n].copy_from_slice(&text.as_bytes()[..n]);
                        data[p + n] = 0;
                        n as i32
                    }
                    _ => 0,
                };
            }
            14 => {
                let Some(f) = mem.read(a, b, 128) else {
                    return -1;
                };
                let f = String::from_utf8_lossy(&f).into_owned();
                let io = &mut *self;
                let limit = io.mqtt_num.get(&4).copied().unwrap_or(4).clamp(1, 16) as usize;
                if let Some(i) = io.mqtt_filters.iter().position(|x| *x == f) {
                    return i as i32;
                }
                if io.mqtt_filters.len() >= limit {
                    return -1;
                }
                io.mqtt_filters.push(f);
                io.mqtt_slots.push((String::new(), 0));
                return io.mqtt_filters.len() as i32 - 1;
            }
            15 => {
                let Some((text, _)) = self.mqtt_slots.get(a as usize).cloned() else {
                    return 0;
                };
                let Some(data) = mem.bytes() else {
                    return 0;
                };
                let (p, cap) = (b as usize, len.max(0) as usize);
                if p + cap > data.len() {
                    return 0;
                }
                let n = text.len().min(cap);
                data[p..p + n].copy_from_slice(&text.as_bytes()[..n]);
                return n as i32;
            }
            _ => {}
        }
        if op == 5 {
            let Some(unix) = self.net_unix else {
                return 0;
            };
            let (h, m, sec, ..) = civil(unix as i64 + len as i64);
            let line = format!("{h:02}:{m:02}:{sec:02}");
            let Some(data) = mem.bytes() else {
                return 0;
            };
            let (p, cap) = (a as usize, b as usize);
            if p + cap > data.len() || cap == 0 {
                return 0;
            }
            let n = line.len().min(cap);
            data[p..p + n].copy_from_slice(&line.as_bytes()[..n]);
            return n as i32;
        }
        let io = &mut *self;
        match op {
            0 => {
                io.net_num.insert(a, b);
                0
            }
            1 => match text {
                Some(t) => {
                    io.net_text
                        .insert(a, String::from_utf8_lossy(&t).into_owned());
                    1
                }
                None => 0,
            },
            2 => {
                io.net_radio = a;
                0
            }
            3 => io.net_wifi.get(&a).copied().unwrap_or(0),
            4 => match (a, io.net_unix) {
                (0, u) => i32::from(u.is_some()),
                (1, _) => io.net_age,
                (_, None) => 0,
                (9, Some(u)) => u as i32,
                (f, Some(u)) => {
                    let (h, m, sec, day, month, year, wd) = civil(u as i64 + b as i64);
                    [h, m, sec, day, month, year, wd]
                        .get(f as usize - 2)
                        .copied()
                        .unwrap_or(0)
                }
            },
            6 => {
                io.eth_num.insert(a, b);
                0
            }
            7 => match text {
                Some(t) => {
                    io.eth_text
                        .insert(a, String::from_utf8_lossy(&t).into_owned());
                    1
                }
                None => 0,
            },
            8 => io.eth_state.get(&a).copied().unwrap_or(0),
            9 => {
                io.mqtt_num.insert(a, b);
                0
            }
            10 => match text {
                Some(t) => {
                    io.mqtt_text
                        .insert(a, String::from_utf8_lossy(&t).into_owned());
                    1
                }
                None => 0,
            },
            11 => io.mqtt_state.get(&a).copied().unwrap_or(0),
            16 => io.mqtt_slots.get(a as usize).map_or(0, |s| s.1),
            _ => 0,
        }
    }
}

/// A widget of the web page as the simulator keeps it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebStubItem {
    /// 0 number, 1 text shown, 2 text input, 3 secret text input, 4 chart.
    pub kind: u8,
    pub rw: bool,
    /// The module forbids the page to write (`write` input of the text input).
    pub locked: bool,
    pub dec: u8,
    /// Chart: points the master keeps.
    pub points: u8,
    pub lo: f32,
    pub hi: f32,
    pub value: f32,
    pub text: String,
}

impl StubIo {
    /// A message from the broker: every slot whose filter matches takes it (up to 128 bytes).
    pub fn mqtt_receive(&mut self, topic: &str, payload: &str) {
        for (i, f) in self.mqtt_filters.iter().enumerate() {
            if mqtt_match(f, topic) {
                let cut = payload.len().min(128);
                self.mqtt_slots[i].0 =
                    String::from_utf8_lossy(&payload.as_bytes()[..cut]).into_owned();
                self.mqtt_slots[i].1 += 1;
            }
        }
    }
}

/// Topic against a filter with `+` and `#`.
fn mqtt_match(filter: &str, topic: &str) -> bool {
    let (mut f, mut t) = (filter.split('/'), topic.split('/'));
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// Date and time of a Unix second: hour, minute, second, day, month, year, weekday (1 = Monday... 7 = Sunday, as the C++ block).
fn civil(t: i64) -> (i32, i32, i32, i32, i32, i32, i32) {
    let days = t.div_euclid(86400);
    let rest = t.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = (days + 3).rem_euclid(7) + 1;
    (
        (rest / 3600) as i32,
        (rest % 3600 / 60) as i32,
        (rest % 60) as i32,
        day as i32,
        month as i32,
        year as i32,
        weekday as i32,
    )
}
