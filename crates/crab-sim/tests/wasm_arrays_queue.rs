//! Shift register, ring buffer and stack in a module give the numbers of the simulation, scan by
//! scan (spec arrays-structs, the queue blocks).

mod support;

#[test]
fn queues_match_the_simulation() {
    // 14 scans: the edges rise several times, the ring wraps, the stack fills and underflows.
    support::compare("wasm_arrays_queue", 30, 14, 10);
}
