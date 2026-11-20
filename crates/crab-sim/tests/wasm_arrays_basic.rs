//! Array read and write in a module give the numbers of the simulation (spec arrays-structs,
//! criterion 3).

mod support;

#[test]
fn get_and_set_match_the_simulation() {
    // 12 scans: the index passes the end of every array, `far` shows the error flag.
    let n = support::compare("wasm_arrays_basic", 20, 12, 10);
    assert!(n >= 20);
}
