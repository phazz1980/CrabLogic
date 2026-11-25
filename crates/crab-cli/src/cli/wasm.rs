use std::path::Path;
use std::process::ExitCode;

use crab_build::upload_plan::{self, SheetModule, StopReport, UploadReport};
use crab_codegen::{GenError, Program};
use crab_core::logic::SheetSpeed;
use crab_core::Project;

use super::common::{
    load, print_diagnostics, EXIT_BUILD_FAILED, EXIT_GEN_FAILED, EXIT_SCHEMA_ERRORS,
};
use super::link::{explicit_link, open_link, project_link_spec, LinkSpec};

/// Strictness flags of `gen-wasm --upload`.
#[derive(Clone, Copy)]
pub(crate) struct UploadFlags {
    pub(crate) require_aot: bool,
    pub(crate) allow_stop: bool,
}

/// Uploads one `.wasm` module to the master firmware over serial (as an AOT image if the master runs AOT).
pub(crate) fn run_upload_wasm(
    file: &Path,
    name: Option<String>,
    allow: Option<Vec<String>>,
    port: Option<String>,
    host: Option<String>,
    project: Option<&Path>,
) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let link = match explicit_link(port, host, project) {
        Ok(link) => link,
        Err(message) => return failed(message),
    };
    let wasm = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) => return failed(format!("не прочитан {}: {e}", file.display())),
    };
    let manifest_path = file.with_file_name("module.toml");
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => match crab_sim::mcu::Manifest::parse(&text) {
            Ok(manifest) => Some(manifest),
            Err(e) => return failed(e),
        },
        Err(_) => None,
    };
    let name = name
        .or_else(|| {
            manifest
                .as_ref()
                .map(|m| m.name.clone())
                .filter(|n| !n.is_empty())
        })
        .or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let allow = allow.unwrap_or_else(|| manifest.map(|m| m.allow).unwrap_or_default());
    let cache = file.parent().unwrap_or(Path::new("."));
    push_module(&name, &wasm, &allow, &[], link, cache, None)
}

/// Sends one module to the master firmware over serial: as an AOT image when the master runs AOT
/// (built into `cache`, copied to `aot_copy` if given), otherwise as `.wasm` with a warning.
fn push_module(
    name: &str,
    wasm: &[u8],
    allow: &[String],
    needs: &[String],
    link: LinkSpec,
    cache: &Path,
    aot_copy: Option<&Path>,
) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let (mut serial, port) = match open_link(link) {
        Ok(opened) => opened,
        Err(message) => return failed(message),
    };
    let info = match crab_build::mcu_module::wait_for_master_info(
        &mut serial,
        std::time::Duration::from_secs(8),
    ) {
        Ok(info) => info,
        Err(e) => return failed(e.to_string()),
    };
    let missing = info.missing_caps(needs);
    if !missing.is_empty() {
        return failed(crab_build::mcu_module::missing_caps_message(name, &missing));
    }
    if info.keeps_modules_in_ram() {
        eprintln!(
            "предупреждение: {}",
            crab_build::mcu_module::OLD_STORAGE_MESSAGE
        );
    }
    // No cancelling from the command line: Ctrl+C ends the process.
    let never_cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut shown = 0;
    let image = match crab_build::mcu_module::module_image(&info, wasm, cache, &mut |target| {
        crab_build::wamrc::ensure_for(
            target,
            None,
            &never_cancelled,
            &mut |line| println!("{line}"),
            &mut |got, total| {
                let percent = (got * 100).checked_div(total).unwrap_or(0);
                if percent >= shown + 10 {
                    shown = percent / 10 * 10;
                    println!("  скачано {shown}%");
                }
            },
        )
    }) {
        Ok(image) => image,
        Err(e) => return failed(e.to_string()),
    };
    if let Some(warning) = &image.warning {
        eprintln!("предупреждение: {warning}");
    }
    let started = std::time::Instant::now();
    let uploaded = if image.aot {
        println!(
            "Модуль «{name}»: AOT {} байт (WASM {} байт) -> {port}...",
            image.bytes.len(),
            wasm.len()
        );
        if let Some(path) = aot_copy {
            if let Err(e) = std::fs::write(path, &image.bytes) {
                return failed(format!("не записан {}: {e}", path.display()));
            }
        }
        match crab_build::mcu_module::module_memory(wasm) {
            Some(mem) => {
                crab_build::mcu_module::upload_image(&mut serial, name, &image.bytes, mem, allow)
            }
            None => Err(crab_build::mcu_module::ModuleUploadError::OldModule),
        }
    } else {
        println!("Модуль «{name}» ({} байт) -> {port}...", wasm.len());
        crab_build::mcu_module::upload_module(&mut serial, name, wasm, allow)
    };
    match uploaded {
        Ok(done) => {
            match done.swap_us {
                Some(us) => println!(
                    "Модуль «{name}» залит и запущен за {:.1} с (замена {} мс)",
                    started.elapsed().as_secs_f32(),
                    us / 1000
                ),
                None => println!(
                    "Модуль «{name}» залит и запущен за {:.1} с",
                    started.elapsed().as_secs_f32()
                ),
            }
            ExitCode::SUCCESS
        }
        Err(e) => failed(e.to_string()),
    }
}

