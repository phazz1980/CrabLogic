// The C side of the WAMR backend of the Rust master: everything that needs WAMR structures or
// FreeRTOS internals, so the Rust side (src/board/engine.rs) deals with opaque pointers only.
#include <stdio.h>
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "wasm_export.h"
#include "wasm_runtime_common.h"
#include "wasm.h"
#include "aot_runtime.h"
#include "sdkconfig.h"
#if CONFIG_IDF_TARGET_ESP32
#include "esp_memory_utils.h"
#include "esp_log.h"
#include "esp_mmu_map.h"
#include "spi_flash_mmap.h"
#endif

// ESP32-C3: the instruction and data buses share one MMU table, so flash mapped at
// DROM 0x3C.. is also visible to the CPU at IROM 0x42.. with the same offset.
#define DROM_LOW 0x3C000000u
#define DROM_HIGH 0x3C800000u
#define IBUS_DELTA 0x06000000u

// Native stack kept free below a module call (WAMR checks it on every call).
#define STACK_GUARD 3072

// The Wasm C API is not built; wasm_runtime_common.c still references this on a path never taken.
void wasm_trap_delete(void *trap) { (void)trap; }

// Shrink the module's single memory to `bytes` (the module's `crab.mem`), like the C++ master
// does with wasm3's memoryLimit. 0 keeps the declared size.
static int set_memory(wasm_module_t m, uint32_t bytes)
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

#if CONFIG_IDF_TARGET_ESP32
// Classic ESP32: the two buses have separate MMU tables, so the text of each AOT module
// (literal pool + code) is mapped once more, for the instruction bus, while the module lives.
//
// Two modules may share a 64 KB flash page (areas of the storage are 4 KB): mapping a page that
// is already mapped gives no new mapping but a pointer into the existing one
// (ESP_ERR_INVALID_STATE). The module that made the mapping owns it; when it goes while a module
// that borrowed it still runs (a sheet replaced by a new version in the next area), the mapping
// passes to the borrower instead of being unmapped under its running code.
#define MAX_AOT 16
static struct {
    wasm_module_t module;
    // Start of the mapping it owns, or the address it got inside a borrowed one.
    void *vaddr;
    size_t len;
    int owned;
} s_ibus[MAX_AOT];
#endif

// For an AOT module loaded in indirect (XIP) mode straight from flash: point the function
// table at the instruction-bus alias of the text. Returns -1 if the text is not in DROM.
static int aot_to_ibus(AOTModule *a)
{
#if CONFIG_IDF_TARGET_ESP32C3
    uint32_t i;
    if ((uint32_t)a->code < DROM_LOW || (uint32_t)a->code >= DROM_HIGH)
        return -1;
    for (i = 0; i < a->func_count; i++) {
        uint32_t p = (uint32_t)a->func_ptrs[i];
        if (p >= DROM_LOW && p < DROM_HIGH)
            a->func_ptrs[i] = (void *)(p + IBUS_DELTA);
    }
    a->code = (void *)((uint32_t)a->code + IBUS_DELTA);
    return 0;
#elif CONFIG_IDF_TARGET_ESP32
    uint32_t i, slot, text_low, text_high;
    size_t phys, page;
    const void *ibus = NULL;
    int32_t delta;
    for (slot = 0; slot < MAX_AOT && s_ibus[slot].module; slot++)
        ;
    if (slot == MAX_AOT || !esp_ptr_in_drom(a->literal))
        return -1;
    // The literal pool sits right before the code: L32R reaches it pc-relative, so both go
    // to the instruction bus together, with the same distance between them.
    text_low = (uint32_t)a->literal;
    text_high = (uint32_t)a->code + a->code_size;
    phys = spi_flash_cache2phys(a->literal);
    if (phys == SPI_FLASH_CACHE2PHYS_FAIL)
        return -1;
    page = phys & ~(size_t)(SPI_FLASH_MMU_PAGE_SIZE - 1);
    {
        size_t len = phys - page + (text_high - text_low);
        void *ptr = NULL;
        esp_err_t err = esp_mmu_map(page, len, MMU_TARGET_FLASH0, MMU_MEM_CAP_EXEC | MMU_MEM_CAP_32BIT,
                                    ESP_MMU_MMAP_FLAG_PADDR_SHARED, &ptr);
        if (err != ESP_OK && err != ESP_ERR_INVALID_STATE)
            return -1;
        s_ibus[slot].owned = err == ESP_OK;
        s_ibus[slot].vaddr = ptr;
        s_ibus[slot].len = (len + SPI_FLASH_MMU_PAGE_SIZE - 1) & ~(size_t)(SPI_FLASH_MMU_PAGE_SIZE - 1);
        ibus = ptr;
    }
    delta = (int32_t)((uint32_t)ibus + (phys - page) - text_low);
    for (i = 0; i < a->func_count; i++) {
        uint32_t p = (uint32_t)a->func_ptrs[i];
        if (p >= text_low && p < text_high)
            a->func_ptrs[i] = (void *)(p + delta);
    }
    a->code = (void *)((uint32_t)a->code + delta);
    s_ibus[slot].module = (wasm_module_t)a;
    return 0;
#else
    (void)a;
    return -1;
#endif
}

