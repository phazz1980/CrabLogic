# Задачи: полный переход на Rust

Спека и ответы — `spec.md`, план — `plan.md`. Этапы E–G начинаются только после закрытия A–D.

## A. Замер

- [x] A1 `crablogic audit-rust`: блоки библиотеки и примеры → собирается ли модуль, `needs` против `MODCAPS`
  Rust-мастера. Проверка: тест в `crab-cli` на двух примерах (рабочем и с веб-интерфейсом).
- [x] A2 `gaps.md` по выводу A1; пополнить этап B этого файла по таблице.

## B. Rust-мастер догоняет

- [ ] B1 HTTP-сервер и веб-интерфейс в Rust-мастере (`board/web.rs`). Проверка: пример с веб-интерфейсом
  открывается в браузере с C3 COM5 и ESP32 COM10.
- [x] B2 Датчики и выводы на расширителе PCF8574 в модулях (`wasm/hardware.rs`). Проверка: тест генерации
  + пример на плате.
- [x] B3 Недостающие виджеты и строки в блоках (`wasm/web.rs`, `wasm/block.rs`). Проверка: тесты `crab-codegen`.
- [x] B4 Прочее из `gaps.md` (пусто: замер 0 из 166, см. `gaps.md`).
  Сделано 12.11.2026 (до замера A1, по составу C++-мастера):
  - веб-интерфейс: логика `firmware/master-rs/src/web.rs` (11 тестов на ПК) и связка `board/web.rs` (HTTP-сервер
    ESP-IDF на 80, раскладка и форма сети в NVS `crab_web`, страница из `crab-blocks` через `build.rs`),
    `modcaps` `web`. ESP32 COM10, точка доступа: сервер стартует («страница устройства на порту 80»). Не
    проверено в браузере (B1 открыт); нет поиска сетей и применения формы сети к Wi-Fi (вопрос 35);
  - Modbus RTU/TCP: ядро `src/modbus.rs` (16 тестов, по библиотеке Flprog) и связка `board/modbus.rs`, `modcaps`
    `modbus`, модули требуют его (`MASTER_CAPS`). ESP32: модуль `modbus_rtu` принят, работает без ошибок; обмен с
    устройствами не проверен (вопрос 34);
  - уже были: АЦП, ШИМ, retain/retain_text, UART, PCF8574 (опрос), DS18B20/DHT22 драйверами (`drivers/`,
    `bus.pulse_read`, тесты `crab-sim/tests/sensor_drivers.rs`).
- [x] B5 Повтор A1: все блоки и примеры собираются модулями (или осознанно помечены — записать в `gaps.md`).

  A1/A2/B5 12.11.2026: `crablogic audit-rust` (`crates/crab-cli/src/audit.rs`, тесты на `pump`, `modbus_rtu` и на
  всю библиотеку) — 0 из 166 не собираются; найдена и исправлена ошибка `scale_table` (см. `gaps.md`). B2/B3
  закрыты замером (виджеты, строки, расширитель собираются модулями). Датчики-драйверы на выводах
  расширителя — ошибка генерации с понятным текстом (1-Wire и DHT через PCF8574 невозможны по таймингам).

## C. Заливка мастера без arduino-cli

- [x] C1 Проба: `espflash` как библиотека в `crab-build` рядом с `crab-debug` (конфликт `serialport`?).
- [x] C2 `build-dist.ps1`: образы `master-rs` для C3 и ESP32 в `dist/CrabLogic/firmware/<чип>/` + `info.toml`.
  Проверка: размеры образов и свободная куча записаны; если не влезает — варианты образа (риск в `plan.md`).
- [x] C3 `crab-build/src/flash.rs`: выбор образа по чипу, прошивка, прогресс. Проверка: тест выбора образа
  на ПК; прошивка C3 COM5 и ESP32 COM10.
- [ ] C4 Редактор «Залить мастер» и CLI `crablogic flash-master` через `flash.rs`; предложение перепрошить
  старый мастер. Проверка: вручную в редакторе на обеих платах.
  Итог 12.11.2026: C1 — `espflash` 4.7 без CLI-функций, `serialport` одной версии с `crab-debug`. C2 — `-BuildMaster`
  собирает оба мастера, без флага копируются последние ELF в `firmware/<чип>/` (ELF, `partitions.csv`, `info.toml`).
  C3 — `flash.rs` (образ из ELF + свой загрузчик espflash, DIO, 460800), тесты выбора образа; ESP32 COM10 прошит
  `crablogic flash-master`, модуль залит в перенесённый `crabmod` и пережил сброс. C3 COM5 не проверен (платы нет на
  порту). C4 — CLI готов; в редакторе «Залить мастер» прошивает образ, фоновая сборка C++-мастера убрана;
  проверка в редакторе вручную и предложение перепрошить C++-мастер — осталось.

