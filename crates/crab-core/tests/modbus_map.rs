//! Modbus register map of tags: owners, sizes, next free address, directions from links,
//! overlaps, value types, and the project check of tags, devices and masters.
use std::path::PathBuf;

use crab_core::modbus_map::{ipv4_octets, slot_size, ModbusConflict, TagDirection};
use crab_core::{check_project, DiagKind, Library, Logic, Project, Severity};

fn library() -> Library {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (lib, errors) = Library::load(root.join("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

fn logic(src: &str) -> Logic {
    toml::from_str(src).unwrap()
}

/// A `modbus_tag` block of the owner `device` (an id of a device or a slave block).
fn tag(id: &str, device: &str, table: &str, kind: &str, reg: i64) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"modbus_tag\"\ndevice = \"{device}\"\ntable = \"{table}\"\nkind = \"{kind}\"\nreg = {reg}\n\n"
    )
}

/// A `modbus_device` block: a slave of the master `master` with the Unit ID `unit`.
fn device(id: &str, master: &str, unit: i64) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"modbus_device\"\nmaster = \"{master}\"\nunit = {unit}\n\n"
    )
}

/// A `modbus_device` block with its own address: `ip` (empty = the master's) and `port` (0 = the
/// master's).
fn device_at(id: &str, master: &str, unit: i64, ip: &str, port: i64) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"modbus_device\"\nmaster = \"{master}\"\nunit = {unit}\nip = \"{ip}\"\nport = {port}\n\n"
    )
}

/// A master block of the given type, with an optional `slaves` limit.
fn master(id: &str, kind: &str, slaves: Option<i64>) -> String {
    let limit = slaves.map_or(String::new(), |n| format!("slaves = {n}\n"));
    format!("[[block]]\nid = \"{id}\"\ntype = \"{kind}\"\n{limit}\n")
}

/// A slave-role block of this board.
fn slave(id: &str) -> String {
    format!("[[block]]\nid = \"{id}\"\ntype = \"modbus_rtu_slave\"\n\n")
}

/// A connection `from` → `to`, both written as `block.port`.
fn link(from: &str, to: &str) -> String {
    format!("[[link]]\nfrom = \"{from}\"\nto = \"{to}\"\n\n")
}

fn conflicts_of(l: &Logic, pick: fn(&ModbusConflict) -> bool) -> Vec<ModbusConflict> {
    l.modbus_conflicts().into_iter().filter(pick).collect()
}

fn is_overlap(c: &ModbusConflict) -> bool {
    matches!(c, ModbusConflict::Overlap { .. })
}

fn is_bad_kind(c: &ModbusConflict) -> bool {
    matches!(c, ModbusConflict::BadKind { .. })
}

/// Runs the project check on `logic.toml` and returns the Modbus error messages.
fn modbus_messages(name: &str, src: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("crab-modbus-map-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), src).unwrap();
    let project = Project::load(&dir).unwrap();
    let report = check_project(&project, &library());
    std::fs::remove_dir_all(&dir).ok();

    report
        .diagnostics
        .iter()
        .filter(|d| {
            d.severity == Severity::Error
                && d.kind == DiagKind::ParamValue
                && d.message.starts_with("Modbus:")
        })
        .map(|d| d.message.clone())
        .collect()
}

fn has(messages: &[String], part: &str) -> bool {
    messages.iter().any(|m| m.contains(part))
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
fn slots_follow_block_order_and_owner() {
    let l = logic(&format!(
        "{}{}{}",
        tag("cmd", "d1", "holding", "int", 0),
        slave("s"),
        tag("level", "s", "input", "float", 4)
    ));
    let slots = l.modbus_slots();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].id, "cmd");
    assert_eq!(slots[0].owner, "d1");
    assert_eq!(slots[1].id, "level");
    assert_eq!(slots[1].owner, "s");
    assert_eq!(slots[1].size, 2);
    assert_eq!(slots[1].address, 4);
}

