# CrabLogic: инструкции для ИИ-помощника

Это папка программы CrabLogic (визуальное программирование ESP32). Пользователь просит сделать что-то
для своей программы — работай по инструкции нужной папки `skills/`:

- свой блок (логика или драйвер датчика, код `block.wasm`) — [`skills/crablogic-block/SKILL.md`](skills/crablogic-block/SKILL.md);
- своя плата (контроллер: выводы, распаянные реле и входы) — [`skills/crablogic-board/SKILL.md`](skills/crablogic-board/SKILL.md).

Оглавление — [`skills/README.md`](skills/README.md). Общаемся с пользователем по-русски.

- `crablogic.exe` в этой папке — проверка и сборка: `crablogic --library library <команда>`.
- Свои блоки и платы — только в `user_library/`, исходники блоков — в `blocks_src/`.
- Не меняй `library/`, `firmware/`, `examples/` и программы `*.exe`.
- Проверка блока после правки: `skills/crablogic-block/hooks/check.ps1 <папка блока>` (в Claude Code она
  запускается сама — `.claude/settings.json`).
