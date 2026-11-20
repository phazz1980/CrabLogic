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
