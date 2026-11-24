//! Power sheets of a 400 V panel with devices on chosen conductors (`specs/panel-phase`, task 3).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_document, Document, ItemKind, Lang, Nets, Op, WireClass};

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

/// The feed feeds `Q2` by its `load`; `Q3` and `Q4` hang on the feed by `from`, so each is a drop
/// from the buses. `Q2` takes the conductor `L2` alone; `E1` takes `L2` and the neutral, which
/// passes `Q2` by from the feed.
const PANEL: &str = r#"
supply = "ac400"

[[device]]
id = "X1"
kind = "feed"
load = "Q2"

[[device]]
id = "Q2"
kind = "breaker"
poles = 1
conductors = ["L2"]
load = "E1"

[[device]]
id = "E1"
kind = "load"
rating = "2 кВт"

[[device]]
id = "Q3"
kind = "breaker"
poles = 3
from = "X1"
load = "M1"

[[device]]
id = "M1"
kind = "motor"
rating = "1,1 кВт"

[[device]]
id = "Q4"
kind = "breaker"
poles = 4
from = "X1"
"#;

fn build(panel: &str) -> (Document, Nets) {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    build_document(&project, &library(), Lang::Ru).unwrap()
}

/// Key of the first power wire from `from` to `to`, as the nets number it.
fn key(nets: &Nets, from: &str, to: &str) -> String {
    nets.wires
        .iter()
        .find(|w| w.class == WireClass::Power && w.from.device == from && w.to.device == to)
        .unwrap_or_else(|| panic!("no wire {from} > {to}"))
        .key()
}

/// Key of the power wire of the conductor `terminal` of the feed that goes to `to`.
fn feed_wire(nets: &Nets, to: &str, terminal: &str) -> String {
    nets.wires
        .iter()
        .find(|w| {
            w.class == WireClass::Power
                && w.from.device == "X1"
                && w.from.terminal == terminal
                && w.to.device == to
        })
        .unwrap_or_else(|| panic!("no {terminal} wire to {to}"))
        .key()
}

/// The points of the line of the wire `key`.
fn points(doc: &Document, key: &str) -> Vec<(f32, f32)> {
    for sheet in &doc.sheets {
        for item in &sheet.items {
            let ItemKind::Wire { key: k, .. } = &item.kind else {
                continue;
            };
            if k != key {
                continue;
            }
            for &i in &item.ops {
                if let Op::Line { points, .. } = &sheet.ops[i] {
                    return points.clone();
                }
            }
        }
    }
    panic!("no line of wire {key}");
}

/// Where a drop from a bus leaves the bus: the first point of its vertical line.
fn drop_top(doc: &Document, key: &str) -> (f32, f32) {
    let line = points(doc, key);
    assert_eq!(line.len(), 2, "a drop is one vertical line: {line:?}");
    line[0]
}

#[test]
fn a_single_phase_breaker_drops_from_the_bus_of_its_own_line() {
    let (doc, nets) = build(PANEL);
    // `Q2` takes `L2` alone: its drop starts on the bus of `L2`.
    let q2 = drop_top(&doc, &feed_wire(&nets, "Q2", "L2"));
    // Pole 1 of `Q3` is `L2` and starts on the same bus; its pole 0 is `L1`, a bus of its own.
    let q3_l2 = drop_top(&doc, &feed_wire(&nets, "Q3", "L2"));
    let q3_l1 = drop_top(&doc, &feed_wire(&nets, "Q3", "L1"));
    assert_eq!(
        q2.1, q3_l2.1,
        "Q2 and the L2 pole of Q3 share the bus of L2"
    );
    assert_ne!(q3_l1.1, q2.1, "L1 is a bus of its own");
}

#[test]
fn the_neutral_past_a_breaker_is_dropped_from_the_neutral_bus() {
    let (doc, nets) = build(PANEL);
    assert_eq!(nets.conductors["E1"], ["L2", "N"]);
    // The neutral of `E1` comes from the feed, past `Q2`: a drop from the bus of `N`, the same bus
    // as the neutral of the four-pole breaker `Q4`.
    let e1_n = drop_top(&doc, &key(&nets, "X1", "E1"));
    let q4_n = drop_top(&doc, &feed_wire(&nets, "Q4", "N"));
    assert_eq!(e1_n.1, q4_n.1, "E1 takes the neutral from the bus N");
}

#[test]
fn a_load_under_a_breaker_hangs_on_it_not_on_the_bus() {
    let (doc, nets) = build(PANEL);
    // The phase of `E1` comes from `Q2` (its pole 0): the wire leaves `Q2`, not the bus of `L2`.
    let from_q2 = points(&doc, &key(&nets, "Q2", "E1"));
    let l2_bus = drop_top(&doc, &feed_wire(&nets, "Q2", "L2"));
    assert_ne!(from_q2[0].1, l2_bus.1);
}

#[test]
fn the_poles_of_a_four_pole_breaker_are_in_bus_order() {
    let (doc, nets) = build(PANEL);
    // A device that takes all the conductors has pole k on the line k.
    let tops: Vec<f32> = ["L1", "L2", "L3", "N"]
        .iter()
        .map(|line| drop_top(&doc, &feed_wire(&nets, "Q4", line)).1)
        .collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
}

#[test]
fn a_two_pole_breaker_under_a_contactor_takes_a_phase_and_the_neutral() {
    let panel = r#"
supply = "ac400"

[[device]]
id = "X1"
kind = "feed"
load = "K1"

[[device]]
id = "K1"
kind = "contactor"
poles = 3
load = "Q2"

[[device]]
id = "Q2"
kind = "breaker"
poles = 2
load = "E1"

[[device]]
id = "E1"
kind = "load"
"#;
    let (_, nets) = build(panel);
    assert_eq!(nets.conductors["Q2"], ["L1", "N"]);
}

#[test]
fn a_single_phase_main_breaker_hangs_on_the_bus_of_its_phase() {
    // The only circuit of the feed: a one-pole breaker on `L2` does not stand across the lines,
    // it drops from the bus of `L2` like the pole `L2` of the three-pole `Q3` would.
    let panel = r#"
supply = "ac400"

[[device]]
id = "X1"
kind = "feed"
load = "Q1"

[[device]]
id = "Q1"
kind = "breaker"
poles = 1
conductors = ["L2"]
load = "E1"

[[device]]
id = "E1"
kind = "load"
"#;
    let (doc, nets) = build(panel);
    let q1 = drop_top(&doc, &feed_wire(&nets, "Q1", "L2"));
    let (doc3, nets3) = build(PANEL);
    let q3_l2 = drop_top(&doc3, &feed_wire(&nets3, "Q3", "L2"));
    assert_eq!(q1.1, q3_l2.1, "Q1 drops from the bus of L2");
    // The neutral of `E1` names the feed as its source end, not a missing pole of `Q1`.
    for sheet in &doc.sheets {
        for item in &sheet.items {
            if let ItemKind::Wire { from, to, .. } = &item.kind {
                if to.device == "E1" && to.port == "in:1" {
                    assert_eq!(from.device, "X1");
                }
            }
        }
    }
}
