//! Which sheet a block is drawn on.

use crate::{Instance, Program};

/// Sheet a block is drawn on; blocks without coordinates are on sheet 1.
pub fn sheet_of(program: &Program, inst: &Instance) -> u32 {
    program
        .project
        .layout
        .blocks
        .get(&inst.block.id)
        .map_or(1, |p| p.sheet)
}
