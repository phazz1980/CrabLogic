//! Main contacts on the coil of another device (`specs/panel-coil-device`): a contactor with `of`.

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::nets::terminals_of;
use crab_schematic::{build_document, build_nets, Lang, BOARD};

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

fn pump(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
}

/// K1 has the coil (bound to `pump`) and its own main contacts; K2 is the main contacts of the
/// coil of K1 and feeds the second motor.
const PANEL: &str = r#"
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
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "K2"
kind = "contactor"
of = "K1"
from = "Q1"
load = "M2"
[[device]]
id = "M1"
kind = "motor"
[[device]]
id = "M2"
kind = "motor"
[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

#[test]
fn contacts_of_another_coil_have_no_coil_of_their_own() {
    let project = pump(PANEL);
    let nets = build_nets(&project, &library());
    // Only the coil of K1 is on a terminal of the board; nothing drives K2.
    assert!(nets.board_terminals.iter().all(|t| t.device != "K2"));
    assert!(nets.board_terminals.iter().any(|t| t.device == "K1"));
    assert!(nets.wires.iter().all(|w| {
        !(w.from.device == "K2" && w.from.terminal.starts_with('A'))
            && !(w.to.device == "K2" && w.to.terminal.starts_with('A'))
    }));
    assert!(nets.wires.iter().any(|w| w.from.device == BOARD));
    // K2 has the poles only, no `A1`/`A2`.
    let panel = &project.panel;
    let terminals = terminals_of(panel, panel.device("K2").unwrap());
    assert!(
        !terminals.iter().any(|t| t.starts_with('A')),
        "{terminals:?}"
    );
    assert!(terminals.contains(&"1".to_owned()));
    let own = terminals_of(panel, panel.device("K1").unwrap());
    assert!(own.contains(&"A1".to_owned()) && own.contains(&"A2".to_owned()));
}

#[test]
fn the_symbol_is_drawn_and_refers_to_the_coil_of_the_target() {
    let (doc, _) = build_document(&pump(PANEL), &library(), Lang::Ru).unwrap();
    // The power symbol is there; the coil is drawn once, under K1 only.
    let k2 = doc.power_at.get("K2").expect("K2 on a power sheet");
    assert!(!doc.coil_at.contains_key("K2"));
    let coil = doc.coil_at.get("K1").expect("coil of K1").to_string();
    let body = &doc.sheets[k2.sheet - 1].body;
    // The reference under both K1 and K2 is the place of the coil of K1.
    let refs = body.matches(&format!("\"{coil}\"")).count();
    assert!(refs >= 2, "{refs} refs to {coil}");
}

#[test]
fn a_contactor_without_of_still_has_its_own_coil() {
    let project = pump(&PANEL.replace("of = \"K1\"\n", ""));
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    // With an empty `of` K2 is a plain contactor again (the check asks for its binding).
    assert!(!doc.coil_at.contains_key("K2"));
    assert!(doc.power_at.contains_key("K2"));
    assert!(project
        .panel
        .has_own_coil(project.panel.device("K2").unwrap()));
}
