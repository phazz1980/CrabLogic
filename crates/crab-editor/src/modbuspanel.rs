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
                let add = crate::panels::add_group(
                    ui,
                    ("modbus_kind", kind),
                    crate::i18n::tr(caption),
                    crate::i18n::tr(hint),
                    enabled,
                    |ui| {
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
                    },
                );
                if add {
                    actions.push(RadioAction::Add(kind));
                }
            }
        });
}

/// Under a master: a «Slave» group (with «+») and one row per slave device, each with its tags.
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
        let add = crate::panels::add_group(
            ui,
            ("modbus_devices", master.id.as_str()),
            "Slave",
            crate::i18n::tr(
                "Добавить slave (устройство Modbus) под этот master. Предел числа slave задаётся параметром «Макс. slave» в настройках master",
            ),
            !full,
            |ui| {
                for device in devices {
                    device_row(app, ui, master, device, conflicts, actions);
                }
            },
        );
        if add {
            actions.push(RadioAction::AddDevice(master.id.clone()));
        }
    });
}

/// One slave device: a flat row with its Unit ID (and under a TCP master the address and port);
/// a double click opens its settings, the context menu deletes it with its tags. Devices are never
/// placed on a sheet, so they have no mark, find or drag. Its tags follow in a group.
fn device_row(
    app: &EditorApp,
    ui: &mut Ui,
    master: &Block,
    device: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let unit = int_param(device, "unit").unwrap_or(*UNIT_RANGE.start());
    let mut caption = format!("{} · Unit {unit}", device.id);
    if shows_net_fields(&master.kind) {
        let ip = text_param(device, "ip", "");
        let port = int_param(device, "port").unwrap_or(0);
        if !ip.is_empty() {
            caption.push_str(&format!(" · {ip}"));
        }
        if port != 0 {
            caption.push_str(&format!(":{port}"));
        }
    }
    let resp = ui
        .selectable_label(false, caption)
        .on_hover_text(crate::i18n::tr(
            "Двойной щелчок — свойства (Unit ID, адрес, порт)",
        ));
    if resp.double_clicked() {
        actions.push(RadioAction::Settings(device.id.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button(crate::i18n::tr("Свойства...")).clicked() {
            actions.push(RadioAction::Settings(device.id.clone()));
            ui.close();
        }
        ui.separator();
        if ui
            .button(crate::i18n::tr("Удалить из проекта"))
            .on_hover_text(crate::i18n::tr("Вместе с его тегами"))
            .clicked()
        {
            actions.push(RadioAction::Remove(device.id.clone()));
            ui.close();
        }
    });
    ui.indent(("modbus_device_tags", device.id.as_str()), |ui| {
        tag_group(app, ui, &device.id, conflicts, actions);
    });
}

/// Under a slave-role block: its tags group.
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

/// The «Теги» group of one owner (a device or a slave-role block): «+» and the tag rows.
fn tag_group(
    app: &EditorApp,
    ui: &mut Ui,
    owner: &str,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let logic = &doc.project.logic;
    let add = crate::panels::add_group(
        ui,
        ("modbus_tags", owner),
        crate::i18n::tr("Теги"),
        crate::i18n::tr(
            "Добавить тег: регистр или бит. Адрес — следующий свободный в таблице Holding",
        ),
        true,
        |ui| {
            for slot in logic.modbus_tags_of(owner) {
                // Copies of a tag (blocks with `tag` set) are not listed.
                if let Some(b) = logic.block(&slot.id).filter(|b| b.tag.is_none()) {
                    tag_row(app, ui, b, conflicts, actions);
                }
            }
        },
    );
    if add {
        actions.push(RadioAction::AddTag(owner.to_owned()));
    }
}

/// One tag: a flat row like a variable (name, table, type, address, size, direction, mark),
/// everything is edited in its settings (double click). The conflicts of the tag follow in red.
fn tag_row(
    app: &EditorApp,
    ui: &mut Ui,
    b: &Block,
    conflicts: &[ModbusConflict],
    actions: &mut Vec<RadioAction>,
) {
    let Some(doc) = &app.doc else { return };
    let id = b.id.as_str();
    let direction = doc.project.logic.modbus_tag_direction(id);
    let table = text_param(b, "table", NEW_TAG_TABLE);
    let kind = text_param(b, "kind", "int");
    let reg = int_param(b, "reg").unwrap_or(0);
    let caption = format!(
        "{id} · {} {kind} {reg} · {} · {}",
        table_caption(table),
        size_text(table, kind),
        crate::i18n::tr(direction_caption(direction)),
    );
    let color = match kind {
        "bool" => Some(crate::canvas::signal_color(
            &app.theme.palette,
            crab_core::SignalType::Bool,
        )),
        "float" => Some(crate::canvas::signal_color(
            &app.theme.palette,
            crab_core::SignalType::Float,
        )),
        _ => Some(crate::canvas::signal_color(
            &app.theme.palette,
            crab_core::SignalType::Int,
        )),
    };
    crate::radiopanel::row_captioned(app, ui, b, caption, color, actions);
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
}
