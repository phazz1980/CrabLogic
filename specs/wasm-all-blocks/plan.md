# WASM-вывод для всех блоков — план

## Решено (07.11.2026)
- Код блока для модуля живёт в `crab-blocks` рядом с `behavior()` и `emit_cpp()`: третья функция `emit_wasm`.
  Большой `match` в `crab-codegen/src/wasm.rs` для этих блоков не растёт, а сокращается.
- Совпадение модуля и симуляции проверяют существующие тесты `crab-sim/tests/wasm_emit.rs` (`compare` по фикстурам).

## Как устроено
- `crab-blocks` зависит только от `crab-core`; чтобы не тянуть в него сборщик модуля, там объявлен трейт
  `WasmEmit` — примитивы, которые нужны блоку: `input(i, ty)`, `param(i, ty)`, `set_out(i, ty)`, `out_get`,
  `state_i/state_f` (глобал состояния блока), `tmp(ty)`, `op(Instruction)`, `choose(...)`, `now()`.
  Реализует его эмиттер в `crab-codegen/src/wasm.rs` (методы, что уже есть: `input_as`, `tmp_f`, `choose`, `global`…).
- `Block` в `crab-blocks` получает поле `wasm: Option<fn(&mut dyn WasmEmit) -> Result<(), String>>`;
  `block()` в `wasm.rs` сначала смотрит на него, потом — на старый `match`.
- `crab-blocks` берёт зависимость `wasm-encoder` (та же версия, что в `crab-codegen`) ради типа `Instruction`.
- Числовая модель — как в `specs/numeric-model`: `bool`/`int` — i32, `float` — f32.
  `sin/cos/tan/sqrt/pow` в WASM нет: `sqrt` — `f32.sqrt`; `sin/cos/tan/pow` — импорт мастера
  (`math_f(op, a, b)`, новая функция мастера, версия ABI) или свой полином в модуле — решить в задаче 2.3.

## Этапы
1. Чистая логика без новых функций мастера: `abs`, `sq`, `sqrt`, биты и слова, `number_change`, `string_change`,
   `scale_table`, `pid`, `dms`, `decoder`, `encoder`, `timer_on_rs`, `random` (тот же xorshift, что `next_random`).
2. Математика с функциями libm: `sin`, `cos`, `tan`, `pow`.
3. Мастер: сохранение переменных и `flash_store` (новый импорт `retain_*`, версия ABI), `cycle_report`.
4. Строки между листами, `var_get`/`mux` со строками.
5. Расширители (`pcf8574_*`, вывод на расширителе), `kincony_kc868_a6_inputs`, смена формата UART,
   округление из входа, параметры из переменных.

## Риски
- Расхождение f32: `sin/cos/tan/pow` должны давать те же биты, что C++ на плате и `Behavior` на ПК.
- Мастер-прошивка и версия ABI: этапы 2–3 требуют новой прошивки мастера; старые модули остаются рабочими.
- Размер модуля: полиномы для libm увеличивают его; проверить лимит LittleFS.
