//! The manual layout of the sheets (`specs/panel-editor`, task 4): moved symbols, bent wires,
//! references that follow the place.

use std::path::PathBuf;

use crab_core::{symbol_key, Library, Project};
use crab_schematic::doc::typst_source;
use crab_schematic::{build_document, Document, ItemKind, Lang, Nets, Op, WireClass};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn setup() -> (Library, Project) {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    (lib, Project::load(repo_path("examples/pump")).unwrap())
}

fn build(project: &Project, lib: &Library) -> (Document, Nets) {
    build_document(project, lib, Lang::Ru).unwrap()
}

/// The origin of the symbol of `device` on `view`.
fn origin(doc: &Document, view: &str, device: &str) -> (f32, f32) {
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
        .unwrap_or_else(|| panic!("no {view} symbol of {device}"))
}

/// The points of the line of the first item of the wire `key`.
fn wire_points(doc: &Document, key: &str) -> Vec<(f32, f32)> {
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
    panic!("no wire {key}");
}

fn power_wire(nets: &Nets, from: &str, to: &str) -> String {
    nets.wires
        .iter()
        .find(|w| w.class == WireClass::Power && w.from.device == from && w.to.device == to)
        .unwrap_or_else(|| panic!("no wire {from} > {to}"))
        .key()
}

#[test]
fn a_moved_symbol_stands_where_it_was_put() {
    let (lib, mut project) = setup();
    let (before, _) = build(&project, &lib);
    let (x, y) = origin(&before, "power", "K1");
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 40.0, 25.0);
    let (after, _) = build(&project, &lib);
    assert_eq!(origin(&after, "power", "K1"), (x + 40.0, y + 25.0));
    // The others stay.
    assert_eq!(
        origin(&after, "power", "Q1"),
        origin(&before, "power", "Q1")
    );
}

#[test]
fn a_wire_to_a_moved_symbol_is_orthogonal() {
    let (lib, mut project) = setup();
    let (before, nets) = build(&project, &lib);
    let key = power_wire(&nets, "Q1", "K1");
    assert_eq!(wire_points(&before, &key).len(), 2);
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 40.0, 0.0);
    let (after, _) = build(&project, &lib);
    let points = wire_points(&after, &key);
    assert_eq!(points.len(), 4, "{points:?}");
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        assert!(a.0 == b.0 || a.1 == b.1, "not orthogonal: {points:?}");
    }
}

#[test]
fn the_bend_of_a_wire_sets_the_height_of_its_crossing() {
    let (lib, mut project) = setup();
    let (_, nets) = build(&project, &lib);
    let key = power_wire(&nets, "Q1", "K1");
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 40.0, 0.0);
    let (auto, _) = build(&project, &lib);
    let auto_y = wire_points(&auto, &key)[1].1;
    project.panel_layout.set_bend(&key, Some(auto_y + 20.0));
    let (bent, _) = build(&project, &lib);
    assert_eq!(wire_points(&bent, &key)[1].1, auto_y + 20.0);
}

#[test]
fn references_follow_the_place_of_the_symbols() {
    let (lib, mut project) = setup();
    let (before, _) = build(&project, &lib);
    // A whole column to the right (about 36 mm).
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 300.0, 0.0);
    let (after, _) = build(&project, &lib);
    let (a, b) = (
        before.power_at.get("K1").unwrap(),
        after.power_at.get("K1").unwrap(),
    );
    assert_ne!(a.zone, b.zone);
    // The coil on the control sheet names the contacts where they are drawn now.
    let text = typst_source(&after, Lang::Ru);
    assert!(text.contains(&b.to_string()), "{b}");
}

#[test]
fn a_moved_device_of_the_control_sheet_has_an_orthogonal_signal_wire() {
    let (lib, mut project) = setup();
    let (before, nets) = build(&project, &lib);
    let signal = nets
        .wires
        .iter()
        .find(|w| w.class == WireClass::Signal && w.to.device == "B1")
        .or_else(|| {
            nets.wires
                .iter()
                .find(|w| w.class == WireClass::Signal && w.from.device == "B1")
        })
        .expect("a signal wire of B1")
        .key();
    assert_eq!(wire_points(&before, &signal).len(), 2);
    let (x, _) = origin(&before, "control", "B1");
    project
        .panel_layout
        .set_shift(&symbol_key("control", "B1"), 50.0, 0.0);
    let (after, _) = build(&project, &lib);
    assert_eq!(origin(&after, "control", "B1").0, x + 50.0);
    assert_eq!(wire_points(&after, &signal).len(), 4);
}

