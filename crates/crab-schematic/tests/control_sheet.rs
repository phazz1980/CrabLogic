//! Control sheets and cross references (`specs/electrical-schematics`, task 5).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project, SheetId};
use crab_schematic::Page;
use crab_schematic::{build_document, Align, Document, ItemKind, Lang, Op};

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
        // The channel of the terminal shows the tag and the designation of the terminal.
        for text in [
            &format!("-A1:{}", t.terminal),
            &t.tag,
            &format!("-{}", t.device),
        ] {
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
    for text in ["+24V", "0V"] {
        assert!(body.contains(&quoted(text)), "нет {text}");
    }
    // The note under the 0V rail: the name of the supply, then its source.
    assert!(body.contains(&quoted("Питание 24 В:")) && body.contains("-G1 ("));
}

#[test]
fn the_other_sheet_names_the_place_of_the_coil_and_of_the_contacts() {
    let doc = document(&pump(PUMP_PANEL));
    let coil = &doc.coil_at["K1"];
    let contacts = &doc.power_at["K1"];
    // The sheets the references point to are the ones that draw the device.
    assert_eq!(doc.sheets[contacts.sheet - 1].title, "Силовая часть");
    // The sheet has the motor's contactor and a sensor of no load: the default title.
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
        .filter(|(_, s)| matches!(s.id, Some(SheetId::Control(_))))
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
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
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

/// The name of the first drawing sheet after the power sheets: the first control sheet, or the first
/// free sheet when the panel has no control sheet.
fn after_power(project: &Project) -> SheetId {
    let control = document(project)
        .sheets
        .iter()
        .any(|s| matches!(s.id, Some(SheetId::Control(_))));
    if control {
        SheetId::Control(0)
    } else {
        SheetId::Free(0)
    }
}

/// Ports of the symbol of `device` on the drawing sheets, if it is drawn by hand there.
fn control_symbol_ports(doc: &Document, device: &str) -> Option<Vec<String>> {
    doc.sheets
        .iter()
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

/// Whether a wire with the key `key` is drawn on the drawing sheets.
fn control_has_wire(doc: &Document, key: &str) -> bool {
    doc.sheets
        .iter()
        .flat_map(|s| &s.items)
        .any(|i| matches!(&i.kind, ItemKind::Wire { key: k, .. } if k == key))
}

/// A coil put by hand is its own symbol with its terminals; a link from its terminal is drawn.
/// The contactor K9 is not in a power chain: a contactor of the chain is drawn by the generator.
#[test]
fn a_coil_put_by_hand_is_its_own_symbol() {
    // Q2 is fed from the feed (a device of a power chain is drawn by the generator), so the
    // breaker put by hand is the free Q9.
    let mut project = pump(&format!(
        "{FREE_CONTACTOR_PANEL}\n[[device]]\nid = \"K9\"\nkind = \"contactor\"\nfree = true\n\
         [[device]]\nid = \"Q9\"\nkind = \"breaker\"\nfree = true\n\
         [[link]]\nfrom = \"K9:A1\"\nto = \"Q9:2\"\n"
    ));
    let at = after_power(&project);
    project.panel_layout.place("K9", at, 300.0, 300.0);
    project.panel_layout.place("K9/coil", at, 300.0, 200.0);
    project.panel_layout.place("Q9", at, 500.0, 300.0);
    let doc = document(&project);
    assert_eq!(
        control_symbol_ports(&doc, "K9/coil"),
        Some(vec!["A1".to_owned(), "A2".to_owned()])
    );
    // The coil terminals belong to the coil symbol, not to the box of the device.
    let box_ports = control_symbol_ports(&doc, "K9").expect("K9 drawn by hand");
    assert!(!box_ports.iter().any(|p| p == "A1" || p == "A2"));
    // The link from the coil terminal to the breaker is drawn between the pins.
    assert!(control_has_wire(&doc, "K9:A1>Q9:2"));
}

#[test]
fn a_coil_put_by_hand_draws_without_its_device() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    project
        .panel_layout
        .place("K1/coil", after_power(&project), 300.0, 300.0);
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
    let at = after_power(&project);
    project.panel_layout.place("K1/coil", at, 300.0, 300.0);
    let before = document(&project);
    let coil = before.coil_at["K1"].clone();
    // No control sheet (no terminal of the board is bound): the coil is on a free sheet.
    assert_eq!(before.sheets[coil.sheet - 1].title, "Схема 1");
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
    let at = after_power(&project);
    project.panel_layout.place("K1/coil", at, 300.0, 300.0);
    project.panel_layout.place("K1_1", at, 300.0, 200.0);
    let before = document(&project);
    let aux = before.coil_at["K1_1"].clone();
    let coil_sheet = before.coil_at["K1"].sheet;
    assert!(before.sheets[coil_sheet - 1]
        .body
        .contains(&quoted(&format!("13-14 {aux}"))));
    project.panel_layout.place("K1_1", at, 900.0, 200.0);
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
    let at = after_power(&project);
    project.panel_layout.place("K1/coil", at, 300.0, 300.0);
    let doc = document(&project);
    assert_eq!(control_symbol_ports(&doc, "K1/coil"), None);
    assert!(control_symbol_ports(&doc, "K1").is_some());
}

/// A part key that is not a part of the device (`K1/foo`) draws nothing and names no place.
#[test]
fn an_unknown_part_put_by_hand_draws_nothing() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    let at = after_power(&project);
    project.panel_layout.place("K1/foo", at, 300.0, 300.0);
    let doc = document(&project);
    assert!(!symbol_drawn(&doc, "K1/foo"));
    assert!(!doc.coil_at.contains_key("K1"));
}

