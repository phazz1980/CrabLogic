//! Arrays and structures (`specs/arrays-structs`): names, length, initial values, fields and the
//! length limit of the chip. Checks of the blocks that reach an array come with the blocks.

use std::collections::HashSet;

use super::*;
use crate::arrays::{max_len, MAX_LEN};
use crate::pins::Chip;

fn error(report: &mut Report, message: String) {
    report.push(DiagKind::Array, Severity::Error, None, None, message);
}

/// What does not depend on the board: names, length, initial values, fields.
pub(super) fn check_arrays(logic: &Logic, report: &mut Report) {
    // Names are shared with the variables: a block names any of them in one parameter.
    let mut names: HashSet<&str> = logic.vars.iter().map(|v| v.name.as_str()).collect();
    for array in &logic.arrays {
        let name = array.name.as_str();
        if !crate::vars::is_valid_var_name(name) {
            error(report, format!("недопустимое имя массива «{name}»"));
        } else if !names.insert(name) {
            error(
                report,
                format!("имя «{name}» уже занято переменной, массивом или структурой"),
            );
        }
        if array.len == 0 {
            error(
                report,
                format!("массив «{name}»: длина должна быть больше нуля"),
            );
        } else if array.len > MAX_LEN {
            error(
                report,
                format!(
                    "массив «{name}»: длина {} больше предела {MAX_LEN} элементов",
                    array.len
                ),
            );
        }
        if array.items.len() > array.len as usize {
            error(
                report,
                format!(
                    "массив «{name}»: начальных значений {} больше длины {}",
                    array.items.len(),
                    array.len
                ),
            );
        }
        if let Some(i) = array.items.iter().position(|v| !array.ty.accepts(v)) {
            error(
                report,
                format!(
                    "массив «{name}»: начальное значение элемента {i} не подходит к типу {}",
                    array.ty.name()
                ),
            );
        }
    }
    for st in &logic.structs {
        let name = st.name.as_str();
        if !crate::vars::is_valid_var_name(name) {
            error(report, format!("недопустимое имя структуры «{name}»"));
        } else if !names.insert(name) {
            error(
                report,
                format!("имя «{name}» уже занято переменной, массивом или структурой"),
            );
        }
        if st.fields.is_empty() {
            error(report, format!("структура «{name}»: нет ни одного поля"));
        }
        let mut fields = HashSet::new();
        for field in &st.fields {
            let f = field.name.as_str();
            if !crate::vars::is_valid_var_name(f) {
                error(
                    report,
                    format!("структура «{name}»: недопустимое имя поля «{f}»"),
                );
            } else if !fields.insert(f) {
                error(
                    report,
                    format!("структура «{name}»: поле «{f}» объявлено дважды"),
                );
            }
            if field.init.as_ref().is_some_and(|v| !field.ty.accepts(v)) {
                error(
                    report,
                    format!(
                        "структура «{name}»: начальное значение поля «{f}» не подходит к типу {}",
                        field.ty.name()
                    ),
                );
            }
        }
    }
}

