//! Items of the sheets: which operations belong to which symbol or wire (`specs/panel-editor`,
//! task 3).

use std::collections::HashSet;
use std::path::PathBuf;

use crab_core::{Library, Project};
use crab_schematic::{build_document, Document, ItemKind, Lang, Nets};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn pump() -> (Document, Nets) {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(repo_path("examples/pump")).unwrap();
    build_document(&project, &lib, Lang::Ru).unwrap()
}

#[test]
fn items_point_at_existing_operations_and_share_none() {
    let (doc, _) = pump();
    for sheet in &doc.sheets {
        let mut seen = HashSet::new();
        for item in &sheet.items {
            for &i in &item.ops {
                assert!(i < sheet.ops.len(), "{}", sheet.title);
                assert!(seen.insert(i), "operation {i} is in two items");
            }
        }
    }
}

#[test]
fn every_drawn_device_has_a_symbol_with_ports() {
    let (doc, _) = pump();
    let symbols: Vec<(&str, &str, usize)> = doc
        .sheets
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| match &i.kind {
            ItemKind::Symbol {
                view,
                device,
                ports,
                ..
            } => Some((*view, device.as_str(), ports.len())),
            ItemKind::Wire { .. } | ItemKind::Label { .. } | ItemKind::Bus { .. } => None,
        })
        .collect();
    // The devices of the power sheets and of the control sheets, and the board.
    for (view, device) in [
        ("power", "Q1"),
        ("power", "K1"),
        ("power", "M1"),
        ("power", "G1"),
        ("control", "K1"),
        ("control", "B1"),
    ] {
        let found = symbols
            .iter()
            .find(|(v, d, _)| *v == view && *d == device)
            .unwrap_or_else(|| panic!("no {view} symbol of {device}"));
        assert!(found.2 > 0, "{view} {device} has no ports");
    }
    // The board is its channels: a symbol of each bound terminal, with its port.
    assert!(
        symbols
            .iter()
            .any(|(v, d, p)| *v == "board" && d.starts_with("A1/") && *p > 0),
        "no channel of the board"
    );
    // Every symbol has some drawing.
    for sheet in &doc.sheets {
        for item in &sheet.items {
            assert!(!item.ops.is_empty(), "{:?}", item.kind);
        }
    }
}

#[test]
fn every_wire_is_drawn_as_an_item() {
    let (doc, nets) = pump();
    let drawn: HashSet<&str> = doc
        .sheets
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| match &i.kind {
            ItemKind::Wire { key, .. } => Some(key.as_str()),
            ItemKind::Symbol { .. } | ItemKind::Label { .. } | ItemKind::Bus { .. } => None,
        })
        .collect();
    for wire in &nets.wires {
        assert!(drawn.contains(wire.key().as_str()), "{}", wire.key());
    }
}

#[test]
fn the_ends_of_a_wire_are_ports_of_symbols() {
    let (doc, _) = pump();
    for sheet in &doc.sheets {
        let ports: HashSet<(String, String)> = sheet
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Symbol { device, ports, .. } => Some(
                    ports
                        .iter()
                        .map(|p| (device.clone(), p.name.clone()))
                        .collect::<Vec<_>>(),
                ),
                ItemKind::Wire { .. } | ItemKind::Label { .. } | ItemKind::Bus { .. } => None,
            })
            .flatten()
            .collect();
        for item in &sheet.items {
            let ItemKind::Wire { key, from, to } = &item.kind else {
                continue;
            };
            // The far end of a bus is not a symbol: only the ends that name a device with a symbol
            // on this sheet must be a port of it.
            for end in [from, to] {
                let has_symbol = ports.iter().any(|(d, _)| *d == end.device);
                if has_symbol {
                    assert!(
                        ports.contains(&(end.device.clone(), end.port.clone())),
                        "{key}: {}:{} is not a port",
                        end.device,
                        end.port
                    );
                }
            }
        }
    }
}

#[test]
fn designations_by_sheet_are_valid_and_unique() {
    let (lib, _) = Library::load(repo_path("library")).unwrap();
    let project = Project::load(repo_path("examples/pump")).unwrap();
    let (doc, _) = build_document(&project, &lib, Lang::Ru).unwrap();
    let names = crab_schematic::sheet_designations(&doc, &project.panel);
    assert!(!names.is_empty());
    let new: HashSet<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(new.len(), names.len());
    assert!(names
        .iter()
        .any(|(_, n)| n.starts_with('K') && n.contains('.')));
    let mut panel = project.panel.clone();
    panel.rename_all(&names).unwrap();
    for (_, n) in &names {
        assert!(panel.device(n).is_some());
    }
}
