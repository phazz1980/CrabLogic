//! A block with its own `block.wasm` (a hardware driver, `specs/wasm-drivers`) is pasted into
//! the sheet module: its inputs, parameters, outputs, memory, globals and `init` work, its bus
//! calls reach the master with pointers moved into its memory window, and its resources are
//! claimed before its `init`.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, Word};

const DRIVER: &str = r#"(module
  (import "crab" "input" (func $in (param i32) (result f64)))
  (import "crab" "param" (func $param (param i32) (result f64)))
  (import "crab" "output" (func $out (param i32 f64)))
  (import "bus" "i2c_read" (func $rd (param i32 i32 i32) (result i32)))
  (memory 1)
  (global $n (mut i32) (i32.const 0))
  (global $hb i32 (i32.const 256))
  (export "__heap_base" (global $hb))
  (data (i32.const 64) "\00\00\00\00\00\00\59\40")
  (func (export "init")
    (global.set $n (i32.add (i32.const 10) (i32.trunc_f64_s (call $param (i32.const 1))))))
  (func (export "step") (param i32)
    (global.set $n (i32.add (global.get $n) (i32.const 1)))
    (call $out (i32.const 0)
      (f64.add (f64.mul (call $in (i32.const 0)) (f64.const 2)) (call $param (i32.const 1))))
    (call $out (i32.const 1)
      (f64.add (f64.load (i32.const 64)) (f64.convert_i32_s (global.get $n))))
    (drop (call $rd (i32.trunc_f64_s (call $param (i32.const 0))) (i32.const 128) (i32.const 2)))
    (call $out (i32.const 2)
      (f64.convert_i32_u (i32.or (i32.shl (i32.load8_u (i32.const 128)) (i32.const 8))
                                 (i32.load8_u (i32.const 129)))))
    (loop $l (br_if $l (i32.const 0)))))"#;

const BLOCK: &str = r#"[block]
type = "drv"
name = "Датчик"
group = "Тест"

[[input]]
name = "x"
type = "float"
default = 1.5

[[output]]
name = "a"
type = "float"

[[output]]
name = "b"
type = "float"

[[output]]
name = "raw"
type = "int"

[[param]]
name = "address"
type = "int"
default = 118
bus = "i2c-address"

[[param]]
name = "offset"
type = "float"
default = 0.25

[[param]]
name = "sda"
type = "pin"
mode = "io"
default = 21
bus = "i2c-sda"

[[param]]
name = "scl"
type = "pin"
mode = "out"
default = 22
bus = "i2c-scl"
"#;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn setup(root: &Path, logic: &str) -> (Library, Project) {
    let _ = std::fs::remove_dir_all(root);
    let lib_dir = root.join("lib");
    write(&lib_dir.join("drv/block.toml"), BLOCK);
    std::fs::write(
        lib_dir.join("drv/block.wasm"),
        wat::parse_str(DRIVER).unwrap(),
    )
    .unwrap();
    let (lib, errors) = Library::load(&lib_dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = root.join("project");
    write(
        &dir.join("project.toml"),
        "[project]\nname = \"driver_module\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    );
    write(&dir.join("logic.toml"), logic);
    (lib, Project::load(&dir).unwrap())
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()))
}

