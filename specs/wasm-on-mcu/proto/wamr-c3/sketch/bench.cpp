// WAMR on ESP32-C3: heap, step time and hot swap for the same modules as proto/wasmi-c3.
// Modes: "interp" (fast or classic interpreter, .wasm copied to RAM), "aot" (XIP from
// flash, software bounds checks), "aotnb" (XIP, no bounds checks).
#include <Arduino.h>
#include <esp_heap_caps.h>
#include <pthread.h>
#include <freertos/FreeRTOS.h>
#include <freertos/task.h>
#include <freertos/idf_additions.h>
#include "wasm_export.h"
#include "modules.h"

extern "C" {
int shim_set_memory(wasm_module_t m, uint32_t bytes);
int shim_aot_to_ibus(wasm_module_t m);
int shim_is_indirect(wasm_module_t m);
}

// Wasm operand/frame stack per instance (AOT keeps code frames on the native stack).
static const uint32_t kStack = 4096;
static const int kSteps = 1000;
// Native stack kept free below the boundary (WAMR's WASM_STACK_GUARD_SIZE idea).
static const uint32_t kGuard = 3072;
static const int kSwaps = 1000;
#ifndef CRAB_RAW
#define CRAB_RAW 0
#endif

// ---- host functions (module "crab"), same behaviour as proto/wasmi-c3 ----
static float g_inputs[16], g_params[16], g_tags[32];
static int32_t g_pins[32];
static uint32_t g_calls;  // host calls, to report calls per step
#define CNT g_calls++

static int32_t h_adc_read(wasm_exec_env_t, int32_t) { CNT; return 1234; }
static void h_gpio_write(wasm_exec_env_t, int32_t pin, int32_t v) { CNT; g_pins[pin & 31] = v; }
static float h_input_f(wasm_exec_env_t, int32_t i) { CNT; return g_inputs[i & 15]; }
static int32_t h_input_i(wasm_exec_env_t, int32_t i) { CNT; return (int32_t)g_inputs[i & 15]; }
static float h_math(wasm_exec_env_t, int32_t op, float a, float b) { CNT;
  switch (op) {
    case 0: return sqrtf(a);
    case 1: return powf(a, b);
    case 2: return sinf(a);
    case 3: return cosf(a);
    default: return a;
  }
}
static int32_t h_millis(wasm_exec_env_t) { CNT; return (int32_t)millis(); }
static float h_output_f(wasm_exec_env_t, int32_t, float v) { CNT; return v; }
static int32_t h_output_i(wasm_exec_env_t, int32_t, int32_t v) { CNT; return v; }
static float h_param_f(wasm_exec_env_t, int32_t i) { CNT; return g_params[i & 15]; }
static void h_publish(wasm_exec_env_t, int32_t, int32_t, int32_t, int32_t) { CNT;}
static void h_pwm_write(wasm_exec_env_t, int32_t, int32_t) { CNT;}
static void h_subscribe(wasm_exec_env_t, int32_t, int32_t) { CNT;}
static float h_tag_get_f(wasm_exec_env_t, int32_t i) { CNT; return g_tags[i & 31]; }
static int32_t h_tag_get_i(wasm_exec_env_t, int32_t i) { CNT; return (int32_t)g_tags[i & 31]; }
static void h_tag_set_f(wasm_exec_env_t, int32_t i, float v) { CNT; g_tags[i & 31] = v; }
static void h_tag_set_i(wasm_exec_env_t, int32_t i, int32_t v) { CNT; g_tags[i & 31] = (float)v; }
static int32_t h_text(wasm_exec_env_t, int32_t, int32_t, int32_t, int32_t, int32_t, int32_t, int32_t) { CNT; return 0; }
static int32_t h_uart(wasm_exec_env_t, int32_t, int32_t, int32_t, int32_t) { CNT; return 0; }

