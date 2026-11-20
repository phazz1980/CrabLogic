//! Window of a structure (`specs/arrays-structs`, task 3b): name, description and the table of
//! fields (name, type, initial value). A structure takes scalar fields only: bool, int, float and
//! byte; the checks of the project (names, `go`, repeated fields) are made when the window applies.

use super::array::parse_item;
use super::*;
use crab_core::arrays::{ElemType, StructField, StructVar};

/// Types a field of a structure may have (strings are not supported in structures yet).
const FIELD_TYPES: [ElemType; 4] = [
    ElemType::Bool,
    ElemType::Int,
    ElemType::Float,
    ElemType::Byte,
];

/// A structure of the project tree: new (`original` is `None`) or being changed.
pub struct StructDialog {
    pub(super) original: Option<String>,
    name: String,
    description: String,
    fields: Vec<FieldRow>,
    /// Why the last ✓ failed; shown in the window until the next try.
    pub(super) error: Option<String>,
}

/// One field as typed in the window.
#[derive(Clone)]
struct FieldRow {
    name: String,
    ty: ElemType,
    /// Initial value as typed; empty means zero.
    init: String,
}

impl FieldRow {
    fn blank() -> Self {
        FieldRow {
            name: String::new(),
            ty: ElemType::Int,
            init: String::new(),
        }
    }
}

impl Dialog {
    /// Window of a structure of the project; `None` declares a new one.
    pub fn for_struct(app: &EditorApp, name: Option<&str>) -> Option<Self> {
        let logic = &app.doc.as_ref()?.project.logic;
        let st = match name {
            Some(n) => logic.struct_var(n)?.clone(),
            None => StructVar {
                name: String::new(),
                fields: Vec::new(),
                description: String::new(),
            },
        };
        let mut fields: Vec<FieldRow> = st
            .fields
            .iter()
            .map(|f| FieldRow {
                name: f.name.clone(),
                ty: f.ty,
                init: f.init.as_ref().map(value_text).unwrap_or_default(),
            })
            .collect();
        if fields.is_empty() {
            fields.push(FieldRow::blank());
        }
        Some(Self::Struct(StructDialog {
            original: name.map(str::to_owned),
            name: st.name,
            description: st.description,
            fields,
            error: None,
        }))
    }
}

/// The structure the window describes, or what is wrong with its initial values. The names and
/// the other checks are made by the project (`add_struct` / `update_struct`).
pub(super) fn struct_value(d: &StructDialog) -> Result<StructVar, String> {
    let mut fields = Vec::new();
    for row in &d.fields {
        let init = parse_item(row.ty, &row.init)
            .map_err(|e| format!("поле «{}»: {e}", row.name.trim()))?;
        fields.push(StructField {
            name: row.name.trim().to_owned(),
            ty: row.ty,
            init,
        });
    }
    Ok(StructVar {
        name: d.name.trim().to_owned(),
        fields,
        description: d.description.trim().to_owned(),
    })
}

pub(super) fn structure_body(ui: &mut Ui, d: &mut StructDialog) {
    egui::Grid::new("struct_props")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Имя"));
            ui.text_edit_singleline(&mut d.name);
            ui.end_row();
            ui.label(crate::i18n::tr("Описание"));
            ui.text_edit_singleline(&mut d.description);
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.label(crate::i18n::tr(
        "Поля структуры (пустое начальное значение — ноль)",
    ));
    let mut remove = None;
    egui::Grid::new("struct_fields")
        .num_columns(4)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            ui.weak(crate::i18n::tr("Имя"));
            ui.weak(crate::i18n::tr("Тип"));
            ui.weak(crate::i18n::tr("Начальное"));
            ui.label("");
            ui.end_row();
            for (i, row) in d.fields.iter_mut().enumerate() {
                ui.add(egui::TextEdit::singleline(&mut row.name).desired_width(96.0));
                egui::ComboBox::from_id_salt(("struct_field_type", i))
                    .selected_text(row.ty.name())
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for ty in FIELD_TYPES {
                            ui.selectable_value(&mut row.ty, ty, ty.name());
                        }
                    });
                let valid = parse_item(row.ty, &row.init).is_ok();
                let mut edit = egui::TextEdit::singleline(&mut row.init).desired_width(70.0);
                if !valid {
                    edit = edit.text_color(Color32::from_rgb(186, 74, 66));
                }
                ui.add(edit);
                if ui
                    .small_button("−")
                    .on_hover_text(crate::i18n::tr("Удалить поле"))
                    .clicked()
                {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        d.fields.remove(i);
    }
    if ui
        .button(crate::i18n::tr("Добавить поле"))
        .on_hover_text(crate::i18n::tr(
            "Поле получает вход и выход у блоков «Структура: запись» и «Структура: чтение»",
        ))
        .clicked()
    {
        d.fields.push(FieldRow::blank());
    }
    if let Some(e) = &d.error {
        ui.colored_label(Color32::from_rgb(186, 74, 66), e.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(name: &str, fields: &[(&str, ElemType, &str)]) -> StructDialog {
        StructDialog {
            original: None,
            name: name.into(),
            description: " рецепт ".into(),
            fields: fields
                .iter()
                .map(|(n, ty, init)| FieldRow {
                    name: (*n).into(),
                    ty: *ty,
                    init: (*init).into(),
                })
                .collect(),
            error: None,
        }
    }

    #[test]
    fn the_window_becomes_a_structure_with_typed_initial_values() {
        let st = struct_value(&dialog(
            " recipe ",
            &[
                (" temp ", ElemType::Float, "60"),
                ("fan", ElemType::Bool, ""),
            ],
        ))
        .unwrap();
        assert_eq!(st.name, "recipe");
        assert_eq!(st.description, "рецепт");
        assert_eq!(st.fields[0].name, "temp");
        assert_eq!(st.fields[0].init, Some(ParamValue::Float(60.0)));
        assert_eq!(st.fields[1].init, None);
    }

    #[test]
    fn a_bad_initial_value_is_named_by_its_field() {
        let err = struct_value(&dialog(
            "r",
            &[("ok", ElemType::Int, "1"), ("t", ElemType::Byte, "300")],
        ))
        .unwrap_err();
        assert!(err.contains("«t»"), "{err}");
        assert!(struct_value(&dialog("r", &[("b", ElemType::Bool, "maybe")])).is_err());
    }
}
