//! Array blocks in a module (`specs/arrays-structs`, task 4): numeric arrays (`bool`, `int`,
//! `float`, `byte`) of one sheet. An array is a region of the module memory; the initial table is a
//! data segment, so a restarted module starts from it. The code must give the same numbers as
//! `crab-sim` (`crates/crab-sim/src/arrays/`, whose module doc has the rules every block follows):
//!
//! - an index outside `0..len` never traps: a read gives 0 and `err` is true, a write is skipped and
//!   `err` is true (one unsigned compare `index <u len` covers negative indexes too);
//! - an element is stored as its type is stored (`byte` keeps the low 8 bits, `int` is 32 bits wrapping,
//!   `bool` is 0/1, `float` is `f32`);
//! - edges (`go`, `push`, `pop`) are rising edges, the previous level lives in a global of the block;
//! - loops over the elements call [`Emitter::fuel_check`] at the top (see `array_loop`).
//!
//! Arrays of strings are in `array_text.rs`; an array used by blocks of several sheets is an error
//! until the shared arrays are done (task 5).

use super::*;
use crab_core::array_blocks::{array_name, ArrayOp, DEST_PARAM};
use crab_core::arrays::ElemType;

/// Why an array that blocks of several modules use cannot be placed yet (see `array_mem`).
pub(super) const SHARED_ARRAY_REASON: &str = "массивы используют блоки нескольких модулей: листы с общим массивом объединяются в один модуль при сборке, если у них одна скорость и среди них нет листа с web_ui";

/// A numeric array placed in the memory of the module.
#[derive(Debug, Clone, Copy)]
pub(super) struct ArrayMem {
    /// Address of element 0.
    pub base: u32,
    pub len: u32,
    pub ty: ElemType,
}

impl ArrayMem {
    /// Machine type of an element: `f32` for `float`, `i32` for the others.
    pub(super) fn ty_of(self) -> Ty {
        if self.ty == ElemType::Float {
            Ty::F
        } else {
            Ty::I
        }
    }

    /// Bytes an element takes in memory: 1 for `byte`, 4 for the others.
    pub(super) fn size(self) -> u32 {
        if self.ty == ElemType::Byte {
            1
        } else {
            4
        }
    }
}

/// Little-endian bytes of the initial table: the elements of `items` (up to `len`), the rest stays
/// zero because the memory starts zeroed.
pub(super) fn initial_bytes(ty: ElemType, len: u32, items: &[ParamValue]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in items.iter().take(len as usize) {
        let x = pv(value);
        match ty {
            ElemType::Float => bytes.extend_from_slice(&(x as f32).to_le_bytes()),
            ElemType::Byte => bytes.push((x as i64 & 0xFF) as u8),
            ElemType::Bool => bytes.extend_from_slice(&i32::from(x != 0.0).to_le_bytes()),
            ElemType::Int | ElemType::String => {
                bytes.extend_from_slice(&(x as i64 as i32).to_le_bytes())
            }
        }
    }
    bytes
}

