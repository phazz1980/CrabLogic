use std::path::Path;
use std::process::ExitCode;

use crab_codegen::{GenError, Program};

use super::common::{
    load, print_diagnostics, EXIT_BUILD_FAILED, EXIT_GEN_FAILED, EXIT_SCHEMA_ERRORS,
};

/// Reads the project out of the board's flash and saves it as `output`.
/// `pull` read the project copy from the old debug firmware (binary protocol); the Rust master
/// keeps no copy of the project, so there is nothing to read.
pub(crate) fn run_pull(
    _output: &Path,
    _port: &str,
    _password: &str,
    _force: bool,
    _library: Option<&Path>,
) -> ExitCode {
    eprintln!(
        "ошибка: чтение проекта с платы недоступно: мастер-прошивка не хранит копию проекта          (это умела только удалённая отладочная прошивка на C++)"
    );
    ExitCode::FAILURE
}

/// Prints live values every half second until Ctrl+C.
pub(crate) fn run_monitor(
    project_dir: &Path,
    library: Option<&Path>,
    port: Option<String>,
    host: Option<String>,
) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    // The same project the upload builds (unused outputs dropped), or the module CRCs differ.
    let project = crab_codegen::prepare(&project, &lib);
    let mut program = match Program::build(&project, &lib) {
        Ok(p) => p,
        Err(GenError::Schema(d)) => {
            print_diagnostics(&d);
            return ExitCode::from(EXIT_SCHEMA_ERRORS);
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::from(EXIT_GEN_FAILED);
        }
    };
    // The modules the monitor expects are the ones a debug build uploads: joined the same way.
    if let Err(e) = program.auto_group_for_board(true) {
        eprintln!("ошибка: склейка листов: {e}");
        return ExitCode::from(EXIT_GEN_FAILED);
    }
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    // An explicit --port or --host wins; otherwise the project and the ports present decide.
    let link = match (port, host) {
        (Some(_), Some(_)) => return failed("укажите что-то одно: --port или --host".into()),
        (Some(port), None) => crab_build::Chosen::Serial(port),
        (None, Some(host)) => crab_build::Chosen::Net(host),
        (None, None) => match crab_build::project_link(&project.config.board) {
            Ok(link) => link,
            Err(message) => return failed(message),
        },
    };
    let password = &project.config.project.password;
    let (connector, shown) = match &link {
        crab_build::Chosen::Serial(port) => (crab_debug::transport::serial(port), port.clone()),
        crab_build::Chosen::Net(host) => {
            if password.is_empty() {
                return failed(
                    "для связи по сети нужен пароль проекта ([project] password) и ключ на плате (crablogic netkey)"
                        .into(),
                );
            }
            (
                crab_debug::transport::tcp(host, password),
                format!("{host} (по сети)"),
            )
        }
    };
    let table = crab_codegen::debug::debug_table(&program);
    let modules = match crab_build::debug_modules(&program, &table, &project.work_dir()) {
        Ok(modules) => modules,
        Err(message) => return failed(message),
    };
    let names: Vec<String> = table
        .entries
        .iter()
        .map(|e| match &e.source {
            crab_debug::Source::Output(p) => p.to_string(),
            crab_debug::Source::Var(v) => format!("${v}"),
            crab_debug::Source::Array { name, index } => format!("{name}[{index}]"),
        })
        .collect();
    let session = crab_debug::MasterSession::start(table, modules, connector, || {});
    let port = shown;
    let mine = project.content_hash().ok();
    let mut reported = None;
    println!("Монитор {port}, Ctrl+C — выход");
    let mut printed_console = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let live = session.snapshot();
        for line in live.console_after(printed_console) {
            println!("> {line}");
        }
        printed_console = live.console_total;
        let values: Vec<String> = names
            .iter()
            .zip(&live.values)
            .zip(&live.forced)
            .map(|((name, v), f)| {
                let v = v.map_or("—".to_owned(), |v| v.to_string());
                let mark = if f.is_some() { " (F)" } else { "" };
                format!("{name}={v}{mark}")
            })
            .collect();
        if live.status.is_empty() {
            println!("{}", values.join("  "));
        } else {
            println!("[{}] {}", live.status, values.join("  "));
        }
        if live.schema_matches == Some(false) {
            println!("  модули на плате не совпадают с этим проектом: значений нет (залейте модули с --watch)");
        }
        if live.project_hash != reported {
            reported = live.project_hash;
            if let Some(on_board) = reported {
                if Some(on_board) == mine {
                    println!("  проект совпадает с платой");
                } else {
                    println!(
                        "  проект на плате отличается от этого файла (хеш {})",
                        crab_core::hash_hex(&on_board)
                    );
                }
            }
        }
        if let Some(st) = &live.stats {
            println!(
                "  цикл: логика {} мкс (макс {}, средн. {}), период макс {} мкс, сканов {}, куча {} Б",
                st.last_us, st.max_us, st.avg_us, st.period_max_us, st.cycles, st.free_heap
            );
        }
    }
}
