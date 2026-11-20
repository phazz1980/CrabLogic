//! Structure blocks in a module (`specs/arrays-structs`, task 3b): `struct_pack` writes the field
//! inputs while `go` is true, `struct_unpack` gives the fields on its outputs. A structure used by
//! the blocks of one sheet is a region of the module memory with a 4-byte slot per field; the initial
//! values are a data segment. The numbers match `crates/crab-sim/src/structs.rs`: a field is stored as
//! its type is stored (see `array.rs`). The code is straight-line, no loops.
//!
//! A structure used by blocks of several sheets is an error until the shared structures are done.

use super::array::{initial_bytes, ArrayMem};
use super::*;
use crab_core::arrays::ElemType;
use crab_core::struct_blocks::{is_struct_kind, struct_name, PACK};

impl Emitter<'_, '_> {
    /// Emits the structure block `inst`; `Ok(false)` for any other block.
    pub(super) fn struct_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        if !is_struct_kind(&inst.block.kind) {
            return Ok(false);
        }
        let name = struct_name(inst.block).unwrap_or_default().to_owned();
        let slots = self.struct_mem(inst, &name)?;
        if inst.block.kind == PACK {
            // Inputs are the fields in order, then `go`.
            self.flag(inst, slots.len());
            self.op(I::If(BlockType::Empty));
            for (i, mem) in slots.iter().enumerate() {
                self.c_i(0);
                if mem.ty == ElemType::Bool {
                    // Not a conversion: a bool field is `value != 0`, as the simulation stores it.
                    self.flag(inst, i);
                } else {
                    self.input_as(inst, i, mem.ty_of());
                }
                self.array_store(*mem);
            }
            self.op(I::End);
        } else {
            for (i, mem) in slots.iter().enumerate() {
                self.c_i(0);
                self.array_load(*mem);
                self.set_out(inst, i, mem.ty_of());
            }
        }
        Ok(true)
    }

    /// The slots of structure `name` in this module: allocated at the first use, with the initial
    /// values in the data segment. Errors: no such structure, a string field, blocks of other sheets
    /// use it.
    fn struct_mem(&mut self, inst: &Instance, name: &str) -> Result<Vec<ArrayMem>, WasmError> {
        if let Some(slots) = self.structs.get(name) {
            return Ok(slots.clone());
        }
        let program = self.program;
        let Some(var) = program.project.logic.struct_var(name) else {
            return Err(self.unsupported(inst, "структура не найдена"));
        };
        if var.fields.iter().any(|f| f.ty == ElemType::String) {
            return Err(self.unsupported(inst, "строки в структурах пока не поддержаны в модулях"));
        }
        let module = self.module;
        let used_elsewhere = program
            .blocks
            .iter()
            .enumerate()
            .any(|(i, b)| !module.blocks.contains(&i) && struct_name(b.block) == Some(name));
        if used_elsewhere {
            return Err(self.unsupported(
                inst,
                "структуру используют блоки нескольких листов: общие структуры пока не поддержаны в модулях",
            ));
        }
        let mut slots = Vec::new();
        for f in &var.fields {
            let base = self.alloc(4);
            if let Some(init) = &f.init {
                let bytes = initial_bytes(f.ty, 1, std::slice::from_ref(init));
                self.data.push((base, bytes));
            }
            slots.push(ArrayMem {
                base,
                len: 1,
                ty: f.ty,
            });
        }
        self.structs.insert(name.to_owned(), slots.clone());
        Ok(slots)
    }
}
