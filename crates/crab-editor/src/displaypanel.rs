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
    SetParam {
        id: String,
        name: &'static str,
        value: ParamValue,
    },
    /// Adds or removes the menu block of a display.
    Menu {
        display: String,
        on: bool,
    },
    Designer(String),
    Focus(String),
}

pub fn is_display(kind: &str) -> bool {
    DISPLAY_KINDS.contains(&kind)
}

/// Address as shown to the user: `0x3C`.
pub fn format_address(a: i64) -> String {
    format!("0x{a:02X}")
}

/// Parses `0x3C` (any case) or decimal `60`; only 7-bit I²C addresses are accepted.
pub fn parse_address(text: &str) -> Option<i64> {
    let t = text.trim();
    let n = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => i64::from_str_radix(h, 16).ok()?,
        None => t.parse().ok()?,
    };
    (0..=127).contains(&n).then_some(n)
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
            ui.horizontal(|ui| {
                if ui.small_button("+ OLED").clicked() {
                    actions.push(DisplayAction::Add("oled"));
                }
                if ui.small_button("+ LCD").clicked() {
                    actions.push(DisplayAction::Add("lcd"));
                }
            });
            for b in logic.blocks.iter().filter(|b| is_display(&b.kind)) {
                display(app, ui, b, &mut actions);
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
    ui.horizontal(|ui| {
        ui.dnd_drag_source(
            Id::unique(("display", &b.id)),
            DragItem::Topic(b.id.clone()),
            |ui| ui.label(if placed { "▣" } else { "▢" }),
        )
        .response
        .on_hover_text(if placed {
            crate::i18n::tr("Блок уже на плате; перетащите, чтобы переставить")
        } else {
            crate::i18n::tr("Перетащите на плату")
        });
        egui::CollapsingHeader::new(format!("{} ({model_label})", b.id))
            .id_salt(("display", &b.id))
            .default_open(false)
            .show(ui, |ui| {
                if let Some(def) = lib.as_ref().and_then(|l| l.def.param("model")) {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Вид дисплея"));
                        egui::ComboBox::from_id_salt(("display_model", &b.id))
                            .selected_text(model_label.clone())
                            .show_ui(ui, |ui| {
                                for c in &def.choices {
                                    if ui
                                        .selectable_label(Some(&c.value) == model.as_ref(), &c.label)
                                        .clicked()
                                    {
                                        actions.push(DisplayAction::SetParam {
                                            id: b.id.clone(),
                                            name: "model",
                                            value: c.value.clone(),
                                        });
                                    }
                                }
                            });
                    });
                }
                address_row(app, ui, b, actions);
                let menus = menus_of(logic, &b.id);
                let mut on = !menus.is_empty();
                if ui
                    .checkbox(&mut on, crate::i18n::tr("Меню"))
                    .on_hover_text(crate::i18n::tr(
                        "Блок меню на той же плате, что и дисплей; при выключении блок меню удаляется",
                    ))
                    .changed()
                {
                    actions.push(DisplayAction::Menu {
                        display: b.id.clone(),
                        on,
                    });
                }
                if ui.button(crate::i18n::tr("Экраны и меню...")).clicked() {
                    actions.push(DisplayAction::Designer(b.id.clone()));
                }
                if ui
                    .add_enabled(placed, egui::Button::new(crate::i18n::tr("Найти на плате")))
                    .clicked()
                {
                    actions.push(DisplayAction::Focus(b.id.clone()));
                }
            });
    });
}

/// Address field: shown as hex, edited as text, applied when the field loses focus.
fn address_row(app: &EditorApp, ui: &mut Ui, b: &Block, actions: &mut Vec<DisplayAction>) {
    let current = match param_value(app, b, "address") {
        Some(ParamValue::Int(a)) => a,
        _ => return,
    };
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Адрес I²C"));
        let buf_id = ui.make_persistent_id(("display_address", &b.id));
        let mut text: String = ui
            .data_mut(|d| d.get_temp(buf_id))
            .unwrap_or_else(|| format_address(current));
        let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(60.0));
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(buf_id, text.clone()));
        } else {
            if resp.lost_focus() || resp.changed() {
                if let Some(a) = parse_address(&text).filter(|a| *a != current) {
                    actions.push(DisplayAction::SetParam {
                        id: b.id.clone(),
                        name: "address",
                        value: ParamValue::Int(a),
                    });
                }
            }
            ui.data_mut(|d| d.remove_temp::<String>(buf_id));
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
            DisplayAction::Designer(id) => app.open_web_designer(&format!("/screens?display={id}")),
            DisplayAction::Add(kind) => {
                let pos = crate::panels::free_spot(app);
                let Some(doc) = &mut app.doc else { return };
                match doc.edit(&lib, None, |p| p.add_block(&lib, kind, pos)) {
                    Ok(id) => app.focus_block(&id),
                    Err(e) => app.info(LogLevel::Error, e.to_string()),
                }
            }
            DisplayAction::SetParam { id, name, value } => {
                let Some(doc) = &mut app.doc else { return };
                let key = Some(format!("display:{id}.{name}"));
                if let Err(e) = doc.edit(&lib, key, |p| p.set_param(&lib, &id, name, Some(value))) {
                    app.info(LogLevel::Error, e.to_string());
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
    fn address_text_round_trips() {
        assert_eq!(format_address(60), "0x3C");
        assert_eq!(format_address(5), "0x05");
        assert_eq!(parse_address("0x3c"), Some(60));
        assert_eq!(parse_address(" 0X27 "), Some(39));
        assert_eq!(parse_address("61"), Some(61));
        assert_eq!(parse_address("0x80"), None);
        assert_eq!(parse_address("zz"), None);
        assert_eq!(parse_address(""), None);
    }

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
