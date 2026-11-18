//! The «Modbus» folder inside the «Сеть» folder of the left panel: the Modbus blocks of the project
//! with buttons to add them, find them on the sheet, open their settings and delete them. They are
//! ordinary visible blocks; they are added from here, not from the library. The actions are those
//! of the radio section (`radiopanel::apply` carries them out).
//!
//! The slave tags are the hidden `modbus_write` («из схемы») and `modbus_read` («в схему») blocks.
//! They get a table of their own, see [`tags`].

use crab_core::modbus_map::{slot_size, ModbusConflict};
use crab_core::project::MODBUS_KINDS;
use crab_core::{Block, ParamValue};
use eframe::egui::{self, Color32, Ui};

use crate::app::EditorApp;
use crate::canvas::DragItem;
use crate::radiopanel::RadioAction;

/// Tables in the order of their combo box: (value, caption).
const TABLES: [(&str, &str); 4] = [
    ("holding", "Holding"),
    ("input", "Input"),
    ("coil", "Coil"),
    ("discrete", "Discrete"),
];

/// What the «+» buttons add, in order: (kind, caption).
const ADD: [(&str, &str); 6] = [
    ("modbus_rtu_master", "+ RTU Master"),
    ("modbus_rtu_slave", "+ RTU Slave"),
    ("modbus_tcp_master", "+ TCP Master"),
    ("modbus_tcp_slave", "+ TCP Slave"),
    ("modbus_master_read", "+ Чтение (master)"),
    ("modbus_master_write", "+ Запись (master)"),
];

pub fn show(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<RadioAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let has = |kind: &str| logic.blocks.iter().any(|b| b.kind == kind);
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new("Modbus").strong())
        .id_salt("modbus_panel")
        .default_open(
            logic
                .blocks
                .iter()
                .any(|b| MODBUS_KINDS.contains(&b.kind.as_str())),
        )
        .show(ui, |ui| {
            // One group per kind: its add button on its own line, its blocks right under it.
            for (kind, caption) in ADD {
                let info = app.lib.get(kind).map(|l| &l.def.info);
                // Masters, slaves and the RTU/TCP links are one per board (`single`).
                let enabled = !info.is_some_and(|i| i.single) || !has(kind);
                let hint = info.map_or("", |i| i.description.as_str());
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
                for b in logic
                    .blocks
                    .iter()
                    .filter(|b| b.tag.is_none() && b.kind == kind)
                {
                    crate::radiopanel::row(app, ui, b, actions);
                }
            }
            tags(app, ui, actions);
        });
}

/// The «Теги slave» sub-section: two buttons that add a tag and one row per tag. Copies of a tag
/// (blocks with `tag` set) are not listed.
fn tags(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<RadioAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let conflicts = logic.modbus_conflicts();
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr("Теги slave")).strong())
        .id_salt("modbus_tags")
        .default_open(true)
        .show(ui, |ui| {
            let buttons = [
                (
                    "modbus_write",
                    "+ Из схемы",
                    "Тег «из схемы»: схема записывает значение, master его читает",
                ),
                (
                    "modbus_read",
                    "+ В схему",
                    "Тег «в схему»: master записывает значение, схема его читает",
                ),
            ];
            // One group per direction: its add button, its tags right under it.
            for (kind, caption, hint) in buttons {
                if crate::iconbtn::add_button(
                    ui,
                    true,
                    crate::i18n::tr(caption),
                    crate::i18n::tr(hint),
                )
                .clicked()
                {
                    actions.push(RadioAction::AddTag(kind));
                }
                for b in logic
                    .blocks
                    .iter()
                    .filter(|b| b.tag.is_none() && b.kind == kind)
                {
                    tag_row(app, ui, b, &conflicts, actions);
                }
            }
        });
}

