//! Runs a block written in any language that compiles to WASM, to simulate it on the PC.
//!
//! The module speaks plain core WASM (no WASI, no language runtime):
//! - exports `step(dt_ms: i32)`; optional `init()` resets the state;
//! - imports from module `crab`: `input_i(i) -> i32`, `input_f(i) -> f32`, `param_i`, `param_f`
//!   the same, `output_i(i, v: i32)`, `output_f(i, v: f32)` and `log(ptr: i32, len: i32)` (UTF-8
//!   text in the module memory); the old `f64` interface (`input/param/output`, module `flp`)
//!   is not supported: such a block does not link;
//! - string ports and parameters (the same indexes as the numbers) go through the module
//!   memory: `input_text(i, ptr, cap) -> len` and `param_text(i, ptr, cap) -> len` copy at
//!   most `cap` bytes and return the number of bytes copied; `output_text(i, ptr, len)` sets an output.
//!
//! The host keeps its values in `f64` cells, which hold every `i32` and `f32` exactly, so the
//! imports give bit for bit the values the module was fed (`input_i` truncates the cell,
//! `input_f` rounds it to `f32`: both exact for a value that came from an `i32`/`f32` port).
//! The module keeps its own state between steps.
//! Indexes follow the order of ports and parameters in `block.toml`.

pub mod builtin;
pub mod bus;
pub mod display;
pub mod mcu;
pub mod schema;

pub use crab_blocks::{Bench, Signal, OWN_SLAVE};
pub use display::{DisplayDevice, DisplayKind, DisplayView};
pub use schema::{BenchInput, Computed, Simulation};

use wasmi::{
    Caller, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

/// Fuel for one `step` or `init`: a runaway loop is stopped instead of hanging the editor.
const FUEL_PER_CALL: u64 = 5_000_000;
/// Linear memory a module may grow to.
const MAX_MEMORY: usize = 16 * 1024 * 1024;
/// Log lines kept between reads.
const MAX_LOG_LINES: usize = 200;
/// Longest text a module may set on an output, bytes.
const MAX_TEXT: usize = 4096;

struct Host {
    inputs: Vec<f64>,
    params: Vec<f64>,
    outputs: Vec<f64>,
    input_texts: Vec<String>,
    param_texts: Vec<String>,
    output_texts: Vec<String>,
    log: Vec<String>,
    limits: StoreLimits,
}

/// Import module of the board's bus functions: a `block.wasm` that imports from it is a
/// hardware driver (`specs/wasm-drivers`) and runs only on the board.
pub const BUS_MODULE: &str = "bus";

/// Whether `wasm` is a hardware driver: it imports the board's bus functions.
pub fn is_driver(wasm: &[u8]) -> bool {
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::ImportSection(imports)) = payload {
            return imports
                .into_imports()
                .any(|i| i.is_ok_and(|i| i.module == BUS_MODULE));
        }
    }
    false
}

/// A loaded block module with its state.
pub struct SimModule {
    store: Store<Host>,
    step: TypedFunc<i32, ()>,
    init: Option<TypedFunc<(), ()>>,
}