#[test]
fn slots_use_defaults_for_missing_params() {
    let l = logic("[[block]]\nid = \"bare\"\ntype = \"modbus_tag\"\n\n");
    let slots = l.modbus_slots();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].owner, "");
    assert_eq!(slots[0].table, "holding");
    assert_eq!(slots[0].kind, "int");
    assert_eq!(slots[0].address, 0);
    assert_eq!(slots[0].size, 1);
}

#[test]
fn slot_address_is_clamped_to_the_register_range() {
    let l = logic(&format!(
        "{}{}",
        tag("big", "d", "holding", "int", 70000),
        tag("neg", "d", "holding", "int", -5)
    ));
    let slots = l.modbus_slots();
    assert_eq!(slots[0].address, u16::MAX);
    assert_eq!(slots[1].address, 0);
}

#[test]
fn devices_of_a_master_in_block_order() {
    let l = logic(&format!(
        "{}{}{}{}",
        device("a", "m1", 1),
        device("b", "m2", 1),
        device("c", "m1", 2),
        master("m1", "modbus_rtu_master", None)
    ));
    let ids: Vec<&str> = l
        .modbus_devices("m1")
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(ids, ["a", "c"]);
    assert_eq!(l.modbus_devices("m2").len(), 1);
    assert!(l.modbus_devices("nobody").is_empty());
}

#[test]
fn tags_of_one_owner() {
    let l = logic(&format!(
        "{}{}{}",
        tag("a", "d1", "holding", "int", 0),
        tag("b", "d2", "holding", "int", 0),
        tag("c", "d1", "coil", "bool", 3)
    ));
    let ids: Vec<String> = l.modbus_tags_of("d1").into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["a", "c"]);
    assert!(l.modbus_tags_of("none").is_empty());
}

#[test]
fn float_at_zero_then_next_free_is_two() {
    let l = logic(&tag("f", "d", "holding", "float", 0));
    assert_eq!(l.modbus_next_free("d", "holding"), 2);
    assert_eq!(l.modbus_next_free("d", "input"), 0);
}

#[test]
fn next_free_is_per_owner() {
    let l = logic(&format!(
        "{}{}",
        tag("a", "d1", "holding", "int", 0),
        tag("b", "d2", "holding", "int", 9)
    ));
    assert_eq!(l.modbus_next_free("d1", "holding"), 1);
    assert_eq!(l.modbus_next_free("d2", "holding"), 10);
    assert_eq!(l.modbus_next_free("empty", "holding"), 0);
}

#[test]
fn coils_count_bits_for_next_free() {
    let l = logic(&format!(
        "{}{}",
        tag("a", "d", "coil", "bool", 0),
        tag("b", "d", "coil", "bool", 5)
    ));
    assert_eq!(l.modbus_next_free("d", "coil"), 6);
    // A float in a coil table is still one bit.
    let l = logic(&tag("f", "d", "coil", "float", 3));
    assert_eq!(l.modbus_next_free("d", "coil"), 4);
}

#[test]
fn next_free_saturates_at_the_end_of_the_range() {
    let l = logic(&tag("top", "d", "holding", "float", 65535));
    assert_eq!(l.modbus_next_free("d", "holding"), u16::MAX);
}

#[test]
fn table_sizes_are_per_owner() {
    let l = logic(&format!(
        "{}{}{}{}",
        tag("a", "d", "holding", "int", 0),
        tag("b", "d", "holding", "int", 10),
        tag("c", "d", "discrete", "bool", 2),
        tag("e", "other", "input", "float", 7)
    ));
    assert_eq!(l.modbus_table_sizes("d"), [11, 0, 0, 3]);
    assert_eq!(l.modbus_table_sizes("other"), [0, 9, 0, 0]);
    assert_eq!(l.modbus_table_sizes("none"), [0, 0, 0, 0]);
}

