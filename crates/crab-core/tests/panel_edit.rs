//! Reconnect and rebind of the electrical panel (`specs/panel-editor`, task 1).

use std::path::PathBuf;

use crab_core::{
    check_panel, rebind_device, rewire, Block, Library, Panel, ParamValue, Project, WireEnd,
};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn library() -> Library {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

/// The pump example with a panel built from `src`.
fn pump_with(src: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(src).unwrap();
    project
}

fn panel(src: &str) -> Panel {
    toml::from_str(src).unwrap()
}

fn load_of<'a>(panel: &'a Panel, id: &str) -> &'a str {
    &panel.device(id).unwrap().load
}

fn bind_of<'a>(panel: &'a Panel, id: &str) -> &'a str {
    &panel.device(id).unwrap().bind
}

fn errors(project: &Project) -> Vec<String> {
    check_panel(project, &library())
        .errors()
        .map(|d| d.message.clone())
        .collect()
}

/// Q1 feeds K1, K1 feeds M1; Q2 is free; X1 is the feed; B1, BT1, L1, T1 are not in the power
/// graph (sensor, button, load, terminal).
const CHAIN: &str = r#"
[[device]]
id = "X1"
kind = "feed"

[[device]]
id = "Q1"
kind = "breaker"
load = "K1"

[[device]]
id = "Q2"
kind = "breaker"

