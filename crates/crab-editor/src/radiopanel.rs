//! Radio (LoRa) in the left panel: the modem and the partner blocks of the project with a button
//! to find them on the sheet and one to delete them. Radio blocks are ordinary visible blocks;
//! they are added from here, not from the library.

use crab_core::project::{MODBUS_KINDS, RADIO_KINDS, UART_KINDS};
use crab_core::ParamValue;
use eframe::egui::{self, Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;
use crate::dialogs::Dialog;

pub enum RadioAction {
    Add(&'static str),
    /// Adds a slave device under the master with this id (`modbus_device`, with the next free
    /// Unit ID).
    AddDevice(String),
    /// Adds a tag to the owner with this id (a slave device or a slave-role block), at the next
    /// free address of its holding table.
    AddTag(String),
    Focus(String),
    Remove(String),
    Settings(String),
}

/// What the «+» buttons add: (kind, caption, hint).
const ADD: [(&str, &str, &str); 3] = [
    (
        "lora_e220",
        "+ Модем",
        "Радиомодуль E220: в проекте он один (роль «master» или «slave» задаётся в его свойствах)",
    ),
    (
        "lora_point",
        "+ Slave",
        "Блок «LoRa: slave» на стороне master: команды slave и его ответы",
    ),
    (
        "lora_centre",
        "+ Master",
        "Блок «LoRa: master» на стороне slave: команды master и значения в ответ",
    ),
];

pub fn show(app: &EditorApp, ui: &mut Ui) -> Vec<RadioAction> {
    let mut actions = Vec::new();
    let Some(doc) = &app.doc else {
        return actions;
    };
    let logic = &doc.project.logic;
    let modem_exists = logic.blocks.iter().any(|b| b.kind == "lora_e220");
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr("Радио")).strong())
        .id_salt("radio_panel")
        .default_open(
            logic
                .blocks
                .iter()
                .any(|b| RADIO_KINDS.contains(&b.kind.as_str())),
        )
        .show(ui, |ui| {
            // The «LoRa E220» subfolder, like «Modbus» in the network folder: one group per
            // role, its add button on its own line, its blocks right under it.
            egui::CollapsingHeader::new(egui::RichText::new("LoRa E220").strong())
                .id_salt("radio_lora_e220")
                .default_open(
                    logic
                        .blocks
                        .iter()
                        .any(|b| RADIO_KINDS.contains(&b.kind.as_str())),
                )
                .show(ui, |ui| {
                    for (kind, caption, hint) in ADD {
                        // One modem per board (`single` in the library).
                        let enabled = kind != "lora_e220" || !modem_exists;
                        let add = crate::panels::add_group(
                            ui,
                            ("radio_kind", kind),
                            crate::i18n::tr(caption),
                            crate::i18n::tr(hint),
                            enabled,
                            |ui| {
                                for b in logic
                                    .blocks
                                    .iter()
                                    .filter(|b| b.tag.is_none() && b.kind == kind)
                                {
                                    row(app, ui, b, false, &mut actions);
                                }
                            },
                        );
                        if add {
                            actions.push(RadioAction::Add(kind));
                        }
                    }
                });
        });
    ui.separator();
    actions
}

/// One block row, like the row of a variable: the whole row is dragged onto a sheet, a click finds
/// the block on the board (when it is there), a double click opens its parameters, the context menu
/// has the rest. `show_kind` adds the block name after the id (for lists that mix kinds).
pub(crate) fn row(
    app: &EditorApp,
    ui: &mut Ui,
    b: &crab_core::Block,
    show_kind: bool,
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let name = app
        .lib
        .resolve(&doc.project.logic, b)
        .map(|l| l.def.info.name.clone())
        .unwrap_or_else(|| b.kind.clone());
    let caption = if show_kind {
        format!("{}  ·  {}", b.id, crate::i18n::tr(&name))
    } else {
        b.id.clone()
    };
    row_captioned(app, ui, b, caption, None, actions);
}

