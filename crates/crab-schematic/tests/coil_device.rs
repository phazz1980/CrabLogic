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

/// The contact table under a coil is a symbol of its own (`K1/table`) that the offset of the
/// layout moves, like the coil.
#[test]
fn the_contact_table_moves_by_its_own_offset() {
    let lib = library();
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    let origin = |project: &Project| {
        let (doc, _) = build_document(project, &lib, Lang::Ru).unwrap();
        doc.sheets
            .iter()
            .flat_map(|s| s.items.iter())
            .find_map(|i| match &i.kind {
                crab_schematic::ItemKind::Symbol { device, origin, .. } if device == "K1/table" => {
                    Some(*origin)
                }
                _ => None,
            })
    };
    let before = origin(&project).expect("table of K1 drawn");
    project
        .panel_layout
        .set_shift("control:K1/table", 40.0, 20.0);
    let after = origin(&project).unwrap();
    assert_eq!((after.0 - before.0, after.1 - before.1), (40.0, 20.0));
}

/// A worked-out wire the user deleted (`panel.cut`) is left out of the nets.
#[test]
fn a_cut_wire_leaves_the_nets() {
    let lib = library();
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    let wires = build_nets(&project, &lib).wires;
    let key = wires.first().expect("pump has wires").key();
    project.panel.cut.push(key.clone());
    let after = build_nets(&project, &lib).wires;
    assert_eq!(after.len(), wires.len() - 1);
    assert!(after.iter().all(|w| w.key() != key));
}

/// A wire turned into a break keeps its bend and is a line again when turned back.
#[test]
fn a_broken_wire_flag_does_not_lose_the_bend() {
    let mut layout = crab_core::PanelLayout::default();
    layout.set_bend("A:1>B:2", Some(20.0));
    layout.set_broken("A:1>B:2", true);
    assert!(layout.is_broken("A:1>B:2"));
    assert_eq!(layout.bend("A:1>B:2"), Some(20.0));
    layout.set_bend("A:1>B:2", None);
    assert!(layout.is_broken("A:1>B:2"));
    layout.set_broken("A:1>B:2", false);
    assert!(layout.wires.is_empty());
}

/// A deleted wire of the power chain is not drawn on the power sheet either.
#[test]
fn a_cut_wire_is_not_drawn_on_the_power_sheet() {
    let lib = library();
    let project = Project::load(repo_path("examples/pump")).unwrap();
    let drawn = |project: &Project| -> Vec<String> {
        let (doc, _) = build_document(project, &lib, Lang::Ru).unwrap();
        doc.sheets
            .iter()
            .filter(|s| matches!(s.id, Some(crab_core::SheetId::Power(_))))
            .flat_map(|s| s.items.iter())
            .filter_map(|i| match &i.kind {
                crab_schematic::ItemKind::Wire { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    };
    let before = drawn(&project);
    assert!(!before.is_empty());
    for key in before {
        let mut cut = project.clone();
        cut.panel.cut.push(key.clone());
        let after = drawn(&cut);
        assert!(!after.contains(&key), "{key} still drawn: {after:?}");
    }
}
