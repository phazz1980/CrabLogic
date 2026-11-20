//! Arrays of strings in a module give the texts of the simulation, scan by scan (spec arrays-structs,
//! task 4б). The texts are short (well below the 128 bytes a module keeps), so they match exactly.
//! The outputs are compared through their lengths (`text_length`) and their contents (`text_compare`).

mod support;

#[test]
fn text_arrays_match_the_simulation() {
    // 20 scans: the set and get run past the end of the arrays, the ring wraps, the stack fills
    // and pops, the copies come at scans 4 and 6.
    let n = support::compare("wasm_arrays_text", 20, 20, 10);
    assert!(n >= 20);
}
