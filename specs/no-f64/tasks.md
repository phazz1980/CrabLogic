# Задачи: без f64 на плате

1. [x] Модуль листа: округление `float → int` в `f32`, остаток через `crab.math(4)`; `crab-kernel`
   `fmodf` и `ABI_VERSION = 5`; симуляция — `f32 %`. Проверка: тест краёв эмиттера (как
   `numeric_edges_match_the_simulation`), тест «в модуле нет f64» на `examples/*` без драйверов. Готово: тест `float_rounding_and_remainder_edges_match_the_simulation`, `example_modules_have_no_f64` (42 модуля; пропущены проекты с oled/lcd/ds18b20/dht22 — до задач 4–5).
2. [x] Интерфейс блоков v2: прокладки `input_i/f`, `input_text`, `param_i/f`, `param_text`, `output_i/f`,
   ячейки 4 байта; старый `f64`-интерфейс пока работает рядом (драйверы ещё не пересобраны). Проверка:
   тесты `user_block`/`block_check` на блоках v2, `crab-sim` `driver_module`, старые тесты не сломаны. Готово: `crates/crab-sim/tests/driver_v2_module.rs`, раскладка доп. слотов дисплея — `user_block.rs` и plan.md; метка мастера `abi4` → `abi5`.
3. [x] Хост на ПК (`crab-sim`) и шаблоны своего блока (`crab-build/src/wasm.rs`, `crab-editor/src/blockedit`),
   образцы `skills/crablogic-block/examples/*` на новый интерфейс. Проверка: `crablogic block check` на
   образцах, тесты blockedit. Готово: образцы bh1750/run_hours/window_check на v2 в целых, `block check` без ошибок; справка `10-block-editor.md`, `12-drivers.md`, `skills/` обновлены. Заготовки редактора блоков — только C++-шаблоны, менять нечего.
4. [x] Драйверы `ds18b20`, `dht22` в целых, пересборка `block.wasm`. Проверка: тесты драйверов, `driver_module`. Готово: `sensor_drivers.rs` проходит, ds18b20 2489→1808 Б, dht22 1203→1241 Б; `example_modules_have_no_f64` проверяет и примеры с датчиками. Исправлен `drivers/build.ps1` (лишний `librarylock.wasm` у крейта без вариантов).
5. [x] `crab-ui-format` v3 (меню в целых, виджет `String`), драйвер `display` в целых, строковые слоты,
   пересборка `oled`/`lcd`. Проверка: тесты формата (v1/v2/v3), 30 тестов драйвера, снимок
   `display_menu_picture` без изменений.
6. [x] Виджет `disp_string`: библиотека, генератор, проверка, конструктор экранов, симуляция, пример.
   Проверка: тест генерации, `crablogic check examples/display_menu`, предпросмотр в браузере. Готово 5–6: oled 22042→23614 Б, lcd 22694→23843 Б; `display_menu_picture` без изменений; `example_modules_have_no_f64` проверяет и `display_menu`; конструктор проверен в браузере. Справка — задача 9 (новый виджет, целые в меню, слоты драйвера).
7. [x] Мастер: `pulse.rs` в целых, `math` op 4; сборка ESP32 и C3. Проверка: хост-тесты мастера. Готово: статистика замеров в целых (десятые уровней, среднее время вызова), в `firmware/master-rs/src` нет `f64`, 90 хост-тестов проходят; сборку под платы не запускали (по заданию).
7а. [x] Отказ от старого интерфейса: `block.wasm` с `crab.input/param/output` (`f64`), модулем `flp` или любой
   инструкцией/типом `f64` — ошибка с кодом и подсказкой «пересоберите блок»; убрать поддержку v1 из
   генератора. C++-путь (`arduino/wasm.rs`, `crab-build/src/wasm_c.rs`, w2c2) не трогаем: он уходит вместе с C++-мастером (specs/rust-only). Усилить проверку типов: при постоянном номере порта в вызове `input_i/f` и т. п. сверять тип
   именно этого порта (сейчас сверяется только наличие порта такого типа). Проверка: тесты отказа, `crablogic block check` на старом блоке. Готово: код `wasm-f64` в `block check`, константа `OLD_F64`; v1 убран из `user_block.rs`, `driver.rs` (f64 в типах, локальных, глобальных и инструкциях отклоняется) и из `crab-sim`; при постоянном номере порта сверяется тип именно этого порта. Пересобраны на v2 `examples/wasm_driver` (bmp280) и `examples/wasm_block` (lowpass).
8. [x] Общий тест: собранные модули всех `examples/*` с драйверами — без `f64`. Проверка: тест. Готово: `sheets.rs` — `example_modules_have_no_f64` (без пропусков, с блоками проекта), `library_block_wasm_has_no_f64`, `a_block_with_f64_is_refused`.
9. [x] Справка: ABI 5 вместо 4 (`help/05-build.md` «Формат модулей», `help/08-errors.md` строка «модули нового формата», `abi_version() = 5`), `help/13-displays.md`, справка о своих блоках на WASM, `help/08-errors.md`, `help/09-news.md`,
   `docs.md` затронутых блоков, переводы.
10. [ ] Плата: заливка мастера, пример дисплея, бенч ESP32/C3 (медиана). Проверка — пользователь и бенч.
11. [x] Сверка с критериями спеки (13.11.2026):
   - 0, 0а — выполнено: интерфейс v2 с ячейками по 4 байта; драйверы считают в целых, `f32` только у портов `float`
     (обзор diff ds18b20, dht22, display).
   - 1 — выполнено: `example_modules_have_no_f64` (все `examples/*` с драйверами, без пропусков),
     `library_block_wasm_has_no_f64`.
   - 2 — выполнено: код `wasm-f64` в `block check`, тест `a_block_with_f64_is_refused`; при постоянном номере порта
     сверяется тип этого порта (`a_constant_port_index_is_checked_against_its_port`).
   - 3 — выполнено: `float_rounding_and_remainder_edges_match_the_simulation`.
   - 4 — выполнено: драйверы пересобраны, тесты проходят, `display_menu_picture` без изменений.
   - 5 — НЕ проверено: бенч на платах (задача 10).
   - 6 — выполнено: справка 05, 08, 09, 10, 12, 13, `docs.md` блоков дисплея, образцы `skills/`.
   - 7 — выполнено на ПК (симуляция, конструктор в браузере, пример); на плате не проверено.
   - Тесты: `cargo test --workspace` (60 наборов) и хост-тесты мастера (90) проходят, clippy без предупреждений.
   - Свои блоки пользователя: `lowpass`, `my_block` пересобраны на v2 (`blocks_src/`); `chip_temp` (C++) ждёт
     функции мастера «температура чипа».
