# WASM-вывод для всех блоков — задачи

- [x] 1.1 Трейт `WasmEmit` и поле `Block::wasm` в `crab-blocks`; реализация в `wasm.rs`. Проверка: `cargo test -p crab-codegen`, прежние снимки не меняются.
- [x] 1.2 `abs`, `sq`, `sqrt` + фикстура `wasm_math`, тест `compare`.
- [x] 1.3 Биты (`bit_*`) и слова (`bits_to_byte`, `byte_to_bits`, `*_to_*`) + фикстура `wasm_bits`.
- [x] 1.4 `number_change`, `string_change`, `scale_table`, `random`.
- [x] 1.5 `pid`, `dms`, `decoder`, `encoder` (`timer_on_rs` — блок-плагин на block.wasm, отдельно).
- [ ] 1.6 `selftest_logic`, `selftest_logic2` собираются `gen-wasm`; справка, `docs.md` блоков.
- [x] 2.1 `sin`, `cos`, `tan`, `pow`: импорт мастера `math(op, a, b)` (разрешение `math` в манифесте, `P_MATH` в мастере); тест `trigonometry_and_pow_go_through_the_master`. Совпадение с `sinf` на плате не проверено на железе.
- [x] 3.1 Сохранение числовых и логических переменных и `flash_store` в мастере: импорт `retain(op, key, value)` (NVS `crab_mod`), права `retain`; тесты `flash_store_matches_the_simulation`, `retained_variables_are_loaded_and_saved_through_the_master`. Строковые переменные — в 4.1. На плате не проверено.
- [x] 3.2 `cycle_report` (в модуле ничего не считает; цикл показывает мастер).
- [x] 4.1 Строки в `switch`, `mux`, `dms`, `var_get`, `var_set` и строковые переменные листа (тест `text_selectors_and_variables_match_the_simulation`); в симуляции `switch`/`mux` теперь передают текст, а текстовая константа на порту `same` — текстом.
- [ ] 4.2 Строки между листами (теги-строки в мастере) и сохраняемые строковые переменные (`text_commands`, `wifi_bound`).
- [ ] 5.1 Расширители и `kincony_kc868_a6_inputs` (`pcf_pins`, `kc868_a6_relays`).
- [ ] 5.2 Смена формата UART, округление и параметры из входов/переменных.
- [ ] 5.3 Пометить блоки, которым нужна новая прошивка мастера (импорт/функция мастера, версия ABI): поле `needs_master` (версия ABI) в `block.toml`/`Block`; пометка в библиотеке блоков и справке; при `gen-wasm`/заливке — понятная ошибка «перезалейте мастер», если версия на плате ниже. Сейчас такие: `sin/cos/tan/pow` (`math`, 2.1); дальше — 3.1, 3.2, 4.1, 5.1, 5.2. Проверка: тест на блок с `needs_master`, снимок сообщения.
- [ ] 6 Сверка: `gen-wasm` на всех примерах, `selftest_*` на ESP32-C3, обновить `Архитектура и план.md`.
