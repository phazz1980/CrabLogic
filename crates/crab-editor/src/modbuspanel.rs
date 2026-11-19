//! The «Modbus» folder inside the «Сеть» folder of the left panel: a tree of the Modbus blocks.
//! A master (RTU or TCP) holds its slave devices (`modbus_device`, hidden, never on a sheet), each
//! device holds its tags; the slave role of this board (`modbus_rtu_slave`, `modbus_tcp_slave`)
//! holds its tags directly. Masters, slaves and tags are added from here; a tag is dragged onto a
//! sheet from its row. The actions are those of the radio section (`radiopanel::apply` carries
//! them out).

use crab_core::modbus_map::{slot_size, ModbusConflict, TagDirection};
use crab_core::project::MODBUS_KINDS;
use crab_core::{Block, Logic, ParamValue};
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

/// Table a new tag gets.
pub const NEW_TAG_TABLE: &str = "holding";

/// Slaves a master takes when its `slaves` parameter is not set (the library default).
pub const DEFAULT_SLAVES: usize = 4;

/// Unit IDs a slave device can have (Modbus addresses 1..=247).
const UNIT_RANGE: std::ops::RangeInclusive<i64> = 1..=247;

/// What the «+» buttons add, in order: (kind, caption). The four roles of the board, one of each
/// kind per board (`single` in their library info).
const ADD: [(&str, &str); 4] = [
    ("modbus_rtu_master", "+ RTU Master"),
    ("modbus_rtu_slave", "+ RTU Slave"),
    ("modbus_tcp_master", "+ TCP Master"),
    ("modbus_tcp_slave", "+ TCP Slave"),
];

/// Whether `kind` is a master: it polls the slave devices under it.
pub fn is_master(kind: &str) -> bool {
    matches!(kind, "modbus_rtu_master" | "modbus_tcp_master")
}

/// Whether `kind` is the slave role of this board: its tags are the registers the master reads.
pub fn is_slave_role(kind: &str) -> bool {
    matches!(kind, "modbus_rtu_slave" | "modbus_tcp_slave")
}

/// Whether the devices under a master of `master_kind` have an IP address and a port: only a TCP
/// master reaches its devices by address, an RTU bus has neither.
pub fn shows_net_fields(master_kind: &str) -> bool {
    master_kind == "modbus_tcp_master"
}

/// Whether `text` is an IPv4 address written as a dotted quad `a.b.c.d` (each part 0..=255), with
/// surrounding spaces ignored. Empty text is valid too: it means the master's address.
pub fn valid_ip(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    let mut parts = 0;
    for part in text.split('.') {
        parts += 1;
        let octet = !part.is_empty()
            && part.len() <= 3
            && part.bytes().all(|c| c.is_ascii_digit())
            && part.parse::<u8>().is_ok();
        if !octet {
            return false;
        }
    }
    parts == 4
}

pub fn show(app: &EditorApp, ui: &mut Ui, actions: &mut Vec<RadioAction>) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let conflicts = logic.modbus_conflicts();
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
            // One group per role: its add button on its own line, its block and the block's
            // children right under it.
            for (kind, caption) in ADD {
                let info = app.lib.get(kind).map(|l| &l.def.info);
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
                    crate::radiopanel::row(app, ui, b, false, actions);
                    if is_master(kind) {
                        master_children(app, ui, b, &conflicts, actions);
                    } else {
                        slave_tags(app, ui, b, &conflicts, actions);
                    }
                }
            }
        });
}

/// Under a master: its «+ Slave» button and one row per slave device, each with its tags.
fn master_children(
    app: &EditorApp,
    ui: &mut Ui,
    master: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    ui.indent(("modbus_slaves", master.id.as_str()), |ui| {
        let devices = logic.modbus_devices(&master.id);
        let full = devices.len() >= slave_limit(master);
        if crate::iconbtn::add_button(
            ui,
            !full,
            crate::i18n::tr("+ Slave"),
            crate::i18n::tr(
                "Добавить slave (устройство Modbus) под этот master. Предел числа slave задаётся параметром «Макс. slave» в настройках master",
            ),
        )
        .clicked()
        {
            actions.push(RadioAction::AddDevice(master.id.clone()));
        }
        for device in devices {
            device_row(app, ui, master, device, conflicts, actions);
        }
    });
}