/// One tag: placed marker and drag handle, direction, name, find button, table, type, address,
/// «слова наоборот» for float, size, delete. The conflicts of the tag follow in red.
fn tag_row(
    app: &EditorApp,
    ui: &mut Ui,
    b: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let id = b.id.as_str();
    let placed = doc.project.layout.blocks.contains_key(&b.id);
    let from_schema = b.kind == "modbus_write";
    let table = text_param(b, "table", default_table(&b.kind));
    let kind = text_param(b, "kind", "int");
    let reg = match b.params.get("reg") {
        Some(ParamValue::Int(n)) => (*n).clamp(0, i64::from(u16::MAX)),
        _ => 0,
    };
    let swap = matches!(b.params.get("swap"), Some(ParamValue::Bool(true)));

    ui.horizontal_wrapped(|ui| {
        ui.dnd_drag_source(
            ui.scope_id().with(("modbus_tag_drag", id)),
            DragItem::Topic(b.id.clone()),
            |ui| ui.label(if placed { "▣" } else { "▢" }),
        )
        .response
        .on_hover_text(if placed {
            crate::i18n::tr("Тег уже на плате; перетащите, чтобы переставить")
        } else {
            crate::i18n::tr("Перетащите на плату")
        });
        ui.label(crate::i18n::tr(if from_schema {
            "из схемы"
        } else {
            "в схему"
        }));

        // The name is edited in a buffer of its own and renamed when the field loses focus.
        let name_id = ui.scope_id().with(("modbus_tag_name", id));
        let mut name = ui
            .data(|d| d.get_temp::<String>(name_id))
            .unwrap_or_else(|| id.to_owned());
        let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(110.0));
        if edit.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(name_id));
            let new = name.trim();
            if new != id {
                actions.push(RadioAction::Rename {
                    old: id.to_owned(),
                    new: new.to_owned(),
                });
            }
        } else if edit.changed() {
            ui.data_mut(|d| d.insert_temp(name_id, name));
        }

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

        let mut picked_table = None;
        egui::ComboBox::from_id_salt(("modbus_tag_table", id))
            .width(90.0)
            .selected_text(table_caption(&table))
            .show_ui(ui, |ui| {
                for (value, caption) in TABLES {
                    if ui.selectable_label(table == value, caption).clicked() {
                        picked_table = Some(value);
                    }
                }
            })
            .response
            .on_hover_text(crate::i18n::tr("Таблица регистров Modbus"));
        if let Some(value) = picked_table {
            if value != table {
                actions.push(RadioAction::SetTagTable {
                    id: b.id.clone(),
                    table: value.to_owned(),
                    kind: kind_for_table(value, &kind),
                });
            }
        }

        let mut picked_kind = None;
        egui::ComboBox::from_id_salt(("modbus_tag_kind", id))
            .width(70.0)
            .selected_text(kind.as_str())
            .show_ui(ui, |ui| {
                for allowed in allowed_kinds(&table) {
                    if ui.selectable_label(kind == *allowed, *allowed).clicked() {
                        picked_kind = Some(*allowed);
                    }
                }
            })
            .response
            .on_hover_text(crate::i18n::tr("Тип значения"));
        if let Some(value) = picked_kind {
            if value != kind {
                actions.push(RadioAction::SetParam {
                    id: b.id.clone(),
                    name: "kind".to_owned(),
                    value: ParamValue::Text(value.to_owned()),
                });
            }
        }

        let mut address = reg;
        if ui
            .add(egui::DragValue::new(&mut address).range(0..=i64::from(u16::MAX)))
            .on_hover_text(crate::i18n::tr("Адрес в таблице"))
            .changed()
        {
            actions.push(RadioAction::SetParam {
                id: b.id.clone(),
                name: "reg".to_owned(),
                value: ParamValue::Int(address),
            });
        }

        if kind == "float" {
            let mut word_swap = swap;
            if ui
                .checkbox(&mut word_swap, crate::i18n::tr("слова наоборот"))
                .on_hover_text(crate::i18n::tr(
                    "Поменять местами два регистра float (порядок CDAB вместо ABCD)",
                ))
                .changed()
            {
                actions.push(RadioAction::SetParam {
                    id: b.id.clone(),
                    name: "swap".to_owned(),
                    value: ParamValue::Bool(word_swap),
                });
            }
        }

        ui.weak(size_text(&table, &kind));

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

    for conflict in conflicts.iter().filter(|c| conflict_mentions(c, id)) {
        ui.colored_label(Color32::RED, conflict_message(conflict));
    }
}

/// Text value of a parameter, `default` when it is not set or not text.
fn text_param(b: &Block, name: &str, default: &str) -> String {
    match b.params.get(name) {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => default.to_owned(),
    }
}

/// Caption of a table in its combo box.
fn table_caption(table: &str) -> &str {
    TABLES
        .iter()
        .find(|(value, _)| *value == table)
        .map_or(table, |(_, caption)| caption)
}

/// Types a table takes: coil and discrete tables are bits (`bool`), holding and input tables are
/// registers (`int` or `float`).
pub fn allowed_kinds(table: &str) -> &'static [&'static str] {
    match table {
        "coil" | "discrete" => &["bool"],
        _ => &["int", "float"],
    }
}

