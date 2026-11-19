# План: Modbus деревом

Затронуто: `library/modbus_*` (блоки), `crab-core` (модель, проверка, карта регистров), `crab-codegen`
(`wasm/modbus.rs`), мастер (`firmware/master-rs/src/modbus.rs`, если число slave у master растёт), `crab-sim`,
редактор (`modbuspanel.rs`, `radiopanel.rs`, диалог блока), справка.

## Договорённости об именах (этап 1)
- `modbus_device` (скрытый, на лист не ставится): параметры `master` (текст: id блока master) и `unit` (1..247). Не больше `slaves` на master (параметр master «Макс. slave», int 1..32, по умолчанию 4).
- `modbus_tag`: параметры `device` (текст: id `modbus_device` для master; id блока `modbus_rtu_slave`/`modbus_tcp_slave`
  для роли slave), `table`, `kind`, `reg`, `swap`. Порты: вход `in`/`in_f`/`in_b` и выход `out`/`out_f`/`out_b` по `kind`, выход `ok`;
  оба значения-порта необязательные (вход без источника — не ошибка).
- Направление: подключён выход значения — чтение (для slave-роли «в схему»); подключён вход — запись (для slave-роли «из схемы»);
  оба или ни один — диагностика проверки.
- `MODBUS_KINDS` в `crab-core/src/project.rs` = блоки master/slave + `modbus_device` + `modbus_tag`.
- Параметры `count`, `slave1..4` (число slave = число `modbus_device`; лимит `slaves`), `holding/input/coils/discrete` у master убираются: число slave и размеры таблиц slave считаются
  по устройствам и их тегам (как у slave-роли по тегам).

## Данные
- Новый скрытый блок `modbus_device` («slave у master»): `unit`, `name`, для TCP `ip`, `port`; связь с
  master — параметр `master` (id блока). Тег ссылается на устройство параметром `device`.
- Новый блок тега `modbus_tag`: `table`, `kind`, `reg`, `swap`, `device` (для slave-роли — id slave-блока);
  порты `in` и `out` (по `kind`), `ok`. Направление выводится при сборке по связям.
- Блоки `modbus_master_read`, `modbus_master_write`, `modbus_read`, `modbus_write` и параметры `slave1..4`,
  `count` у master убираются.

## Проверка и генерация
- `check_modbus`: теги без устройства, оба порта подключены / ни один, пересечения внутри таблицы устройства.
- Генератор по связям выбирает чтение или запись; slave-роль считает размеры таблиц по тегам.
- Мастер на плате: ограничение числа slave у master (сейчас 4) — проверить; если растёт, учесть ОЗУ C3.

## Риски
- Порт, у которого направление зависит от связей, новый для проекта: проверить автоподсветку портов и отладку (значения).
- Замена блоков ломает формат проекта: примеры (`examples/modbus_*`) переписать, тесты `library_modules.rs` обновить.
- Опрос нескольких slave по TCP — нагрузка на мастер, нужен замер (`crablogic bench`).

## Этап 2: IP у каждого slave
- `modbus_device`: параметры `ip` (string, по умолчанию пусто = адрес master) и `port` (int, 0 = порт master).
- Настройки в мастер: для slave n коды `272 + 2n` (IP, первый октет в младшем байте, 0 = как у master) и `273 + 2n` (порт, 0 = как у master); таблица настроек 16 + 8·32 + 64 кодов.
- Прошивка: `Transport::select(remote)`; TCP master держит по одному клиенту на разный адрес (slave с одним адресом делят соединение).
- Проверка: ip — адрес вида a.b.c.d; ip/port у slave RTU master не нужны; одинаковые Unit ID допустимы у разных адресов.
