# Задачи: события bool и диагностика мастера

- [x] Мастер (`specs/wasm-on-mcu/proto/master/master.ino`): время шага, кольцо смен 0/1, команды `ev`, `rmax`, поля `us=` и `caps=` в `list` — проверка: компиляция мастера (`master_with_web_compiles`).
- [x] `crab-debug::master`: разбор `list` в `Stats`, `free=`, `caps=` — тесты `list_gives_cycle_statistics`, `older_master_without_times_still_reports_cycles`.
- [x] События на график — тесты `events_draw_a_square_wave_at_board_time`, `sample_does_not_repeat_what_events_told`.
- [x] Кнопка «Сбросить максимумы» шлёт `rmax`.
- [x] Справка `help/06-debug.md`, новости.
- [ ] Проверка на плате: меандр известной частоты, перепрошивка мастера.
- [x] Провода на схеме: значение по истории с задержкой 0,35 с (`DebugState::shown`), перерисовка 30 раз в секунду.
- [ ] Кнопка «Синхронизировать графики»: общее окно времени (масштаб, сдвиг, «следовать») для всех открытых графиков; общее состояние вместо `ChartView` на каждый график — проверка: тест на общий вид и ручная проверка двух графиков.
