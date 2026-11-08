"""Generate Arduino stubs that pull WAMR sources into the sketch, and a gcc
response file with WAMR defines and include paths.

Usage (from wamr-c3/):  python tools/gen_wamr.py [classic|fast]
Writes sketch/src/wamr/*.c|*.S and build/flags.txt.
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WAMR = ROOT / ".deps" / "wamr" / "core"
OUT = ROOT / "sketch" / "src" / "wamr"
FLAGS = ROOT / "build" / "flags.txt"

interp = sys.argv[1] if len(sys.argv) > 1 else "classic"  # fast corrupts the heap on thermostat17

SOURCES = [
    "iwasm/common/wasm_application.c",
    "iwasm/common/wasm_blocking_op.c",
    "iwasm/common/wasm_exec_env.c",
    "iwasm/common/wasm_loader_common.c",
    "iwasm/common/wasm_memory.c",
    "iwasm/common/wasm_native.c",
    "iwasm/common/wasm_runtime_common.c",
    "iwasm/common/arch/invokeNative_riscv.S",
    "iwasm/interpreter/wasm_loader.c",
    "iwasm/interpreter/wasm_runtime.c",
    f"iwasm/interpreter/wasm_interp_{interp}.c",
    "iwasm/aot/aot_intrinsic.c",
    "iwasm/aot/aot_loader.c",
    "iwasm/aot/aot_runtime.c",
    "iwasm/aot/arch/aot_reloc_riscv.c",
    "shared/mem-alloc/mem_alloc.c",
    "shared/platform/esp-idf/espidf_clock.c",
    "shared/platform/esp-idf/espidf_malloc.c",
    "shared/platform/esp-idf/espidf_memmap.c",
    "shared/platform/esp-idf/espidf_platform.c",
    "shared/platform/esp-idf/espidf_thread.c",
    "shared/platform/esp-idf/espidf_socket.c",
    "shared/platform/esp-idf/espidf_file.c",
    "shared/platform/common/libc-util/libc_errno.c",
]
SOURCES += [f"shared/mem-alloc/ems/{p.name}" for p in sorted((WAMR / "shared/mem-alloc/ems").glob("*.c"))]
SOURCES += [f"shared/utils/{p.name}" for p in sorted((WAMR / "shared/utils").glob("*.c"))]

INCLUDES = [
    "iwasm/include", "iwasm/common", "iwasm/interpreter", "iwasm/aot",
    "shared/utils", "shared/mem-alloc", "shared/platform/esp-idf",
    "shared/platform/include", "shared/platform/common/libc-util",
]

DEFINES = [
    "BH_PLATFORM_ESP_IDF", "BUILD_TARGET_RISCV32_ILP32",
    "WASM_ENABLE_INTERP=1", f"WASM_ENABLE_FAST_INTERP={1 if interp == 'fast' else 0}",
    "WASM_ENABLE_AOT=1", "WASM_ENABLE_LIBC_BUILTIN=0", "WASM_ENABLE_LIBC_WASI=0",
    "WASM_ENABLE_MULTI_MODULE=0", "WASM_ENABLE_BULK_MEMORY=1", "WASM_ENABLE_SHARED_MEMORY=0",
    "WASM_ENABLE_MINI_LOADER=0", "WASM_DISABLE_HW_BOUND_CHECK=1",
    "WASM_DISABLE_STACK_HW_BOUND_CHECK=1", "WASM_DISABLE_WAKEUP_BLOCKING_OP=0",
    "WASM_ENABLE_SIMD=0", "WASM_ENABLE_REF_TYPES=0", "WASM_ENABLE_THREAD_MGR=0",
    "BH_MALLOC=wasm_runtime_malloc", "BH_FREE=wasm_runtime_free",
]

OUT.mkdir(parents=True, exist_ok=True)
for old in OUT.iterdir():
    old.unlink()
names = set()
for rel in SOURCES:
    src = WAMR / rel
    assert src.exists(), src
    name = src.name
    assert name not in names, name
    names.add(name)
    (OUT / name).write_text(f'#include "{src.as_posix()}"\n', encoding="utf-8")

lines = [f"-D{d}" for d in DEFINES] + [f'"-I{(WAMR / i).as_posix()}"' for i in INCLUDES]
lines.append("-Wno-format")
FLAGS.parent.mkdir(exist_ok=True)
FLAGS.write_text(" ".join(lines) + "\n", encoding="utf-8")
print(f"{len(SOURCES)} stubs ({interp}), flags -> {FLAGS}")
