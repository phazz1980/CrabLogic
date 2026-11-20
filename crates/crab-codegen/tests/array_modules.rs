//! Array blocks in sheet modules (spec arrays-structs, task 4): every block on every numeric
//! element type builds a valid module; what a module cannot take gives a clear error.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::ElemType;
use crab_core::{Library, Project};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn library() -> Library {
    let (lib, errors) = Library::load(repo("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

/// A project folder with the given `logic.toml` and `layout.toml` texts.
fn project(tag: &str, logic: &str, layout: &str) -> (Project, PathBuf) {
    let dir = std::env::temp_dir().join(format!("crab_array_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    std::fs::write(dir.join("layout.toml"), layout).unwrap();
    (Project::load(&dir).unwrap(), dir)
}

/// Builds the module of every sheet; the wasm of each is validated by `wasmparser`.
fn build(project: &Project, lib: &Library) -> Result<Vec<Vec<u8>>, String> {
    let program = Program::build(project, lib).map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    for sheet in program.sheets() {
        let module = program.wasm_module(sheet).map_err(|e| e.to_string())?;
        wasmparser::Validator::new()
            .validate_all(&module.wasm)
            .map_err(|e| format!("sheet {sheet}: {e}"))?;
        all.push(module.wasm);
    }
    Ok(all)
}

/// A schema with one block of `op` on an array of `ty` (and a second array for the copy).
fn one_block(op: ArrayOp, ty: ElemType) -> String {
    let t = ty.name();
    let init = match ty {
        ElemType::Bool => "true",
        ElemType::Float => "1.5",
        ElemType::String => "\"ab\"",
        _ => "7",
    };
    let dest = if op == ArrayOp::Copy {
        "dest = \"b\"\n"
    } else {
        ""
    };
    // Inputs without a default value need a constant.
    let value = if op.inputs().iter().any(|(name, _, _)| *name == "value") {
        format!("[block.const]\nvalue = {init}\n")
    } else {
        String::new()
    };
    format!(
        "[[array]]\nname = \"a\"\ntype = \"{t}\"\nlen = 4\nitems = [{init}]\n\n\
         [[array]]\nname = \"b\"\ntype = \"{t}\"\nlen = 3\n\n\
         [[block]]\nid = \"x\"\ntype = \"{}\"\narray = \"a\"\n{dest}{value}",
        op.kind()
    )
}

#[test]
fn every_array_block_builds_a_valid_module_on_every_numeric_type() {
    let lib = library();
    let mut built = 0;
    for op in ArrayOp::ALL {
        for ty in [
            ElemType::Bool,
            ElemType::Int,
            ElemType::Float,
            ElemType::Byte,
        ] {
            if !op.exists_for(ty) {
                continue;
            }
            let (p, dir) = project("each", &one_block(op, ty), "");
            let result = build(&p, &lib);
            let _ = std::fs::remove_dir_all(dir);
            let modules = result.unwrap_or_else(|e| panic!("{} on {}: {e}", op.kind(), ty.name()));
            assert_eq!(modules.len(), 1);
            built += 1;
        }
    }
    // 8 blocks on 4 types + sum, min, max on 3 numeric types.
    assert_eq!(built, 8 * 4 + 3 * 3);
}

/// Every block that exists on an array of strings builds a valid module (`array_text.rs`).
#[test]
fn every_array_block_builds_a_valid_module_on_strings() {
    let lib = library();
    let mut built = 0;
    for op in ArrayOp::ALL {
        if !op.exists_for(ElemType::String) {
            continue;
        }
        let (p, dir) = project("text", &one_block(op, ElemType::String), "");
        let result = build(&p, &lib);
        let _ = std::fs::remove_dir_all(dir);
        let modules = result.unwrap_or_else(|e| panic!("{} on strings: {e}", op.kind()));
        assert_eq!(modules.len(), 1);
        built += 1;
    }
    // get, set, fill, copy, find, shift, ring, stack: sum, min and max have no string form.
    assert_eq!(built, 8);
}

/// Two string arrays of the longest length a chip takes (256) need 2 * 256 * 132 bytes, more than the
/// 64 KB page: the module is refused with the memory reason, not a panic.
#[test]
fn string_arrays_that_do_not_fit_the_memory_page_are_refused() {
    let lib = library();
    let mut logic = String::new();
    for name in ["s1", "s2"] {
        logic.push_str(&format!(
            "[[array]]\nname = \"{name}\"\ntype = \"string\"\nlen = 256\n\n\
             [[block]]\nid = \"g{name}\"\ntype = \"array_get\"\narray = \"{name}\"\n\
             [block.const]\nindex = 0\n\n"
        ));
    }
    let (p, dir) = project("text_big", &logic, "");
    let err = build(&p, &lib).unwrap_err();
    let _ = std::fs::remove_dir_all(dir);
    assert!(err.contains("памяти"), "{err}");
}

/// A copy between a string array and a numeric one is not a schema the checker lets through: the
/// check reports the mismatch before any module is made (the codegen guard is the second line).
#[test]
fn a_copy_between_string_and_numeric_arrays_is_refused() {
    let lib = library();
    let logic = "[[array]]\nname = \"s\"\ntype = \"string\"\nlen = 2\n\n\
                 [[array]]\nname = \"n\"\ntype = \"int\"\nlen = 2\n\n\
                 [[block]]\nid = \"c\"\ntype = \"array_copy\"\narray = \"s\"\ndest = \"n\"\n";
    let (p, dir) = project("text_mixed", logic, "");
    let report = crab_core::check::check_project(&p, &lib);
    let _ = std::fs::remove_dir_all(dir);
    assert!(
        report.errors().any(|d| d.message.contains("разных типов")),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn an_array_of_two_sheets_is_refused_with_a_reason() {
    let lib = library();
    let logic = "[[array]]\nname = \"a\"\ntype = \"int\"\nlen = 4\n\n\
                 [[block]]\nid = \"w\"\ntype = \"array_set\"\narray = \"a\"\n\
                 [block.const]\nindex = 0\nvalue = 1\n\n\
                 [[block]]\nid = \"r\"\ntype = \"array_get\"\narray = \"a\"\n\
                 [block.const]\nindex = 0\n";
    let layout = "[[sheet]]\nnumber = 2\nname = \"Вторая\"\n\n\
                  [block.w]\nx = 0.0\ny = 0.0\nsheet = 1\n\n\
                  [block.r]\nx = 0.0\ny = 0.0\nsheet = 2\n";
    let (p, dir) = project("sheets", logic, layout);
    let err = build(&p, &lib).unwrap_err();
    let _ = std::fs::remove_dir_all(dir);
    assert!(err.contains("нескольких модулей"), "{err}");
}

#[test]
fn an_array_that_does_not_fit_the_memory_page_is_refused() {
    let lib = library();
    // 70 arrays of 256 ints take 1 KB each: together more than the 64 KB page.
    let mut logic = String::new();
    for k in 0..70 {
        logic.push_str(&format!(
            "[[array]]\nname = \"a{k}\"\ntype = \"int\"\nlen = 256\n\n\
             [[block]]\nid = \"g{k}\"\ntype = \"array_get\"\narray = \"a{k}\"\n\
             [block.const]\nindex = 0\n\n"
        ));
    }
    let (p, dir) = project("big", &logic, "");
    let err = build(&p, &lib).unwrap_err();
    let _ = std::fs::remove_dir_all(dir);
    assert!(err.contains("памяти"), "{err}");
}

/// Every array block is used in some example (the rule of the project: a block, an example).
#[test]
fn every_array_block_has_an_example() {
    let used: std::collections::BTreeSet<String> = std::fs::read_dir(repo("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.join("project.toml").is_file())
        .flat_map(|dir| Project::load(&dir).unwrap().logic.blocks)
        .map(|b| b.kind)
        .collect();
    let missing: Vec<&str> = ArrayOp::ALL
        .iter()
        .map(|op| op.kind())
        .filter(|kind| !used.contains(*kind))
        .collect();
    assert!(
        missing.is_empty(),
        "блоки массивов без примера: {missing:?}"
    );
}

#[test]
fn the_structure_example_builds_a_module() {
    let lib = library();
    let project = Project::load(repo("examples/struct_recipe")).unwrap();
    let modules = build(&project, &lib).unwrap();
    assert_eq!(modules.len(), 1);
}

#[test]
fn a_structure_on_two_sheets_is_refused() {
    let lib = library();
    let mut project = Project::load(repo("examples/struct_recipe")).unwrap();
    // Moves the unpack block to sheet 2: the two blocks of one structure are in different modules.
    for (id, sheet) in [("recipe_pack1", 1), ("recipe_unpack1", 2)] {
        project.layout.blocks.insert(
            id.into(),
            crab_core::BlockPos {
                sheet,
                x: 40.0,
                y: 40.0,
            },
        );
    }
    let err = build(&project, &lib).unwrap_err();
    assert!(err.contains("нескольких модулей"), "{err}");
}
