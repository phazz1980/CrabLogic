# План: блоки на коде (C++ «как Arduino» и Rust)

Спека — `spec.md` (решения 12.11.2026). Основа уже есть: `crab-build::wasm` (zig → WASM, метка
`block.wasm.generated`), `crab-codegen::wasm::{driver,user_block}` (любой `block.wasm` вклеивается в модуль листа,
импорты `crab.*` и `bus.*`), `crab-sim` (wasmi), упаковка папки блока в проект (`blockfile::read_files`).

## Как устроен блок на C++

Папка блока: `block.toml` + `block.cpp` (+ `docs.md`), после сборки `block.wasm` и `block.wasm.generated`.

```cpp
#include <crab.h>
CRAB_INPUT(float, temp);
CRAB_PARAM(float, limit, 50.0);
CRAB_PARAM(pin,   led, 2);        // type "pin": the master lets the block use it
CRAB_OUTPUT(bool, alarm);
void setup() { pinMode(led, OUTPUT); }
void loop()  { alarm = temp > limit; digitalWrite(led, alarm); }
```

1. **Разбор портов** — `crab-core::codeblock` (новый модуль, без zig): строки `CRAB_INPUT/PARAM/OUTPUT` на верхнем
   уровне, по одной на строку, аргументы — литералы. Типы: `bool`, `int`, `float`, `pin`; параметр с `bus = …`
   (`CRAB_I2C_ADDRESS(addr, 0x76)`, `CRAB_PIN(sda, 21, "i2c-sda")`). Результат — порты `BlockDef` в порядке
   объявления; ошибки со строками. Сохранение блока пишет порты в `block.toml`, остальное (название, группа, описание,
   `sim`) — из формы. Строковые порты — не в первой версии.
2. **Обёртка** — `crab-codegen::codeblock::wrap(def, user_src)`: макросы `CRAB_*` раскрываются в `static T name;`,
   добавляются экспорты `init()` (обнулить, `setup()`) и `step(dt_ms)` (читать входы и параметры через
   `crab.input/param`, `millis` += dt, `loop()`, записать выходы). Сам `crab.h` вшит строкой в `crab-codegen`
   (`include_str!`), пишется рядом с исходником во временную папку сборки.
3. **`crab.h`** — подмножество Arduino поверх импортов: `millis/micros` (из `dt`; `micros` на плате — `bus.micros`),
   `pinMode/digitalRead/digitalWrite` (`bus.gpio_*`), `Wire` (`i2c_*`), `Serial1/Serial2` (`uart_*`), `SPI`
   (`spi_*`), `OneWire` (`ow_*`), `Serial.print/println` → `crab.log`, `String` из `sim_string.h`, `min/max/abs/
   constrain/map`, `bitRead/bitWrite`. Без виртуальных функций и глобальных конструкторов (запреты `driver.rs`:
   таблицы, косвенные вызовы, bulk memory). `delay`, `delayMicroseconds`, `analogRead`, `analogWrite`, `attachInterrupt`
   объявлены с `[[deprecated]]`-подобной ошибкой (`static_assert` через шаблон) с русским текстом-подсказкой.
   Импорты `bus.*` попадают в модуль только при использовании — блок чистой логики остаётся симулируемым.
4. **Сборка** — `crab-build::wasm::compile_block(zig, def, src)`: флаги драйверов (`-mcpu=mvp`, стек 4096,
   `--export=step,init,__heap_base`), затем проверка модуля тем же разбором, что у `driver.rs` (память ≤ 32 КБ,
   нет запрещённого). Ошибки clang разбираются в `(строка, текст)`; строки обёртки пересчитываются в строки
   `block.cpp` (`#line`); частые ошибки переводятся (таблица из ~10 шаблонов в `crab-build`).
5. **zig по кнопке** — `crab-build::zig_fetch`: версия, URL `ziglang.org` и SHA-256 архива зашиты в код; `ureq` +
   `zip` распаковывают в `<папка программы>/tools/zig/`, затем `zig_version`. Настройка `zig_path` остаётся
   (ручной путь главнее). Флажок `template_sim` сохраняется только для старых шаблонов.

## Rust

- Крейт `crab` в `sdk/rust/crab/` (вне workspace: `no_std`, цель `wasm32`), без proc-macro — `macro_rules!`
  `crab::block! { inputs { temp: f32 } params { limit: f32 = 50.0 } outputs { alarm: bool } state { … } }`:
  не нужен компилятор для ПК (линкер MSVC). Макрос выдаёт экспорты, `panic_handler` и секцию `crab.ports`
  (текст TOML портов, `#[link_section]`). Обёртки шин — `crab::i2c`, `crab::gpio`, `crab::uart`, `crab::spi`,
  `crab::onewire` с `Result`.
- Редактор: если в папке блока есть `rust/Cargo.toml`, кнопка «Собрать» зовёт `cargo build --release --target
  wasm32-unknown-unknown` (флаги как в `examples/wasm_driver/build.ps1`), копирует `block.wasm`; при каждой
  подгрузке `block.wasm` порты из `crab.ports` (`wasmparser`) сверяются с `block.toml` и обновляют его.
- Упаковка: `rust/target/` исключить из `read_files` (сейчас папка блока упаковывается целиком).

## Редактор блока (`crab-editor/src/blockedit/`)

- Новый вид блока «На коде»: вкладки «Код» (C++ с подсветкой `cpphl`, ошибки под полем и метки на строках),
  «Описание», «Пробный прогон». Вкладки портов и параметров — только чтение (порты из кода), кроме подписей, `sim`
  и описаний. «Код ESP32» и «Код C++ для платы» у таких блоков скрыты.
- Сборка при сохранении в фоне (по образцу `simbuild.rs`); без zig — карточка «Скачать компилятор (… МБ)».
- «Перевести в код» у блока с `esp32.cpp.tmpl`: разделы `@global/@setup/@loop/@functions` → `block.cpp`, подстановки
  портов → имена, объявления `CRAB_*` из `block.toml`; шаблон переименовывается в `esp32.cpp.tmpl.old`. Шаблоны
  с `@global once` и библиотеками — с предупреждением, что править придётся руками.
- ИИ: команда `create_block` получает вариант `code` (C++); `get_block_type` показывает `block.cpp`.

## Мастер

Не меняется. `analogRead`/`analogWrite` требуют новых функций шины (АЦП, LEDC) — отдельная задача в конце
(`needs_master`), до неё — ошибка сборки с подсказкой «используйте блоки analog_in / pwm_out».

## Риски

- Глобальные конструкторы C++ и `__wasm_call_ctors`: обёртка вызывает их из `init` или запрещает (проверка модуля).
- Clang может выдать `memory.fill/copy` и таблицы (switch → `br_table` допустим, указатели на функции — нет);
  `-mcpu=mvp` и проверка модуля ловят это с понятным текстом.
- Размер и версия zig: пин версии и хэша, обновление — вручную вместе с template-sim.
- Ошибки clang на английском и длинные: перевод только частых, остальное — как есть, первые 5.
- Rust без proc-macro ограничивает синтаксис `block!`; расширять по мере надобности.
