//! The restart block (`specs/restart`): a rising edge on `go` asks the master (`crab.restart`) to
//! restart the board after `delay`; the first `guard` of the module's life the block ignores `go`
//! (`blocked`), so a held command cannot make a restart loop.

use super::text::A;
use super::*;

/// `crab.restart` operations (`crab_kernel::abi::restart_op`).
const OP_REQUEST: i32 = 0;
const OP_CANCEL: i32 = 1;
const OP_REASON: i32 = 2;

impl Emitter<'_, '_> {
    /// `restart(op, a, b)`; leaves the result on the stack.
    fn restart_call(&mut self, op: i32, a: A, b: A) {
        self.c_i(op);
        self.push_arg(a);
        self.push_arg(b);
        self.hw_call(HW_RESTART);
    }

    pub(super) fn restart_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        if inst.block.kind != "restart" {
            return Ok(false);
        }
        let (prev, pending, done) = (self.state_i(), self.state_i(), self.state_i());
        let blocked = self.tmp_i();
        let go = self.tmp_i();
        let cancel = self.tmp_i();

        // The text of the reason is asked once, at the first step.
        if let Some(out) = self.out_buf(inst, "reason") {
            self.op(I::GlobalGet(done));
            self.op(I::I32Eqz);
            self.op(I::If(BlockType::Empty));
            self.c_i(1);
            self.op(I::GlobalSet(done));
            self.restart_call(OP_REASON, A::C(out.ptr as i32), A::C(out.cap as i32));
            self.op(I::GlobalSet(out.len));
            self.op(I::End);
        }

        // blocked = time since the module started < guard
        self.op(I::GlobalGet(GLOBAL_NOW));
        self.param_ms(inst, 1);
        self.op(I::I32LtU);
        self.op(I::LocalSet(blocked));

        self.flag(inst, 0);
        self.op(I::LocalSet(go));
        self.flag(inst, 1);
        self.op(I::LocalSet(cancel));

        // cancel wins: it drops a request that is on its way
        self.op(I::LocalGet(cancel));
        self.op(I::GlobalGet(pending));
        self.op(I::I32And);
        self.op(I::If(BlockType::Empty));
        self.restart_call(OP_CANCEL, A::C(0), A::C(0));
        self.op(I::Drop);
        self.c_i(0);
        self.op(I::GlobalSet(pending));
        self.op(I::End);

        // rising edge of go, not blocked, nothing on its way, no cancel held
        self.op(I::LocalGet(go));
        self.op(I::GlobalGet(prev));
        self.op(I::I32Eqz);
        self.op(I::I32And);
        self.op(I::LocalGet(blocked));
        self.op(I::I32Eqz);
        self.op(I::I32And);
        self.op(I::GlobalGet(pending));
        self.op(I::I32Eqz);
        self.op(I::I32And);
        self.op(I::LocalGet(cancel));
        self.op(I::I32Eqz);
        self.op(I::I32And);
        self.op(I::If(BlockType::Empty));
        self.c_i(OP_REQUEST);
        self.param_ms(inst, 0);
        self.c_i(0);
        self.hw_call(HW_RESTART);
        self.op(I::Drop);
        self.c_i(1);
        self.op(I::GlobalSet(pending));
        self.op(I::End);
        self.op(I::LocalGet(go));
        self.op(I::GlobalSet(prev));

        self.op(I::GlobalGet(pending));
        self.set_named(inst, "pending", Ty::I);
        self.op(I::LocalGet(blocked));
        self.set_named(inst, "blocked", Ty::I);
        Ok(true)
    }
}
