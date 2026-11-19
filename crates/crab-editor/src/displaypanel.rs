//! Displays in the left panel: OLED and LCD blocks of the project with their settings,
//! an optional menu block on the same sheet and a button that opens the screen designer.
//! Displays are ordinary visible blocks; they are added from here, not from the library.

use crab_core::{Block, BlockPos, Logic, ParamValue};
use eframe::egui::{self, Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;

/// Block kinds that are displays.
pub const DISPLAY_KINDS: [&str; 2] = ["oled", "lcd"];
const MENU: &str = "menu";

pub enum DisplayAction {
    Add(&'static str),
    /// Adds or removes the menu block of a display.
    Menu {
        display: String,
        on: bool,
    },
    Designer(String),
    Focus(String),
    /// Opens the properties window of the block.
    Settings(String),
    /// Deletes the display with its menu blocks.
    Remove(String),
}

pub fn is_display(kind: &str) -> bool {
    DISPLAY_KINDS.contains(&kind)
}

/// Menu blocks that belong to the display `display`.
pub fn menus_of<'a>(logic: &'a Logic, display: &str) -> Vec<&'a Block> {
    logic
        .blocks
        .iter()
        .filter(|b| {
            b.kind == MENU
                && matches!(b.params.get("display"), Some(ParamValue::Text(t)) if t == display)
        })
        .collect()
}

/// Current value of a parameter: the block's own or the default of the library.
fn param_value(app: &EditorApp, b: &Block, name: &str) -> Option<ParamValue> {
    let logic = &app.doc.as_ref()?.project.logic;
    b.params.get(name).cloned().or_else(|| {
        app.lib
            .resolve(logic, b)
            .and_then(|l| l.def.param(name).and_then(|p| p.default.clone()))
    })
}

pub fn show(app: &EditorApp, ui: &mut Ui) -> Vec<DisplayAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let logic = &doc.project.logic;
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr("Дисплеи")).strong())
        .id_salt("displays_panel")
        .default_open(logic.blocks.iter().any(|b| is_display(&b.kind)))
        .show(ui, |ui| {
            // One group per kind: its add button on its own line, its displays right under it.
            for (kind, caption) in [("oled", "+ OLED"), ("lcd", "+ LCD")] {
                let add =
                    crate::panels::add_group(ui, ("display_kind", kind), caption, "", true, |ui| {
                        for b in logic.blocks.iter().filter(|b| b.kind == kind) {
                            display(app, ui, b, &mut actions);
                        }
                    });
                if add {
                    actions.push(DisplayAction::Add(kind));
                }
            }
        });
    ui.separator();
    actions
}

fn display(app: &EditorApp, ui: &mut Ui, b: &Block, actions: &mut Vec<DisplayAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let lib = app.lib.resolve(logic, b);
    let model = param_value(app, b, "model");
    let model_label = lib
        .as_ref()
        .and_then(|l| l.def.param("model"))
        .and_then(|p| p.choices.iter().find(|c| Some(&c.value) == model.as_ref()))
        .map(|c| c.label.clone())
        .unwrap_or_default();
    let placed = doc.project.layout.blocks.contains_key(&b.id);
    // Like a variable: the row is dragged onto a sheet, a click finds the block there, a
    // double click opens its parameters, the context menu has the rest.
    let selected = app.selection.as_ref().is_some_and(|s| s.has_block(&b.id));
    let caption = format!("{} ({model_label})", b.id);
    let text = if placed {
        egui::RichText::new(format!("→ {caption}"))
    } else {
        egui::RichText::new(format!("✕ {caption}")).color(app.theme.palette.warning)
    };
    let resp = crate::panels::drag_row(
        ui,
        Id::unique(("display", &b.id)),
        DragItem::Topic(b.id.clone()),
        selected,
        text,
    )
    .on_hover_text(if placed {
        crate::i18n::tr("Щелчок — найти на плате. Двойной щелчок — свойства; перетащите на плату, чтобы переставить.")
    } else {
        crate::i18n::tr("Перетащите на плату. Двойной щелчок — свойства.")
    });
    if resp.double_clicked() {
        actions.push(DisplayAction::Settings(b.id.clone()));
    } else if resp.clicked() && placed {
        actions.push(DisplayAction::Focus(b.id.clone()));
    }
    resp.context_menu(|ui| {
        if placed && ui.button(crate::i18n::tr("Найти на плате")).clicked() {
            actions.push(DisplayAction::Focus(b.id.clone()));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Свойства...")).clicked() {
            actions.push(DisplayAction::Settings(b.id.clone()));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Экраны и меню...")).clicked() {
            actions.push(DisplayAction::Designer(b.id.clone()));
            ui.close();
        }
        let mut menu_on = !menus_of(logic, &b.id).is_empty();
        if ui
            .checkbox(&mut menu_on, crate::i18n::tr("Меню"))
            .on_hover_text(crate::i18n::tr(
                "Блок меню на той же плате, что и дисплей; при выключении блок меню удаляется",
            ))
            .changed()
        {
            actions.push(DisplayAction::Menu {
                display: b.id.clone(),
                on: menu_on,
            });
            ui.close();
        }
        ui.separator();
        if ui
            .button(crate::i18n::tr("Удалить дисплей"))
            .on_hover_text(crate::i18n::tr(
                "Дисплей и его блок меню удаляются из проекта",
            ))
            .clicked()
        {
            actions.push(DisplayAction::Remove(b.id.clone()));
            ui.close();
        }
    });
}