/// What to do with the upload report: an error with `--require-aot`, a warning text otherwise.
pub(crate) fn report_verdict(
    report: Option<&UploadReport>,
    require_aot: bool,
) -> Result<Option<String>, String> {
    match report {
        None => Ok(None),
        Some(report) if require_aot => Err(report.message()),
        Some(report) => Ok(Some(report.message())),
    }
}

/// What to do with the stop report: an error without `--allow-stop` (with a hint), a warning text
/// with it.
pub(crate) fn stop_verdict(
    stops: Option<&StopReport>,
    allow_stop: bool,
) -> Result<Option<String>, String> {
    match stops {
        None => Ok(None),
        Some(stops) if allow_stop => Ok(Some(stops.message())),
        Some(stops) => Err(format!(
            "{}
Запустите с --allow-stop, чтобы остановить листы на время заливки.",
            stops.message()
        )),
    }
}

/// Sends all the sheet modules over one opened port: the placement (AOT or plain) is decided for
/// all of them together. `numbers` are the sheet numbers of `modules`; the AOT image of a module
/// is also written next to its `.wasm`. `remove_stale`: modules on the board that are not among
/// `modules` are removed (the whole project is uploaded).
fn upload_sheets(
    project: &Project,
    modules: &[crab_codegen::WasmModule],
    numbers: &[u32],
    allows: &[Vec<String>],
    link: LinkSpec,
    flags: UploadFlags,
    remove_stale: bool,
) -> ExitCode {
    let UploadFlags {
        require_aot,
        allow_stop,
    } = flags;
    use crab_build::mcu_module as mm;
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let (mut serial, port) = match open_link(link) {
        Ok(opened) => opened,
        Err(message) => return failed(message),
    };
    let info = match mm::wait_for_master_info(&mut serial, std::time::Duration::from_secs(8)) {
        Ok(info) => info,
        Err(e) => return failed(e.to_string()),
    };
    for module in modules {
        let missing = info.missing_caps(&module.needs);
        if !missing.is_empty() {
            return failed(mm::missing_caps_message(&module.name, &missing));
        }
    }
    if info.keeps_modules_in_ram() {
        eprintln!("предупреждение: {}", mm::OLD_STORAGE_MESSAGE);
    }
    let space = if info.takes_aot() {
        match upload_plan::read_crabmod_space(&mut serial) {
            Ok(space) => space,
            Err(e) => return failed(e.to_string()),
        }
    } else {
        None
    };
    let sheet_modules: Vec<SheetModule> = modules
        .iter()
        .zip(numbers)
        .map(|(m, &sheet)| SheetModule {
            sheet,
            name: &m.name,
            wasm: &m.wasm,
            wasm_aot: &m.wasm_aot,
            speed: project
                .logic
                .sheets
                .iter()
                .find(|s| s.number == sheet)
                .map_or(SheetSpeed::Normal, |s| s.speed),
        })
        .collect();
    let cache = project.work_dir().join("aot");
    // No cancelling from the command line: Ctrl+C ends the process.
    let never_cancelled = std::sync::atomic::AtomicBool::new(false);
    let mut shown = 0;
    let planned = upload_plan::plan_upload(
        &info,
        space.as_ref(),
        &sheet_modules,
        &cache,
        &mut |target| {
            crab_build::wamrc::ensure_for(
                target,
                None,
                &never_cancelled,
                &mut |line| println!("{line}"),
                &mut |got, total| {
                    let percent = (got * 100).checked_div(total).unwrap_or(0);
                    if percent >= shown + 10 {
                        shown = percent / 10 * 10;
                        println!("  скачано {shown}%");
                    }
                },
            )
        },
    );
    let plan = match planned {
        Ok(plan) => plan,
        Err(e) => return failed(e.to_string()),
    };
    match report_verdict(plan.report.as_ref(), require_aot) {
        Ok(Some(text)) => eprintln!("предупреждение: {text}"),
        Ok(None) => {}
        Err(text) => return failed(text),
    }
    match stop_verdict(plan.stops.as_ref(), allow_stop) {
        Ok(Some(text)) => eprintln!("предупреждение: {text}"),
        Ok(None) => {}
        Err(text) => return failed(text),
    }
    // Modules of an earlier build that this one no longer has (a sheet joined into another
    // module, renamed or deleted) would run beside the new ones: remove them first.
    if let (true, Some(space)) = (remove_stale, &space) {
        let keep: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        for name in space.stale(&keep) {
            if let Err(e) = mm::delete_module(&mut serial, &name) {
                return failed(e.to_string());
            }
            println!("Модуль «{name}» убран с платы: его нет в новой сборке");
        }
    }
    for (((module, allow), image), &in_place) in modules
        .iter()
        .zip(allows)
        .zip(&plan.images)
        .zip(&plan.in_place)
    {
        let name = &module.name;
        if in_place {
            if let Err(e) = mm::delete_module(&mut serial, name) {
                return failed(e.to_string());
            }
            println!("Модуль «{name}» остановлен для замены на месте");
        }
        let started = std::time::Instant::now();
        let uploaded = if image.aot {
            println!(
                "Модуль «{name}»: AOT {} байт (WASM {} байт) -> {port}...",
                image.bytes.len(),
                module.wasm.len()
            );
            let path = project
                .work_dir()
                .join("wasm")
                .join(name)
                .join(format!("{name}.aot"));
            if let Err(e) = std::fs::write(&path, &image.bytes) {
                return failed(format!("не записан {}: {e}", path.display()));
            }
            match mm::module_memory(&module.wasm) {
                Some(mem) => mm::upload_image(&mut serial, name, &image.bytes, mem, allow),
                None => Err(mm::ModuleUploadError::OldModule),
            }
        } else {
            println!("Модуль «{name}» ({} байт) -> {port}...", module.wasm.len());
            mm::upload_module(&mut serial, name, &module.wasm, allow)
        };
        match uploaded {
            Ok(done) => match done.swap_us {
                Some(us) => println!(
                    "Модуль «{name}» залит и запущен за {:.1} с (замена {} мс)",
                    started.elapsed().as_secs_f32(),
                    us / 1000
                ),
                None => println!(
                    "Модуль «{name}» залит и запущен за {:.1} с",
                    started.elapsed().as_secs_f32()
                ),
            },
            Err(e) => return failed(e.to_string()),
        }
    }
    // The project name for the network search; an old master that does not know it is skipped.
    let _ = mm::set_board_name(&mut serial, &project.config.project.name);
    // Remember the board as the editor does (`board.toml`): the debugger (`monitor`, the editor)
    // recognises an AOT image by it. A failure here never fails the upload.
    if info.takes_aot() {
        let remembered = upload_plan::read_crabmod_space(&mut serial)
            .map_err(|e| e.to_string())
            .and_then(|space| {
                let Some(space) = space else { return Ok(()) };
                let mut memo = crab_build::board_memo::BoardMemo::new(&info, &space);
                memo.modules = modules
                    .iter()
                    .zip(&plan.images)
                    .map(|(m, image)| crab_build::board_memo::MemoModule {
                        name: m.name.clone(),
                        wasm_crc: mm::crc32(&m.wasm),
                        image_crc: mm::crc32(&image.bytes),
                    })
                    .collect();
                crab_build::board_memo::save(&project.work_dir(), &memo).map_err(|e| e.to_string())
            });
        if let Err(e) = remembered {
            eprintln!("предупреждение: не удалось запомнить состояние платы: {e}");
        }
    }
    ExitCode::SUCCESS
}

