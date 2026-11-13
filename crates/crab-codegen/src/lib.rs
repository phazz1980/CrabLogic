//! Code generation: a checked schema becomes a [`Program`], and every sheet of it a WASM module
//! (`wasm`) for the Rust master firmware.

pub mod block_check;
pub mod debug;
pub mod error;
pub mod retain;
pub mod rust_master;
pub mod sheet;
pub mod stats;
pub mod wasm;

use crab_core::{BlockDef, Library, Project};

pub use block_check::{check_block, BlockCheck, BlockDiag};
pub use error::GenError;
pub use program::{Input, Instance, Program, Signal, Source};
pub use rust_master::{
    block_has_cpp, check_project, has_cpp_code, rust_master_diagnostics, CPP_BLOCK_HINT,
};
pub use sheet::{LocalVar, SheetError, SheetModule, Tag, TagKind, TagUse};
pub use wasm::{WasmError, WasmModule, MASTER_CAPS, MAX_BLOCK_CONFIG};

pub mod program;

/// Id of the sample instance of a block (the block editor and the canvas icons show it).
pub fn preview_id(def: &BlockDef) -> String {
    format!("{}_1", def.info.kind)
}

/// The project as the generators take it: disabled links dropped, and an unused output
/// (it waits in the project tree with nothing to write) left out. An unused input is
/// harmless and stays.
pub fn prepare(project: &Project, lib: &Library) -> Project {
    let mut project = project.without_disabled();
    for id in project.unused_tags(lib) {
        let output = project
            .logic
            .block(&id)
            .and_then(|b| lib.resolve(&project.logic, b))
            .is_some_and(|l| l.def.outputs.is_empty());
        if output {
            project.remove_block(&id).expect("listed by unused_tags");
        }
    }
    project
}