/// The same row with a caption of the caller (a Modbus tag shows its table, type and address).
pub(crate) fn row_captioned(
    app: &EditorApp,
    ui: &mut Ui,
    b: &crab_core::Block,
    caption: String,
    color: Option<egui::Color32>,
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let placed = doc.project.layout.blocks.contains_key(&b.id);
    let selected = app.selection.as_ref().is_some_and(|s| s.has_block(&b.id));
    let text = crate::panels::row_label(
        ui,
        caption,
        color,
        crate::panels::off_sheet_cross(ui, !placed),
    );
    let resp = crate::panels::drag_row(
        ui,
        Id::unique(("radio", &b.id)),
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
        actions.push(RadioAction::Settings(b.id.clone()));
    } else if resp.clicked() && placed {
        actions.push(RadioAction::Focus(b.id.clone()));
    }
    resp.context_menu(|ui| {
        if placed && ui.button(crate::i18n::tr("Найти на плате")).clicked() {
            actions.push(RadioAction::Focus(b.id.clone()));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Свойства...")).clicked() {
            actions.push(RadioAction::Settings(b.id.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button(crate::i18n::tr("Удалить из проекта")).clicked() {
            actions.push(RadioAction::Remove(b.id.clone()));
            ui.close();
        }
    });
}

pub fn apply(app: &mut EditorApp, actions: Vec<RadioAction>) {
    for action in actions {
        let lib = app.lib.clone();
        match action {
            RadioAction::Focus(id) => app.focus_block(&id),
            RadioAction::Settings(id) => app.dialog = Dialog::for_block(app, &id),
            RadioAction::Add(kind) => {
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { return };
                // A Modbus or radio block waits in the panel until it is dragged onto a sheet.
                let waits = MODBUS_KINDS.contains(&kind)
                    || RADIO_KINDS.contains(&kind)
                    || UART_KINDS.contains(&kind);
                let result = doc.edit(&lib, None, |p| {
                    p.add_block(&lib, kind, pos).inspect(|id| {
                        if waits {
                            p.layout.blocks.remove(id);
                        }
                    })
                });
                match result {
                    Ok(id) if !waits => app.focus_block(&id),
                    Ok(_) => {}
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
            RadioAction::Remove(id) => {
                let Some(doc) = &mut app.doc else { continue };
                // A master, a slave device or a slave-role block takes its devices and tags along;
                // the whole removal is one undo step.
                let ids = crate::modbuspanel::cascade_ids(&doc.project.logic, &id);
                let result = doc.edit(&lib, None, |p| {
                    ids.iter().try_for_each(|x| p.remove_block(x))
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            RadioAction::AddDevice(master) => {
                let full = app
                    .doc
                    .as_ref()
                    .is_some_and(|d| crate::modbuspanel::slaves_full(&d.project.logic, &master));
                if full {
                    app.info(
                        LogLevel::Error,
                        crate::i18n::tr("Лимит slave у этого master исчерпан: поднимите «Макс. slave» в его настройках")
                            .to_owned(),
                    );
                    continue;
                }
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { continue };
                let unit = crate::modbuspanel::next_unit(&doc.project.logic, &master);
                let result = doc.edit(&lib, None, |p| {
                    let id = p.add_block(&lib, "modbus_device", pos)?;
                    // A device waits in the panel: it is never placed on a sheet.
                    p.layout.blocks.remove(&id);
                    p.set_param(&lib, &id, "master", Some(ParamValue::Text(master.clone())))?;
                    p.set_param(&lib, &id, "unit", Some(ParamValue::Int(unit)))
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            RadioAction::AddTag(owner) => {
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { continue };
                let reg = doc
                    .project
                    .logic
                    .modbus_next_free(&owner, crate::modbuspanel::NEW_TAG_TABLE);
                let id = crate::modbuspanel::free_tag_id(&doc.project.logic, reg);
                let result = doc.edit(&lib, None, |p| {
                    let auto = p.add_block(&lib, "modbus_tag", pos)?;
                    // The tag waits in the panel until it is dragged onto a sheet.
                    p.layout.blocks.remove(&auto);
                    let table = crate::modbuspanel::NEW_TAG_TABLE;
                    p.set_param(&lib, &auto, "device", Some(ParamValue::Text(owner.clone())))?;
                    p.set_param(
                        &lib,
                        &auto,
                        "table",
                        Some(ParamValue::Text(table.to_owned())),
                    )?;
                    p.set_param(
                        &lib,
                        &auto,
                        "kind",
                        Some(ParamValue::Text("int".to_owned())),
                    )?;
                    p.set_param(&lib, &auto, "reg", Some(ParamValue::Int(i64::from(reg))))?;
                    p.rename_block(&auto, &id)
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
        }
    }
}
