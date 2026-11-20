//! Shift register, ring buffer and stack of an array in a module. Filled in by the task of the
//! queue group; see `array.rs` for the helpers and the rules.

use super::*;
use crab_core::array_blocks::ArrayOp;

impl Emitter<'_, '_> {
    pub(super) fn array_queue(
        &mut self,
        inst: &Instance,
        _op: ArrayOp,
        _mem: array::ArrayMem,
    ) -> Result<(), WasmError> {
        Err(self.unsupported(inst, "блок массива пока не поддержан в модулях"))
    }
}
