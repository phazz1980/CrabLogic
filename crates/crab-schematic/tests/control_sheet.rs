//! Control sheets and cross references (`specs/electrical-schematics`, task 5).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::Page;
use crab_schematic::{build_document, Document, ItemKind, Lang};

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
    // Under the coil: the table of the contacts, the main ones (1-4 on a single phase) first.
    assert!(doc.sheets[coil.sheet - 1]
        .body
        .contains(&quoted(&format!("1-4 {contacts}"))));
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

/// A panel with a contactor K1 that no board terminal is bound to, so the generator does not
/// draw it on the control sheet.
const FREE_CONTACTOR_PANEL: &str = r#"
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
"#;

/// Ports of the symbol of `device` on the control sheets, if it is drawn there.
fn control_symbol_ports(doc: &Document, device: &str) -> Option<Vec<String>> {
    doc.sheets
        .iter()
        .filter(|s| s.title.starts_with("Цепи управления"))
        .flat_map(|s| &s.items)
        .find_map(|i| match &i.kind {
            ItemKind::Symbol {
                view: "control",
                device: d,
                ports,
                ..
            } if d == device => Some(ports.iter().map(|p| p.name.clone()).collect()),
            _ => None,
        })
}

/// Whether a wire with the key `key` is drawn on the control sheets.
fn control_has_wire(doc: &Document, key: &str) -> bool {
    doc.sheets
        .iter()
        .filter(|s| s.title.starts_with("Цепи управления"))
        .flat_map(|s| &s.items)
        .any(|i| matches!(&i.kind, ItemKind::Wire { key: k, .. } if k == key))
}

#[test]
fn a_coil_put_by_hand_is_its_own_symbol() {
    let mut project = pump(&format!(
        "{FREE_CONTACTOR_PANEL}\n[[link]]\nfrom = \"K1:A1\"\nto = \"Q2:2\"\n"
    ));
    project.panel_layout.place("K1", 0, 300.0, 300.0);
    project.panel_layout.place("K1/coil", 0, 300.0, 200.0);
    project.panel_layout.place("Q2", 0, 500.0, 300.0);
    let doc = document(&project);
    assert_eq!(
        control_symbol_ports(&doc, "K1/coil"),
        Some(vec!["A1".to_owned(), "A2".to_owned()])
    );
    // The coil terminals belong to the coil symbol, not to the box of the device.
    let box_ports = control_symbol_ports(&doc, "K1").expect("K1 on the control sheet");
    assert!(!box_ports.iter().any(|p| p == "A1" || p == "A2"));
    // The link from the coil terminal to the breaker is drawn between the pins.
    assert!(control_has_wire(&doc, "K1:A1>Q2:2"));
}

#[test]
fn a_coil_put_by_hand_draws_without_its_device() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    project.panel_layout.place("K1/coil", 0, 300.0, 300.0);
    let doc = document(&project);
    assert_eq!(
        control_symbol_ports(&doc, "K1/coil"),
        Some(vec!["A1".to_owned(), "A2".to_owned()])
    );
    assert_eq!(control_symbol_ports(&doc, "K1"), None);
}

/// Whether a symbol of `device` is drawn on any sheet.
fn symbol_drawn(doc: &Document, device: &str) -> bool {
    doc.sheets
        .iter()
        .flat_map(|s| &s.items)
        .any(|i| matches!(&i.kind, ItemKind::Symbol { device: d, .. } if d == device))
}

/// A coil put by hand is registered under its contactor: the power sheet writes its place under
/// the contactor, and moving the coil moves that reference.
#[test]
fn a_coil_put_by_hand_is_the_place_of_its_contactor() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    project.panel_layout.place("K1/coil", 0, 300.0, 300.0);
    let before = document(&project);
    let coil = before.coil_at["K1"].clone();
    assert_eq!(before.sheets[coil.sheet - 1].title, "Цепи управления");
    let contacts = &before.power_at["K1"];
    assert!(before.sheets[contacts.sheet - 1]
        .body
        .contains(&quoted(&coil.to_string())));
    // Moved along the sheet: the reference follows the coil.
    project
        .panel_layout
        .set_shift(&crab_core::symbol_key("control", "K1/coil"), 500.0, 0.0);
    let after = document(&project);
    assert_ne!(after.coil_at["K1"].zone, coil.zone);
    assert!(after.sheets[contacts.sheet - 1]
        .body
        .contains(&quoted(&after.coil_at["K1"].to_string())));
}

/// The table under a coil drawn by the generator lists the main contacts with their place on the
/// power sheets and each auxiliary contact with its terminals and its place; moving the auxiliary
/// contact changes its line.
#[test]
fn the_contact_table_under_a_coil_names_the_contacts_and_their_places() {
    let mut project = pump(CONTACTS_PANEL);
    let before = document(&project);
    let coil_sheet = before.coil_at["K1"].sheet;
    let body = &before.sheets[coil_sheet - 1].body;
    assert!(body.contains(&quoted(&format!("1-4 {}", before.power_at["K1"]))));
    let aux = before.coil_at["K1_1"].clone();
    assert!(body.contains(&quoted(&format!("13-14 {aux}"))));
    // The auxiliary contact is moved along the sheet: its line in the table follows it.
    project
        .panel_layout
        .set_shift(&crab_core::symbol_key("control", "K1_1"), 400.0, 0.0);
    let after = document(&project);
    assert_ne!(after.coil_at["K1_1"].zone, aux.zone);
    assert!(after.sheets[coil_sheet - 1]
        .body
        .contains(&quoted(&format!("13-14 {}", after.coil_at["K1_1"]))));
}

