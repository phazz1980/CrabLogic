//! Test stands of `crablogic bench`: projects built in memory from templates, so the size is a
//! number. Every block of a chain feeds the next one, so none is optimised away and every block
//! runs on every step.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crab_core::{
    Block, BlockPos, BoardConfig, Layout, Link, Logic, ParamValue, PortRef, Project, ProjectConfig,
    ProjectInfo, UartConfig,
};

use super::secrets::{Net, Secrets, WifiSettings};

/// Chips the Rust master exists for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Esp32,
    Esp32c3,
}

/// Pins a stand may use on a chip: ones that are free on a bare dev board.
pub struct Pins {
    /// Output pin the chains end on (not an ADC or a call-stand pin).
    pub out: i64,
    pub gpio: &'static [i64],
    pub adc: &'static [i64],
    pub i2c: (i64, i64),
    /// UART number, RX, TX.
    pub uart: (u8, i64, i64),
}

impl Chip {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "esp32" => Some(Self::Esp32),
            "esp32c3" => Some(Self::Esp32c3),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Esp32 => "esp32",
            Self::Esp32c3 => "esp32c3",
        }
    }

    pub fn fqbn(self) -> String {
        format!("esp32:esp32:{}", self.name())
    }

    pub fn pins(self) -> Pins {
        match self {
            Self::Esp32 => Pins {
                out: 4,
                gpio: &[4, 5, 13, 14, 18, 19, 23, 25, 26, 27],
                adc: &[32, 33, 34, 35, 36, 39],
                i2c: (21, 22),
                uart: (1, 32, 33),
            },
            Self::Esp32c3 => Pins {
                out: 5,
                gpio: &[0, 1, 2, 3, 4, 5, 6, 7],
                adc: &[0, 1, 2, 3, 4],
                i2c: (8, 9),
                uart: (1, 6, 7),
            },
        }
    }
}

