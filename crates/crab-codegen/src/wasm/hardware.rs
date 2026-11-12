use super::*;

impl<'a, 'p> Emitter<'a, 'p> {
    pub(super) fn hw_call(&mut self, import: u32) {
        self.hw.insert(import);
        self.op(I::Call(import));
    }

    /// Sensors on 1-Wire/DHT pins: the master polls them on its own, the module takes the last
    /// values (`sensor_read(key, period_ms, field)`: 0 temperature, 1 humidity, 2 read ok).
    pub(super) fn sensor_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        // (kind code, index of the pin param, index of the period param, outputs as fields)
        let (kind, index, period, fields): (i32, i32, usize, &[(usize, i32)]) =
            match inst.block.kind.as_str() {
                "ds18b20" => (0, self.const_param(inst, 1)? as i32, 2, &[(0, 0), (1, 2)]),
                "dht22" => (1, 0, 1, &[(0, 0), (1, 1), (2, 2)]),
                _ => return Ok(false),
            };
        if inst.block.expander.is_some() {
            return Err(self.unsupported(inst, "датчик на расширителе не поддержан"));
        }
        let pin = self.const_param(inst, 0)? as i32;
        let period_ms = self.const_param(inst, period)? as i32;
        // One byte of pin and one nibble of index fit the key the master tells sensors apart by.
        let key = kind | (index & 15) << 4 | (pin & 0xFF) << 8;
        for &(output, field) in fields {
            self.c_i(key);
            self.c_i(period_ms);
            self.c_i(field);
            self.hw_call(HW_SENSOR_READ);
            if inst.lib.def.outputs[output].name == "ok" {
                self.c_f(0.0);
                self.op(I::F32Ne);
                self.set_out(inst, output, Ty::I);
            } else {
                self.set_out(inst, output, Ty::F);
            }
        }
        Ok(true)
    }

    /// A block with a native part (`native.cpp`, `specs/drivers`): the master runs its `CRAB_STEP`.
    /// The module writes the inputs, then the parameters, as 4-byte words into its memory and calls
    /// `native(kind, inst, in_ptr, out_ptr)`; the outputs come back as words. `kind` is the index of
    /// the block type among the native types of the project (sorted by name), `inst` the number of
    /// this block among those of its type.
    pub(super) fn native_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        if inst.lib.native_path().is_none() {
            return Ok(false);
        }
        let def = &inst.lib.def;
        let mut kinds: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for other in &self.program.blocks {
            if other.lib.native_path().is_some() {
                kinds
                    .entry(other.block.kind.as_str())
                    .or_default()
                    .push(other.block.id.as_str());
            }
        }
        let kind = kinds
            .keys()
            .position(|k| *k == inst.block.kind.as_str())
            .expect("this block is among the native ones");
        let number = kinds[inst.block.kind.as_str()]
            .iter()
            .position(|id| *id == inst.block.id.as_str())
            .expect("this block is among its type");
        let in_words = def.inputs.len() + def.params.len();
        let in_ptr = self.alloc(4 * in_words.max(1) as u32);
        let out_ptr = self.alloc(4 * def.outputs.len().max(1) as u32);
        let ty_of = |t: SignalType| if t == SignalType::Float { Ty::F } else { Ty::I };
        let store = |ty: Ty| {
            let arg = Self::mem_arg(0, 2);
            if ty == Ty::F {
                I::F32Store(arg)
            } else {
                I::I32Store(arg)
            }
        };
        let mut at = in_ptr;
        for (i, port) in def.inputs.iter().enumerate() {
            let ty = ty_of(port.ty);
            self.c_i(at as i32);
            self.input_as(inst, i, ty);
            self.op(store(ty));
            at += 4;
        }
        for (i, p) in def.params.iter().enumerate() {
            let ty = if p.kind == ParamKind::Float {
                Ty::F
            } else {
                Ty::I
            };
            self.c_i(at as i32);
            let got = self.param(inst, i);
            self.coerce(got, ty);
            self.op(store(ty));
            at += 4;
        }
        self.c_i(kind as i32);
        self.c_i(number as i32);
        self.c_i(in_ptr as i32);
        self.c_i(out_ptr as i32);
        self.hw_call(HW_NATIVE);
        // The master answers 0 when it cannot run the part: the outputs stay as they were.
        self.op(I::If(BlockType::Empty));
        for (k, port) in def.outputs.iter().enumerate() {
            if !inst.outputs.contains_key(port.name.as_str()) {
                continue;
            }
            let ty = ty_of(port.ty);
            self.c_i((out_ptr + 4 * k as u32) as i32);
            let arg = Self::mem_arg(0, 2);
            self.op(if ty == Ty::F {
                I::F32Load(arg)
            } else {
                I::I32Load(arg)
            });
            self.set_out(inst, k, ty);
        }
        self.op(I::End);
        Ok(true)
    }

    /// PCF8574 on I²C: the master talks to the chip, the module takes the last byte
    /// (`pcf_read(key, period_ms, field)`: 0 byte, 1 answered) or hands over the byte to write
    /// (`pcf_write(key, byte)` returns whether the chip answered the last write).
    pub(super) fn pcf_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let kind = inst
            .lib
            .def
            .info
            .code_of
            .as_deref()
            .unwrap_or(&inst.block.kind);
        let reading = match kind {
            "pcf8574_in" => true,
            "pcf8574_out" => false,
            _ => return Ok(false),
        };
        if inst.block.expander.is_some() {
            return Err(self.unsupported(inst, "микросхема PCF8574 на расширителе не поддержана"));
        }
        let address = self.const_param(inst, 0)? as i32;
        let sda = self.const_param(inst, 1)? as i32;
        let scl = self.const_param(inst, 2)? as i32;
        let inverted = self.const_param(inst, 3)? != 0.0;
        let count = (self.const_param(inst, 4)? as usize).clamp(1, 8);
        // One byte per pin; a negative pin (board default) becomes 0xFF.
        let key = address & 0x7F | (sda & 0xFF) << 8 | (scl & 0xFF) << 16;
        if reading {
            let period_ms = self.const_param(inst, 5)? as i32;
            let raw = self.tmp_i();
            self.c_i(key);
            self.c_i(period_ms);
            self.c_i(0);
            self.hw_call(HW_PCF_READ);
            if inverted {
                self.c_i(0xFF);
                self.op(I::I32Xor);
            }
            self.op(I::LocalSet(raw));
            for i in 0..count {
                if !inst
                    .outputs
                    .contains_key(inst.lib.def.outputs[i].name.as_str())
                {
                    continue;
                }
                self.op(I::LocalGet(raw));
                self.c_i(1 << i);
                self.op(I::I32And);
                self.c_i(0);
                self.op(I::I32Ne);
                self.set_out(inst, i, Ty::I);
            }
            self.c_i(key);
            self.c_i(period_ms);
            self.c_i(1);
            self.hw_call(HW_PCF_READ);
            self.set_out(inst, 8, Ty::I);
        } else {
            self.c_i(key);
            self.c_i(0);
            for i in 0..count {
                self.input_as(inst, i, Ty::I);
                self.c_i(1 << i);
                self.op(I::I32Mul);
                self.op(I::I32Or);
            }
            if inverted {
                self.c_i(0xFF);
                self.op(I::I32Xor);
            }
            self.hw_call(HW_PCF_WRITE);
            self.set_out(inst, 0, Ty::I);
        }
        Ok(true)
    }

    /// The pin number with the `on_fault` bits. The default (low) adds none, so such a module still
    /// runs on a master without the `on_fault` capability.
    fn fault_pin(&mut self, pin: i32, fault: i32) -> i32 {
        if fault == FAULT_LOW {
            return pin;
        }
        self.extras.insert("on_fault");
        pin | fault << PIN_FAULT_SHIFT
    }

    /// Digital/analog/PWM pins of the board: calls into the master.
    pub(super) fn hardware_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        // A block with its own driver (`block.wasm`) wins over the built-in master call.
        if self.user_block(inst)?
            || self.sensor_block(inst)?
            || self.pcf_block(inst)?
            || self.modbus_block(inst)?
            || self.uart_block(inst)?
            || self.text_block(inst)?
            || self.net_block(inst)?
            || self.native_block(inst)?
            || self.mqtt_block(inst)?
            || self.sparkplug_block(inst)?
            || self.web_block(inst)?
        {
            return Ok(true);
        }
        let kind = inst.block.kind.as_str();
        if !matches!(kind, "digital_in" | "analog_in" | "digital_out" | "pwm_out") {
            return Ok(false);
        }
        if let Some(name) = inst
            .block
            .expander
            .as_deref()
            .filter(|_| inst.block.tag.is_none())
        {
            if matches!(kind, "digital_in" | "digital_out") {
                let name = name.to_owned();
                self.expander_block(inst, &name)?;
                return Ok(true);
            }
        }
        if inst.block.expander.is_some() {
            return Err(self.unsupported(inst, "вывод на расширителе пока не поддержан в модулях"));
        }
        let pin = self.const_param(inst, 0)? as i32;
        match kind {
            "digital_in" => {
                let pullup = self.const_param(inst, 1)? != 0.0;
                let inverted = self.const_param(inst, 2)? != 0.0;
                self.c_i(pin | if pullup { PIN_PULLUP } else { 0 });
                self.hw_call(HW_GPIO_READ);
                self.c_i(0);
                self.op(I::I32Ne);
                if inverted {
                    self.op(I::I32Eqz);
                }
            }
            "analog_in" => {
                self.c_i(pin);
                self.hw_call(HW_ADC_READ);
                self.clamp_int(0, 4095);
            }
            "digital_out" => {
                let inverted = self.const_param(inst, 1)? != 0.0;
                let logic = self.state_i();
                self.hold_unless_enabled(inst, |e| {
                    e.flag(inst, 0);
                    e.op(I::GlobalSet(logic));
                });
                let fault = match self.const_param(inst, 3)? as i32 {
                    2 => FAULT_HOLD,
                    on => {
                        if (on != 0) != inverted {
                            FAULT_HIGH
                        } else {
                            FAULT_LOW
                        }
                    }
                };
                let pin = self.fault_pin(pin, fault);
                self.c_i(pin);
                self.op(I::GlobalGet(logic));
                if inverted {
                    self.op(I::I32Eqz);
                }
                self.hw_call(HW_GPIO_WRITE);
                return Ok(true);
            }
            _ => {
                let duty = self.global(Ty::F, &ParamValue::Float(0.0));
                self.hold_unless_enabled(inst, |e| {
                    e.input_as(inst, 0, Ty::F);
                    let x = e.tmp_f();
                    e.op(I::LocalSet(x));
                    // clamp(x, 0, 100) as selects
                    e.c_f(100.0);
                    e.op(I::LocalGet(x));
                    e.op(I::LocalGet(x));
                    e.c_f(100.0);
                    e.op(I::F32Gt);
                    e.op(I::Select);
                    e.op(I::LocalSet(x));
                    e.c_f(0.0);
                    e.op(I::LocalGet(x));
                    e.op(I::LocalGet(x));
                    e.c_f(0.0);
                    e.op(I::F32Lt);
                    e.op(I::Select);
                    e.op(I::GlobalSet(duty));
                });
                let fault = match self.const_param(inst, 2)? as i32 {
                    1 => FAULT_HIGH,
                    2 => FAULT_HOLD,
                    _ => FAULT_LOW,
                };
                // The master passes the value to `analogWrite`: 8 bits, 0..255, rounded half up.
                let pin = self.fault_pin(pin, fault);
                self.c_i(pin);
                self.op(I::GlobalGet(duty));
                self.c_f(2.55);
                self.op(I::F32Mul);
                let y = self.tmp_f();
                self.op(I::LocalSet(y));
                self.op(I::LocalGet(y));
                self.op(I::F32Trunc);
                let t = self.tmp_f();
                self.op(I::LocalSet(t));
                self.op(I::LocalGet(t));
                self.c_f(1.0);
                self.c_f(0.0);
                self.op(I::LocalGet(y));
                self.op(I::LocalGet(t));
                self.op(I::F32Sub);
                self.c_f(0.5);
                self.op(I::F32Ge);
                self.op(I::Select);
                self.op(I::F32Add);
                self.op(I::I32TruncSatF32U);
                self.hw_call(HW_PWM_WRITE);
                return Ok(true);
            }
        }
        self.set_out(inst, 0, Ty::I);
        Ok(true)
    }
}
