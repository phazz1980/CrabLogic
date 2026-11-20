//! Shift register, ring buffer and stack of an array in a module (`array_shift`, `array_ring`,
//! `array_stack`). All three act on rising edges of their flag inputs and keep the previous level
//! and their own state (head, count) in globals; the rules and helpers are in `array.rs`, the
//! behaviour to copy is `crates/crab-sim/src/arrays/queue.rs`.

use super::*;
use crab_core::array_blocks::ArrayOp;

impl Emitter<'_, '_> {
    /// Emits `Shift`, `Ring` or `Stack` of array `mem` (other ops are not queues).
    pub(super) fn array_queue(
        &mut self,
        inst: &Instance,
        op: ArrayOp,
        mem: array::ArrayMem,
    ) -> Result<(), WasmError> {
        match op {
            ArrayOp::Shift => self.array_shift(inst, mem),
            ArrayOp::Ring => self.array_ring(inst, mem),
            ArrayOp::Stack => self.array_stack(inst, mem),
            _ => return Err(self.unsupported(inst, "блок массива пока не поддержан в модулях")),
        }
        Ok(())
    }

    /// `array_shift`: in 0 = value, 1 = go. On the rising edge of `go` every element takes the one
    /// below it (from the last down, so nothing is overwritten early) and `value` goes to 0.
    fn array_shift(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let prev = self.state_i();
        self.rising_edge(inst, 1, prev);
        self.op(I::If(BlockType::Empty));
        // i = len - 1 down to 1: element i = element i - 1.
        let i = self.tmp_i();
        self.c_i(mem.len as i32 - 1);
        self.op(I::LocalSet(i));
        self.op(I::Block(BlockType::Empty));
        self.op(I::Loop(BlockType::Empty));
        self.fuel_check();
        self.op(I::LocalGet(i));
        self.c_i(1);
        self.op(I::I32LtS);
        self.op(I::BrIf(1));
        self.op(I::LocalGet(i));
        self.array_scale(mem);
        self.op(I::LocalGet(i));
        self.c_i(1);
        self.op(I::I32Sub);
        self.array_scale(mem);
        self.array_load(mem);
        self.array_store(mem);
        self.op(I::LocalGet(i));
        self.c_i(1);
        self.op(I::I32Sub);
        self.op(I::LocalSet(i));
        self.op(I::Br(0));
        self.op(I::End);
        self.op(I::End);
        // Element 0 takes the new value.
        self.c_i(0);
        self.input_as(inst, 0, mem.ty_of());
        self.array_store(mem);
        self.op(I::End);
    }

    /// `array_ring`: in 0 = value, 1 = push; out 0 = head, 1 = count. On the rising edge of `push`
    /// the value goes to the head, the head moves on (wrapping at the length) and the count grows
    /// up to the length.
    fn array_ring(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let len = mem.len as i32;
        let prev = self.state_i();
        let head = self.state_i();
        let count = self.state_i();
        self.rising_edge(inst, 1, prev);
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(head));
        self.array_scale(mem);
        self.input_as(inst, 0, mem.ty_of());
        self.array_store(mem);
        // head = head + 1, back to 0 at the length.
        self.op(I::GlobalGet(head));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::GlobalSet(head));
        self.op(I::GlobalGet(head));
        self.c_i(len);
        self.op(I::I32GeU);
        self.op(I::If(BlockType::Empty));
        self.c_i(0);
        self.op(I::GlobalSet(head));
        self.op(I::End);
        // count = min(count + 1, len).
        self.op(I::GlobalGet(count));
        self.c_i(len);
        self.op(I::I32LtU);
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::GlobalSet(count));
        self.op(I::End);
        self.op(I::End);
        self.op(I::GlobalGet(head));
        self.set_out(inst, 0, Ty::I);
        self.op(I::GlobalGet(count));
        self.set_out(inst, 1, Ty::I);
    }

    /// `array_stack`: in 0 = value, 1 = push, 2 = pop; out 0 = top (0 when empty), 1 = count,
    /// 2 = empty, 3 = full, 4 = err. Push is done before pop in the same scan; a refused push
    /// (full) or pop (empty) raises `err` for this scan only.
    fn array_stack(&mut self, inst: &Instance, mem: array::ArrayMem) {
        let ty = mem.ty_of();
        let len = mem.len as i32;
        let prev_push = self.state_i();
        let prev_pop = self.state_i();
        let count = self.state_i();
        let push_edge = self.tmp_i();
        let pop_edge = self.tmp_i();
        let err = self.tmp_i();
        self.rising_edge(inst, 1, prev_push);
        self.op(I::LocalSet(push_edge));
        self.rising_edge(inst, 2, prev_pop);
        self.op(I::LocalSet(pop_edge));
        self.c_i(0);
        self.op(I::LocalSet(err));

        // Push: element count = value, count + 1; a full stack refuses it.
        self.op(I::LocalGet(push_edge));
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.c_i(len);
        self.op(I::I32LtU);
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.array_scale(mem);
        self.input_as(inst, 0, ty);
        self.array_store(mem);
        self.op(I::GlobalGet(count));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::GlobalSet(count));
        self.op(I::Else);
        self.c_i(1);
        self.op(I::LocalSet(err));
        self.op(I::End);
        self.op(I::End);

        // Pop: count - 1; an empty stack refuses it.
        self.op(I::LocalGet(pop_edge));
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.c_i(1);
        self.op(I::I32Sub);
        self.op(I::GlobalSet(count));
        self.op(I::Else);
        self.c_i(1);
        self.op(I::LocalSet(err));
        self.op(I::End);
        self.op(I::End);

        // Top element when there is one, else 0.
        self.op(I::GlobalGet(count));
        self.choose(
            ty,
            |e| {
                e.op(I::GlobalGet(count));
                e.c_i(1);
                e.op(I::I32Sub);
                e.array_scale(mem);
                e.array_load(mem);
            },
            |e| e.c(ty, 0.0),
        );
        self.set_out(inst, 0, ty);
        self.op(I::GlobalGet(count));
        self.set_out(inst, 1, Ty::I);
        self.op(I::GlobalGet(count));
        self.op(I::I32Eqz);
        self.set_out(inst, 2, Ty::I);
        self.op(I::GlobalGet(count));
        self.c_i(len);
        self.op(I::I32GeU);
        self.set_out(inst, 3, Ty::I);
        self.op(I::LocalGet(err));
        self.set_out(inst, 4, Ty::I);
    }
}
