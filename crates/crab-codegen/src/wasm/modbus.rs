use super::*;
use crab_core::modbus_map::{ipv4_octets, TagDirection};
use crab_core::Block;

/// Most slave devices one master polls (the firmware has room for this many blocks).
const MAX_DEVICES: i32 = 32;
/// Unit ID of a device without a `unit` parameter (the library default).
const DEFAULT_UNIT: i32 = 1;

/// Text parameter of a block that is not an instance (a device or a slave of the project).
fn block_text<'b>(block: &'b Block, name: &str) -> &'b str {
    match block.params.get(name) {
        Some(ParamValue::Text(t)) => t.as_str(),
        _ => "",
    }
}

/// Integer parameter of a block that is not an instance, `default` when it is not set.
fn block_int(block: &Block, name: &str, default: i32) -> i32 {
    match block.params.get(name) {
        Some(ParamValue::Int(n)) => i32::try_from(*n).unwrap_or(default),
        _ => default,
    }
}

/// The owner of a `modbus_tag` as the firmware sees it.
struct TagOwner {
    /// The owner is a `modbus_device` of a master: the board polls it.
    master: bool,
    /// The master (or this slave) is a TCP one.
    tcp: bool,
    /// Unit ID the firmware calls the device with (0 for the slave role of this board).
    unit: i32,
}

impl<'a, 'p> Emitter<'a, 'p> {
    /// `modbus(op, a, b, c, d)` with the first four as constants and `d` pushed by `d`.
    pub(super) fn modbus_call(
        &mut self,
        op: i32,
        a: i32,
        b: i32,
        c: i32,
        d: impl FnOnce(&mut Self),
    ) {
        self.c_i(op);
        self.c_i(a);
        self.c_i(b);
        self.c_i(c);
        d(self);
        self.hw_call(HW_MODBUS);
    }

    /// The `ip` parameter (dotted, as in the C++ output) as `IPAddress` bits: the first octet is
    /// the lowest byte.
    pub(super) fn ip_bits(&self, inst: &Instance) -> Result<i32, WasmError> {
        let text = self.text_param(inst, "ip");
        let octets: Vec<u8> = text
            .split('.')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        let Ok(o): Result<[u8; 4], _> = octets.try_into() else {
            return Err(self.unsupported(inst, "адрес сервера должен быть вида 192.168.1.10"));
        };
        Ok(i32::from_le_bytes(o))
    }

    /// The `ip` of a slave device as `IPAddress` bits, the same encoding as [`Self::ip_bits`].
    /// 0 when the device has no address of its own (the master's is used).
    fn device_ip_bits(&self, inst: &Instance, device: &Block) -> Result<i32, WasmError> {
        let text = block_text(device, "ip");
        if text.is_empty() {
            return Ok(0);
        }
        match ipv4_octets(text) {
            Some(octets) => Ok(i32::from_le_bytes(octets)),
            None => Err(self.unsupported(
                inst,
                &format!("адрес slave «{}» должен быть вида 192.168.1.10", device.id),
            )),
        }
    }

    /// One setting of the master's Modbus (the answer is of no use).
    pub(super) fn modbus_cfg(&mut self, code: i32, value: i32) {
        self.modbus_call(0, code, value, 0, |e| e.c_i(0));
        self.op(I::Drop);
    }

    /// The owner of a tag, named by its `device` parameter: a `modbus_device` of a master or the
    /// slave block of this board. Fails when the owner is neither.
    fn tag_owner(&self, inst: &Instance) -> Result<TagOwner, WasmError> {
        let logic = &self.program.project.logic;
        let device = self.text_param(inst, "device");
        let Some(owner) = logic.block(&device) else {
            return Err(self.unsupported(
                inst,
                "тег не привязан к slave или устройству (параметр device)",
            ));
        };
        match owner.kind.as_str() {
            "modbus_device" => {
                let master_id = block_text(owner, "master");
                let Some(master) = logic.block(master_id) else {
                    return Err(
                        self.unsupported(inst, "устройство тега не привязано к Modbus master")
                    );
                };
                // The slave is named by its place in the master's list, not by its Unit ID: two
                // slaves at different addresses may share one (the master firmware reads a key of
                // `(index + 1) << 8` as an index).
                let index = logic
                    .modbus_devices(master_id)
                    .iter()
                    .position(|d| d.id == owner.id)
                    .unwrap_or(0);
                Ok(TagOwner {
                    master: true,
                    tcp: master.kind.starts_with("modbus_tcp"),
                    unit: i32::try_from(index + 1).unwrap_or(1) << 8,
                })
            }
            "modbus_rtu_slave" | "modbus_tcp_slave" => Ok(TagOwner {
                master: false,
                tcp: owner.kind.starts_with("modbus_tcp"),
                unit: 0,
            }),
            _ => Err(self.unsupported(
                inst,
                "параметр device тега должен указывать на modbus_device или Modbus slave",
            )),
        }
    }

