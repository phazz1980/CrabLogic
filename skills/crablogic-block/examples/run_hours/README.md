# run_hours — логика с памятью и параметрами

Блок с `memory = true`: наработка и число запусков лежат в `static`, `init()` их сбрасывает,
фронт `run` находится сравнением с предыдущим шагом, время идёт из `dt_ms`.

Команды — из папки программы (где `library/`); в репозитории исходников вместо `crablogic` —
`cargo run -q -p crab-cli --`.

- Проверка поведения на ПК: `cargo test` в этой папке.
- Сборка: `powershell -NoProfile -ExecutionPolicy Bypass -File skills/crablogic-block/examples/run_hours/build.ps1`
  (кладёт `project/library/run_hours/block.wasm`; блок лежит в `project/library/` — так проект несёт свои блоки).
- Проверка блока: `crablogic --library library block check skills/crablogic-block/examples/run_hours/project/library/run_hours`.
- Проверка проекта: `crablogic --library library check skills/crablogic-block/examples/run_hours/project`
  и `crablogic --library library gen-wasm skills/crablogic-block/examples/run_hours/project`.