#if CRAB_RAW
// Raw natives: WAMR passes arguments as a uint64 slot array and skips signature-driven marshalling.
#define A_I(n) ((int32_t)args[n])
#define A_F(n) (*(float *)&args[n])
#define R_I(x) (*(int32_t *)&args[0] = (x))
#define R_F(x) (*(float *)&args[0] = (x))
static void r_adc_read(wasm_exec_env_t e, uint64_t *args) { R_I(h_adc_read(e, A_I(0))); }
static void r_gpio_write(wasm_exec_env_t e, uint64_t *args) { h_gpio_write(e, A_I(0), A_I(1)); }
static void r_input_f(wasm_exec_env_t e, uint64_t *args) { R_F(h_input_f(e, A_I(0))); }
static void r_input_i(wasm_exec_env_t e, uint64_t *args) { R_I(h_input_i(e, A_I(0))); }
static void r_math(wasm_exec_env_t e, uint64_t *args) { R_F(h_math(e, A_I(0), A_F(1), A_F(2))); }
static void r_millis(wasm_exec_env_t e, uint64_t *args) { R_I(h_millis(e)); }
static void r_output_f(wasm_exec_env_t e, uint64_t *args) { R_F(h_output_f(e, A_I(0), A_F(1))); }
static void r_output_i(wasm_exec_env_t e, uint64_t *args) { R_I(h_output_i(e, A_I(0), A_I(1))); }
static void r_param_f(wasm_exec_env_t e, uint64_t *args) { R_F(h_param_f(e, A_I(0))); }
static void r_publish(wasm_exec_env_t e, uint64_t *args) { h_publish(e, A_I(0), A_I(1), A_I(2), A_I(3)); }
static void r_pwm_write(wasm_exec_env_t e, uint64_t *args) { h_pwm_write(e, A_I(0), A_I(1)); }
static void r_subscribe(wasm_exec_env_t e, uint64_t *args) { h_subscribe(e, A_I(0), A_I(1)); }
static void r_tag_get_f(wasm_exec_env_t e, uint64_t *args) { R_F(h_tag_get_f(e, A_I(0))); }
static void r_tag_get_i(wasm_exec_env_t e, uint64_t *args) { R_I(h_tag_get_i(e, A_I(0))); }
static void r_tag_set_f(wasm_exec_env_t e, uint64_t *args) { h_tag_set_f(e, A_I(0), A_F(1)); }
static void r_tag_set_i(wasm_exec_env_t e, uint64_t *args) { h_tag_set_i(e, A_I(0), A_I(1)); }
static void r_text(wasm_exec_env_t e, uint64_t *args) { R_I(h_text(e, A_I(0), A_I(1), A_I(2), A_I(3), A_I(4), A_I(5), A_I(6))); }
static void r_uart(wasm_exec_env_t e, uint64_t *args) { R_I(h_uart(e, A_I(0), A_I(1), A_I(2), A_I(3))); }
#define SYM(name, sig) {#name, (void *)r_##name, sig, nullptr}
#else
#define SYM(name, sig) {#name, (void *)h_##name, sig, nullptr}
#endif
static NativeSymbol g_natives[] = {
  SYM(adc_read, "(i)i"), SYM(gpio_write, "(ii)"), SYM(input_f, "(i)f"), SYM(input_i, "(i)i"),
  SYM(math, "(iff)f"), SYM(millis, "()i"), SYM(output_f, "(if)f"), SYM(output_i, "(ii)i"),
  SYM(param_f, "(i)f"), SYM(publish, "(iiii)"), SYM(pwm_write, "(ii)"), SYM(subscribe, "(ii)"),
  SYM(tag_get_f, "(i)f"), SYM(tag_get_i, "(i)i"), SYM(tag_set_f, "(if)"), SYM(tag_set_i, "(ii)"),
  SYM(text, "(iiiiiii)i"), SYM(uart, "(iiii)i"),
};

// ---- helpers ----
static size_t freeHeap() { return heap_caps_get_free_size(MALLOC_CAP_8BIT); }
static size_t largest() { return heap_caps_get_largest_free_block(MALLOC_CAP_8BIT); }

enum Mode { INTERP, AOT, AOTNB };
static const char *kModeName[] = {"interp", "aot", "aotnb"};

struct Mod {
  uint8_t *copy = nullptr;  // RAM copy of the .wasm (interpreter only)
  wasm_module_t module = nullptr;
  wasm_module_inst_t inst = nullptr;
  wasm_exec_env_t env = nullptr;
  wasm_function_inst_t step = nullptr;
  size_t heap_load = 0, heap_inst = 0, heap_step1 = 0;
};

static char g_err[128];

static void unloadMod(Mod &m) {
  if (m.env) wasm_runtime_destroy_exec_env(m.env);
  if (m.inst) wasm_runtime_deinstantiate(m.inst);
  if (m.module) wasm_runtime_unload(m.module);
  if (m.copy) free(m.copy);
  m = Mod();
}

static bool callStep(Mod &m) {
  uint32_t argv[1] = {10};
  return wasm_runtime_call_wasm(m.env, m.step, 1, argv);
}

