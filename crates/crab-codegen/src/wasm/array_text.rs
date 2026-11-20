//! Arrays of strings in a module (`specs/arrays-structs`, task 4б). An element takes `TEXT_ELEM`
//! bytes: a 4-byte little-endian length, then `STR_CAP` bytes of text. A text longer than `STR_CAP`
//! is cut to `STR_CAP` bytes here (cut at a character boundary, as `text_buffer` does); the
//! simulation keeps all of it, so long texts differ between the two. Every element therefore holds
//! at most `STR_CAP` bytes and the length word never goes past it.
//!
//! The rules are those of `array.rs`: an index outside `0..len` never traps (a read gives an empty
//! text and `err`, a write is skipped and `err`, the module counts a fault for get and set), edges
//! are rising edges with the previous level in a global of the block, loops take fuel. The behaviour
//! to copy is `crates/crab-sim/src/arrays/` (`Items::Text`). Copies and comparisons are byte loops
//! written in wasm; no host call is needed.

use super::text::{Str, MEM_PAGE};
use super::*;
use crab_core::array_blocks::{array_name, ArrayOp, DEST_PARAM};
use crab_core::arrays::ElemType;

/// Bytes of one element: the length word, then the text. The estimate of the RAM uses the same size
/// (`crab_core::arrays::STRING_ELEM_BYTES`), checked here at compile time.
pub(super) const TEXT_ELEM: u32 = 4 + STR_CAP;
const _: () = assert!(TEXT_ELEM == crab_core::arrays::STRING_ELEM_BYTES);

/// A string array placed in the memory of the module.
#[derive(Debug, Clone, Copy)]
pub(super) struct TextArray {
    /// Address of element 0.
    pub base: u32,
    pub len: u32,
}

/// The bytes of `text` that fit an element: at most `STR_CAP`, cut at a character boundary.
fn cut(text: &str) -> &[u8] {
    let mut n = text.len().min(STR_CAP as usize);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    &text.as_bytes()[..n]
}

/// Data segments of the initial table of a string array at `base`: one (address, bytes) per given
/// element with text. The other elements stay zero (empty) because the memory starts zeroed.
pub(super) fn text_initial(base: u32, len: u32, items: &[ParamValue]) -> Vec<(u32, Vec<u8>)> {
    let mut segments = Vec::new();
    for (i, value) in items.iter().take(len as usize).enumerate() {
        let ParamValue::Text(text) = value else {
            continue;
        };
        let bytes = cut(text);
        if bytes.is_empty() {
            continue;
        }
        let mut data = (bytes.len() as u32).to_le_bytes().to_vec();
        data.extend_from_slice(bytes);
        segments.push((base + i as u32 * TEXT_ELEM, data));
    }
    segments
}

