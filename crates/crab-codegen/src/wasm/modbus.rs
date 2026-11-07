use super::*;

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

    /// One setting of the master's Modbus (the answer is of no use).
    pub(super) fn modbus_cfg(&mut self, code: i32, value: i32) {
        self.modbus_call(0, code, value, 0, |e| e.c_i(0));
        self.op(I::Drop);
    }

    /// Modbus RTU: the master firmware runs the protocol, blocks set it up (once) and read and
    /// write the tables. See `modbus` in `master.ino`.
    pub(super) fn modbus_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let kind = inst.block.kind.as_str();
        if !matches!(
            kind,
            "modbus_rtu_master"
                | "modbus_rtu_slave"
                | "modbus_tcp_master"
                | "modbus_tcp_slave"
                | "modbus_master_read"
                | "modbus_master_write"
                | "modbus_read"
                | "modbus_write"
        ) {
            return Ok(false);
        }
        let out_index = |name: &str| {
            inst.lib
                .def
                .outputs
                .iter()
                .position(|p| p.name == name)
                .unwrap_or(0)
        };
        if matches!(
            kind,
            "modbus_rtu_master" | "modbus_rtu_slave" | "modbus_tcp_master" | "modbus_tcp_slave"
        ) {
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
                let Some((uart, _)) = crate::arduino::uart_of(self.program, inst) else {
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
            let tables = ["holding", "input", "coils", "discrete"];
            if master {
                let count = self.named_param(inst, "count")?.clamp(1, 4);
                settings.push((7, count));
                for n in 0..count {
                    let b = 16 + 8 * n;
                    settings.push((b, self.named_param(inst, &format!("slave{}", n + 1))?));
                    settings.push((b + 1, self.named_param(inst, "period")?));
                    settings.push((b + 2, self.named_param(inst, "timeout")?));
                    for (t, name) in tables.iter().enumerate() {
                        settings.push((b + 3 + t as i32, self.named_param(inst, name)?));
                    }
                }
            } else {
                settings.push((7, self.named_param(inst, "address")?));
                for (t, name) in tables.iter().enumerate() {
                    settings.push((8 + t as i32, self.named_param(inst, name)?));
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
            return Ok(true);
        }
        let master = kind.starts_with("modbus_master");
        let writing = kind.ends_with("write");
        // Bit 9 of `b` addresses the TCP role (see `modbus` in `master.ino`).
        let tcp = if master {
            self.text_param(inst, "master") == "tcp"
        } else {
            self.text_param(inst, "slave") != "rtu"
        };
        let slave = if master {
            self.named_param(inst, "slave")?
        } else {
            0
        };
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
        let b = table | k << 4 | i32::from(master) << 8 | i32::from(tcp) << 9;
        let reg = self.named_param(inst, "reg")?;
        let port = ["", "_f", "_b"][k as usize];
        if writing {
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
            return Ok(true);
        }
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
        if master && inst.outputs.contains_key("ok") {
            self.modbus_call(3, slave, i32::from(tcp) << 9, 0, |e| e.c_i(0));
            self.set_out(inst, out_index("ok"), Ty::I);
        }
        Ok(true)
    }
}
