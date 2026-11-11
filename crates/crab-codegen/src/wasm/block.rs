use super::*;

impl<'a, 'p> Emitter<'a, 'p> {
    /// Emits the block; the value of type `Ty` it leaves on the stack is output 0.
    pub(super) fn block(&mut self, inst: &Instance) -> Result<(), WasmError> {
        if self.hardware_block(inst)? {
            return Ok(());
        }
        let has_string = inst.inputs.values().any(|i| i.ty == SignalType::String)
            || inst
                .outputs
                .values()
                .any(|&s| self.program.signals[s].ty == SignalType::String);
        if has_string {
            return Err(self.unsupported(inst, "строки в этом блоке пока не поддержаны в модулях"));
        }
        let kind = inst.block.kind.as_str();
        if let Some(code) = crab_blocks::wasm_block(kind) {
            let ty = code(&mut BlockEmit { e: self, inst })
                .map_err(|why| self.unsupported(inst, &why))?;
            if let Some(ty) = ty {
                if inst.lib.def.outputs.is_empty() {
                    self.op(I::Drop);
                } else {
                    self.set_out(inst, 0, ty.into());
                }
            }
            return Ok(());
        }
        let ty = match kind {
            "and" => {
                self.fold_flags(inst, I::I32And, 1);
                Ty::I
            }
            "or" => {
                self.fold_flags(inst, I::I32Or, 0);
                Ty::I
            }
            "xor" => {
                self.fold_flags(inst, I::I32Xor, 0);
                Ty::I
            }
            "not" => {
                self.flag(inst, 0);
                self.op(I::I32Eqz);
                Ty::I
            }
            "rs_trigger" | "sr_trigger" => {
                // rs: reset wins; sr: set wins. The output keeps its value otherwise.
                let (first, second) = if kind == "rs_trigger" { (1, 0) } else { (0, 1) };
                let (first_value, second_value) =
                    if kind == "rs_trigger" { (0, 1) } else { (1, 0) };
                self.flag(inst, first);
                self.choose(
                    Ty::I,
                    |e| e.c_i(first_value),
                    |e| {
                        e.flag(inst, second);
                        e.choose(
                            Ty::I,
                            |e| e.c_i(second_value),
                            |e| {
                                e.out_get(inst, 0);
                            },
                        );
                    },
                );
                Ty::I
            }
            "hysteresis" => {
                let (x, on, off) = (self.tmp_f(), self.tmp_f(), self.tmp_f());
                self.input_as(inst, 0, Ty::F);
                self.op(I::LocalSet(x));
                self.param_as(inst, 0, Ty::F);
                self.op(I::LocalSet(on));
                self.param_as(inst, 1, Ty::F);
                self.op(I::LocalSet(off));
                let get = |e: &mut Self, l: u32| e.op(I::LocalGet(l));
                // Comparison direction depends on which threshold is higher.
                let band = |e: &mut Self, hi_on: bool| {
                    // set when x crosses `on`, reset when it crosses `off`
                    get(e, x);
                    get(e, on);
                    e.op(if hi_on { I::F32Ge } else { I::F32Le });
                    e.choose(
                        Ty::I,
                        |e| e.c_i(1),
                        |e| {
                            get(e, x);
                            get(e, off);
                            e.op(if hi_on { I::F32Le } else { I::F32Ge });
                            e.choose(
                                Ty::I,
                                |e| e.c_i(0),
                                |e| {
                                    e.out_get(inst, 0);
                                },
                            );
                        },
                    );
                };
                get(self, on);
                get(self, off);
                self.op(I::F32Ge);
                self.choose(Ty::I, |e| band(e, true), |e| band(e, false));
                Ty::I
            }
            "compare" => {
                let mode = self.const_param(inst, 0)? as i64;
                let w = self.input_ty(inst, 0).widen(self.input_ty(inst, 1));
                self.input_as(inst, 0, w);
                self.input_as(inst, 1, w);
                self.compare(w, mode);
                Ty::I
            }
            "greater" | "less" => {
                let w = self.input_ty(inst, 0).widen(self.param_ty(inst, 0));
                self.input_as(inst, 0, w);
                self.param_as(inst, 0, w);
                self.compare(w, if kind == "greater" { 4 } else { 2 });
                Ty::I
            }
            "add" | "sub" | "mul" => {
                let w = self.input_ty(inst, 0).widen(self.input_ty(inst, 1));
                self.input_as(inst, 0, w);
                self.input_as(inst, 1, w);
                self.op(match (kind, w) {
                    ("add", Ty::I) => I::I32Add,
                    ("add", Ty::F) => I::F32Add,
                    ("sub", Ty::I) => I::I32Sub,
                    ("sub", Ty::F) => I::F32Sub,
                    (_, Ty::I) => I::I32Mul,
                    (_, Ty::F) => I::F32Mul,
                });
                w
            }
            "min" | "max" => {
                let w = self.input_ty(inst, 0).widen(self.input_ty(inst, 1));
                self.input_as(inst, 0, w);
                self.input_as(inst, 1, w);
                self.pick(w, kind == "max");
                w
            }
            "div" => {
                // Always `float`; a zero divisor gives 0.
                let (a, b) = (self.tmp_f(), self.tmp_f());
                self.input_as(inst, 0, Ty::F);
                self.op(I::LocalSet(a));
                self.input_as(inst, 1, Ty::F);
                self.op(I::LocalSet(b));
                self.op(I::LocalGet(b));
                self.c_f(0.0);
                self.op(I::F32Ne);
                self.choose(
                    Ty::F,
                    |e| {
                        e.op(I::LocalGet(a));
                        e.op(I::LocalGet(b));
                        e.op(I::F32Div);
                    },
                    |e| e.c_f(0.0),
                );
                Ty::F
            }
            "mod" => {
                let w = self.input_ty(inst, 0).widen(self.input_ty(inst, 1));
                let (a, b) = (self.tmp(w), self.tmp(w));
                self.input_as(inst, 0, w);
                self.op(I::LocalSet(a));
                self.input_as(inst, 1, w);
                self.op(I::LocalSet(b));
                self.op(I::LocalGet(b));
                self.flagify(w);
                self.choose(
                    w,
                    |e| {
                        e.op(I::LocalGet(a));
                        if w == Ty::I {
                            e.op(I::LocalGet(b));
                            e.op(I::I32RemS);
                        } else {
                            // fmod through `f64`: a - b * trunc(a / b), with the sign of `a`
                            e.op(I::F64PromoteF32);
                            e.op(I::LocalGet(b));
                            e.op(I::F64PromoteF32);
                            e.op(I::LocalGet(a));
                            e.op(I::F64PromoteF32);
                            e.op(I::LocalGet(b));
                            e.op(I::F64PromoteF32);
                            e.op(I::F64Div);
                            e.op(I::F64Trunc);
                            e.op(I::F64Mul);
                            e.op(I::F64Sub);
                            e.op(I::LocalGet(a));
                            e.op(I::F64PromoteF32);
                            e.op(I::F64Copysign);
                            e.op(I::F32DemoteF64);
                        }
                    },
                    |e| e.c(w, 0.0),
                );
                w
            }
            "limit" => {
                let tx = self.input_ty(inst, 0);
                let w = tx
                    .widen(self.param_ty(inst, 0))
                    .widen(self.param_ty(inst, 1));
                let (lo, hi) = (self.tmp(w), self.tmp(w));
                self.param_as(inst, 0, w);
                self.param_as(inst, 1, w);
                self.pick(w, false);
                self.op(I::LocalSet(lo));
                self.param_as(inst, 0, w);
                self.param_as(inst, 1, w);
                self.pick(w, true);
                self.op(I::LocalSet(hi));
                self.input_as(inst, 0, w);
                self.op(I::LocalGet(lo));
                self.pick(w, true);
                self.op(I::LocalGet(hi));
                self.pick(w, false);
                self.coerce(w, tx);
                tx
            }
            "scale" => {
                let (in_min, in_max, out_min, out_max, span, y) = (
                    self.tmp_f(),
                    self.tmp_f(),
                    self.tmp_f(),
                    self.tmp_f(),
                    self.tmp_f(),
                    self.tmp_f(),
                );
                for (i, l) in [in_min, in_max, out_min, out_max].into_iter().enumerate() {
                    self.param_as(inst, i, Ty::F);
                    self.op(I::LocalSet(l));
                }
                self.op(I::LocalGet(in_max));
                self.op(I::LocalGet(in_min));
                self.op(I::F32Sub);
                self.op(I::LocalSet(span));
                self.op(I::LocalGet(span));
                self.c_f(0.0);
                self.op(I::F32Ne);
                self.choose(
                    Ty::F,
                    |e| {
                        e.op(I::LocalGet(out_min));
                        e.input_as(inst, 0, Ty::F);
                        e.op(I::LocalGet(in_min));
                        e.op(I::F32Sub);
                        e.op(I::LocalGet(out_max));
                        e.op(I::LocalGet(out_min));
                        e.op(I::F32Sub);
                        e.op(I::F32Mul);
                        e.op(I::LocalGet(span));
                        e.op(I::F32Div);
                        e.op(I::F32Add);
                    },
                    |e| e.op(I::LocalGet(out_min)),
                );
                self.op(I::LocalSet(y));
                // Optional clamp to the output range (parameter 4):
                // `y < lo ? lo : y > hi ? hi : y` with lo = min(out_min, out_max), hi = max(..).
                let (lo, hi) = (self.tmp_f(), self.tmp_f());
                for (l, greater) in [(lo, false), (hi, true)] {
                    self.op(I::LocalGet(out_min));
                    self.op(I::LocalGet(out_max));
                    self.pick(Ty::F, greater);
                    self.op(I::LocalSet(l));
                }
                self.op(I::LocalGet(lo));
                self.op(I::LocalGet(hi));
                self.op(I::LocalGet(y));
                self.op(I::LocalGet(y));
                self.op(I::LocalGet(hi));
                self.op(I::F32Gt);
                self.op(I::Select);
                self.op(I::LocalGet(y));
                self.op(I::LocalGet(lo));
                self.op(I::F32Lt);
                self.op(I::Select);
                self.op(I::LocalGet(y));
                self.param_flag(inst, 4);
                self.op(I::Select);
                Ty::F
            }
            "t_trigger" => {
                let prev = self.state_i();
                self.flag(inst, 1);
                self.choose(
                    Ty::I,
                    |e| e.c_i(0),
                    |e| {
                        e.flag(inst, 0);
                        e.op(I::GlobalGet(prev));
                        e.op(I::I32Eqz);
                        e.op(I::I32And);
                        e.choose(
                            Ty::I,
                            |e| {
                                e.out_get(inst, 0);
                                e.op(I::I32Eqz);
                            },
                            |e| {
                                e.out_get(inst, 0);
                            },
                        );
                    },
                );
                self.flag(inst, 0);
                self.op(I::GlobalSet(prev));
                Ty::I
            }
            "rising_edge" | "falling_edge" => {
                let prev = self.state_i();
                let n = self.tmp_i();
                self.flag(inst, 0);
                self.op(I::LocalSet(n));
                if kind == "rising_edge" {
                    self.op(I::LocalGet(n));
                    self.op(I::GlobalGet(prev));
                    self.op(I::I32Eqz);
                } else {
                    self.op(I::LocalGet(n));
                    self.op(I::I32Eqz);
                    self.op(I::GlobalGet(prev));
                }
                self.op(I::I32And);
                self.op(I::LocalGet(n));
                self.op(I::GlobalSet(prev));
                Ty::I
            }
            "timer_on" => {
                let (running, start) = (self.state_i(), self.state_i());
                self.flag(inst, 0);
                self.choose(
                    Ty::I,
                    |e| {
                        e.op(I::GlobalGet(running));
                        e.op(I::I32Eqz);
                        e.op(I::If(BlockType::Empty));
                        e.c_i(1);
                        e.op(I::GlobalSet(running));
                        e.op(I::GlobalGet(GLOBAL_NOW));
                        e.op(I::GlobalSet(start));
                        e.op(I::End);
                        e.since(start);
                        e.param_ms(inst, 0);
                        e.op(I::I32GeU);
                    },
                    |e| {
                        e.c_i(0);
                        e.op(I::GlobalSet(running));
                        e.c_i(0);
                    },
                );
                Ty::I
            }
            "timer_off" => {
                let (on, off_at) = (self.state_i(), self.state_i());
                self.flag(inst, 0);
                self.choose(
                    Ty::I,
                    |e| {
                        e.c_i(1);
                        e.op(I::GlobalSet(on));
                        e.c_i(1);
                    },
                    |e| {
                        e.op(I::GlobalGet(on));
                        e.op(I::If(BlockType::Empty));
                        e.c_i(0);
                        e.op(I::GlobalSet(on));
                        e.op(I::GlobalGet(GLOBAL_NOW));
                        e.op(I::GlobalSet(off_at));
                        e.op(I::End);
                        e.out_get(inst, 0);
                        e.c_i(0);
                        e.op(I::I32Ne);
                        e.since(off_at);
                        e.param_ms(inst, 0);
                        e.op(I::I32LtU);
                        e.op(I::I32And);
                    },
                );
                Ty::I
            }
            "pulse" => {
                let (prev, active, start) = (self.state_i(), self.state_i(), self.state_i());
                let n = self.tmp_i();
                self.flag(inst, 0);
                self.op(I::LocalSet(n));
                self.op(I::LocalGet(n));
                self.op(I::GlobalGet(prev));
                self.op(I::I32Eqz);
                self.op(I::I32And);
                self.op(I::GlobalGet(active));
                self.op(I::I32Eqz);
                self.op(I::I32And);
                self.op(I::If(BlockType::Empty));
                self.c_i(1);
                self.op(I::GlobalSet(active));
                self.op(I::GlobalGet(GLOBAL_NOW));
                self.op(I::GlobalSet(start));
                self.op(I::End);
                self.op(I::LocalGet(n));
                self.op(I::GlobalSet(prev));
                self.op(I::GlobalGet(active));
                self.since(start);
                self.param_ms(inst, 0);
                self.op(I::I32GeU);
                self.op(I::I32And);
                self.op(I::If(BlockType::Empty));
                self.c_i(0);
                self.op(I::GlobalSet(active));
                self.op(I::End);
                self.op(I::GlobalGet(active));
                Ty::I
            }
            "generator" => {
                // Deadline form (`specs/generator-deadline`): one subtraction and one compare per
                // scan; the division happens only when a scan skipped more than one phase.
                // Zeroed state means "running since time 0" (as the old formula measured from 0).
                let (start, ph, idle) = (self.state_i(), self.state_i(), self.state_i());
                let (on, off, len, d, r, period) = (
                    self.tmp_i(),
                    self.tmp_i(),
                    self.tmp_i(),
                    self.tmp_i(),
                    self.tmp_i(),
                    self.tmp_i(),
                );
                self.flag(inst, 0);
                self.choose(
                    Ty::I,
                    |e| {
                        e.param_ms(inst, 0);
                        e.op(I::LocalSet(on));
                        e.param_ms(inst, 1);
                        e.op(I::LocalSet(off));
                        // (re)start after a disable: the first phase is "on" (`ph` is the off flag)
                        e.op(I::GlobalGet(idle));
                        e.op(I::If(BlockType::Empty));
                        e.c_i(0);
                        e.op(I::GlobalSet(idle));
                        e.c_i(0);
                        e.op(I::GlobalSet(ph));
                        e.op(I::GlobalGet(GLOBAL_NOW));
                        e.op(I::GlobalSet(start));
                        e.op(I::End);
                        // len = length of the current phase
                        e.op(I::LocalGet(off));
                        e.op(I::LocalGet(on));
                        e.op(I::GlobalGet(ph));
                        e.op(I::Select);
                        e.op(I::LocalSet(len));
                        e.since(start);
                        e.op(I::LocalSet(d));
                        e.op(I::LocalGet(d));
                        e.op(I::LocalGet(len));
                        e.op(I::I32GeU);
                        e.op(I::If(BlockType::Empty));
                        // d - len < length of the other phase (= on + off - len): switch once
                        e.op(I::LocalGet(d));
                        e.op(I::LocalGet(len));
                        e.op(I::I32Sub);
                        e.op(I::LocalGet(on));
                        e.op(I::LocalGet(off));
                        e.op(I::I32Add);
                        e.op(I::LocalGet(len));
                        e.op(I::I32Sub);
                        e.op(I::I32LtU);
                        e.op(I::If(BlockType::Empty));
                        e.op(I::GlobalGet(ph));
                        e.c_i(1);
                        e.op(I::I32Xor);
                        e.op(I::GlobalSet(ph));
                        e.op(I::GlobalGet(start));
                        e.op(I::LocalGet(len));
                        e.op(I::I32Add);
                        e.op(I::GlobalSet(start));
                        e.op(I::Else);
                        // catch up: period = max(on + off, 1), r = d % period
                        e.op(I::LocalGet(on));
                        e.op(I::LocalGet(off));
                        e.op(I::I32Add);
                        e.op(I::LocalSet(period));
                        e.c_i(1);
                        e.op(I::LocalGet(period));
                        e.op(I::LocalGet(period));
                        e.op(I::I32Eqz);
                        e.op(I::Select);
                        e.op(I::LocalSet(period));
                        e.op(I::LocalGet(d));
                        e.op(I::LocalGet(period));
                        e.op(I::I32RemU);
                        e.op(I::LocalSet(r));
                        e.op(I::LocalGet(r));
                        e.op(I::LocalGet(len));
                        e.op(I::I32LtU);
                        e.op(I::If(BlockType::Empty));
                        e.op(I::GlobalGet(GLOBAL_NOW));
                        e.op(I::LocalGet(r));
                        e.op(I::I32Sub);
                        e.op(I::GlobalSet(start));
                        e.op(I::Else);
                        e.op(I::GlobalGet(ph));
                        e.c_i(1);
                        e.op(I::I32Xor);
                        e.op(I::GlobalSet(ph));
                        e.op(I::GlobalGet(GLOBAL_NOW));
                        e.op(I::LocalGet(r));
                        e.op(I::LocalGet(len));
                        e.op(I::I32Sub);
                        e.op(I::I32Sub);
                        e.op(I::GlobalSet(start));
                        e.op(I::End);
                        e.op(I::End);
                        e.op(I::End);
                        // out = on phase and a non-empty on time
                        e.op(I::GlobalGet(ph));
                        e.op(I::I32Eqz);
                        e.op(I::LocalGet(on));
                        e.c_i(0);
                        e.op(I::I32Ne);
                        e.op(I::I32And);
                    },
                    |e| {
                        e.c_i(1);
                        e.op(I::GlobalSet(idle));
                        e.c_i(0);
                    },
                );
                Ty::I
            }
            "debounce" => {
                let (last, changed) = (self.state_i(), self.state_i());
                let n = self.tmp_i();
                self.flag(inst, 0);
                self.op(I::LocalSet(n));
                self.op(I::LocalGet(n));
                self.op(I::GlobalGet(last));
                self.op(I::I32Ne);
                self.op(I::If(BlockType::Empty));
                self.op(I::LocalGet(n));
                self.op(I::GlobalSet(last));
                self.op(I::GlobalGet(GLOBAL_NOW));
                self.op(I::GlobalSet(changed));
                self.op(I::End);
                self.since(changed);
                self.param_ms(inst, 0);
                self.op(I::I32GeU);
                self.choose(
                    Ty::I,
                    |e| e.op(I::GlobalGet(last)),
                    |e| {
                        e.out_get(inst, 0);
                    },
                );
                Ty::I
            }
            "counter" => {
                let (up, down) = (self.state_i(), self.state_i());
                self.flag(inst, 2);
                self.choose(
                    Ty::I,
                    |e| e.c_i(0),
                    |e| {
                        e.out_get(inst, 0);
                        e.flag(inst, 0);
                        e.op(I::GlobalGet(up));
                        e.op(I::I32Eqz);
                        e.op(I::I32And);
                        e.op(I::I32Add);
                        e.flag(inst, 1);
                        e.op(I::GlobalGet(down));
                        e.op(I::I32Eqz);
                        e.op(I::I32And);
                        e.op(I::I32Sub);
                    },
                );
                self.flag(inst, 0);
                self.op(I::GlobalSet(up));
                self.flag(inst, 1);
                self.op(I::GlobalSet(down));
                Ty::I
            }
            "constant" => self.param(inst, 0),
            "switch" => {
                let w = self.input_ty(inst, 1).widen(self.input_ty(inst, 2));
                self.input_as(inst, 2, w);
                self.input_as(inst, 1, w);
                self.flag(inst, 0);
                self.op(I::Select);
                w
            }
            "mux" => {
                let count = inst
                    .lib
                    .def
                    .inputs
                    .iter()
                    .skip(1)
                    .filter(|p| inst.inputs.contains_key(p.name.as_str()))
                    .count()
                    .max(1);
                let w = (1..=count).fold(Ty::I, |t, j| t.widen(self.input_ty(inst, j)));
                let s = self.tmp_i();
                let r = self.tmp(w);
                self.input_as(inst, 0, Ty::I);
                self.c_i(0);
                self.pick(Ty::I, true);
                self.op(I::LocalSet(s));
                self.input_as(inst, count, w);
                self.op(I::LocalSet(r));
                for j in (0..count - 1).rev() {
                    self.input_as(inst, 1 + j, w);
                    self.op(I::LocalGet(r));
                    self.op(I::LocalGet(s));
                    self.c_i((j + 1) as i32);
                    self.op(I::I32LtS);
                    self.op(I::Select);
                    self.op(I::LocalSet(r));
                }
                self.op(I::LocalGet(r));
                w
            }
            "float_to_int" => {
                // Same as the sketch: `roundf` (half away from zero), `floorf`, `ceilf` or a
                // cut; the result saturates and NaN is 0.
                if self.input_ty(inst, 0) == Ty::I {
                    self.input_as(inst, 0, Ty::I);
                    self.set_out(inst, 0, Ty::I);
                    return Ok(());
                }
                let mode = inst
                    .params
                    .get("mode")
                    .map_or(0.0, |v| num(v, ParamKind::Int)) as i64;
                self.input_as(inst, 0, Ty::F);
                match mode {
                    0 => {
                        // x + copysign(0.5, x) in f64 is exact for any f32, then cut.
                        let x = self.tmp_f();
                        self.op(I::LocalSet(x));
                        self.op(I::LocalGet(x));
                        self.op(I::F64PromoteF32);
                        self.op(I::F64Const(0.5.into()));
                        self.op(I::LocalGet(x));
                        self.op(I::F64PromoteF32);
                        self.op(I::F64Copysign);
                        self.op(I::F64Add);
                        self.op(I::F64Trunc);
                        self.op(I::I32TruncSatF64S);
                    }
                    1 => {
                        self.op(I::F32Floor);
                        self.coerce(Ty::F, Ty::I);
                    }
                    2 => {
                        self.op(I::F32Ceil);
                        self.coerce(Ty::F, Ty::I);
                    }
                    _ => self.coerce(Ty::F, Ty::I),
                }
                Ty::I
            }
            "int_to_float" => {
                self.input_as(inst, 0, Ty::F);
                Ty::F
            }
            "var_get" => {
                let name = inst.block.var_name().unwrap_or_default().to_owned();
                self.var_read(&name)
            }
            "var_set" => {
                let name = inst.block.var_name().unwrap_or_default().to_owned();
                let enable = inst
                    .lib
                    .def
                    .inputs
                    .get(1)
                    .is_some_and(|p| inst.inputs.contains_key(p.name.as_str()));
                if enable {
                    self.flag(inst, 1);
                    self.op(I::If(BlockType::Empty));
                }
                let from = self.input(inst, 0);
                self.var_write(&name, from);
                if enable {
                    self.op(I::End);
                }
                return Ok(());
            }
            _ => {
                if self.user_block(inst)? {
                    return Ok(());
                }
                let reason = if inst.lib.def.info.kind.is_empty() {
                    "блок пока нельзя выразить в модуле"
                } else {
                    "блок пока не поддержан в модулях"
                };
                return Err(self.unsupported(inst, reason));
            }
        };
        if inst.lib.def.outputs.is_empty() {
            self.op(I::Drop);
        } else {
            self.set_out(inst, 0, ty);
        }
        Ok(())
    }
}
