use super::*;

impl<'a, 'p> Emitter<'a, 'p> {
    /// The retained variables the module owns: its own locals and the tags it writes first.
    pub(super) fn collect_retained(&mut self) {
        let logic = &self.program.project.logic;
        let mut names: Vec<(String, Option<u32>)> = Vec::new();
        for l in &self.module.locals {
            names.push((l.name.clone(), None));
        }
        for t in &self.module.tags {
            if let (TagKind::Var(name), Some(_)) = (&t.tag.kind, &t.init) {
                names.push((name.clone(), Some(t.tag.number)));
            }
        }
        for (name, tag) in names {
            let Some(var) = logic.var(&name).filter(|v| v.retain) else {
                continue;
            };
            let (settle, period) = self.program.project.retain_timing(var);
            if var.ty.signal() == SignalType::String {
                let found = match tag {
                    Some(n) => self.str_tags.get(&n).copied(),
                    None => self.str_vars.get(&name).copied(),
                };
                if let Some(buf) = found {
                    let mut copy = || Buf {
                        ptr: self.alloc(STR_CAP),
                        cap: STR_CAP,
                        len: self.global(Ty::I, &ParamValue::Int(0)),
                    };
                    let saved = copy();
                    // The text of the previous scan: a second buffer, compared with the
                    // equality helper (cheaper to get right than a hash).
                    let prev = copy();
                    let (changed_at, written_at) = (self.state_i(), self.state_i());
                    let dirty_at = self.state_i();
                    self.hw.insert(HW_RETAIN_TEXT);
                    self.hw.insert(HW_TEXT);
                    self.retained_text.push(RetainedText {
                        key: crate::retain::key_bits(var) as i32,
                        buf,
                        saved,
                        prev,
                        changed_at,
                        dirty_at,
                        written_at,
                        settle: settle as i32,
                        period: period as i32,
                    });
                }
                continue;
            }
            let ty = Ty::of(var.ty.signal());
            let init = var.init.clone().unwrap_or(ParamValue::Int(0));
            let default = match ty {
                Ty::I => pv(&init) as i32,
                Ty::F => (pv(&init) as f32).to_bits() as i32,
            };
            let (saved, prev) = (self.state_i(), self.state_i());
            let (changed_at, written_at) = (self.state_i(), self.state_i());
            let dirty_at = self.state_i();
            self.retained.push(Retained {
                name,
                tag,
                ty,
                key: crate::retain::key_bits(var) as i32,
                default,
                saved,
                prev,
                changed_at,
                dirty_at,
                written_at,
                settle: settle as i32,
                period: period as i32,
                force: None,
            });
        }
    }

    /// Leaves on the stack whether a differing value may be written now: the last write is
    /// `period` old and the value either rested for `settle` or has been waiting for a whole
    /// `period` (a value that never rests is still saved once per period). Wrapping `u32`
    /// milliseconds, so the clock wrap is harmless.
    fn retain_due(&mut self, r: (u32, u32, u32), settle: i32, period: i32) {
        let (changed_at, dirty_at, written_at) = r;
        if period > 0 {
            self.since(written_at);
            self.c_i(period);
            self.op(I::I32GeU);
        } else {
            self.c_i(1);
        }
        if settle > 0 {
            self.since(changed_at);
            self.c_i(settle);
            self.op(I::I32GeU);
        } else {
            self.c_i(1);
        }
        if period > 0 {
            self.since(dirty_at);
            self.c_i(period);
            self.op(I::I32GeU);
            self.op(I::I32Or);
        }
        self.op(I::I32And);
    }

    /// `dst = src` for texts (`dst` gets the length).
    fn copy_text(&mut self, dst: &Buf, src: &Buf) {
        self.text_call(
            1,
            [
                A::C(dst.ptr as i32),
                A::C(dst.cap as i32),
                A::C(src.ptr as i32),
                A::G(src.len),
                A::C(0),
                A::C(0),
            ],
        );
        self.op(I::GlobalSet(dst.len));
    }

