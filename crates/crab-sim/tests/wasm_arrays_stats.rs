//! Fill, copy, sum, minimum, maximum and search of arrays in a module give the numbers of the
//! simulation (spec arrays-structs, task 4).

mod support;

#[test]
fn stats_match_the_simulation() {
    // 14 scans: the sources change over time, the edges come at scans 4 and 6, the fills start at 1..3.
    let n = support::compare("wasm_arrays_stats", 30, 14, 10);
    assert!(n >= 30);
}
