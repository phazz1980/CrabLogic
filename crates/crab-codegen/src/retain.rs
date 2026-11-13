//! NVS keys of retained variables (`retain`): the numeric key a module passes to the master.

use crab_core::Var;

/// FNV-1a of a text: the numeric key a module passes to the master's `retain`.
pub fn fnv(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in text.bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    hash
}

/// The key of a variable: changes with the type so a value saved under another type is not read back.
pub fn key_bits(var: &Var) -> u32 {
    fnv(&format!("{}:{}", var.name, var.ty.cpp()))
}
