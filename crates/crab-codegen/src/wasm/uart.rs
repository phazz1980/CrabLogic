use super::*;

impl<'a, 'p> Emitter<'a, 'p> {
    /// `uart(op, port, a, b)` with `a` pushed by `a`.
    pub(super) fn uart_call(&mut self, op: i32, port: i32, a: impl FnOnce(&mut Self), b: i32) {
        self.c_i(op);
        self.c_i(port);
        a(self);
        self.c_i(b);
        self.hw_call(HW_UART);
    }

    /// The port start-up, once per block: the master opens it with the block's settings.
    pub(super) fn uart_setup(&mut self, port: i32, u: &crab_core::uart::UartConfig) {
        let (data, _, stop) = u.frame().unwrap_or((8, 'N', 1));
        let settings = [
            (0, 0),
            (1, u.speed as i32),
            (2, i32::from(data)),
            (3, i32::from(u.parity_code())),
            (4, i32::from(stop)),
            (5, u.rx as i32),
            (6, u.tx as i32),
            (7, u.de as i32),
            (8, u.rx_buffer as i32),
            (63, 0),
        ];
        let done = self.state_i();
        self.op(I::GlobalGet(done));
        self.op(I::I32Eqz);
        self.op(I::If(BlockType::Empty));
        for (code, value) in settings {
            self.uart_call(0, port, |e| e.c_i(code), value);
            self.op(I::Drop);
        }
        self.c_i(1);
        self.op(I::GlobalSet(done));
        self.op(I::End);
    }