/// One slave device: its id, the Unit ID (applied at once), under a TCP master also the IP address
/// and port, and delete, which takes its tags too. Devices are never placed on a sheet, so they
/// have no find or drag.
fn device_row(
    app: &EditorApp,
    ui: &mut Ui,
    master: &Block,
    device: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.label(device.id.as_str());
        ui.weak(crate::i18n::tr("Unit ID"));
        let mut unit = int_param(device, "unit")
            .unwrap_or(*UNIT_RANGE.start())
            .clamp(*UNIT_RANGE.start(), *UNIT_RANGE.end());
        if ui
            .add(egui::DragValue::new(&mut unit).range(UNIT_RANGE))
            .on_hover_text(crate::i18n::tr("Адрес устройства (Unit ID), 1..247"))
            .changed()
        {
            actions.push(RadioAction::SetParam {
                id: device.id.clone(),
                name: "unit".to_owned(),
                value: ParamValue::Int(unit),
            });
        }
        if shows_net_fields(&master.kind) {
            ip_field(ui, master, device, actions);
            port_field(ui, device, actions);
        }
        if crate::iconbtn::icon_button(
            ui,
            true,
            crate::iconbtn::Icon::Delete,
            crate::i18n::tr("Удалить slave вместе с его тегами"),
        )
        .clicked()
        {
            actions.push(RadioAction::Remove(device.id.clone()));
        }
    });
    ui.indent(("modbus_device_tags", device.id.as_str()), |ui| {
        tag_group(app, ui, &device.id, conflicts, actions);
    });
}

/// The IP address of a device under a TCP master. Like the tag name it is edited in a buffer of
/// its own and applied when the field loses focus, and only if it is a valid address or empty
/// (empty = the master's address). The text is red while it is not an address.
fn ip_field(ui: &mut Ui, master: &Block, device: &Block, actions: &mut Vec<RadioAction>) {
    let saved = text_param(device, "ip", "");
    let master_ip = text_param(master, "ip", "");
    let hint = if master_ip.is_empty() {
        crate::i18n::tr("адрес master")
    } else {
        master_ip
    };
    let buffer_id = ui.scope_id().with(("modbus_device_ip", device.id.as_str()));
    let mut ip = ui
        .data(|d| d.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| saved.to_owned());
    let red = (!valid_ip(&ip)).then_some(Color32::RED);
    let edit = ui
        .add(
            egui::TextEdit::singleline(&mut ip)
                .desired_width(110.0)
                .hint_text(hint.to_owned())
                .text_color_opt(red),
        )
        .on_hover_text(crate::i18n::tr(
            "IP-адрес slave, вид a.b.c.d; пусто — адрес master",
        ));
    if edit.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(buffer_id));
        let new = ip.trim();
        if valid_ip(new) && new != saved {
            actions.push(RadioAction::SetParam {
                id: device.id.clone(),
                name: "ip".to_owned(),
                value: ParamValue::Text(new.to_owned()),
            });
        }
    } else if edit.changed() {
        ui.data_mut(|d| d.insert_temp(buffer_id, ip));
    }
}

/// The port of a device under a TCP master; 0 means the master's port.
fn port_field(ui: &mut Ui, device: &Block, actions: &mut Vec<RadioAction>) {
    let mut port = int_param(device, "port")
        .unwrap_or(0)
        .clamp(0, i64::from(u16::MAX));
    if ui
        .add(
            egui::DragValue::new(&mut port)
                .range(0..=i64::from(u16::MAX))
                .prefix(crate::i18n::tr("порт ")),
        )
        .on_hover_text(crate::i18n::tr("0 — порт master"))
        .changed()
    {
        actions.push(RadioAction::SetParam {
            id: device.id.clone(),
            name: "port".to_owned(),
            value: ParamValue::Int(port),
        });
    }
}

/// Under a slave-role block: its tags, with the «+ Тег» button.
fn slave_tags(
    app: &EditorApp,
    ui: &mut Ui,
    slave: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    ui.indent(("modbus_slave_tags", slave.id.as_str()), |ui| {
        tag_group(app, ui, &slave.id, conflicts, actions);
    });
}

