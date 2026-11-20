//! Array blocks fill, copy, sum, min, max and find run through the simulation of a schema.

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

#[test]
fn fill_writes_every_element_while_go_is_on() {
    let mut s = sim(r#"
[[array]]
name = "a"
type = "byte"
len = 3
items = [1, 2]

[[block]]
id = "f"
type = "array_fill"
array = "a"
[block.const]
value = 257
go = true
"#);
    s.step(5).unwrap();
    assert_eq!(s.array("a").unwrap().items, Items::Num(vec![1.0, 1.0, 1.0]));
}

#[test]
fn fill_without_go_changes_nothing() {
    let mut s = sim(r#"
[[array]]
name = "a"
type = "int"
len = 2
items = [3, 4]

[[block]]
id = "f"
type = "array_fill"
array = "a"
[block.const]
value = 9
go = false
"#);
    s.step(5).unwrap();
    assert_eq!(s.array("a").unwrap().items, Items::Num(vec![3.0, 4.0]));
}

const COPY: &str = r#"
[[array]]
name = "src"
type = "int"
len = 3
items = [1, 2, 3]

[[array]]
name = "dst"
type = "int"
len = 2

[[block]]
id = "ok"
type = "array_copy"
array = "src"
dest = "dst"
[block.const]
go = true
"#;

#[test]
fn copy_runs_on_the_first_scan_by_the_shorter_length() {
    let mut s = sim(COPY);
    s.step(1).unwrap();
    assert_eq!(s.array("dst").unwrap().items, Items::Num(vec![1.0, 2.0]));
    assert_eq!(s.output("ok", "err"), Some(0.0));
}

// A copy into a missing array or one of another type is a check error now (see
// `check::arrays`); the behaviour against such a destination stays covered by the unit tests
// in `arrays/stats.rs`.

#[test]
fn sum_of_ints_and_floats() {
    let mut s = sim(r#"
[[array]]
name = "i"
type = "int"
len = 4
items = [1, 2, 3, -10]

[[array]]
name = "x"
type = "float"
len = 3
items = [0.5, 0.25, 1.0]

[[block]]
id = "si"
type = "array_sum"
array = "i"

[[block]]
id = "sx"
type = "array_sum"
array = "x"
"#);
    s.step(2).unwrap();
    assert_eq!(s.output("si", "out"), Some(-4.0));
    assert_eq!(s.output("sx", "out"), Some(1.75));
}

#[test]
fn min_and_max_with_the_index_of_the_first_occurrence() {
    let mut s = sim(r#"
[[array]]
name = "a"
type = "int"
len = 5
items = [5, -3, 9, -3, 9]

[[block]]
id = "mn"
type = "array_min"
array = "a"

[[block]]
id = "mx"
type = "array_max"
array = "a"
"#);
    s.step(2).unwrap();
    assert_eq!(s.output("mn", "out"), Some(-3.0));
    assert_eq!(s.output("mn", "index"), Some(1.0));
    assert_eq!(s.output("mx", "out"), Some(9.0));
    assert_eq!(s.output("mx", "index"), Some(2.0));
}

#[test]
fn find_found_and_not_found() {
    let mut s = sim(r#"
[[array]]
name = "a"
type = "int"
len = 4
items = [4, 8, 8, 1]

[[block]]
id = "hit"
type = "array_find"
array = "a"
[block.const]
value = 8

[[block]]
id = "miss"
type = "array_find"
array = "a"
[block.const]
value = 7
"#);
    s.step(2).unwrap();
    assert_eq!(s.output("hit", "index"), Some(1.0));
    assert_eq!(s.output("hit", "found"), Some(1.0));
    assert_eq!(s.output("miss", "index"), Some(-1.0));
    assert_eq!(s.output("miss", "found"), Some(0.0));
}

#[test]
fn find_in_a_byte_array_compares_the_stored_value() {
    let mut s = sim(r#"
[[array]]
name = "a"
type = "byte"
len = 3
items = [4, 1, 9]

[[block]]
id = "f"
type = "array_find"
array = "a"
[block.const]
value = 257
"#);
    s.step(2).unwrap();
    assert_eq!(s.output("f", "index"), Some(1.0));
    assert_eq!(s.output("f", "found"), Some(1.0));
}

#[test]
fn find_in_a_string_array() {
    let mut s = sim(r#"
[[array]]
name = "names"
type = "string"
len = 3
items = ["ab", "cd", "ef"]

[[block]]
id = "hit"
type = "array_find"
array = "names"
[block.const]
value = "ef"

[[block]]
id = "miss"
type = "array_find"
array = "names"
[block.const]
value = "e"
"#);
    s.step(2).unwrap();
    assert_eq!(s.output("hit", "index"), Some(2.0));
    assert_eq!(s.output("hit", "found"), Some(1.0));
    assert_eq!(s.output("miss", "index"), Some(-1.0));
    assert_eq!(s.output("miss", "found"), Some(0.0));
}
