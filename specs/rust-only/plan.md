# Полный переход на Rust — план

Спека: `spec.md` (согласована 12.11.2026, ответы 1–11).

## Уточнение к «Что есть сейчас»

`MODCAPS` Rust-мастера (`firmware/master-rs/src/board/mod.rs`) уже содержит все имена из `MASTER_CAPS`
(math, retain, tag_text, retain_text, uart_format, uart_console, pcf_expander, sparkplug, on_fault, abi4, bus),
а DS18B20 и DHT22 уже сделаны драйверами `block.wasm`. Значит, список «не умеет» в спеке устарел.
Настоящие пробелы надо получить замером (этап A), а не по памяти. Известно на сегодня:

- нет HTTP-сервера, то есть блоков веб-интерфейса (`crab-codegen/src/wasm/web.rs` есть, а в мастере нет);
- часть блоков отвергается генератором модулей (`WasmError::Unsupported`): виджеты, датчики и выводы
  на расширителе, строки в части блоков;
- редактор заливает только C++-мастер (`crab-build/src/master.rs`, arduino-cli); Rust-мастер шьётся
  руками (`cargo run` / `espflash`);
- Rust-мастер есть только для C3 и ESP32; другие чипы из `custom-boards` (S2, S3…) — вне перехода,
  на них заливка будет недоступна с понятной ошибкой.

## Этапы

**A. Замер.** Новая команда `crablogic audit-rust` (временная, удаляется в этапе E): по каждому блоку
библиотеки и каждому примеру — собирается ли модуль, какие `needs` не покрыты `MODCAPS` Rust-мастера.
Итог — таблица в `gaps.md`; по ней пополняется этап B.

**B. Rust-мастер догоняет.** По `gaps.md`: HTTP-сервер и веб-интерфейс (`board/web.rs`, `esp-idf-svc::http`),
блоки на расширителе, недостающие виджеты/строки в `crab-codegen/src/wasm/`. Новые датчики —
только драйверами `block.wasm` (решение `wasm-drivers`). Каждый пункт — тест в `crab-codegen`/`crab-kernel`
на ПК и проверка на C3 COM5 / ESP32 COM10.

**C. Заливка мастера из редактора без arduino-cli.**
- Образы: `scripts/build-dist.ps1` собирает `master-rs` для `riscv32imc-esp-espidf` и `xtensa-esp32-espidf`
  и кладёт в `dist/CrabLogic/firmware/{esp32c3,esp32}/` (bootloader, partitions, app, `info.toml` с версией
  и `MODCAPS`).
- Новый модуль `crab-build/src/flash.rs` на крейте `espflash` (как библиотека): выбор образа по чипу платы,
  прошивка по COM, прогресс в журнал. Редактор и CLI (`crablogic flash-master`) зовут его.
- Проверка версии: мастер старше образа в дистрибутиве → предложение перепрошить (как сейчас по `modcaps`).

**D. Пометка неподдержанного и старые проекты.**
- `crab-core`: признак блока «работает на Rust-мастере» вычисляется из генератора модулей; проверка схемы
  выдаёт ошибку `E…` «не работает на Rust-мастере» (новый код в `error-codes`), библиотека — значок.
- Блоки с `esp32.cpp.tmpl`/`native.cpp`: ошибка «блок на C++ больше не поддерживается» + ссылка на `skills/`.
- Проекты в режиме «Скетч»: одно сообщение при открытии; поля сборки скетча и библиотеки Arduino
  не пишутся при сохранении (`crab-core/src/packed.rs`, `project.toml`).

**E. Удаление C++** (только после закрытия B и проверки на обеих платах):
- `crab-codegen`: `arduino/`, `arduino.rs`, `program.rs`, `tasks.rs`, `retain.rs`, `expander.rs`, `preview.rs`
  и снимки `tests/snapshots/*.ino` — что не нужно генератору модулей; `lang.rs` (вывод под язык) остаётся
  с одним выводом WASM.
