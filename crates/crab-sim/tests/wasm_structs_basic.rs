//! A structure written by `struct_pack` and read by `struct_unpack` gives the numbers of the
//! simulation in a module (spec arrays-structs, task 3b).

mod support;

#[test]
fn pack_and_unpack_match_the_simulation() {
    // 14 scans: `cnt` wraps past 255 on scan 10, `en` is off on scans 4..7.
    let n = support::compare("wasm_structs_basic", 4, 14, 10);
    assert!(n >= 4);
}

#[test]
fn unwired_pack_inputs_take_constants_and_initial_values() {
    // `temp` is a constant, `cnt` has no wire (its initial value), `en` is on by default.
    let n = support::compare("wasm_structs_defaults", 3, 8, 10);
    assert!(n >= 3);
}

#[test]
fn the_simulation_gives_the_defaults_themselves() {
    let lib = support::library();
    let project = crab_core::Project::load(support::repo_path(
        "crates/crab-sim/tests/fixtures/wasm_structs_defaults",
    ))
    .unwrap();
    let mut sim = crab_sim::schema::Simulation::new(&project.logic, &lib).unwrap();
    for _ in 0..3 {
        sim.step(10).unwrap();
    }
    assert_eq!(sim.output("un", "temp"), Some(2.25));
    assert_eq!(sim.output("un", "cnt"), Some(200.0));
}
