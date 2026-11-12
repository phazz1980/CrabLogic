# bh1750 — драйвер датчика по I²C

Эталон драйвера: параметры-выводы с `bus = "i2c-sda"` / `"i2c-scl"`, адрес с `bus = "i2c-address"`,
`i2c_open` в `init()`, шаг без ожидания (читаем по периоду, а не ждём измерения), при ошибке `ok = 0`
и повтор позже. Значения `sim` в `block.toml` — то, что блок показывает в симуляции на ПК.

Команды — из папки программы (где `library/`); в репозитории исходников вместо `crablogic` —
`cargo run -q -p crab-cli --`.

- Сборка: `powershell -NoProfile -ExecutionPolicy Bypass -File skills/crablogic-block/examples/bh1750/build.ps1`
  (кладёт `project/library/bh1750/block.wasm`; блок лежит в `project/library/` — так проект несёт свои блоки).
- Проверка блока: `crablogic --library library block check skills/crablogic-block/examples/bh1750/project/library/bh1750`.
- Проверка проекта: `crablogic --library library check skills/crablogic-block/examples/bh1750/project`
  и `crablogic --library library gen-wasm skills/crablogic-block/examples/bh1750/project`.
- На железе не проверено.