#[test]
fn direction_from_connections() {
    let src = format!(
        "{}{}{}{}{}{}",
        tag("rd", "d", "holding", "int", 0),
        tag("wr", "d", "holding", "int", 1),
        tag("both", "d", "holding", "int", 2),
        tag("none", "d", "holding", "int", 3),
        tag("okonly", "d", "holding", "int", 4),
        link("rd.out", "x.in")
    );
    let src = format!(
        "{src}{}{}{}{}",
        link("y.out_f", "wr.in_f"),
        link("both.out_b", "z.in"),
        link("w.out", "both.in_b"),
        link("okonly.ok", "q.in")
    );
    let l = logic(&src);
    assert_eq!(l.modbus_tag_direction("rd"), TagDirection::Read);
    assert_eq!(l.modbus_tag_direction("wr"), TagDirection::Write);
    assert_eq!(l.modbus_tag_direction("both"), TagDirection::Both);
    assert_eq!(l.modbus_tag_direction("none"), TagDirection::None);
    // The `ok` output does not count as a read.
    assert_eq!(l.modbus_tag_direction("okonly"), TagDirection::None);
    assert_eq!(l.modbus_tag_direction("unknown"), TagDirection::None);
}

#[test]
fn overlap_of_float_and_int_is_found() {
    let l = logic(&format!(
        "{}{}",
        tag("f", "d", "holding", "float", 0),
        tag("i", "d", "holding", "int", 1)
    ));
    assert_eq!(
        conflicts_of(&l, is_overlap),
        [ModbusConflict::Overlap {
            owner: "d".into(),
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
        tag("f", "d", "holding", "float", 0),
        tag("i", "d", "holding", "int", 2)
    ));
    assert!(conflicts_of(&l, is_overlap).is_empty());
}

#[test]
fn same_address_in_other_table_or_other_owner_is_no_overlap() {
    let l = logic(&format!(
        "{}{}{}",
        tag("h", "d", "holding", "int", 0),
        tag("i", "d", "input", "int", 0),
        tag("o", "other", "holding", "int", 0)
    ));
    assert!(conflicts_of(&l, is_overlap).is_empty());
}

#[test]
fn overlap_inside_one_owner_only() {
    let l = logic(&format!(
        "{}{}{}",
        tag("a", "d1", "holding", "int", 5),
        tag("b", "d2", "holding", "int", 5),
        tag("c", "d1", "holding", "int", 5)
    ));
    let found = conflicts_of(&l, is_overlap);
    assert_eq!(
        found,
        [ModbusConflict::Overlap {
            owner: "d1".into(),
            table: "holding".into(),
            first: "a".into(),
            second: "c".into(),
            address: 5,
        }]
    );
}

#[test]
fn bad_kind_in_coil_and_bool_in_holding() {
    let l = logic(&format!(
        "{}{}{}",
        tag("c", "d", "coil", "int", 0),
        tag("h", "d", "holding", "bool", 0),
        tag("ok", "d", "coil", "bool", 1)
    ));
    assert_eq!(
        conflicts_of(&l, is_bad_kind),
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
fn all_conflicts_over_owners() {
    let l = logic(&format!(
        "{}{}{}",
        tag("a", "d1", "holding", "int", 0),
        tag("b", "d1", "holding", "int", 0),
        tag("c", "d2", "coil", "int", 0)
    ));
    let all = l.modbus_conflicts();
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(is_overlap));
    assert!(all.iter().any(is_bad_kind));
}

#[test]
fn check_reports_overlap_and_bad_kind() {
    let src = format!(
        "{}{}{}{}{}",
        master("m", "modbus_rtu_master", None),
        device("d", "m", 1),
        tag("f", "d", "holding", "float", 0),
        tag("i", "d", "holding", "int", 1),
        tag("x", "d", "coil", "float", 0)
    );
    let src = format!(
        "{src}{}{}{}",
        link("src.out", "f.in"),
        link("src.out", "i.in"),
        link("src.out", "x.in")
    );
    let msgs = modbus_messages("overlap", &src);
    assert!(
        has(
            &msgs,
            "«f» и «i» пересекаются в таблице holding устройства «d» (адрес 1)"
        ),
        "{msgs:#?}"
    );
    assert!(
        has(&msgs, "тег «x»: тип float не подходит таблице coil"),
        "{msgs:#?}"
    );
}

#[test]
fn check_reports_tag_without_owner() {
    let src = format!(
        "{}{}",
        "[[block]]\nid = \"lost\"\ntype = \"modbus_tag\"\n\n",
        link("src.out", "lost.in")
    );
    let msgs = modbus_messages("no_owner", &src);
    assert!(
        has(&msgs, "тег «lost» не привязан к slave или устройству"),
        "{msgs:#?}"
    );
}

#[test]
fn check_reports_unknown_owner() {
    let src = format!(
        "{}{}",
        tag("t", "ghost", "holding", "int", 0),
        link("src.out", "t.in")
    );
    let msgs = modbus_messages("unknown_owner", &src);
    assert!(
        has(&msgs, "тег «t»: устройство «ghost» не найдено"),
        "{msgs:#?}"
    );
}

#[test]
fn check_accepts_tag_of_slave_role() {
    let src = format!(
        "{}{}{}",
        slave("s"),
        tag("t", "s", "holding", "int", 0),
        link("t.out", "dst.in")
    );
    let msgs = modbus_messages("slave_owner", &src);
    assert!(!has(&msgs, "не найдено"), "{msgs:#?}");
    assert!(!has(&msgs, "не привязан"), "{msgs:#?}");
    assert!(!has(&msgs, "не подключён"), "{msgs:#?}");
}

#[test]
fn check_reports_tag_connected_both_ways() {
    let src = format!(
        "{}{}{}{}",
        slave("s"),
        tag("t", "s", "holding", "int", 0),
        link("t.out", "dst.in"),
        link("src.out", "t.in")
    );
    let msgs = modbus_messages("both", &src);
    assert!(
        has(
            &msgs,
            "тег «t» читается и записывается: подключите только вход или только выход"
        ),
        "{msgs:#?}"
    );
}

#[test]
fn check_reports_tag_not_connected() {
    let src = format!("{}{}", slave("s"), tag("t", "s", "holding", "int", 0));
    let msgs = modbus_messages("none", &src);
    assert!(has(&msgs, "тег «t» не подключён к схеме"), "{msgs:#?}");
}

#[test]
fn check_reports_too_many_devices() {
    let src = format!(
        "{}{}{}{}",
        master("m", "modbus_rtu_master", Some(1)),
        device("a", "m", 1),
        device("b", "m", 2),
        ""
    );
    let msgs = modbus_messages("slaves_limit", &src);
    assert!(
        has(&msgs, "у master «m» 2 slave, а предел «slaves» — 1"),
        "{msgs:#?}"
    );
}

#[test]
fn check_uses_four_as_default_slave_limit() {
    let mut src = master("m", "modbus_tcp_master", None);
    for n in 1..=5 {
        src.push_str(&device(&format!("d{n}"), "m", n));
    }
    let msgs = modbus_messages("default_limit", &src);
    assert!(
        has(&msgs, "у master «m» 5 slave, а предел «slaves» — 4"),
        "{msgs:#?}"
    );
}

#[test]
fn check_accepts_devices_within_the_limit() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_rtu_master", Some(2)),
        device("a", "m", 1),
        device("b", "m", 2)
    );
    let msgs = modbus_messages("within_limit", &src);
    assert!(!has(&msgs, "предел"), "{msgs:#?}");
}