/// The type a tag has after its table changes to `table`: `kind` when the table takes it,
/// otherwise the table's first type (`bool` in a bit table, `int` in a register table).
pub fn kind_for_table(table: &str, kind: &str) -> String {
    let allowed = allowed_kinds(table);
    let kept = allowed
        .iter()
        .find(|k| **k == kind)
        .copied()
        .unwrap_or(allowed[0]);
    kept.to_owned()
}

/// The table a new tag of block `kind` gets: the block default («из схемы» is `input`, «в схему»
/// is `holding`).
pub fn default_table(kind: &str) -> &'static str {
    if kind == "modbus_write" {
        "input"
    } else {
        "holding"
    }
}

/// The id of a new tag at address `reg`, until the user renames it.
pub fn default_id(reg: u16) -> String {
    format!("reg_{reg}")
}

/// Cells a tag takes, as text for the row: bits for coil and discrete, registers otherwise.
pub fn size_text(table: &str, kind: &str) -> String {
    if matches!(table, "coil" | "discrete") {
        "1 бит".to_owned()
    } else {
        format!("{} рег.", slot_size(table, kind))
    }
}

/// A conflict in the words of the check of the schema (`check::modbus`).
pub fn conflict_message(conflict: &ModbusConflict) -> String {
    match conflict {
        ModbusConflict::Overlap {
            table,
            first,
            second,
            address,
        } => format!(
            "Modbus: теги «{first}» и «{second}» пересекаются в таблице {table} (адрес {address})"
        ),
        ModbusConflict::BadKind { id, table, kind } => {
            format!("Modbus: тег «{id}»: тип {kind} не подходит таблице {table}")
        }
    }
}

/// Whether the conflict is about the tag `id`.
pub fn conflict_mentions(conflict: &ModbusConflict, id: &str) -> bool {
    match conflict {
        ModbusConflict::Overlap { first, second, .. } => first == id || second == id,
        ModbusConflict::BadKind { id: bad, .. } => bad == id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_tables_take_bool_only() {
        assert_eq!(allowed_kinds("coil"), ["bool"]);
        assert_eq!(allowed_kinds("discrete"), ["bool"]);
        assert_eq!(allowed_kinds("holding"), ["int", "float"]);
        assert_eq!(allowed_kinds("input"), ["int", "float"]);
    }

    #[test]
    fn table_change_corrects_the_type() {
        // Bit tables: anything that is not bool becomes bool.
        assert_eq!(kind_for_table("coil", "int"), "bool");
        assert_eq!(kind_for_table("discrete", "float"), "bool");
        assert_eq!(kind_for_table("coil", "bool"), "bool");
        // Register tables: bool becomes int, int and float stay.
        assert_eq!(kind_for_table("holding", "bool"), "int");
        assert_eq!(kind_for_table("input", "bool"), "int");
        assert_eq!(kind_for_table("input", "float"), "float");
        assert_eq!(kind_for_table("holding", "int"), "int");
    }

    #[test]
    fn default_table_follows_the_direction() {
        assert_eq!(default_table("modbus_write"), "input");
        assert_eq!(default_table("modbus_read"), "holding");
    }

    #[test]
    fn default_id_is_the_address() {
        assert_eq!(default_id(0), "reg_0");
        assert_eq!(default_id(40001), "reg_40001");
    }

    #[test]
    fn size_text_counts_cells() {
        assert_eq!(size_text("holding", "float"), "2 рег.");
        assert_eq!(size_text("input", "int"), "1 рег.");
        assert_eq!(size_text("coil", "bool"), "1 бит");
        assert_eq!(size_text("discrete", "bool"), "1 бит");
    }

    #[test]
    fn conflicts_are_worded_as_in_the_check() {
        let overlap = ModbusConflict::Overlap {
            table: "holding".to_owned(),
            first: "a".to_owned(),
            second: "b".to_owned(),
            address: 3,
        };
        assert_eq!(
            conflict_message(&overlap),
            "Modbus: теги «a» и «b» пересекаются в таблице holding (адрес 3)"
        );
        assert!(conflict_mentions(&overlap, "a"));
        assert!(conflict_mentions(&overlap, "b"));
        assert!(!conflict_mentions(&overlap, "c"));

        let bad = ModbusConflict::BadKind {
            id: "x".to_owned(),
            table: "coil".to_owned(),
            kind: "int".to_owned(),
        };
        assert_eq!(
            conflict_message(&bad),
            "Modbus: тег «x»: тип int не подходит таблице coil"
        );
        assert!(conflict_mentions(&bad, "x"));
        assert!(!conflict_mentions(&bad, "y"));
    }
}