static bool callVoid(Mod &m, const char *name, int results) {
  wasm_function_inst_t f = wasm_runtime_lookup_function(m.inst, name);
  if (!f) return true;
  uint32_t argv[1] = {0};
  (void)results;
  return wasm_runtime_call_wasm(m.env, f, 0, argv);
}

static bool loadMod(const Blob &b, Mode mode, Mod &m, bool verbose) {
  size_t h0 = freeHeap();
  const uint8_t *src;
  size_t len;
  if (mode == INTERP) {
    // The interpreter loader rewrites bytecode, so the buffer must be writable RAM.
    m.copy = (uint8_t *)malloc(b.wasm_len);
    if (!m.copy) { Serial.println("  copy: out of memory"); return false; }
    memcpy(m.copy, b.wasm, b.wasm_len);
    src = m.copy;
    len = b.wasm_len;
  } else {
    src = mode == AOT ? b.aot : b.aotnb;
    len = mode == AOT ? b.aot_len : b.aotnb_len;
    if (!src) { Serial.println("  no aot image"); return false; }
  }
  m.module = wasm_runtime_load((uint8_t *)src, len, g_err, sizeof(g_err));
  if (!m.module) { Serial.printf("  load: %s\n", g_err); unloadMod(m); return false; }
  if (mode != INTERP) {
    int moved = shim_aot_to_ibus(m.module);
    if (verbose) Serial.printf("  xip: indirect=%d, func ptrs -> ibus: %d\n", shim_is_indirect(m.module), moved);
    if (moved < 0) { Serial.println("  aot text not in DROM"); unloadMod(m); return false; }
  }
  shim_set_memory(m.module, b.mem);
  m.heap_load = h0 - freeHeap();
  m.inst = wasm_runtime_instantiate(m.module, kStack, 0, g_err, sizeof(g_err));
  if (!m.inst) { Serial.printf("  instantiate: %s\n", g_err); unloadMod(m); return false; }
  m.env = wasm_runtime_create_exec_env(m.inst, kStack);
  if (!m.env) { Serial.println("  exec env failed"); unloadMod(m); return false; }
  // Otherwise WAMR asks vTaskGetInfo() on every call, which scans the whole task stack.
  wasm_runtime_set_native_stack_boundary(m.env, pxTaskGetStackStart(xTaskGetCurrentTaskHandle()) + kGuard);
  m.heap_inst = h0 - freeHeap() - m.heap_load;
  m.step = wasm_runtime_lookup_function(m.inst, "step");
  if (!m.step) { Serial.println("  no step"); unloadMod(m); return false; }
  if (!callVoid(m, "abi_version", 1) || !callVoid(m, "init", 0)) {
    Serial.printf("  init: %s\n", wasm_runtime_get_exception(m.inst));
    unloadMod(m);
    return false;
  }
  if (!callStep(m)) {
    Serial.printf("  step: %s\n", wasm_runtime_get_exception(m.inst));
    unloadMod(m);
    return false;
  }
  m.heap_step1 = h0 - freeHeap() - m.heap_load - m.heap_inst;
  return true;
}

static void measureHeapAndStep(Mode mode) {
  Serial.printf("\n== %s: heap per module (bytes) and step time ==\n", kModeName[mode]);
  Serial.println("module          code    load    inst  +step1   total  dropped  step_us  load_us  calls");
  for (const Blob &b : BLOBS) {
    size_t before = freeHeap();
    uint32_t t0 = micros();
    Mod m;
    if (!loadMod(b, mode, m, &b == &BLOBS[0])) { Serial.printf("%-12s failed\n", b.name); continue; }
    uint32_t load_us = micros() - t0;
    for (int i = 0; i < 10; i++) callStep(m);
    g_calls = 0;
    uint32_t t1 = micros();
    bool ok = true;
    for (int i = 0; i < kSteps; i++) ok &= callStep(m);
    uint32_t step_ns = (micros() - t1) * 1000 / kSteps;
    uint32_t calls = g_calls / kSteps;
    size_t total = before - freeHeap();
    Mod snap = m;  // unloadMod resets m
    unloadMod(m);
    m = snap;
    long dropped = (long)before - (long)freeHeap();
    size_t code = mode == INTERP ? b.wasm_len : (mode == AOT ? b.aot_len : b.aotnb_len);
    Serial.printf("%-12s %7u %7u %7u %7u %7u %8ld %5u.%02u %8u %6u%s\n", b.name, (unsigned)code,
                  (unsigned)m.heap_load, (unsigned)m.heap_inst, (unsigned)m.heap_step1, (unsigned)total,
                  dropped, (unsigned)(step_ns / 1000), (unsigned)(step_ns % 1000 / 10), (unsigned)load_us,
                  (unsigned)calls, ok ? "" : "  (trap)");
  }
}