#[test]
fn a_symbol_never_leaves_the_frame() {
    let (lib, mut project) = setup();
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 100_000.0, 100_000.0);
    project
        .panel_layout
        .set_shift(&symbol_key("control", "B1"), -100_000.0, 100_000.0);
    let (doc, _) = build(&project, &lib);
    let page = &doc.page;
    for (view, device) in [("power", "K1"), ("control", "B1")] {
        let (x, y) = origin(&doc, view, device);
        assert!(x >= page.x0() && x <= page.x1(), "{view} {device} x={x}");
        assert!(y >= page.y0() && y <= page.y1(), "{view} {device} y={y}");
    }
}

#[test]
fn the_same_layout_gives_the_same_document() {
    let (lib, mut project) = setup();
    project
        .panel_layout
        .set_shift(&symbol_key("power", "K1"), 40.0, 10.0);
    project
        .panel_layout
        .set_shift(&symbol_key("control", "K1"), -30.0, 0.0);
    let one = typst_source(&build(&project, &lib).0, Lang::Ru);
    let two = typst_source(&build(&project, &lib).0, Lang::Ru);
    assert_eq!(one, two);
}

#[test]
fn keys_of_devices_that_are_not_drawn_do_nothing() {
    let (lib, mut project) = setup();
    let plain = typst_source(&build(&project, &lib).0, Lang::Ru);
    project
        .panel_layout
        .set_shift(&symbol_key("power", "NOPE"), 40.0, 10.0);
    let with = typst_source(&build(&project, &lib).0, Lang::Ru);
    assert_eq!(plain, with);
}

/// The place of the text of the label `key` (the first operation of its item).
fn label_at(doc: &Document, key: &str) -> Option<(f32, f32)> {
    doc.sheets.iter().find_map(|s| {
        s.items.iter().find_map(|i| match &i.kind {
            ItemKind::Label { key: k } if k == key => match s.ops.get(*i.ops.first()?)? {
                Op::Text { x, y, .. } => Some((*x, *y)),
                _ => None,
            },
            _ => None,
        })
    })
}

#[test]
fn a_label_moves_by_its_shift_and_the_rest_stays() {
    let (lib, mut project) = setup();
    let (before, _) = build(&project, &lib);
    let name = crab_core::panel_layout::label_key("Q1", "name");
    let other = crab_core::panel_layout::label_key("Q1", "rating");
    let (x, y) = label_at(&before, &name).expect("the designation is a label");
    let rating = label_at(&before, &other);
    project.panel_layout.set_shift(&name, 30.0, -10.0);
    let (after, _) = build(&project, &lib);
    assert_eq!(label_at(&after, &name), Some((x + 30.0, y - 10.0)));
    assert_eq!(label_at(&after, &other), rating);
    // The symbol itself did not move.
    assert_eq!(
        origin(&before, "power", "Q1"),
        origin(&after, "power", "Q1")
    );
}