/// The blocks that reach an array: it exists, the destination of a copy is another array of the same
/// type, sum / min / max have numbers to work on, a constant index is inside the array, and the
/// array is written from one block only (like a variable: the order of two writers would be luck).
pub(super) fn check_array_blocks(logic: &Logic, report: &mut Report) {
    use crate::array_blocks::{ArrayOp, ARRAY_PARAM, DEST_PARAM};
    // Remembers who writes an array first; a second writer is an error.
    fn claim<'a>(
        writers: &mut std::collections::HashMap<&'a str, &'a str>,
        array: &'a str,
        block: &'a str,
        report: &mut Report,
    ) {
        match writers.get(array) {
            Some(prev) => report.block_error(
                DiagKind::Array,
                block,
                format!("массив «{array}» уже записывается блоком «{prev}»"),
            ),
            None => {
                writers.insert(array, block);
            }
        }
    }
    let mut writers: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for block in logic.blocks.iter().filter(|b| b.tag.is_none()) {
        let Some(op) = ArrayOp::from_kind(&block.kind) else {
            continue;
        };
        let name = match block.params.get(ARRAY_PARAM) {
            Some(ParamValue::Text(n)) if !n.is_empty() => n.as_str(),
            _ => {
                report.block_error(
                    DiagKind::Array,
                    &block.id,
                    format!("блок «{}»: не выбран массив", block.id),
                );
                continue;
            }
        };
        let Some(array) = logic.array(name) else {
            report.block_error(
                DiagKind::Array,
                &block.id,
                format!("блок «{}»: нет массива «{name}»", block.id),
            );
            continue;
        };
        if !op.exists_for(array.ty) {
            report.block_error(
                DiagKind::Array,
                &block.id,
                format!(
                    "блок «{}»: сумма, минимум и максимум работают с массивами int, float и byte, а «{name}» — {}",
                    block.id,
                    array.ty.name()
                ),
            );
        }
        if matches!(op, ArrayOp::Get | ArrayOp::Set) {
            let fed = logic
                .links
                .iter()
                .any(|l| l.to.block == block.id && l.to.port == "index");
            let constant = match block.consts.get("index") {
                Some(ParamValue::Int(n)) => Some(*n as f64),
                Some(ParamValue::Float(x)) => Some(*x),
                _ => None,
            };
            if let Some(i) = constant.filter(|i| !fed && (*i < 0.0 || *i >= f64::from(array.len))) {
                report.block_error(
                    DiagKind::Array,
                    &block.id,
                    format!(
                        "блок «{}»: индекс {i} вне массива «{name}» длиной {}",
                        block.id, array.len
                    ),
                );
            }
        }
        // Writers: the array of the block, and for a copy the destination.
        if matches!(
            op,
            ArrayOp::Set | ArrayOp::Fill | ArrayOp::Shift | ArrayOp::Ring | ArrayOp::Stack
        ) {
            claim(&mut writers, name, &block.id, report);
        }
        if op == ArrayOp::Copy {
            let dest = match block.params.get(DEST_PARAM) {
                Some(ParamValue::Text(n)) if !n.is_empty() => n.as_str(),
                _ => {
                    report.block_error(
                        DiagKind::Array,
                        &block.id,
                        format!("блок «{}»: не выбран массив-приёмник", block.id),
                    );
                    continue;
                }
            };
            match logic.array(dest) {
                None => report.block_error(
                    DiagKind::Array,
                    &block.id,
                    format!("блок «{}»: нет массива-приёмника «{dest}»", block.id),
                ),
                Some(_) if dest == name => report.block_error(
                    DiagKind::Array,
                    &block.id,
                    format!(
                        "блок «{}»: массив нельзя копировать в самого себя",
                        block.id
                    ),
                ),
                Some(d) if d.ty != array.ty => report.block_error(
                    DiagKind::Array,
                    &block.id,
                    format!(
                        "блок «{}»: «{name}» ({}) и «{dest}» ({}) разных типов",
                        block.id,
                        array.ty.name(),
                        d.ty.name()
                    ),
                ),
                Some(_) => {
                    claim(&mut writers, dest, &block.id, report);
                }
            }
        }
    }
}