/// A coil put by hand on a motor (`M1/coil`) is not drawn: only contactors and relays have one.
#[test]
fn a_coil_put_by_hand_on_a_motor_draws_nothing() {
    let mut project = pump(FREE_CONTACTOR_PANEL);
    let at = after_power(&project);
    project.panel_layout.place("M1/coil", at, 300.0, 300.0);
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
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
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

/// The two rails of the control sheet: the `+24V` rail (the upper one) and the `0V` rail (the
/// lower one), the long horizontal lines of the sheet.
fn rails(body: &str) -> (f32, f32) {
    let mut ys: Vec<f32> = body
        .lines()
        .filter(|l| l.starts_with("W("))
        .filter_map(|l| {
            // The width, then the two ends of the line.
            let n = call_numbers(l);
            (n.len() == 5 && n[2] == n[4] && (n[1] - n[3]).abs() > 200.0).then_some(n[2])
        })
        .collect();
    ys.sort_by(f32::total_cmp);
    assert_eq!(ys.len(), 2, "две шины: {ys:?}");
    (ys[0], ys[1])
}

/// The origin of the symbol of `device` on the view `view`, if the sheets draw it.
fn symbol_origin(doc: &Document, view: &str, device: &str) -> Option<(f32, f32)> {
    doc.sheets
        .iter()
        .flat_map(|s| &s.items)
        .find_map(|i| match &i.kind {
            ItemKind::Symbol {
                view: v,
                device: d,
                origin,
                ..
            } if *v == view && d == device => Some(*origin),
            _ => None,
        })
}

/// The port names of the symbol of `device` on the view `view`, if the sheets draw it.
fn ports_of(doc: &Document, view: &str, device: &str) -> Option<Vec<String>> {
    doc.sheets
        .iter()
        .flat_map(|s| &s.items)
        .find_map(|i| match &i.kind {
            ItemKind::Symbol {
                view: v,
                device: d,
                ports,
                ..
            } if *v == view && d == device => Some(ports.iter().map(|p| p.name.clone()).collect()),
            _ => None,
        })
}

/// Whether a wire runs from the port `from` (device, port) to the port `to` on the sheets.
fn has_wire_end(doc: &Document, from: (&str, &str), to: (&str, &str)) -> bool {
    doc.sheets.iter().flat_map(|s| &s.items).any(|i| {
        matches!(&i.kind, ItemKind::Wire { from: f, to: t, .. }
            if (f.device.as_str(), f.port.as_str()) == from
                && (t.device.as_str(), t.port.as_str()) == to)
    })
}

/// The title of the first control sheet.
fn control_title(doc: &Document) -> &str {
    &doc.sheets
        .iter()
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
        .expect("лист управления")
        .title
}

/// The terminal of the board the device is bound to.
fn bound_terminal(nets: &crab_schematic::Nets, device: &str) -> String {
    nets.board_terminals
        .iter()
        .find(|t| t.device == device)
        .unwrap_or_else(|| panic!("{device} is not bound to the board"))
        .terminal
        .clone()
}

/// The control sheet is a ladder: the `+24V` rail at the top, the `0V` rail at the bottom, the
/// devices between them, and the board is its channels (no board rectangle).
#[test]
fn a_control_sheet_is_a_ladder() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let body = control_body(&doc);
    let (plus, zero) = rails(body);
    assert!(plus < zero, "+24V {plus} выше 0V {zero}");
    let sheet = doc
        .sheets
        .iter()
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
        .expect("лист управления");
    for item in &sheet.items {
        if let ItemKind::Symbol {
            view: "control",
            device,
            origin,
            ..
        } = &item.kind
        {
            assert!(
                origin.1 > plus && origin.1 < zero,
                "{device} на высоте {} между шинами {plus} и {zero}",
                origin.1
            );
        }
    }
    assert!(
        symbol_origin(&doc, "board", "A1").is_none(),
        "нет платы-прямоугольника"
    );
    assert!(!nets.board_terminals.is_empty());
    for t in &nets.board_terminals {
        let channel = format!("A1/{}", t.terminal);
        assert_eq!(
            ports_of(&doc, "board", &channel),
            Some(vec![t.terminal.clone()]),
            "канал {channel}"
        );
    }
}

