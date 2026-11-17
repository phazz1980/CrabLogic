//! Radio (LoRa) in the left panel: the modem and the partner blocks of the project with a button
//! to find them on the sheet and one to delete them. Radio blocks are ordinary visible blocks;
//! they are added from here, not from the library.

use crab_core::project::RADIO_KINDS;
use eframe::egui::{self, Id, Ui};

use crate::app::{EditorApp, LogLevel};
use crate::canvas::DragItem;

pub enum RadioAction {
    Add(&'static str),
    Focus(String),
    Remove(String),
}

/// What the «+» buttons add: (kind, caption, hint).
const ADD: [(&str, &str, &str); 3] = [
    (
        "lora_e220",
        "+ Модем",
        "Радиомодуль E220: в проекте он один (роль «центр» или «ведомый» задаётся в его свойствах)",
    ),
    (
        "lora_point",
        "+ Ведомый",
        "Блок «LoRa: ведомый» на стороне центра: команды ведомому и его ответы",
    ),
    (
        "lora_centre",
        "+ Центр",
        "Блок «LoRa: центр» на стороне ведомого: команды центра и значения в ответ",
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
                    if ui
                        .add_enabled(enabled, egui::Button::new(crate::i18n::tr(caption)).small())
                        .on_hover_text(crate::i18n::tr(hint))
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

fn row(app: &EditorApp, ui: &mut Ui, b: &crab_core::Block, actions: &mut Vec<RadioAction>) {
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
        if ui
            .add_enabled(placed, egui::Button::new("⌖").small())
            .on_hover_text(crate::i18n::tr("Найти на плате"))
            .clicked()
        {
            actions.push(RadioAction::Focus(b.id.clone()));
        }
        if ui
            .add(egui::Button::new("🗑").small())
            .on_hover_text(crate::i18n::tr("Удалить из проекта"))
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
        }
    }
}
