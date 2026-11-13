//! Interface v2 of `block.wasm` in a sheet module (`specs/no-f64`): `crab.input_i/input_f/
//! param_i/param_f/output_i/output_f` and the text imports work on 4-byte typed cells, and a
//! module with v2 blocks only has no `f64` in it. Wrong-typed imports and a mix of v1 and v2 are
//! refused with the block named.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, Word};
use wasmparser::{Parser, Payload};

const BLOCK: &str = r#"[block]
type = "v2t"
name = "Тест v2"
group = "Тест"

[[input]]
name = "a"
type = "int"
default = 7

[[input]]
name = "f"
type = "float"
default = 2.5

[[input]]
name = "flag"
type = "bool"
default = true

[[input]]
name = "s"
type = "string"

[[output]]
name = "oi"
type = "int"

[[output]]
name = "of"
type = "float"

[[output]]
name = "ob"
type = "bool"

[[output]]
name = "len"
type = "int"

[[output]]
name = "text"
type = "string"

[[param]]
name = "n"
type = "int"
default = 5

[[param]]
name = "k"
type = "float"
default = 0.5

[[param]]
name = "t"
type = "string"
default = "xyz"
"#;

/// oi = a + n; of = f * k; ob = flag; len = input_text(s, cap 4) * 10 + param_text(t, cap 8);
/// text = the first 4 bytes of `s` (written with output_text).
const WASM: &str = r#"(module
  (import "crab" "input_i" (func $ii (param i32) (result i32)))
  (import "crab" "input_f" (func $if (param i32) (result f32)))
  (import "crab" "param_i" (func $pi (param i32) (result i32)))
  (import "crab" "param_f" (func $pf (param i32) (result f32)))
  (import "crab" "output_i" (func $oi (param i32 i32)))
  (import "crab" "output_f" (func $of (param i32 f32)))
  (import "crab" "input_text" (func $it (param i32 i32 i32) (result i32)))
  (import "crab" "param_text" (func $pt (param i32 i32 i32) (result i32)))
  (import "crab" "output_text" (func $ot (param i32 i32 i32)))
  (memory 1)
  (global $hb i32 (i32.const 256))
  (export "__heap_base" (global $hb))
  (func (export "step") (param i32)
    (call $oi (i32.const 0) (i32.add (call $ii (i32.const 0)) (call $pi (i32.const 0))))
    (call $of (i32.const 1) (f32.mul (call $if (i32.const 1)) (call $pf (i32.const 1))))
    (call $oi (i32.const 2) (call $ii (i32.const 2)))
    (call $oi (i32.const 3)
      (i32.add
        (i32.mul (call $it (i32.const 3) (i32.const 32) (i32.const 4)) (i32.const 10))
        (call $pt (i32.const 2) (i32.const 64) (i32.const 8))))
    (call $ot (i32.const 4) (i32.const 32) (i32.const 4))
    ;; a bad range is ignored
    (call $ot (i32.const 4) (i32.const 100000) (i32.const 1))))"#;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn setup(name: &str, block: &str, wat_src: &str, logic: &str) -> (PathBuf, Library, Project) {
    let root = std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib_dir = root.join("lib");
    write(&lib_dir.join("v2t/block.toml"), block);
    // The text blocks of the repository library.
    for kind in ["text_const", "text_length"] {
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../library")
            .join(kind);
        for entry in std::fs::read_dir(&from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "block.wasm" {
                write(
                    &lib_dir.join(kind).join(entry.file_name()),
                    &std::fs::read_to_string(entry.path()).unwrap(),
                );
            }
        }
    }
    std::fs::write(
        lib_dir.join("v2t/block.wasm"),
        wat::parse_str(wat_src).unwrap(),
    )
    .unwrap();
    let (lib, errors) = Library::load(&lib_dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = root.join("project");
    write(
        &dir.join("project.toml"),
        "[project]\nname = \"v2\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    );
    write(&dir.join("logic.toml"), logic);
    let project = Project::load(&dir).unwrap();
    (root, lib, project)
}

/// Any `f64` type, local or operator in the module.
fn f64_uses(wasm: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            Payload::TypeSection(r) => {
                for group in r {
                    for sub in group.unwrap().into_types() {
                        if let wasmparser::CompositeInnerType::Func(f) = &sub.composite_type.inner {
                            if f.params()
                                .iter()
                                .chain(f.results())
                                .any(|v| *v == wasmparser::ValType::F64)
                            {
                                found.push("function type with f64".to_owned());
                            }
                        }
                    }
                }
            }
            Payload::GlobalSection(r) => {
                for g in r {
                    if g.unwrap().ty.content_type == wasmparser::ValType::F64 {
                        found.push("f64 global".to_owned());
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                for l in body.get_locals_reader().unwrap() {
                    if l.unwrap().1 == wasmparser::ValType::F64 {
                        found.push("f64 local".to_owned());
                    }
                }
                for op in body.get_operators_reader().unwrap() {
                    let op = format!("{:?}", op.unwrap());
                    if op.contains("F64") {
                        found.push(op);
                    }
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn v2_block_runs_in_the_sheet_module_without_f64() {
    let logic = "[[block]]\nid = \"tc\"\ntype = \"text_const\"\nvalue = \"hello\"\n\n\
        [[block]]\nid = \"b\"\ntype = \"v2t\"\n\n\
        [[block]]\nid = \"tl\"\ntype = \"text_length\"\n\n\
        [[link]]\nfrom = \"tc.out\"\nto = \"b.s\"\n\n\
        [[link]]\nfrom = \"b.text\"\nto = \"tl.in\"\n";
    let (root, lib, project) = setup("v2_module", BLOCK, WASM, logic);
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    for wasm in [&built.wasm, &built.wasm_aot] {
        wasmparser::Validator::new().validate_all(wasm).unwrap();
        let f64s = f64_uses(wasm);
        assert!(f64s.is_empty(), "f64 in the module: {f64s:?}");
    }
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut module = McuModule::load(&built.wasm, &manifest, built.watched.len()).unwrap();
    module.step(10).unwrap();
    let out = |label: &str| {
        let i = built.watched.iter().position(|l| l == label).unwrap();
        module.outputs()[i]
    };
    assert_eq!(out("b.oi"), Word::int(12));
    assert_eq!(out("b.of"), Word::float(1.25));
    assert_eq!(out("b.ob"), Word::int(1));
    // "hello" cut to 4 bytes by cap, "xyz" has 3: 4 * 10 + 3.
    assert_eq!(out("b.len"), Word::int(43));
    // The text output got "hell"; the bad range did not change it.
    assert_eq!(out("tl.out"), Word::int(4));
    let _ = std::fs::remove_dir_all(&root);
}

fn refused(name: &str, block: &str, wat_src: &str) -> String {
    // A default keeps the schema check quiet: the generator is what is under test.
    let block = block.replace(
        "name = \"a\"",
        "name = \"a\"
default = 1",
    );
    let block = block.as_str();
    let (root, lib, project) = setup(
        name,
        block,
        wat_src,
        "[[block]]\nid = \"b\"\ntype = \"v2t\"\n",
    );
    let program = Program::build(&project, &lib).unwrap();
    let err = program.wasm_module(1).err().unwrap().to_string();
    let _ = std::fs::remove_dir_all(&root);
    err
}

#[test]
fn an_import_of_the_wrong_type_is_refused() {
    // Only an int input and output: `input_f` and `output_f` have no port to serve.
    let block = "[block]\ntype = \"v2t\"\nname = \"x\"\ngroup = \"Тест\"\n\n\
        [[input]]\nname = \"a\"\ntype = \"int\"\n\n[[output]]\nname = \"o\"\ntype = \"int\"\n";
    for (import, name) in [
        (
            "(import \"crab\" \"input_f\" (func $g (param i32) (result f32)))",
            "input_f",
        ),
        (
            "(import \"crab\" \"output_f\" (func $g (param i32 f32)))",
            "output_f",
        ),
    ] {
        let wat_src = format!("(module {import} (func (export \"step\") (param i32)))");
        let err = refused("v2_wrong", block, &wat_src);
        assert!(err.contains(name) && err.contains("тип"), "{err}");
    }
    // `input_i` on a block with only a float input.
    let block = "[block]\ntype = \"v2t\"\nname = \"x\"\ngroup = \"Тест\"\n\n\
        [[input]]\nname = \"a\"\ntype = \"float\"\n";
    let wat_src = "(module (import \"crab\" \"input_i\" (func $g (param i32) (result i32)))
        (func (export \"step\") (param i32) (drop (call $g (i32.const 0)))))";
    let err = refused("v2_wrong_in", block, wat_src);
    assert!(err.contains("input_i"), "{err}");
}

/// The old `f64` interface is refused before anything else, also next to v2 imports.
#[test]
fn old_f64_import_next_to_v2_is_refused() {
    let block = "[block]\ntype = \"v2t\"\nname = \"x\"\ngroup = \"Тест\"\n\n\
        [[input]]\nname = \"a\"\ntype = \"int\"\n";
    let wat_src = "(module
        (import \"crab\" \"input\" (func $a (param i32) (result f64)))
        (import \"crab\" \"input_i\" (func $b (param i32) (result i32)))
        (func (export \"step\") (param i32)))";
    let err = refused("v2_mixed", block, wat_src);
    assert!(err.contains("f64") && err.contains("пересоберите"), "{err}");
}
