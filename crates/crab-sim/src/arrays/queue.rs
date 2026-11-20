//! Shift register, ring buffer and stack. Filled in by the task of the queue group.

use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::ElemType;

use super::{ArrayBehavior, Noop};

/// Behaviour of `Shift`, `Ring` and `Stack`.
pub(super) fn behavior(_op: ArrayOp, _ty: ElemType) -> Box<dyn ArrayBehavior> {
    Box::new(Noop)
}
