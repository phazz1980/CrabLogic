//! Hand wires between sheets and to the supply lines (`specs/panel-palette`, tasks 3 and 4): a wire
//! whose other end is on another sheet breaks off with the place of that end, a wire to a bus ends
//! on the line of the sheet that draws it, or breaks off with the name of the line.

use std::path::PathBuf;

use crab_core::{Library, PanelLayout, Project, SheetId};
use crab_schematic::{build_document, Document, Item, ItemKind, Lang, Nets, Op, Sheet};

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

/// A sensor on the board (it makes a control sheet) and two contactors that are not in a power
/// chain. Each test adds its links and puts its coils by hand.
const PANEL: &str = r#"
supply = "ac230"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor"

[[device]]
id = "K1"
kind = "contactor"
free = true

[[device]]
id = "K2"
kind = "contactor"
free = true
"#;

/// The project of the pump example with the panel `panel`, the given links and the coils put by
/// hand (`(key, sheet)`).
fn project_in(panel: &str, links: &str, coils: &[(&str, SheetId)]) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str(&format!("{panel}\n{links}")).unwrap();
    project.panel_layout = PanelLayout::default();
    for (key, sheet) in coils {
        project.panel_layout.place(key, *sheet, 300.0, 300.0);
    }
    project
}

/// [`project_in`] with the panel `PANEL`.
fn project(links: &str, coils: &[(&str, SheetId)]) -> Project {
    project_in(PANEL, links, coils)
}

/// A feed, a breaker with its motor (a power sheet), a breaker and a contactor that are not in a
/// power chain (put by hand). `supply` is the supply of the panel.
fn chain(supply: &str) -> String {
    format!(
        r#"
supply = "{supply}"

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

[[device]]
id = "Q8"
kind = "breaker"
free = true

[[device]]
id = "K1"
kind = "contactor"
free = true
"#
    )
}

fn document(project: &Project) -> Document {
    build_document(project, &library(), Lang::Ru).unwrap().0
}

/// The sheet with the document number `number` (the title sheet is 1).
fn sheet(doc: &Document, number: usize) -> &Sheet {
    &doc.sheets[number - 1]
}

/// The document number of the drawing sheet `id`.
fn number_of(doc: &Document, id: SheetId) -> usize {
    doc.sheets
        .iter()
        .position(|s| s.id == Some(id))
        .map(|i| i + 1)
        .unwrap_or_else(|| panic!("no sheet {id:?}"))
}

/// The item of the wire `key` on the sheet, if it is drawn there.
fn wire<'a>(sheet: &'a Sheet, key: &str) -> Option<&'a Item> {
    sheet
        .items
        .iter()
        .find(|i| matches!(&i.kind, ItemKind::Wire { key: k, .. } if k == key))
}

/// The end points of the lines of the wire `key` on the sheet.
fn wire_ends(sheet: &Sheet, key: &str) -> Vec<(f32, f32)> {
    wire(sheet, key)
        .into_iter()
        .flat_map(|item| item.ops.iter().map(|&i| &sheet.ops[i]))
        .filter_map(|op| match op {
            Op::Line { points, .. } => points.last().copied(),
            _ => None,
        })
        .collect()
}

/// Whether the sheet writes the text `text`: a break mark's reference or the name of a line. The
/// texts of a sheet are items of their own (labels the user can move), not parts of the wire.
fn says(sheet: &Sheet, text: &str) -> bool {
    sheet
        .ops
        .iter()
        .any(|op| matches!(op, Op::Text { text: t, .. } if t == text))
}

#[test]
fn a_wire_to_another_sheet_breaks_off_with_the_place_of_its_other_end() {
    let project = project(
        "[[link]]\nfrom = \"K1:A1\"\nto = \"B1:S\"\n",
        &[("K1/coil", SheetId::Free(0))],
    );
    let doc = document(&project);
    let key = "K1:A1>B1:S";
    let free = number_of(&doc, SheetId::Free(0));
    let control = number_of(&doc, SheetId::Control(0));
    // The sensor's terminal is drawn on the control sheet, the coil's on the free one.
    let there = &doc.terminal_at[&("B1".to_owned(), "S".to_owned())];
    let back = &doc.terminal_at[&("K1".to_owned(), "A1".to_owned())];
    assert_eq!(there.sheet, control);
    assert_eq!(back.sheet, free);
    assert!(wire(sheet(&doc, free), key).is_some());
    assert!(wire(sheet(&doc, control), key).is_some());
    assert!(says(sheet(&doc, free), &there.to_string()));
    assert!(says(sheet(&doc, control), &back.to_string()));
}

