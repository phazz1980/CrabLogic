//! `crablogic report`: the bug report ZIP without the editor window (`specs/bug-report`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crab_core::report::{self, BoardData, Report};

pub struct ReportArgs {
    pub port: Option<String>,
    pub project: Option<PathBuf>,
    pub describe: Option<String>,
    pub out: Option<PathBuf>,
    pub erase_core: bool,
}

pub fn run_report(args: ReportArgs, library: Option<&Path>) -> ExitCode {
    let now = chrono::Local::now();
    let mut problems = Vec::new();
    let mut project_name = None;
    let mut board_name = None;
    let mut project_bytes = None;
    if let Some(path) = &args.project {
        match crate::load(path, library) {
            Ok((mut project, lib)) => {
                project_name = Some(project.config.project.name.clone());
                board_name = Some(project.config.board.fqbn.clone());
                report::mask_secrets(&mut project, &lib);
                match project.archive_bytes(&[]) {
                    Ok(bytes) => project_bytes = Some(bytes),
                    Err(e) => problems.push(format!("не удалось упаковать проект: {e}")),
                }
            }
            Err(code) => return code,
        }
    }
    let mut board = None;
    let mut master_version = None;
    if let Some(port) = &args.port {
        let dump = crab_debug::board_report::collect_from_port(port);
        problems.extend(dump.problems);
        master_version = report::master_version(&dump.text);
        board = Some(BoardData {
            port: port.clone(),
            text: dump.text,
            coredump: dump.coredump,
        });
    }
    let dump_read = board.as_ref().is_some_and(|b| b.coredump.is_some());
    let r = Report {
        description: args.describe.unwrap_or_default(),
        app: format!("crablogic cli {}", env!("CARGO_PKG_VERSION")),
        created: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        project_name,
        board_name,
        master_version,
        recent: Vec::new(),
        editor_log: None,
        panic: None,
        board,
        project: project_bytes,
        problems,
    };
    let out = args.out.unwrap_or_else(|| {
        report::report_file_name(&now.format("%Y-%m-%d-%H%M%S").to_string()).into()
    });
    if let Err(e) = report::write_report(&r, &out) {
        eprintln!("ошибка: {e}");
        return ExitCode::from(crate::EXIT_GEN_FAILED);
    }
    println!("отчёт сохранён: {}", out.display());
    for p in &r.problems {
        println!("не удалось: {p}");
    }
    if args.erase_core && dump_read {
        let port = args.port.as_deref().unwrap_or_default();
        match serialport::new(port, 115_200)
            .timeout(std::time::Duration::from_millis(50))
            .open()
        {
            Ok(mut p) => match crab_debug::board_report::erase_core(&mut p) {
                Ok(()) => println!("дамп сбоя на плате стёрт"),
                Err(e) => eprintln!("ошибка: не удалось стереть дамп: {e}"),
            },
            Err(e) => eprintln!("ошибка: не удалось открыть порт {port}: {e}"),
        }
    } else if args.erase_core {
        println!("дамп сбоя не был прочитан, стирать нечего");
    }
    ExitCode::SUCCESS
}
