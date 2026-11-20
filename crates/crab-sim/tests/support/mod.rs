//! Shared by the tests that run a sheet as a module next to the simulation (`wasm_arrays*.rs`).

use std::path::PathBuf;

use crab_codegen::Program;
use crab_core::{Library, Project, SignalType};
use crab_sim::mcu::{Manifest, McuModule, Word};
use crab_sim::schema::Simulation;

pub fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

pub fn library() -> Library {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

/// The carrier value `expected` of the signal `label` as the 32-bit word the module must hold.
fn word(program: &Program, label: &str, expected: f64) -> Word {
    let name = label.replace('.', "_");
    let ty = program
        .signals
        .iter()
        .find(|s| s.name == name)
        .map_or(SignalType::Float, |s| s.ty);
    if ty == SignalType::Float {
        Word::float(expected as f32)
    } else {
        Word::int(expected as i32)
    }
}

/// Runs the sheet of the fixture `crates/crab-sim/tests/fixtures/<name>` as a module next to the
/// simulation, `steps` scans of `dt` ms, and compares every watched output after every scan.
/// Returns the number of watched outputs (the fixture must watch at least `min_outputs`).
pub fn compare(name: &str, min_outputs: usize, steps: u32, dt: u32) -> usize {
    let lib = library();
    let project = Project::load(repo_path(&format!("crates/crab-sim/tests/fixtures/{name}"))).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    assert!(
        built.watched.len() >= min_outputs,
        "{name}: only {} outputs watched: {:?}",
        built.watched.len(),
        built.watched
    );
    for step in 0..steps {
        sim.step(dt).unwrap();
        module.step(dt).unwrap();
        for (i, label) in built.watched.iter().enumerate() {
            let (block, port) = label.split_once('.').unwrap();
            let expected = word(&program, label, sim.output(block, port).unwrap());
            let got = module.outputs()[i];
            assert_eq!(got, expected, "{name}, step {step}: {label}");
        }
    }
    built.watched.len()
}
