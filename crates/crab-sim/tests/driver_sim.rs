//! A hardware driver (`block.wasm` importing `bus.*`, `specs/wasm-drivers`) runs only on the
//! board; in the simulation its outputs take the `sim` values of `block.toml`.

use crab_core::{Library, Logic};
use crab_sim::{Computed, SimModule, Simulation};

const DRIVER: &str = r#"(module
  (import "bus" "i2c_read" (func $read (param i32 i32 i32) (result i32)))
  (import "crab" "output" (func $output (param i32 f64)))
  (memory 1)
  (func (export "step") (param i32)
    (drop (call $read (i32.const 118) (i32.const 0) (i32.const 2)))
    (call $output (i32.const 0) (f64.const 99))))"#;

fn library(root: &std::path::Path) -> Library {
    let dir = root.join("drv_demo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("block.toml"),
        "[block]\ntype = \"drv_demo\"\nname = \"Датчик\"\ngroup = \"Тест\"\n\n\
         [[output]]\nname = \"t\"\ntype = \"float\"\nsim = 21.5\n\n\
         [[output]]\nname = \"ok\"\ntype = \"bool\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("block.wasm"), wat::parse_str(DRIVER).unwrap()).unwrap();
    Library::load(root).unwrap().0
}

#[test]
fn driver_outputs_take_sim_values() {
    let root = std::env::temp_dir().join("crab_driver_sim_test");
    let _ = std::fs::remove_dir_all(&root);
    let lib = library(&root);
    let logic: Logic = toml::from_str("[[block]]\nid = \"sensor\"\ntype = \"drv_demo\"\n").unwrap();
    let mut sim = Simulation::new(&logic, &lib).unwrap();
    assert_eq!(sim.computed("sensor"), Some(Computed::Manual));
    assert_eq!(sim.output("sensor", "t"), Some(21.5));
    assert_eq!(sim.output("sensor", "ok"), Some(0.0));
    sim.step(10).unwrap();
    assert_eq!(sim.output("sensor", "t"), Some(21.5));
    assert!(sim.notes.is_empty(), "{:?}", sim.notes);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn trial_run_says_a_driver_runs_only_on_the_board() {
    let wasm = wat::parse_str(DRIVER).unwrap();
    assert!(crab_sim::is_driver(&wasm));
    let err = SimModule::load(&wasm, 2).err().unwrap();
    assert!(err.contains("драйвер железа"), "{err}");
    let plain = wat::parse_str(r#"(module (func (export "step") (param i32)))"#).unwrap();
    assert!(!crab_sim::is_driver(&plain));
}

#[test]
fn sim_value_only_on_outputs() {
    let toml = "[block]\ntype = \"x\"\nname = \"X\"\ngroup = \"Тест\"\n\n\
                [[input]]\nname = \"a\"\ntype = \"int\"\nsim = 1\n";
    let def: crab_core::BlockDef = toml::from_str(toml).unwrap();
    assert!(def.validate("x").unwrap_err().contains("sim"));
}
