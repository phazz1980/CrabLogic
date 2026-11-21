//! The panel of the example `examples/pump` (`specs/electrical-schematics`, task 11).

use std::path::PathBuf;

use crab_core::{check_panel, Library, Project};
use crab_schematic::{build_document_in, Format, Lang};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

#[test]
fn the_pump_example_has_a_clean_panel_that_makes_all_the_sheets() {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty());
    let project = Project::load(repo_path("examples/pump")).unwrap();
    assert!(
        !project.panel.is_empty(),
        "examples/pump/panel.toml is missing"
    );
    let report = check_panel(&project, &lib);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    for lang in [Lang::Ru, Lang::En] {
        let (doc, nets) = build_document_in(&project, &lib, lang, Format::A3).unwrap();
        // Title, power, control, three tables.
        assert!(doc.sheets.len() >= 6, "{}", doc.sheets.len());
        assert!(!nets.wires.is_empty());
    }
}