impl Emitter<'_, '_> {
    /// Emits the array block `inst`; `Ok(false)` for a block that is not an array block.
    pub(super) fn array_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let Some(op) = ArrayOp::from_kind(&inst.block.kind) else {
            return Ok(false);
        };
        let name = array_name(inst.block).unwrap_or_default().to_owned();
        let mem = self.array_mem(inst, &name)?;
        match op {
            ArrayOp::Get => self.array_get(inst, mem),
            ArrayOp::Set => self.array_set(inst, mem),
            ArrayOp::Fill
            | ArrayOp::Copy
            | ArrayOp::Sum
            | ArrayOp::Min
            | ArrayOp::Max
            | ArrayOp::Find => self.array_stats(inst, op, mem)?,
            ArrayOp::Shift | ArrayOp::Ring | ArrayOp::Stack => self.array_queue(inst, op, mem)?,
        }
        Ok(true)
    }

    /// Whether blocks of other modules use the array `name` (as the array or as a copy destination).
    pub(super) fn array_used_elsewhere(&self, name: &str) -> bool {
        let module = self.module;
        self.program.blocks.iter().enumerate().any(|(i, b)| {
            !module.blocks.contains(&i)
                && ArrayOp::from_kind(&b.block.kind).is_some()
                && (array_name(b.block) == Some(name)
                    || matches!(b.block.params.get(DEST_PARAM), Some(ParamValue::Text(d)) if d == name))
        })
    }

    /// The region of array `name` in this module: allocated at the first use, with its initial table
    /// in the data segment. Errors: no such array, a string array copied to or from a numeric one,
    /// blocks of other sheets use it.
    pub(super) fn array_mem(&mut self, inst: &Instance, name: &str) -> Result<ArrayMem, WasmError> {
        if let Some(&mem) = self.arrays.get(name) {
            return Ok(mem);
        }
        let program = self.program;
        let Some(var) = program.project.logic.array(name) else {
            return Err(self.unsupported(inst, "массив не найден"));
        };
        if var.ty == ElemType::String {
            return Err(self.unsupported(
                inst,
                "копирование между массивом строк и числовым массивом пока не поддержано в модулях",
            ));
        }
        if self.array_used_elsewhere(name) {
            return Err(self.unsupported(inst, SHARED_ARRAY_REASON));
        }
        let mem = ArrayMem {
            base: 0,
            len: var.len,
            ty: var.ty,
        };
        let base = self.alloc(var.len.saturating_mul(mem.size()));
        let bytes = initial_bytes(var.ty, var.len, &var.items);
        if !bytes.is_empty() {
            // Not `data()`: that one shares equal constants and an array is written to.
            self.data.push((base, bytes));
        }
        let mem = ArrayMem { base, ..mem };
        self.arrays.insert(name.to_owned(), mem);
        Ok(mem)
    }

    /// An index (`i32`) is on the stack: leaves its byte offset from the start of the array.
    pub(super) fn array_scale(&mut self, mem: ArrayMem) {
        if mem.size() == 4 {
            self.c_i(2);
            self.op(I::I32Shl);
        }
    }

    fn array_arg(mem: ArrayMem) -> wasm_encoder::MemArg {
        Self::mem_arg(u64::from(mem.base), if mem.size() == 4 { 2 } else { 0 })
    }

    /// The byte offset of an element (see [`Emitter::array_scale`]) is on the stack: leaves the
    /// element as a value of [`ArrayMem::ty_of`].
    pub(super) fn array_load(&mut self, mem: ArrayMem) {
        let arg = Self::array_arg(mem);
        self.op(match mem.ty {
            ElemType::Float => I::F32Load(arg),
            ElemType::Byte => I::I32Load8U(arg),
            _ => I::I32Load(arg),
        });
    }

    /// The byte offset of an element and then its new value (of [`ArrayMem::ty_of`]) are on the
    /// stack: stores the value as the element type is stored. A `bool` array gets 0 or 1 whatever
    /// the value is.
    pub(super) fn array_store(&mut self, mem: ArrayMem) {
        let arg = Self::array_arg(mem);
        if mem.ty == ElemType::Bool {
            self.flagify(Ty::I);
        }
        self.op(match mem.ty {
            ElemType::Float => I::F32Store(arg),
            ElemType::Byte => I::I32Store8(arg),
            _ => I::I32Store(arg),
        });
    }

    /// Leaves 1 when the index in local `idx` is inside the array (one unsigned compare).
    pub(super) fn array_in_range(&mut self, mem: ArrayMem, idx: u32) {
        self.op(I::LocalGet(idx));
        self.c_i(mem.len as i32);
        self.op(I::I32LtU);
    }

    /// Runs `body` once for every element: `body` gets the local that holds the index. The loop
    /// takes fuel at the top, so a long array lets the other modules run.
    pub(super) fn array_loop(&mut self, mem: ArrayMem, body: impl FnOnce(&mut Self, u32)) {
        let i = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(i));
        self.op(I::Block(BlockType::Empty));
        self.op(I::Loop(BlockType::Empty));
        self.fuel_check();
        self.op(I::LocalGet(i));
        self.c_i(mem.len as i32);
        self.op(I::I32GeU);
        self.op(I::BrIf(1));
        body(self, i);
        self.op(I::LocalGet(i));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::LocalSet(i));
        self.op(I::Br(0));
        self.op(I::End);
        self.op(I::End);
    }

    /// Leaves 1 on the scan when input `i` of the block goes from false to true; `prev` is a global
    /// of the block (from [`Emitter::state_i`]) that remembers the previous level.
    pub(super) fn rising_edge(&mut self, inst: &Instance, i: usize, prev: u32) {
        self.flag(inst, i);
        let cur = self.tmp_i();
        self.op(I::LocalSet(cur));
        self.op(I::LocalGet(cur));
        self.op(I::GlobalGet(prev));
        self.op(I::I32Eqz);
        self.op(I::I32And);
        self.op(I::LocalGet(cur));
        self.op(I::GlobalSet(prev));
    }

    /// Debug build (`watch`): reports the first `ARRAY_WATCH` elements of every array of the module
    /// like block outputs (`output_i/f`, one slot each, label `array:<name>[<i>]`) and stores what the
    /// host answers back, so a forced value reaches the array. Arrays go by name, so the slots are
    /// the same on every build. Needs the slots left under `MAX_WATCHED`.
    pub(super) fn array_watch(&mut self) {
        if !self.watch {
            return;
        }
        let mut arrays: Vec<(String, ArrayMem)> =
            self.arrays.iter().map(|(n, m)| (n.clone(), *m)).collect();
        arrays.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, mem) in arrays {
            for i in 0..mem.len.min(crab_debug::ARRAY_WATCH) {
                if self.watched.len() >= MAX_WATCHED {
                    return;
                }
                self.watched.push(crab_debug::array_label(&name, i));
                let slot = self.watched.len() - 1;
                let at = (i * mem.size()) as i32;
                // Store address, then slot and value for the host call, then the answer.
                self.c_i(at);
                self.c_i(slot as i32);
                self.c_i(at);
                self.array_load(mem);
                self.op(I::Call(if mem.ty_of() == Ty::F {
                    OUTPUT_F
                } else {
                    OUTPUT_I
                }));
                self.array_store(mem);
            }
        }
    }

    /// Counts one fault of the module (an index outside its array): the counter is a global that the
    /// module reports through its `faults()` export, and the master adds it to the failures of the
    /// module. The counter wraps; the master takes the difference between two reads.
    pub(super) fn count_fault(&mut self) {
        let counter = match self.fault_counter {
            Some(g) => g,
            None => {
                let g = self.state_i();
                self.fault_counter = Some(g);
                g
            }
        };
        self.op(I::GlobalGet(counter));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::GlobalSet(counter));
    }

    /// `array_get`: in 0 = index; out 0 = value, out 1 = err.
    fn array_get(&mut self, inst: &Instance, mem: ArrayMem) {
        let ty = mem.ty_of();
        let idx = self.tmp_i();
        self.input_as(inst, 0, Ty::I);
        self.op(I::LocalSet(idx));
        self.array_in_range(mem, idx);
        self.choose(
            ty,
            |e| {
                e.op(I::LocalGet(idx));
                e.array_scale(mem);
                e.array_load(mem);
            },
            |e| e.c(ty, 0.0),
        );
        self.set_out(inst, 0, ty);
        let err = self.tmp_i();
        self.op(I::LocalGet(idx));
        self.c_i(mem.len as i32);
        self.op(I::I32GeU);
        self.op(I::LocalSet(err));
        self.op(I::LocalGet(err));
        self.op(I::If(BlockType::Empty));
        self.count_fault();
        self.op(I::End);
        self.op(I::LocalGet(err));
        self.set_out(inst, 1, Ty::I);
    }

    /// `array_set`: in 0 = index, 1 = value; out 0 = err.
    fn array_set(&mut self, inst: &Instance, mem: ArrayMem) {
        let idx = self.tmp_i();
        self.input_as(inst, 0, Ty::I);
        self.op(I::LocalSet(idx));
        self.array_in_range(mem, idx);
        self.op(I::If(BlockType::Empty));
        self.op(I::LocalGet(idx));
        self.array_scale(mem);
        self.input_as(inst, 1, mem.ty_of());
        self.array_store(mem);
        self.op(I::Else);
        self.count_fault();
        self.op(I::End);
        self.op(I::LocalGet(idx));
        self.c_i(mem.len as i32);
        self.op(I::I32GeU);
        self.set_out(inst, 0, Ty::I);
    }
}