    /// UART bytes: the master owns the port; a module takes one byte per step or hands bytes
    /// over (see `uart` in `master.ino`).
    pub(super) fn uart_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let kind = inst.block.kind.as_str();
        if !matches!(kind, "uart_read" | "uart_send" | "uart_config") {
            return Ok(false);
        }
        let Some((u, _)) = crate::arduino::uart_of(self.program, inst) else {
            return Err(self.unsupported(inst, "не найден UART блока"));
        };
        // Sending on USB / UART0 goes to the console of the master (its lines, shown by the editor);
        // reading and configuring them stay with the master.
        let console = (u.number == 0 || u.number == crab_core::uart::USB) && kind == "uart_send";
        if !console && !(1..=2).contains(&u.number) {
            return Err(self.unsupported(
                inst,
                "USB и UART0 заняты протоколом мастера: читать и настраивать можно UART1 и UART2, отправлять — и в консоль мастера (UART0)",
            ));
        }
        if console {
            self.extras.insert("uart_console");
        }
        let port = if console { 0 } else { i32::from(u.number) };
        let out_index = |name: &str| {
            inst.lib
                .def
                .outputs
                .iter()
                .position(|p| p.name == name)
                .unwrap_or(0)
        };
        match kind {
            "uart_read" => {
                self.uart_setup(port, &u);
                if self.named_param(inst, "mode")? == 0 {
                    self.uart_read_line(inst, port)?;
                    return Ok(true);
                }
                let r = self.tmp_i();
                self.uart_call(1, port, |e| e.c_i(0), 0);
                self.op(I::LocalSet(r));
                if inst.outputs.contains_key("new") {
                    self.op(I::LocalGet(r));
                    self.c_i(0);
                    self.op(I::I32GeS);
                    self.set_out(inst, out_index("new"), Ty::I);
                }
                if inst.outputs.contains_key("byte") {
                    // The last byte stays while nothing new comes.
                    self.op(I::LocalGet(r));
                    self.c_i(0);
                    self.op(I::I32GeS);
                    self.op(I::If(BlockType::Empty));
                    self.op(I::LocalGet(r));
                    self.set_out(inst, out_index("byte"), Ty::I);
                    self.op(I::End);
                }
                if inst.outputs.contains_key("available") {
                    self.uart_call(2, port, |e| e.c_i(0), 0);
                    self.set_out(inst, out_index("available"), Ty::I);
                }
            }
            "uart_config" => {
                let format_used = inst.inputs.get("new_format").is_some_and(
                    |i| !matches!(&i.source, Source::Default(ParamValue::Text(t)) if t.is_empty()),
                );
                let format = if format_used {
                    Some(self.str_in(inst, "new_format")?)
                } else {
                    None
                };
                self.uart_setup(port, &u);
                if inst.inputs.contains_key("apply") {
                    // On the rising edge of `apply` the speed changes.
                    let prev = self.state_i();
                    let apply = self.tmp_i();
                    let speed = inst
                        .lib
                        .def
                        .inputs
                        .iter()
                        .position(|p| p.name == "new_speed")
                        .unwrap_or(0);
                    let apply_in = inst
                        .lib
                        .def
                        .inputs
                        .iter()
                        .position(|p| p.name == "apply")
                        .unwrap_or(0);
                    self.flag(inst, apply_in);
                    self.op(I::LocalSet(apply));
                    self.op(I::LocalGet(apply));
                    self.op(I::GlobalGet(prev));
                    self.op(I::I32Eqz);
                    self.op(I::I32And);
                    self.op(I::If(BlockType::Empty));
                    // A new frame format first (the port restarts at its present speed), then the speed.
                    let ok = self.tmp_i();
                    self.c_i(1);
                    self.op(I::LocalSet(ok));
                    if let Some(f) = format {
                        self.push_arg(f.len());
                        self.op(I::I32Eqz);
                        self.choose(
                            Ty::I,
                            |e| e.c_i(1),
                            |e| {
                                e.c_i(7);
                                e.extras.insert("uart_format");
                                e.c_i(port);
                                e.push_arg(f.ptr());
                                e.push_arg(f.len());
                                e.hw_call(HW_UART);
                            },
                        );
                        self.op(I::LocalSet(ok));
                    }
                    self.uart_call(5, port, |e| e.input_as(inst, speed, Ty::I), 0);
                    self.op(I::Drop);
                    if inst.outputs.contains_key("ok") {
                        self.op(I::LocalGet(ok));
                        self.set_out(inst, out_index("ok"), Ty::I);
                    }
                    self.op(I::End);
                    self.op(I::LocalGet(apply));
                    self.op(I::GlobalSet(prev));
                }
                self.uart_call(4, port, |e| e.c_i(0), 0);
                self.set_out(inst, out_index("baud"), Ty::I);
            }
            _ => {
                if !console {
                    self.uart_setup(port, &u);
                }
                self.uart_send(inst, port)?;
            }
        }
        Ok(true)
    }

    /// `uart_send` in binary mode: the value goes out as 1, 2 or 4 bytes when the trigger fires.
    pub(super) fn uart_send(&mut self, inst: &Instance, port: i32) -> Result<(), WasmError> {
        let value_ty = inst.inputs.get("value").map(|i| i.ty);
        let text_mode =
            self.named_param(inst, "send_as")? == 0 || value_ty == Some(SignalType::String);
        let is_str = value_ty == Some(SignalType::String);
        let trigger = self.named_param(inst, "trigger")?;
        let ty = self.input_ty(inst, 1);
        let is_bool = value_ty == Some(SignalType::Bool);
        let int_size = self.named_param(inst, "int_size")?;
        let size = if is_bool {
            0
        } else if ty == Ty::F {
            2
        } else {
            int_size.clamp(0, 2)
        };
        let fire = self.tmp_i();
        let en = self.tmp_i();
        self.flag(inst, 0);
        self.op(I::LocalSet(en));
        let (started, last) = (self.state_i(), self.global(ty, &ParamValue::Int(0)));
        let last_str = if is_str && trigger == 2 {
            Some(self.str_last(inst)?)
        } else {
            None
        };
        match trigger {
            0 => {
                let prev = self.state_i();
                self.op(I::LocalGet(en));
                self.op(I::GlobalGet(prev));
                self.op(I::I32Eqz);
                self.op(I::I32And);
                self.op(I::LocalSet(fire));
                self.op(I::LocalGet(en));
                self.op(I::GlobalSet(prev));
            }
            1 => {
                self.op(I::LocalGet(en));
                self.op(I::LocalSet(fire));
            }
            _ => {
                let deadband = self.named_param_f(inst, "deadband");
                self.op(I::GlobalGet(started));
                self.op(I::I32Eqz);
                if let Some(keep) = last_str {
                    self.str_same(inst, keep)?;
                    self.op(I::I32Eqz);
                } else {
                    self.input_as(inst, 1, ty);
                    self.op(I::GlobalGet(last));
                    if ty == Ty::F && deadband > 0.0 {
                        self.op(I::F32Sub);
                        self.op(I::F32Abs);
                        self.c_f(deadband as f32);
                        self.op(I::F32Ge);
                    } else if ty == Ty::F {
                        self.op(I::F32Ne);
                    } else {
                        self.op(I::I32Ne);
                    }
                }
                self.op(I::I32Or);
                self.op(I::LocalGet(en));
                self.op(I::I32And);
                self.op(I::LocalSet(fire));
            }
        }
        let sent = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(sent));
        self.op(I::LocalGet(fire));
        self.op(I::If(BlockType::Empty));
        if text_mode {
            self.uart_send_text(inst, port)?;
        } else {
            self.uart_call(
                3,
                port,
                |e| {
                    if ty == Ty::F {
                        e.input_as(inst, 1, Ty::F);
                        e.op(I::I32ReinterpretF32);
                    } else {
                        e.input_as(inst, 1, Ty::I);
                        if is_bool {
                            e.c_i(0);
                            e.op(I::I32Ne);
                        } else if size == 0 {
                            e.clamp_int(0, 255);
                        } else if size == 1 {
                            e.clamp_int(-32768, 32767);
                        }
                    }
                },
                size,
            );
        }
        self.op(I::LocalSet(sent));
        if trigger == 2 {
            self.op(I::LocalGet(sent));
            self.op(I::If(BlockType::Empty));
            if let Some(keep) = last_str {
                self.str_keep(inst, keep)?;
            } else {
                self.input_as(inst, 1, ty);
                self.op(I::GlobalSet(last));
            }
            self.c_i(1);
            self.op(I::GlobalSet(started));
            self.op(I::End);
        }
        self.op(I::End);
        if inst.outputs.contains_key("sent") {
            self.op(I::LocalGet(sent));
            let o = inst
                .lib
                .def
                .outputs
                .iter()
                .position(|p| p.name == "sent")
                .unwrap_or(0);
            self.set_out(inst, o, Ty::I);
        }
        Ok(())
    }
}