static void measureSwap(Mode mode, const Blob &b) {
  size_t f0 = freeHeap(), l0 = largest();
  uint32_t tsum = 0;
  for (int i = 1; i <= kSwaps; i++) {
    uint32_t t = micros();
    Mod m;
    if (!loadMod(b, mode, m, false)) { Serial.printf("  swap %d failed\n", i); return; }
    unloadMod(m);
    tsum += micros() - t;
    long drift = (long)f0 - (long)freeHeap();
    if (drift > 20000) { Serial.printf("  drift %ld after %d swaps, stop\n", drift, i); return; }
  }
  Serial.printf("%-6s %-12s swaps %d: %u us/swap, free %u -> %u, largest %u -> %u\n", kModeName[mode], b.name,
                kSwaps, (unsigned)(tsum / kSwaps), (unsigned)f0, (unsigned)freeHeap(), (unsigned)l0,
                (unsigned)largest());
}

static void measureAllTogether(Mode mode) {
  size_t f0 = freeHeap();
  Mod mods[sizeof(BLOBS) / sizeof(BLOBS[0])];
  int n = 0;
  for (int k = 0; k < 5; k++)  // the five sheet modules, not the micro benchmarks
    if (loadMod(BLOBS[k], mode, mods[n], false)) n++;
  Serial.printf("%-6s all %d modules together: %u bytes\n", kModeName[mode], n, (unsigned)(f0 - freeHeap()));
  for (int i = 0; i < n; i++) unloadMod(mods[i]);
}

static void *benchMain(void *) {
  Serial.begin(115200);
  delay(2500);
  Serial.printf("\nWAMR prototype on ESP32-C3, %s interpreter, %s natives, stack %u\n",
                WASM_ENABLE_FAST_INTERP ? "fast" : "classic", CRAB_RAW ? "raw" : "plain", (unsigned)kStack);
  Serial.printf("heap before runtime: free %u, largest %u\n", (unsigned)freeHeap(), (unsigned)largest());
  size_t h0 = freeHeap();
  RuntimeInitArgs args;
  memset(&args, 0, sizeof(args));
  args.mem_alloc_type = Alloc_With_System_Allocator;
#if !CRAB_RAW
  args.native_module_name = "crab";
  args.native_symbols = g_natives;
  args.n_native_symbols = sizeof(g_natives) / sizeof(g_natives[0]);
#endif
  if (!wasm_runtime_full_init(&args)) { Serial.println("runtime init failed"); return nullptr; }
#if CRAB_RAW
  if (!wasm_runtime_register_natives_raw("crab", g_natives, sizeof(g_natives) / sizeof(g_natives[0]))) {
    Serial.println("register raw natives failed");
    return nullptr;
  }
#endif
  Serial.printf("runtime init: %u bytes, free %u\n", (unsigned)(h0 - freeHeap()), (unsigned)freeHeap());
  for (int i = 0; i < 32; i++) g_tags[i] = i;
  for (int i = 0; i < 16; i++) { g_inputs[i] = i * 0.5f; g_params[i] = 1.0f + i; }

  for (Mode mode : {INTERP, AOT, AOTNB}) measureHeapAndStep(mode);
  Serial.println("\n== together ==");
  for (Mode mode : {INTERP, AOT, AOTNB}) measureAllTogether(mode);
  Serial.println("\n== hot swap (load + instantiate + init + step + unload) ==");
  for (Mode mode : {INTERP, AOT}) {
    measureSwap(mode, BLOBS[2]);  // thermostat17
    measureSwap(mode, BLOBS[4]);  // selftest107
  }
  Serial.printf("\nfinal heap: free %u, largest %u\nok\n######## done\n", (unsigned)freeHeap(), (unsigned)largest());
  return nullptr;
}

// WAMR's ESP-IDF port calls pthread_self(), so the runtime must live in a pthread.
void bench_setup() {
  pthread_attr_t attr;
  pthread_attr_init(&attr);
  pthread_attr_setstacksize(&attr, 32768);
  pthread_t t;
  if (pthread_create(&t, &attr, benchMain, nullptr) != 0) { Serial.println("pthread_create failed"); return; }
  pthread_join(t, nullptr);
}
