//! Window of an array (`specs/arrays-structs`, task 8): name, element type, length, the table of
//! initial values (one field per element) and the description.

use super::*;
use crab_core::arrays::{ArrayVar, ElemType, MAX_LEN};

/// An array of the project tree: new (`original` is `None`) or being changed.
pub struct ArrayDialog {
    pub(super) original: Option<String>,
    name: String,
    ty: ElemType,
    len: u32,
    /// Initial value of every element as typed; empty means zero (false, an empty string).
    items: Vec<String>,
    description: String,
}

impl Dialog {
    /// Window of an array of the project; `None` declares a new one.
    pub fn for_array(app: &EditorApp, name: Option<&str>) -> Option<Self> {
        let logic = &app.doc.as_ref()?.project.logic;
        let array = match name {
            Some(n) => logic.array(n)?.clone(),
            None => ArrayVar {
                name: String::new(),
                ty: ElemType::Int,
                len: 8,
                items: Vec::new(),
                description: String::new(),
            },
        };
        let mut items: Vec<String> = array.items.iter().map(value_text).collect();
        items.resize(array.len as usize, String::new());
        Some(Self::Array(ArrayDialog {
            original: name.map(str::to_owned),
            name: array.name,
            ty: array.ty,
            len: array.len,
            items,
            description: array.description,
        }))
    }
}

/// One typed initial value: `None` for an empty field.
pub(super) fn parse_item(ty: ElemType, text: &str) -> Result<Option<ParamValue>, String> {
    let t = text.trim();
    if t.is_empty() && ty != ElemType::String {
        return Ok(None);
    }
    let bad = || format!("«{t}» не подходит к типу {}", ty.name());
    Ok(Some(match ty {
        ElemType::Bool => match t {
            "true" | "1" => ParamValue::Bool(true),
            "false" | "0" => ParamValue::Bool(false),
            _ => return Err(bad()),
        },
        ElemType::Int | ElemType::Byte => {
            let v = ParamValue::Int(t.parse().map_err(|_| bad())?);
            if !ty.accepts(&v) {
                return Err(bad());
            }
            v
        }
        ElemType::Float => ParamValue::Float(t.parse().map_err(|_| bad())?),
        ElemType::String => {
            if text.is_empty() {
                return Ok(None);
            }
            ParamValue::Text(text.to_owned())
        }
    }))
}

/// The array the window describes, or what is wrong with it.
pub(super) fn array_value(d: &ArrayDialog) -> Result<ArrayVar, String> {
    let mut parsed = Vec::new();
    for (i, text) in d.items.iter().take(d.len as usize).enumerate() {
        parsed.push(parse_item(d.ty, text).map_err(|e| format!("элемент {i}: {e}"))?);
    }
    // Trailing empty fields are not stored; an empty one in the middle is zero.
    let used = parsed
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |i| i + 1);
    let zero = match d.ty {
        ElemType::Bool => ParamValue::Bool(false),
        ElemType::Float => ParamValue::Float(0.0),
        ElemType::String => ParamValue::Text(String::new()),
        ElemType::Int | ElemType::Byte => ParamValue::Int(0),
    };
    let items = parsed
        .into_iter()
        .take(used)
        .map(|v| v.unwrap_or_else(|| zero.clone()))
        .collect();
    Ok(ArrayVar {
        name: d.name.trim().to_owned(),
        ty: d.ty,
        len: d.len,
        items,
        description: d.description.trim().to_owned(),
    })
}

pub(super) fn array_body(ui: &mut Ui, d: &mut ArrayDialog) {
    egui::Grid::new("array_props")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(crate::i18n::tr("Имя"));
            ui.text_edit_singleline(&mut d.name);
            ui.end_row();
            ui.label(crate::i18n::tr("Тип элемента"));
            egui::ComboBox::from_id_salt("array_type")
                .selected_text(d.ty.name())
                .show_ui(ui, |ui| {
                    for ty in ElemType::ALL {
                        ui.selectable_value(&mut d.ty, ty, ty.name());
                    }
                });
            ui.end_row();
            ui.label(crate::i18n::tr("Длина"));
            ui.add(egui::DragValue::new(&mut d.len).range(1..=MAX_LEN))
                .on_hover_text(crate::i18n::tr(
                    "Число элементов: до 256 на ESP32 и ESP32-S3, до 64 на ESP32-C3",
                ));
            ui.end_row();
            ui.label(crate::i18n::tr("Описание"));
            ui.text_edit_singleline(&mut d.description);
            ui.end_row();
        });
    if d.ty == ElemType::String {
        ui.weak(crate::i18n::tr(
            "Массивы строк пока работают только в симуляции: в модуле для платы их ещё нет.",
        ));
    }
    d.items.resize(d.len as usize, String::new());
    ui.add_space(6.0);
    ui.label(crate::i18n::tr("Начальные значения (пусто — ноль)"));
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .show(ui, |ui| {
            egui::Grid::new("array_items")
                .num_columns(8)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for (i, text) in d.items.iter_mut().enumerate() {
                        ui.weak(format!("[{i}]"));
                        let valid = parse_item(d.ty, text).is_ok();
                        let mut edit = egui::TextEdit::singleline(text).desired_width(54.0);
                        if !valid {
                            edit = edit.text_color(Color32::from_rgb(186, 74, 66));
                        }
                        ui.add(edit);
                        if (i + 1) % 4 == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(ty: ElemType, len: u32, items: &[&str]) -> ArrayDialog {
        ArrayDialog {
            original: None,
            name: " a ".into(),
            ty,
            len,
            items: items.iter().map(|s| (*s).to_owned()).collect(),
            description: String::new(),
        }
    }

    #[test]
    fn trailing_empty_fields_are_dropped_and_middle_ones_are_zero() {
        let a = array_value(&dialog(ElemType::Int, 5, &["10", "", "30", "", ""])).unwrap();
        assert_eq!(a.name, "a");
        assert_eq!(
            a.items,
            vec![ParamValue::Int(10), ParamValue::Int(0), ParamValue::Int(30)]
        );
        let none = array_value(&dialog(ElemType::Float, 3, &["", "", ""])).unwrap();
        assert!(none.items.is_empty());
    }

    #[test]
    fn values_are_checked_against_the_type() {
        assert!(array_value(&dialog(ElemType::Byte, 2, &["255", "256"])).is_err());
        assert!(array_value(&dialog(ElemType::Int, 1, &["x"])).is_err());
        assert!(array_value(&dialog(ElemType::Bool, 2, &["true", "0"])).is_ok());
        assert!(array_value(&dialog(ElemType::Bool, 1, &["2"])).is_err());
        let f = array_value(&dialog(ElemType::Float, 2, &["1.5", "2"])).unwrap();
        assert_eq!(
            f.items,
            vec![ParamValue::Float(1.5), ParamValue::Float(2.0)]
        );
        let s = array_value(&dialog(ElemType::String, 2, &["hi", ""])).unwrap();
        assert_eq!(s.items, vec![ParamValue::Text("hi".into())]);
        let err = array_value(&dialog(ElemType::Int, 3, &["1", "2", "q"])).unwrap_err();
        assert!(err.contains("элемент 2"), "{err}");
    }
}
