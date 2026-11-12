# Блоки для проверки платы: имена портов и параметров

Нужны, чтобы собрать проект-образец (шаг 6). Параметры пишутся прямо в `[[block]]` (`pin = 4`), связь —
`from = "блок.порт"`, `to = "блок.порт"`.

## Выводы платы

| Тип (`type`) | Входы | Выходы | Параметры |
| --- | --- | --- | --- |
| `digital_in` — цифровой вход | — | `out` | `pin`, `pullup` (`false` при внешней подтяжке), `inverted` |
| `digital_out` — цифровой выход | `in`, `en` | — | `pin`, `inverted` (реле/светодиод «включаются нулём»), `write_enable`, `on_fault` |
| `analog_in` — аналоговый вход | — | `out` | `pin` |
| `pwm_out` — ШИМ-выход | `in`, `en` | — | `pin`, `write_enable`, `on_fault` |

`pin` — номер GPIO числом. Вход `en` можно не подключать.

## Периферия платы (`[[device]]`)

Тип в проекте — `<id платы>_<id устройства>`. Адрес и шина уже закреплены платой, в проекте их не задают.

| `kind` | Входы | Выходы |
| --- | --- | --- |
| `pcf8574_out` — реле, выходы | `p0` … `p7` (столько, сколько `channels`) | `ok` (связь с микросхемой есть) |
| `pcf8574_in` — входы | — | `p0` … `p7`, `ok` |

## Образец `logic.toml`

Для платы `demo_relay4` (вход на GPIO32 с внешней подтяжкой, реле на PCF8574). Если на плате есть и
`[[device]]` с `kind = "pcf8574_in"` (например, `id = "inputs"`), его блок `demo_relay4_inputs` ставится так же,
а его выходы `inputs.p0`… подключаются к входам реле.

```toml
[[block]]
id = "button"
type = "digital_in"
pin = 32
pullup = false

[[block]]
id = "relays"
type = "demo_relay4_relays"

[[link]]
from = "button.out"
to = "relays.p0"
```

Полный рабочий пример — [`../examples/relay_board/project/logic.toml`](../examples/relay_board/project/logic.toml).