## D. Пометки и старые проекты

- [x] D1 Ошибка проверки «не работает на Rust-мастере» (новый код в `error-codes`, `help/08-errors.md`)
  и значок в библиотеке. Проверка: тест `crab-core` + снимок списка ошибок.
- [x] D2 Ошибка «блок на C++ больше не поддерживается» со ссылкой на `skills/`. Проверка: тест с блоком
  с `esp32.cpp.tmpl`.
- [x] D3 Режим «Скетч» в старом проекте: одно сообщение при открытии, настройки скетча не сохраняются.
  Проверка: тест `crab-core` (открыть → сохранить → полей нет).

  D1–D3 12.11.2026: виды `rust-master` и `cpp-block` (`crab-core/src/check/types.rs`, разделы в
  `help/08-errors.md`), считает их `crab_codegen::rust_master_diagnostics` (тесты: блок с `esp32.cpp.tmpl`, пример
  `pump`); `crab_codegen::check_project` = проверка ядра + они — в редакторе, CLI `check` и ИИ/MCP. Значок ⚠ и
  подсказка у блоков на C++ в «Пользовательских». D3: `Project::drop_sketch_settings` (`[board.options]`) с тестом
  «открыть → сохранить → полей нет»; редактор при открытии пишет одно предупреждение и помечает проект
  изменённым. Не проверено глазами: значок и сообщение в окне редактора; переводы справки `help/<язык>/`.

## E. Удаление C++

- [ ] E1 Тест генерации модуля или симуляции есть у каждого блока (до удаления снимков `.ino`).
- [ ] E2 `crab-codegen`: удалить генератор Arduino и снимки `.ino`; вывод языка — только WASM.
- [ ] E3 `crab-build`: удалить `master.rs`, `native.rs`, `direct.rs`, `cores.rs`, установку arduino-cli
  (и `wasm_c.rs`, если не нужен).
- [ ] E4 `crab-core` (`native.rs`, `native_wizard.rs`, шаблоны C++ в `library.rs`), `crab-sim` (native-тесты).
- [ ] E5 `crab-editor`: меню, кнопки, вкладки редактора блока про C++/скетч/arduino-cli; i18n-ключи.
- [ ] E6 `crab-ai`, `crab-mcp`: команды сборки/заливки → модули/мастер; создание блока без C++.
- [ ] E7 `crab-cli`: убрать `gen`/`build`/`upload`; удалить `audit-rust`.
- [ ] E8 Удалить `libs/`, `specs/wasm-on-mcu/proto/master/`. Проверка всего этапа: `cargo fmt`,
  `cargo clippy --all-targets`, `cargo test`; все `examples/` проходят `gen-wasm`; `grep -ri "arduino"`
  по `crates/` и `help/` — только история в `09-news.md`.

## F. Папка skills

- [x] F0 (по просьбе 12.11.2026) `skills/crablogic-board/`: своя плата — SKILL.md, `reference/board-toml.md`,
  `reference/chips.md`, `examples/relay_board/` (плата `demo_relay4` + проект), `checklist.md`; `skills/README.md`,
  `AGENTS.md`; команда `crablogic board check`. Проверка: `board check` на образце и на KC868-A6 — «Ошибок нет»,
  на сломанных (GPIO40, `pcf8575_in`, ESP32-S3) — ошибка с причиной; проект образца проходит `check` и
  `gen-wasm`. Чистая сессия Sonnet (12.11.2026) по инструкции сделала плату C3 `mini_relay_c3`: `board check`,
  `check`, `gen-wasm` прошли; по её замечаниям дописаны запуск с `--library`, где ищется `user_library` и чья копия
  главнее, правило для занятых USB/UART0 (текст `busy` заменяет предупреждение чипа), инверсия/подтяжка через
  блоки, выводы I²C, АЦП2 у C3, `reference/blocks.md` с портами. Не сделано: `.cursor/rules`, хуки,
  копия `skills/` в дистрибутив (F5).