impl SimModule {
    /// Loads a module for a block with `n_outputs` outputs and runs its `init`, if there is one.
    pub fn load(wasm: &[u8], n_outputs: usize) -> Result<Self, String> {
        if is_driver(wasm) {
            return Err("это драйвер железа (функции bus.*): на компьютере он не выполняется,                  в симуляции схемы выходы берут значения sim из block.toml"
                .to_owned());
        }
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module =
            Module::new(&engine, wasm).map_err(|e| format!("модуль WASM не загружен: {e}"))?;

        let mut store = Store::new(
            &engine,
            Host {
                inputs: Vec::new(),
                params: Vec::new(),
                outputs: vec![0.0; n_outputs],
                input_texts: Vec::new(),
                param_texts: Vec::new(),
                output_texts: vec![String::new(); n_outputs],
                log: Vec::new(),
                limits: StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build(),
            },
        );
        store.limiter(|h| &mut h.limits);

        let mut linker = <Linker<Host>>::new(&engine);
        let bad = |e: wasmi::errors::LinkerError| format!("ошибка подключения функций: {e}");
        // Text and log imports (module `crab`; the old `f64` interface and module `flp` are gone).
        {
            linker
                .func_wrap(
                    "crab",
                    "input_text",
                    |mut c: Caller<'_, Host>, i: i32, ptr: i32, cap: i32| -> i32 {
                        let text = c.data().input_texts.get(i as usize).cloned();
                        copy_text(&mut c, text, ptr, cap)
                    },
                )
                .map_err(bad)?
                .func_wrap(
                    "crab",
                    "param_text",
                    |mut c: Caller<'_, Host>, i: i32, ptr: i32, cap: i32| -> i32 {
                        let text = c.data().param_texts.get(i as usize).cloned();
                        copy_text(&mut c, text, ptr, cap)
                    },
                )
                .map_err(bad)?
                .func_wrap(
                    "crab",
                    "output_text",
                    |mut c: Caller<'_, Host>, i: i32, ptr: i32, len: i32| {
                        let Some(memory) = c.get_export("memory").and_then(|e| e.into_memory())
                        else {
                            return;
                        };
                        let mut bytes = vec![0u8; (len.max(0) as usize).min(MAX_TEXT)];
                        if memory.read(&c, ptr as usize, &mut bytes).is_ok() {
                            if let Some(slot) = c.data_mut().output_texts.get_mut(i as usize) {
                                *slot = String::from_utf8_lossy(&bytes).into_owned();
                            }
                        }
                    },
                )
                .map_err(bad)?
                .func_wrap(
                    "crab",
                    "log",
                    |mut c: Caller<'_, Host>, ptr: i32, len: i32| {
                        let Some(memory) = c.get_export("memory").and_then(|e| e.into_memory())
                        else {
                            return;
                        };
                        let mut bytes = vec![0u8; (len.max(0) as usize).min(1024)];
                        if memory.read(&c, ptr as usize, &mut bytes).is_ok() {
                            let log = &mut c.data_mut().log;
                            if log.len() < MAX_LOG_LINES {
                                log.push(String::from_utf8_lossy(&bytes).into_owned());
                            }
                        }
                    },
                )
                .map_err(bad)?;
        }

        // Value imports.
        linker
            .func_wrap("crab", "input_i", |c: Caller<'_, Host>, i: i32| -> i32 {
                to_i32(c.data().inputs.get(i as usize).copied())
            })
            .map_err(bad)?
            .func_wrap("crab", "input_f", |c: Caller<'_, Host>, i: i32| -> f32 {
                c.data().inputs.get(i as usize).map_or(0.0, |&x| x as f32)
            })
            .map_err(bad)?
            .func_wrap("crab", "param_i", |c: Caller<'_, Host>, i: i32| -> i32 {
                to_i32(c.data().params.get(i as usize).copied())
            })
            .map_err(bad)?
            .func_wrap("crab", "param_f", |c: Caller<'_, Host>, i: i32| -> f32 {
                c.data().params.get(i as usize).map_or(0.0, |&x| x as f32)
            })
            .map_err(bad)?
            .func_wrap(
                "crab",
                "output_i",
                |mut c: Caller<'_, Host>, i: i32, v: i32| {
                    if let Some(slot) = c.data_mut().outputs.get_mut(i as usize) {
                        *slot = f64::from(v);
                    }
                },
            )
            .map_err(bad)?
            .func_wrap(
                "crab",
                "output_f",
                |mut c: Caller<'_, Host>, i: i32, v: f32| {
                    if let Some(slot) = c.data_mut().outputs.get_mut(i as usize) {
                        *slot = f64::from(v);
                    }
                },
            )
            .map_err(bad)?;

        let instance: Instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("модуль не запущен: {e}"))?;
        let step = instance
            .get_typed_func::<i32, ()>(&store, "step")
            .map_err(|_| "в модуле нет функции step(dt_ms: i32)".to_owned())?;
        let init = instance.get_typed_func::<(), ()>(&store, "init").ok();

        let mut sim = Self { store, step, init };
        sim.reset()?;
        Ok(sim)
    }

    /// Runs `init` again; the module drops its state.
    pub fn reset(&mut self) -> Result<(), String> {
        self.store
            .data_mut()
            .outputs
            .iter_mut()
            .for_each(|o| *o = 0.0);
        self.store
            .data_mut()
            .output_texts
            .iter_mut()
            .for_each(String::clear);
        let Some(init) = self.init else {
            return Ok(());
        };
        self.refuel()?;
        init.call(&mut self.store, ()).map_err(describe)
    }

    /// One step of `dt_ms` milliseconds; returns the outputs of the block.
    pub fn step(&mut self, dt_ms: u32, inputs: &[f64], params: &[f64]) -> Result<Vec<f64>, String> {
        self.step_text(dt_ms, inputs, params, &[], &[])
            .map(|(outputs, _)| outputs)
    }

    /// Like [`SimModule::step`] for a block with string ports and parameters: the texts
    /// are indexed like the numbers (empty for the other ports); returns the output texts too.
    pub fn step_text(
        &mut self,
        dt_ms: u32,
        inputs: &[f64],
        params: &[f64],
        input_texts: &[String],
        param_texts: &[String],
    ) -> Result<(Vec<f64>, Vec<String>), String> {
        let host = self.store.data_mut();
        host.inputs = inputs.to_vec();
        host.params = params.to_vec();
        host.input_texts = input_texts.to_vec();
        host.param_texts = param_texts.to_vec();
        self.refuel()?;
        self.step
            .call(&mut self.store, dt_ms as i32)
            .map_err(describe)?;
        let host = self.store.data();
        Ok((host.outputs.clone(), host.output_texts.clone()))
    }

    /// Lines the module wrote with `log` since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().log)
    }

    fn refuel(&mut self) -> Result<(), String> {
        self.store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|e| format!("ошибка лимита выполнения: {e}"))
    }
}