    /// Modbus RTU/TCP: the master firmware runs the protocol, blocks set it up (once) and read and
    /// write the tables. See `modbus` in `master.ino`. Kinds: the masters and slaves, `modbus_tag`
    /// (one register or bit of a device; the direction comes from the links) and `modbus_device`
    /// (a slave of a master: it generates nothing, its settings go with the master).
    pub(super) fn modbus_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let kind = inst.block.kind.as_str();
        match kind {
            "modbus_device" => return Ok(true),
            "modbus_rtu_master" | "modbus_rtu_slave" | "modbus_tcp_master" | "modbus_tcp_slave"
            | "modbus_tag" => {}
            _ => return Ok(false),
        }
        self.extras.insert("modbus");
        let out_index = |name: &str| {
            inst.lib
                .def
                .outputs
                .iter()
                .position(|p| p.name == name)
                .unwrap_or(0)
        };
        if kind != "modbus_tag" {
            self.modbus_settings(inst, &out_index)?;
            return Ok(true);
        }
        self.modbus_tag(inst, &out_index)
    }

    /// Settings of a master or a slave role, sent once; the slave role also gets its error output.
    fn modbus_settings(
        &mut self,
        inst: &Instance,
        out_index: &dyn Fn(&str) -> usize,
    ) -> Result<(), WasmError> {
        let kind = inst.block.kind.as_str();
        let master = kind.ends_with("master");
        let tcp = kind.starts_with("modbus_tcp");
        // Code 0 comes first: it starts the list.
        let mut settings = if tcp {
            let eth = self.text_param(inst, "net") == "ethernet";
            let mut s = vec![
                (0, if master { 3 } else { 4 }),
                (1, i32::from(eth)),
                (2, self.named_param(inst, "port")?),
            ];
            if master {
                s.push((3, self.ip_bits(inst)?));
            }
            s
        } else {
            let Some((uart, _)) = crate::program::uart_of(self.program, inst) else {
                return Err(self.unsupported(inst, "не найден UART блока"));
            };
            vec![
                (0, if master { 1 } else { 2 }),
                (1, self.named_param(inst, "uart")?),
                (2, uart.speed as i32),
                (3, i32::from(uart.parity_code())),
                (4, uart.rx as i32),
                (5, uart.tx as i32),
                (6, uart.de as i32),
            ]
        };
        let logic = &self.program.project.logic;
        if master {
            // One slave block per device of this master, up to the master's `slaves` limit.
            let devices = logic.modbus_devices(&inst.block.id);
            let limit = self.named_param(inst, "slaves")?.clamp(1, MAX_DEVICES);
            let count = i32::try_from(devices.len())
                .unwrap_or(MAX_DEVICES)
                .clamp(1, limit);
            settings.push((7, count));
            for n in 0..count {
                let b = 16 + 8 * n;
                let device = devices.get(n as usize).copied();
                let (unit, sizes) = match device {
                    Some(device) => (
                        block_int(device, "unit", DEFAULT_UNIT),
                        logic.modbus_table_sizes(&device.id),
                    ),
                    None => (DEFAULT_UNIT, [0; 4]),
                };
                settings.push((b, unit));
                settings.push((b + 1, self.named_param(inst, "period")?));
                settings.push((b + 2, self.named_param(inst, "timeout")?));
                for (t, size) in sizes.iter().enumerate() {
                    settings.push((b + 3 + t as i32, *size as i32));
                }
                // The address of the device itself (TCP only): code 272 + 2n is the IP, 273 + 2n
                // the port; 0 means the master's.
                if let (true, Some(device)) = (tcp, device) {
                    settings.push((272 + 2 * n, self.device_ip_bits(inst, device)?));
                    settings.push((273 + 2 * n, block_int(device, "port", 0)));
                }
            }
        } else {
            settings.push((7, self.named_param(inst, "address")?));
            // The table sizes are those of this slave's own tags.
            let sizes = logic.modbus_table_sizes(&inst.block.id);
            for (t, size) in sizes.iter().enumerate() {
                settings.push((8 + t as i32, *size as i32));
            }
        }
        let done = self.state_i();
        self.op(I::GlobalGet(done));
        self.op(I::I32Eqz);
        self.op(I::If(BlockType::Empty));
        for (code, value) in settings {
            self.modbus_cfg(code, value);
        }
        self.modbus_cfg(63, 0);
        self.c_i(1);
        self.op(I::GlobalSet(done));
        self.op(I::End);
        if !master && inst.outputs.contains_key("error") {
            self.modbus_call(3, -1, i32::from(tcp) << 9, 0, |e| e.c_i(0));
            self.set_out(inst, out_index("error"), Ty::I);
        }
        Ok(())
    }

    /// One tag: a write when an input is connected, a read when an output is. The `ok` output
    /// asks the master for the state of the device (master side only).
    fn modbus_tag(
        &mut self,
        inst: &Instance,
        out_index: &dyn Fn(&str) -> usize,
    ) -> Result<bool, WasmError> {
        let direction = self
            .program
            .project
            .logic
            .modbus_tag_direction(&inst.block.id);
        let owner = self.tag_owner(inst)?;
        let (master, tcp, slave) = (owner.master, owner.tcp, owner.unit);
        // Bit 9 of `b` addresses the TCP role (see `modbus` in `master.ino`).
        let table = match self.text_param(inst, "table").as_str() {
            "input" => 1,
            "coil" => 2,
            "discrete" => 3,
            _ => 0,
        };
        let k = match self.text_param(inst, "kind").as_str() {
            "float" => 1,
            "bool" => 2,
            _ => 0,
        };
        // Bit 10 asks the master for float words in CDAB order ("Слова наоборот").
        let swap = k == 1 && self.named_param(inst, "swap")? != 0;
        let b =
            table | k << 4 | i32::from(master) << 8 | i32::from(tcp) << 9 | i32::from(swap) << 10;
        let reg = self.named_param(inst, "reg")?;
        let port = ["", "_f", "_b"][k as usize];
        match direction {
            TagDirection::Write => {
                let name = format!("in{port}");
                let i = inst
                    .lib
                    .def
                    .inputs
                    .iter()
                    .position(|p| p.name == name)
                    .unwrap_or(0);
                self.modbus_call(2, slave, b, reg, |e| {
                    if k == 1 {
                        e.input_as(inst, i, Ty::F);
                        e.op(I::I32ReinterpretF32);
                    } else {
                        e.input_as(inst, i, Ty::I);
                    }
                });
                self.op(I::Drop);
            }
            TagDirection::Read => {
                let name = format!("out{port}");
                if inst.outputs.contains_key(name.as_str()) {
                    self.modbus_call(1, slave, b, reg, |e| e.c_i(0));
                    if k == 1 {
                        self.op(I::F32ReinterpretI32);
                        self.set_out(inst, out_index(&name), Ty::F);
                    } else {
                        self.set_out(inst, out_index(&name), Ty::I);
                    }
                }
            }
            TagDirection::Both | TagDirection::None => {
                return Err(self.unsupported(
                    inst,
                    "тег читается и записывается или не подключён: подключите вход или выход",
                ));
            }
        }
        if master && inst.outputs.contains_key("ok") {
            self.modbus_call(3, slave, i32::from(tcp) << 9, 0, |e| e.c_i(0));
            self.set_out(inst, out_index("ok"), Ty::I);
        }
        Ok(true)
    }
}
