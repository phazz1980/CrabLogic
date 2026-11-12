//! `crablogic audit-rust` (temporary, `specs/rust-only` stage A): what of the library and the examples
//! does not go to the Rust master yet. A block is tried alone in a one-block project with its
//! defaults; an example with all its sheets. Modules that build are checked against `MODCAPS` of the
//! master images.

use std::path::{Path, PathBuf};

use crab_codegen::{GenError, Program};
use crab_core::{Library, Project, Severity};

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Block type or example folder name.
    pub name: String,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The module builds and the master has everything it needs.
    Ok,
    /// The module builds but needs what the master does not report.
    MissingCaps(Vec<String>),
    /// The module generator refuses (the reason as it says it).
    Unsupported(String),
    /// The block alone is not a valid schema (a parameter without a default and the like): not a
    /// verdict about the master.
    Schema(String),
}

impl Row {
    pub fn line(&self) -> String {
        match &self.verdict {
            Verdict::Ok => format!("ok      {}", self.name),
            Verdict::MissingCaps(caps) => {
                format!("caps    {}: нет у мастера: {}", self.name, caps.join(", "))
            }
            Verdict::Unsupported(why) => format!("нет     {}: {why}", self.name),
            Verdict::Schema(why) => format!("схема   {}: {why}", self.name),
        }
    }
}

/// Verdict for a loaded project: every sheet as a module.
pub fn audit_project(project: &Project, lib: &Library, caps: &[String]) -> Verdict {
    let project = crab_codegen::prepare(project, lib);
    let program = match Program::build(&project, lib) {
        Ok(p) => p,
        Err(GenError::Schema(diags)) => {
            let first = diags
                .iter()
                .find(|d| d.severity == Severity::Error)
                .map_or_else(|| "ошибка схемы".to_owned(), |d| d.message.clone());
            return Verdict::Schema(first);
        }
        Err(e) => return Verdict::Schema(e.to_string()),
    };
    let mut missing: Vec<String> = Vec::new();
    for sheet in program.sheets() {
        match program.wasm_module(sheet) {
            Ok(module) => {
                for need in module.needs {
                    if !caps.contains(&need) && !missing.contains(&need) {
                        missing.push(need);
                    }
                }
            }
            Err(e) => return Verdict::Unsupported(e.to_string()),
        }
    }
    if missing.is_empty() {
        Verdict::Ok
    } else {
        Verdict::MissingCaps(missing)
    }
}

/// Every block type of the library, alone.
pub fn audit_blocks(lib: &Library, caps: &[String]) -> Vec<Row> {
    let root = std::env::temp_dir().join(format!("crab_audit_{}", std::process::id()));
    let mut blocks: Vec<_> = lib.iter().collect();
    blocks.sort_by(|a, b| a.def.info.kind.cmp(&b.def.info.kind));
    let rows = blocks
        .into_iter()
        .map(|block| {
            let kind = block.def.info.kind.clone();
            let verdict = match crab_codegen::block_check::one_block_project(&root, &block.def)
                .map_err(|e| e.to_string())
                .and_then(|dir| Project::load(&dir).map_err(|e| e.to_string()))
            {
                Ok(project) => audit_project(&project, lib, caps),
                Err(e) => Verdict::Schema(e),
            };
            Row {
                name: kind,
                verdict,
            }
        })
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    rows
}

/// Every example folder (a folder with `project.toml`) under `examples`.
pub fn audit_examples(examples: &Path, lib: &Library, caps: &[String]) -> Vec<Row> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(examples)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    dirs.retain(|d| d.join("project.toml").is_file());
    dirs.sort();
    dirs.into_iter()
        .map(|dir| {
            let name = dir
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let verdict = match Project::load(&dir) {
                Ok(project) => {
                    // The example's own blocks and boards, for this example only.
                    let mut lib = lib.clone();
                    let session = std::env::temp_dir()
                        .join(format!("crab_audit_blocks_{}", std::process::id()));
                    crab_core::packed::add_project_blocks(&dir, &session, &mut lib);
                    let boards = dir.join(crab_core::packed::PACKED_BOARDS);
                    if boards.is_dir() {
                        let (b, _) = crab_core::boards::Boards::load(&boards);
                        lib.set_boards(b);
                    }
                    let verdict = audit_project(&project, &lib, caps);
                    let _ = std::fs::remove_dir_all(&session);
                    verdict
                }
                Err(e) => Verdict::Schema(e.to_string()),
            };
            Row { name, verdict }
        })
        .collect()
}

/// `MODCAPS` of the Rust master: from `info.toml` of the ESP32 image, else every capability the
/// module generator knows (the master source has them all).
pub fn master_caps() -> Vec<String> {
    let from_image = crab_build::flash::firmware_dir()
        .and_then(|dir| crab_build::flash::find_image(&dir, "esp32:esp32:esp32").ok())
        .map(|image| image.info.modcaps)
        .filter(|caps| !caps.is_empty());
    match from_image {
        Some(caps) => caps.split(',').map(|c| c.trim().to_owned()).collect(),
        None => crab_codegen::MASTER_CAPS
            .iter()
            .map(|c| (*c).to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn lib() -> Library {
        let (lib, errors) = Library::load(repo("library")).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        lib
    }

    fn caps() -> Vec<String> {
        crab_codegen::MASTER_CAPS
            .iter()
            .map(|c| (*c).to_owned())
            .collect()
    }

    #[test]
    fn a_plain_example_goes_to_the_rust_master() {
        let project = Project::load(repo("examples/pump")).unwrap();
        assert_eq!(audit_project(&project, &lib(), &caps()), Verdict::Ok);
    }

    #[test]
    fn missing_master_features_are_named() {
        let project = Project::load(repo("examples/modbus_rtu")).unwrap();
        let without_modbus: Vec<String> = caps().into_iter().filter(|c| c != "modbus").collect();
        assert_eq!(
            audit_project(&project, &lib(), &without_modbus),
            Verdict::MissingCaps(vec!["modbus".into()])
        );
    }

    #[test]
    fn every_library_block_goes_to_the_rust_master() {
        let rows = audit_blocks(&lib(), &caps());
        let bad: Vec<String> = rows
            .iter()
            .filter(|r| r.verdict != Verdict::Ok)
            .map(Row::line)
            .collect();
        assert!(bad.is_empty(), "{bad:#?}");
    }
}
