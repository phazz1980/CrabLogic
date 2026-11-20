//! The «Структуры» folder of the left panel (`specs/arrays-structs`, task 3b), right after
//! «Массивы»: the structures of the project and nothing else. «+» makes a structure, a double click
//! or the menu changes it, the menu adds blocks of it or deletes it. A row dragged onto a sheet
//! gives a `struct_unpack` block of the structure. The blocks «Структура: собрать» and
//! «Структура: разобрать» are in the library on the right, group «Массивы».

use eframe::egui::{Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;
use crate::dialogs::Dialog;
use crate::panels::{drag_row, group, row_label, Level};
use crab_core::struct_blocks::{struct_name, PACK, UNPACK};
use crab_core::Logic;

pub enum StructAction {
    /// Opens the window of a new structure.
    New,
    /// Opens the window of the structure.
    Edit(String),
    /// Deletes the structure and its blocks.
    Remove(String),
    /// Adds a block (kind `struct_pack` or `struct_unpack`) of the structure to the active sheet.
    AddBlock(&'static str, String),
}

/// Whether a block of the structure stands on a sheet: `placed` tells the ids of the placed blocks.
fn on_sheet(logic: &Logic, placed: impl Fn(&str) -> bool, name: &str) -> bool {
    logic
        .blocks
        .iter()
        .any(|b| struct_name(b) == Some(name) && placed(&b.id))
}

/// «name {field: type, ...}» as the row shows the structure.
fn caption(st: &crab_core::arrays::StructVar) -> String {
    let fields: Vec<String> = st
        .fields
        .iter()
        .map(|f| format!("{}: {}", f.name, f.ty.name()))
        .collect();
    format!("{} {{{}}}", st.name, fields.join(", "))
}

pub fn show(app: &EditorApp, ui: &mut Ui) -> Vec<StructAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let logic = &doc.project.logic;
    let pal = app.theme.palette;
    let tools = |ui: &mut Ui| {
        if ui
            .small_button("+")
            .on_hover_text(crate::i18n::tr("Добавить структуру"))
            .clicked()
        {
            actions.push(StructAction::New);
        }
    };
    // `actions` is borrowed by `tools` until the group is built, so the rows collect their own.
    let mut rows = Vec::new();
    group(
        ui,
        "structs",
        crate::i18n::tr("Структуры"),
        Level::Mid,
        tools,
        |ui| {
            for st in &logic.structs {
                // Like a variable: an orange cross after the name while no block of the structure
                // stands on a sheet.
                let cross = (!on_sheet(
                    logic,
                    |id| doc.project.layout.blocks.contains_key(id),
                    &st.name,
                ))
                .then_some(pal.warning);
                let text = row_label(ui, caption(st), None, cross);
                let resp = drag_row(
                    ui,
                    Id::unique(("struct", &st.name)),
                    DragItem::Struct(st.name.clone()),
                    false,
                    text,
                )
                .on_hover_text(crate::i18n::fmt(
                    "{0}\nПеретащите на плату: появится блок «Структура: разобрать». Двойной щелчок — изменить. Блоки собрать и разобрать — в библиотеке справа, группа «Массивы».",
                    &[&st.description],
                ));
                if resp.double_clicked() {
                    rows.push(StructAction::Edit(st.name.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button(crate::i18n::tr("Изменить...")).clicked() {
                        rows.push(StructAction::Edit(st.name.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(crate::i18n::tr("Добавить блок: собрать"))
                        .clicked()
                    {
                        rows.push(StructAction::AddBlock(PACK, st.name.clone()));
                        ui.close();
                    }
                    if ui
                        .button(crate::i18n::tr("Добавить блок: разобрать"))
                        .clicked()
                    {
                        rows.push(StructAction::AddBlock(UNPACK, st.name.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(crate::i18n::tr("Удалить вместе с блоками"))
                        .clicked()
                    {
                        rows.push(StructAction::Remove(st.name.clone()));
                        ui.close();
                    }
                });
            }
            if logic.structs.is_empty() {
                ui.weak(crate::i18n::tr("нет"));
            }
        },
    );
    actions.append(&mut rows);
    actions
}

pub fn apply(app: &mut EditorApp, actions: Vec<StructAction>) {
    for action in actions {
        match action {
            StructAction::New => app.dialog = Dialog::for_struct(app, None),
            StructAction::Edit(name) => app.dialog = Dialog::for_struct(app, Some(&name)),
            StructAction::Remove(name) => {
                let lib = app.lib.clone();
                let Some(doc) = &mut app.doc else { continue };
                match doc.edit(&lib, None, |p| p.remove_struct(&name)) {
                    Ok(()) => app.selection = None,
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
            StructAction::AddBlock(kind, name) => {
                let lib = app.lib.clone();
                let pos = crate::panels::free_spot(app);
                if let Some(doc) = &mut app.doc {
                    match doc.edit(&lib, None, |p| p.add_struct_block(kind, &name, pos)) {
                        Ok(id) => app.focus_block(&id),
                        Err(e) => app.info(LogLevel::Error, e.to_string()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::arrays::{ElemType, StructField, StructVar};
    use crab_core::{Block, ParamValue};

    fn block(id: &str, kind: &str, st: &str) -> Block {
        let mut b = Block {
            id: id.into(),
            kind: kind.into(),
            ..Default::default()
        };
        b.params
            .insert("struct".into(), ParamValue::Text(st.into()));
        b
    }

    #[test]
    fn a_structure_is_on_a_sheet_when_one_of_its_blocks_is_placed() {
        let logic = Logic {
            blocks: vec![
                block("recipe_pack1", PACK, "recipe"),
                block("recipe_unpack1", UNPACK, "recipe"),
                block("other_unpack1", UNPACK, "other"),
            ],
            ..Default::default()
        };
        let none = |_: &str| false;
        assert!(!on_sheet(&logic, none, "recipe"));
        let only_unpack = |id: &str| id == "recipe_unpack1";
        assert!(on_sheet(&logic, only_unpack, "recipe"));
        assert!(!on_sheet(&logic, only_unpack, "other"));
        assert!(!on_sheet(&logic, |_| true, "missing"));
    }

    #[test]
    fn the_row_shows_the_fields_with_their_types() {
        let st = StructVar {
            name: "recipe".into(),
            fields: vec![
                StructField {
                    name: "temp".into(),
                    ty: ElemType::Float,
                    init: None,
                },
                StructField {
                    name: "fan".into(),
                    ty: ElemType::Bool,
                    init: None,
                },
            ],
            description: String::new(),
        };
        assert_eq!(caption(&st), "recipe {temp: float, fan: bool}");
    }
}
