//! Every block of the library and every example goes to a sheet module (table-driven).
//! This is the coverage that replaced the old sketch snapshots: a block that the module
//! generator refuses fails here by name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crab_codegen::{GenError, Program};
use crab_core::{Library, Project, Severity};

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

/// Builds every sheet of the project as a module; the error text of the first failure.
fn modules(project: &Project, lib: &Library) -> Result<usize, String> {
    let project = crab_codegen::prepare(project, lib);
    let program = match Program::build(&project, lib) {
        Ok(p) => p,
        Err(GenError::Schema(diags)) => {
            let first = diags
                .iter()
                .find(|d| d.severity == Severity::Error)
                .map_or_else(|| "schema error".to_owned(), |d| d.message.clone());
            return Err(first);
        }
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = 0;
    for sheet in program.sheets() {
        let module = program.wasm_module(sheet).map_err(|e| e.to_string())?;
        assert!(!module.wasm.is_empty());
        bytes += module.wasm.len();
    }
    Ok(bytes)
}

#[test]
fn every_library_block_builds_a_module() {
    let lib = library();
    let root = std::env::temp_dir().join(format!("crab_lib_modules_{}", std::process::id()));
    // Screen widgets, menu items and the menu only exist next to a display: alone they fail the
    // schema check by design; `examples/display_menu` covers them below.
    let mut blocks: Vec<_> = lib
        .iter()
        .filter(|b| {
            let kind = b.def.info.kind.as_str();
            !crab_core::project::is_screen_kind(kind) && kind != "menu"
        })
        .collect();
    blocks.sort_by(|a, b| a.def.info.kind.cmp(&b.def.info.kind));
    assert!(blocks.len() > 100, "the library looks empty");
    let mut bad = Vec::new();
    for block in blocks {
        let kind = &block.def.info.kind;
        let result = crab_codegen::block_check::one_block_project(&root, &block.def)
            .map_err(|e| e.to_string())
            .and_then(|dir| Project::load(&dir).map_err(|e| e.to_string()))
            .and_then(|project| modules(&project, &lib));
        if let Err(e) = result {
            bad.push(format!("{kind}: {e}"));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn every_example_builds_modules() {
    let lib = library();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(repo("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.join("project.toml").is_file())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty());
    let session = std::env::temp_dir().join(format!("crab_ex_blocks_{}", std::process::id()));
    let mut bad = Vec::new();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let project = match Project::load(&dir) {
            Ok(p) => p,
            Err(e) => {
                bad.push(format!("{name}: {e}"));
                continue;
            }
        };
        // The example's own blocks and boards, for this example only.
        let mut lib = lib.clone();
        crab_core::packed::add_project_blocks(&dir, &session, &mut lib);
        let boards = dir.join(crab_core::packed::PACKED_BOARDS);
        if boards.is_dir() {
            let (b, _) = crab_core::boards::Boards::load(&boards);
            lib.set_boards(b);
        }
        if let Err(e) = modules(&project, &lib) {
            bad.push(format!("{name}: {e}"));
        }
    }
    let _ = std::fs::remove_dir_all(&session);
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Every library block must be covered by an example, and thus by `every_example_builds_modules`.
#[test]
fn every_block_has_an_example() {
    let lib = library();
    let used: BTreeSet<String> = std::fs::read_dir(repo("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.join("project.toml").is_file())
        .flat_map(|dir| Project::load(&dir).unwrap().logic.blocks)
        .map(|b| b.kind)
        .collect();
    let missing: Vec<&str> = lib
        .iter()
        .map(|b| b.def.info.kind.as_str())
        .filter(|kind| !used.contains(*kind))
        .collect();
    assert!(
        missing.is_empty(),
        "блоки без примера в examples/: {missing:?}"
    );
}

#[test]
fn schema_errors_stop_generation() {
    let mut project = Project::load(repo("examples/pump")).unwrap();
    project.logic.links.pop();
    let error = modules(&project, &library()).unwrap_err();
    assert!(error.contains("pump"), "{error}");
}
