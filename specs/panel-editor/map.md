# Щит (электросхемы): карта кода для другого ИИ

Единицы схемы: 1 = 0,25 мм. Комментарии в коде — английские, интерфейс — русский через `i18n::tr` (`crates/crab-editor/i18n/en.json`).

## Состояние (22.11.2026)
- `specs/panel-editor/tasks.md`: задачи 1–5 готовы; открыты 6 (перепайка мышью), 7 (ИИ и MCP — в работе), 8 (справка), 9 (сверка).
- **Внимание:** `specs/panel-free-sheets/spec.md` (ждёт ответов) убирает особый лист управления: `is_control`, `Placed.power`, отдельный заголовок листа. Не вкладываться в развитие этих мест, пока спека не решена.

## Данные (`crates/crab-core`)
- `src/panel.rs` — `Panel { supply, devices, links, wire_numbers }`.
  - `Device`: `id`, `kind`, `rating`, `part`, `bind` (тег или `блок.порт`), `load`, `from`, `poles`, `of`, `form`, `pins`, `free`, `series`.
  - **Топология питания:** устройство называет в `load` то, что оно питает или коммутирует (`K1.load = M1`). Если источник питает несколько устройств, первое он называет через `load`, а остальные ссылаются на него своим `from` (ветвь линии, точку рисует лист). `free` — только что добавлено и ещё не подключено, на листах не рисуется.
  - `of` — аппарат доп. контакта (или двигатель датчика); `series` — катушка, в цепи которой стоит контакт.
  - `DeviceKind` — 14 видов: Feed, Breaker, Rcd, Fuse, Switch, Psu, Contactor, Relay, Motor, Load, Terminal, Sensor, Button, Aux. Список выбора в редакторе `DeviceKind::ALL` — 11 из них: без Aux (доп. контакт добавляется от аппарата), Fuse и Switch (их нет и в командах ИИ).
  - `WireLink { from, to }` — ручной провод `"dev:term"`; заменяет автоматический провод на тех же выводах.
  - `wire_numbers` — закреплённые номера проводов, пишет генератор (`Nets::store`), руками не правятся.
  - `rename` / `rename_all` (с обменом имён) / `remove` обновляют ссылки `load`, `of`, `series`, `from`, `links`, `wire_numbers`.
- `src/panel_edit.rs` — проверяемые правки: `Panel::branch_power`, `Panel::reconnect_power`, `rebind_device(project, lib, …)`, `rename_device` / `rename_devices` (щит + раскладка вместе — **использовать их, а не `Panel::rename` напрямую**).
- `src/panel_layout.rs` — `PanelLayout` (только для редактора, на перечни и проверку не влияет):
  - `symbols` — сдвиги символов и надписей: ключи `symbol_key` = `"<вид>:<id>"`, `label_key` = `"label:<id>:<что>"` (поворот и размер — `label_style`), `bus_key` = `"bus:<лист>:<линия>"`;
  - `wires` — изгиб средней части провода (`bend`);
  - `bus_to` — перенос шин питания на следующие листы;
  - `placed` (`Placed{power,sheet,x,y}`) — устройства, положенные на лист вручную: `place`, `place_power`, `unplace`;
  - `rename` / `rename_all`, `prune` (вычистка записей удалённых устройств), `clear_symbol`, `clear_all`.
- `src/check/panel.rs` — проверки щита. Файлы на диске: `panel.toml` (данные), `panel_layout.toml` (раскладка).

### Правила согласованности ключей
- Ключ провода везде один: `"A:pin>B:pin"` (`nets::Wire::key`, `WireLink::key`, `wire_numbers`, `PanelLayout.wires`).
- Всё, что меняет id устройства, обязано переименовать и ключи раскладки (`rename_device`).
- Генерация — чистая функция: одинаковые проект и раскладка дают одинаковый документ.