[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"

[[device]]
id = "M1"
kind = "motor"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"

[[device]]
id = "BT1"
kind = "button"

[[device]]
id = "L1"
kind = "load"

[[device]]
id = "T1"
kind = "terminal"
"#;

#[test]
fn reconnect_moves_the_feeder_and_releases_the_old_one() {
    let mut p = panel(CHAIN);
    p.reconnect_power("K1", Some("Q2")).unwrap();
    assert_eq!(load_of(&p, "Q1"), "");
    assert_eq!(load_of(&p, "Q2"), "K1");
    assert_eq!(load_of(&p, "K1"), "M1");
}

#[test]
fn reconnect_replaces_the_load_of_the_new_parent() {
    let mut p = panel(CHAIN);
    p.devices.iter_mut().find(|d| d.id == "Q2").unwrap().load = "L1".into();
    p.reconnect_power("K1", Some("Q2")).unwrap();
    assert_eq!(load_of(&p, "Q2"), "K1");
    assert_eq!(load_of(&p, "Q1"), "");
    assert_eq!(load_of(&p, "K1"), "M1");
}

#[test]
fn reconnect_to_none_leaves_the_device_without_a_feeder() {
    let mut p = panel(CHAIN);
    p.reconnect_power("K1", None).unwrap();
    assert!(p.devices.iter().all(|d| d.load != "K1"));
    assert_eq!(load_of(&p, "K1"), "M1");
}

#[test]
fn reconnect_from_a_feed_leaves_the_device_without_a_feeder() {
    let mut p = panel(CHAIN);
    p.reconnect_power("K1", Some("X1")).unwrap();
    assert!(p.devices.iter().all(|d| d.load != "K1"));
    assert_eq!(load_of(&p, "Q1"), "");
    assert_eq!(load_of(&p, "K1"), "M1");
}

#[test]
fn reconnect_refuses_a_loop_and_changes_nothing() {
    // Q1 feeds K1, which feeds K2.
    let base = panel(
        r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"

[[device]]
id = "K1"
kind = "contactor"
load = "K2"

[[device]]
id = "K2"
kind = "relay"
"#,
    );
    for (device, parent) in [("Q1", "K2"), ("Q1", "K1"), ("K1", "K2")] {
        let mut p = base.clone();
        let err = p.reconnect_power(device, Some(parent)).unwrap_err();
        assert!(err.contains("замкнулась бы"), "{device} {parent}: {err}");
        assert_eq!(p, base, "{device} {parent}");
    }
}

#[test]
fn reconnect_to_the_same_parent_changes_nothing() {
    let mut p = panel(CHAIN);
    let before = p.clone();
    p.reconnect_power("K1", Some("Q1")).unwrap();
    assert_eq!(p, before);
}

#[test]
fn reconnect_refusals_change_nothing() {
    let cases = [
        ("nope", Some("Q2"), "нет устройства «nope»"),
        ("K1", Some("nope"), "нет устройства «nope»"),
        ("K1", Some("K1"), "не может питать само себя"),
        // Kinds that are never fed by a device.
        ("B1", Some("Q2"), "датчик не питается"),
        ("BT1", Some("Q2"), "кнопка не питается"),
        // Kinds that cannot feed.
        ("Q2", Some("B1"), "датчик не может питать"),
        ("Q2", Some("BT1"), "кнопка не может питать"),
        ("Q2", Some("M1"), "двигатель не может питать"),
        ("Q2", Some("L1"), "нагрузка не может питать"),
    ];
    for (device, parent, message) in cases {
        let mut p = panel(CHAIN);
        let before = p.clone();
        let err = p.reconnect_power(device, parent).unwrap_err();
        assert!(err.contains(message), "{device} {parent:?}: {err}");
        assert_eq!(p, before, "{device} {parent:?}");
    }
}

#[test]
fn bind_and_clear_a_device() {
    let mut project = pump_with(CHAIN);
    rebind_device(&mut project, &library(), "Q2", None).unwrap();
    rebind_device(&mut project, &library(), "K1", None).unwrap();
    assert_eq!(bind_of(&project.panel, "K1"), "");
    rebind_device(&mut project, &library(), "K1", Some("pump")).unwrap();
    assert_eq!(bind_of(&project.panel, "K1"), "pump");
    // The device that already has the tag may keep it.
    rebind_device(&mut project, &library(), "K1", Some("pump")).unwrap();
    assert_eq!(bind_of(&project.panel, "K1"), "pump");
}

#[test]
fn sensor_takes_a_port_of_its_input() {
    let mut project = pump_with(CHAIN);
    rebind_device(&mut project, &library(), "B1", Some("sensor")).unwrap();
    assert_eq!(bind_of(&project.panel, "B1"), "sensor");
    rebind_device(&mut project, &library(), "B1", Some("sensor.out")).unwrap();
    assert_eq!(bind_of(&project.panel, "B1"), "sensor.out");
}

#[test]
fn rebind_refusals_change_nothing() {
    let mut project = pump_with(&format!(
        "{CHAIN}\n[[device]]\nid = \"K2\"\nkind = \"relay\"\n"
    ));
    let cases: [(&str, Option<&str>, &str); 8] = [
        ("Z9", Some("pump"), "нет устройства «Z9»"),
        ("K1", Some("nowhere"), "нет входа или выхода «nowhere»"),
        // Devices the schema does not drive.
        ("Q2", Some("pump"), "автомат не привязывается"),
        ("M1", Some("pump"), "двигатель не привязывается"),
        // Direction: a coil on an input, a sensor on an output.
        ("K2", Some("sensor"), "реле привязывается к выходу платы"),
        ("B1", Some("pump"), "датчик привязывается к входу платы"),
        // A coil or a button needs a digital signal: `sensor` is analog.
        ("BT1", Some("sensor"), "дискретному сигналу"),
        // One device per tag: K1 already has `pump`.
        ("K2", Some("pump"), "уже привязан к -K1"),
    ];
    for (device, tag, message) in cases {
        let before = project.panel.clone();
        let err = rebind_device(&mut project, &library(), device, tag).unwrap_err();
        assert!(err.contains(message), "{device} {tag:?}: {err}");
        assert_eq!(project.panel, before, "{device} {tag:?}");
    }
}

/// A panel with no mistakes: the breaker Q1 feeds the contactor K1 of the pump, which feeds M1,
/// and the sensor B1 is on the input of the pump example.
const SENSIBLE: &str = r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"

[[device]]
id = "Q2"
kind = "breaker"

[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"

[[device]]
id = "M1"
kind = "motor"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

#[test]
fn a_bound_device_still_passes_the_panel_check() {
    let mut project = pump_with(SENSIBLE);
    assert!(errors(&project).is_empty(), "{:?}", errors(&project));
    rebind_device(&mut project, &library(), "K1", None).unwrap();
    rebind_device(&mut project, &library(), "K1", Some("pump")).unwrap();
    assert!(errors(&project).is_empty(), "{:?}", errors(&project));
}

#[test]
fn reconnect_keeps_the_panel_checked() {
    let mut project = pump_with(SENSIBLE);
    project.panel.reconnect_power("K1", Some("Q2")).unwrap();
    assert_eq!(load_of(&project.panel, "Q2"), "K1");
    assert_eq!(load_of(&project.panel, "Q1"), "");
    assert!(errors(&project).is_empty(), "{:?}", errors(&project));
    project.panel.reconnect_power("K1", None).unwrap();
    assert!(errors(&project).is_empty(), "{:?}", errors(&project));
}

/// `CHAIN` with a relay K2 on a branch of Q1 (`from`), a free breaker Q3, and a hand-drawn wire
/// Q1:2 to K1:1 with its number.
fn wired(extra: &str) -> Project {
    let src = format!(
        r#"{CHAIN}
[[device]]
id = "K2"
kind = "relay"
from = "Q1"

[[device]]
id = "Q3"
kind = "breaker"

[[link]]
from = "Q1:2"
to = "K1:1"

[wire_numbers]
"Q1:2>K1:1" = 3
{extra}
"#
    );
    pump_with(&src)
}

/// A second digital output of the project, on the board terminal `IO27`.
fn with_second_output(project: &mut Project) {
    project.logic.blocks.push(Block {
        id: "pump2".into(),
        kind: "digital_out".into(),
        params: [("pin".to_owned(), ParamValue::Int(27))]
            .into_iter()
            .collect(),
        ..Default::default()
    });
}

#[test]
fn rewire_moves_the_end_of_a_hand_wire() {
    let mut project = wired("");
    rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::To, "M1:L1").unwrap();
    assert_eq!(project.panel.links.len(), 1);
    assert_eq!(project.panel.links[0].from, "Q1:2");
    assert_eq!(project.panel.links[0].to, "M1:L1");
    assert!(!project.panel.wire_numbers.contains_key("Q1:2>K1:1"));

    rewire(
        &mut project,
        &library(),
        "Q1:2>M1:L1",
        WireEnd::From,
        "Q3:2",
    )
    .unwrap();
    assert_eq!(project.panel.links[0].from, "Q3:2");
}

#[test]
fn rewire_refuses_a_hand_wire_that_already_exists() {
    let mut project = wired("");
    project
        .panel
        .links
        .push(toml::from_str("from = \"Q1:2\"\nto = \"M1:L1\"").unwrap());
    let before = project.clone();
    let err = rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::To, "M1:L1").unwrap_err();
    assert!(err.contains("такой провод уже есть"), "{err}");
    assert_eq!(project, before);
}

