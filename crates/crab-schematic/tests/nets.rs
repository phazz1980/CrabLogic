//! Wires and numbers of the pump panel (`specs/electrical-schematics`, task 3).

use std::collections::HashSet;
use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_nets, Nets, BOARD, EXTERNAL};

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

const PUMP_PANEL: &str = r#"
supply = "ac230"

[[device]]
id = "X1"
kind = "feed"

[[device]]
id = "Q1"
kind = "breaker"
load = "K1"

[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"

[[device]]
id = "M1"
kind = "motor"

[[device]]
id = "Q2"
kind = "breaker"
load = "G1"

[[device]]
id = "G1"
kind = "psu"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

fn pump(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
}

fn keys(nets: &Nets) -> Vec<String> {
    nets.wires.iter().map(|w| w.key()).collect()
}

#[test]
fn power_control_and_signal_wires_follow_the_devices() {
    let nets = build_nets(&pump(PUMP_PANEL), &library());
    let keys = keys(&nets);
    for expected in [
        // mains: feed → breaker → contactor → motor, poles 1/2 for L and 3/4 for N
        "X1:L>Q1:1",
        "X1:N>Q1:3",
        "Q1:2>K1:1",
        "Q1:4>K1:3",
        "K1:2>M1:L",
        "K1:4>M1:N",
        // the power supply behind its own breaker
        "X1:L>Q2:1",
        "Q2:2>G1:L",
        "Q2:4>G1:N",
        // earth
        "X1:PE>M1:PE",
        "X1:PE>G1:PE",
        // coil and sensor
        "A1:IO26>K1:A1",
        "K1:A2>G1:0V",
        "G1:+24V>B1:+",
        "G1:0V>B1:-",
        "B1:S>A1:IO34",
    ] {
        assert!(
            keys.iter().any(|k| k == expected),
            "нет {expected}: {keys:#?}"
        );
    }
    assert_eq!(nets.wires.len(), 17, "{keys:#?}");

    let rows: Vec<_> = nets
        .board_terminals
        .iter()
        .map(|t| (t.terminal.as_str(), t.tag.as_str(), t.device.as_str()))
        .collect();
    assert_eq!(rows, [("IO26", "pump", "K1"), ("IO34", "sensor", "B1")]);
    for t in &nets.board_terminals {
        let wire = nets.wires.iter().find(|w| w.number == t.wire).unwrap();
        assert!(wire.from.device == BOARD || wire.to.device == BOARD);
    }
}

#[test]
fn numbers_are_unique_and_the_same_every_time() {
    let project = pump(PUMP_PANEL);
    let lib = library();
    let first = build_nets(&project, &lib);
    assert_eq!(first, build_nets(&project, &lib));
    let numbers: HashSet<u32> = first.wires.iter().map(|w| w.number).collect();
    assert_eq!(numbers.len(), first.wires.len());
    assert!(first.wires.iter().all(|w| w.number >= 1));
    assert_eq!(first.wires[0].number, 1);
}

#[test]
fn wires_keep_their_numbers_when_the_panel_changes() {
    let lib = library();
    let mut project = pump(PUMP_PANEL);
    let before = build_nets(&project, &lib);
    before.store(&mut project.panel);

    // A device is added in front of the others: the old wires keep their numbers.
    project.panel.devices.insert(
        0,
        toml::from_str("id = \"Q3\"\nkind = \"breaker\"").unwrap(),
    );
    let after = build_nets(&project, &lib);
    for wire in &before.wires {
        let same = after.wires.iter().find(|w| w.key() == wire.key()).unwrap();
        assert_eq!(same.number, wire.number, "{}", wire.key());
    }
    let largest = before.wires.iter().map(|w| w.number).max().unwrap();
    let added: Vec<_> = after
        .wires
        .iter()
        .filter(|w| !before.wires.iter().any(|b| b.key() == w.key()))
        .collect();
    assert_eq!(added.len(), 2, "{added:#?}");
    assert!(added.iter().all(|w| w.number > largest));
    after.store(&mut project.panel);

    // A device is taken away: its wires and numbers go, the others keep theirs.
    project.panel.devices.retain(|d| d.id != "B1");
    let fewer = build_nets(&project, &lib);
    assert!(fewer
        .wires
        .iter()
        .all(|w| w.from.device != "B1" && w.to.device != "B1"));
    for wire in &fewer.wires {
        let old = after.wires.iter().find(|w| w.key() == wire.key()).unwrap();
        assert_eq!(old.number, wire.number, "{}", wire.key());
    }
    fewer.store(&mut project.panel);
    assert_eq!(project.panel.wire_numbers.len(), fewer.wires.len());
}

#[test]
fn control_source_is_the_feed_on_24_volts_and_external_otherwise() {
    let lib = library();
    let dc = r#"
supply = "dc24"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "K1"
kind = "relay"
bind = "pump"
"#;
    let keys_dc = keys(&build_nets(&pump(dc), &lib));
    assert!(keys_dc.iter().any(|k| k == "K1:A2>X1:0V"), "{keys_dc:#?}");

    let ac_without_psu = r#"
[[device]]
id = "K1"
kind = "relay"
bind = "pump"
"#;
    let keys_ac = keys(&build_nets(&pump(ac_without_psu), &lib));
    assert!(
        keys_ac.iter().any(|k| *k == format!("K1:A2>{EXTERNAL}:0V")),
        "{keys_ac:#?}"
    );
}

#[test]
fn three_phase_poles_follow_the_conductors() {
    let panel = r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;
    let keys = keys(&build_nets(&pump(panel), &library()));
    for expected in [
        "X1:L1>Q1:1",
        "X1:L2>Q1:3",
        "X1:L3>Q1:5",
        "X1:N>Q1:7",
        "Q1:6>M1:L3",
        "Q1:8>M1:N",
    ] {
        assert!(
            keys.iter().any(|k| k == expected),
            "нет {expected}: {keys:#?}"
        );
    }
}

#[test]
fn broken_binding_is_skipped_not_fatal() {
    let panel = r#"
[[device]]
id = "K1"
kind = "contactor"
bind = "nowhere"
"#;
    let nets = build_nets(&pump(panel), &library());
    assert!(nets.board_terminals.is_empty());
    assert!(nets
        .wires
        .iter()
        .all(|w| w.from.device != BOARD && w.to.device != BOARD));
}
