# WAMR на ESP32-C3 — замер ОЗУ и скорости (этап 0 `specs/wasm-kernel`)

Прототип сравнивает WAMR (классический интерпретатор и AOT с XIP) с wasm3 и wasmi. Результаты — в `../../plan.md`,
раздел «WAMR на ESP32-C3».

Собран на ядре Arduino (`arduino-cli`, esp32 3.3.12 — внутри ESP-IDF 5.x), а не на чистом ESP-IDF: arduino-cli уже
стоит, а слой платформы WAMR (`shared/platform/esp-idf`) тот же. Для замера рантайма разницы нет.

## Подготовка (один раз)

`.deps/` не в git:
- `.deps/wamr/` — исходники WAMR 2.4.5 (`git clone --depth 1 -b WAMR-2.4.5 https://github.com/bytecodealliance/wasm-micro-runtime .deps/wamr`);
- `.deps/wamrc/wamrc.exe` — из `wamrc-2.4.5-x86_64-windows-2022.zip` с релиза WAMR на GitHub.

```
python tools/gen_wamr.py            # заглушки sketch/src/wamr/*.c и build/flags.txt (classic; "fast" — быстрый интерпретатор)
cargo run -q --manifest-path tools/prep/Cargo.toml   # modules/*.wasm|aot|aotnb и sketch/modules.h
```

`modules/thermostat17.wasm`, `selftest42.wasm`, `selftest107.wasm` — те же файлы, что в `../wasmi-c3/modules/`
(выход `crab-cli gen-wasm`); `blink`, `counter`, `hang` собираются из `../*.wat`.

Замеры ускорения AOT (plan.md, «Ускорение шага AOT»): микромодули `wat/calls7.wat`, `calls1.wat`, `loop100.wat`
и варианты листов с обёртками импортов `wat/selftest*w.wat` (обёртка-функция) / `*wi.wat` (`call_indirect`).
Последние получаются так: `prep print` → `build/sz/s107.wat`, затем
`python tools/wrap_calls.py build/sz/s107.wat wat/selftest107` (пишет `…w.wat` и `…wi.wat`).

## Сборка и заливка (bash, из этой папки)

```
FQBN="esp32:esp32:esp32c3:CDCOnBoot=cdc,FlashMode=qio,FlashFreq=80,PartitionScheme=huge_app"
F="@$(pwd -W)/build/flags.txt"
arduino-cli compile --fqbn "$FQBN" --build-path build/out \
  --build-property "compiler.c.extra_flags=$F" --build-property "compiler.cpp.extra_flags=$F" \
  --build-property "compiler.S.extra_flags=$F" sketch
arduino-cli upload -p COM5 --fqbn "$FQBN" --input-dir build/out sketch
```

После смены `gen_wamr.py classic|fast` удалите `build/out/sketch`: arduino-cli не видит смены флагов в файле
и не убирает старые заглушки.

Raw-нативы (`wasm_runtime_register_natives_raw`): добавьте ` -DCRAB_RAW=1` в конец копии `build/flags.txt`, укажите её
в `F=` и тоже удалите `build/out/sketch`. Заголовок вывода покажет `raw natives`.

Вывод — USB-консоль 115200, в конце строка `######## done`. Прошивка ≈ 587 КБ, из них ≈ 172 КБ — образы модулей.

## Устройство

- `sketch/bench.cpp` — замеры: куча на модуль и шаг, все модули сразу, 1000 замен с контролем дрейфа кучи.
  Рантайм запускается в pthread (WAMR под ESP-IDF вызывает `pthread_self`), граница стека задаётся один раз
  (`wasm_runtime_set_native_stack_boundary`), иначе каждый вызов сканирует стек FreeRTOS.
- `sketch/src/shim.c` — доступ к внутренностям WAMR: размер памяти из `crab.mem` (как `memoryLimit` у wasm3)
  и перевод указателей функций AOT с шины данных (0x3C…) на шину команд (0x42…) для XIP.
- `sketch/src/capi_stub.c` — заглушка `wasm_trap_delete` (c-api не собираем).
- Быстрый интерпретатор (`fast`) портит кучу на thermostat17 — по умолчанию classic (см. plan.md).
