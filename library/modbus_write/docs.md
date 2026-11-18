# Запись Modbus

Записывает значение в таблицу ведомого устройства, откуда его читает мастер.
`slave` — tcp («Modbus TCP Slave») или rtu («Modbus RTU Slave»);
`table` — holding, input, coil или discrete; `kind` — int, float или bool,
от него зависит вход (`in`, `in_f`, `in_b`).

`float` занимает два регистра подряд, начиная с адреса `reg`.
`swap` («Слова наоборот», только для float) — поменять местами два регистра:
порядок CDAB вместо ABCD. Так делают некоторые системы SCADA и панели.

Примеры: `examples/modbus_wifi`, `examples/modbus_rtu`.

В модуле WASM работает со slave `rtu` (протокол ведёт мастер-прошивка); `tcp` пока только в обычном скетче.
