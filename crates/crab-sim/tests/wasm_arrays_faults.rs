//! An index outside an array is a fault of the module: it counts in the `faults()` export, which
//! the master adds to the failures of the board (spec arrays-structs, task 12).

mod support;

use crab_codegen::Program;
use crab_core::Project;
use crab_kernel::engine::Instance;
use crab_sim::mcu::{Manifest, McuModule};
use crab_sim::schema::Simulation;

#[test]
fn indexes_outside_an_array_are_counted_as_faults() {
    let lib = support::library();
    let project = Project::load(support::repo_path(
        "crates/crab-sim/tests/fixtures/wasm_arrays_basic",
    ))
    .unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    assert_eq!(module.faults(), 0);
    let mut expected = 0u32;
    for _ in 0..12 {
        sim.step(10).unwrap();
        module.step(10).unwrap();
        // The simulation raises `err` of the same blocks in the same scan.
        for b in &project.logic.blocks {
            if matches!(b.kind.as_str(), "array_get" | "array_set")
                && sim.output(&b.id, "err") == Some(1.0)
            {
                expected += 1;
            }
        }
        assert_eq!(module.faults(), expected);
    }
    assert!(expected > 0, "the fixture never leaves an array");
}
