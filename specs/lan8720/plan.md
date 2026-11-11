# Ethernet на LAN8720 — план

## Формат блока (`library/ethernet/block.toml`)

Один блок `ethernet`, имя в палитре «Ethernet». Новый параметр `chip` (список, только при генерации):
`w5500` — «W5500 (SPI)» (по умолчанию, старые проекты не меняются), `lan8720` — «LAN8720 (RMII, только ESP32)».

- SPI-параметры `cs`, `sck`, `miso`, `mosi` и `mac` — с `when = 'chip == "w5500"'` (у EMAC свой заводской MAC).
- `preset` (список, при `chip == "lan8720"`): `module` — «ESP32 + модуль LAN8720» (по умолчанию: адрес PHY 1,
  MDC 23, MDIO 18, такт — вход GPIO0, питания нет), `wt32_eth01` (1, 23, 18, такт — вход GPIO0, питание 16),
  `olimex_poe` (0, 23, 18, такт — выход GPIO17, питание 12), `custom` — «Свои выводы».
- При `preset == "custom"`: `phy_addr` (0–31), `mdc`, `mdio`, `power_pin` (`min = -1`, −1 — нет),
  `clock` (список: 0 вход GPIO0, 1 выход GPIO0, 2 выход GPIO16, 3 выход GPIO17 — как `FLPROG_RMII_CLOCK_*`).
- Скрытые `fixed`-выводы с условиями `when` — чтобы общая проверка выводов (`pins::pin_uses`) видела RMII без
  нового кода: 19, 21, 22, 25, 26, 27 (всегда при LAN8720); MDC/MDIO пресета; питание пресета (16 или 12);
  такт (0, 16 или 17 — по пресету или `clock`).
- `dhcp`, `ip`, `gateway`, `subnet`, `net_period`, выходы — без изменений, общие для обоих чипов.

Действующие значения (пресет или свои) считает `crab_core::lan8720::Lan8720` (в ядре, а не в `crab-blocks`:
ей пользуются проверка схемы, генератор C++ и генератор WASM; тест сверяет её с `fixed`-выводами `block.toml`).

## Проверка (`check`)

В `crab-blocks` (там, где проверки по типу блока) или `crab-core/check` — по месту, где уже живут проверки
«блок только для такого чипа»:
- LAN8720 на плате не ESP32 (C3/S2/S3) — ошибка «у этого чипа нет встроенного Ethernet (EMAC)…»;
- такт «вход GPIO0» без вывода питания PHY — предупреждение (только `custom` с `power_pin = -1`; у пресетов — нет);
- пересечения выводов — общей проверкой.

## Прошивка C++ (`crab-blocks/src/net.rs`, `ethernet`)

`FLProgWiznetInterface crabEth;` без CS, в `setup`: `wiznetMode(false)`, `rmiiMode(true)`,
`setRmiiPhyAddress/MdcPin/MdioPin/PowerPin/ClockMode`; адрес — `rmiiDhcpMode`, `rmiiLocalIP`, `rmiiGateway`,
`rmiiSubnet`, `rmiiDns` (у RMII свои настройки адреса). `ready` — `ethernetIsReady()`, адрес — `localIP()`
(библиотека ведёт маршрут на RMII). Службы (`net = "ethernet"`) не меняются. Снимок кода; сборка `arduino-cli`.

## WASM и Rust-мастер

- `crab-codegen/src/wasm/net.rs`: в настройке Ethernet (op 6) новые коды: чип, адрес PHY, MDC, MDIO, питание,
  такт. Для W5500 коды не шлются — старые модули и мастера не затронуты.
- Мастер (`firmware/master-rs/src/board/eth.rs`, `eth_cfg.rs`): при чипе LAN8720 — `EthDriver::new_rmii`
  (esp-idf-svc, `RmiiEthChipset::LAN87XX`), только для `xtensa-esp32-espidf` (`cfg`); на C3 — строка в журнале
  «LAN8720 needs ESP32 (EMAC)», `ready = false`. Питание PHY поднимается до старта драйвера. Проверка выводов —
  в `eth_cfg.rs` (тесты на ПК). Дальше — тот же `EspNetif`, что у W5500.
- `crab-build/src/master.rs`: признак `ethernet` уже включает сеть; отдельной возможности в `modcaps` не нужно,
  мастер на C3 честно отвечает журналом.

## Пример и справка

`examples/eth_lan8720/` (Ethernet LAN8720 + MQTT), тесты генерации C++ и WASM. Справка:
`library/ethernet/docs.md`, `help/` (сеть, ошибки, новости).

## Риски

- `CONFIG_ETH_USE_ESP32_EMAC` в ядре arduino-esp32 — проверить сборкой.
- Пины MDC/MDIO в esp-idf-svc — типизированные; для своих выводов нужен `AnyIOPin` по номеру (`unsafe`).
- Такт 50 МГц на GPIO0 и загрузка — только предупреждение, на железе не проверить без платы.