#[test]
fn a_wire_to_a_bus_ends_on_the_rail_of_the_sheet_that_draws_it() {
    let project = project(
        "[[link]]\nfrom = \"K2:A2\"\nto = \"bus:0V\"\n",
        &[("K2/coil", SheetId::Control(0))],
    );
    let doc = document(&project);
    let key = "K2:A2>bus:0V";
    let on = sheet(&doc, number_of(&doc, SheetId::Control(0)));
    // The `0V` rail stands 180 above the bottom of the body (`ZERO_RAIL_DY`).
    let rail_y = doc.page.y1() - 180.0;
    let ends = wire_ends(on, key);
    assert!(
        ends.iter().any(|&(_, y)| (y - rail_y).abs() < 0.01),
        "{ends:?} must end on the rail at {rail_y}"
    );
    // Only the vertical wire: no stub with a break mark.
    assert_eq!(ends.len(), 1, "{ends:?}");
}

#[test]
fn a_wire_to_a_bus_breaks_off_with_the_name_of_the_line_where_it_is_not_drawn() {
    let project = project(
        "[[link]]\nfrom = \"K1:A2\"\nto = \"bus:0V\"\n",
        &[("K1/coil", SheetId::Free(0))],
    );
    let doc = document(&project);
    let key = "K1:A2>bus:0V";
    let on = sheet(&doc, number_of(&doc, SheetId::Free(0)));
    assert!(wire(on, key).is_some());
    assert!(says(on, "0V"));
    // The line is not drawn here: no place of another sheet is written.
    assert!(!on
        .ops
        .iter()
        .any(|op| matches!(op, Op::Text { text, .. } if text.starts_with('/'))));
}

#[test]
fn the_wire_list_names_a_bus_end_by_its_line() {
    let project = project(
        "[[link]]\nfrom = \"K1:A2\"\nto = \"bus:0V\"\n",
        &[("K1/coil", SheetId::Free(0))],
    );
    let doc = document(&project);
    let row = doc
        .tables
        .wires
        .rows
        .iter()
        .find(|row| row[2] == "K1:A2")
        .expect("the wire is in the list");
    assert_eq!(row[3], "0V");
}

#[test]
fn a_bus_that_no_line_of_the_supply_has_is_no_wire() {
    let project = project(
        "[[link]]\nfrom = \"K1:A2\"\nto = \"bus:X9\"\n",
        &[("K1/coil", SheetId::Free(0))],
    );
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    let key = "K1:A2>bus:X9";
    assert!(nets.wires.iter().all(|w| w.key() != key));
    assert!(doc.sheets.iter().all(|s| wire(s, key).is_none()));
}

/// The end points of the wire `key`: the pairs of points of its lines.
fn wire_segments(sheet: &Sheet, key: &str) -> Vec<((f32, f32), (f32, f32))> {
    wire(sheet, key)
        .into_iter()
        .flat_map(|item| item.ops.iter().map(|&i| &sheet.ops[i]))
        .filter_map(|op| match op {
            Op::Line { points, .. } => Some(points.clone()),
            _ => None,
        })
        .flat_map(|points| points.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>())
        .collect()
}

#[test]
fn a_wire_to_a_terminal_of_the_power_sheet_breaks_off_with_its_place() {
    let project = project_in(
        &chain("ac230"),
        "[[link]]\nfrom = \"K1:A1\"\nto = \"Q1:1\"\n",
        &[("K1/coil", SheetId::Free(0))],
    );
    let doc = document(&project);
    let key = "K1:A1>Q1:1";
    let power = number_of(&doc, SheetId::Power(0));
    let free = number_of(&doc, SheetId::Free(0));
    let there = &doc.terminal_at[&("Q1".to_owned(), "1".to_owned())];
    let back = &doc.terminal_at[&("K1".to_owned(), "A1".to_owned())];
    assert_eq!(there.sheet, power);
    assert_eq!(back.sheet, free);
    assert!(wire(sheet(&doc, power), key).is_some());
    assert!(says(sheet(&doc, free), &there.to_string()));
    assert!(says(sheet(&doc, power), &back.to_string()));
}

