//! The array examples (`examples/array_average`, `examples/array_tools`): what they do in the
//! simulation, and the same numbers from their modules.

mod support;

use crab_core::Project;
use crab_sim::Simulation;

fn sim(example: &str) -> Simulation {
    let lib = support::library();
    let project = Project::load(support::repo_path(&format!("examples/{example}"))).unwrap();
    Simulation::new(&project.logic, &lib).unwrap()
}

fn run(s: &mut Simulation, ms: u32) {
    for _ in 0..ms / 10 {
        s.step(10).unwrap();
    }
}

#[test]
fn average_pump_follows_the_mean_not_the_last_sample() {
    let mut s = sim("array_average");
    // Pressure 5 bar: the mean is 5, the pump stays off however long we wait.
    s.bench_mut().set_level(34, 2048.0);
    run(&mut s, 3000);
    assert_eq!(s.output("ring", "count"), Some(8.0));
    assert!((s.output("avg", "out").unwrap() - 5.0).abs() < 0.01);
    assert_eq!(s.output("delay", "out"), Some(0.0), "the pump stays off");
    // Pressure drops to about 1 bar: the window refills in 1.6 s, then the delay of 5 s counts.
    s.bench_mut().set_level(34, 410.0);
    run(&mut s, 2500);
    assert_eq!(
        s.output("delay", "out"),
        Some(0.0),
        "the delay has not run out yet"
    );
    run(&mut s, 5000);
    assert_eq!(s.output("delay", "out"), Some(1.0));
    assert!((s.output("avg", "out").unwrap() - 1.0).abs() < 0.01);
}

#[test]
fn a_shaky_pressure_lights_the_lamp() {
    let mut s = sim("array_average");
    s.bench_mut().set_level(34, 2048.0);
    run(&mut s, 3000);
    assert_eq!(s.output("shaky", "out"), Some(0.0));
    // One low sample inside the window: the span is 5 bar, more than 3.
    s.bench_mut().set_level(34, 0.0);
    run(&mut s, 250);
    s.bench_mut().set_level(34, 2048.0);
    run(&mut s, 100);
    assert_eq!(s.output("shaky", "out"), Some(1.0));
    // Eight good samples later the low one has left the window.
    run(&mut s, 2000);
    assert_eq!(s.output("shaky", "out"), Some(0.0));
}

#[test]
fn tools_example_walks_the_table_and_keeps_a_journal() {
    let mut s = sim("array_tools");
    run(&mut s, 4500);
    // The table was read, the last entry rewritten as 100 + step, the 60 is still there.
    assert_eq!(s.output("find", "found"), Some(1.0));
    assert_eq!(s.output("find", "index"), Some(2.0));
    // The journal holds the last four picks, newest at 0.
    let journal = s.array("journal").unwrap();
    assert_eq!(journal.len(), 4);
    assert!(journal.num(0) != 0.0 || journal.num(1) != 0.0);
    // The snapshot (taken every 5 s) is still empty before the first save, then fills.
    run(&mut s, 1500);
    let snapshot = s.array("snapshot").unwrap();
    assert!((0..4).any(|i| snapshot.num(i) != 0.0), "{snapshot:?}");
    // The stack never overflows its array: err only shows for one scan.
    assert!(s.output("stack", "count").unwrap() <= 4.0);
}

#[test]
fn array_examples_match_their_modules() {
    // Hardware pins read 0 both in the simulation and in the module here.
    support::compare_at("examples/array_average", 10, 80, 100);
    support::compare_at("examples/array_tools", 10, 120, 100);
}
