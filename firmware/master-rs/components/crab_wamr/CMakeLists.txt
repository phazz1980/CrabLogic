# WAMR (classic interpreter + AOT loader) as an ESP-IDF component, with the same sources and
# defines as the stage 0 prototype (specs/wasm-on-mcu/proto/wamr-c3), plus the CrabLogic shim.
# RISC-V (ESP32-C3) only for now; Xtensa comes with task 3.5.
set(WAMR_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../.deps/wamr")
set(CORE "${WAMR_ROOT}/core")
if(NOT EXISTS "${CORE}/iwasm/include/wasm_export.h")
  message(FATAL_ERROR "WAMR sources not found in ${WAMR_ROOT}: see firmware/master-rs/README.md")
endif()

file(GLOB EMS "${CORE}/shared/mem-alloc/ems/*.c")
file(GLOB UTILS "${CORE}/shared/utils/*.c")

idf_component_register(
  SRCS
    "shim.c"
    "${CORE}/iwasm/common/wasm_application.c"
    "${CORE}/iwasm/common/wasm_blocking_op.c"
    "${CORE}/iwasm/common/wasm_exec_env.c"
    "${CORE}/iwasm/common/wasm_loader_common.c"
    "${CORE}/iwasm/common/wasm_memory.c"
    "${CORE}/iwasm/common/wasm_native.c"
    "${CORE}/iwasm/common/wasm_runtime_common.c"
    "${CORE}/iwasm/common/arch/invokeNative_riscv.S"
    "${CORE}/iwasm/interpreter/wasm_loader.c"
    "${CORE}/iwasm/interpreter/wasm_runtime.c"
    # The fast interpreter corrupts the heap on thermostat17 (stage 0), so classic it is.
    "${CORE}/iwasm/interpreter/wasm_interp_classic.c"
    "${CORE}/iwasm/aot/aot_intrinsic.c"
    "${CORE}/iwasm/aot/aot_loader.c"
    "${CORE}/iwasm/aot/aot_runtime.c"
    "${CORE}/iwasm/aot/arch/aot_reloc_riscv.c"
    "${CORE}/shared/mem-alloc/mem_alloc.c"
    "${CORE}/shared/platform/esp-idf/espidf_clock.c"
    "${CORE}/shared/platform/esp-idf/espidf_malloc.c"
    "${CORE}/shared/platform/esp-idf/espidf_memmap.c"
    "${CORE}/shared/platform/esp-idf/espidf_platform.c"
    "${CORE}/shared/platform/esp-idf/espidf_thread.c"
    "${CORE}/shared/platform/esp-idf/espidf_socket.c"
    "${CORE}/shared/platform/esp-idf/espidf_file.c"
    "${CORE}/shared/platform/common/libc-util/libc_errno.c"
    ${EMS}
    ${UTILS}
  INCLUDE_DIRS
    "${CORE}/iwasm/include"
  PRIV_INCLUDE_DIRS
    "${CORE}/iwasm/common"
    "${CORE}/iwasm/interpreter"
    "${CORE}/iwasm/aot"
    "${CORE}/shared/utils"
    "${CORE}/shared/mem-alloc"
    "${CORE}/shared/platform/esp-idf"
    "${CORE}/shared/platform/include"
    "${CORE}/shared/platform/common/libc-util"
  PRIV_REQUIRES pthread lwip esp_timer vfs freertos)

target_compile_definitions(${COMPONENT_LIB} PRIVATE
  BH_PLATFORM_ESP_IDF
  BUILD_TARGET_RISCV32_ILP32
  WASM_ENABLE_INTERP=1
  WASM_ENABLE_FAST_INTERP=0
  WASM_ENABLE_AOT=1
  WASM_ENABLE_LIBC_BUILTIN=0
  WASM_ENABLE_LIBC_WASI=0
  WASM_ENABLE_MULTI_MODULE=0
  WASM_ENABLE_BULK_MEMORY=1
  WASM_ENABLE_SHARED_MEMORY=0
  WASM_ENABLE_MINI_LOADER=0
  WASM_DISABLE_HW_BOUND_CHECK=1
  WASM_DISABLE_STACK_HW_BOUND_CHECK=1
  WASM_DISABLE_WAKEUP_BLOCKING_OP=0
  WASM_ENABLE_SIMD=0
  WASM_ENABLE_REF_TYPES=0
  WASM_ENABLE_THREAD_MGR=0
  BH_MALLOC=wasm_runtime_malloc
  BH_FREE=wasm_runtime_free)

target_compile_options(${COMPONENT_LIB} PRIVATE -Wno-format -Wno-error)