// Unloads a module and, on the classic ESP32, drops its instruction-bus mapping.
void crab_wamr_unload(wasm_module_t m)
{
#if CONFIG_IDF_TARGET_ESP32
    uint32_t slot, other;
    for (slot = 0; slot < MAX_AOT; slot++) {
        if (s_ibus[slot].module != m)
            continue;
        s_ibus[slot].module = NULL;
        if (!s_ibus[slot].owned)
            continue;
        // A module still running inside this mapping takes it over.
        for (other = 0; other < MAX_AOT; other++) {
            uintptr_t at = (uintptr_t)s_ibus[other].vaddr, start = (uintptr_t)s_ibus[slot].vaddr;
            if (s_ibus[other].module && !s_ibus[other].owned && at >= start
                && at < start + s_ibus[slot].len)
                break;
        }
        if (other < MAX_AOT) {
            s_ibus[other].owned = 1;
            s_ibus[other].vaddr = s_ibus[slot].vaddr;
            s_ibus[other].len = s_ibus[slot].len;
        } else {
            esp_mmu_unmap(s_ibus[slot].vaddr);
        }
    }
#endif
    wasm_runtime_unload(m);
}

// Starts the runtime and registers the raw natives of the "crab" module. `natives` must stay
// alive and writable (WAMR sorts it in place).
int crab_wamr_init(NativeSymbol *natives, uint32_t count)
{
    RuntimeInitArgs args;
#if CONFIG_IDF_TARGET_ESP32
    // A borrowed mapping (see `s_ibus`) is expected: its warning would only clutter the console
    // the editor reads.
    esp_log_level_set("mmap", ESP_LOG_ERROR);
#endif
    memset(&args, 0, sizeof(args));
    args.mem_alloc_type = Alloc_With_System_Allocator;
    if (!wasm_runtime_full_init(&args))
        return 0;
    return wasm_runtime_register_natives_raw("crab", natives, count) ? 1 : 0;
}

// Loads a `.wasm` (the buffer is RAM and stays with the module) or an XIP `.aot` (the buffer
// is flash-mapped). `memory` is the module's `crab.mem`.
wasm_module_t crab_wamr_load(uint8_t *buf, uint32_t len, uint32_t memory, char *err, uint32_t err_len)
{
    wasm_module_t m = wasm_runtime_load(buf, len, err, err_len);
    if (!m)
        return NULL;
    if (((WASMModuleCommon *)m)->module_type == Wasm_Module_AoT) {
        AOTModule *a = (AOTModule *)m;
        if (!a->is_indirect_mode || aot_to_ibus(a) < 0) {
            snprintf(err, err_len, "AOT image is not an XIP image in flash");
            crab_wamr_unload(m);
            return NULL;
        }
    }
    set_memory(m, memory);
    return m;
}

int crab_wamr_is_aot(wasm_module_t m)
{
    return ((WASMModuleCommon *)m)->module_type == Wasm_Module_AoT;
}

// Import `index` of the module; returns 1 for a function import.
int crab_wamr_import(wasm_module_t m, int32_t index, const char **module, const char **name)
{
    wasm_import_t t;
    memset(&t, 0, sizeof(t));
    wasm_runtime_get_import_type(m, index, &t);
    *module = t.module_name;
    *name = t.name;
    return t.kind == WASM_IMPORT_EXPORT_KIND_FUNC;
}

// The execution environment of the calling task; modules must be called from this task.
wasm_exec_env_t crab_wamr_exec_env(wasm_module_inst_t inst, uint32_t stack)
{
    wasm_exec_env_t env = wasm_runtime_create_exec_env(inst, stack);
    if (env)
        wasm_runtime_set_native_stack_boundary(
            env, pxTaskGetStackStart(xTaskGetCurrentTaskHandle()) + STACK_GUARD);
    return env;
}

// The linear memory of an instance (NULL and 0 if it has none).
uint8_t *crab_wamr_memory(wasm_module_inst_t inst, uint32_t *len)
{
    wasm_memory_inst_t mem = wasm_runtime_get_default_memory(inst);
    *len = 0;
    if (!mem)
        return NULL;
    *len = (uint32_t)(wasm_memory_get_cur_page_count(mem) * wasm_memory_get_bytes_per_page(mem));
    return (uint8_t *)wasm_memory_get_base_address(mem);
}
