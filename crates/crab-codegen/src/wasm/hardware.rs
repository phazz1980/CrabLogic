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
            || self.restart_block(inst)?
            || self.mqtt_block(inst)?
            || self.sparkplug_block(inst)?
            || self.web_block(inst)?
        {
            return Ok(true);
        }
        let kind = inst.block.kind.as_str();
        if !matches!(
            kind,
            "digital_in" | "analog_in" | "digital_out" | "pwm_out" | "analog_out"
        ) {
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
            "analog_out" => {
                let code = self.state_i();
                self.hold_unless_enabled(inst, |e| {
                    e.input_as(inst, 0, Ty::I);
                    e.clamp_int(0, 255);
                    e.op(I::GlobalSet(code));
                });
                // Fault bits: 0 volts adds none, hold is `FAULT_HOLD`, a fixed code rides in bits 16-23.
                let fault = match self.const_param(inst, 2)? as i32 {
                    1 => self.fault_pin(0, FAULT_HOLD),
                    2 => {
                        let value = (self.const_param(inst, 3)? as i32).clamp(0, 255);
                        self.fault_pin(0, FAULT_VALUE) | value << PIN_FAULT_VALUE_SHIFT
                    }
                    _ => 0,
                };
                self.extras.insert("dac");
                self.c_i(pin | fault);
                self.op(I::GlobalGet(code));
                self.hw_call(HW_DAC_WRITE);
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