/// Interpreter or AOT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Interp,
    Aot,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Interp => "interp",
            Self::Aot => "aot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// A generator and an output only: the baseline of the call costs.
    Empty,
    Logic,
    Float,
    Timers,
    Text,
    GpioIn,
    GpioOut,
    Adc,
    I2c,
    Retain,
    Uart,
    /// The float sheet of 100 blocks with Wi-Fi and MQTT publishing every 100 ms.
    Net,
    /// The board is a Modbus TCP slave over Wi-Fi (20 registers fed by a float chain); the PC polls it.
    ModbusTcp,
    /// Copies of a small sheet until the board takes no more.
    Sheets,
    /// Module restart and board reset timing (the logic sheet of 100 blocks).
    Restart,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Logic => "logic",
            Self::Float => "float",
            Self::Timers => "timers",
            Self::Text => "text",
            Self::GpioIn => "gpio_in",
            Self::GpioOut => "gpio_out",
            Self::Adc => "adc",
            Self::I2c => "i2c",
            Self::Retain => "retain",
            Self::Uart => "uart",
            Self::Net => "net",
            Self::ModbusTcp => "modbus_tcp",
            Self::Sheets => "sheets",
            Self::Restart => "restart",
        }
    }

    /// A stand of the call-cost table: `N` identical calls of the master.
    pub fn is_call(self) -> bool {
        matches!(
            self,
            Self::GpioIn | Self::GpioOut | Self::Adc | Self::I2c | Self::Retain | Self::Uart
        )
    }

    /// Stands of this family come in several sizes.
    fn has_size(self) -> bool {
        matches!(
            self,
            Self::Logic
                | Self::Float
                | Self::Timers
                | Self::Text
                | Self::Net
                | Self::ModbusTcp
                | Self::Restart
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stand {
    /// `logic_100_aot`, `gpio_in_aot`: family, size (when it has one), mode.
    pub id: String,
    pub family: Family,
    /// Blocks asked for (a call stand: the number of calls; the real count is in [`Built`]).
    pub n: usize,
    pub mode: Mode,
}

impl Stand {
    fn new(family: Family, n: usize, mode: Mode) -> Self {
        let id = if family.has_size() {
            format!("{}_{n}_{}", family.name(), mode.name())
        } else {
            format!("{}_{}", family.name(), mode.name())
        };
        Self {
            id,
            family,
            n,
            mode,
        }
    }
}

/// Sizes of the sheet stands.
pub const SIZES: [usize; 3] = [10, 100, 500];
/// Calls in one call stand (fewer if the chip has fewer pins for it).
pub const CALLS: usize = 8;
/// Holding registers of the Modbus stand.
pub const MODBUS_REGISTERS: usize = 20;
/// Most sheets the memory stand tries.
pub const MAX_SHEETS: usize = 12;

/// All stands in the order they run.
pub fn standard_stands() -> Vec<Stand> {
    let mut stands = Vec::new();
    for family in [Family::Logic, Family::Float, Family::Timers, Family::Text] {
        for n in SIZES {
            for mode in [Mode::Aot, Mode::Interp] {
                stands.push(Stand::new(family, n, mode));
            }
        }
    }
    stands.push(Stand::new(Family::Empty, 0, Mode::Aot));
    for family in [
        Family::GpioIn,
        Family::GpioOut,
        Family::Adc,
        Family::I2c,
        Family::Retain,
        Family::Uart,
    ] {
        stands.push(Stand::new(family, CALLS, Mode::Aot));
    }
    stands.push(Stand::new(Family::Net, 100, Mode::Aot));
    stands.push(Stand::new(Family::ModbusTcp, 100, Mode::Aot));
    stands.push(Stand::new(Family::Sheets, 0, Mode::Aot));
    stands.push(Stand::new(Family::Restart, 100, Mode::Aot));
    stands
}

/// A stand turned into a project.
pub struct Built {
    pub project: Project,
    /// Blocks of the project.
    pub blocks: usize,
    /// Calls per step for a call stand, 0 otherwise.
    pub calls: usize,
}

pub fn build_project(
    stand: &Stand,
    chip: Chip,
    secrets: Option<&Secrets>,
) -> Result<Built, String> {
    let mut b = Builder::new(stand.id.as_str(), chip);
    let pins = chip.pins();
    let mut calls = 0;
    match stand.family {
        Family::Empty => b.logic_sheet(1, 0, true),
        Family::Logic | Family::Restart => b.logic_sheet(1, stand.n, true),
        Family::Float => {
            b.float_sheet(stand.n);
        }
        Family::Timers => b.timers_sheet(stand.n),
        Family::Text => b.text_sheet(stand.n),
        Family::Net => {
            let net = secrets
                .and_then(Secrets::net)
                .ok_or("нет настроек Wi-Fi и MQTT в bench/secrets.toml")?;
            let last = b.float_sheet(stand.n);
            b.net(&net, &last);
        }
        Family::ModbusTcp => {
            let wifi = secrets
                .and_then(Secrets::wifi)
                .ok_or("нет настроек Wi-Fi в bench/secrets.toml")?;
            b.modbus_tcp(&wifi, stand.n);
        }
        Family::Sheets => b.logic_sheet(1, 10, false),
        family => calls = b.calls(family, stand.n.min(pins.gpio.len()), &pins),
    }
    Ok(Built {
        blocks: b.project.logic.blocks.len(),
        calls,
        project: b.project,
    })
}

/// The project of the memory stand with `k` sheets of 10 logic blocks.
pub fn sheets_project(chip: Chip, k: usize) -> Project {
    let mut b = Builder::new("bench_sheets", chip);
    for sheet in 1..=k {
        b.logic_sheet(sheet as u32, 10, false);
    }
    b.project
}

struct Builder {
    project: Project,
    sheet: u32,
}

impl Builder {
    fn new(name: &str, chip: Chip) -> Self {
        let project = Project {
            path: PathBuf::from("bench").join(format!("{name}.crab")),
            config: ProjectConfig {
                project: ProjectInfo {
                    name: name.to_owned(),
                    description: String::new(),
                    comment_lang: String::new(),
                    password: String::new(),
                    embed: false,
                },
                board: BoardConfig {
                    fqbn: chip.fqbn(),
                    model: None,
                    port: None,
                    options: BTreeMap::new(),
                },
                uart: Vec::new(),
                retain: Default::default(),
            },
            logic: Logic::default(),
            layout: Layout::default(),
        };
        Self { project, sheet: 1 }
    }

    fn add(&mut self, id: &str, kind: &str, params: &[(&str, ParamValue)]) {
        let block = Block {
            id: id.to_owned(),
            kind: kind.to_owned(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            ..Default::default()
        };
        self.project.logic.blocks.push(block);
        let at = self.project.layout.blocks.len() as f32 * 10.0;
        self.project.layout.blocks.insert(
            id.to_owned(),
            BlockPos {
                sheet: self.sheet,
                x: at,
                y: 0.0,
            },
        );
    }

    fn constant(&mut self, id: &str, port: &str, value: ParamValue) {
        let block = self
            .project
            .logic
            .blocks
            .iter_mut()
            .find(|b| b.id == id)
            .expect("block added before");
        block.consts.insert(port.to_owned(), value);
    }

    fn link(&mut self, from: &str, to: &str) {
        self.project.logic.links.push(Link {
            from: from.parse::<PortRef>().expect("valid reference"),
            to: to.parse::<PortRef>().expect("valid reference"),
        });
    }

    fn chip(&self) -> Chip {
        if self.project.config.board.fqbn.ends_with("c3") {
            Chip::Esp32c3
        } else {
            Chip::Esp32
        }
    }

    /// A generator, `n` gates in a chain and (with `sink`) an output pin.
    fn logic_sheet(&mut self, sheet: u32, n: usize, sink: bool) {
        self.sheet = sheet;
        let p = format!("s{sheet}_");
        let src = format!("{p}gen");
        self.generator(&src);
        let mut prev = format!("{src}.out");
        for i in 0..n {
            let id = format!("{p}g{i}");
            let kind = ["and", "or", "xor", "not"][i % 4];
            self.add(&id, kind, &[]);
            if kind == "not" {
                self.link(&prev, &format!("{id}.in"));
            } else {
                self.link(&prev, &format!("{id}.a"));
                self.link(&format!("{src}.out"), &format!("{id}.b"));
            }
            prev = format!("{id}.out");
        }
        if sink {
            self.sink_bool(&prev);
        }
    }

    fn generator(&mut self, id: &str) {
        self.add(
            id,
            "generator",
            &[
                ("time_on", ParamValue::Text("100ms".into())),
                ("time_off", ParamValue::Text("100ms".into())),
            ],
        );
    }

    /// The chain's last bool output goes to a pin.
    fn sink_bool(&mut self, from: &str) {
        let pin = self.chip().pins().out;
        let id = format!("s{}_out", self.sheet);
        self.add(&id, "digital_out", &[("pin", ParamValue::Int(pin))]);
        self.link(from, &format!("{id}.in"));
    }

    /// An ADC reading as a float: the source of the float and text chains.
    fn adc_float(&mut self) -> String {
        let pin = self.chip().pins().adc[0];
        self.add("adc", "analog_in", &[("pin", ParamValue::Int(pin))]);
        self.add("adc_f", "int_to_float", &[]);
        self.link("adc.out", "adc_f.in");
        "adc_f.out".into()
    }

    /// Ends a float chain: compares with a limit and drives the output pin.
    fn sink_float(&mut self, from: &str) {
        self.add("limit", "greater", &[("b", ParamValue::Float(0.5))]);
        self.link(from, "limit.a");
        self.sink_bool("limit.out");
    }

    /// Returns the output of the last chain block (`block.out`).
    fn float_sheet(&mut self, n: usize) -> String {
        let mut prev = self.adc_float();
        for i in 0..n {
            let id = format!("f{i}");
            let kind = ["add", "mul", "sub", "div"][i % 4];
            self.add(&id, kind, &[]);
            self.constant(&id, "b", ParamValue::Float(1.0));
            self.link(&prev, &format!("{id}.a"));
            prev = format!("{id}.out");
        }
        self.sink_float(&prev);
        prev
    }

    fn timers_sheet(&mut self, n: usize) {
        self.generator("gen");
        let mut prev = "gen.out".to_owned();
        for i in 0..n {
            let id = format!("t{i}");
            let kind = if i % 2 == 0 { "timer_on" } else { "timer_off" };
            self.add(&id, kind, &[("time", ParamValue::Text("1ms".into()))]);
            self.link(&prev, &format!("{id}.in"));
            prev = format!("{id}.out");
        }
        self.sink_bool(&prev);
    }

    /// float -> text -> length -> float, round and round.
    fn text_sheet(&mut self, n: usize) {
        let mut prev = self.adc_float();
        let mut last_is_text = false;
        for i in 0..n {
            let id = format!("x{i}");
            match i % 3 {
                0 => {
                    self.add(&id, "num_to_text", &[("decimals", ParamValue::Int(1))]);
                    last_is_text = true;
                }
                1 => {
                    self.add(&id, "text_length", &[]);
                    last_is_text = false;
                }
                _ => {
                    self.add(&id, "int_to_float", &[]);
                    last_is_text = false;
                }
            }
            self.link(&prev, &format!("{id}.in"));
            prev = format!("{id}.out");
        }
        if last_is_text {
            self.add("x_end", "text_length", &[]);
            self.link(&prev, "x_end.in");
            prev = "x_end.out".into();
        }
        self.sink_float(&prev);
    }

    /// `n` blocks of one kind, each on its own pin or address. Returns how many were added.
    fn calls(&mut self, family: Family, n: usize, pins: &Pins) -> usize {
        self.generator("gen");
        let mut count = 0;
        for i in 0..n {
            let id = format!("c{i}");
            match family {
                Family::GpioIn => {
                    self.add(&id, "digital_in", &[("pin", ParamValue::Int(pins.gpio[i]))]);
                }
                Family::GpioOut => {
                    self.add(
                        &id,
                        "digital_out",
                        &[("pin", ParamValue::Int(pins.gpio[i]))],
                    );
                    self.link("gen.out", &format!("{id}.in"));
                }
                Family::Adc => {
                    let Some(&pin) = pins.adc.get(i) else { break };
                    self.add(&id, "analog_in", &[("pin", ParamValue::Int(pin))]);
                }
                Family::I2c => self.add(
                    &id,
                    "pcf8574_in",
                    &[
                        ("address", ParamValue::Int(32 + i as i64)),
                        ("sda", ParamValue::Int(pins.i2c.0)),
                        ("scl", ParamValue::Int(pins.i2c.1)),
                        ("count", ParamValue::Int(1)),
                        ("period", ParamValue::Text("1ms".into())),
                    ],
                ),
                Family::Retain => {
                    self.add(&id, "flash_store", &[]);
                    self.constant(&id, "in", ParamValue::Float(1.0));
                    self.constant(&id, "save", ParamValue::Bool(false));
                }
                Family::Uart => {
                    if i == 0 {
                        let (number, rx, tx) = pins.uart;
                        self.project.config.uart = vec![UartConfig {
                            number,
                            speed: 115_200,
                            rx,
                            tx,
                            ..UartConfig::default()
                        }];
                    }
                    self.add(
                        &id,
                        "uart_send",
                        &[
                            ("port", ParamValue::Int(i64::from(pins.uart.0))),
                            ("trigger", ParamValue::Int(1)),
                        ],
                    );
                    self.link("gen.out", &format!("{id}.en"));
                    self.constant(&id, "value", ParamValue::Int(7));
                }
                _ => unreachable!("not a call stand"),
            }
            count += 1;
        }
        count
    }

    /// Wi-Fi, a Modbus TCP slave with [`MODBUS_REGISTERS`] holding registers, and a float chain of
    /// `n` blocks whose evenly spaced outputs are written to the registers (two per float).
    fn modbus_tcp(&mut self, wifi: &WifiSettings, n: usize) {
        let _ = self.float_sheet(n);
        self.add(
            "wifi",
            "wifi",
            &[
                ("ssid", ParamValue::Text(wifi.ssid.clone())),
                ("password", ParamValue::Text(wifi.password.clone())),
            ],
        );
        self.add(
            "slave",
            "modbus_tcp_slave",
            &[
                ("holding", ParamValue::Int(MODBUS_REGISTERS as i64)),
                ("input", ParamValue::Int(0)),
                ("coils", ParamValue::Int(0)),
                ("discrete", ParamValue::Int(0)),
            ],
        );
        let floats = MODBUS_REGISTERS / 2;
        for k in 0..floats {
            let id = format!("reg{k}");
            self.add(
                &id,
                "modbus_write",
                &[
                    ("table", ParamValue::Text("holding".into())),
                    ("kind", ParamValue::Text("float".into())),
                    ("reg", ParamValue::Int(2 * k as i64)),
                ],
            );
            let tap = ((k + 1) * n / floats).saturating_sub(1);
            self.link(&format!("f{tap}.out"), &format!("{id}.in_f"));
        }
    }

    fn net(&mut self, net: &Net, last: &str) {
        self.add(
            "wifi",
            "wifi",
            &[
                ("ssid", ParamValue::Text(net.ssid.clone())),
                ("password", ParamValue::Text(net.wifi_password.clone())),
            ],
        );
        self.add(
            "broker",
            "mqtt",
            &[
                ("broker", ParamValue::Text(net.host.clone())),
                ("port", ParamValue::Int(i64::from(net.port))),
                ("user", ParamValue::Text(net.user.clone())),
                ("password", ParamValue::Text(net.mqtt_password.clone())),
                ("client_id", ParamValue::Text("crabbench".into())),
            ],
        );
        self.add(
            "pub",
            "mqtt_publish",
            &[
                ("topic", ParamValue::Text("crab/bench".into())),
                ("trigger", ParamValue::Int(2)),
                ("period", ParamValue::Int(100)),
            ],
        );
        self.link(last, "pub.value");
    }
}
