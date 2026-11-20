//! Shift register, ring buffer and stack in the simulation of a schema. Edges come from
//! `web_switch` blocks (nobody computes them) that the test sets by hand between scans.

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

/// A web switch `id` (nobody computes it), to be driven by hand.
fn button(id: &str) -> String {
    format!("[[block]]\nid = \"{id}\"\ntype = \"web_switch\"\n\n")
}

/// A link `from` -> `to`.
fn link(from: &str, to: &str) -> String {
    format!("[[link]]\nfrom = \"{from}\"\nto = \"{to}\"\n\n")
}

/// An int array `a` of `len` elements.
fn array_a(len: u32, items: &str) -> String {
    format!("[[array]]\nname = \"a\"\ntype = \"int\"\nlen = {len}\n{items}\n\n")
}

/// An array block `id` of `kind` on array `a` with the constant `value` on its value input.
fn block(id: &str, kind: &str, value: i32) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"{kind}\"\narray = \"a\"\n[block.const]\nvalue = {value}\n\n"
    )
}

/// A press: the button is high for one scan and low for the next one.
fn press(s: &mut Simulation, id: &str) {
    assert!(s.set_output(id, "out", 1.0));
    s.step(10).unwrap();
    assert!(s.set_output(id, "out", 0.0));
    s.step(10).unwrap();
}

fn items(s: &Simulation) -> Vec<f64> {
    match &s.array("a").unwrap().items {
        Items::Num(v) => v.clone(),
        Items::Text(_) => panic!("numbers expected"),
    }
}

fn ring_sim(len: u32) -> Simulation {
    let mut logic = array_a(len, "");
    logic += &button("btn");
    logic += &block("ring", "array_ring", 5);
    logic += &link("btn.out", "ring.push");
    sim(&logic)
}

#[test]
fn ring_of_three_wraps_after_five_writes() {
    let mut s = ring_sim(3);
    s.step(10).unwrap();
    assert_eq!(s.output("ring", "count"), Some(0.0));
    for _ in 0..5 {
        press(&mut s, "btn");
    }
    // Five writes into three places: the head wrapped (5 % 3 = 2), the count stays 3.
    assert_eq!(items(&s), vec![5.0, 5.0, 5.0]);
    assert_eq!(s.output("ring", "head"), Some(2.0));
    assert_eq!(s.output("ring", "count"), Some(3.0));
}

#[test]
fn ring_head_and_count_grow_then_wrap() {
    let mut s = ring_sim(3);
    let mut seen = Vec::new();
    for _ in 0..4 {
        press(&mut s, "btn");
        seen.push((s.output("ring", "head"), s.output("ring", "count")));
    }
    assert_eq!(
        seen,
        vec![
            (Some(1.0), Some(1.0)),
            (Some(2.0), Some(2.0)),
            (Some(0.0), Some(3.0)),
            (Some(1.0), Some(3.0)),
        ]
    );
}

#[test]
fn stack_fills_up_and_reports_full_and_err() {
    let mut logic = array_a(2, "");
    logic += &button("push");
    logic += &button("pop");
    logic += &block("st", "array_stack", 7);
    logic += &link("push.out", "st.push");
    logic += &link("pop.out", "st.pop");
    let mut s = sim(&logic);
    s.step(10).unwrap();
    assert_eq!(s.output("st", "empty"), Some(1.0));

    press(&mut s, "push");
    assert_eq!(s.output("st", "count"), Some(1.0));
    assert_eq!(s.output("st", "out"), Some(7.0));
    assert_eq!(s.output("st", "empty"), Some(0.0));
    press(&mut s, "push");
    assert_eq!(s.output("st", "full"), Some(1.0));
    assert_eq!(s.output("st", "err"), Some(0.0));

    // The third push is refused: err for that scan only.
    assert!(s.set_output("push", "out", 1.0));
    s.step(10).unwrap();
    assert_eq!(s.output("st", "err"), Some(1.0));
    assert_eq!(s.output("st", "count"), Some(2.0));
    assert!(s.set_output("push", "out", 0.0));
    s.step(10).unwrap();
    assert_eq!(s.output("st", "err"), Some(0.0));

    press(&mut s, "pop");
    assert_eq!(s.output("st", "count"), Some(1.0));
    press(&mut s, "pop");
    assert_eq!(s.output("st", "empty"), Some(1.0));
    assert_eq!(s.output("st", "out"), Some(0.0));
}

#[test]
fn shift_moves_the_values_up() {
    let mut logic = array_a(3, "items = [1, 2, 3]");
    logic += &button("go");
    logic += &block("sh", "array_shift", 9);
    logic += &link("go.out", "sh.go");
    let mut s = sim(&logic);
    s.step(10).unwrap();
    assert_eq!(items(&s), vec![1.0, 2.0, 3.0]);
    press(&mut s, "go");
    assert_eq!(items(&s), vec![9.0, 1.0, 2.0]);
    press(&mut s, "go");
    assert_eq!(items(&s), vec![9.0, 9.0, 1.0]);
}

#[test]
fn reset_clears_the_private_state() {
    let mut s = ring_sim(3);
    press(&mut s, "btn");
    press(&mut s, "btn");
    assert_eq!(s.output("ring", "count"), Some(2.0));
    s.reset().unwrap();
    s.step(10).unwrap();
    assert_eq!(s.output("ring", "count"), Some(0.0));
    assert_eq!(s.output("ring", "head"), Some(0.0));
}