/// The «+ Тег» button of one owner (a device or a slave-role block) and its tag rows.
fn tag_group(
    app: &EditorApp,
    ui: &mut Ui,
    owner: &str,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    if crate::iconbtn::add_button(
        ui,
        true,
        crate::i18n::tr("+ Тег"),
        crate::i18n::tr(
            "Добавить тег: регистр или бит. Адрес — следующий свободный в таблице Holding",
        ),
    )
    .clicked()
    {
        actions.push(RadioAction::AddTag(owner.to_owned()));
    }
    for slot in logic.modbus_tags_of(owner) {
        // Copies of a tag (blocks with `tag` set) are not listed.
        if let Some(b) = logic.block(&slot.id).filter(|b| b.tag.is_none()) {
            tag_row(app, ui, b, conflicts, actions);
        }
    }
}

/// One tag: placed marker and drag handle, name, find button, table, type, address, «слова
/// наоборот» for float, size, direction, delete. The conflicts of the tag follow in red.
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
    let direction = doc.project.logic.modbus_tag_direction(id);
    let table = text_param(b, "table", NEW_TAG_TABLE);
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
            .selected_text(table_caption(table))
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
                    kind: kind_for_table(value, kind),
                });
            }
        }

        let mut picked_kind = None;
        egui::ComboBox::from_id_salt(("modbus_tag_kind", id))
            .width(70.0)
            .selected_text(kind)
            .show_ui(ui, |ui| {
                for allowed in allowed_kinds(table) {
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

        ui.weak(size_text(table, kind));

        let text = crate::i18n::tr(direction_caption(direction));
        let hint = crate::i18n::tr(
            "Направление задаёт связь на листе: подключён выход — тег читается из устройства, подключён вход — пишется в него",
        );
        if direction == TagDirection::Both {
            ui.colored_label(Color32::RED, text).on_hover_text(hint);
        } else {
            ui.weak(text).on_hover_text(hint);
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

    for conflict in conflicts.iter().filter(|c| conflict_mentions(c, id)) {
        ui.colored_label(Color32::RED, conflict_message(conflict));
    }
}

/// Text value of a parameter, `default` when it is not set or not text.
fn text_param<'a>(b: &'a Block, name: &str, default: &'a str) -> &'a str {
    match b.params.get(name) {
        Some(ParamValue::Text(t)) => t.as_str(),
        _ => default,
    }
}

/// Integer value of a parameter, `None` when it is not set or not an integer.
fn int_param(b: &Block, name: &str) -> Option<i64> {
    match b.params.get(name) {
        Some(ParamValue::Int(n)) => Some(*n),
        _ => None,
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

/// Cells a tag takes, as text for the row: bits for coil and discrete, registers otherwise.
pub fn size_text(table: &str, kind: &str) -> String {
    if matches!(table, "coil" | "discrete") {
        "1 бит".to_owned()
    } else {
        format!("{} рег.", slot_size(table, kind))
    }
}

/// The label of a tag's direction, in the words of the tree.
pub fn direction_caption(direction: TagDirection) -> &'static str {
    match direction {
        TagDirection::Read => "читает",
        TagDirection::Write => "пишет",
        TagDirection::Both => "читает и пишет!",
        TagDirection::None => "не подключён",
    }
}

/// How many slaves a master takes: its `slaves` parameter, the library default when it is not set.
pub fn slave_limit(master: &Block) -> usize {
    int_param(master, "slaves")
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEFAULT_SLAVES)
}

/// Whether the master `master_id` has as many slave devices as it takes.
pub fn slaves_full(logic: &Logic, master_id: &str) -> bool {
    let limit = logic.block(master_id).map_or(DEFAULT_SLAVES, slave_limit);
    logic.modbus_devices(master_id).len() >= limit
}

/// The smallest Unit ID from 1 that no slave device of `master_id` has.
pub fn next_unit(logic: &Logic, master_id: &str) -> i64 {
    let used: Vec<i64> = logic
        .modbus_devices(master_id)
        .iter()
        .filter_map(|d| int_param(d, "unit"))
        .collect();
    (*UNIT_RANGE.start()..)
        .find(|unit| !used.contains(unit))
        .unwrap_or(*UNIT_RANGE.start())
}

/// The id of a new tag at address `reg`: `reg_<reg>`, with a numeric suffix when it is taken.
pub fn free_tag_id(logic: &Logic, reg: u16) -> String {
    let base = format!("reg_{reg}");
    if logic.block(&base).is_none() {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|id| logic.block(id).is_none())
        .expect("infinite range")
}

/// Ids to remove with the block `id`, the block first. A master takes its devices; a master, a
/// device and a slave-role block take the tags of each. A tag comes with its copies first:
/// removing a tag that has copies only hands it over to a copy.
pub fn cascade_ids(logic: &Logic, id: &str) -> Vec<String> {
    let Some(block) = logic.block(id) else {
        return vec![id.to_owned()];
    };
    let mut ids = vec![id.to_owned()];
    let owners: Vec<String> = if is_master(&block.kind) {
        let devices: Vec<String> = logic
            .modbus_devices(id)
            .iter()
            .map(|d| d.id.clone())
            .collect();
        ids.extend(devices.iter().cloned());
        std::iter::once(id.to_owned()).chain(devices).collect()
    } else if block.kind == "modbus_device" || is_slave_role(&block.kind) {
        vec![id.to_owned()]
    } else {
        return ids;
    };
    for owner in &owners {
        for slot in logic.modbus_tags_of(owner) {
            ids.extend(logic.tag_copies(&slot.id).map(str::to_owned));
            ids.push(slot.id);
        }
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

/// Makes the parameters that name a block follow its rename: `device` of the tags and `master`
/// of the slave devices. Call it after `Project::rename_block`, which does not update them.
pub fn follow_rename(logic: &mut Logic, old: &str, new: &str) {
    for b in &mut logic.blocks {
        let param = match b.kind.as_str() {
            "modbus_tag" => "device",
            "modbus_device" => "master",
            _ => continue,
        };
        if let Some(ParamValue::Text(t)) = b.params.get_mut(param) {
            if t == old {
                *t = new.to_owned();
            }
        }
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
            ..
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

    fn block(id: &str, kind: &str, params: &[(&str, ParamValue)]) -> Block {
        let mut b = Block {
            id: id.to_owned(),
            kind: kind.to_owned(),
            ..Default::default()
        };
        for (name, value) in params {
            b.params.insert((*name).to_owned(), value.clone());
        }
        b
    }

    fn text(s: &str) -> ParamValue {
        ParamValue::Text(s.to_owned())
    }

    fn device(id: &str, master: &str, unit: i64) -> Block {
        block(
            id,
            "modbus_device",
            &[("master", text(master)), ("unit", ParamValue::Int(unit))],
        )
    }

    fn tag(id: &str, owner: &str, reg: i64) -> Block {
        block(
            id,
            "modbus_tag",
            &[
                ("device", text(owner)),
                ("table", text("holding")),
                ("kind", text("int")),
                ("reg", ParamValue::Int(reg)),
            ],
        )
    }

    fn logic(blocks: Vec<Block>) -> Logic {
        Logic {
            blocks,
            ..Default::default()
        }
    }

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
    fn new_tags_go_to_holding() {
        assert_eq!(NEW_TAG_TABLE, "holding");
    }

    #[test]
    fn free_tag_id_is_the_address_until_taken() {
        let empty = Logic::default();
        assert_eq!(free_tag_id(&empty, 0), "reg_0");
        assert_eq!(free_tag_id(&empty, 40001), "reg_40001");

        let one = logic(vec![block("reg_100", "modbus_tag", &[])]);
        assert_eq!(free_tag_id(&one, 100), "reg_100_2");
        let two = logic(vec![
            block("reg_100", "modbus_tag", &[]),
            block("reg_100_2", "modbus_tag", &[]),
        ]);
        assert_eq!(free_tag_id(&two, 100), "reg_100_3");
        assert_eq!(free_tag_id(&two, 101), "reg_101");
    }

    #[test]
    fn next_unit_is_the_smallest_free_one() {
        let none = logic(vec![block("m", "modbus_rtu_master", &[])]);
        assert_eq!(next_unit(&none, "m"), 1);

        let gap = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            device("d2", "m", 2),
            device("d3", "m", 4),
        ]);
        assert_eq!(next_unit(&gap, "m"), 3);

        // Units of another master do not count.
        let other = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            block("n", "modbus_tcp_master", &[]),
            device("d1", "n", 1),
        ]);
        assert_eq!(next_unit(&other, "m"), 1);
        assert_eq!(next_unit(&other, "n"), 2);
    }

    #[test]
    fn slave_limit_comes_from_the_master() {
        let plain = block("m", "modbus_rtu_master", &[]);
        assert_eq!(slave_limit(&plain), DEFAULT_SLAVES);
        let eight = block("m", "modbus_rtu_master", &[("slaves", ParamValue::Int(8))]);
        assert_eq!(slave_limit(&eight), 8);

        let full = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            device("d2", "m", 2),
            device("d3", "m", 3),
            device("d4", "m", 4),
        ]);
        assert!(slaves_full(&full, "m"));
        assert!(!slaves_full(&full, "other"));
    }

    #[test]
    fn cascade_of_a_master_takes_devices_and_their_tags() {
        let project = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            device("d2", "m", 2),
            tag("t1", "d1", 0),
            tag("t2", "d2", 0),
            block("other", "modbus_tcp_master", &[]),
            device("x1", "other", 1),
            tag("tx", "x1", 0),
        ]);
        assert_eq!(
            cascade_ids(&project, "m"),
            ["m", "d1", "d2", "t1", "t2"].map(str::to_owned)
        );
    }

    #[test]
    fn cascade_of_a_device_and_of_a_slave_role_takes_their_tags() {
        let project = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            device("d2", "m", 2),
            tag("t1", "d1", 0),
            tag("t2", "d2", 0),
            block("s", "modbus_tcp_slave", &[]),
            tag("ts", "s", 0),
        ]);
        assert_eq!(cascade_ids(&project, "d1"), ["d1", "t1"].map(str::to_owned));
        assert_eq!(cascade_ids(&project, "s"), ["s", "ts"].map(str::to_owned));
        assert_eq!(cascade_ids(&project, "t1"), ["t1"].map(str::to_owned));
        assert_eq!(
            cascade_ids(&project, "missing"),
            ["missing"].map(str::to_owned)
        );
    }

    #[test]
    fn cascade_puts_copies_before_their_tag() {
        let mut copy = tag("t1_copy", "d1", 0);
        copy.tag = Some("t1".to_owned());
        let project = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            tag("t1", "d1", 0),
            copy,
        ]);
        assert_eq!(
            cascade_ids(&project, "d1"),
            ["d1", "t1_copy", "t1"].map(str::to_owned)
        );
    }

    #[test]
    fn rename_moves_the_references() {
        let mut project = logic(vec![
            block("m", "modbus_rtu_master", &[]),
            device("d1", "m", 1),
            device("d2", "n", 1),
            tag("t1", "d1", 0),
            tag("t2", "s", 0),
        ]);
        follow_rename(&mut project, "m", "master");
        assert_eq!(project.modbus_devices("master").len(), 1);
        assert_eq!(project.modbus_devices("n").len(), 1);

        follow_rename(&mut project, "d1", "dev");
        assert_eq!(project.modbus_tags_of("dev").len(), 1);
        assert_eq!(project.modbus_tags_of("s").len(), 1);
        assert_eq!(project.modbus_tags_of("d1").len(), 0);
    }

    #[test]
    fn direction_is_worded_for_the_row() {
        assert_eq!(direction_caption(TagDirection::Read), "читает");
        assert_eq!(direction_caption(TagDirection::Write), "пишет");
        assert_eq!(direction_caption(TagDirection::Both), "читает и пишет!");
        assert_eq!(direction_caption(TagDirection::None), "не подключён");
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
            owner: "d".to_owned(),
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

    #[test]
    fn only_a_tcp_master_shows_ip_and_port() {
        assert!(shows_net_fields("modbus_tcp_master"));
        assert!(!shows_net_fields("modbus_rtu_master"));
        assert!(!shows_net_fields("modbus_tcp_slave"));
        assert!(!shows_net_fields(""));
    }

    #[test]
    fn ip_must_be_a_dotted_quad_or_empty() {
        for good in [
            "",
            "   ",
            "192.168.1.10",
            "0.0.0.0",
            "255.255.255.255",
            " 10.0.0.5 ",
        ] {
            assert!(valid_ip(good), "{good:?} should be valid");
        }
        for bad in [
            "256.1.1.1",
            "1.2.3",
            "1.2.3.4.5",
            "1.2.3.",
            "1..2.3",
            ".1.2.3",
            "1.2.3.-1",
            "+1.2.3.4",
            "1234.1.1.1",
            "a.b.c.d",
            "1.2.3.4 5",
            "1.2.3.4/24",
        ] {
            assert!(!valid_ip(bad), "{bad:?} should be invalid");
        }
    }
}