#[test]
fn check_reports_duplicate_unit_of_one_master() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_rtu_master", None),
        device("a", "m", 7),
        device("b", "m", 7)
    );
    let msgs = modbus_messages("dup_unit", &src);
    assert!(
        has(&msgs, "у master «m» два slave с Unit ID 7: «a» и «b»"),
        "{msgs:#?}"
    );
}

#[test]
fn check_allows_same_unit_under_different_masters() {
    let src = format!(
        "{}{}{}{}",
        master("m1", "modbus_rtu_master", None),
        master("m2", "modbus_tcp_master", None),
        device("a", "m1", 7),
        device("b", "m2", 7)
    );
    let msgs = modbus_messages("same_unit_two_masters", &src);
    assert!(!has(&msgs, "Unit ID"), "{msgs:#?}");
}

#[test]
fn check_reports_device_without_master() {
    let src = format!("{}{}", device("a", "nobody", 1), device("b", "s", 2));
    let msgs = modbus_messages("no_master", &src);
    assert!(
        has(&msgs, "slave «a»: master «nobody» не найден"),
        "{msgs:#?}"
    );
    assert!(has(&msgs, "slave «b»: master «s» не найден"), "{msgs:#?}");
}

#[test]
fn check_reports_device_whose_master_is_not_a_master() {
    let src = format!("{}{}", slave("s"), device("a", "s", 1));
    let msgs = modbus_messages("slave_as_master", &src);
    assert!(has(&msgs, "slave «a»: master «s» не найден"), "{msgs:#?}");
}

