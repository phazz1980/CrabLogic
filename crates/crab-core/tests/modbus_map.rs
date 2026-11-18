//! Modbus register map: sizes, next free address, overlaps, value types and the project check.
use std::path::PathBuf;

use crab_core::modbus_map::{slot_size, ModbusConflict};
use crab_core::{check_project, DiagKind, Library, Logic, Project, Report, Severity};

fn library() -> Library {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (lib, errors) = Library::load(root.join("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

fn logic(src: &str) -> Logic {
    toml::from_str(src).unwrap()
}

/// A "Modbus write" block (table defaults to input) or "Modbus read" block (holding).
fn tag(kind_of_block: &str, id: &str, table: &str, kind: &str, reg: i64) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"{kind_of_block}\"\ntable = \"{table}\"\nkind = \"{kind}\"\nreg = {reg}\n\n"
    )
}

fn write(id: &str, table: &str, kind: &str, reg: i64) -> String {
    tag("modbus_write", id, table, kind, reg)
}

fn read(id: &str, table: &str, kind: &str, reg: i64) -> String {
    tag("modbus_read", id, table, kind, reg)
}

fn overlaps(l: &Logic) -> Vec<ModbusConflict> {
    l.modbus_conflicts()
        .into_iter()
        .filter(|c| matches!(c, ModbusConflict::Overlap { .. }))
        .collect()
}

#[test]
fn sizes_of_int_float_and_bool() {
    assert_eq!(slot_size("holding", "int"), 1);
    assert_eq!(slot_size("holding", "float"), 2);
    assert_eq!(slot_size("input", "float"), 2);
    assert_eq!(slot_size("holding", "bool"), 1);
    assert_eq!(slot_size("coil", "float"), 1);
    assert_eq!(slot_size("discrete", "int"), 1);
}

#[test]
fn slots_follow_block_order_and_kind_of_block() {
    let l = logic(&format!(
        "{}{}",
        read("cmd", "holding", "int", 0),
        write("level", "input", "float", 4)
    ));
    let slots = l.modbus_slots();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].id, "cmd");
    assert!(!slots[0].from_schema);
    assert_eq!(slots[1].id, "level");
    assert!(slots[1].from_schema);
    assert_eq!(slots[1].size, 2);
    assert_eq!(slots[1].address, 4);
}

#[test]
fn float_at_zero_then_next_free_is_two() {
    let l = logic(&read("f", "holding", "float", 0));
    assert_eq!(l.modbus_next_free("holding", "int"), 2);
    assert_eq!(l.modbus_next_free("input", "int"), 0);
}

#[test]
fn coils_count_bits_for_next_free() {
    let l = logic(&format!(
        "{}{}",
        write("a", "coil", "bool", 0),
        write("b", "coil", "bool", 5)
    ));
    assert_eq!(l.modbus_next_free("coil", "bool"), 6);
    // A float in a coil table is still one bit.
    let l = logic(&write("f", "coil", "float", 3));
    assert_eq!(l.modbus_next_free("coil", "bool"), 4);
}

#[test]
fn overlap_of_float_and_int_is_found() {
    let l = logic(&format!(
        "{}{}",
        read("f", "holding", "float", 0),
        read("i", "holding", "int", 1)
    ));
    let found = overlaps(&l);
    assert_eq!(
        found,
        [ModbusConflict::Overlap {
            table: "holding".into(),
            first: "f".into(),
            second: "i".into(),
            address: 1,
        }]
    );
}

#[test]
fn adjacent_float_and_int_do_not_overlap() {
    let l = logic(&format!(
        "{}{}",
        read("f", "holding", "float", 0),
        read("i", "holding", "int", 2)
    ));
    assert!(overlaps(&l).is_empty());
}

#[test]
fn same_address_in_other_table_is_no_overlap() {
    let l = logic(&format!(
        "{}{}",
        read("h", "holding", "int", 0),
        write("i", "input", "int", 0)
    ));
    assert!(overlaps(&l).is_empty());
}

#[test]
fn table_sizes_count_up_to_the_farthest_cell() {
    let l = logic(&format!(
        "{}{}",
        read("a", "holding", "int", 0),
        read("b", "holding", "int", 10)
    ));
    assert_eq!(l.modbus_table_sizes(), [11, 0, 0, 0]);
    assert_eq!(l.modbus_next_free("holding", "int"), 11);
}

#[test]
fn next_free_saturates_at_the_end_of_the_range() {
    let l = logic(&read("top", "holding", "float", 65535));
    assert_eq!(l.modbus_next_free("holding", "int"), u16::MAX);
}

#[test]
fn bad_kind_in_coil_and_bool_in_holding() {
    let l = logic(&format!(
        "{}{}{}",
        write("c", "coil", "int", 0),
        read("h", "holding", "bool", 0),
        write("ok", "coil", "bool", 1)
    ));
    let bad: Vec<ModbusConflict> = l
        .modbus_conflicts()
        .into_iter()
        .filter(|c| matches!(c, ModbusConflict::BadKind { .. }))
        .collect();
    assert_eq!(
        bad,
        [
            ModbusConflict::BadKind {
                id: "c".into(),
                table: "coil".into(),
                kind: "int".into(),
            },
            ModbusConflict::BadKind {
                id: "h".into(),
                table: "holding".into(),
                kind: "bool".into(),
            },
        ]
    );
}

#[test]
fn project_check_reports_overlap_and_bad_kind() {
    let dir = std::env::temp_dir().join(format!("crab-modbus-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    let src = format!(
        "{}{}{}",
        read("f", "holding", "float", 0),
        read("i", "holding", "int", 1),
        write("x", "coil", "float", 0)
    );
    std::fs::write(dir.join("logic.toml"), src).unwrap();
    let project = Project::load(&dir).unwrap();
    let report: Report = check_project(&project, &library());
    std::fs::remove_dir_all(&dir).ok();

    let errors: Vec<&str> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error && d.kind == DiagKind::ParamValue)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("«f» и «i» пересекаются в таблице holding (адрес 1)")),
        "{errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("тег «x»: тип float не подходит таблице coil")),
        "{errors:#?}"
    );
}
