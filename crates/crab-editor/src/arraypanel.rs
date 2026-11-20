//! The «Массивы» folder of the left panel (`specs/arrays-structs`, task 8): the arrays of the
//! project and nothing else. «+» makes an array, a double click or the menu changes it, the menu
//! deletes it. The blocks that work on arrays (read, write, sum, ...) are in the library on the
//! right, group «Массивы»; the array a block works on is chosen in its properties.

use eframe::egui::Ui;

use crate::app::{EditorApp, LogLevel};
use crate::dialogs::Dialog;
use crate::panels::{group, row_label, Level};
use crab_core::array_blocks::{array_name, ArrayOp, DEST_PARAM};

pub enum ArrayAction {
    /// Opens the window of a new array.
    New,
    /// Opens the window of the array.
    Edit(String),
    /// Deletes the array and its blocks.
    Remove(String),
}

/// Whether any block names `array` (as the array, or as the destination of a copy).
fn used(logic: &crab_core::Logic, array: &str) -> bool {
    logic.blocks.iter().any(|b| {
        array_name(b) == Some(array)
            || (ArrayOp::from_kind(&b.kind) == Some(ArrayOp::Copy)
                && matches!(b.params.get(DEST_PARAM), Some(crab_core::ParamValue::Text(d)) if d == array))
    })
}

pub fn show(app: &EditorApp, ui: &mut Ui) -> Vec<ArrayAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let logic = &doc.project.logic;
    let pal = app.theme.palette;
    let tools = |ui: &mut Ui| {
        if ui
            .small_button("+")
            .on_hover_text(crate::i18n::tr("Добавить массив"))
            .clicked()
        {
            actions.push(ArrayAction::New);
        }
    };
    // `actions` is borrowed by `tools` until the group is built, so the rows collect their own.
    let mut rows = Vec::new();
    group(
        ui,
        "arrays",
        crate::i18n::tr("Массивы"),
        Level::Mid,
        tools,
        |ui| {
            for array in &logic.arrays {
                // Like a variable: coloured by the signal type, an orange cross after the name
                // while no block uses the array.
                let caption = format!("{} <{}[{}]>", array.name, array.ty.name(), array.len);
                let color = Some(crate::canvas::signal_color(&pal, array.ty.signal()));
                let cross = (!used(logic, &array.name)).then_some(pal.warning);
                let resp = ui
                    .selectable_label(false, row_label(ui, caption, color, cross))
                    .on_hover_text(crate::i18n::fmt(
                        "{0}\nДвойной щелчок — изменить. Блоки работы с массивом (чтение, запись и другие) — в библиотеке справа, группа «Массивы».",
                        &[&array.description],
                    ));
                if resp.double_clicked() {
                    rows.push(ArrayAction::Edit(array.name.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button(crate::i18n::tr("Изменить...")).clicked() {
                        rows.push(ArrayAction::Edit(array.name.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(crate::i18n::tr("Удалить вместе с блоками"))
                        .clicked()
                    {
                        rows.push(ArrayAction::Remove(array.name.clone()));
                        ui.close();
                    }
                });
            }
            if logic.arrays.is_empty() {
                ui.weak(crate::i18n::tr("нет"));
            }
        },
    );
    actions.append(&mut rows);
    actions
}

pub fn apply(app: &mut EditorApp, actions: Vec<ArrayAction>) {
    for action in actions {
        match action {
            ArrayAction::New => app.dialog = Dialog::for_array(app, None),
            ArrayAction::Edit(name) => app.dialog = Dialog::for_array(app, Some(&name)),
            ArrayAction::Remove(name) => {
                let lib = app.lib.clone();
                let Some(doc) = &mut app.doc else { continue };
                match doc.edit(&lib, None, |p| p.remove_array(&name)) {
                    Ok(()) => app.selection = None,
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
        }
    }
}
