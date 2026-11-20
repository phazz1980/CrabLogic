//! Forcing an element of an array in a debug build (spec arrays-structs, task 7): the module reports
//! every element, takes the forced value back and keeps it in the array.

mod support;

use crab_codegen::Program;
use crab_core::Project;
use crab_sim::mcu::{Manifest, McuModule, Word};

#[test]
fn a_forced_element_reaches_the_array() {
    let lib = support::library();
    let project = Project::load(support::repo_path(
        "crates/crab-sim/tests/fixtures/wasm_arrays_force",
    ))
    .unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let slot = |label: &str| {
        built
            .watched
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("{label} is not watched: {:?}", built.watched))
    };
    let out = |module: &McuModule, label: &str| module.outputs()[slot(label)];

    module.step(10).unwrap();
    // The elements are reported with their initial values.
    assert_eq!(out(&module, "array:a[0]"), Word::int(5));
    assert_eq!(out(&module, "array:a[1]"), Word::int(6));
    assert_eq!(out(&module, "array:f[1]"), Word::float(2.5));
    assert_eq!(out(&module, "sum.out"), Word::int(18));

    // Force a[1]: it reaches the array at the end of this scan, the blocks see it on the next.
    assert!(module.force(slot("array:a[1]"), Word::int(100)));
    module.step(10).unwrap();
    module.step(10).unwrap();
    assert_eq!(out(&module, "r1.out"), Word::int(100));
    assert_eq!(out(&module, "sum.out"), Word::int(112));
    // Forcing a float element.
    assert!(module.force(slot("array:f[0]"), Word::float(0.25)));
    module.step(10).unwrap();
    module.step(10).unwrap();
    assert_eq!(out(&module, "fsum.out"), Word::float(2.75));

    // After the force is lifted the array keeps the value (it is the element now).
    module.unforce(slot("array:a[1]"));
    module.step(10).unwrap();
    assert_eq!(out(&module, "array:a[1]"), Word::int(100));
    assert_eq!(out(&module, "sum.out"), Word::int(112));
    // The other elements were not touched.
    assert_eq!(out(&module, "array:a[0]"), Word::int(5));
    assert_eq!(out(&module, "array:a[2]"), Word::int(7));
}