#[test]
fn driver_runs_inside_the_sheet_module() {
    let root = temp_root("driver_module");
    let (lib, project) = setup(&root, "[[block]]\nid = \"sensor\"\ntype = \"drv\"\n");
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    wasmparser::Validator::new()
        .validate_all(&built.wasm)
        .unwrap();
    wasmparser::Validator::new()
        .validate_all(&built.wasm_aot)
        .unwrap();
    assert!(built.needs.contains(&"bus".to_owned()), "{:?}", built.needs);
    let manifest = Manifest::parse(&built.manifest).unwrap();
    assert_eq!(manifest.allow, ["bus"]);
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.io_mut().i2c_in = vec![0x12, 0x34];
    module.step(10).unwrap();

    let out = |module: &McuModule, label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    assert_eq!(out(&module, "sensor.a"), Word::float(3.25));
    // Data segment (100.0) plus the counter set by `init` (10) and one step.
    assert_eq!(out(&module, "sensor.b"), Word::float(111.0));
    assert_eq!(out(&module, "sensor.raw"), Word::int(0x1234));

    let calls = module.io_mut().bus_calls.clone();
    // Claims in parameter order, the lock, then the step's read into the window.
    assert_eq!(calls[0], [0, 2, 118, 0, 0]);
    assert_eq!(calls[1], [0, 1, 21, 0, 0]);
    assert_eq!(calls[2], [0, 1, 22, 0, 0]);
    assert_eq!(calls[3], [1, 0, 0, 0, 0]);
    let read = calls[4];
    assert_eq!((read[0], read[1], read[3]), (22, 118, 2));
    assert!(
        read[2] >= 128 + 16,
        "pointer moved into the window: {read:?}"
    );

    module.step(10).unwrap();
    assert_eq!(out(&module, "sensor.b"), Word::float(112.0));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn two_instances_keep_their_own_state() {
    let root = temp_root("driver_module_two");
    let (lib, project) = setup(
        &root,
        "[[block]]\nid = \"s1\"\ntype = \"drv\"\noffset = 1.0\n\n[[block]]\nid = \"s2\"\ntype = \"drv\"\noffset = 2.0\naddress = 119\nsda = 21\nscl = 22\n",
    );
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    module.step(10).unwrap();
    let out = |label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    assert_eq!(out("s1.a"), Word::float(4.0));
    assert_eq!(out("s2.a"), Word::float(5.0));
    // `init` saw the parameter: 100 + 10 + offset + two steps.
    assert_eq!(out("s1.b"), Word::float(113.0));
    assert_eq!(out("s2.b"), Word::float(114.0));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_driver_with_a_table_is_refused_with_its_block() {
    let root = temp_root("driver_module_bad");
    let (lib, project) = setup(&root, "[[block]]\nid = \"sensor\"\ntype = \"drv\"\n");
    std::fs::write(
        root.join("lib/drv/block.wasm"),
        wat::parse_str(
            r#"(module (table 1 funcref) (type $t (func))
                 (func (export "step") (param i32) (call_indirect (type $t) (i32.const 0))))"#,
        )
        .unwrap(),
    )
    .unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let err = program.wasm_module(1).err().unwrap().to_string();
    assert!(
        err.contains("sensor") && err.contains("call_indirect"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn spi_and_onewire_calls_reach_the_master() {
    let root = temp_root("driver_module_spi");
    let (lib, project) = setup(&root, "[[block]]\nid = \"sensor\"\ntype = \"drv\"\n");
    std::fs::write(
        root.join("lib/drv/block.wasm"),
        wat::parse_str(
            r#"(module
              (import "bus" "spi_open" (func $so (param i32 i32 i32 i32) (result i32)))
              (import "bus" "spi_transfer" (func $st (param i32 i32 i32 i32) (result i32)))
              (import "bus" "ow_reset" (func $or (param i32) (result i32)))
              (import "bus" "ow_write" (func $ow (param i32 i32 i32) (result i32)))
              (import "bus" "ow_read" (func $orr (param i32 i32 i32) (result i32)))
              (import "bus" "ow_read_bit" (func $orb (param i32) (result i32)))
              (import "bus" "ow_write_bit" (func $owb (param i32 i32) (result i32)))
              (import "bus" "pulse_read" (func $pr (param i32 i32 i32 i32) (result i32)))
              (memory 1)
              (global i32 (i32.const 256))
              (export "__heap_base" (global 0))
              (func (export "step") (param i32)
                (drop (call $so (i32.const 18) (i32.const 23) (i32.const 19) (i32.const 1000000)))
                (drop (call $st (i32.const 5) (i32.const 32) (i32.const 4) (i32.const 3)))
                (drop (call $or (i32.const 4)))
                (drop (call $ow (i32.const 4) (i32.const 40) (i32.const 2)))
                (drop (call $orr (i32.const 4) (i32.const 48) (i32.const 9)))
                (drop (call $orb (i32.const 4)))
                (drop (call $owb (i32.const 4) (i32.const 1)))
                (drop (call $pr (i32.const 4) (i32.const 1100) (i32.const 64) (i32.const 84)))))"#,
        )
        .unwrap(),
    )
    .unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    wasmparser::Validator::new()
        .validate_all(&built.wasm_aot)
        .unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    let calls: Vec<[i32; 5]> = module
        .io_mut()
        .bus_calls
        .iter()
        .copied()
        .filter(|c| c[0] >= 50)
        .collect();
    // Pointers move into the window by the same base.
    let base = calls[1][2] - 32;
    assert!(base > 0);
    assert_eq!(
        calls,
        [
            [50, 18, 23, 19, 1_000_000],
            [51, 5, base + 32, 4, 3],
            [60, 4, 0, 0, 0],
            [61, 4, base + 40, 2, 0],
            [62, 4, base + 48, 9, 0],
            [63, 4, 0, 0, 0],
            [64, 4, 1, 0, 0],
            [65, 4, 1100, base + 64, 84],
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}
