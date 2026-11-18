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
    /// Adds a slave tag: `modbus_write` («из схемы») or `modbus_read` («в схему»), at the next free
    /// address of its table.
    AddTag(&'static str),
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
                row(app, ui, b, &mut actions);
            }
        });
    ui.separator();
    actions
}

pub(crate) fn row(
    app: &EditorApp,
    ui: &mut Ui,
    b: &crab_core::Block,
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
        ui.label(format!("{}  ·  {}", b.id, crate::i18n::tr(&name)));
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
                match doc.edit(&lib, None, |p| p.add_block(&lib, kind, pos)) {
                    Ok(id) => app.focus_block(&id),
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
            RadioAction::Remove(id) => {
                let Some(doc) = &mut app.doc else { return };
                if let Err(e) = doc.edit(&lib, None, |p| p.remove_block(&id)) {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
            RadioAction::AddTag(kind) => {
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { return };
                let table = crate::modbuspanel::default_table(kind);
                let result = doc.edit(&lib, None, |p| {
                    let id = p.add_block(&lib, kind, pos)?;
                    // The tag waits in the panel until it is dragged onto a sheet.
                    p.layout.blocks.remove(&id);
                    let reg = p.logic.modbus_next_free(table, "int");
                    p.set_param(&lib, &id, "table", Some(ParamValue::Text(table.to_owned())))?;
                    p.set_param(&lib, &id, "kind", Some(ParamValue::Text("int".to_owned())))?;
                    p.set_param(&lib, &id, "reg", Some(ParamValue::Int(i64::from(reg))))?;
                    let named = crate::modbuspanel::default_id(reg);
                    if p.logic.block(&named).is_none() {
                        p.rename_block(&id, &named)
                    } else {
                        Ok(())
                    }
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
                let Some(doc) = &mut app.doc else { return };
                if let Err(e) = doc.edit(&lib, None, |p| p.rename_block(&old, &new)) {
                    app.info(LogLevel::Error, e.to_string());
                }
            }
        }
    }
}