/// An output: its channel stands under the `+24V` rail, not on it, and the wire runs down from the
/// channel to the coil, which stands above the `0V` rail.
#[test]
fn an_output_feeds_its_coil_from_a_channel_at_the_top() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let terminal = bound_terminal(&nets, "K1");
    let (plus, zero) = rails(control_body(&doc));
    let (_, channel_y) = symbol_origin(&doc, "board", &format!("A1/{terminal}")).expect("канал");
    assert!(
        channel_y > plus && channel_y < plus + 60.0,
        "канал на {channel_y}, +24V на {plus}"
    );
    let (_, coil_y) = symbol_origin(&doc, "control", "K1").expect("катушка");
    assert!(coil_y > channel_y && coil_y < zero, "катушка на {coil_y}");
    assert!(has_wire_end(&doc, ("A1", &terminal), ("K1", "signal")));
}

/// An input: its channel stands above the `0V` rail and is not joined to it; the device feeds it
/// from above.
#[test]
fn an_input_channel_stands_above_the_zero_rail() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let terminal = bound_terminal(&nets, "B1");
    let (_, zero) = rails(control_body(&doc));
    let (_, top) = symbol_origin(&doc, "board", &format!("A1/{terminal}")).expect("канал");
    // The box is 28 high and stands 10 or more above the rail.
    assert!(
        top + 28.0 <= zero - 10.0,
        "канал до {} над 0V {zero}",
        top + 28.0
    );
    assert!(zero - top < 80.0, "канал на {top}, далеко от 0V {zero}");
    // The sensor feeds its channel: its signal wire ends there.
    assert!(
        has_wire_end(&doc, ("B1", "signal"), ("A1", &terminal))
            || has_wire_end(&doc, ("A1", &terminal), ("B1", "signal"))
    );
}

