//! Control sheets and cross references (`specs/electrical-schematics`, task 5).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::Page;
use crab_schematic::{build_document, Document, Lang};

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

fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

fn document(project: &Project) -> Document {
    build_document(project, &library(), Lang::Ru).unwrap().0
}

#[test]
fn every_input_and_output_of_the_panel_is_on_the_control_sheet() {
    let project = pump(PUMP_PANEL);
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    let body = &doc.sheets[2].body;
    assert!(!nets.board_terminals.is_empty());
    for t in &nets.board_terminals {
        for text in [&t.terminal, &t.tag, &format!("-{}", t.device)] {
            assert!(
                body.contains(&quoted(text)),
                "нет «{text}» на листе управления"
            );
        }
        // The wire of the terminal is numbered.
        assert!(
            body.contains(&quoted(&t.wire.to_string())),
            "нет номера {}",
            t.wire
        );
    }
    for text in ["+24V", "0V", "-A1"] {
        assert!(body.contains(&quoted(text)), "нет {text}");
    }
    assert!(
        body.contains(&quoted("Питание 24 В: -G1 (1.B1)")) || body.contains("Питание 24 В: -G1 (")
    );
}

#[test]
fn the_other_sheet_names_the_place_of_the_coil_and_of_the_contacts() {
    let doc = document(&pump(PUMP_PANEL));
    let coil = &doc.coil_at["K1"];
    let contacts = &doc.power_at["K1"];
    // The sheets the references point to are the ones that draw the device.
    assert_eq!(doc.sheets[contacts.sheet - 1].title, "Силовая часть");
    assert_eq!(doc.sheets[coil.sheet - 1].title, "Цепи управления");
    assert!(doc.sheets[contacts.sheet - 1].body.contains(&quoted("-K1")));
    assert!(doc.sheets[coil.sheet - 1].body.contains(&quoted("-K1")));
    // The power sheet says where the coil is, the control sheet where the contacts are.
    assert!(doc.sheets[contacts.sheet - 1]
        .body
        .contains(&quoted(&coil.to_string())));
    assert!(doc.sheets[coil.sheet - 1]
        .body
        .contains(&quoted(&contacts.to_string())));
}

/// The first `RO(x, y, w, h, ...)` call with this size: its middle, in sheet units.
fn rect_centre(body: &str, w: f32, h: f32) -> (f32, f32) {
    for line in body.lines() {
        let Some(args) = line.strip_prefix("RO(") else {
            continue;
        };
        let nums: Vec<f32> = args
            .split(", ")
            .take(4)
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() == 4 && (nums[2] - w).abs() < 0.01 && (nums[3] - h).abs() < 0.01 {
            return (nums[0] + w / 2.0, nums[1] + h / 2.0);
        }
    }
    panic!("нет прямоугольника {w}×{h}");
}

#[test]
fn the_zone_of_a_coil_is_the_zone_where_it_is_drawn() {
    let doc = document(&pump(PUMP_PANEL));
    let coil = &doc.coil_at["K1"];
    let (x, y) = rect_centre(&doc.sheets[coil.sheet - 1].body, 20.0, 50.0);
    assert_eq!(coil.zone, Page::default().zone(x, y));
    // The sensor is a box of 60 × 50 on the same sheet.
    let sensor = &doc.coil_at["B1"];
    let (x, y) = rect_centre(&doc.sheets[sensor.sheet - 1].body, 60.0, 50.0);
    assert_eq!(sensor.zone, Page::default().zone(x, y));
}

#[test]
fn references_are_the_same_in_every_build() {
    let project = pump(PUMP_PANEL);
    let a = document(&project);
    let b = document(&project);
    assert_eq!(a.coil_at, b.coil_at);
    assert_eq!(a.power_at, b.power_at);
}