/// An auxiliary contact put on the control sheet by hand is named in the table under the coil
/// of its contactor, and moving it changes the reference.
#[test]
fn a_contact_put_by_hand_is_named_in_the_table_under_the_coil() {
    let panel =
        format!("{FREE_CONTACTOR_PANEL}\n[[device]]\nid = \"K1_1\"\nkind = \"aux\"\nof = \"K1\"\n");
    let mut project = pump(&panel);
    project.panel_layout.place("K1/coil", 0, 300.0, 300.0);
    project.panel_layout.place("K1_1", 0, 300.0, 200.0);
    let before = document(&project);
    let aux = before.coil_at["K1_1"].clone();
    let coil_sheet = before.coil_at["K1"].sheet;
    assert!(before.sheets[coil_sheet - 1]
        .body
        .contains(&quoted(&format!("13-14 {aux}"))));
    project.panel_layout.place("K1_1", 0, 900.0, 200.0);
    let after = document(&project);
    assert_ne!(after.coil_at["K1_1"].zone, aux.zone);
    assert!(after.sheets[coil_sheet - 1]
        .body
        .contains(&quoted(&format!("13-14 {}", after.coil_at["K1_1"]))));
}

/// A coil put by hand for a contactor that the generator draws on the control sheet gives way:
/// the generator's coil is the only one drawn.
#[test]
fn a_coil_put_by_hand_gives_way_to_the_coil_the_generator_draws() {
    let mut project = pump(PUMP_PANEL);
    project.panel_layout.place("K1/coil", 0, 300.0, 300.0);
    let doc = document(&project);
    assert_eq!(control_symbol_ports(&doc, "K1/coil"), None);
    assert!(control_symbol_ports(&doc, "K1").is_some());
}

/// A part key that is not a part of the device (`K1/foo`) draws nothing and names no place.
#[test]
fn an_unknown_part_put_by_hand_draws_nothing() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    project.panel_layout.place("K1/foo", 0, 300.0, 300.0);
    let doc = document(&project);
    assert!(!symbol_drawn(&doc, "K1/foo"));
    assert!(!doc.coil_at.contains_key("K1"));
}

/// A coil put by hand on a motor (`M1/coil`) is not drawn: only contactors and relays have one.
#[test]
fn a_coil_put_by_hand_on_a_motor_draws_nothing() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    project.panel_layout.place("M1/coil", 0, 300.0, 300.0);
    let doc = document(&project);
    assert!(!symbol_drawn(&doc, "M1/coil"));
    assert!(!doc.coil_at.contains_key("M1"));
}

/// The pump with the coil of `K1` and `aux` auxiliary contacts of `K1` in the panel.
fn coil_panel(aux: usize) -> String {
    let mut panel = PUMP_PANEL.to_owned();
    for i in 1..=aux {
        panel.push_str(&format!(
            "\n[[device]]\nid = \"K1_{i}\"\nkind = \"aux\"\nof = \"K1\"\n"
        ));
    }
    panel
}

/// The control sheet of a document.
fn control_body(doc: &Document) -> &str {
    &doc.sheets
        .iter()
        .find(|s| s.title.starts_with("Цепи управления"))
        .expect("лист управления")
        .body
}

/// The numbers of a markup call (`W(...)`, `T(...)`), in order; the quoted text is left out.
fn call_numbers(line: &str) -> Vec<f32> {
    let args = line.split_once('(').map_or(line, |(_, args)| args);
    args.replace(['(', ')'], "")
        .split(", ")
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// The height of the `+24V` bus: the upper of the two long horizontal lines of the control sheet.
/// The lines are the buses, `+24V` above `0V`.
fn plus_bus_y(body: &str) -> f32 {
    let mut ys: Vec<f32> = body
        .lines()
        .filter(|l| l.starts_with("W("))
        .filter_map(|l| {
            // The width, then the two ends of the line.
            let n = call_numbers(l);
            (n.len() == 5 && n[2] == n[4] && (n[1] - n[3]).abs() > 100.0).then_some(n[2])
        })
        .collect();
    ys.sort_by(f32::total_cmp);
    assert_eq!(ys.len(), 2, "две шины: {ys:?}");
    ys[0]
}

/// The heights of the lines of the contact table: the texts `1-4 ...`, `13-14 ...` on the sheet.
fn table_ys(body: &str) -> Vec<f32> {
    body.lines()
        .filter(|l| l.starts_with("T("))
        .filter_map(|l| {
            let text = l
                .rsplit_once(", \"")?
                .1
                .trim_end_matches(')')
                .trim_end_matches('"');
            let table_like = text.starts_with(|c: char| c.is_ascii_digit()) && text.contains('-');
            table_like.then(|| call_numbers(l)[1])
        })
        .collect()
}

/// A coil with six auxiliary contacts has a table of seven lines, longer than the four that fit
/// above `+24V`: the buses go down, and the last line stays above `+24V`. With two auxiliary
/// contacts the buses stay where they are.
#[test]
fn a_long_contact_table_pushes_the_buses_down() {
    let none = document(&pump(&coil_panel(0)));
    let two = document(&pump(&coil_panel(2)));
    let six = document(&pump(&coil_panel(6)));

    let body = control_body(&six);
    let lines = table_ys(body);
    assert_eq!(lines.len(), 7, "строки таблицы: {lines:?}");
    let last = lines.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        last < plus_bus_y(body),
        "последняя строка {last} ниже +24V {}",
        plus_bus_y(body)
    );

    let base = plus_bus_y(control_body(&none));
    assert_eq!(plus_bus_y(control_body(&two)), base);
    // Three lines beyond the four that fit: the buses are pushed down by three lines.
    assert!((plus_bus_y(body) - base - 3.0 * 13.0).abs() < 0.05);
}