## Построение листов (`crates/crab-schematic`) — чистая функция проект → `Document`
- `src/lib.rs` — `build_document` / `build_document_in`, PDF; `src/doc.rs` — модель `Document` (листы, операции рисования, вывод в Typst); `title.rs` — основная надпись; `lang.rs` — язык подписей; `error.rs`.
- `src/nets.rs` — цепи и провода: автоматические + ручные (`panel.links` заменяют автоматические на тех же выводах), `terminals_of`, `aux_terminals`, `Nets.manual`, нумерация (`wire_numbers`, `store`).
- `src/power.rs` — силовые листы: цепочки от ввода, стек ввода, ответвления с точкой, шины питания, ручные устройства и провода (`placed.power`).
- `src/control.rs` — лист «Цепи управления»: плата, шины 24V/0V, колонки катушек и контактов; `draw_manual` (коробка устройства с выводами), `link_route`, `terminal_of_port`, `Pins`. Лист строится, только если есть привязки, размещённые устройства или провода.
- `src/symbols.rs` (символы), `bus.rs` (шины), `route.rs` (ортогональные маршруты), `scene.rs` (`Sheet/Item/ItemKind/Port/End` — слой поверх операций для интерактива), `tables.rs` (перечни), `xref.rs` (перекрёстные ссылки `/лист.зона`), `sheet_names.rs` (обозначения по листам), `canvas.rs` (рисование и цвета).
- Тесты: `crates/crab-schematic/tests/{power_sheet,control_sheet,nets,scene,layout,ops}.rs`.

## Редактор (`crates/crab-editor`)
- `src/panelview.rs` (~2800 строк, читать диапазонами):
  - состояние и кэш — `Key`, `Cache`, `Drag::{Symbol,Bend,Link}`, `DevicePayload`, `LayoutEdit` (начало файла);
  - дерево — `tree`, `coil_row`, `aux_row`, «Питается от»;
  - действия — `Action`, `apply`, `find`;
  - свойства — `properties`, `label_properties`, `Edit`, `apply_edit`;
  - проверка попадания на листе — `symbol_at`, `port_at`, `link_at`, `bus_at`, `label_at`, `bend_at`, `ref_at`, `movable`, `is_power`, `is_control`;
  - рисование и жесты — `sheet`, `draw_sheet` (самая длинная функция), `draw_ops`, `apply_layout_edit`;
  - панель инструментов и PDF — `toolbar`, `start_pdf`;
  - тесты в конце файла.
- `src/doc.rs` — история и отмена:
  - `edit_panel` — только `Panel`;
  - `edit_panel_project` — весь проект за один шаг отмены (переименование);
  - `edit_panel_layout` — только раскладка.
  Все три поднимают `panel_revision`. Снимок отмены включает `panel_layout`.

### Путь одной правки
жест мыши → `Drag` в `draw_sheet` → `edit_panel*` (`doc.rs`, один шаг отмены) → `panel_revision` → `refresh` пересобирает `Document` (`build_document_in`) → сцена → проверка попадания (`port_at` / `link_at`) → кадр.

## Прочее
- `crates/crab-ai/src/panel.rs` — команды ИИ и MCP для щита: `get_panel`, `add/set/remove_panel_device`, `set_panel_supply`, `build_panel`; в работе `move_panel_symbol`, `reconnect_panel_device` (задача 7). Ручных проводов и размещения на листе в командах нет.
- `help/18-panel.md`, `help/09-news.md` — справка и «что нового», правятся вместе с кодом.
- `specs/panel-*/`, `specs/symbol-editor/`, `specs/panel-free-sheets/` — спецификации.
- `examples/pump/panel.toml` — пример.

## Работа
- Сборка на диске D. Временная папка — на D, чтобы не забивать системный диск:
  - Bash: `export TMP=D:/CrabLogic/target/tmp TEMP=D:/CrabLogic/target/tmp`;
  - PowerShell: `$env:TMP='D:\CrabLogic\target\tmp'; $env:TEMP=$env:TMP`.
- Проверять по крейту, `-j 2` ограничивает память при сборке: `cargo test -j 2 -p crab-schematic -p crab-core -p crab-editor`, затем `cargo fmt` и `cargo clippy --all-targets`.
- Правила проекта — `CLAUDE.md`. Жесты на листе глазами пока не проверялись.