#[test]
fn rewire_moves_the_downstream_end_of_a_load_link() {
    // Q1 feeds K1 by `load`; the wire goes to the new breaker Q2 instead.
    let mut project = pump_with(CHAIN);
    rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::To, "Q2:1").unwrap();
    assert_eq!(load_of(&project.panel, "Q1"), "Q2");
    assert_eq!(load_of(&project.panel, "Q2"), "");
    assert!(project.panel.devices.iter().all(|d| d.load != "K1"));
    assert_eq!(project.panel.device("K1").unwrap().from, "");
    assert_eq!(load_of(&project.panel, "K1"), "M1");
}

#[test]
fn rewire_moves_the_upstream_end_of_a_load_link() {
    // K1 is now fed by Q2, and Q1 feeds nothing.
    let mut project = pump_with(CHAIN);
    rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::From, "Q2:2").unwrap();
    assert_eq!(load_of(&project.panel, "Q2"), "K1");
    assert_eq!(load_of(&project.panel, "Q1"), "");
    assert_eq!(load_of(&project.panel, "K1"), "M1");
}

#[test]
fn rewire_moves_the_end_of_a_branch() {
    // K2 is a branch of Q1 (`from`); its end moves to Q2.
    let mut project = wired("");
    rewire(&mut project, &library(), "Q1:4>K2:1", WireEnd::To, "Q2:1").unwrap();
    assert_eq!(project.panel.device("Q2").unwrap().from, "Q1");
    assert_eq!(project.panel.device("K2").unwrap().from, "");
    assert_eq!(load_of(&project.panel, "Q1"), "K1");

    // And the feed of the branch is changed from the other end.
    rewire(&mut project, &library(), "Q1:4>Q2:1", WireEnd::From, "Q3:4").unwrap();
    assert_eq!(project.panel.device("Q2").unwrap().from, "Q3");
}

#[test]
fn rewire_to_another_terminal_of_the_same_device_keeps_the_feed() {
    // The end of the branch Q1 → K2 goes to another terminal of K2: K2 is still fed by Q1.
    let mut project = wired("");
    rewire(&mut project, &library(), "Q1:4>K2:1", WireEnd::To, "K2:3").unwrap();
    assert_eq!(project.panel.device("K2").unwrap().from, "Q1");
    let mut project = pump_with(CHAIN);
    rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::To, "K1:3").unwrap();
    assert_eq!(load_of(&project.panel, "Q1"), "K1");
}

#[test]
fn rewire_refuses_an_earth_wire_and_a_loop() {
    let mut project = pump_with(CHAIN);
    let before = project.clone();
    // The earth wire between K1 and M1 is not the power chain.
    assert!(rewire(&mut project, &library(), "K1:pe>M1:pe", WireEnd::To, "Q2:1").is_err());
    // Q1 fed by itself, and Q1 fed by K1 that it feeds.
    assert!(rewire(&mut project, &library(), "Q1:2>K1:1", WireEnd::To, "Q1:1").is_err());
    assert!(rewire(&mut project, &library(), "K1:2>M1:1", WireEnd::To, "Q1:1").is_err());
    assert_eq!(project.panel, before.panel);
}