#[test]
fn check_allows_same_unit_at_another_ip() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_tcp_master", None),
        device_at("a", "m", 1, "192.168.0.10", 0),
        device_at("b", "m", 1, "192.168.0.11", 1502)
    );
    let msgs = modbus_messages("same_unit_other_ip", &src);
    assert!(!has(&msgs, "Unit ID"), "{msgs:#?}");
    assert!(!has(&msgs, "IP-адрес"), "{msgs:#?}");
}

#[test]
fn check_reports_same_unit_at_the_same_endpoint() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_tcp_master", None),
        device_at("a", "m", 1, "192.168.0.10", 0),
        device_at("b", "m", 1, "192.168.0.10", 0)
    );
    let msgs = modbus_messages("same_endpoint", &src);
    assert!(
        has(&msgs, "у master «m» два slave с Unit ID 1: «a» и «b»"),
        "{msgs:#?}"
    );
}

#[test]
fn check_takes_the_master_endpoint_for_empty_ip_and_port() {
    // Both devices are polled at 192.168.1.50:502 (the master's ip, the default port).
    let src = format!(
        "[[block]]\nid = \"m\"\ntype = \"modbus_tcp_master\"\nip = \"192.168.1.50\"\n\n{}{}{}",
        device_at("a", "m", 1, "", 0),
        device_at("b", "m", 1, "192.168.1.50", 502),
        device_at("c", "m", 1, "192.168.1.50", 1502)
    );
    let msgs = modbus_messages("master_endpoint", &src);
    assert!(
        has(&msgs, "у master «m» два slave с Unit ID 1: «a» и «b»"),
        "{msgs:#?}"
    );
    assert!(!has(&msgs, "«c»"), "{msgs:#?}");
}

#[test]
fn check_reports_device_ip_that_is_not_a_dotted_quad() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_tcp_master", None),
        device_at("a", "m", 1, "192.168.0", 0),
        device_at("b", "m", 2, "192.168.0.300", 0)
    );
    let msgs = modbus_messages("bad_ip", &src);
    assert!(
        has(&msgs, "slave «a»: адрес «192.168.0» — не IP-адрес"),
        "{msgs:#?}"
    );
    assert!(
        has(&msgs, "slave «b»: адрес «192.168.0.300» — не IP-адрес"),
        "{msgs:#?}"
    );
}

#[test]
fn check_reports_ip_or_port_under_rtu_master() {
    let src = format!(
        "{}{}{}",
        master("m", "modbus_rtu_master", None),
        device_at("a", "m", 1, "10.0.0.5", 0),
        device_at("b", "m", 2, "", 1502)
    );
    let msgs = modbus_messages("rtu_address", &src);
    assert!(
        has(
            &msgs,
            "slave «a»: адрес и порт slave нужны только для Modbus TCP master"
        ),
        "{msgs:#?}"
    );
    assert!(
        has(
            &msgs,
            "slave «b»: адрес и порт slave нужны только для Modbus TCP master"
        ),
        "{msgs:#?}"
    );
}

#[test]
fn ipv4_octets_accepts_only_four_octets() {
    assert_eq!(ipv4_octets("192.168.0.10"), Some([192, 168, 0, 10]));
    assert_eq!(ipv4_octets("0.0.0.0"), Some([0, 0, 0, 0]));
    assert_eq!(ipv4_octets("192.168.0"), None);
    assert_eq!(ipv4_octets("192.168.0.1.5"), None);
    assert_eq!(ipv4_octets("192.168.0.256"), None);
    assert_eq!(ipv4_octets("192..0.1"), None);
    assert_eq!(ipv4_octets("192.168.0.-1"), None);
    assert_eq!(ipv4_octets(""), None);
}
