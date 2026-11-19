//! Radio (LoRa) in the left panel: the modem and the partner blocks of the project with a button
//! to find them on the sheet and one to delete them. Radio blocks are ordinary visible blocks;
//! they are added from here, not from the library.

use crab_core::project::{MODBUS_KINDS, RADIO_KINDS};
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
    /// Sets one parameter of a block (the type, address or word order of a tag).
    SetParam {
        id: String,
        name: String,
        value: ParamValue,
    },
    /// Sets the table of a tag and its type, which the table may force, in one edit.
    SetTagTable {
        id: String,
        table: String,
        kind: String,
    },
    /// Renames a block (the name of a tag).
    Rename {
        old: String,
        new: String,
    },
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
            ui.horizontal_wrapped(|ui| {
                for (kind, caption, hint) in ADD {
                    // One modem per board (`single` in the library).
                    let enabled = kind != "lora_e220" || !modem_exists;
                    if crate::iconbtn::add_button(
                        ui,
                        enabled,
                        crate::i18n::tr(caption),
                        crate::i18n::tr(hint),
                    )
                    .clicked()
                    {
                        actions.push(RadioAction::Add(kind));
                    }
                }
            });
            for b in logic
                .blocks
                .iter()
                .filter(|b| b.tag.is_none() && RADIO_KINDS.contains(&b.kind.as_str()))
            {
                row(app, ui, b, true, &mut actions);
            }
        });
    ui.separator();
    actions
}

/// One block row: placed marker, id, and the find / settings / delete buttons. `show_kind` adds the
/// block name after the id (for lists that mix kinds under one header).
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
    let placed = doc.project.layout.blocks.contains_key(&b.id);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.dnd_drag_source(
            Id::unique(("radio", &b.id)),
            DragItem::Topic(b.id.clone()),
            |ui| ui.label(if placed { "▣" } else { "▢" }),
        )
        .response
        .on_hover_text(if placed {
            crate::i18n::tr("Блок уже на плате; перетащите, чтобы переставить")
        } else {
            crate::i18n::tr("Перетащите на плату")
        });
        ui.label(if show_kind {
            format!("{}  ·  {}", b.id, crate::i18n::tr(&name))
        } else {
            b.id.clone()
        });
        ui.add_space(4.0);
        if crate::iconbtn::icon_button(
            ui,
            placed,
            crate::iconbtn::Icon::Find,
            crate::i18n::tr("Найти на плате"),
        )
        .clicked()
        {
            actions.push(RadioAction::Focus(b.id.clone()));
        }
        let hint = if b.kind == "lora_e220" {
            Some("Роль, канал, скорость, ключ, UART и выводы модема")
        } else if MODBUS_KINDS.contains(&b.kind.as_str()) {
            Some("Параметры блока")
        } else {
            None
        };
        if hint.is_some_and(|hint| {
            crate::iconbtn::icon_button(
                ui,
                true,
                crate::iconbtn::Icon::Settings,
                &format!(
                    "{}: {}",
                    crate::i18n::tr("Настройки..."),
                    crate::i18n::tr(hint)
                ),
            )
            .clicked()
        }) {
            actions.push(RadioAction::Settings(b.id.clone()));
        }
        if crate::iconbtn::icon_button(
            ui,
            true,
            crate::iconbtn::Icon::Delete,
            crate::i18n::tr("Удалить из проекта"),
        )
        .clicked()
        {
            actions.push(RadioAction::Remove(b.id.clone()));
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
                // A Modbus block waits in the panel until it is dragged onto a sheet.
                let waits = MODBUS_KINDS.contains(&kind);
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
            RadioAction::SetParam { id, name, value } => {
                let Some(doc) = &mut app.doc else { return };
                let key = Some(format!("tag:{id}.{name}"));
                if let Err(e) = doc.edit(&lib, key, |p| p.set_param(&lib, &id, &name, Some(value)))
                {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            RadioAction::SetTagTable { id, table, kind } => {
                let Some(doc) = &mut app.doc else { return };
                let result = doc.edit(&lib, None, |p| {
                    p.set_param(&lib, &id, "table", Some(ParamValue::Text(table)))?;
                    p.set_param(&lib, &id, "kind", Some(ParamValue::Text(kind)))
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            RadioAction::Rename { old, new } => {
                let Some(doc) = &mut app.doc else { continue };
                // The tags and devices that name the block follow its new id.
                let result = doc.edit(&lib, None, |p| {
                    p.rename_block(&old, &new)
                        .map(|()| crate::modbuspanel::follow_rename(&mut p.logic, &old, &new))
                });
                if let Err(e) = result {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
        }
    }
}
