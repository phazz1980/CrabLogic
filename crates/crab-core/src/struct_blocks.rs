//! Blocks of the structures (`specs/arrays-structs`, task 3б): `struct_pack` writes the fields of a
//! structure from its inputs, `struct_unpack` gives the fields on its outputs. A block names its
//! structure in the parameter `struct`, and its ports follow the fields of that structure, so the
//! library keeps only a base block (no ports) and the real definition is derived per structure
//! ([`derive`]).
//!
//! Only scalars go over the wires: one port per field. The structure itself is a project object (see
//! [`crate::arrays`]) that the blocks reach by name, like the arrays.
//!
//! Strings are not supported in a structure that blocks use (the check says so), and a field may not
//! be called `go`: that is the enable input of `struct_pack`.

use std::collections::BTreeMap;

use crate::array_blocks::{port, text_param};
use crate::arrays::{ElemType, StructVar};
use crate::library::{BlockDef, BlockInfo, LibraryBlock};
use crate::{Block, Logic, ParamValue};

/// Parameter of the structure blocks naming the structure.
pub const STRUCT_PARAM: &str = "struct";
/// Writes the inputs into the fields of the structure.
pub const PACK: &str = "struct_pack";
/// Gives the fields of the structure on the outputs.
pub const UNPACK: &str = "struct_unpack";
/// Enable input of `struct_pack`: the fields are written while it is true.
pub const GO: &str = "go";

pub fn is_struct_kind(kind: &str) -> bool {
    kind == PACK || kind == UNPACK
}

/// Structure named by the block, or `None` for any other block.
pub fn struct_name(block: &Block) -> Option<&str> {
    if !is_struct_kind(&block.kind) {
        return None;
    }
    match block.params.get(STRUCT_PARAM) {
        Some(ParamValue::Text(name)) => Some(name),
        _ => None,
    }
}

/// Name and description of a structure block in Russian (the library on the right lists them).
pub fn caption(kind: &str) -> (&'static str, &'static str) {
    if kind == PACK {
        (
            "Структура: запись",
            "Пока вход go включён, записывает входы в поля структуры.",
        )
    } else {
        ("Структура: чтение", "Отдаёт поля структуры на выходы.")
    }
}

fn base(kind: &str) -> LibraryBlock {
    let (name, description) = caption(kind);
    LibraryBlock {
        def: BlockDef {
            info: BlockInfo {
                kind: kind.to_owned(),
                symbol: None,
                symbol_param: None,
                name: name.to_owned(),
                group: "Массивы".to_owned(),
                shared: String::new(),
                description: description.to_owned(),
                // Reading gives the value of the previous scan, so loops through a structure are
                // allowed, like through a variable.
                memory: kind == UNPACK,
                single: false,
                hidden: false,
                pullable: false,
                uart: None,
                board: None,
                code_of: None,
                expander: false,
            },
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: vec![text_param(STRUCT_PARAM, "Имя структуры")],
        },
        dir: Default::default(),
    }
}

/// The two blocks without ports: what the library lists and `add_block` creates.
pub(crate) fn base_blocks() -> BTreeMap<String, LibraryBlock> {
    [PACK, UNPACK]
        .into_iter()
        .map(|k| (k.to_owned(), base(k)))
        .collect()
}

fn named<'a>(logic: &'a Logic, block: &Block) -> Option<&'a StructVar> {
    logic.struct_var(struct_name(block)?)
}

/// Key of the derived definition: the kind and the fields it was derived from, so a changed
/// structure gives a new entry. `None` for a block without a known structure.
pub(crate) fn derived_key(logic: &Logic, block: &Block) -> Option<String> {
    let st = named(logic, block)?;
    let mut key = format!("{}|{}", block.kind, st.name);
    for f in &st.fields {
        key.push_str(&format!("|{}:{}={:?}", f.name, f.ty.name(), f.init));
    }
    Some(key)
}