impl Emitter<'_, '_> {
    /// Emits the block `inst` when it works on a string array; `Ok(false)` for any other block.
    pub(super) fn text_array_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        let Some(op) = ArrayOp::from_kind(&inst.block.kind) else {
            return Ok(false);
        };
        let Some(name) = array_name(inst.block) else {
            return Ok(false);
        };
        let program = self.program;
        let Some(var) = program.project.logic.array(name) else {
            return Ok(false);
        };
        if var.ty != ElemType::String {
            return Ok(false);
        }
        let name = name.to_owned();
        let ta = self.text_array_mem(inst, &name)?;
        match op {
            ArrayOp::Get => self.text_get(inst, ta),
            ArrayOp::Set => self.text_set(inst, ta)?,
            ArrayOp::Fill => self.text_fill(inst, ta)?,
            ArrayOp::Copy => self.text_copy(inst, ta)?,
            ArrayOp::Find => self.text_find(inst, ta)?,
            ArrayOp::Shift => self.text_shift(inst, ta)?,
            ArrayOp::Ring => self.text_ring(inst, ta)?,
            ArrayOp::Stack => self.text_stack(inst, ta)?,
            ArrayOp::Sum | ArrayOp::Min | ArrayOp::Max => {
                return Err(
                    self.unsupported(inst, "сумма и экстремумы массива строк не существуют")
                );
            }
        }
        Ok(true)
    }

    /// The region of string array `name` in this module, with its initial table in the data segment.
    fn text_array_mem(&mut self, inst: &Instance, name: &str) -> Result<TextArray, WasmError> {
        if let Some(&ta) = self.text_arrays.get(name) {
            return Ok(ta);
        }
        let program = self.program;
        let Some(var) = program.project.logic.array(name) else {
            return Err(self.unsupported(inst, "массив не найден"));
        };
        if var.ty != ElemType::String {
            return Err(self.unsupported(
                inst,
                "копирование между массивом строк и числовым массивом пока не поддержано в модулях",
            ));
        }
        if self.array_used_elsewhere(name) {
            return Err(self.unsupported(inst, super::array::SHARED_ARRAY_REASON));
        }
        // The size is checked in 64 bits: a huge array must not overflow the allocator.
        let bytes = u64::from(var.len) * u64::from(TEXT_ELEM);
        let base = if bytes > u64::from(MEM_PAGE) {
            self.mem_overflow = true;
            0
        } else {
            self.alloc(bytes as u32)
        };
        for (at, data) in text_initial(base, var.len, &var.items) {
            self.data.push((at, data));
        }
        let ta = TextArray { base, len: var.len };
        self.text_arrays.insert(name.to_owned(), ta);
        Ok(ta)
    }

    /// Loop over the `len` elements of a string array: `body` gets the local with the index. Takes
    /// fuel at the top, like `array_loop`.
    fn text_loop(&mut self, len: u32, body: impl FnOnce(&mut Self, u32)) {
        let i = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(i));
        self.op(I::Block(BlockType::Empty));
        self.op(I::Loop(BlockType::Empty));
        self.fuel_check();
        self.op(I::LocalGet(i));
        self.c_i(len as i32);
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

    /// The address of element `index` (the closure pushes the index) in a new local.
    fn text_elem(&mut self, ta: TextArray, index: impl FnOnce(&mut Self)) -> u32 {
        index(self);
        self.c_i(TEXT_ELEM as i32);
        self.op(I::I32Mul);
        self.c_i(ta.base as i32);
        self.op(I::I32Add);
        let e = self.tmp_i();
        self.op(I::LocalSet(e));
        e
    }

    /// A new local holding `a + c`.
    fn local_plus(&mut self, a: u32, c: i32) -> u32 {
        let r = self.tmp_i();
        self.op(I::LocalGet(a));
        self.c_i(c);
        self.op(I::I32Add);
        self.op(I::LocalSet(r));
        r
    }

    /// A new local holding the constant `c`.
    fn local_const(&mut self, c: i32) -> u32 {
        let r = self.tmp_i();
        self.c_i(c);
        self.op(I::LocalSet(r));
        r
    }

    /// Copies `n` bytes from address `src` to address `dst` (all in locals), one byte per pass.
    /// The two ranges must not overlap.
    fn text_bytes(&mut self, dst: u32, src: u32, n: u32) {
        let k = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(k));
        self.op(I::Block(BlockType::Empty));
        self.op(I::Loop(BlockType::Empty));
        self.fuel_check();
        self.op(I::LocalGet(k));
        self.op(I::LocalGet(n));
        self.op(I::I32GeU);
        self.op(I::BrIf(1));
        self.op(I::LocalGet(dst));
        self.op(I::LocalGet(k));
        self.op(I::I32Add);
        self.op(I::LocalGet(src));
        self.op(I::LocalGet(k));
        self.op(I::I32Add);
        self.op(I::I32Load8U(Self::mem_arg(0, 0)));
        self.op(I::I32Store8(Self::mem_arg(0, 0)));
        self.op(I::LocalGet(k));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::LocalSet(k));
        self.op(I::Br(0));
        self.op(I::End);
        self.op(I::End);
    }

    /// Writes the text of `n` bytes (a local, at most `STR_CAP`) from `src` to element `e`.
    fn text_put(&mut self, e: u32, src: u32, n: u32) {
        self.op(I::LocalGet(e));
        self.op(I::LocalGet(n));
        self.op(I::I32Store(Self::mem_arg(0, 2)));
        let dst = self.local_plus(e, 4);
        self.text_bytes(dst, src, n);
    }

    /// Copies element `from` to element `to` (both addresses in locals).
    fn text_move(&mut self, to: u32, from: u32) {
        let n = self.tmp_i();
        self.op(I::LocalGet(from));
        self.op(I::I32Load(Self::mem_arg(0, 2)));
        self.op(I::LocalSet(n));
        let bytes = self.local_plus(from, 4);
        self.text_put(to, bytes, n);
    }

    /// Copies the text of element `e` into the buffer `out` and sets the length of `out`.
    fn text_to_buf(&mut self, out: Buf, e: u32) {
        let n = self.tmp_i();
        self.op(I::LocalGet(e));
        self.op(I::I32Load(Self::mem_arg(0, 2)));
        self.op(I::LocalSet(n));
        let bytes = self.local_plus(e, 4);
        let dst = self.local_const(out.ptr as i32);
        self.text_bytes(dst, bytes, n);
        self.op(I::LocalGet(n));
        self.op(I::GlobalSet(out.len));
    }

    /// Length of the string `s` cut to `STR_CAP`, in a new local.
    fn text_len_local(&mut self, s: Str) -> u32 {
        let n = self.tmp_i();
        self.push_arg(s.len());
        self.c_i(STR_CAP as i32);
        self.pick(Ty::I, false);
        self.op(I::LocalSet(n));
        n
    }

    /// `eq` (a local) becomes 1 when element `e` holds the `n` bytes at `src`, else 0.
    fn text_equal(&mut self, eq: u32, e: u32, src: u32, n: u32) {
        self.c_i(0);
        self.op(I::LocalSet(eq));
        self.op(I::LocalGet(e));
        self.op(I::I32Load(Self::mem_arg(0, 2)));
        self.op(I::LocalGet(n));
        self.op(I::I32Eq);
        self.op(I::If(BlockType::Empty));
        self.c_i(1);
        self.op(I::LocalSet(eq));
        let bytes = self.local_plus(e, 4);
        let k = self.tmp_i();
        self.c_i(0);
        self.op(I::LocalSet(k));
        self.op(I::Block(BlockType::Empty));
        self.op(I::Loop(BlockType::Empty));
        self.fuel_check();
        self.op(I::LocalGet(k));
        self.op(I::LocalGet(n));
        self.op(I::I32GeU);
        self.op(I::BrIf(1));
        self.op(I::LocalGet(bytes));
        self.op(I::LocalGet(k));
        self.op(I::I32Add);
        self.op(I::I32Load8U(Self::mem_arg(0, 0)));
        self.op(I::LocalGet(src));
        self.op(I::LocalGet(k));
        self.op(I::I32Add);
        self.op(I::I32Load8U(Self::mem_arg(0, 0)));
        self.op(I::I32Ne);
        self.op(I::If(BlockType::Empty));
        self.c_i(0);
        self.op(I::LocalSet(eq));
        // Leaves the byte loop (if, loop, block: depth 2).
        self.op(I::Br(2));
        self.op(I::End);
        self.op(I::LocalGet(k));
        self.c_i(1);
        self.op(I::I32Add);
        self.op(I::LocalSet(k));
        self.op(I::Br(0));
        self.op(I::End);
        self.op(I::End);
        self.op(I::End);
    }

    /// Index input `idx` against the length `len`: `err` is set when it is outside (and counted as a
    /// fault), the flag goes to output `out`.
    fn text_index_err(&mut self, inst: &Instance, idx: u32, len: u32, out: usize) {
        let err = self.tmp_i();
        self.op(I::LocalGet(idx));
        self.c_i(len as i32);
        self.op(I::I32GeU);
        self.op(I::LocalSet(err));
        self.op(I::LocalGet(err));
        self.op(I::If(BlockType::Empty));
        self.count_fault();
        self.op(I::End);
        self.op(I::LocalGet(err));
        self.set_out(inst, out, Ty::I);
    }

    /// `array_get` on strings: in 0 = index; out 0 = text, out 1 = err.
    fn text_get(&mut self, inst: &Instance, ta: TextArray) {
        let idx = self.tmp_i();
        self.input_as(inst, 0, Ty::I);
        self.op(I::LocalSet(idx));
        if let Some(out) = self.out_buf(inst, "out") {
            self.op(I::LocalGet(idx));
            self.c_i(ta.len as i32);
            self.op(I::I32LtU);
            self.op(I::If(BlockType::Empty));
            let e = self.text_elem(ta, |e| e.op(I::LocalGet(idx)));
            self.text_to_buf(out, e);
            self.op(I::Else);
            self.c_i(0);
            self.op(I::GlobalSet(out.len));
            self.op(I::End);
        }
        self.text_index_err(inst, idx, ta.len, 1);
    }

    /// `array_set` on strings: in 0 = index, 1 = value; out 0 = err.
    fn text_set(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let idx = self.tmp_i();
        self.input_as(inst, 0, Ty::I);
        self.op(I::LocalSet(idx));
        let value = self.str_in(inst, "value")?;
        self.op(I::LocalGet(idx));
        self.c_i(ta.len as i32);
        self.op(I::I32LtU);
        self.op(I::If(BlockType::Empty));
        let e = self.text_elem(ta, |e| e.op(I::LocalGet(idx)));
        let n = self.text_len_local(value);
        let src = self.local_const(value.ptr as i32);
        self.text_put(e, src, n);
        self.op(I::End);
        self.text_index_err(inst, idx, ta.len, 0);
        Ok(())
    }

    /// `array_fill` on strings: in 0 = value, 1 = go. While `go` is on every element gets the text.
    fn text_fill(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let value = self.str_in(inst, "value")?;
        self.flag(inst, 1);
        self.op(I::If(BlockType::Empty));
        let n = self.text_len_local(value);
        let src = self.local_const(value.ptr as i32);
        self.text_loop(ta.len, |e, i| {
            let el = e.text_elem(ta, |e| e.op(I::LocalGet(i)));
            e.text_put(el, src, n);
        });
        self.op(I::End);
        Ok(())
    }

    /// `array_copy` on strings: in 0 = go (rising edge), out 0 = err. The destination must be an array
    /// of strings; the first `min(len_src, len_dest)` elements are copied.
    fn text_copy(&mut self, inst: &Instance, src: TextArray) -> Result<(), WasmError> {
        let Some(ParamValue::Text(name)) = inst.block.params.get(DEST_PARAM) else {
            return Err(self.unsupported(inst, "не задан массив-приёмник"));
        };
        let name = name.clone();
        let program = self.program;
        match program.project.logic.array(&name) {
            Some(var) if var.ty == ElemType::String => {}
            _ => return Err(self.unsupported(
                inst,
                "копирование между массивом строк и числовым массивом пока не поддержано в модулях",
            )),
        }
        let dest = self.text_array_mem(inst, &name)?;
        let both = src.len.min(dest.len);
        let prev = self.state_i();
        self.rising_edge(inst, 0, prev);
        self.op(I::If(BlockType::Empty));
        self.text_loop(both, |e, i| {
            let to = e.text_elem(dest, |e| e.op(I::LocalGet(i)));
            let from = e.text_elem(src, |e| e.op(I::LocalGet(i)));
            e.text_move(to, from);
        });
        self.op(I::End);
        // The destination exists in a module (the check guarantees it): no error.
        self.c_i(0);
        self.set_out(inst, 0, Ty::I);
        Ok(())
    }

    /// `array_find` on strings: in 0 = value; out 0 = index of the first equal element or -1, out 1 =
    /// found. Equal means the same length and the same bytes.
    fn text_find(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let wanted = self.str_in(inst, "value")?;
        let n = self.text_len_local(wanted);
        let src = self.local_const(wanted.ptr as i32);
        let at = self.local_const(-1);
        let found = self.local_const(0);
        let eq = self.tmp_i();
        self.text_loop(ta.len, |e, i| {
            // Only until the first hit.
            e.op(I::LocalGet(found));
            e.op(I::I32Eqz);
            e.op(I::If(BlockType::Empty));
            let el = e.text_elem(ta, |e| e.op(I::LocalGet(i)));
            e.text_equal(eq, el, src, n);
            e.op(I::LocalGet(eq));
            e.op(I::If(BlockType::Empty));
            e.op(I::LocalGet(i));
            e.op(I::LocalSet(at));
            e.c_i(1);
            e.op(I::LocalSet(found));
            e.op(I::End);
            e.op(I::End);
        });
        self.op(I::LocalGet(at));
        self.set_out(inst, 0, Ty::I);
        self.op(I::LocalGet(found));
        self.set_out(inst, 1, Ty::I);
        Ok(())
    }

    /// `array_shift` on strings: in 0 = value, 1 = go. On the rising edge of `go` every element takes
    /// the one below it (from the last down) and `value` goes to element 0. An empty array is left alone.
    fn text_shift(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let value = self.str_in(inst, "value")?;
        let prev = self.state_i();
        self.rising_edge(inst, 1, prev);
        self.op(I::If(BlockType::Empty));
        if ta.len > 0 {
            let i = self.tmp_i();
            self.c_i(ta.len as i32 - 1);
            self.op(I::LocalSet(i));
            self.op(I::Block(BlockType::Empty));
            self.op(I::Loop(BlockType::Empty));
            self.fuel_check();
            self.op(I::LocalGet(i));
            self.c_i(1);
            self.op(I::I32LtS);
            self.op(I::BrIf(1));
            let to = self.text_elem(ta, |e| e.op(I::LocalGet(i)));
            let from = self.text_elem(ta, |e| {
                e.op(I::LocalGet(i));
                e.c_i(1);
                e.op(I::I32Sub);
            });
            self.text_move(to, from);
            self.op(I::LocalGet(i));
            self.c_i(1);
            self.op(I::I32Sub);
            self.op(I::LocalSet(i));
            self.op(I::Br(0));
            self.op(I::End);
            self.op(I::End);
            let n = self.text_len_local(value);
            let src = self.local_const(value.ptr as i32);
            let first = self.text_elem(ta, |e| e.c_i(0));
            self.text_put(first, src, n);
        }
        self.op(I::End);
        Ok(())
    }

    /// `array_ring` on strings: in 0 = value, 1 = push; out 0 = head, 1 = count. On the rising edge of
    /// `push` the text goes to the head, the head moves on (wrapping) and the count grows up to the length.
    fn text_ring(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let value = self.str_in(inst, "value")?;
        let prev = self.state_i();
        let head = self.state_i();
        let count = self.state_i();
        let len = ta.len as i32;
        self.rising_edge(inst, 1, prev);
        self.op(I::If(BlockType::Empty));
        if ta.len > 0 {
            let n = self.text_len_local(value);
            let src = self.local_const(value.ptr as i32);
            let e = self.text_elem(ta, |e| e.op(I::GlobalGet(head)));
            self.text_put(e, src, n);
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
        }
        self.op(I::End);
        self.op(I::GlobalGet(head));
        self.set_out(inst, 0, Ty::I);
        self.op(I::GlobalGet(count));
        self.set_out(inst, 1, Ty::I);
        Ok(())
    }

    /// `array_stack` on strings: in 0 = value, 1 = push, 2 = pop; out 0 = top text (empty when the
    /// stack is empty), 1 = count, 2 = empty, 3 = full, 4 = err. Push is done before pop in the same
    /// scan; a refused push (full) or pop (empty) raises `err` for this scan only.
    fn text_stack(&mut self, inst: &Instance, ta: TextArray) -> Result<(), WasmError> {
        let len = ta.len as i32;
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

        // Push: the element at `count` takes the text, count + 1; a full stack refuses it.
        self.op(I::LocalGet(push_edge));
        self.op(I::If(BlockType::Empty));
        self.op(I::GlobalGet(count));
        self.c_i(len);
        self.op(I::I32LtU);
        self.op(I::If(BlockType::Empty));
        let value = self.str_in(inst, "value")?;
        let n = self.text_len_local(value);
        let src = self.local_const(value.ptr as i32);
        let e = self.text_elem(ta, |e| e.op(I::GlobalGet(count)));
        self.text_put(e, src, n);
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

        // Top text when there is one, else empty.
        if let Some(out) = self.out_buf(inst, "out") {
            self.op(I::GlobalGet(count));
            self.op(I::If(BlockType::Empty));
            let top = self.text_elem(ta, |e| {
                e.op(I::GlobalGet(count));
                e.c_i(1);
                e.op(I::I32Sub);
            });
            self.text_to_buf(out, top);
            self.op(I::Else);
            self.c_i(0);
            self.op(I::GlobalSet(out.len));
            self.op(I::End);
        }
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
        Ok(())
    }
}