#[test]
fn rewire_moves_the_board_end_to_another_tag() {
    let mut project = pump_with(CHAIN);
    with_second_output(&mut project);
    rewire(
        &mut project,
        &library(),
        "A1:IO26>K1:A1",
        WireEnd::From,
        "A1:IO27",
    )
    .unwrap();
    assert_eq!(bind_of(&project.panel, "K1"), "pump2");
}

#[test]
fn rewire_refuses_a_board_terminal_without_a_tag() {
    let mut project = pump_with(CHAIN);
    let before = project.clone();
    let err = rewire(
        &mut project,
        &library(),
        "A1:IO26>K1:A1",
        WireEnd::From,
        "A1:IO5",
    )
    .unwrap_err();
    assert!(
        err.contains("к выводу IO5 не привязан тег проекта"),
        "{err}"
    );
    assert_eq!(project, before);
}

#[test]
fn rewire_moves_the_device_end_of_a_board_wire() {
    // K1 gives up `pump`, and the relay K2 takes it.
    let mut project = wired("");
    rewire(
        &mut project,
        &library(),
        "A1:IO26>K1:A1",
        WireEnd::To,
        "K2:A1",
    )
    .unwrap();
    assert_eq!(bind_of(&project.panel, "K1"), "");
    assert_eq!(bind_of(&project.panel, "K2"), "pump");
}

#[test]
fn rewire_refuses_a_wire_the_generator_draws() {
    // Q2 and M1 are not linked in the panel: the wire is made by the generator.
    let mut project = pump_with(CHAIN);
    let before = project.clone();
    let err = rewire(&mut project, &library(), "Q2:2>M1:L1", WireEnd::To, "L1:1").unwrap_err();
    assert!(
        err.contains("-Q2 — -M1: этот провод строит генератор"),
        "{err}"
    );
    assert_eq!(project, before);
}

#[test]
fn rewire_refusals_change_nothing() {
    let cases: [(&str, WireEnd, &str, &str); 7] = [
        ("Q1:2", WireEnd::To, "Q1:2", "неверный провод «Q1:2»"),
        (
            "Q1:2>K1:1",
            WireEnd::To,
            "Q1:2",
            "не может начинаться и кончаться на одном выводе",
        ),
        (
            "Q1:2>K1:1",
            WireEnd::To,
            "Z9:1",
            "-Z9: нет такого устройства в панели",
        ),
        ("Q1:2>K1:1", WireEnd::To, "K1", "неверный вывод «K1»"),
        // The board end of a wire moves only to another terminal of the board.
        (
            "A1:IO26>K1:A1",
            WireEnd::From,
            "K1:2",
            "к выводу платы провод можно подключить только",
        ),
        (
            "A1:IO26>K1:A1",
            WireEnd::From,
            "A1:IO5",
            "к выводу IO5 не привязан тег проекта",
        ),
        // A motor takes no tag of the board.
        (
            "A1:IO26>K1:A1",
            WireEnd::To,
            "M1:L1",
            "двигатель не привязывается",
        ),
    ];
    for (key, end, target, message) in cases {
        let mut project = wired("");
        let before = project.clone();
        let err = rewire(&mut project, &library(), key, end, target).unwrap_err();
        assert!(err.contains(message), "{key} {end:?} {target}: {err}");
        assert_eq!(project, before, "{key} {end:?} {target}");
    }
}

#[test]
fn a_terminal_is_connected_to_a_line_or_a_contact_and_back_to_the_generator() {
    use crab_core::connect_terminal;
    let mut panel = pump_with(CHAIN).panel;
    connect_terminal(&mut panel, "K1:A1", Some("bus:L1")).unwrap();
    assert_eq!(panel.links.len(), 1);
    // A new target replaces the old hand wire on the same terminal.
    connect_terminal(&mut panel, "K1:A1", Some("Q2:1")).unwrap();
    assert_eq!(
        panel.links.iter().map(|l| l.key()).collect::<Vec<_>>(),
        ["K1:A1>Q2:1"]
    );
    // None gives the terminal back to the generator.
    connect_terminal(&mut panel, "K1:A1", None).unwrap();
    assert!(panel.links.is_empty());
    assert!(connect_terminal(&mut panel, "K1:A1", Some("K1:A1")).is_err());
    assert!(connect_terminal(&mut panel, "K1", Some("bus:L1")).is_err());
}
