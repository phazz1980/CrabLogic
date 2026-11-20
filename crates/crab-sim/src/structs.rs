//! Structures in the simulation (`specs/arrays-structs`, task 3б): the fields of a structure and the
//! behaviour of `struct_pack` / `struct_unpack` (`crab_core::struct_blocks`). The module that runs
//! on the board must give the same numbers: a field is stored as its type is stored there
//! ([`crate::arrays::store`]).

use crab_core::arrays::{ElemType, StructVar};
use crab_core::ParamValue;

use crate::arrays::store;

/// The values of the fields, in the order of the declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct StructState {
    pub fields: Vec<(String, ElemType, f64)>,
}

impl StructState {
    /// The structure after power-on: the initial value of every field, zero where there is none.
    pub fn new(var: &StructVar) -> Self {
        let fields = var
            .fields
            .iter()
            .map(|f| {
                let x = match &f.init {
                    Some(ParamValue::Bool(b)) => f64::from(u8::from(*b)),
                    Some(ParamValue::Int(n)) => *n as f64,
                    Some(ParamValue::Float(x)) => *x,
                    _ => 0.0,
                };
                (f.name.clone(), f.ty, store(f.ty, x))
            })
            .collect();
        StructState { fields }
    }

    /// Value of field `name`; 0 for a field that is not there.
    pub fn num(&self, name: &str) -> f64 {
        self.fields
            .iter()
            .find(|f| f.0 == name)
            .map_or(0.0, |f| f.2)
    }

    /// Stores a number in the field `name` as its type is stored; an unknown field is left alone.
    pub fn set_num(&mut self, name: &str, value: f64) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.0 == name) {
            f.2 = store(f.1, value);
        }
    }

    /// `struct_pack` on one scan: while `go` is true every field takes its input (inputs are the
    /// fields in order, then `go`).
    pub fn pack(&mut self, inputs: &[f64]) {
        let n = self.fields.len();
        if inputs.get(n).is_some_and(|go| *go == 0.0) {
            return;
        }
        for (field, value) in self.fields.iter_mut().zip(inputs) {
            field.2 = store(field.1, *value);
        }
    }

    /// `struct_unpack` on one scan: the outputs are the fields in order.
    pub fn unpack(&self, out: &mut [f64]) {
        for (slot, field) in out.iter_mut().zip(&self.fields) {
            *slot = field.2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::arrays::StructField;

    fn recipe() -> StructVar {
        StructVar {
            name: "recipe".into(),
            fields: vec![
                StructField {
                    name: "temp".into(),
                    ty: ElemType::Float,
                    init: Some(ParamValue::Float(60.5)),
                },
                StructField {
                    name: "time".into(),
                    ty: ElemType::Byte,
                    init: None,
                },
                StructField {
                    name: "fan".into(),
                    ty: ElemType::Bool,
                    init: None,
                },
            ],
            description: String::new(),
        }
    }

    #[test]
    fn it_starts_from_the_initial_values() {
        let s = StructState::new(&recipe());
        assert_eq!(s.num("temp"), 60.5);
        assert_eq!(s.num("time"), 0.0);
        assert_eq!(s.num("none"), 0.0);
    }

    #[test]
    fn pack_stores_by_type_and_waits_for_go() {
        let mut s = StructState::new(&recipe());
        // `time` is a byte, `fan` a bool; go is on.
        s.pack(&[1.0, 300.0, 7.0, 1.0]);
        assert_eq!(
            (s.num("temp"), s.num("time"), s.num("fan")),
            (1.0, 44.0, 1.0)
        );
        // go is off: nothing changes.
        s.pack(&[9.0, 9.0, 0.0, 0.0]);
        assert_eq!(
            (s.num("temp"), s.num("time"), s.num("fan")),
            (1.0, 44.0, 1.0)
        );
        let mut out = [0.0; 3];
        s.unpack(&mut out);
        assert_eq!(out, [1.0, 44.0, 1.0]);
    }
}
