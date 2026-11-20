//! Fill, copy, sum, minimum, maximum and search of an array in a module. Filled in by the task of
//! the stats group; see `array.rs` for the helpers and the rules.

use super::*;
use crab_core::array_blocks::ArrayOp;

impl Emitter<'_, '_> {
    pub(super) fn array_stats(
        &mut self,
        inst: &Instance,
        _op: ArrayOp,
        _mem: array::ArrayMem,
    ) -> Result<(), WasmError> {
        Err(self.unsupported(inst, "блок массива пока не поддержан в модулях"))
    }
}
