// The Wasm C API is not built; wasm_runtime_common.c still references this on a path we never take.
void wasm_trap_delete(void *trap) { (void)trap; }