/// Motor columns: the coil that switches M1 and the sensor of M1 stand next to each other, and the
/// sheet that holds only them is named after the motor. With another load on the sheet, its name
/// is the default one.
#[test]
fn rows_of_one_motor_stand_together_and_name_the_sheet() {
    const MOTOR: &str = r#"
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
id = "B1"
kind = "sensor"
bind = "sensor.out"
of = "M1"
"#;
    let (doc, nets) = build_document(&pump(MOTOR), &library(), Lang::Ru).unwrap();
    let mut columns: Vec<(f32, String)> = nets
        .board_terminals
        .iter()
        .map(|t| {
            let (x, _) =
                symbol_origin(&doc, "board", &format!("A1/{}", t.terminal)).expect("канал");
            (x, t.device.clone())
        })
        .collect();
    columns.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<&str> = columns.iter().map(|c| c.1.as_str()).collect();
    let k1 = order.iter().position(|d| *d == "K1").expect("K1");
    let b1 = order.iter().position(|d| *d == "B1").expect("B1");
    assert_eq!(k1.abs_diff(b1), 1, "столбцы: {order:?}");
    assert_eq!(control_title(&doc), "Управление -M1");

    // Another load on the sheet: the default name, and the motor's columns stay together.
    let mut project = pump(&format!(
        "{MOTOR}\n[[device]]\nid = \"Q2\"\nkind = \"breaker\"\nload = \"K2\"\n\
         [[device]]\nid = \"K2\"\nkind = \"contactor\"\nbind = \"out2\"\nload = \"G1\"\n\
         [[device]]\nid = \"G1\"\nkind = \"psu\"\n"
    ));
    project
        .logic
        .blocks
        .push(toml::from_str("id = \"out2\"\ntype = \"digital_out\"\npin = 27\n").unwrap());
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    let mut columns: Vec<(f32, String)> = nets
        .board_terminals
        .iter()
        .map(|t| {
            let (x, _) =
                symbol_origin(&doc, "board", &format!("A1/{}", t.terminal)).expect("канал");
            (x, t.device.clone())
        })
        .collect();
    columns.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<&str> = columns.iter().map(|c| c.1.as_str()).collect();
    let k1 = order.iter().position(|d| *d == "K1").expect("K1");
    let b1 = order.iter().position(|d| *d == "B1").expect("B1");
    assert_eq!(k1.abs_diff(b1), 1, "столбцы: {order:?}");
    assert_eq!(control_title(&doc), "Цепи управления");
}

/// Every column names its tag under the rails: a movable caption per device.
#[test]
fn column_captions_name_the_tags() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let body = control_body(&doc);
    let sheet = doc
        .sheets
        .iter()
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
        .expect("лист управления");
    for t in &nets.board_terminals {
        assert!(body.contains(&quoted(&t.tag)), "нет подписи {}", t.tag);
        let key = crab_core::panel_layout::label_key(&t.device, "column");
        let found = sheet
            .items
            .iter()
            .any(|i| matches!(&i.kind, ItemKind::Label { key: k } if *k == key));
        assert!(found, "нет подписи столбца {key}");
    }
}

