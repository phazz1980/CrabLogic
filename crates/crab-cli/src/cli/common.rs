use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crab_core::{Diagnostic, Library, Project, Severity};

/// Exit code when the schema has errors.
pub(crate) const EXIT_SCHEMA_ERRORS: u8 = 1;
/// Exit code when the project or library cannot be loaded.
pub(crate) const EXIT_LOAD_FAILED: u8 = 2;
/// Exit code when module generation fails for another reason (a block the generator refuses, files).
pub(crate) const EXIT_GEN_FAILED: u8 = 3;
/// Exit code when an upload to the board fails.
pub(crate) const EXIT_BUILD_FAILED: u8 = 4;

/// Session folder of the blocks a project brings (see `load_library`).
pub(crate) fn session_blocks_dir() -> PathBuf {
    std::env::temp_dir()
        .join("crablogic_cli_blocks")
        .join(std::process::id().to_string())
}

pub(crate) fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        match d.severity {
            Severity::Error => eprintln!("{d}"),
            Severity::Warning => println!("{d}"),
        }
    }
}

/// Loads the project and the block library, printing errors.
pub(crate) fn load(
    project_dir: &Path,
    library: Option<&Path>,
) -> Result<(Project, Library), ExitCode> {
    let project = Project::load(project_dir).map_err(|e| {
        eprintln!("ошибка: {e}");
        ExitCode::from(EXIT_LOAD_FAILED)
    })?;
    let lib = load_library(project_dir, library).ok_or(ExitCode::from(EXIT_LOAD_FAILED))?;
    Ok((project, lib))
}

/// Loads the block library; library problems are printed as warnings.
pub(crate) fn load_library(project_dir: &Path, explicit: Option<&Path>) -> Option<Library> {
    let Some(dir) = explicit
        .map(Path::to_path_buf)
        .or_else(|| find_library(project_dir))
    else {
        eprintln!(
            "ошибка: не найдена папка библиотеки блоков `library/`; укажите её через --library"
        );
        return None;
    };
    // A board packed into the project file but missing here is installed (the library has to
    // know it); the project's own blocks are only loaded for this run, the user library keeps
    // what it had.
    if crab_core::project::is_project_file(project_dir) || project_dir.is_dir() {
        if let Ok((lib, _)) = Library::load_with_user(&dir) {
            let user = crab_core::user_library_dir(&dir);
            let boards = crab_core::packed::unpack_missing_boards(project_dir, &user, &lib);
            if !boards.is_empty() {
                println!("из проекта добавлена плата: {}", boards.join(", "));
            }
        }
    }
    match Library::load_with_user(&dir) {
        Ok((mut lib, mut errors)) => {
            let (kinds, more) =
                crab_core::packed::add_project_blocks(project_dir, &session_blocks_dir(), &mut lib);
            errors.extend(more);
            if !kinds.is_empty() {
                println!("блоки проекта: {}", kinds.join(", "));
            }
            for e in errors {
                println!("предупреждение: блок пропущен: {e}");
            }
            Some(lib)
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            None
        }
    }
}

/// Looks for `library/` in the project folder and its parents, then next to the executable.
/// The `library/` of a project folder holds that project's own blocks, not the library.
pub(crate) fn find_library(project_dir: &Path) -> Option<PathBuf> {
    let project_dir = std::path::absolute(project_dir).ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    [project_dir, exe_dir]
        .into_iter()
        .flatten()
        .flat_map(|start| {
            start
                .ancestors()
                .map(|a| a.join("library"))
                .collect::<Vec<_>>()
        })
        .find(|dir| {
            let own = dir
                .parent()
                .is_some_and(|p| p.join(crab_core::project::PROJECT_FILE).is_file());
            dir.is_dir() && !own && crab_core::library::holds_blocks(dir)
        })
}
