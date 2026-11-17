//! LoRa in the schema simulation (`specs/lora-e220`): there is no radio on the PC. The blocks of
//! the partners keep the values of `block.toml` (`online`/`linked` true, the values 0) and the
//! outputs can be set by hand, as for any block whose code runs only on the board.

use std::path::PathBuf;

use crab_core::{Library, Project};
use crab_sim::Simulation;

fn simulation(example: &str) -> Simulation {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (lib, errors) = Library::load(root.join("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(root.join(example)).unwrap();
    Simulation::new(&project.logic, &lib).unwrap()
}

fn run(sim: &mut Simulation) {
    for _ in 0..5 {
        sim.step(20).unwrap();
    }
}

#[test]
fn the_centre_sees_its_slaves_online_and_takes_their_values_by_hand() {
    let mut sim = simulation("examples/lora_star");
    run(&mut sim);
    assert_eq!(sim.output("modem", "ready"), Some(1.0));
    assert_eq!(sim.output("pump_node", "online"), Some(1.0));
    assert_eq!(sim.output("tank_node", "online"), Some(1.0));
    // The tank is cold: the fan stays off. Set the temperature by hand: the fan comes on.
    assert_eq!(sim.output("hot", "out"), Some(0.0));
    sim.set_output("tank_node", "ans0f", 55.0);
    run(&mut sim);
    assert_eq!(sim.output("hot", "out"), Some(1.0));
    sim.set_output("tank_node", "ans0f", 20.0);
    run(&mut sim);
    assert_eq!(sim.output("hot", "out"), Some(0.0));
    // A slave that stops answering: the schema can react to `online`.
    sim.set_output("tank_node", "online", 0.0);
    run(&mut sim);
    assert_eq!(sim.output("tank_node", "online"), Some(0.0));
}

#[test]
fn the_slave_gets_commands_by_hand_and_runs_without_the_link() {
    let mut sim = simulation("examples/lora_slave");
    run(&mut sim);
    assert_eq!(sim.output("modem", "ready"), Some(1.0));
    assert_eq!(sim.output("centre", "linked"), Some(1.0));
    sim.set_output("centre", "cmd0b", 1.0);
    run(&mut sim);
    assert_eq!(sim.output("centre", "cmd0b"), Some(1.0));
    // Without the link the last command stays.
    sim.set_output("centre", "linked", 0.0);
    run(&mut sim);
    assert_eq!(sim.output("centre", "cmd0b"), Some(1.0));
}
