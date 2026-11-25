use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crab_core::Project;

use super::common::{
    load, load_library, print_diagnostics, EXIT_BUILD_FAILED, EXIT_GEN_FAILED, EXIT_LOAD_FAILED,
    EXIT_SCHEMA_ERRORS,
};

pub(crate) fn run_convert(dir: &Path, library: Option<&Path>) -> ExitCode {
    let result = Project::load(dir).and_then(|p| {
        if p.path.exists() {
            println!("Проект уже в одном файле: {}", p.path.display());
            return Ok(());
        }
        // Without the library the archive is written without the PDF of sheets.
        let warning = match load_library(dir, library) {
            Some(lib) => crab_pdf::save_project(&p, &lib)?,
            None => p.save().map(|()| None)?,
        };
        println!("Записан {}", p.path.display());
        if let Some(w) = warning {
            println!("предупреждение: {w}");
        }
        Ok(())
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ошибка: {e}");
            ExitCode::from(EXIT_LOAD_FAILED)
        }
    }
}

/// `crablogic panel`: the electrical documentation of the panel as a PDF.
pub(crate) fn run_panel(
    project_dir: &Path,
    library: Option<&Path>,
    lang: &str,
    format: &str,
    out: Option<&Path>,
    save_numbers: bool,
    csv: Option<&Path>,
) -> ExitCode {
    let Some(format) = crab_schematic::Format::from_code(format) else {
        eprintln!("ошибка: формат листов — a3 или a4, а не «{format}»");
        return ExitCode::from(EXIT_LOAD_FAILED);
    };
    let Some(lang) = crab_schematic::Lang::from_code(lang) else {
        eprintln!("ошибка: язык подписей — ru или en, а не «{lang}»");
        return ExitCode::from(EXIT_LOAD_FAILED);
    };
    let (mut project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    if project.panel.is_empty() {
        eprintln!("ошибка: в проекте нет щита (panel.toml)");
        return ExitCode::from(EXIT_SCHEMA_ERRORS);
    }
    let report = crab_core::check_panel(&project, &lib);
    print_diagnostics(&report.diagnostics);
    let errors = report.errors().count();
    if errors > 0 {
        eprintln!(
            "Щит {}: ошибок — {errors}, документация не собрана",
            project_dir.display()
        );
        return ExitCode::from(EXIT_SCHEMA_ERRORS);
    }
    let (doc, nets) = match crab_schematic::build_document_in(&project, &lib, lang, format) {
        Ok(built) => built,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::from(EXIT_GEN_FAILED);
        }
    };
    if doc.sheets.is_empty() {
        eprintln!("ошибка: в щите нет силовых цепей, рисовать нечего");
        return ExitCode::from(EXIT_SCHEMA_ERRORS);
    }
    let pdf = match crab_pdf::compile(crab_schematic::doc::typst_source(&doc, lang)) {
        Ok(pdf) => pdf,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::from(EXIT_BUILD_FAILED);
        }
    };
    let path = out.map_or_else(
        || PathBuf::from(format!("{}_panel.pdf", project.display_name())),
        Path::to_path_buf,
    );
    if let Err(e) = std::fs::write(&path, pdf) {
        eprintln!("ошибка: не записан {}: {e}", path.display());
        return ExitCode::from(EXIT_BUILD_FAILED);
    }
    println!(
        "Записан {}: листов — {}, проводов — {}",
        path.display(),
        doc.sheets.len(),
        nets.wires.len()
    );
    if let Some(dir) = csv {
        let stem = project.display_name();
        let tables = &doc.tables;
        let files = [
            ("terminals", &tables.terminals),
            ("wires", &tables.wires),
            ("bom", &tables.bom),
        ];
        let written = std::fs::create_dir_all(dir).and_then(|()| {
            for (name, table) in files {
                if table.rows.is_empty() {
                    continue;
                }
                let path = dir.join(format!("{stem}_{name}.csv"));
                std::fs::write(&path, crab_schematic::tables::to_csv(table))?;
                println!("Записан {}", path.display());
            }
            Ok(())
        });
        if let Err(e) = written {
            eprintln!("ошибка: таблицы CSV не записаны: {e}");
            return ExitCode::from(EXIT_BUILD_FAILED);
        }
    }
    if save_numbers {
        nets.store(&mut project.panel);
        match crab_pdf::save_project(&project, &lib) {
            Ok(None) => println!("Номера проводов записаны в проект"),
            Ok(Some(warning)) => {
                println!("Номера проводов записаны в проект");
                println!("предупреждение: {warning}");
            }
            Err(e) => {
                eprintln!("ошибка: номера не записаны: {e}");
                return ExitCode::from(EXIT_BUILD_FAILED);
            }
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn run_check(project_dir: &Path, library: Option<&Path>) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };

    let report = crab_codegen::check_project(&project, &lib);
    print_diagnostics(&report.diagnostics);
    // The panel is checked apart from the schema: its mistakes never stop a build, but they
    // are reported here so the documentation can be trusted.
    let panel = crab_core::check_panel(&project, &lib);
    print_diagnostics(&panel.diagnostics);

    let errors = report.errors().count() + panel.errors().count();
    if errors > 0 {
        eprintln!("Проверка {}: ошибок — {errors}", project_dir.display());
        return ExitCode::from(EXIT_SCHEMA_ERRORS);
    }
    let order: Vec<&str> = report
        .order
        .iter()
        .map(|&i| project.logic.blocks[i].id.as_str())
        .collect();
    println!("Проверка {}: ошибок нет", project_dir.display());
    println!("Порядок выполнения: {}", order.join(" → "));
    if !report.feedback.is_empty() {
        let links: Vec<String> = report
            .feedback
            .iter()
            .map(|&i| {
                let l = &project.logic.links[i];
                format!("{} → {}", l.from, l.to)
            })
            .collect();
        println!(
            "Обратные связи (значение с прошлого цикла): {}",
            links.join(", ")
        );
    }
    ExitCode::SUCCESS
}
