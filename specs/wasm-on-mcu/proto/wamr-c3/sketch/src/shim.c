// Small helpers that need WAMR internal structures.
#include "wasm_export.h"
#include "wasm_runtime_common.h"
#include "wasm.h"
#include "aot_runtime.h"

// ESP32-C3: the instruction and data buses share one MMU table, so flash mapped at
// DROM 0x3C.. is also visible to the CPU at IROM 0x42.. with the same offset.
#define DROM_LOW 0x3C000000u
#define DROM_HIGH 0x3C800000u
#define IBUS_DELTA 0x06000000u

// Shrink the module's single memory to `bytes` (the module's `crab.mem`), like the
// master does with wasm3's memoryLimit. 0 keeps the declared size.
int shim_set_memory(wasm_module_t m, uint32_t bytes)
{
    WASMModuleCommon *c = (WASMModuleCommon *)m;
    WASMMemory *mem = NULL;
    if (bytes == 0)
        return 0;
    bytes = (bytes + 15) & ~15u;
    if (c->module_type == Wasm_Module_Bytecode) {
        WASMModule *w = (WASMModule *)m;
        if (w->memory_count)
            mem = &w->memories[0];
    }
    else {
        AOTModule *a = (AOTModule *)m;
        if (a->memory_count)
            mem = &a->memories[0];
    }
    if (!mem)
        return -1;
    // A module that declares no memory keeps none (saves one page of heap).
    if (mem->init_page_count == 0)
        return 0;
    mem->num_bytes_per_page = bytes;
    mem->init_page_count = 1;
    mem->max_page_count = 1;
    return 1;
}

// For an AOT module loaded in indirect (XIP) mode straight from flash: point the
// function table at the instruction-bus alias of the text. Returns the number of
// pointers moved, -1 if the text is not in DROM.
int shim_aot_to_ibus(wasm_module_t m)
{
    WASMModuleCommon *c = (WASMModuleCommon *)m;
    AOTModule *a = (AOTModule *)m;
    uint32_t i, n = 0;
    if (c->module_type != Wasm_Module_AoT)
        return 0;
    if ((uint32_t)a->code < DROM_LOW || (uint32_t)a->code >= DROM_HIGH)
        return -1;
    for (i = 0; i < a->func_count; i++) {
        uint32_t p = (uint32_t)a->func_ptrs[i];
        if (p >= DROM_LOW && p < DROM_HIGH) {
            a->func_ptrs[i] = (void *)(p + IBUS_DELTA);
            n++;
        }
    }
    a->code = (void *)((uint32_t)a->code + IBUS_DELTA);
    return (int)n;
}

int shim_is_indirect(wasm_module_t m)
{
    WASMModuleCommon *c = (WASMModuleCommon *)m;
    return c->module_type == Wasm_Module_AoT && ((AOTModule *)m)->is_indirect_mode;
}
