//! Array blocks in the simulation of a schema: read and write by index, the initial table, a bad
//! index, reset. Shared helpers for the tests of the other groups live in this file's pattern:
//! a `Logic` is read from TOML text and run step by step.

use std::path::PathBuf;

use crab_core::{Library, Logic};
use crab_sim::arrays::Items;
use crab_sim::Simulation;

fn sim(logic_toml: &str) -> Simulation {
    let lib_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let (lib, _) = Library::load(lib_dir).unwrap();
    let logic: Logic = toml::from_str(logic_toml).unwrap();
    Simulation::new(&logic, &lib).unwrap()
}

const SCHEMA: &str = r#"
[[array]]
name = "a"
type = "int"
len = 4
items = [10, 20]

[[block]]
id = "put"
type = "array_set"
array = "a"
[block.const]
index = 2
value = 77

[[block]]
id = "read"
type = "array_get"
array = "a"
[block.const]
index = 1

# An index computed at run time (a constant index outside the array is a check error): 4 + 5 = 9.
[[block]]
id = "nine"
type = "add"
[block.const]
a = 4
b = 5

[[block]]
id = "far"
type = "array_get"
array = "a"

[[link]]
from = "nine.out"
to = "far.index"
"#;

#[test]
fn set_writes_and_get_reads_by_index() {
    let mut s = sim(SCHEMA);
    // The initial table is there before the first scan.
    assert_eq!(
        s.array("a").unwrap().items,
        Items::Num(vec![10.0, 20.0, 0.0, 0.0])
    );
    s.step(10).unwrap();
    assert_eq!(s.array("a").unwrap().num(2), 77.0);
    assert_eq!(s.output("read", "out"), Some(20.0));
    assert_eq!(s.output("read", "err"), Some(0.0));
    assert_eq!(s.output("put", "err"), Some(0.0));
}

#[test]
fn a_bad_index_gives_zero_and_the_flag() {
    let mut s = sim(SCHEMA);
    s.step(10).unwrap();
    assert_eq!(s.output("far", "out"), Some(0.0));
    assert_eq!(s.output("far", "err"), Some(1.0));
}

#[test]
fn reset_restores_the_initial_table() {
    let mut s = sim(SCHEMA);
    s.step(10).unwrap();
    assert_eq!(s.array("a").unwrap().num(2), 77.0);
    s.reset().unwrap();
    assert_eq!(s.array("a").unwrap().num(2), 0.0);
    assert_eq!(s.array("a").unwrap().num(0), 10.0);
}