#[test]
fn the_number_of_a_wire_is_one_line_and_a_label() {
    let (lib, project) = setup();
    let (doc, nets) = build(&project, &lib);
    let wire = nets
        .wires
        .iter()
        .find(|w| w.from.device == "Q1")
        .expect("a wire from Q1");
    let key = wire.label_key();
    let sheet = doc
        .sheets
        .iter()
        .find(|s| {
            s.items
                .iter()
                .any(|i| matches!(&i.kind, ItemKind::Label { key: k } if *k == key))
        })
        .expect("the number is a label");
    let item = sheet
        .items
        .iter()
        .find(|i| matches!(&i.kind, ItemKind::Label { key: k } if *k == key))
        .unwrap();
    // One horizontal line of text: the number of the wire.
    for &o in &item.ops {
        match &sheet.ops[o] {
            Op::Text { vertical, text, .. } => {
                assert!(!vertical);
                assert_eq!(*text, wire.number.to_string());
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_label_can_be_turned_and_resized() {
    let (lib, mut project) = setup();
    let name = crab_core::panel_layout::label_key("Q1", "name");
    project.panel_layout.set_label_style(&name, true, 30.0);
    let (doc, _) = build(&project, &lib);
    let text = doc.sheets.iter().find_map(|s| {
        s.items.iter().find_map(|i| match &i.kind {
            ItemKind::Label { key } if *key == name => match s.ops.get(*i.ops.first()?)? {
                Op::Text { size, vertical, .. } => Some((*size, *vertical)),
                _ => None,
            },
            _ => None,
        })
    });
    assert_eq!(text, Some((30.0, true)));
    // The shift and the style are kept apart: moving does not drop the turn.
    project.panel_layout.set_shift(&name, 20.0, 0.0);
    assert_eq!(project.panel_layout.label_style(&name), (true, 30.0));
    // Back to the defaults: the entry is gone.
    project.panel_layout.set_label_style(&name, false, 0.0);
    project.panel_layout.set_shift(&name, 0.0, 0.0);
    assert!(project.panel_layout.is_empty());
}

#[test]
fn every_text_of_a_diagram_sheet_is_a_label() {
    let (lib, project) = setup();
    let (doc, _) = build(&project, &lib);
    let mut texts = 0;
    for sheet in doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая") || s.title.starts_with("Цепи управления"))
    {
        let labelled: std::collections::HashSet<usize> = sheet
            .items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::Label { .. }))
            .flat_map(|i| i.ops.iter().copied())
            .collect();
        for (i, op) in sheet.ops.iter().enumerate() {
            if let Op::Text { text, .. } = op {
                texts += 1;
                assert!(labelled.contains(&i), "{} — {text}", sheet.title);
            }
        }
    }
    assert!(texts > 20);
}

/// The numbers of the sheets (from 1) that have a supply line item.
fn sheets_with_bus(doc: &Document) -> Vec<usize> {
    doc.sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.items
                .iter()
                .any(|i| matches!(i.kind, ItemKind::Bus { .. }))
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// The sheet references (`/N.`) in the body of a sheet.
fn sheet_refs(body: &str) -> Vec<String> {
    body.split('"')
        .filter(|t| t.starts_with('/') && t.contains('.'))
        .map(str::to_owned)
        .collect()
}

/// One long chain of breakers from the feed: only the first sheet has drops from the buses.
/// `singles` breakers of their own on the feed come after it: each one is a drop on its sheet.
fn chain_project(project: &mut Project, singles: usize) {
    let mut panel = String::from("supply = \"ac230\"\n[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
    for i in 1..=200 {
        let next = if i < 200 {
            format!("Q{}", i + 1)
        } else {
            "M1".to_owned()
        };
        panel.push_str(&format!(
            "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nload = \"{next}\"\n"
        ));
    }
    for i in 1..=singles {
        panel.push_str(&format!("[[device]]\nid = \"S{i}\"\nkind = \"breaker\"\n"));
    }
    panel.push_str("[[device]]\nid = \"M1\"\nkind = \"motor\"\n");
    project.panel = toml::from_str(&panel).unwrap();
}

/// The numbers of the power sheets (from 1).
fn power_sheets(doc: &Document) -> Vec<usize> {
    doc.sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.title.starts_with("Силовая"))
        .map(|(i, _)| i + 1)
        .collect()
}

#[test]
fn supply_lines_are_carried_on_straight_to_the_sheet_named() {
    let (lib, mut project) = setup();
    chain_project(&mut project, 0);
    let (plain, _) = build(&project, &lib);
    let with_bus = sheets_with_bus(&plain);
    assert_eq!(with_bus.len(), 1, "{with_bus:?}");
    let power = power_sheets(&plain);
    assert!(power.len() >= 3, "{power:?}");
    // Carried to the third power sheet: the lines go straight there; the second one (no drops of
    // its own in this chain) does not get them.
    project.panel_layout.set_bus_to(0, Some(power[2]));
    let (carried, _) = build(&project, &lib);
    assert_eq!(sheets_with_bus(&carried), vec![power[0], power[2]]);
    // The end on the source sheet refers to the target, and the start on the target back to the
    // source.
    let source = sheet_refs(&carried.sheets[power[0] - 1].body);
    assert!(
        source
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[2]))),
        "{source:?}"
    );
    let target = sheet_refs(&carried.sheets[power[2] - 1].body);
    assert!(
        target
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[0]))),
        "{target:?}"
    );
    // Past the last sheet: the missing power sheets are made, empty; only the sheet named gets
    // the lines.
    let last = *power.last().unwrap();
    project.panel_layout.set_bus_to(0, Some(last + 2));
    let (more, _) = build(&project, &lib);
    let made: Vec<usize> = more
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.title.starts_with("Силовая"))
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(made.len(), power.len() + 2, "{made:?}");
    assert_eq!(sheets_with_bus(&more), vec![power[0], last + 2]);
    let made_last = sheet_refs(&more.sheets[last + 1].body);
    assert!(
        made_last
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[0]))),
        "{made_last:?}"
    );
    // The control sheets, the tables and the title follow the new sheets.
    assert_eq!(more.sheets.len(), plain.sheets.len() + 2);
    // Taking it back restores the automatic result.
    project.panel_layout.set_bus_to(0, None);
    let (back, _) = build(&project, &lib);
    assert_eq!(sheets_with_bus(&back), with_bus);
}

