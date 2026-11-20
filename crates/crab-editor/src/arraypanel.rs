//! The «Массивы» folder of the left panel (`specs/arrays-structs`, task 8). A row is an array of the
//! project, like the row of a variable: drag it onto a sheet to read it (an `array_get` block), a
//! click finds its blocks on the board (a repeated click shows the next one), a double click opens
//! the window of the array, the context menu puts any block of the array on the board.

use crab_core::array_blocks::{array_name, ArrayOp, DEST_PARAM};
use eframe::egui::{Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;
use crate::dialogs::Dialog;
use crate::panels::{drag_row, free_spot, group, placed_images, row_label, Level};

pub enum ArrayAction {
    /// Opens the window of a new array.
    New,
    /// Opens the window of the array.
    Edit(String),
    /// Puts a block of the operation on the board.
    Place(ArrayOp, String),
    /// Finds the blocks of the array: the key of the row and the ids on the boards in order.
    Focus(String, Vec<String>),
    Remove(String),
}

/// Ids of the blocks that name `array` (as the array, or as the destination of a copy).
fn blocks_of<'a>(logic: &'a crab_core::Logic, array: &str) -> Vec<&'a str> {
    logic
        .blocks
        .iter()
        .filter(|b| {
            array_name(b) == Some(array)
                || (ArrayOp::from_kind(&b.kind) == Some(ArrayOp::Copy)
                    && matches!(b.params.get(DEST_PARAM), Some(crab_core::ParamValue::Text(d)) if d == array))
        })
        .map(|b| b.id.as_str())
        .collect()
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
                let blocks = blocks_of(logic, &array.name);
                let images = placed_images(&doc.project, blocks.iter().copied());
                let sel = app
                    .selection
                    .as_ref()
                    .is_some_and(|s| images.iter().any(|b| s.has_block(b)));
                // Like a variable: coloured by the signal type, an orange cross after the name while
                // no block uses the array.
                let caption = format!("{} <{}[{}]>", array.name, array.ty.name(), array.len);
                let color = Some(crate::canvas::signal_color(&pal, array.ty.signal()));
                let text = if blocks.is_empty() {
                    row_label(ui, caption, color, Some(pal.warning))
                } else {
                    row_label(ui, caption, color, None)
                };
                let resp = drag_row(
                    ui,
                    Id::unique(("array", &array.name)),
                    DragItem::Array(array.name.clone()),
                    sel,
                    text,
                )
                .on_hover_text(crate::i18n::fmt(
                    "{0}\nЩелчок — найти на плате, повторный — следующий блок. Перетащите на плату: появится блок чтения; запись и остальные блоки — в меню правой кнопки. Двойной щелчок — изменить.",
                    &[&array.description],
                ));
                if resp.double_clicked() {
                    rows.push(ArrayAction::Edit(array.name.clone()));
                } else if resp.clicked() && !images.is_empty() {
                    rows.push(ArrayAction::Focus(format!("array:{}", array.name), images));
                }
                resp.context_menu(|ui| {
                    // Reading and writing stand first, like for a variable; the other blocks follow.
                    if ui
                        .button(crate::i18n::tr("Поставить чтение на плату"))
                        .clicked()
                    {
                        rows.push(ArrayAction::Place(ArrayOp::Get, array.name.clone()));
                        ui.close();
                    }
                    if ui
                        .button(crate::i18n::tr("Поставить запись на плату"))
                        .clicked()
                    {
                        rows.push(ArrayAction::Place(ArrayOp::Set, array.name.clone()));
                        ui.close();
                    }
                    ui.menu_button(crate::i18n::tr("Другие блоки"), |ui| {
                        for op in ArrayOp::ALL.into_iter().filter(|op| {
                            op.exists_for(array.ty) && !matches!(op, ArrayOp::Get | ArrayOp::Set)
                        }) {
                            if ui.button(crate::i18n::tr(op.caption().0)).clicked() {
                                rows.push(ArrayAction::Place(op, array.name.clone()));
                                ui.close();
                            }
                        }
                    });
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
            ArrayAction::Focus(key, images) => {
                // A repeated click on the same row shows the next block.
                let index = match &app.tree_cycle {
                    Some((last, i)) if *last == key => (i + 1) % images.len(),
                    _ => 0,
                };
                app.focus_block(&images[index]);
                app.tree_cycle = Some((key, index));
            }
            ArrayAction::Place(op, array) => {
                let lib = app.lib.clone();
                let pos = free_spot(app);
                let Some(doc) = &mut app.doc else { continue };
                match doc.edit(&lib, None, |p| p.add_array_block(op, &array, pos)) {
                    Ok(id) => app.focus_block(&id),
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
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
