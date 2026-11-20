//! Fill, copy, sum, minimum, maximum and search. Filled in by the task of the stats group.

use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::ElemType;

use super::{ArrayBehavior, Noop};

/// Behaviour of `Fill`, `Copy`, `Sum`, `Min`, `Max` and `Find`.
pub(super) fn behavior(_op: ArrayOp, _ty: ElemType) -> Box<dyn ArrayBehavior> {
    Box::new(Noop)
}