/// A hand wire from a terminal of the board (`A1:IO5 -> K9:A1`) joins the channel box of the
/// terminal, which is drawn in its column.
#[test]
fn a_hand_wire_from_a_terminal_joins_its_channel() {
    let (_, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let terminal = bound_terminal(&nets, "K1");
    let mut project = pump(&format!(
        "{PUMP_PANEL}\n[[device]]\nid = \"K9\"\nkind = \"contactor\"\nfree = true\n\
         [[link]]\nfrom = \"A1:{terminal}\"\nto = \"K9:A1\"\n"
    ));
    project
        .panel_layout
        .place("K9/coil", SheetId::Control(0), 900.0, 300.0);
    let doc = document(&project);
    assert!(symbol_origin(&doc, "board", &format!("A1/{terminal}")).is_some());
    assert!(has_wire_end(&doc, ("A1", &terminal), ("K9", "A1")));
}

/// The contact table under a coil sits under the `0V` rail, above the captions of the columns.
/// A coil with six auxiliary contacts has seven lines.
#[test]
fn a_contact_table_sits_under_the_zero_rail_above_the_captions() {
    let doc = document(&pump(&coil_panel(6)));
    let body = control_body(&doc);
    let (_, zero) = rails(body);
    let lines = table_ys(body);
    assert_eq!(lines.len(), 7, "строки таблицы: {lines:?}");
    let first = lines.iter().copied().fold(f32::MAX, f32::min);
    let last = lines.iter().copied().fold(f32::MIN, f32::max);
    assert!(first > zero, "таблица на {first}, над 0V {zero}");
    // The captions stand 30 above the bottom of the body, 12 high.
    assert!(
        last < doc.page.y1() - 30.0 - 12.0,
        "таблица до {last}, подписи ниже"
    );
}

/// The `+24V` and `0V` rails of the control sheet of a document: the start and the end of each.
fn rail_spans(body: &str) -> Vec<(f32, f32)> {
    body.lines()
        .filter(|l| l.starts_with("W("))
        .filter_map(|l| {
            let n = call_numbers(l);
            (n.len() == 5 && n[2] == n[4] && (n[1] - n[3]).abs() > 200.0)
                .then(|| (n[1].min(n[3]), n[1].max(n[3])))
        })
        .collect()
}

/// The rails run from the left of the body to the right edge (less the room of the reference of
/// the next sheet), on a sheet with two columns.
#[test]
fn the_rails_run_to_the_right_edge_of_the_sheet() {
    let doc = document(&pump(PUMP_PANEL));
    let spans = rail_spans(control_body(&doc));
    assert_eq!(spans.len(), 2, "две шины: {spans:?}");
    for (start, end) in spans {
        assert!(start <= doc.page.x0() + 60.0, "шина с {start}");
        assert!(
            end >= doc.page.x1() - 80.0,
            "шина до {end}, край листа {}",
            doc.page.x1()
        );
    }
}

/// The text of a channel (its tag and `-A1:IO5`) stays at least 6 from the terminal of the box, on
/// the top edge of an input and on the bottom edge of an output.
#[test]
fn channel_text_keeps_clear_of_its_terminal() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let mut checked = 0;
    for sheet in &doc.sheets {
        for item in &sheet.items {
            let ItemKind::Symbol {
                view: "board",
                device,
                ports,
                ..
            } = &item.kind
            else {
                continue;
            };
            let Some(port) = ports.first() else { continue };
            let (ox, oy) = symbol_origin(&doc, "board", device).expect("канал");
            let terminal = device.trim_start_matches("A1/");
            let tag = &nets
                .board_terminals
                .iter()
                .find(|t| t.terminal == terminal)
                .expect("клемма платы")
                .tag;
            let name = format!("-A1:{terminal}");
            for op in &sheet.ops {
                // The texts of the channel: the captions of the columns stand far from it.
                if let Op::Text { x, y, text, .. } = op {
                    let near = (x - ox).abs() < 40.0 && (y - oy).abs() < 40.0;
                    if near && (text == tag || *text == name) {
                        assert!(
                            (y - port.y).abs() >= 6.0,
                            "«{text}» на {y}, клемма {terminal} на {}",
                            port.y
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 2 * nets.board_terminals.len());
}

/// The terminals of a sensor: `+` at the top (on the `+24V` rail), `S` at the bottom left (the
/// signal) and `-` at the bottom right.
#[test]
fn sensor_terminals_are_named_by_their_place() {
    let doc = document(&pump(PUMP_PANEL));
    let (sx, sy) = symbol_origin(&doc, "control", "B1").expect("датчик");
    let texts: Vec<(&str, f32, f32)> = doc
        .sheets
        .iter()
        .flat_map(|s| &s.ops)
        .filter_map(|op| match op {
            Op::Text { x, y, text, .. }
                if matches!(text.as_str(), "+" | "S" | "-")
                    && (x - sx).abs() < 40.0
                    && (y - sy).abs() < 70.0 =>
            {
                Some((text.as_str(), *x, *y))
            }
            _ => None,
        })
        .collect();
    let at = |t: &str| {
        texts
            .iter()
            .find(|(s, _, _)| *s == t)
            .map(|(_, x, y)| (*x, *y))
    };
    let (px, py) = at("+").expect("нет «+»");
    assert!(
        px < sx && py < sy + 10.0,
        "+ на ({px}, {py}), датчик ({sx}, {sy})"
    );
    let (zx, zy) = at("S").expect("нет «S»");
    assert!(zx < sx && zy > sy + 25.0, "S на ({zx}, {zy})");
    let (mx, my) = at("-").expect("нет «-»");
    assert!(mx > sx && my > sy + 25.0, "- на ({mx}, {my})");
    assert_eq!(texts.len(), 3, "знаки датчика: {texts:?}");
}

/// Five contacts in series with a coil stand between its channel and the coil, one above
/// another, none over another.
#[test]
fn series_contacts_fit_between_the_channel_and_the_coil() {
    let mut panel = PUMP_PANEL.to_owned();
    for i in 1..=5 {
        panel.push_str(&format!(
            "\n[[device]]\nid = \"K1_{i}\"\nkind = \"aux\"\nof = \"K1\"\nseries = \"K1\"\n"
        ));
    }
    let (doc, nets) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
    let terminal = bound_terminal(&nets, "K1");
    let (_, channel_top) = symbol_origin(&doc, "board", &format!("A1/{terminal}")).expect("канал");
    // The channel is 34 high: its terminal is on the bottom edge. A contact is 50 high.
    let channel_bottom = channel_top + 34.0;
    let (_, coil_top) = symbol_origin(&doc, "control", "K1").expect("катушка");
    let mut tops: Vec<f32> = (1..=5)
        .map(|i| {
            symbol_origin(&doc, "control", &format!("K1_{i}"))
                .expect("контакт")
                .1
        })
        .collect();
    tops.sort_by(f32::total_cmp);
    for y in &tops {
        assert!(
            *y >= channel_bottom && y + 50.0 <= coil_top,
            "контакт {y} вне {channel_bottom}..{coil_top}"
        );
    }
    for pair in tops.windows(2) {
        assert!(
            pair[1] - pair[0] >= 50.0,
            "контакты перекрываются: {tops:?}"
        );
    }
}

/// A contact table has at most eight lines: when there are more, the eighth is `+N`, the lines
/// not shown.
#[test]
fn a_long_contact_table_ends_with_the_number_of_lines_not_shown() {
    let doc = document(&pump(&coil_panel(9)));
    let body = control_body(&doc);
    // The pole line and nine contacts: ten lines, seven shown and `+3`.
    assert_eq!(
        table_ys(body).len(),
        7,
        "строки таблицы: {:?}",
        table_ys(body)
    );
    assert!(body.contains(&quoted("+3")), "нет строки «+3»");
}

/// The note under the `0V` rail is not over the table of the first column: its text range and the
/// ranges of the table lines do not overlap.
#[test]
fn the_supply_note_does_not_cover_the_first_table() {
    let doc = document(&pump(PUMP_PANEL));
    let sheet = doc
        .sheets
        .iter()
        .find(|s| matches!(s.id, Some(SheetId::Control(_))))
        .expect("лист управления");
    // The text box of a line: its width is half its size per character, from the anchor.
    let boxed = |x: f32, text: &str, size: f32, align: Align| {
        let w = text.chars().count() as f32 * 0.5 * size;
        match align {
            Align::Left => (x, x + w),
            Align::Right => (x - w, x),
            Align::Center => (x - w / 2.0, x + w / 2.0),
        }
    };
    let mut note = Vec::new();
    let mut table = Vec::new();
    for op in &sheet.ops {
        if let Op::Text {
            x,
            text,
            size,
            align,
            ..
        } = op
        {
            if text.starts_with("Питание") || text.starts_with("-G1") {
                note.push(boxed(*x, text, *size, *align));
            } else if text.starts_with(|c: char| c.is_ascii_digit()) && text.contains('-') {
                table.push(boxed(*x, text, *size, *align));
            }
        }
    }
    assert!(
        !note.is_empty() && !table.is_empty(),
        "note {note:?}, table {table:?}"
    );
    for (a0, a1) in &note {
        for (b0, b1) in &table {
            assert!(
                a1 <= b0 || b1 <= a0,
                "заметка {a0}..{a1} над таблицей {b0}..{b1}"
            );
        }
    }
}
