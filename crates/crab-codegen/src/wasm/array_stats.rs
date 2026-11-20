//! Fill, copy, sum, minimum, maximum and search of an array in a module. Behaviour is that of
//! `crates/crab-sim/src/arrays/stats.rs`; see `array.rs` for the helpers and the rules.

use super::*;
use crab_core::array_blocks::{ArrayOp, DEST_PARAM};
use crab_core::arrays::ElemType;

impl Emitter<'_, '_> {
    pub(super) fn array_stats(
        &mut self,
        inst: &Instance,
        op: ArrayOp,
        mem: array::ArrayMem,
    ) -> Result<(), WasmError> {
        match op {
            ArrayOp::Fill => self.array_fill(inst, mem),
            ArrayOp::Copy => self.array_copy(inst, mem)?,
            ArrayOp::Sum => self.array_sum(inst, mem),
            ArrayOp::Min => self.array_extreme(inst, mem, false),
            ArrayOp::Max => self.array_extreme(inst, mem, true),
            ArrayOp::Find => self.array_find(inst, mem),
            _ => return Err(self.unsupported(inst, "блок массива пока не поддержан в модулях")),
        }
        Ok(())
    }

    /// `array_fill`: in 0 = value, 1 = go. While `go` is on every element gets the value.
    fn array_fill(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let ty = mem.ty_of();
        self.flag(inst, 1);
        self.op(I::If(BlockType::Empty));
        let value = self.tmp(ty);
        self.input_as(inst, 0, ty);
        self.op(I::LocalSet(value));
        self.array_loop(mem, |e, i| {
            e.op(I::LocalGet(i));
            e.array_scale(mem);
            e.op(I::LocalGet(value));
            e.array_store(mem);
        });
        self.op(I::End);
    }

    /// `array_copy`: in 0 = go (rising edge), out 0 = err. The destination is the array named by the
    /// block's `dest` parameter; the first `min(len_src, len_dest)` elements are copied.
    fn array_copy(&mut self, inst: &Instance, src: array::ArrayMem) -> Result<(), WasmError> {
        let Some(ParamValue::Text(name)) = inst.block.params.get(DEST_PARAM) else {
            return Err(self.unsupported(inst, "не задан массив-приёмник"));
        };
        let name = name.clone();
        let dest = self.array_mem(inst, &name)?;
        let both = array::ArrayMem {
            len: src.len.min(dest.len),
            ..src
        };
        let prev = self.state_i();
        self.rising_edge(inst, 0, prev);
        self.op(I::If(BlockType::Empty));
        self.array_loop(both, |e, i| {
            e.op(I::LocalGet(i));
            e.array_scale(dest);
            e.op(I::LocalGet(i));
            e.array_scale(src);
            e.array_load(src);
            e.array_store(dest);
        });
        self.op(I::End);
        // The destination exists in a module (the check guarantees it): no error.
        self.c_i(0);
        self.set_out(inst, 0, Ty::I);
        Ok(())
    }

    /// `array_sum`: out 0 = sum. Int and byte wrap at 32 bits, float accumulates in `f32`, in order.
    fn array_sum(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let ty = mem.ty_of();
        let acc = self.tmp(ty);
        self.c(ty, 0.0);
        self.op(I::LocalSet(acc));
        self.array_loop(mem, |e, i| {
            e.op(I::LocalGet(acc));
            e.op(I::LocalGet(i));
            e.array_scale(mem);
            e.array_load(mem);
            e.op(if ty == Ty::F { I::F32Add } else { I::I32Add });
            e.op(I::LocalSet(acc));
        });
        self.op(I::LocalGet(acc));
        self.set_out(inst, 0, ty);
    }

    /// `array_min` / `array_max`: out 0 = value, out 1 = index of its first occurrence (strict
    /// comparison). Starts from element 0, so the array must not be empty (the check guarantees it).
    fn array_extreme(&mut self, inst: &Instance, mem: array::ArrayMem, greater: bool) {
        let ty = mem.ty_of();
        let best = self.tmp(ty);
        let best_at = self.tmp_i();
        let x = self.tmp(ty);
        self.c_i(0);
        self.array_load(mem);
        self.op(I::LocalSet(best));
        self.c_i(0);
        self.op(I::LocalSet(best_at));
        self.array_loop(mem, |e, i| {
            e.op(I::LocalGet(i));
            e.array_scale(mem);
            e.array_load(mem);
            e.op(I::LocalSet(x));
            e.op(I::LocalGet(x));
            e.op(I::LocalGet(best));
            e.compare(ty, if greater { 4 } else { 2 });
            e.op(I::If(BlockType::Empty));
            e.op(I::LocalGet(x));
            e.op(I::LocalSet(best));
            e.op(I::LocalGet(i));
            e.op(I::LocalSet(best_at));
            e.op(I::End);
        });
        self.op(I::LocalGet(best));
        self.set_out(inst, 0, ty);
        self.op(I::LocalGet(best_at));
        self.set_out(inst, 1, Ty::I);
    }

    /// `array_find`: in 0 = value; out 0 = index of the first equal element or -1, out 1 = found.
    /// The value is first brought to the form the elements are stored in (see `array_store`).
    fn array_find(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let ty = mem.ty_of();
        match mem.ty {
            ElemType::Byte => {
                self.input_as(inst, 0, Ty::I);
                self.c_i(255);
                self.op(I::I32And);
            }
            ElemType::Bool => self.flag(inst, 0),
            _ => self.input_as(inst, 0, ty),
        }
        let wanted = self.tmp(ty);
        self.op(I::LocalSet(wanted));
        let at = self.tmp_i();
        self.c_i(-1);
        self.op(I::LocalSet(at));
        let found = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(found));
        self.array_loop(mem, |e, i| {
            // Only until the first hit.
            e.op(I::LocalGet(found));
            e.op(I::I32Eqz);
            e.op(I::If(BlockType::Empty));
            e.op(I::LocalGet(i));
            e.array_scale(mem);
            e.array_load(mem);
            e.op(I::LocalGet(wanted));
            e.compare(ty, 0);
            e.op(I::If(BlockType::Empty));
            e.op(I::LocalGet(i));
            e.op(I::LocalSet(at));
            e.c_i(1);
            e.op(I::LocalSet(found));
            e.op(I::End);
            e.op(I::End);
        });
        self.op(I::LocalGet(at));
        self.set_out(inst, 0, Ty::I);
        self.op(I::LocalGet(found));
        self.set_out(inst, 1, Ty::I);
    }
}
