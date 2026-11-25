use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crab_core::{Library, Severity};

use super::common::{find_library, EXIT_SCHEMA_ERRORS};
use super::BlockAction;

/// `block check`: one block folder with every problem named and a hint how to fix it.
pub(crate) fn run_block_check(dir: &Path, library: Option<&Path>) -> ExitCode {
    let here = Path::new(".");
    let Some(lib_dir) = library
        .map(Path::to_path_buf)
        .or_else(|| find_library(here))
    else {
        eprintln!(
            "ошибка: не найдена папка библиотеки блоков `library/`; укажите её через --library"
        );
        return ExitCode::FAILURE;
    };
    let lib = match Library::load_with_user(&lib_dir) {
        Ok((lib, _)) => lib,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::FAILURE;
        }
    };
    let check = crab_codegen::check_block(dir, &lib);
    let mut errors = 0;
    for d in &check.diags {
        let word = if d.severity == Severity::Error {
            errors += 1;
            "ошибка"
        } else {
            "предупреждение"
        };
        println!("{word} [{}]: {}", d.code, d.message);
        println!("  → {}", d.hint);
    }
    if let Some(kind) = &check.kind {
        let mut line = format!(
            "Блок «{kind}» ({})",
            if check.driver {
                "драйвер"
            } else {
                "логика"
            }
        );
        if let Some(n) = check.wasm_bytes {
            line.push_str(&format!(": block.wasm {n} байт"));
        }
        if let Some(n) = check.module_bytes {
            line.push_str(&format!(", модуль листа {n} байт"));
        }
        println!("{line}");
    }
    if errors == 0 {
        println!("Ошибок нет");
        ExitCode::SUCCESS
    } else {
        println!("Ошибок — {errors}");
        ExitCode::from(EXIT_SCHEMA_ERRORS)
    }
}

pub(crate) fn run_block(action: BlockAction, library: Option<&Path>) -> ExitCode {
    if let BlockAction::Check { dir } = &action {
        return run_block_check(dir, library);
    }
    use crab_core::pack::{self, PackImport};
    let here = Path::new(".");
    let Some(dir) = library
        .map(Path::to_path_buf)
        .or_else(|| find_library(here))
    else {
        eprintln!(
            "ошибка: не найдена папка библиотеки блоков `library/`; укажите её через --library"
        );
        return ExitCode::FAILURE;
    };
    let lib = match Library::load_with_user(&dir) {
        Ok((lib, _)) => lib,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::FAILURE;
        }
    };
    let user = crab_core::user_library_dir(&dir);
    let result = match action {
        BlockAction::Check { .. } => unreachable!("handled above"),
        BlockAction::List => {
            for (folder, info) in pack::list(&user) {
                println!("{folder} · версия {} · {}", info.version, info.description);
                for block in lib.iter().filter(|b| lib.is_user(&b.def.info.kind)) {
                    if block.dir.starts_with(user.join(&folder)) {
                        println!("  {} — {}", block.def.info.kind, block.def.info.name);
                    }
                }
            }
            // Blocks copied by hand into a folder without `pack.toml` still load: show them too.
            let packed: Vec<PathBuf> = pack::list(&user)
                .into_iter()
                .map(|(folder, _)| user.join(folder))
                .collect();
            let loose: Vec<_> = lib
                .iter()
                .filter(|b| lib.is_user(&b.def.info.kind))
                .filter(|b| b.dir.starts_with(&user) && !packed.iter().any(|p| b.dir.starts_with(p)))
                .collect();
            if !loose.is_empty() {
                println!("без набора (нет pack.toml):");
                for block in loose {
                    println!("  {} — {}", block.def.info.kind, block.def.info.name);
                }
            }
            Ok(())
        }
        BlockAction::Export { pack: name, file } => {
            let file = file.unwrap_or_else(|| PathBuf::from(format!("{name}.{}", pack::EXTENSION)));
            pack::export_pack(&user, &name, &file)
                .map(|()| println!("набор «{name}» сохранён: {}", file.display()))
        }
        BlockAction::Import { file, replace } => pack::read_pack_file(&file).and_then(|inc| {
            if let PackImport::NotNewer(have) = inc.status(&user) {
                if !replace {
                    return Err(format!(
                        "набор «{}» уже есть (версия {have}, в файле {}); добавьте --replace, чтобы заменить",
                        inc.info.name, inc.info.version
                    ));
                }
            }
            inc.install(&user, &lib).map(|_| {
                println!(
                    "набор «{}» (версия {}) добавлен; блоков: {}",
                    inc.info.name,
                    inc.info.version,
                    inc.kinds().len()
                )
            })
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ошибка: {e}");
            ExitCode::FAILURE
        }
    }
}