#[test]
fn many_terminals_go_to_several_control_sheets() {
    let mut project = pump("");
    let mut panel = String::new();
    for i in 1..=15 {
        project.logic.blocks.push(
            toml::from_str(&format!(
                "id = \"out{i}\"\ntype = \"digital_out\"\npin = {}\n",
                i + 20
            ))
            .unwrap(),
        );
        panel.push_str(&format!(
            "[[device]]\nid = \"K{i}\"\nkind = \"relay\"\nbind = \"out{i}\"\n"
        ));
    }
    project.panel = toml::from_str::<Panel>(&panel).unwrap();
    let doc = document(&project);
    // The relays are on the power sheets too (nothing feeds them but the supply); the control
    // sheets hold 13 columns each.
    let control: Vec<usize> = doc
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.title.starts_with("Цепи управления"))
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(
        control.len(),
        2,
        "15 колонок: 13 на первом листе, 2 на втором"
    );
    assert_eq!(doc.sheets[control[0] - 1].title, "Цепи управления (1/2)");
    assert_eq!(doc.sheets[control[1] - 1].title, "Цепи управления (2/2)");
    for i in 1..=15 {
        let id = format!("K{i}");
        let at = &doc.coil_at[&id];
        assert_eq!(at.sheet, control[usize::from(i > 13)], "{id}");
        let on_control = control
            .iter()
            .filter(|&&n| doc.sheets[n - 1].body.contains(&quoted(&format!("-{id}"))))
            .count();
        assert_eq!(on_control, 1, "{id}");
    }
}

#[test]
fn button_and_coil_without_a_source_use_the_external_supply() {
    let panel = r#"
[[device]]
id = "S1"
kind = "button"
bind = "sensor.out"
[[device]]
id = "K1"
kind = "relay"
bind = "pump"
"#;
    let doc = document(&pump(panel));
    let control = doc
        .sheets
        .iter()
        .find(|s| s.title == "Цепи управления")
        .expect("лист управления");
    assert!(control.body.contains(&quoted("-S1")));
    assert!(control.body.contains("внешний источник"));
}

const CONTACTS_PANEL: &str = r#"
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
load = "M1"
[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
series = "K1"
[[device]]
id = "K1_2"
kind = "aux"
of = "Q1"
series = "K1"
[[device]]
id = "M1"
kind = "motor"
"#;

/// An auxiliary contact on an input stands under its terminal like a button, the contacts in
/// series with a coil stand above the coil, and every wire between them is drawn.
#[test]
fn auxiliary_contacts_stand_on_the_control_sheet() {
    use crab_schematic::scene::ItemKind;
    let doc = document(&pump(CONTACTS_PANEL));
    let sheet = doc
        .sheets
        .iter()
        .find(|s| s.title.starts_with("Цепи управления"))
        .unwrap();
    // The designation of the device the contact belongs to and the terminals are written.
    for text in ["-Q1", "-K1", "11", "12", "13", "14", "23", "24"] {
        assert!(sheet.body.contains(&quoted(text)), "нет надписи {text}");
    }
    // The wires of the chain: board -> K1_1 -> K1_2 -> coil.
    let wires: Vec<(String, String)> = sheet
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Wire { from, to, .. } => Some((from.device.clone(), to.device.clone())),
            _ => None,
        })
        .collect();
    for (from, to) in [("A1", "K1_1"), ("K1_1", "K1_2"), ("K1_2", "K1")] {
        assert!(
            wires
                .iter()
                .any(|w| (w.0.as_str(), w.1.as_str()) == (from, to)),
            "нет провода {from} -> {to}: {wires:?}"
        );
    }
    // The contact on the input is a symbol with a wire to the board.
    assert!(wires.iter().any(|w| w.1 == "Q1_1" || w.0 == "Q1_1"));
}

/// The power sheet names where each auxiliary contact is used.
#[test]
fn the_power_sheet_names_the_place_of_the_auxiliary_contacts() {
    let doc = document(&pump(CONTACTS_PANEL));
    let power = &doc.sheets[1].body;
    // Under the contacts of the breaker: the references to the control sheet.
    let refs = power
        .split('"')
        .filter(|t| t.starts_with('/') || t.contains(" /"))
        .count();
    assert!(refs >= 2, "{refs} references only");
}
