use super::*;

impl<'a, 'p> Emitter<'a, 'p> {
    /// Collects the expander chips used by blocks of the module.
    pub(super) fn collect_expanders(&mut self) {
        let program = self.program;
        let logic = &program.project.logic;
        for &bi in &self.module.blocks {
            let inst = &program.blocks[bi];
            let Some(name) = inst.block.expander.as_deref() else {
                continue;
            };
            if inst.block.tag.is_some() {
                continue;
            }
            let Some(e) = logic.expander(name) else {
                continue;
            };
            let Some(p) = crab_core::expander::pin_param(&inst.lib.def) else {
                continue;
            };
            let Some(ParamValue::Int(pin)) = inst.params.get(p.name.as_str()) else {
                continue;
            };
            if !self.xp.contains_key(name) {
                let all = ((1u32 << e.chip.pins()) - 1) as i32;
                let pin_byte = |v: i64| (v & 0xFF) as i32;
                let key = i32::from(e.address) & 0x7F
                    | pin_byte(e.sda) << 8
                    | pin_byte(e.scl) << 16
                    | if e.chip.pins() > 8 { 1 << 24 } else { 0 };
                let in_word = self.global(Ty::I, &ParamValue::Int(i64::from(all)));
                let out_word = self.global(Ty::I, &ParamValue::Int(i64::from(all)));
                self.extras.insert("pcf_expander");
                self.xp.insert(
                    name.to_owned(),
                    XChip {
                        key,
                        period: e.period_ms().unwrap_or(20) as i32,
                        in_word,
                        out_word,
                        in_mask: 0,
                    },
                );
            }
            if inst.block.kind == "digital_in" && (0..16).contains(pin) {
                if let Some(chip) = self.xp.get_mut(name) {
                    chip.in_mask |= 1 << pin;
                }
            }
        }
    }

    /// Start of a step: the pins of the chips that have inputs.
    pub(super) fn expander_poll(&mut self) {
        for chip in self.xp.values().cloned().collect::<Vec<_>>() {
            if chip.in_mask == 0 {
                continue;
            }
            // Only an answer of the chip counts: otherwise the previous pins stay.
            self.c_i(chip.key);
            self.c_i(chip.period);
            self.c_i(1);
            self.hw_call(HW_PCF_READ);
            self.op(I::If(BlockType::Empty));
            self.c_i(chip.key);
            self.c_i(chip.period);
            self.c_i(0);
            self.hw_call(HW_PCF_READ);
            self.op(I::GlobalSet(chip.in_word));
            self.op(I::End);
        }
    }

    /// End of a step: the wanted levels (inputs high) go to the chips.
    pub(super) fn expander_flush(&mut self) {
        for chip in self.xp.values().cloned().collect::<Vec<_>>() {
            self.c_i(chip.key);
            self.op(I::GlobalGet(chip.out_word));
            self.c_i(chip.in_mask);
            self.op(I::I32Or);
            self.hw_call(HW_PCF_WRITE);
            self.op(I::Drop);
        }
    }

    /// `digital_in` / `digital_out` standing on a pin of an expander.
    pub(super) fn expander_block(&mut self, inst: &Instance, name: &str) -> Result<(), WasmError> {
        let program = self.program;
        let shared = program.blocks.iter().enumerate().any(|(i, b)| {
            b.block.expander.as_deref() == Some(name) && !self.module.blocks.contains(&i)
        });
        if shared {
            return Err(self.unsupported(
                inst,
                "расширитель используют блоки с разных листов: в модулях это пока не поддержано",
            ));
        }
        let Some(chip) = self.xp.get(name).cloned() else {
            return Err(self.unsupported(inst, "расширитель не найден в проекте"));
        };
        let at = crab_core::expander::pin_param(&inst.lib.def)
            .and_then(|p| inst.lib.def.params.iter().position(|q| q.name == p.name))
            .unwrap_or(0);
        let pin = self.const_param(inst, at)? as i32;
        match inst.block.kind.as_str() {
            "digital_in" => {
                let inverted = self.const_param(inst, 2)? != 0.0;
                self.op(I::GlobalGet(chip.in_word));
                self.c_i(pin);
                self.op(I::I32ShrU);
                self.c_i(1);
                self.op(I::I32And);
                if inverted {
                    self.op(I::I32Eqz);
                }
                self.set_out(inst, 0, Ty::I);
            }
            _ => {
                let inverted = self.const_param(inst, 1)? != 0.0;
                let logic = self.state_i();
                self.hold_unless_enabled(inst, |e| {
                    e.flag(inst, 0);
                    e.op(I::GlobalSet(logic));
                });
                self.op(I::GlobalGet(chip.out_word));
                self.c_i(!(1 << pin));
                self.op(I::I32And);
                self.op(I::GlobalGet(logic));
                if inverted {
                    self.op(I::I32Eqz);
                }
                self.c_i(pin);
                self.op(I::I32Shl);
                self.op(I::I32Or);
                self.op(I::GlobalSet(chip.out_word));
            }
        }
        Ok(())
    }
}