- [x] F1 `crablogic block check <папка>` (логика в `crab-core`/`crab-codegen`). Проверка: тесты на исправный
  блок и на 5 типовых ошибок (порт, тип, параметр, `block.wasm` без экспорта, лишний импорт).
  12.11.2026: `crab_codegen::check_block` (`block_check.rs`), коды `toml`, `def`, `builtin`, `cpp`, `no-wasm`,
  `wasm`, `string`, `schema`, `gen` и предупреждения `docs`, `sim`, `description`; `one_block_project` перенесён из
  `audit.rs`. Тесты: исправный блок, драйвер `dht22`, 5 типовых ошибок, `cpp`, `string`, занятое имя, образцы skills.
- [x] F2 `skills/crablogic-block/SKILL.md` с триггерами, `reference/`, `checklist.md`.
  12.11.2026: `reference/block-toml.md`, `wasm-api.md`, `errors.md`; `AGENTS.md`, `skills/README.md`, справка
  `05-build.md`, `12-drivers.md` (блок-логика без железа, где лежат блоки проекта, коды возврата шины), `09-news.md`.
- [x] F3 `examples/` в skills: логика, логика с памятью, драйвер датчика. Проверка: все три проходят F1.
  12.11.2026: `window_check` (504 Б), `run_hours` (670 Б), `bh1750` (1461 Б): блок в `project/library/<тип>/`,
  `build.ps1` берёт имя из папки; тест `skill_examples_pass`. На железе не проверено.
- [x] F4 Хуки `hooks/check.ps1|.sh` + `.claude/settings.json` (`PostToolUse`, `Stop`), `AGENTS.md`,
  `.cursor/rules/crablogic-block.mdc`. Проверка: правка `block.toml` с ошибкой → хук возвращает её модели.
  12.11.2026: `skills/crablogic-block/hooks/check.ps1` (ручной режим, PostToolUse по JSON из stdin — `block.toml`,
  `docs.md`, `block.wasm`, `logic.toml`/`project.toml`; `-Stop` — блоки `blocks_src/`, `user_library/`, изменённые за
  6 ч, `stop_hook_active` не держит), `check.sh` — обёртка; `skills/dist/` — `.claude/settings.json`, `AGENTS.md`,
  `CLAUDE.md` (`@AGENTS.md`), `.cursor/rules/crablogic-block.mdc`, `crablogic-board.mdc`. Проверено командами из
  `settings.json` через bash с JSON на входе: ошибка в `block.toml` (путь с кириллицей) → код 2 и текст; Stop → код 2,
  после исправления 0. Не проверено: живая сессия Claude Code и Cursor в папке дистрибутива.
- [x] F5 Дистрибутив: всё из F2–F4 и `crablogic.exe` в папке пользовательских блоков, пути относительные.
  Проверка: хук срабатывает в распакованном `dist/CrabLogic.zip`.
  12.11.2026: `scripts/copy-skills.ps1 -Dest` (зовёт `build-dist.ps1`): `skills/` без `target`/`dist`, файлы
  `skills/dist/` в корень, `blocks_src/`; справка `07-ai.md`, `dist-README.txt`. Проверено на сборке в песочнице
  (копия `library/` + `copy-skills.ps1`), полный `build-dist` и zip не запускались.
- [ ] F6 Проверка инструкции: субагент Sonnet/Haiku в чистой папке дистрибутива делает 3 блока из спеки
  без помощи; пробелы дописаны в `skills/`.
  12.11.2026, предварительно (не в дистрибутиве, а в копии `library/` + `skills/`): Haiku сам сделал «Среднее из
  трёх» (медиана), «Ограничитель скорости», драйвер SHT31 — `block check`, `check`, `gen-wasm` без ошибок, установил
  в `user_library`. По его 10 замечаниям: инструкция без ссылок на `help/` (в дистрибутиве её нет), запуск
  `build.ps1` с `-ExecutionPolicy Bypass`, рабочая папка `blocks_src/`, что копировать при установке, тесты
  поведения `cargo test` (раскладка `mod wasm` под `cfg(wasm32)`), короткие циклы можно, `default` у входов,
  буферы шины. Повторить в распакованном дистрибутиве после F5.

## G. Выпуск 2.0.0

- [ ] G1 Справка и переводы, README, `dist-README.txt`, `Архитектура и план.md`, CLAUDE.md.
- [ ] G2 Сверка критериев спеки; что не проверено на железе — в ответе.
- [ ] G3 Версия 2.0.0, `build-dist`, релиз (по подтверждению пользователя).