#[test]
fn carried_lines_chain_from_sheet_to_sheet() {
    let (lib, mut project) = setup();
    // No drops of their own past the first power sheet, so only the sheets on the chain get lines.
    chain_project(&mut project, 0);
    let (plain, _) = build(&project, &lib);
    let power = power_sheets(&plain);
    assert!(power.len() >= 3, "{power:?}");
    // The number of the power sheet `k` (from 0); past the last one, the generator makes the
    // missing sheets.
    let sheet_no = |k: usize| {
        if k < power.len() {
            power[k]
        } else {
            power[power.len() - 1] + k + 1 - power.len()
        }
    };
    // Sheet 0 carries its lines to sheet 2, which carries them on to sheet 4.
    project.panel_layout.set_bus_to(0, Some(sheet_no(2)));
    project.panel_layout.set_bus_to(2, Some(sheet_no(4)));
    let (carried, _) = build(&project, &lib);
    let with_bus = sheets_with_bus(&carried);
    assert_eq!(
        with_bus,
        vec![sheet_no(0), sheet_no(2), sheet_no(4)],
        "{with_bus:?}"
    );
    assert!(!with_bus.contains(&sheet_no(1)) && !with_bus.contains(&sheet_no(3)));
    // Each end refers to the sheet it is carried to, each start to the sheet it comes from.
    let refs = |k: usize| sheet_refs(&carried.sheets[sheet_no(k) - 1].body);
    let refers_to = |refs: &[String], k: usize| {
        refs.iter()
            .any(|r| r.starts_with(&format!("/{}.", sheet_no(k))))
    };
    assert!(refers_to(&refs(0), 2), "{:?}", refs(0));
    assert!(refers_to(&refs(2), 4), "{:?}", refs(2));
    assert!(refers_to(&refs(2), 0), "{:?}", refs(2));
    assert!(refers_to(&refs(4), 2), "{:?}", refs(4));
}

#[test]
fn a_line_refers_to_the_sheet_named_past_a_sheet_with_drops_of_its_own() {
    let (lib, mut project) = setup();
    // Many breakers of their own on the feed: they fill past the first power sheet, so the second
    // one has a drop from the feed of its own.
    chain_project(&mut project, 400);
    let (plain, _) = build(&project, &lib);
    let power = power_sheets(&plain);
    assert!(power.len() >= 3, "{power:?}");
    let with_bus = sheets_with_bus(&plain);
    assert!(with_bus.contains(&power[1]), "{with_bus:?}");
    project.panel_layout.set_bus_to(0, Some(power[2]));
    let (carried, _) = build(&project, &lib);
    assert!(sheets_with_bus(&carried).contains(&power[2]));
    // The end on the source refers to the sheet named, not to the drop in between.
    let source = sheet_refs(&carried.sheets[power[0] - 1].body);
    assert!(
        source
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[2]))),
        "{source:?}"
    );
    assert!(
        !source
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[1]))),
        "{source:?}"
    );
    // The start on the sheet named refers back to the source.
    let target = sheet_refs(&carried.sheets[power[2] - 1].body);
    assert!(
        target
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[0]))),
        "{target:?}"
    );
    // The sheet between keeps its own line and its own references.
    let between = sheet_refs(&carried.sheets[power[1] - 1].body);
    assert!(
        between
            .iter()
            .any(|r| r.starts_with(&format!("/{}.", power[0]))),
        "{between:?}"
    );
}