#[test]
fn a_wire_to_a_supply_line_ends_on_its_line_with_a_junction() {
    let mut project = project_in(
        &chain("ac400"),
        "[[link]]\nfrom = \"Q8:1\"\nto = \"bus:L1\"\n",
        &[],
    );
    // The lines of the sheet end under the devices of the chain: the breaker stands over the lines
    // (its terminal at x = 200), not beyond their ends.
    project
        .panel_layout
        .place("Q8", SheetId::Power(0), 130.0, 300.0);
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    let key = "Q8:1>bus:L1";
    let on = sheet(&doc, number_of(&doc, SheetId::Power(0)));
    // The vertical wire from the breaker's terminal, down or up to the line.
    let segments = wire_segments(on, key);
    let &(start, end) = segments
        .iter()
        .find(|(a, b)| a.0 == b.0 && a.1 != b.1)
        .unwrap_or_else(|| panic!("no vertical wire in {segments:?}"));
    let (x, y) = end;
    // The line `L1` runs through that point: a horizontal segment at `y` covers `x`.
    let on_line = on.ops.iter().any(|op| match op {
        Op::Line { points, .. } => points.windows(2).any(|w| {
            (w[0].1 - y).abs() < 0.01
                && (w[1].1 - y).abs() < 0.01
                && w[0].0.min(w[1].0) <= x
                && x <= w[0].0.max(w[1].0)
        }),
        _ => false,
    });
    assert!(on_line, "the wire from {start:?} must end on a line at {y}");
    // The junction dot where the wire meets the line.
    assert!(on.ops.iter().any(|op| matches!(
        op,
        Op::Dot { x: dx, y: dy, .. } if (dx - x).abs() < 0.01 && (dy - y).abs() < 0.01
    )));
    // The wire carries the number of the L1 conductor of the feed.
    let line = nets
        .wires
        .iter()
        .find(|w| w.from.device == "X1" && w.from.terminal == "L1")
        .expect("the feed's L1 conductor");
    let hand = nets
        .wires
        .iter()
        .find(|w| w.key() == key)
        .expect("hand wire");
    assert_eq!(hand.number, line.number);
}

#[test]
fn a_wire_to_the_0v_line_has_the_number_of_the_automatic_0v_wires() {
    // The pump's contactor `K1` has its coil wired to the 0 V line by the generator.
    let mut text = std::fs::read_to_string(repo_path("examples/pump/panel.toml")).unwrap();
    text.push_str(
        "\n[[device]]\nid = \"K9\"\nkind = \"contactor\"\nfree = true\n\n\
         [[link]]\nfrom = \"K9:A2\"\nto = \"bus:0V\"\n",
    );
    let project = project_in(&text, "", &[("K9/coil", SheetId::Control(0))]);
    let (_, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    let auto = nets
        .wires
        .iter()
        .find(|w| w.from.device == "K1" && w.from.terminal == "A2")
        .expect("the coil of K1 is wired to 0 V");
    assert_eq!(auto.to.terminal, "0V");
    let hand = nets
        .wires
        .iter()
        .find(|w| w.key() == "K9:A2>bus:0V")
        .expect("the hand wire");
    assert_eq!(hand.number, auto.number);
}

/// The wires that are not hand wires to a supply line, by key and number, sorted.
fn automatic_wires(nets: &Nets) -> Vec<(String, u32)> {
    let mut wires: Vec<(String, u32)> = nets
        .wires
        .iter()
        .filter(|w| w.from.device != "bus" && w.to.device != "bus")
        .map(|w| (w.key(), w.number))
        .collect();
    wires.sort();
    wires
}

/// The symbols of the power sheets with their places.
fn power_symbols(doc: &Document) -> Vec<(usize, String, (f32, f32))> {
    let mut found = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for item in &sheet.items {
            if let ItemKind::Symbol {
                view: "power",
                device,
                origin,
                ..
            } = &item.kind
            {
                found.push((index, device.clone(), *origin));
            }
        }
    }
    found
}

#[test]
fn a_bus_link_on_a_chained_terminal_leaves_the_chain_as_it_is() {
    let base = project_in(&chain("ac400"), "", &[]);
    let linked = project_in(
        &chain("ac400"),
        "[[link]]\nfrom = \"Q1:1\"\nto = \"bus:L1\"\n",
        &[],
    );
    let (base_doc, base_nets) = build_document(&base, &library(), Lang::Ru).unwrap();
    let (doc, nets) = build_document(&linked, &library(), Lang::Ru).unwrap();
    // The automatic wires keep their numbers, the feed's L1 wire to the breaker among them.
    assert_eq!(automatic_wires(&base_nets), automatic_wires(&nets));
    // The devices of the chain stand where they stood.
    assert_eq!(power_symbols(&base_doc), power_symbols(&doc));
    // The hand wire joins the same potential: the number of the L1 conductor of the feed.
    let feed = nets
        .wires
        .iter()
        .find(|w| w.from.device == "X1" && w.from.terminal == "L1")
        .expect("the feed's L1 conductor");
    let hand = nets
        .wires
        .iter()
        .find(|w| w.key() == "Q1:1>bus:L1")
        .expect("the hand wire");
    assert_eq!(hand.number, feed.number);
}