/// Position below the lowest block of `sheet`.
fn spot_on(app: &EditorApp, sheet: u32) -> BlockPos {
    let bottom = app.doc.as_ref().map_or(0.0, |d| {
        d.project
            .layout
            .blocks
            .values()
            .filter(|p| p.sheet == sheet)
            .map(|p| p.y + 80.0)
            .fold(0.0_f32, f32::max)
    });
    BlockPos {
        sheet,
        x: 40.0,
        y: bottom + 20.0,
    }
}

pub fn apply(app: &mut EditorApp, actions: Vec<DisplayAction>) {
    for action in actions {
        let lib = app.lib.clone();
        match action {
            DisplayAction::Focus(id) => app.focus_block(&id),
            DisplayAction::Settings(id) => {
                app.dialog = crate::dialogs::Dialog::for_block(app, &id);
            }
            DisplayAction::Remove(id) => {
                let Some(doc) = &mut app.doc else { return };
                let result = doc.edit(&lib, None, |p| {
                    let mut ids: Vec<String> = menus_of(&p.logic, &id)
                        .iter()
                        .map(|b| b.id.clone())
                        .collect();
                    ids.push(id.clone());
                    ids.iter().try_for_each(|i| p.remove_block(i))
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            DisplayAction::Designer(id) => app.open_web_designer(&format!("/screens?display={id}")),
            DisplayAction::Add(kind) => {
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { return };
                // The display waits in the panel until it is dragged onto a sheet.
                let result = doc.edit(&lib, None, |p| {
                    p.add_block(&lib, kind, pos).inspect(|id| {
                        p.layout.blocks.remove(id);
                    })
                });
                match result {
                    Ok(_) => {}
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
            DisplayAction::Menu { display, on } => {
                // The menu stands on the sheet of its display (or on the current one).
                let sheet = app
                    .doc
                    .as_ref()
                    .and_then(|d| d.project.layout.blocks.get(&display))
                    .map_or(app.sheet, |p| p.sheet);
                let pos = spot_on(app, sheet);
                let Some(doc) = &mut app.doc else { return };
                let result = doc.edit(&lib, None, |p| {
                    if on {
                        let id = p.add_block(&lib, MENU, pos)?;
                        p.set_param(
                            &lib,
                            &id,
                            "display",
                            Some(ParamValue::Text(display.clone())),
                        )
                    } else {
                        let ids: Vec<String> = menus_of(&p.logic, &display)
                            .iter()
                            .map(|b| b.id.clone())
                            .collect();
                        ids.iter().try_for_each(|id| p.remove_block(id))
                    }
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_belongs_to_its_display() {
        let mut logic = Logic::default();
        let menu = |id: &str, display: &str| {
            let mut b = Block {
                id: id.into(),
                kind: MENU.into(),
                ..Default::default()
            };
            b.params
                .insert("display".into(), ParamValue::Text(display.into()));
            b
        };
        logic.blocks.push(menu("menu_1", "oled_1"));
        logic.blocks.push(menu("menu_2", "lcd_1"));
        let ids: Vec<_> = menus_of(&logic, "oled_1")
            .iter()
            .map(|b| b.id.clone())
            .collect();
        assert_eq!(ids, ["menu_1"]);
        assert!(menus_of(&logic, "oled_2").is_empty());
    }
}
