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
            if var.ty.signal() == SignalType::String {
                let found = match tag {
                    Some(n) => self.str_tags.get(&n).copied(),
                    None => self.str_vars.get(&name).copied(),
                };
                if let Some(buf) = found {
                    let saved = Buf {
                        ptr: self.alloc(STR_CAP),
                        cap: STR_CAP,
                        len: self.global(Ty::I, &ParamValue::Int(0)),
                    };
                    let last = self.state_i();
                    self.hw.insert(HW_RETAIN_TEXT);
                    self.hw.insert(HW_TEXT);
                    self.retained_text.push(RetainedText {
                        key: crate::retain::key_bits(var) as i32,
                        buf,
                        saved,
                        last,
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
            let (saved, last) = (self.state_i(), self.state_i());
            self.retained.push(Retained {
                name,
                tag,
                ty,
                key: crate::retain::key_bits(var) as i32,
                default,
                saved,
                last,
            });
        }
    }

    /// Writes a retained value that changed, at most once a second.
    pub(super) fn retain_save(&mut self) {
        self.scratch_i = 0;
        self.scratch_f = 0;
        for r in self.retained_text.clone() {
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalGet(r.last));
            self.op(I::I32Sub);
            self.c_i(1000);
            self.op(I::I32GeU);
            self.op(I::If(BlockType::Empty));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.last));
            // equal? (mode 0, case sensitive)
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
            self.op(I::If(BlockType::Empty));
            self.c_i(1);
            self.c_i(r.key);
            self.c_i(r.buf.ptr as i32);
            self.op(I::GlobalGet(r.buf.len));
            self.hw_call(HW_RETAIN_TEXT);
            self.op(I::Drop);
            self.text_call(
                1,
                [
                    A::C(r.saved.ptr as i32),
                    A::C(r.saved.cap as i32),
                    A::C(r.buf.ptr as i32),
                    A::G(r.buf.len),
                    A::C(0),
                    A::C(0),
                ],
            );
            self.op(I::GlobalSet(r.saved.len));
            self.op(I::End);
            self.op(I::End);
        }
        for r in self.retained.clone() {
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalGet(r.last));
            self.op(I::I32Sub);
            self.c_i(1000);
            self.op(I::I32GeU);
            self.op(I::If(BlockType::Empty));
            self.op(I::GlobalGet(GLOBAL_NOW));
            self.op(I::GlobalSet(r.last));
            let bits = self.tmp_i();
            let ty = self.var_read(&r.name);
            if ty == Ty::F {
                self.op(I::I32ReinterpretF32);
            }
            self.op(I::LocalTee(bits));
            self.op(I::GlobalGet(r.saved));
            self.op(I::I32Ne);
            self.op(I::If(BlockType::Empty));
            self.c_i(1);
            self.c_i(r.key);
            self.op(I::LocalGet(bits));
            self.hw_call(HW_RETAIN);
            self.op(I::Drop);
            self.op(I::LocalGet(bits));
            self.op(I::GlobalSet(r.saved));
            self.op(I::End);
            self.op(I::End);
        }
    }
}
