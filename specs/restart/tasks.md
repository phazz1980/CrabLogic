# Задачи: блок «Перезапуск платы»

Ответы на вопросы получены 19.11.2026.

- [x] 1. ABI: импорт `restart` (id 33, `abi.rs`, `restart_op`), без смены версии ABI. Проверка: тест списка импортов.
- [x] 2. Хост ПК и симуляция модулей: `StubIo` (`restart_request`, `restart_reason`). Проверка: тест на ПК.
- [x] 3. Мастер: `board/reboot.rs`, выгрузка модулей, ожидание очереди `retain` (≤ 500 мс), запись в журнал, `esp_restart`; `restart` в `modcaps=`. Проверка: сборка прошивки C3 (см. ниже).
- [x] 4. `library/restart`: `block.toml`, `docs.md`, генерация (`wasm/restart.rs`). Проверка: `every_library_block_builds_a_module`, тест `restart_block_asks_once_after_the_guard_and_can_be_cancelled`.
- [x] 5. Проверка схемы: один блок на проект (`single = true`), `go` обязателен, `cancel` необязателен.
- [x] 6. Пример `examples/restart_on_offline` (`crablogic check` без ошибок).
- [ ] 7. Плата C3: перезапуск по фронту, удержание, guard, `reset=software`, выходы безопасны. Не проверено на железе.
- [x] 8. Справка (`docs.md` блока, новости). Не сделано: замер `bench` после правки мастера, переводы справки, поведение блока в окне симуляции (`crab-sim` schema: показать «перезапуск»).
