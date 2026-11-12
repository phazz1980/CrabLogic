//! Check problems only the module generator sees (`specs/rust-only`): a block it refuses does not
//! run on the Rust master; a block with C++ code is not supported any more. The editor, the CLI and
//! the AI add these to the schema check of `crab-core` (which does not know the generator).

use crab_core::library::ESP32_CPP_TEMPLATE;
use crab_core::{DiagKind, Diagnostic, Library, Project, Severity};

use crate::{Program, WasmError};

/// Where the C++ code of blocks is to be rewritten: the instructions next to the program.
pub const CPP_BLOCK_HINT: &str =
    "блок на C++ больше не поддерживается: перепишите его как block.wasm \
     (инструкция и примеры — папка skills рядом с программой, справка «Драйверы железа на WASM»)";

/// The schema check of `crab-core` plus, when it finds no errors, [`rust_master_diagnostics`]: what
/// the editor, the CLI and the AI show.
pub fn check_project(project: &Project, lib: &Library) -> crab_core::Report {
    let mut report = crab_core::check_project(project, lib);
    if !report.has_errors() {
        report
            .diagnostics
            .extend(rust_master_diagnostics(project, lib));
    }
    report
}

/// Whether a library block carries C++ code of its own (and no `block.wasm` that would replace it).
pub fn block_has_cpp(b: &crab_core::LibraryBlock) -> bool {
    let cpp = b.inline_templates.contains_key(ESP32_CPP_TEMPLATE)
        || b.template_path(ESP32_CPP_TEMPLATE).is_some()
        || matches!(b.native_part(), Ok(Some(_)));
    cpp && b.wasm_path().is_none()
}

/// Whether the library block of `kind` carries C++ code of its own.
pub fn has_cpp_code(lib: &Library, kind: &str) -> bool {
    lib.get(kind).is_some_and(block_has_cpp)
}

/// Errors for the blocks of `project` the Rust master cannot run. Empty while the schema itself has
/// errors (the generator needs a valid schema; the check of `crab-core` reports those). Per sheet
/// only the first refused block is found.
pub fn rust_master_diagnostics(project: &Project, lib: &Library) -> Vec<Diagnostic> {
    let project = crate::prepare(project, lib);
    let Ok(program) = Program::build(&project, lib) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sheet in program.sheets() {
        let Err(WasmError::Unsupported {
            block,
            kind,
            reason,
        }) = program.wasm_module(sheet)
        else {
            continue;
        };
        let (diag_kind, message) = if !kind.is_empty() && has_cpp_code(lib, &kind) {
            (
                DiagKind::CppBlock,
                format!("блок «{block}» ({kind}): {CPP_BLOCK_HINT}"),
            )
        } else if block.is_empty() {
            (
                DiagKind::RustMaster,
                format!("лист {sheet} не работает на мастере на Rust: {reason}"),
            )
        } else {
            (
                DiagKind::RustMaster,
                format!("блок «{block}» ({kind}) не работает на мастере на Rust: {reason}"),
            )
        };
        out.push(Diagnostic {
            kind: diag_kind,
            severity: Severity::Error,
            block: (!block.is_empty()).then_some(block),
            link: None,
            message,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn setup(name: &str, block_files: &[(&str, &str)], logic: &str) -> (PathBuf, Library, Project) {
        let root = std::env::temp_dir().join(format!("crab_rm_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (file, text) in block_files {
            write(&root.join("lib").join(file), text);
        }
        let (lib, errors) = Library::load(root.join("lib")).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let dir = root.join("project");
        write(
            &dir.join("project.toml"),
            "[project]\nname = \"rm\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
        );
        write(&dir.join("logic.toml"), logic);
        let project = Project::load(&dir).unwrap();
        (root, lib, project)
    }

    const CPP_BLOCK: &str = "[block]\ntype = \"old\"\nname = \"Старый\"\ngroup = \"Тест\"\n\n\
        [[output]]\nname = \"out\"\ntype = \"float\"\n";

    #[test]
    fn a_cpp_block_is_refused_with_the_skills_hint() {
        let (root, lib, project) = setup(
            "cpp",
            &[
                ("old/block.toml", CPP_BLOCK),
                ("old/esp32.cpp.tmpl", "{{ out.out }} = 1.0;\n"),
            ],
            "[[block]]\nid = \"x\"\ntype = \"old\"\n",
        );
        let diags = rust_master_diagnostics(&project, &lib);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].kind, DiagKind::CppBlock);
        assert_eq!(diags[0].block.as_deref(), Some("x"));
        assert!(diags[0].message.contains("skills"), "{}", diags[0].message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_schema_the_master_runs_has_no_problems() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (lib, _) = Library::load(dir.join("library")).unwrap();
        let project = Project::load(dir.join("examples/pump")).unwrap();
        assert!(rust_master_diagnostics(&project, &lib).is_empty());
    }
}