/// The block with a port per field of its structure.
pub(crate) fn derive(logic: &Logic, block: &Block) -> Option<LibraryBlock> {
    let st = named(logic, block)?;
    let mut derived = base(&block.kind);
    for f in &st.fields {
        let signal = f.ty.signal();
        if block.kind == PACK {
            let mut input = port(&f.name, signal, "Значение поля", true);
            // A field without a wire gets its initial value (zero when there is none).
            input.default = match (f.ty, f.init.clone()) {
                (ElemType::String, _) => None,
                (_, Some(v)) => Some(v),
                (ElemType::Bool, None) => Some(ParamValue::Bool(false)),
                (ElemType::Float, None) => Some(ParamValue::Float(0.0)),
                (_, None) => Some(ParamValue::Int(0)),
            };
            derived.def.inputs.push(input);
        } else {
            derived
                .def
                .outputs
                .push(port(&f.name, signal, "Значение поля", false));
        }
    }
    if block.kind == PACK {
        let mut go = port(
            GO,
            crate::SignalType::Bool,
            "Записывать, пока включено",
            true,
        );
        go.default = Some(ParamValue::Bool(true));
        derived.def.inputs.push(go);
    }
    Some(derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrays::StructField;
    use crate::{Library, SignalType};

    fn logic() -> Logic {
        Logic {
            structs: vec![StructVar {
                name: "recipe".into(),
                fields: vec![
                    StructField {
                        name: "temp".into(),
                        ty: ElemType::Float,
                        init: Some(ParamValue::Float(60.0)),
                    },
                    StructField {
                        name: "time".into(),
                        ty: ElemType::Int,
                        init: None,
                    },
                    StructField {
                        name: "fan".into(),
                        ty: ElemType::Bool,
                        init: None,
                    },
                ],
                description: String::new(),
            }],
            ..Default::default()
        }
    }

    fn block(kind: &str, st: &str) -> Block {
        let mut b = Block {
            id: "b1".into(),
            kind: kind.into(),
            ..Default::default()
        };
        b.params
            .insert(STRUCT_PARAM.into(), ParamValue::Text(st.into()));
        b
    }

    #[test]
    fn the_ports_follow_the_fields() {
        let lib = Library::default();
        let l = logic();
        let pack = lib.resolve(&l, &block(PACK, "recipe")).unwrap();
        let ins: Vec<_> = pack
            .def
            .inputs
            .iter()
            .map(|p| (p.name.as_str(), p.ty))
            .collect();
        assert_eq!(
            ins,
            [
                ("temp", SignalType::Float),
                ("time", SignalType::Int),
                ("fan", SignalType::Bool),
                ("go", SignalType::Bool)
            ]
        );
        // A field without a wire keeps its initial value; `go` is on.
        assert_eq!(pack.def.inputs[0].default, Some(ParamValue::Float(60.0)));
        assert_eq!(pack.def.inputs[3].default, Some(ParamValue::Bool(true)));
        assert!(pack.def.outputs.is_empty());
        let unpack = lib.resolve(&l, &block(UNPACK, "recipe")).unwrap();
        assert_eq!(unpack.def.outputs.len(), 3);
        assert_eq!(unpack.def.outputs[2].ty, SignalType::Bool);
        assert!(unpack.def.inputs.is_empty());
        assert!(unpack.def.info.memory);
    }

    #[test]
    fn an_unknown_structure_leaves_the_base_block() {
        let lib = Library::default();
        let l = logic();
        let b = lib.resolve(&l, &block(PACK, "nothing")).unwrap();
        assert!(b.def.inputs.is_empty() && b.def.outputs.is_empty());
        assert_eq!(b.def.params[0].name, STRUCT_PARAM);
    }

    #[test]
    fn a_changed_structure_gives_new_ports() {
        let lib = Library::default();
        let mut l = logic();
        let b = block(UNPACK, "recipe");
        assert_eq!(lib.resolve(&l, &b).unwrap().def.outputs.len(), 3);
        l.structs[0].fields.pop();
        assert_eq!(lib.resolve(&l, &b).unwrap().def.outputs.len(), 2);
    }
}