/// Length against the limit of the chip of the project (64 on the ESP32-C3).
pub(super) fn check_array_limits(logic: &Logic, chip: Option<Chip>, report: &mut Report) {
    let limit = max_len(chip);
    if limit >= MAX_LEN {
        return;
    }
    for array in logic.arrays.iter().filter(|a| a.len > limit) {
        error(
            report,
            format!(
                "массив «{}»: длина {} больше предела {limit} элементов для этой платы",
                array.name, array.len
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrays::{ArrayVar, ElemType, StructField, StructVar};

    fn array(name: &str, ty: ElemType, len: u32, items: Vec<ParamValue>) -> ArrayVar {
        ArrayVar {
            name: name.into(),
            ty,
            len,
            items,
            description: String::new(),
        }
    }

    fn messages(logic: &Logic) -> Vec<String> {
        let mut report = Report::default();
        check_arrays(logic, &mut report);
        report.diagnostics.into_iter().map(|d| d.message).collect()
    }

    #[test]
    fn a_good_array_and_struct_pass() {
        let logic = Logic {
            arrays: vec![array(
                "окно",
                ElemType::Float,
                8,
                vec![ParamValue::Float(1.0), ParamValue::Int(2)],
            )],
            structs: vec![StructVar {
                name: "рецепт".into(),
                fields: vec![StructField {
                    name: "temp".into(),
                    ty: ElemType::Float,
                    init: Some(ParamValue::Float(60.0)),
                }],
                description: String::new(),
            }],
            ..Default::default()
        };
        assert!(messages(&logic).is_empty());
    }

    #[test]
    fn names_are_shared_with_variables() {
        let mut logic = Logic {
            arrays: vec![
                array("a", ElemType::Int, 4, Vec::new()),
                array("a", ElemType::Int, 4, Vec::new()),
                array("", ElemType::Int, 4, Vec::new()),
            ],
            ..Default::default()
        };
        logic.vars.push(crate::Var {
            name: "t".into(),
            ty: crate::VarType::Int,
            init: None,
            description: String::new(),
            retain: false,
            service: false,
            settle_s: None,
            period_s: None,
        });
        logic.arrays.push(array("t", ElemType::Int, 4, Vec::new()));
        let m = messages(&logic);
        assert_eq!(
            m.iter().filter(|x| x.contains("уже занято")).count(),
            2,
            "{m:?}"
        );
        assert_eq!(
            m.iter().filter(|x| x.contains("недопустимое имя")).count(),
            1,
            "{m:?}"
        );
    }

    #[test]
    fn length_and_initial_values_are_checked() {
        let logic = Logic {
            arrays: vec![
                array("z", ElemType::Int, 0, Vec::new()),
                array("big", ElemType::Byte, 257, Vec::new()),
                array("many", ElemType::Int, 2, vec![ParamValue::Int(1); 3]),
                array(
                    "bad",
                    ElemType::Byte,
                    4,
                    vec![ParamValue::Int(1), ParamValue::Int(300)],
                ),
            ],
            ..Default::default()
        };
        let m = messages(&logic);
        assert!(m.iter().any(|x| x.contains("больше нуля")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("больше предела 256")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("больше длины")), "{m:?}");
        assert!(
            m.iter().any(|x| x.contains("элемента 1 не подходит")),
            "{m:?}"
        );
    }

    #[test]
    fn struct_fields_are_checked() {
        let field = |name: &str, init| StructField {
            name: name.into(),
            ty: ElemType::Bool,
            init,
        };
        let logic = Logic {
            structs: vec![
                StructVar {
                    name: "пусто".into(),
                    fields: Vec::new(),
                    description: String::new(),
                },
                StructVar {
                    name: "s".into(),
                    fields: vec![
                        field("a", None),
                        field("a", None),
                        field("b", Some(ParamValue::Int(5))),
                    ],
                    description: String::new(),
                },
            ],
            ..Default::default()
        };
        let m = messages(&logic);
        assert!(m.iter().any(|x| x.contains("нет ни одного поля")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("объявлено дважды")), "{m:?}");
        assert!(
            m.iter().any(|x| x.contains("поля «b» не подходит")),
            "{m:?}"
        );
    }

    fn block(id: &str, kind: &str, array: &str) -> crate::Block {
        let mut b = crate::Block {
            id: id.into(),
            kind: kind.into(),
            ..Default::default()
        };
        b.params
            .insert("array".into(), ParamValue::Text(array.into()));
        b
    }

    fn block_messages(logic: &Logic) -> Vec<String> {
        let mut report = Report::default();
        check_array_blocks(logic, &mut report);
        report.diagnostics.into_iter().map(|d| d.message).collect()
    }

    fn with_blocks(arrays: Vec<ArrayVar>, blocks: Vec<crate::Block>) -> Logic {
        Logic {
            arrays,
            blocks,
            ..Default::default()
        }
    }

    #[test]
    fn blocks_need_an_existing_array() {
        let mut none = block("n", "array_get", "");
        none.params.remove("array");
        let logic = with_blocks(
            vec![array("a", ElemType::Int, 4, Vec::new())],
            vec![
                none,
                block("m", "array_get", "нет"),
                block("ok", "array_get", "a"),
            ],
        );
        let m = block_messages(&logic);
        assert!(m.iter().any(|x| x.contains("не выбран массив")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("нет массива «нет»")), "{m:?}");
        assert_eq!(m.len(), 2, "{m:?}");
    }

    #[test]
    fn a_constant_index_must_be_inside() {
        let mut far = block("far", "array_get", "a");
        far.consts.insert("index".into(), ParamValue::Int(4));
        let mut ok = block("ok", "array_get", "a");
        ok.consts.insert("index".into(), ParamValue::Int(3));
        let mut neg = block("neg", "array_set", "a");
        neg.consts.insert("index".into(), ParamValue::Int(-1));
        let logic = with_blocks(
            vec![array("a", ElemType::Int, 4, Vec::new())],
            vec![far, ok, neg],
        );
        let m = block_messages(&logic);
        assert_eq!(
            m.iter().filter(|x| x.contains("вне массива")).count(),
            2,
            "{m:?}"
        );
    }

    #[test]
    fn an_array_is_written_by_one_block() {
        let logic = with_blocks(
            vec![
                array("a", ElemType::Int, 4, Vec::new()),
                array("b", ElemType::Int, 4, Vec::new()),
            ],
            vec![
                block("w1", "array_set", "a"),
                block("w2", "array_fill", "a"),
                block("r", "array_get", "a"),
                block("only", "array_set", "b"),
            ],
        );
        let m = block_messages(&logic);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].contains("уже записывается блоком «w1»"), "{m:?}");
    }

    #[test]
    fn copy_needs_another_array_of_the_same_type() {
        let copy = |id: &str, dest: Option<&str>| {
            let mut b = block(id, "array_copy", "a");
            if let Some(d) = dest {
                b.params.insert("dest".into(), ParamValue::Text(d.into()));
            }
            b
        };
        let logic = with_blocks(
            vec![
                array("a", ElemType::Int, 4, Vec::new()),
                array("b", ElemType::Int, 4, Vec::new()),
                array("f", ElemType::Float, 4, Vec::new()),
            ],
            vec![
                copy("ok", Some("b")),
                copy("same", Some("a")),
                copy("other", Some("f")),
                copy("lost", Some("x")),
                copy("nodest", None),
            ],
        );
        let m = block_messages(&logic);
        assert!(m.iter().any(|x| x.contains("в самого себя")), "{m:?}");
        assert!(m.iter().any(|x| x.contains("разных типов")), "{m:?}");
        assert!(
            m.iter().any(|x| x.contains("нет массива-приёмника")),
            "{m:?}"
        );
        assert!(
            m.iter().any(|x| x.contains("не выбран массив-приёмник")),
            "{m:?}"
        );
        assert_eq!(m.len(), 4, "{m:?}");
    }

    #[test]
    fn sum_needs_numbers() {
        let logic = with_blocks(
            vec![
                array("s", ElemType::String, 2, Vec::new()),
                array("n", ElemType::Byte, 2, Vec::new()),
            ],
            vec![
                block("bad", "array_sum", "s"),
                block("ok", "array_max", "n"),
            ],
        );
        let m = block_messages(&logic);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].contains("int, float и byte"), "{m:?}");
    }

    #[test]
    fn the_c3_takes_shorter_arrays() {
        let logic = Logic {
            arrays: vec![
                array("ok", ElemType::Int, 64, Vec::new()),
                array("long", ElemType::Int, 65, Vec::new()),
            ],
            ..Default::default()
        };
        let count = |chip| {
            let mut report = Report::default();
            check_array_limits(&logic, chip, &mut report);
            report.diagnostics.len()
        };
        assert_eq!(count(Some(Chip::Esp32C3)), 1);
        assert_eq!(count(Some(Chip::Esp32C3SuperMini)), 1);
        assert_eq!(count(Some(Chip::Esp32)), 0);
        assert_eq!(count(Some(Chip::Esp32S3)), 0);
        assert_eq!(count(None), 0);
    }
}
