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
    // One number to a conductor: the wires of a net share it, two nets never do.
    let numbers: HashSet<u32> = first.wires.iter().map(|w| w.number).collect();
    let nets: HashSet<usize> = first.wires.iter().map(|w| w.net).collect();
    assert_eq!(numbers.len(), nets.len());
    for a in &first.wires {
        for b in &first.wires {
            assert_eq!(
                a.net == b.net,
                a.number == b.number,
                "{} {}",
                a.key(),
                b.key()
            );
        }
    }
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
    // The new breaker hangs on the buses of the feed: its wires take the numbers of those buses.
    for w in &added {
        let bus = before
            .wires
            .iter()
            .find(|b| b.from == w.from)
            .unwrap_or_else(|| panic!("{}", w.key()));
        assert_eq!(w.number, bus.number, "{}", w.key());
    }
    assert!(after.wires.iter().all(|w| w.number <= largest));
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
fn manual_coil_links_to_power_buses_replace_generated_links() {
    let panel = r#"
supply = "ac400"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
[[device]]
id = "G1"
kind = "psu"
[[link]]
from = "K1:A1"
to = "bus:L1"
[[link]]
from = "K1:A2"
to = "bus:N"
"#;
    let nets = build_nets(&pump(panel), &library());
    let keys = keys(&nets);
    assert!(keys.iter().any(|key| key == "K1:A1>bus:L1"));
    assert!(keys.iter().any(|key| key == "K1:A2>bus:N"));
    assert!(!keys
        .iter()
        .any(|key| key.contains("K1:A1") && key != "K1:A1>bus:L1"));
    assert!(!keys
        .iter()
        .any(|key| key.contains("K1:A2") && key != "K1:A2>bus:N"));
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

/// A breaker with fewer poles passes on only the first conductors, and what stands under it gets
/// just those (the numbers of poles of a device are 1 or 2 on 230 V, 3 or 4 on 400 V).
#[test]
fn the_poles_of_a_breaker_limit_the_conductors_under_it() {
    let power_wires = |supply: &str, poles: &str| {
        let text = format!(
            "supply = \"{supply}\"\n[[device]]\nid = \"X1\"\nkind = \"feed\"\n\
             [[device]]\nid = \"Q1\"\nkind = \"breaker\"\n{poles}load = \"M1\"\n\
             [[device]]\nid = \"M1\"\nkind = \"motor\"\n"
        );
        let mut project = Project::load(repo_path("examples/pump")).unwrap();
        project.panel = toml::from_str::<Panel>(&text).unwrap();
        let nets = build_nets(&project, &library());
        let on = |device: &str| {
            nets.wires
                .iter()
                .filter(|w| w.class == crab_schematic::WireClass::Power && w.to.device == device)
                .count()
        };
        (on("Q1"), on("M1"))
    };
    assert_eq!(power_wires("ac230", ""), (2, 2));
    assert_eq!(power_wires("ac230", "poles = 1\n"), (1, 1));
    assert_eq!(power_wires("ac400", ""), (4, 4));
    assert_eq!(power_wires("ac400", "poles = 3\n"), (3, 3));
    // One pole on 400 V is a phase: the motor under it gets the neutral from the bus.
    assert_eq!(power_wires("ac400", "poles = 1\n"), (1, 2));
}

fn nets_of(panel: &str) -> Nets {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    build_nets(&project, &library())
}

fn key_set(nets: &Nets) -> HashSet<String> {
    nets.wires.iter().map(|w| w.key()).collect()
}

/// A terminal block in the chain passes every conductor and the earth on (`specs/panel-aux`).
#[test]
fn a_terminal_block_in_the_chain_passes_the_conductors_and_the_earth() {
    let nets = nets_of(
        r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "X1.1"
[[device]]
id = "X1.1"
kind = "terminal"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
[[device]]
id = "X9"
kind = "terminal"
"#,
    );
    let k = key_set(&nets);
    for wire in [
        "Q1:2>X1.1:L",
        "Q1:4>X1.1:N",
        "X1.1:L>M1:L",
        "X1.1:N>M1:N",
        "X1:PE>X1.1:PE",
        "X1.1:PE>M1:PE",
    ] {
        assert!(k.contains(wire), "no wire {wire} in {k:?}");
    }
    // Nothing leaves the earth of the feed for the motor behind the terminal block, and a
    // terminal block outside any chain gets no wires.
    assert!(!k.contains("X1:PE>M1:PE"));
    assert!(!nets
        .wires
        .iter()
        .any(|w| w.to.device == "X9" || w.from.device == "X9"));
}

/// Auxiliary contacts: on an input of the board, and in series with a coil (`13-14`, `21-22`).
#[test]
fn auxiliary_contacts_go_to_an_input_or_in_series_with_a_coil() {
    let nets = nets_of(
        r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
form = "nc"
bind = "sensor"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
series = "K1"
[[device]]
id = "K1_2"
kind = "aux"
of = "K1"
series = "K1"
"#,
    );
    let k = key_set(&nets);
    for wire in [
        // The first contact of the breaker, normally closed: 11-12.
        "EXT:+24V>Q1_1:11",
        "Q1_1:12>A1:IO34",
        // Two contacts in series with the coil, normally open: 13-14 and 23-24.
        "A1:IO26>K1_1:13",
        "K1_1:14>K1_2:23",
        "K1_2:24>K1:A1",
        "K1:A2>EXT:0V",
    ] {
        assert!(k.contains(wire), "no wire {wire} in {k:?}");
    }
    // The coil is no longer fed straight from the board.
    assert!(!k.contains("A1:IO26>K1:A1"));
}

const TERMINAL_PANEL: &str = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "X2"
[[device]]
id = "X2"
kind = "terminal"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;

/// The wires that share a terminal are one conductor with one number; a terminal block splits the
/// conductor, and the number after it can be given by hand.
#[test]
fn a_terminal_block_splits_the_numbers_and_the_number_after_it_can_be_set_by_hand() {
    let lib = library();
    let mut project = pump(TERMINAL_PANEL);
    let nets = build_nets(&project, &lib);
    let number = |key: &str| nets.wires.iter().find(|w| w.key() == key).unwrap().number;
    // Before and after the terminal block the conductor has two numbers.
    assert_ne!(number("Q1:2>X2:L"), number("X2:L>M1:L"));
    assert_ne!(number("X1:L>Q1:1"), number("Q1:2>X2:L"));
    // The earth is one conductor to the terminal block and another after it.
    assert_ne!(number("X1:PE>X2:PE"), number("X2:PE>M1:PE"));

    // A number given by hand stays, and is not given again to another conductor.
    let mut manual = nets.clone();
    manual.store(&mut project.panel);
    project
        .panel
        .wire_numbers
        .insert("X2:L>M1:L".to_owned(), 101);
    let again = build_nets(&project, &lib);
    assert_eq!(
        again
            .wires
            .iter()
            .find(|w| w.key() == "X2:L>M1:L")
            .unwrap()
            .number,
        101
    );
    for w in &again.wires {
        if w.key() != "X2:L>M1:L" {
            assert_ne!(w.number, 101, "{}", w.key());
        }
    }
    again.store(&mut project.panel);
    assert_eq!(project.panel.wire_numbers["X2:L>M1:L"], 101);
    manual.wires.clear();
}

/// A hand wire on `A1` of a coil with contacts in series replaces only the last wire into `A1`:
/// the board and the contacts stay wired.
#[test]
fn a_hand_wire_on_a1_keeps_the_contacts_in_series_wired() {
    let nets = nets_of(
        r#"
supply = "ac230"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
series = "K1"
[[link]]
from = "K1:A1"
to = "bus:+24V"
"#,
    );
    let k = key_set(&nets);
    assert!(k.contains("A1:IO26>K1_1:13"), "{k:?}");
    assert!(k.contains("K1:A1>bus:+24V"), "{k:?}");
    assert!(!k.contains("K1_1:14>K1:A1"), "{k:?}");
}

/// The power wires of a panel as keys, sorted: what runs on the mains side.
fn power_keys(nets: &Nets) -> Vec<String> {
    let mut keys: Vec<String> = nets
        .wires
        .iter()
        .filter(|w| w.class == crab_schematic::WireClass::Power)
        .map(|w| w.key())
        .collect();
    keys.sort();
    keys
}

#[test]
fn a_single_phase_breaker_takes_its_phase_and_the_load_gets_the_neutral_from_the_bus() {
    let nets = nets_of(
        r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
load = "Q3"
[[device]]
id = "Q3"
kind = "breaker"
poles = 1
conductors = ["L2"]
load = "E1"
[[device]]
id = "E1"
kind = "load"
"#,
    );
    let keys = keys(&nets);
    for expected in ["X1:L2>Q3:1", "Q3:2>E1:L2", "X1:N>E1:N"] {
        assert!(
            keys.iter().any(|k| k == expected),
            "нет {expected}: {keys:#?}"
        );
    }
    assert!(!keys
        .iter()
        .any(|k| k.starts_with("X1:L1>") || k.starts_with("X1:L3>")));
    assert_eq!(nets.conductors["Q3"], ["L2"]);
    assert_eq!(nets.conductors["E1"], ["L2", "N"]);
}

#[test]
fn a_two_pole_breaker_on_two_phases_gives_no_neutral_below() {
    let nets = nets_of(
        r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
load = "Q4"
[[device]]
id = "Q4"
kind = "breaker"
poles = 2
conductors = ["L1", "L3"]
load = "E2"
[[device]]
id = "E2"
kind = "load"
"#,
    );
    let keys = keys(&nets);
    for expected in ["X1:L1>Q4:1", "X1:L3>Q4:3", "Q4:2>E2:L1", "Q4:4>E2:L3"] {
        assert!(
            keys.iter().any(|k| k == expected),
            "нет {expected}: {keys:#?}"
        );
    }
    assert!(!keys.iter().any(|k| k.ends_with(":N") && k.contains("E2")));
    assert_eq!(nets.conductors["Q4"], ["L1", "L3"]);
    assert_eq!(nets.conductors["E2"], ["L1", "L3"]);
}

#[test]
fn a_motor_under_a_three_pole_contactor_takes_a_phase_and_the_neutral_from_the_feed() {
    let nets = nets_of(
        r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
poles = 3
load = "M1"
[[device]]
id = "M1"
kind = "motor"
conductors = ["L2", "N"]
"#,
    );
    let keys = keys(&nets);
    // The second phase of the contactor is its second pole (`4`); the neutral passes by the bus.
    for expected in ["K1:4>M1:L2", "X1:N>M1:N"] {
        assert!(
            keys.iter().any(|k| k == expected),
            "нет {expected}: {keys:#?}"
        );
    }
    assert_eq!(nets.conductors["M1"], ["L2", "N"]);
}

#[test]
fn an_old_three_pole_breaker_on_400_volts_gives_the_same_wires() {
    let nets = nets_of(
        r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
poles = 3
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#,
    );
    assert_eq!(
        power_keys(&nets),
        [
            "Q1:2>M1:L1",
            "Q1:4>M1:L2",
            "Q1:6>M1:L3",
            "X1:L1>Q1:1",
            "X1:L2>Q1:3",
            "X1:L3>Q1:5",
        ]
    );
}

/// A sensor wired through a terminal block (`via`): each of its three wires takes two wires, in
/// and out of the block, on the pins `X2:B1+`, `X2:B1-`, `X2:B1S` (`specs/panel-aux`, task 11).
#[test]
fn a_sensor_through_a_terminal_block_takes_two_wires_per_conductor() {
    let panel = PUMP_PANEL.replace(
        "bind = \"sensor.out\"",
        "bind = \"sensor.out\"\nvia = \"X2\"",
    ) + "\n[[device]]\nid = \"X2\"\nkind = \"terminal\"\n";
    let nets = nets_of(&panel);
    let k = key_set(&nets);
    for wire in [
        "G1:+24V>X2:B1+",
        "X2:B1+>B1:+",
        "G1:0V>X2:B1-",
        "X2:B1->B1:-",
        "B1:S>X2:B1S",
        "X2:B1S>A1:IO34",
    ] {
        assert!(k.contains(wire), "no wire {wire} in {k:?}");
    }
    // The wires straight to the sensor are gone.
    for wire in ["G1:+24V>B1:+", "G1:0V>B1:-", "B1:S>A1:IO34"] {
        assert!(!k.contains(wire), "{wire} is still there");
    }
    // The wire in and the wire out of the block are two conductors with two numbers.
    let number = |key: &str| nets.wires.iter().find(|w| w.key() == key).unwrap();
    for (inn, out) in [
        ("G1:+24V>X2:B1+", "X2:B1+>B1:+"),
        ("G1:0V>X2:B1-", "X2:B1->B1:-"),
        ("B1:S>X2:B1S", "X2:B1S>A1:IO34"),
    ] {
        let (a, b) = (number(inn), number(out));
        assert_ne!(a.net, b.net, "{inn} and {out} are one conductor");
        assert_ne!(a.number, b.number, "{inn} and {out} have one number");
    }
    // A sensor wired straight keeps its three wires as before.
    let straight = key_set(&nets_of(PUMP_PANEL));
    assert!(straight.contains("G1:+24V>B1:+"));
    assert!(!straight.iter().any(|k| k.contains("X2:")));
}

/// Two sensors on one terminal block: each has its own pins on the block, so no wire is shared.
#[test]
fn two_sensors_on_one_terminal_block_have_wires_of_their_own() {
    let panel = PUMP_PANEL.replace("bind = \"sensor.out\"", "bind = \"sensor.out\"\nvia = \"X2\"")
        + "\n[[device]]\nid = \"X2\"\nkind = \"terminal\"\n[[device]]\nid = \"B2\"\nkind = \"sensor\"\nbind = \"sensor.out\"\nvia = \"X2\"\n";
    let nets = nets_of(&panel);
    let keys: Vec<String> = nets.wires.iter().map(|w| w.key()).collect();
    let unique: HashSet<&String> = keys.iter().collect();
    assert_eq!(unique.len(), keys.len(), "repeated wire keys: {keys:?}");
    for wire in [
        "X2:B1+>B1:+",
        "X2:B2+>B2:+",
        "B2:S>X2:B2S",
        "X2:B2S>A1:IO34",
    ] {
        assert!(keys.iter().any(|k| k == wire), "no wire {wire} in {keys:?}");
    }
    // The two sensors' wires to the block are two conductors with their own numbers.
    let number = |key: &str| nets.wires.iter().find(|w| w.key() == key).unwrap().number;
    assert_ne!(number("X2:B1+>B1:+"), number("X2:B2+>B2:+"));
}

/// A hand wire on a pin of a sensor's terminal block replaces the automatic wire at that pin.
#[test]
fn a_hand_wire_on_a_sensor_pin_of_a_terminal_block_replaces_the_automatic_one() {
    let panel = PUMP_PANEL.replace("bind = \"sensor.out\"", "bind = \"sensor.out\"\nvia = \"X2\"")
        + "\n[[device]]\nid = \"X2\"\nkind = \"terminal\"\n[[link]]\nfrom = \"X2:B1S\"\nto = \"K1:A2\"\n";
    let k = key_set(&nets_of(&panel));
    assert!(k.contains("X2:B1S>K1:A2"), "{k:?}");
    assert!(!k.contains("X2:B1S>A1:IO34"), "{k:?}");
    // The other pins of the sensor keep their automatic wires.
    assert!(k.contains("X2:B1+>B1:+"));
}

/// Without `via` the sensor's wires and numbers are the same as before a block was added.
#[test]
fn no_via_gives_the_same_wires_and_numbers_as_before() {
    let plain = nets_of(PUMP_PANEL);
    let with_unused_block =
        nets_of(&(PUMP_PANEL.to_owned() + "\n[[device]]\nid = \"X2\"\nkind = \"terminal\"\n"));
    let pairs =
        |n: &Nets| -> Vec<(String, u32)> { n.wires.iter().map(|w| (w.key(), w.number)).collect() };
    assert_eq!(pairs(&plain), pairs(&with_unused_block));
}