/// A host cell as an `i32` (saturating, like `i32.trunc_sat_f64_s`); 0 when there is no cell.
fn to_i32(x: Option<f64>) -> i32 {
    x.map_or(0, |x| x as i32)
}

/// Copies `text` into the module memory (at most `cap` bytes); returns the number of bytes copied.
fn copy_text(c: &mut Caller<'_, Host>, text: Option<String>, ptr: i32, cap: i32) -> i32 {
    let Some(text) = text else {
        return 0;
    };
    let Some(memory) = c.get_export("memory").and_then(|e| e.into_memory()) else {
        return 0;
    };
    let bytes = text.as_bytes();
    let n = bytes.len().min(cap.max(0) as usize);
    match memory.write(&mut *c, ptr as usize, &bytes[..n]) {
        Ok(()) => n as i32,
        Err(_) => 0,
    }
}

/// A trap of the module in plain Russian.
fn describe(e: wasmi::Error) -> String {
    use wasmi::TrapCode;
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => {
            "модуль выполняется слишком долго (возможно, бесконечный цикл)".to_owned()
        }
        Some(TrapCode::MemoryOutOfBounds) => "модуль обратился за пределы памяти".to_owned(),
        Some(TrapCode::IntegerDivisionByZero) => "в модуле деление целого на ноль".to_owned(),
        Some(code) => format!("модуль остановлен: {code}"),
        None => format!("ошибка модуля: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(wat_src: &str, outputs: usize) -> Result<SimModule, String> {
        SimModule::load(&wat::parse_str(wat_src).unwrap(), outputs)
    }

    /// out0 = in0 + in1 * param0
    const SUM: &str = r#"(module
        (import "crab" "input_i" (func $input (param i32) (result i32)))
        (import "crab" "param_i" (func $param (param i32) (result i32)))
        (import "crab" "output_i" (func $output (param i32 i32)))
        (func (export "step") (param i32)
            (call $output (i32.const 0)
                (i32.add (call $input (i32.const 0))
                    (i32.mul (call $input (i32.const 1)) (call $param (i32.const 0)))))))"#;

    /// A pulse counter: out0 counts the steps, `init` resets it; the first step also logs "hi".
    const COUNTER: &str = r#"(module
        (import "crab" "output_i" (func $output (param i32 i32)))
        (import "crab" "log" (func $log (param i32 i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "hi")
        (global $n (mut i32) (i32.const 0))
        (func (export "init") (global.set $n (i32.const 0)))
        (func (export "step") (param i32)
            (global.set $n (i32.add (global.get $n) (i32.const 1)))
            (if (i32.eq (global.get $n) (i32.const 1))
                (then (call $log (i32.const 0) (i32.const 2))))
            (call $output (i32.const 0) (global.get $n))))"#;

    /// The old `f64` interface does not link any more.
    #[test]
    fn the_old_f64_interface_does_not_link() {
        let old = r#"(module
            (import "crab" "input" (func (param i32) (result f64)))
            (func (export "step") (param i32)))"#;
        assert!(load(old, 1).is_err());
        let flp = r#"(module
            (import "flp" "log" (func (param i32 i32)))
            (func (export "step") (param i32)))"#;
        assert!(load(flp, 1).is_err());
    }

    /// v2: out0 (int) = in0 + param0; out1 (float) = in1 * param1; out2 = copied length of text in2.
    const V2: &str = r#"(module
        (import "crab" "input_i" (func $ii (param i32) (result i32)))
        (import "crab" "input_f" (func $if (param i32) (result f32)))
        (import "crab" "param_i" (func $pi (param i32) (result i32)))
        (import "crab" "param_f" (func $pf (param i32) (result f32)))
        (import "crab" "output_i" (func $oi (param i32 i32)))
        (import "crab" "output_f" (func $of (param i32 f32)))
        (import "crab" "input_text" (func $it (param i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "step") (param i32)
            (call $oi (i32.const 0) (i32.add (call $ii (i32.const 0)) (call $pi (i32.const 0))))
            (call $of (i32.const 1) (f32.mul (call $if (i32.const 1)) (call $pf (i32.const 1))))
            (call $oi (i32.const 2) (call $it (i32.const 2) (i32.const 16) (i32.const 3)))
            (call $oi (i32.const 3) (i32.load8_u (i32.const 16)))))"#;

    #[test]
    fn v2_imports_are_exact_i32_and_f32() {
        let mut m = load(V2, 4).unwrap();
        let (out, _) = m
            .step_text(
                10,
                &[41.0, f64::from(0.1f32), 0.0],
                &[1.0, 3.0],
                &["".into(), "".into(), "hello".into()],
                &[],
            )
            .unwrap();
        assert_eq!(out[0], 42.0);
        // f32 multiplication, not f64: 0.1f32 * 3.0f32.
        assert_eq!(out[1] as f32, 0.1f32 * 3.0f32);
        assert_ne!(out[1], f64::from(0.1f32) * 3.0);
        // The text is cut to the capacity and the copied length comes back.
        assert_eq!(out[2], 3.0);
        assert_eq!(out[3], f64::from(b'h'));
        // Missing cells read 0.
        let (out, _) = m.step_text(10, &[], &[], &[], &[]).unwrap();
        assert_eq!(out[..3], [0.0; 3]);
    }

    #[test]
    fn computes_outputs_from_inputs_and_params() {
        let mut m = load(SUM, 1).unwrap();
        assert_eq!(m.step(10, &[1.0, 2.0], &[3.0]).unwrap(), vec![7.0]);
        assert_eq!(m.step(10, &[5.0, 0.0], &[3.0]).unwrap(), vec![5.0]);
    }

    #[test]
    fn keeps_state_between_steps_and_resets() {
        let mut m = load(COUNTER, 1).unwrap();
        m.step(10, &[], &[]).unwrap();
        m.step(10, &[], &[]).unwrap();
        assert_eq!(m.step(10, &[], &[]).unwrap(), vec![3.0]);
        m.reset().unwrap();
        assert_eq!(m.step(10, &[], &[]).unwrap(), vec![1.0]);
    }

    #[test]
    fn module_log_is_collected() {
        let mut m = load(COUNTER, 1).unwrap();
        m.step(10, &[], &[]).unwrap();
        assert_eq!(m.take_log(), vec!["hi".to_owned()]);
        assert!(m.take_log().is_empty());
    }

    #[test]
    fn endless_loop_is_stopped() {
        let mut m = load(
            r#"(module (func (export "step") (param i32) (loop $l (br $l))))"#,
            0,
        )
        .unwrap();
        let err = m.step(10, &[], &[]).unwrap_err();
        assert!(err.contains("слишком долго"), "{err}");
        // The editor stays usable: the next call is stopped the same way, not hung.
        assert!(m.step(10, &[], &[]).is_err());
    }

    #[test]
    fn broken_modules_give_russian_errors() {
        let err = SimModule::load(b"not wasm", 0).err().unwrap();
        assert!(err.contains("не загружен"), "{err}");
        let err = load(r#"(module (func (export "other")))"#, 0)
            .err()
            .unwrap();
        assert!(err.contains("step"), "{err}");
        let err = load(
            r#"(module (import "env" "x" (func)) (func (export "step") (param i32)))"#,
            0,
        )
        .err()
        .unwrap();
        assert!(err.contains("env") || err.contains("x"), "{err}");
    }

    /// The Rust example module of `examples/wasm_block`: exponential smoothing.
    #[test]
    fn example_lowpass_module_smooths() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wasm_block/block/block.wasm"
        );
        let Ok(bytes) = std::fs::read(path) else {
            return; // not built on this machine (see examples/wasm_block/build.ps1)
        };
        let mut m = match SimModule::load(&bytes, 1) {
            Ok(m) => m,
            // Built for the old `f64` interface: needs a rebuild by `skills/crablogic-block`.
            Err(e) if e.contains("flp") => return,
            Err(e) => panic!("{e}"),
        };
        let mut last = 0.0;
        for _ in 0..50 {
            let out = m.step(100, &[10.0], &[1000.0]).unwrap();
            assert!(out[0] >= last && out[0] < 10.0);
            last = out[0];
        }
        assert!((last - 10.0 * (1.0 - (1000.0f64 / 1100.0).powi(50))).abs() < 1e-3);
    }
}
