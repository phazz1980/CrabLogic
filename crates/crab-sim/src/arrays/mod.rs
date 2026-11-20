//! Arrays in the simulation (`specs/arrays-structs`): the storage of an array and the behaviour of
//! the array blocks (`crab_core::array_blocks`). The simulation keeps one [`ArrayState`] per array
//! of the project; a block names its array in the parameter `array` and works on that state on
//! every scan, in the execution order, like the variable blocks do.
//!
//! Rules every block follows (they must match the module that runs on the board):
//! - An index outside `0..len` never panics: a read gives 0 (an empty string) and `err` is true, a
//!   write is skipped and `err` is true.
//! - An element is stored as its type is stored on the board, see [`ArrayState::set_num`].
//! - Edges (`go`, `push`, `pop`) are rising edges: the block remembers the previous level of the
//!   input itself, starting from false.

use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::{ArrayVar, ElemType};
use crab_core::ParamValue;

mod queue;
mod stats;

/// Values of the elements: numbers (bool as 0/1) or strings.
#[derive(Debug, Clone, PartialEq)]
pub enum Items {
    Num(Vec<f64>),
    Text(Vec<String>),
}

/// One array: element type and the values.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayState {
    pub ty: ElemType,
    pub items: Items,
}

impl ArrayState {
    /// The array after power-on: the table of initial values, the rest zero (false, empty).
    pub fn new(var: &ArrayVar) -> Self {
        let len = var.len as usize;
        let mut state = ArrayState {
            ty: var.ty,
            items: if var.ty == ElemType::String {
                Items::Text(vec![String::new(); len])
            } else {
                Items::Num(vec![0.0; len])
            },
        };
        for (i, value) in var.items.iter().take(len).enumerate() {
            match value {
                ParamValue::Text(s) => state.set_text(i, s),
                ParamValue::Bool(b) => state.set_num(i, f64::from(u8::from(*b))),
                ParamValue::Int(n) => state.set_num(i, *n as f64),
                ParamValue::Float(x) => state.set_num(i, *x),
            }
        }
        state
    }

    pub fn len(&self) -> usize {
        match &self.items {
            Items::Num(v) => v.len(),
            Items::Text(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the elements are strings.
    pub fn is_text(&self) -> bool {
        matches!(self.items, Items::Text(_))
    }

    /// Numeric element `i`; 0 for a string array or an index outside the array.
    pub fn num(&self, i: usize) -> f64 {
        match &self.items {
            Items::Num(v) => v.get(i).copied().unwrap_or(0.0),
            Items::Text(_) => 0.0,
        }
    }

    /// String element `i`; empty for a numeric array or an index outside the array.
    pub fn text(&self, i: usize) -> &str {
        match &self.items {
            Items::Text(v) => v.get(i).map_or("", String::as_str),
            Items::Num(_) => "",
        }
    }

    /// Stores a number in element `i` as the type is stored on the board: `byte` keeps the low 8
    /// bits, `int` wraps at 32 bits, `bool` is true for any non-zero value, `float` is a 32-bit
    /// float. An index outside the array or a string array: nothing happens.
    pub fn set_num(&mut self, i: usize, value: f64) {
        let ty = self.ty;
        if let Items::Num(v) = &mut self.items {
            if let Some(slot) = v.get_mut(i) {
                *slot = store(ty, value);
            }
        }
    }

    /// Stores a string in element `i`; a numeric array or an index outside the array: nothing.
    pub fn set_text(&mut self, i: usize, text: &str) {
        if let Items::Text(v) = &mut self.items {
            if let Some(slot) = v.get_mut(i) {
                *slot = text.to_owned();
            }
        }
    }

    /// Copies element `from` of `src` (same type) into element `to` of this array.
    pub fn copy_item(&mut self, to: usize, src: &ArrayState, from: usize) {
        if src.is_text() {
            self.set_text(to, src.text(from));
        } else {
            self.set_num(to, src.num(from));
        }
    }
}

/// A number as the board stores it in a slot of type `ty`: `byte` keeps the low 8 bits, `int` wraps
/// at 32 bits, `bool` is 1 for any non-zero value, `float` is a 32-bit float.
pub fn store(ty: ElemType, value: f64) -> f64 {
    match ty {
        ElemType::Byte => f64::from((value as i64 & 0xFF) as u8),
        ElemType::Int => f64::from(value as i64 as i32),
        ElemType::Bool => f64::from(u8::from(value != 0.0)),
        ElemType::Float | ElemType::String => f64::from(value as f32),
    }
}

/// What a block sees on one scan: the values of its input ports, in port order (`texts` carries
/// the strings of string ports, empty for the others).
pub struct OpCtx<'a> {
    pub inputs: &'a [f64],
    pub texts: &'a [String],
}

/// Behaviour of one array block. `arr` is the array the block names; `dest` is the array named by
/// `dest` (only `Copy`; `None` when it is missing or of another type). `out` and `texts_out` are
/// the output ports in port order, already holding the values of the previous scan.
pub trait ArrayBehavior {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        dest: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        texts_out: &mut [String],
    );
}

/// Does nothing (a block whose behaviour is not written yet).
pub(crate) struct Noop;

impl ArrayBehavior for Noop {
    fn step(
        &mut self,
        _: &mut ArrayState,
        _: Option<&mut ArrayState>,
        _: &OpCtx,
        _: &mut [f64],
        _: &mut [String],
    ) {
    }
}

/// `array_get`: out 0 = value, out 1 = err. In: 0 = index.
struct Get;

impl ArrayBehavior for Get {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        texts_out: &mut [String],
    ) {
        let index = ctx.inputs[0];
        let at = (index >= 0.0 && (index as usize) < arr.len()).then_some(index as usize);
        out[1] = f64::from(u8::from(at.is_none()));
        if arr.is_text() {
            texts_out[0] = at.map_or("", |i| arr.text(i)).to_owned();
        } else {
            out[0] = at.map_or(0.0, |i| arr.num(i));
        }
    }
}

