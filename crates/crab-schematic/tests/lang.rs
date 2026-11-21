//! Both languages of the documentation (`specs/electrical-schematics`, task 8).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_document_in, Format, Lang};

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

/// Every kind of device, both supplies' worth of captions, a long chain (cut marks) and enough
/// devices for the tables to take their words. Only the user's own data (ratings, part numbers)
/// is ASCII here.
const PANEL: &str = r#"
supply = "ac400"

[[device]]
id = "X1"
kind = "feed"
rating = "400 V, 25 A"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"
part = "S203-C16"
load = "F1"
[[device]]
id = "F1"
kind = "rcd"
rating = "30 mA"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
rating = "9 A"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
rating = "0.75 kW"
[[device]]
id = "Q2"
kind = "breaker"
rating = "C10"
load = "G1"
[[device]]
id = "G1"
kind = "psu"
rating = "24 V, 5 A"
[[device]]
id = "R1"
kind = "relay"
bind = "pump"
load = "H1"
[[device]]
id = "H1"
kind = "load"
rating = "50 W"
[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
[[device]]
id = "S1"
kind = "button"
bind = "sensor.out"
[[device]]
id = "XT1"
kind = "terminal"
"#;

fn project(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
}

fn has_cyrillic(text: &str) -> bool {
    text.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c))
}

#[test]
fn the_english_documents_have_no_russian_words() {
    let lib = library();
    for panel in [
        PANEL,
        &PANEL.replace("ac400", "ac230"),
        &PANEL.replace("ac400", "dc24"),
    ] {
        for format in [Format::A3, Format::A4] {
            let (doc, nets) = build_document_in(&project(panel), &lib, Lang::En, format).unwrap();
            assert!(doc.sheets.len() >= 5);
            for sheet in &doc.sheets {
                assert!(!has_cyrillic(&sheet.title), "{}", sheet.title);
                let russian = sheet.body.lines().find(|l| has_cyrillic(l));
                assert!(russian.is_none(), "«{}»: {russian:?}", sheet.title);
            }
            for table in [&doc.tables.terminals, &doc.tables.wires, &doc.tables.bom] {
                let all = table
                    .head
                    .iter()
                    .chain(table.rows.iter().flatten())
                    .chain(table.footer.iter())
                    .chain(std::iter::once(&table.title));
                for cell in all {
                    assert!(!has_cyrillic(cell), "{}: {cell}", table.title);
                }
            }
            assert!(!nets.wires.is_empty());
        }
    }
}

#[test]
fn the_title_block_is_in_the_language_of_the_document() {
    let lib = library();
    for (lang, words) in [
        (Lang::Ru, ["Проект", "Наименование", "Формат", "Лист"]),
        (Lang::En, ["Project", "Title", "Format", "Sheet"]),
    ] {
        let (doc, _) = build_document_in(&project(PANEL), &lib, lang, Format::A3).unwrap();
        let source = crab_schematic::doc::typst_source(&doc, lang);
        for word in words {
            assert!(source.contains(&format!("\"{word}\"")), "{lang:?}: {word}");
        }
    }
}

#[test]
fn both_languages_draw_the_same_sheets_with_the_same_wires() {
    let lib = library();
    let (ru, ru_nets) = build_document_in(&project(PANEL), &lib, Lang::Ru, Format::A3).unwrap();
    let (en, en_nets) = build_document_in(&project(PANEL), &lib, Lang::En, Format::A3).unwrap();
    // Only the words differ: the same number of sheets, rows and wires, the same places.
    assert_eq!(ru.sheets.len(), en.sheets.len());
    assert_eq!(ru_nets.wires, en_nets.wires);
    assert_eq!(ru.power_at, en.power_at);
    assert_eq!(ru.coil_at, en.coil_at);
    for (a, b) in [
        (&ru.tables.terminals, &en.tables.terminals),
        (&ru.tables.wires, &en.tables.wires),
        (&ru.tables.bom, &en.tables.bom),
    ] {
        assert_eq!(a.head.len(), b.head.len());
        assert_eq!(a.rows.len(), b.rows.len());
        assert_eq!(a.widths, b.widths);
    }
    // The language code of the command line.
    assert_eq!(Lang::from_code("en"), Some(Lang::En));
    assert_eq!(Lang::from_code("ru"), Some(Lang::Ru));
    assert_eq!(Lang::from_code("de"), None);
}
