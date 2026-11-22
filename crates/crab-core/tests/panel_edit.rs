//! Reconnect and rebind of the electrical panel (`specs/panel-editor`, task 1).

use std::path::PathBuf;

use crab_core::{check_panel, rebind_device, Library, Panel, Project};

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