/// `array_set`: out 0 = err. In: 0 = index, 1 = value.
struct Set;

impl ArrayBehavior for Set {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let index = ctx.inputs[0];
        let ok = index >= 0.0 && (index as usize) < arr.len();
        out[0] = f64::from(u8::from(!ok));
        if ok {
            let i = index as usize;
            if arr.is_text() {
                arr.set_text(i, &ctx.texts[1]);
            } else {
                arr.set_num(i, ctx.inputs[1]);
            }
        }
    }
}

/// Behaviour of an array block of the operation `op` on an array of `ty`.
pub fn behavior(op: ArrayOp, ty: ElemType) -> Box<dyn ArrayBehavior> {
    match op {
        ArrayOp::Get => Box::new(Get),
        ArrayOp::Set => Box::new(Set),
        ArrayOp::Fill
        | ArrayOp::Copy
        | ArrayOp::Sum
        | ArrayOp::Min
        | ArrayOp::Max
        | ArrayOp::Find => stats::behavior(op, ty),
        ArrayOp::Shift | ArrayOp::Ring | ArrayOp::Stack => queue::behavior(op, ty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(ty: ElemType, len: u32, items: Vec<ParamValue>) -> ArrayVar {
        ArrayVar {
            name: "a".into(),
            ty,
            len,
            items,
            description: String::new(),
        }
    }

    #[test]
    fn the_initial_table_fills_the_head_and_the_rest_is_zero() {
        let a = ArrayState::new(&var(
            ElemType::Byte,
            4,
            vec![ParamValue::Int(7), ParamValue::Int(300)],
        ));
        assert_eq!(a.items, Items::Num(vec![7.0, 44.0, 0.0, 0.0]));
        let s = ArrayState::new(&var(
            ElemType::String,
            2,
            vec![ParamValue::Text("hi".into())],
        ));
        assert_eq!(s.text(0), "hi");
        assert_eq!(s.text(1), "");
    }

    #[test]
    fn elements_are_stored_like_the_board_stores_them() {
        let mut a = ArrayState::new(&var(ElemType::Byte, 1, Vec::new()));
        a.set_num(0, 257.0);
        assert_eq!(a.num(0), 1.0);
        a.set_num(0, -1.0);
        assert_eq!(a.num(0), 255.0);
        let mut b = ArrayState::new(&var(ElemType::Bool, 1, Vec::new()));
        b.set_num(0, 5.0);
        assert_eq!(b.num(0), 1.0);
        let mut i = ArrayState::new(&var(ElemType::Int, 1, Vec::new()));
        i.set_num(0, 4_294_967_297.0);
        assert_eq!(i.num(0), 1.0);
        // Out of range and wrong kind do nothing.
        i.set_num(5, 1.0);
        i.set_text(0, "x");
        assert_eq!(i.num(0), 1.0);
    }

    #[test]
    fn get_and_set_report_a_bad_index() {
        let mut arr = ArrayState::new(&var(ElemType::Int, 3, Vec::new()));
        let (mut out, mut texts) = ([0.0; 2], vec![String::new(); 2]);
        let none = vec![String::new(); 2];
        Set.step(
            &mut arr,
            None,
            &OpCtx {
                inputs: &[1.0, 9.0],
                texts: &none,
            },
            &mut out,
            &mut texts,
        );
        assert_eq!((out[0], arr.num(1)), (0.0, 9.0));
        Set.step(
            &mut arr,
            None,
            &OpCtx {
                inputs: &[3.0, 5.0],
                texts: &none,
            },
            &mut out,
            &mut texts,
        );
        assert_eq!(out[0], 1.0);
        assert_eq!(arr.items, Items::Num(vec![0.0, 9.0, 0.0]));
        Get.step(
            &mut arr,
            None,
            &OpCtx {
                inputs: &[1.0],
                texts: &none,
            },
            &mut out,
            &mut texts,
        );
        assert_eq!((out[0], out[1]), (9.0, 0.0));
        Get.step(
            &mut arr,
            None,
            &OpCtx {
                inputs: &[-1.0],
                texts: &none,
            },
            &mut out,
            &mut texts,
        );
        assert_eq!((out[0], out[1]), (0.0, 1.0));
    }
}
