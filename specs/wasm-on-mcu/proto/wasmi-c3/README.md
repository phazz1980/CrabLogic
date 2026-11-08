# wasmi на ESP32-C3 — замер ОЗУ и скорости

Прототип для сравнения wasmi (`no_std`) с wasm3 в мастере. Результаты — в `../../plan.md`,
раздел «wasmi `no_std` против wasm3 на ESP32-C3».

Нужно: `rustup toolchain install nightly --component rust-src`, `cargo install espflash`.

```
cargo run --release     # собрать и прошить COM5 (порт — в .cargo/config.toml)
```

Вывод идёт в USB-консоль (115200), в конце строка `######## done`.

- `riscv32imc-unknown-none-elf.json` + `src/atomics.rs` — атомики для RV32IMC без расширения A.
- `build.rs` собирает `../blink.wat`, `../counter.wat`, `../hang.wat`; `modules/*.wasm` — выход `crab-cli gen-wasm`
  для `examples/thermostat` и `examples/selftest_logic*`.
- У wasmi нужна фича `portable-dispatch` (см. plan.md).
