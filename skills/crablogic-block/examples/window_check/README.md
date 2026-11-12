# window_check — логика без памяти

Самый простой пользовательский блок на WASM: читает вход и параметры, пишет выходы, ничего не помнит.
Показывает порядок номеров (`input`/`param`/`output` — по порядку в `block.toml`) и `bool` как 0/1.

Команды — из папки программы (где `library/`); в репозитории исходников вместо `crablogic` —
`cargo run -q -p crab-cli --`.

- Проверка поведения на ПК: `cargo test` в этой папке.
- Сборка: `powershell -NoProfile -ExecutionPolicy Bypass -File skills/crablogic-block/examples/window_check/build.ps1`
  (кладёт `project/library/window_check/block.wasm`; блок лежит в `project/library/` — так проект несёт свои блоки).
- Проверка блока: `crablogic --library library block check skills/crablogic-block/examples/window_check/project/library/window_check`.
- Проверка проекта: `crablogic --library library check skills/crablogic-block/examples/window_check/project`
  и `crablogic --library library gen-wasm skills/crablogic-block/examples/window_check/project`.
