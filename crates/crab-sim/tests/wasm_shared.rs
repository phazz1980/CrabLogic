//! An array and a structure used by blocks of two sheets: the build joins the sheets into one module
//! and the module gives the numbers of the simulation (spec arrays-structs, task 5).

mod support;

use crab_codegen::Program;
use crab_core::Project;

#[test]
fn sheets_sharing_data_become_one_module() {
    let lib = support::library();
    let project = Project::load(support::repo_path(
        "crates/crab-sim/tests/fixtures/wasm_shared",
    ))
    .unwrap();
    let mut program = Program::build(&project, &lib).unwrap();
    assert_eq!(program.shared_sheet_sets(), vec![vec![1, 2]]);
    // Apart, the sheets cannot be built: the data is shared.
    assert!(program.wasm_module_with(1, false).is_err());
    program.auto_group_for_board(false).unwrap();
    assert_eq!(program.groups, vec![vec![1, 2]]);
    assert_eq!(program.sheets(), vec![1]);
    assert!(program.wasm_module_with(1, false).is_ok());
}

#[test]
fn a_shared_array_and_structure_match_the_simulation() {
    // 10 scans: `win` is written on scans 0..3 and read on both sheets, the index leaves the array after.
    let n = support::compare("wasm_shared", 4, 10, 10);
    assert!(n >= 4);
}

#[test]
fn the_check_accepts_data_shared_by_two_sheets() {
    let lib = support::library();
    let project = Project::load(support::repo_path(
        "crates/crab-sim/tests/fixtures/wasm_shared",
    ))
    .unwrap();
    let report = crab_codegen::check_project(&project, &lib);
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == crab_core::Severity::Error)
        .map(|d| d.message.clone())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
