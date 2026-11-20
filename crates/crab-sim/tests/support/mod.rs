//! Shared by the tests that run a sheet as a module next to the simulation (`wasm_arrays*.rs`).
#![allow(dead_code)] // each test file uses some of the helpers

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
    compare_at(
        &format!("crates/crab-sim/tests/fixtures/{name}"),
        min_outputs,
        steps,
        dt,
    )
}

/// [`compare`] for any project folder of the repository (an example, for instance).
pub fn compare_at(rel: &str, min_outputs: usize, steps: u32, dt: u32) -> usize {
    let name = rel;
    let lib = library();
    let project = Project::load(repo_path(rel)).unwrap();
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
    // The distinct values each watched output took: a test where nothing moves proves nothing.
    let mut seen: Vec<Vec<u32>> = vec![Vec::new(); built.watched.len()];
    for step in 0..steps {
        sim.step(dt).unwrap();
        module.step(dt).unwrap();
        for (i, label) in built.watched.iter().enumerate() {
            let expected = if let Some((array, index)) = array_label(label) {
                // An element of an array the module reports in a debug build.
                let a = sim.array(array).unwrap();
                let x = a.num(index);
                if a.ty == crab_core::arrays::ElemType::Float {
                    Word::float(x as f32)
                } else {
                    Word::int(x as i32)
                }
            } else {
                let (block, port) = label.split_once('.').unwrap();
                word(&program, label, sim.output(block, port).unwrap())
            };
            let got = module.outputs()[i];
            assert_eq!(got, expected, "{name}, step {step}: {label}");
            let bits = format!("{got:?}").len() as u32 ^ hash(&format!("{got:?}"));
            if !seen[i].contains(&bits) {
                seen[i].push(bits);
            }
        }
    }
    // Only the outputs of blocks count here: the elements of an array may well stay put in a
    // fixture (an unused sensor), they are compared with the simulation all the same.
    let blocks: Vec<usize> = (0..built.watched.len())
        .filter(|&i| array_label(&built.watched[i]).is_none())
        .collect();
    let moving = blocks.iter().filter(|&&i| seen[i].len() > 1).count();
    assert!(
        moving * 4 >= blocks.len(),
        "{name}: only {moving} of {} outputs ever change: the test would pass on any code",
        blocks.len()
    );
    built.watched.len()
}

/// Array name and element of a label `array:<name>[<i>]` (see `crab_debug::array_label`).
fn array_label(label: &str) -> Option<(&str, usize)> {
    let rest = label.strip_prefix("array:")?.strip_suffix(']')?;
    let (name, index) = rest.rsplit_once('[')?;
    Some((name, index.parse().ok()?))
}

fn hash(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}