- `crab-build`: `master.rs`, `native.rs`, `direct.rs`, `cores.rs`, установка arduino-cli и ядер, `wasm_c.rs` —
  если используется только C++-мастером (проверить); остаются `wasm.rs`, `wamrc.rs`, `aot_xtensa.rs`,
  `mcu_module.rs`, `upload_plan.rs`, новый `flash.rs`.
- `crab-core`: `native.rs`, `native_wizard.rs`, поля шаблонов C++ в `library.rs`; `crab-sim`: native-тесты.
- `crab-editor`: меню «Код C++», «Собрать (F5)», «Папка библиотеки Arduino», установка arduino-cli;
  в редакторе блока вкладки «Код ESP32», «Код C++ для платы», «Библиотека Arduino (мастер)», «Создать по
  шаблону» (C++), `blockedit/native.rs`; условие `when` про скетч. «Собрать (F5)» → «Залить модули».
- `crab-ai`, `crab-mcp`: команды build/upload скетча → заливка модулей/мастера; создание блока без C++.
- `crab-cli`: убрать `gen`/`build`/`upload`, `convert` проверить; остаются `check`, `gen-wasm`, `aot`,
  `flash-master`, новое `block check`.
- Репозиторий: `libs/`, `specs/wasm-on-mcu/proto/master/` (C++-мастер), i18n-ключи удалённых пунктов.

**F. Папка skills** (после E, когда формат блока не меняется):
- `crab-cli`: `crablogic block check <папка>` — разбор `block.toml`, сборка тестовой схемы из одного
  блока, генерация модуля, проверка `block.wasm` драйвера (импорты, экспорт, размер); вывод — коды ошибок
  с подсказкой. Код в `crab-core`/`crab-codegen`, CLI только печатает.
- `skills/crablogic-block/`: `SKILL.md`, `reference/`, `examples/` (3 блока, они же лежат в `library/`-тестах),
  `checklist.md`, `hooks/check.ps1|.sh`; `.claude/settings.json` (хуки `PostToolUse` на Write/Edit
  с путём `**/block.toml|docs.md|*.rs|*.c` в папке блока, `Stop` — чек-лист); `AGENTS.md`;
  `.cursor/rules/crablogic-block.mdc`.
- Дистрибутив: `build-dist.ps1` кладёт всё это в `dist/CrabLogic/blocks/` (папка пользовательских блоков)
  с путями хуков относительно неё; `crablogic.exe` — в `PATH` хука через относительный путь.
- Проверка инструкции: субагент Sonnet/Haiku в чистой папке дистрибутива, три задания из спеки.

**G. Выпуск 2.0.0.** Справка (`05-build.md`, `10-block-editor.md`, `12-drivers.md`, `01-quickstart.md`,
`07-ai.md`, `09-news.md` + переводы), README, `Архитектура и план.md`, CLAUDE.md (окружение без arduino-cli,
план работ), `dist-README.txt`; релиз по `release-process`.

## Риски

- **Размер образа**: один Rust-мастер со всем сразу (Wi-Fi, MQTT, Ethernet, HTTP) может не влезть в 4 МБ
  на C3 / не хватить кучи (уже тесно с Wi-Fi). Если не влезет — 2–3 варианта образа по набору функций,
  выбор по `needs` проекта (как `MasterFeatures` в C++-мастере).
- **Пользователи с C++-мастером на плате** после 2.0.0 должны перепрошить мастер — сообщение при подключении.
- **espflash как библиотека** тянет много зависимостей и может конфликтовать по версиям `serialport`
  с `crab-debug` — проверить в этапе C первым делом.
- **Удаление снимков `.ino`** уберёт часть покрытия логики блоков; перед E убедиться, что у каждого блока
  есть тест генерации модуля или симуляции (`crab-sim`).
- Объём большой: этапы A–D идут при живом C++-пути, E — одним шагом с полным прогоном тестов.
