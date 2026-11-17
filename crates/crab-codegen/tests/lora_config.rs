//! The `crab.config` blob of `lora_e220` blocks (`specs/lora-e220`): the partners of a modem,
//! the types of their values, and the layout of the extra cells the driver gets.

use std::path::PathBuf;

use crab_codegen::Program;
use crab_core::{Library, Project};
use lora_e220::config::decode;
use lora_e220::frame::Ty;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn program(example: &str) -> (Library, Project) {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(repo_path(example)).unwrap();
    (lib, project)
}

#[test]
fn centre_config_lists_the_slaves_with_their_types() {
    let (lib, project) = program("examples/lora_star");
    let program = Program::build(&project, &lib).unwrap();
    let configs = program.lora_configs();
    assert_eq!(configs.keys().collect::<Vec<_>>(), ["modem"]);
    let cfg = decode(&configs["modem"]).expect("a valid blob");
    assert!(cfg.centre);
    let peers: Vec<_> = cfg.peers().collect();
    assert_eq!(peers.len(), 2);
    // pump_node: commands bool + int, answers int + bool.
    assert_eq!(peers[0].node, 0);
    assert_eq!(peers[0].tx.types(), [Ty::Bool, Ty::Int]);
    assert_eq!(peers[0].rx.types(), [Ty::Int, Ty::Bool]);
    // tank_node: no commands, one float answer.
    assert_eq!(peers[1].node, 1);
    assert!(peers[1].tx.is_empty());
    assert_eq!(peers[1].rx.types(), [Ty::Float]);
    // Extra cells: inputs are the commands, outputs the answers and a flag for each slave.
    assert_eq!(cfg.n_in(), 2);
    assert_eq!(cfg.n_out(), (2 + 1) + (1 + 1));
}

#[test]
fn slave_config_has_the_centre_as_its_only_partner() {
    let (lib, project) = program("examples/lora_slave");
    let program = Program::build(&project, &lib).unwrap();
    let cfg = decode(&program.lora_configs()["modem"]).expect("a valid blob");
    assert!(!cfg.centre);
    let peers: Vec<_> = cfg.peers().collect();
    assert_eq!(peers.len(), 1);
    // The slave sends the answers (int + bool) and receives the commands (bool + int).
    assert_eq!(peers[0].tx.types(), [Ty::Int, Ty::Bool]);
    assert_eq!(peers[0].rx.types(), [Ty::Bool, Ty::Int]);
}

#[test]
fn a_modem_without_partner_blocks_does_not_build() {
    let (lib, mut project) = program("examples/lora_star");
    // Only the modem stays: no partner blocks; the schema check refuses such a project.
    project.logic.blocks.retain(|b| b.kind == "lora_e220");
    project.logic.links.clear();
    let err = Program::build(&project, &lib).expect_err("must fail");
    assert!(format!("{err:?}").contains("Lora"), "{err:?}");
}

/// With the driver `block.wasm` in the library both ends build into a module: the modem's
/// extra cells (values of the partner blocks) fit its window.
#[test]
fn both_ends_build_into_modules() {
    for example in ["examples/lora_star", "examples/lora_slave"] {
        let (lib, project) = program(example);
        let program = Program::build(&project, &lib).unwrap();
        for sheet in program.sheets() {
            program
                .wasm_module(sheet)
                .unwrap_or_else(|e| panic!("{example}: {e}"));
        }
    }
}