    /// Writes a retained value once it has settled (unchanged for `settle`), no more often
    /// than once per `period`. Every scan only tracks the changes.
    pub(super) fn retain_save(&mut self) {
        self.scratch_i = 0;
        self.scratch_f = 0;
        for r in self.retained_text.clone() {
            // Changed since the previous scan? Remember the text and the time.
            self.text_call(
                5,
                [
                    A::C(0),
                    A::C(0),
                    A::C(r.buf.ptr as i32),
                    A::G(r.buf.len),
                    A::C(r.prev.ptr as i32),
                    A::G(r.prev.len),
                ],
            );
            self.op(I::I32Eqz);
            self.op(I::If(BlockType::Empty));
            // It was clean (equal to flash) a scan ago: the waiting starts now.
            self.text_call(
                5,
                [
                    A::C(0),
                    A::C(0),
                    A::C(r.prev.ptr as i32),
                    A::G(r.prev.len),
                    A::C(r.saved.ptr as i32),
                    A::G(r.saved.len),
                ],
            );
            self.op(I::If(BlockType::Empty));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.dirty_at));
            self.op(I::End);
            self.copy_text(&r.prev, &r.buf);
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.changed_at));
            self.op(I::End);
            // Differs from flash (mode 0, case sensitive)?
            self.text_call(
                5,
                [
                    A::C(0),
                    A::C(0),
                    A::C(r.buf.ptr as i32),
                    A::G(r.buf.len),
                    A::C(r.saved.ptr as i32),
                    A::G(r.saved.len),
                ],
            );
            self.op(I::I32Eqz);
            self.retain_due((r.changed_at, r.dirty_at, r.written_at), r.settle, r.period);
            self.op(I::I32And);
            self.op(I::If(BlockType::Empty));
            self.c_i(1);
            self.c_i(r.key);
            self.c_i(r.buf.ptr as i32);
            self.op(I::GlobalGet(r.buf.len));
            self.hw_call(HW_RETAIN_TEXT);
            self.op(I::Drop);
            self.copy_text(&r.saved, &r.buf);
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.written_at));
            self.op(I::End);
        }
        for r in self.retained.clone() {
            let bits = self.tmp_i();
            let ty = self.var_read(&r.name);
            if ty == Ty::F {
                self.op(I::I32ReinterpretF32);
            }
            self.op(I::LocalSet(bits));
            self.op(I::LocalGet(bits));
            self.op(I::GlobalGet(r.prev));
            self.op(I::I32Ne);
            self.op(I::If(BlockType::Empty));
            // It was clean (equal to flash) a scan ago: the waiting starts now.
            self.op(I::GlobalGet(r.prev));
            self.op(I::GlobalGet(r.saved));
            self.op(I::I32Eq);
            self.op(I::If(BlockType::Empty));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.dirty_at));
            self.op(I::End);
            self.op(I::LocalGet(bits));
            self.op(I::GlobalSet(r.prev));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.changed_at));
            self.op(I::End);
            self.op(I::LocalGet(bits));
            self.op(I::GlobalGet(r.saved));
            self.op(I::I32Ne);
            self.retain_due((r.changed_at, r.dirty_at, r.written_at), r.settle, r.period);
            if let Some(force) = r.force {
                self.op(I::GlobalGet(force));
                self.op(I::I32Or);
            }
            self.op(I::I32And);
            self.op(I::If(BlockType::Empty));
            self.c_i(1);
            self.c_i(r.key);
            self.op(I::LocalGet(bits));
            self.hw_call(HW_RETAIN);
            self.op(I::Drop);
            self.op(I::LocalGet(bits));
            self.op(I::GlobalSet(r.saved));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.written_at));
            self.op(I::End);
            if let Some(force) = r.force {
                self.c_i(0);
                self.op(I::GlobalSet(force));
            }
        }
    }
}
