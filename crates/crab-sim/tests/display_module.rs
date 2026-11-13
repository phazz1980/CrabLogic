//! A display block in a sheet module (`specs/display`): the driver gets the widget values and
//! the menu inputs after its own ports, and what it reports as a confirmed menu variable is
//! written to the variable and saved to flash in the same step. A fake `block.wasm` stands in
//! for the real driver and speaks the index contract of `specs/display/plan.md`.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, Word};

/// Ports of `oled`: P = 1 (`screen`), Q = 1 (`ok`); two widgets with an input, two variables.
/// Inputs: 0 screen, 1 and 2 widgets, 3..7 up/down/ok/back, 7 and 8 variables.
/// Outputs: 0 ok, 1 open, 2 item, 3 changed, 4 changed_slot, 5 and 6 new values.
///
/// It echoes `in(1) + 1000 * in(7)` to `ok` and `in(2)` to `item`; `up` confirms the variable of
/// slot 1 with 1, `down` the variable of slot 0 with 42.5.
const DRIVER: &str = r#"(module
  (import "crab" "input" (func $in (param i32) (result f64)))
  (import "crab" "output" (func $out (param i32 f64)))
  (memory 1)
  (global $hb i32 (i32.const 256))
  (export "__heap_base" (global $hb))
  (func (export "step") (param i32)
    (call $out (i32.const 0)
      (f64.add (call $in (i32.const 1)) (f64.mul (call $in (i32.const 7)) (f64.const 1000))))
    (call $out (i32.const 1) (f64.const 1))
    (call $out (i32.const 2) (call $in (i32.const 2)))
    (call $out (i32.const 3) (f64.const 0))
    (call $out (i32.const 4) (f64.const -1))
    (if (f64.gt (call $in (i32.const 3)) (f64.const 0)) (then
      (call $out (i32.const 3) (f64.const 1))
      (call $out (i32.const 4) (f64.const 1))
      (call $out (i32.const 6) (f64.const 1))))
    (if (f64.gt (call $in (i32.const 4)) (f64.const 0)) (then
      (call $out (i32.const 3) (f64.const 1))
      (call $out (i32.const 4) (f64.const 0))
      (call $out (i32.const 5) (f64.const 42.5))))))"#;

const LOGIC: &str = include_str!("fixtures/display_menu.toml");

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "block.wasm" {
            continue;
        }
        std::fs::copy(entry.path(), to.join(name)).unwrap();
    }
}

/// The library of the display blocks with the fake driver in `oled`, and the project whose menu
/// has `up` and `down` pressed as asked.
fn setup(name: &str, up: bool, down: bool) -> (PathBuf, Library, Project) {
    let root = std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib_dir = root.join("lib");
    for block in [
        "oled",
        "menu",
        "menu_item",
        "disp_text",
        "disp_value",
        "disp_bar",
    ] {
        copy_dir(&repo_path("library").join(block), &lib_dir.join(block));
    }
    std::fs::write(
        lib_dir.join("oled/block.wasm"),
        wat::parse_str(DRIVER).unwrap(),
    )
    .unwrap();
    let (lib, errors) = Library::load(&lib_dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = root.join("project");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"display_module\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    let logic = LOGIC
        .replace("UP_VALUE", &up.to_string())
        .replace("DOWN_VALUE", &down.to_string());
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    let project = Project::load(&dir).unwrap();
    (root, lib, project)
}

struct Run {
    built: crab_codegen::WasmModule,
    module: McuModule,
}

impl Run {
    fn new(lib: &Library, project: &Project) -> Run {
        let program = Program::build(project, lib).unwrap();
        let built = program.wasm_module_with(1, true).unwrap();
        wasmparser::Validator::new()
            .validate_all(&built.wasm)
            .unwrap();
        wasmparser::Validator::new()
            .validate_all(&built.wasm_aot)
            .unwrap();
        let manifest = Manifest::parse(&built.manifest).unwrap();
        let module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
        Run { built, module }
    }

    fn out(&self, label: &str) -> Word {
        let i = self.built.watched.iter().position(|l| l == label).unwrap();
        self.module.outputs()[i]
    }
}

#[test]
fn driver_reads_widget_inputs_and_menu_outputs_reach_the_menu_block() {
    let (root, lib, project) = setup("display_module_idle", false, false);
    let mut run = Run::new(&lib, &project);
    run.module.step(10).unwrap();
    run.module.step(10).unwrap();
    // Widget 0 is the constant 5.0, widget 1 the variable `level` (3.0); the menu variable
    // `setpoint` (slot 0, 20.0) comes after the four buttons.
    assert_eq!(run.out("oled.ok"), Word::int(20_005));
    assert_eq!(run.out("m.item"), Word::int(3));
    assert_eq!(run.out("m.open"), Word::int(1));
    assert_eq!(run.out("m.changed"), Word::int(0));
    assert_eq!(run.module.io().retain_writes, 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn confirmed_number_is_written_and_saved_at_once() {
    let (root, lib, project) = setup("display_module_down", false, true);
    let mut run = Run::new(&lib, &project);
    run.module.step(10).unwrap();
    // The first step: written and saved in that very step, though the value has not rested
    // for the 5 s of the default `settle`.
    assert_eq!(run.module.io().retain_writes, 1);
    assert!(run
        .module
        .io()
        .retain
        .values()
        .any(|&v| v == 42.5f32.to_bits() as i32));
    assert_eq!(run.out("m.changed"), Word::int(1));
    run.module.step(10).unwrap();
    // The variable changed (the driver now gets 42.5 for its echo) and nothing more is written.
    assert_eq!(run.out("oled.ok"), Word::int(42_505));
    assert_eq!(run.module.io().retain_writes, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn confirmed_bool_is_written_without_a_flash_write() {
    let (root, lib, project) = setup("display_module_up", true, false);
    let mut run = Run::new(&lib, &project);
    run.module.step(10).unwrap();
    run.module.step(10).unwrap();
    assert_eq!(run.out("en_mon.out"), Word::int(1));
    assert_eq!(run.out("sp_mon.out"), Word::float(20.0));
    // `enabled` is not retained.
    assert_eq!(run.module.io().retain_writes, 0);
    let _ = std::fs::remove_dir_all(&root);
}