/// Builds the sheets of the project as WASM modules into `.crab/<project>/wasm/<name>/`
/// (`<name>.wasm` + `module.toml`); with `upload` also sends them to the board (AOT-compiled, with
/// `<name>.aot` next to the `.wasm`, if the master runs AOT).
pub(crate) fn run_gen_wasm(
    project_dir: &Path,
    library: Option<&Path>,
    sheet: Option<u32>,
    watch: bool,
    upload: bool,
    mut link: LinkSpec,
    flags: UploadFlags,
) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let project = crab_codegen::prepare(&project, &lib);
    let mut program = match Program::build(&project, &lib) {
        Ok(program) => program,
        Err(GenError::Schema(diagnostics)) => {
            print_diagnostics(&diagnostics);
            eprintln!("Генерация отменена: в схеме есть ошибки (подробнее — crablogic check)");
            return ExitCode::from(EXIT_SCHEMA_ERRORS);
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::from(EXIT_GEN_FAILED);
        }
    };
    // One chosen sheet is built as it is; the rest are joined into modules (specs/module-ram).
    if sheet.is_none() {
        if let Err(e) = program.auto_group_for_board(watch) {
            eprintln!("ошибка: склейка листов: {e}");
            return ExitCode::from(EXIT_GEN_FAILED);
        }
    }
    let sheets = sheet.map_or_else(|| program.sheets(), |n| vec![n]);
    let mut modules = Vec::new();
    let numbers = sheets.clone();
    for n in sheets {
        match program.wasm_module_with(n, watch) {
            Ok(module) => modules.push(module),
            Err(e) => {
                eprintln!("ошибка: лист {n}: {e}");
                return ExitCode::from(EXIT_GEN_FAILED);
            }
        }
    }
    let mut allows = Vec::new();
    for module in &modules {
        let dir = project.work_dir().join("wasm").join(&module.name);
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(format!("{}.wasm", module.name)), &module.wasm))
            .and_then(|()| std::fs::write(dir.join("module.toml"), &module.manifest));
        if let Err(e) = written {
            eprintln!("ошибка: не записан {}: {e}", dir.display());
            return ExitCode::from(EXIT_GEN_FAILED);
        }
        println!(
            "Модуль «{}»: {} байт -> {}",
            module.name,
            module.wasm.len(),
            dir.display()
        );
        allows.push(
            crab_sim::mcu::Manifest::parse(&module.manifest)
                .map(|m| m.allow)
                .unwrap_or_default(),
        );
    }
    if upload {
        link.password = project.config.project.password.clone();
        // Without --port and --host: USB if the saved port is present, else the saved address.
        if link.port.is_none() && link.host.is_none() {
            match project_link_spec(&project) {
                Ok(chosen) => link = chosen,
                Err(message) => {
                    eprintln!("ошибка: {message}");
                    return ExitCode::from(EXIT_BUILD_FAILED);
                }
            }
        }
        return upload_sheets(
            &project,
            &modules,
            &numbers,
            &allows,
            link,
            flags,
            // A single chosen sheet must leave the other modules alone.
            sheet.is_none(),
        );
    }
    ExitCode::SUCCESS
}
