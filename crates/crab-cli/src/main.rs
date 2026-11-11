//! `crablogic` — command line of CrabLogic.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use crab_build::upload_plan::{self, SheetModule, StopReport, UploadReport};
use crab_build::{ArduinoCli, BuildMode, CompileOptions, Compiled};
use crab_codegen::{ArduinoCpp, Backend, GenError, Program};
use crab_core::logic::SheetSpeed;
use crab_core::{check_project, Diagnostic, Library, LocalValues, Project, Severity};

/// Exit code when the schema has errors.
const EXIT_SCHEMA_ERRORS: u8 = 1;
/// Exit code when the project or library cannot be loaded.
const EXIT_LOAD_FAILED: u8 = 2;
/// Exit code when code generation fails for another reason (templates, files).
const EXIT_GEN_FAILED: u8 = 3;
/// Exit code when arduino-cli fails to build or upload.
const EXIT_BUILD_FAILED: u8 = 4;

#[derive(Parser)]
#[command(
    name = "crablogic",
    version,
    about = "CrabLogic: визуальное программирование микроконтроллеров"
)]
struct Cli {
    /// Папка библиотеки блоков (по умолчанию ищется `library/` рядом с проектом
    /// или программой; можно задать переменной CRAB_LIBRARY)
    #[arg(long, global = true, env = "CRAB_LIBRARY")]
    library: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Проверить схему проекта
    Check {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
    },
    /// Сгенерировать скетч Arduino в .crab/<проект>/sketch/sketch.ino рядом с проектом
    Gen {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Вывести скетч в консоль вместо записи в файл
        #[arg(long)]
        stdout: bool,
        /// Встроить модуль онлайн-отладки
        #[arg(long)]
        debug: bool,
        /// Язык комментариев в скетче: ru или en (по умолчанию из project.toml)
        #[arg(long)]
        lang: Option<String>,
    },
    /// Сгенерировать скетч и собрать прошивку через arduino-cli
    Build {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Отладочная сборка: модуль онлайн-отладки, без сильной оптимизации
        #[arg(long)]
        debug: bool,
        /// Собрать заново, даже если прошивка уже собрана из того же скетча
        #[arg(long)]
        force: bool,
    },
    /// Показывать живые значения с платы (прошивка собрана с --debug)
    Monitor {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Порт платы, например COM3 (по умолчанию из настроек проекта)
        #[arg(long, short)]
        port: Option<String>,
    },
    /// Напечатать хеш проекта (логика и настройки, без координат): по нему редактор сверяет файл с платой
    Hash {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
    },
    /// Прочитать проект из прошивки на плате (нужна отладочная прошивка) и сохранить в файл .crab
    Pull {
        /// Куда сохранить проект (файл .crab)
        output: PathBuf,
        /// Порт платы, например COM3
        #[arg(long, short)]
        port: String,
        /// Пароль проекта, если он был задан
        #[arg(long)]
        password: Option<String>,
        /// Заменить файл, если он уже есть
        #[arg(long)]
        force: bool,
    },
    /// Собрать прошивку и залить её на плату
    Upload {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Порт платы, например COM3 (по умолчанию из настроек проекта или единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Отладочная сборка: модуль онлайн-отладки, без сильной оптимизации
        #[arg(long)]
        debug: bool,
        /// Собрать заново, даже если прошивка уже собрана из того же скетча
        #[arg(long)]
        force: bool,
    },
    /// Собрать листы схемы как WASM-модули (.crab/<проект>/wasm/<лист>/) и при желании залить на плату
    GenWasm {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Только этот лист (по умолчанию все листы с блоками)
        #[arg(long)]
        sheet: Option<u32>,
        /// Модули сообщают значения всех выходов блоков (для отладки)
        #[arg(long)]
        watch: bool,
        /// Залить модули на плату с мастер-прошивкой (если мастер умеет AOT, модули компилируются в .aot)
        #[arg(long)]
        upload: bool,
        /// Порт платы, например COM3 (по умолчанию единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Ошибка, если какой-то лист не удаётся ускорить (без флага — предупреждение)
        #[arg(long)]
        require_aot: bool,
        /// Разрешить остановить листы на время заливки, если новая версия не помещается рядом со старой (без флага — ошибка)
        #[arg(long)]
        allow_stop: bool,
    },
    /// Залить один WASM-модуль на плату с мастер-прошивкой (без сброса выходов и соседних модулей); если мастер умеет AOT, модуль компилируется в .aot
    UploadWasm {
        /// Файл модуля .wasm; рядом может лежать module.toml с именем и правами
        file: PathBuf,
        /// Имя модуля на плате (по умолчанию из module.toml или имя файла)
        #[arg(long)]
        name: Option<String>,
        /// Права на железо через запятую, например gpio_write,millis (заменяют module.toml)
        #[arg(long, value_delimiter = ',')]
        allow: Option<Vec<String>>,
        /// Порт платы, например COM3 (по умолчанию единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
    },
    /// Наборы пользовательских блоков: список, экспорт и импорт файлом .crabblocks
    Block {
        #[command(subcommand)]
        action: BlockAction,
    },
    /// Перевести проект из старого формата (папка с project.toml, logic.toml,
    /// layout.toml) в один файл <папка>/<папка>.crab; старые файлы не удаляются
    Convert {
        /// Папка проекта в старом формате
        project: PathBuf,
    },
}

#[derive(Subcommand)]
enum BlockAction {
    /// Показать наборы и блоки пользовательской библиотеки
    List,
    /// Сохранить набор блоков в файл
    Export {
        /// Имя набора (папка в `user_library/`), например `Мои блоки`
        pack: String,
        /// Куда сохранить (по умолчанию `<набор>.crabblocks` в текущей папке)
        file: Option<PathBuf>,
    },
    /// Добавить набор из файла в пользовательскую библиотеку
    Import {
        /// Файл .crabblocks
        file: PathBuf,
        /// Заменить установленный набор, даже если он не старше
        #[arg(long)]
        replace: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { project } => run_check(&project, cli.library.as_deref()),
        Command::Gen {
            project,
            stdout,
            debug,
            lang,
        } => run_gen(&project, cli.library.as_deref(), stdout, debug, lang),
        Command::Build {
            project,
            debug,
            force,
        } => run_build(&project, cli.library.as_deref(), debug, force, None),
        Command::Upload {
            project,
            port,
            debug,
            force,
        } => run_build(&project, cli.library.as_deref(), debug, force, Some(port)),
        Command::Monitor { project, port } => run_monitor(&project, cli.library.as_deref(), port),
        Command::Hash { project } => match Project::load(&project)
            .map_err(|e| e.to_string())
            .and_then(|p| p.content_hash().map_err(|e| e.to_string()))
        {
            Ok(hash) => {
                println!("{}", crab_core::hash_hex(&hash));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("ошибка: {e}");
                ExitCode::from(EXIT_LOAD_FAILED)
            }
        },
        Command::Pull {
            output,
            port,
            password,
            force,
        } => run_pull(
            &output,
            &port,
            password.as_deref().unwrap_or(""),
            force,
            cli.library.as_deref(),
        ),
        Command::UploadWasm {
            file,
            name,
            allow,
            port,
        } => run_upload_wasm(&file, name, allow, port),
        Command::GenWasm {
            project,
            sheet,
            watch,
            upload,
            port,
            require_aot,
            allow_stop,
        } => run_gen_wasm(
            &project,
            cli.library.as_deref(),
            sheet,
            watch,
            upload,
            port,
            UploadFlags {
                require_aot,
                allow_stop,
            },
        ),
        Command::Block { action } => run_block(action, cli.library.as_deref()),
        Command::Convert { project } => run_convert(&project, cli.library.as_deref()),
    }
}

fn run_block(action: BlockAction, library: Option<&Path>) -> ExitCode {
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
        BlockAction::List => {
            for (folder, info) in pack::list(&user) {
                println!("{folder} · версия {} · {}", info.version, info.description);
                for block in lib.iter().filter(|b| lib.is_user(&b.def.info.kind)) {
                    if block.dir.starts_with(user.join(&folder)) {
                        println!("  {} — {}", block.def.info.kind, block.def.info.name);
                    }
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

fn run_convert(dir: &Path, library: Option<&Path>) -> ExitCode {
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

fn run_check(project_dir: &Path, library: Option<&Path>) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };

    let report = check_project(&project, &lib);
    print_diagnostics(&report.diagnostics);

    let errors = report.errors().count();
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

fn run_gen(
    project_dir: &Path,
    library: Option<&Path>,
    stdout: bool,
    debug: bool,
    lang: Option<String>,
) -> ExitCode {
    let (mut project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    if let Some(lang) = lang {
        if !["ru", "en"].contains(&lang.as_str()) {
            eprintln!("Язык комментариев «{lang}» не поддерживается: допустимы ru и en");
            return ExitCode::FAILURE;
        }
        project.config.project.comment_lang = lang;
    }
    let code = match generate(&project, &lib, debug) {
        Ok((code, ..)) => code,
        Err(exit) => return exit,
    };
    if stdout {
        print!("{code}");
        return ExitCode::SUCCESS;
    }
    match write_sketch(&project, &code) {
        Ok(()) => ExitCode::SUCCESS,
        Err(exit) => exit,
    }
}

/// Generates, writes and compiles the sketch; with `upload = Some(port)` also uploads it.
fn run_build(
    project_dir: &Path,
    library: Option<&Path>,
    debug: bool,
    force: bool,
    upload: Option<Option<String>>,
) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let (libraries, attached) = match generate(&project, &lib, debug) {
        Ok((code, libraries, attached)) => match write_sketch(&project, &code) {
            Ok(()) => (libraries, attached),
            Err(exit) => return exit,
        },
        Err(exit) => return exit,
    };
    let build_failed = |e: crab_build::BuildError| {
        eprintln!("ошибка: {e}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let cli = match ArduinoCli::locate() {
        Ok(cli) => cli,
        Err(e) => return build_failed(e),
    };
    let board = &project.config.board;
    let opts = CompileOptions {
        mode: if debug {
            BuildMode::Debug
        } else {
            BuildMode::Release
        },
        force,
        libraries: attached,
        ..Default::default()
    };
    let mut print = |line: &str| println!("{line}");

    let required: Vec<&str> = libraries.iter().map(String::as_str).collect();
    match cli.ensure_libraries(&required, &mut print) {
        Ok(installed) if !installed.is_empty() => {
            println!("Установлены библиотеки: {}", installed.join(", "));
        }
        Ok(_) => {}
        Err(e) => return build_failed(e),
    }
    println!("Сборка для {}...", board.fqbn);
    if let Some(reason) = cli.cli_reason(&project.work_dir(), board, &opts) {
        println!("Сборка через arduino-cli: {reason}");
    }
    match cli.compile(&project.work_dir(), board, &opts, &mut print) {
        Ok(Compiled::UpToDate) => println!("Прошивка уже собрана, скетч не менялся"),
        Ok(Compiled::Direct) => {
            println!("Прямая сборка: изменился только скетч, arduino-cli не запускался")
        }
        Ok(Compiled::Built) => {}
        Err(e) => return build_failed(e),
    }
    let Some(port) = upload else {
        println!(
            "Сборка готова: {}",
            crab_build::build_dir(&project.work_dir()).display()
        );
        return ExitCode::SUCCESS;
    };
    let port = match cli.choose_port(port.as_deref(), board) {
        Ok(port) => port,
        Err(e) => return build_failed(e),
    };
    println!("Заливка на {port}...");
    if let Err(e) = cli.upload(&project.work_dir(), board, &port, &mut print) {
        return build_failed(e);
    }
    println!("Прошивка залита на {port}");
    ExitCode::SUCCESS
}

/// Uploads one `.wasm` module to the master firmware over serial (as an AOT image if the master runs AOT).
fn run_upload_wasm(
    file: &Path,
    name: Option<String>,
    allow: Option<Vec<String>>,
    port: Option<String>,
) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
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
    push_module(&name, &wasm, &allow, &[], port, cache, None)
}

/// Sends one module to the master firmware over serial: as an AOT image when the master runs AOT
/// (built into `cache`, copied to `aot_copy` if given), otherwise as `.wasm` with a warning.
fn push_module(
    name: &str,
    wasm: &[u8],
    allow: &[String],
    needs: &[String],
    port: Option<String>,
    cache: &Path,
    aot_copy: Option<&Path>,
) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let (mut serial, port) = match open_master_port(port) {
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
                Some(us) => println!("Модуль «{name}» залит и запущен (замена {} мс)", us / 1000),
                None => println!("Модуль «{name}» залит и запущен"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => failed(e.to_string()),
    }
}

/// Opens the serial port of the board (the only found one if `port` is not given) without
/// resetting it. Returns the port and its name.
fn open_master_port(
    port: Option<String>,
) -> Result<(Box<dyn serialport::SerialPort>, String), String> {
    let port = match port {
        Some(port) => port,
        None => ArduinoCli::locate()
            .and_then(|cli| cli.serial_ports())
            .and_then(|ports| crab_build::auto_port(&ports))
            .map_err(|e| e.to_string())?,
    };
    let mut serial = serialport::new(&port, 115_200)
        .timeout(std::time::Duration::from_millis(200))
        .open()
        .map_err(|e| format!("порт {port} не открыт: {e}"))?;
    // Release DTR/RTS so opening the port does not reset the board.
    let _ = serial.write_data_terminal_ready(false);
    let _ = serial.write_request_to_send(false);
    Ok((serial, port))
}

/// What to do with the upload report: an error with `--require-aot`, a warning text otherwise.
fn report_verdict(
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
fn stop_verdict(stops: Option<&StopReport>, allow_stop: bool) -> Result<Option<String>, String> {
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
/// is also written next to its `.wasm`.
fn upload_sheets(
    project: &Project,
    modules: &[crab_codegen::WasmModule],
    numbers: &[u32],
    allows: &[Vec<String>],
    port: Option<String>,
    require_aot: bool,
    allow_stop: bool,
) -> ExitCode {
    use crab_build::mcu_module as mm;
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let (mut serial, port) = match open_master_port(port) {
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
                Some(us) => println!("Модуль «{name}» залит и запущен (замена {} мс)", us / 1000),
                None => println!("Модуль «{name}» залит и запущен"),
            },
            Err(e) => return failed(e.to_string()),
        }
    }
    ExitCode::SUCCESS
}

/// Strictness flags of `gen-wasm --upload`.
#[derive(Clone, Copy)]
struct UploadFlags {
    require_aot: bool,
    allow_stop: bool,
}

/// Builds the sheets of the project as WASM modules into `.crab/<project>/wasm/<name>/`
/// (`<name>.wasm` + `module.toml`); with `upload` also sends them to the board (AOT-compiled, with
/// `<name>.aot` next to the `.wasm`, if the master runs AOT).
fn run_gen_wasm(
    project_dir: &Path,
    library: Option<&Path>,
    sheet: Option<u32>,
    watch: bool,
    upload: bool,
    port: Option<String>,
    flags: UploadFlags,
) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let project = crab_codegen::prepare(&project, &lib);
    let program = match Program::build(&project, &lib) {
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
        return upload_sheets(
            &project,
            &modules,
            &numbers,
            &allows,
            port,
            flags.require_aot,
            flags.allow_stop,
        );
    }
    ExitCode::SUCCESS
}

/// Generates the sketch, printing schema errors. Also returns the Arduino libraries it needs.
fn generate(
    project: &Project,
    lib: &Library,
    debug: bool,
) -> Result<(String, Vec<String>, Vec<PathBuf>), ExitCode> {
    let backend = ArduinoCpp { debug };
    // Blocks written as WASM modules need their C translation next to the module.
    if let Some(w2c2) = crab_build::wasm_c::find_w2c2(None) {
        for problem in crab_build::wasm_c::translate_library(&w2c2, lib) {
            eprintln!("предупреждение: {problem}");
        }
    }
    // Wi-Fi passwords and other `local` values are kept by the program, not in the file.
    let store = LocalValues::open_default().unwrap_or_else(|e| {
        eprintln!("предупреждение: {e}");
        LocalValues::default()
    });
    let source = project;
    let project = &project.with_local(lib, &store);
    let result = Program::build(project, lib).and_then(|program| {
        let code = backend.generate(&program)?;
        let code = crab_codegen::embed_project(&code, source)?;
        let libraries = program.libraries().into_iter().map(String::from).collect();
        Ok((code, libraries, program.attached_libraries()))
    });
    match result {
        Ok(generated) => Ok(generated),
        Err(GenError::Schema(diagnostics)) => {
            print_diagnostics(&diagnostics);
            eprintln!("Генерация отменена: в схеме есть ошибки (подробнее — crablogic check)");
            Err(ExitCode::from(EXIT_SCHEMA_ERRORS))
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            Err(ExitCode::from(EXIT_GEN_FAILED))
        }
    }
}

fn write_sketch(project: &Project, code: &str) -> Result<(), ExitCode> {
    match crab_codegen::write_sketch(&project.work_dir(), code) {
        Ok((path, true)) => println!("Скетч записан: {}", path.display()),
        Ok((path, false)) => println!("Скетч не изменился: {}", path.display()),
        Err(e) => {
            eprintln!("ошибка: {e}");
            return Err(ExitCode::from(EXIT_GEN_FAILED));
        }
    }
    Ok(())
}

/// Prints live values every half second until Ctrl+C.
/// Reads the project out of the board's flash and saves it as `output`.
fn run_pull(
    output: &Path,
    port: &str,
    password: &str,
    force: bool,
    library: Option<&Path>,
) -> ExitCode {
    if output.exists() && !force {
        eprintln!(
            "ошибка: файл {} уже есть; укажите --force, чтобы заменить его",
            output.display()
        );
        return ExitCode::FAILURE;
    }
    let mut transport = match crab_debug::transport::serial(port)() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("ошибка: не удалось открыть порт {port}: {e}");
            return ExitCode::from(EXIT_BUILD_FAILED);
        }
    };
    println!("Чтение проекта с платы ({port})...");
    let pulled = match crab_debug::read_project(transport.as_mut(), password, |got, total| {
        eprint!("\r{got} из {total} байт");
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("\nошибка: {e}");
            return ExitCode::from(EXIT_BUILD_FAILED);
        }
    };
    eprintln!();
    let archive = match crab_core::protect::open(&pulled.data, password) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::FAILURE;
        }
    };
    let project = match Project::from_archive_bytes(&archive, output) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::from(EXIT_LOAD_FAILED);
        }
    };
    if let Err(e) = project.save() {
        eprintln!("ошибка: {e}");
        return ExitCode::from(EXIT_LOAD_FAILED);
    }
    println!("Проект сохранён: {}", output.display());
    // The firmware must have been built from this very schema.
    if let (Some(fp), Some(lib)) = (pulled.fingerprint, load_library(output, library)) {
        let project = project.without_disabled();
        match Program::build(&project, &lib) {
            Ok(program) => {
                let same = crab_codegen::debug::debug_table(&program).fingerprint() == fp;
                if same {
                    println!("Отпечаток схемы совпадает с прошивкой.");
                } else {
                    println!("Внимание: отпечаток схемы не совпадает с прошивкой.");
                }
            }
            Err(e) => eprintln!("предупреждение: отпечаток не проверен: {e}"),
        }
    }
    ExitCode::SUCCESS
}

fn run_monitor(project_dir: &Path, library: Option<&Path>, port: Option<String>) -> ExitCode {
    let (project, lib) = match load(project_dir, library) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let project = project.without_disabled();
    let program = match Program::build(&project, &lib) {
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
    let Some(port) = port.or_else(|| project.config.board.port.clone()) else {
        eprintln!(
            "ошибка: укажите порт через --port или в файле проекта ([board] port = \"COM3\")"
        );
        return ExitCode::from(EXIT_BUILD_FAILED);
    };
    let table = crab_codegen::debug::debug_table(&program);
    let names: Vec<String> = table
        .entries
        .iter()
        .map(|e| match &e.source {
            crab_debug::Source::Output(p) => p.to_string(),
            crab_debug::Source::Var(v) => format!("${v}"),
        })
        .collect();
    let session = crab_debug::Session::start_with(
        table,
        crab_debug::transport::serial(&port),
        project.config.project.password.clone(),
        || {},
    );
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
        println!("[{}] {}", live.status, values.join("  "));
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

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for d in diagnostics {
        match d.severity {
            Severity::Error => eprintln!("{d}"),
            Severity::Warning => println!("{d}"),
        }
    }
}

/// Loads the project and the block library, printing errors.
fn load(project_dir: &Path, library: Option<&Path>) -> Result<(Project, Library), ExitCode> {
    let project = Project::load(project_dir).map_err(|e| {
        eprintln!("ошибка: {e}");
        ExitCode::from(EXIT_LOAD_FAILED)
    })?;
    let lib = load_library(project_dir, library).ok_or(ExitCode::from(EXIT_LOAD_FAILED))?;
    Ok((project, lib))
}

/// Loads the block library; library problems are printed as warnings.
fn load_library(project_dir: &Path, explicit: Option<&Path>) -> Option<Library> {
    let Some(dir) = explicit
        .map(Path::to_path_buf)
        .or_else(|| find_library(project_dir))
    else {
        eprintln!(
            "ошибка: не найдена папка библиотеки блоков `library/`; укажите её через --library"
        );
        return None;
    };
    // User blocks packed into the project file but missing here are restored first;
    // a project folder (old format) may bring a board of its own.
    if crab_core::project::is_project_file(project_dir) || project_dir.is_dir() {
        if let Ok((mut lib, _)) = Library::load_with_user(&dir) {
            let user = crab_core::user_library_dir(&dir);
            let boards = crab_core::packed::unpack_missing_boards(project_dir, &user, &lib);
            if !boards.is_empty() {
                println!("из проекта добавлена плата: {}", boards.join(", "));
                if let Ok((reloaded, _)) = Library::load_with_user(&dir) {
                    lib = reloaded;
                }
            }
            let written = crab_core::packed::unpack_missing_blocks(project_dir, &user, &lib);
            if !written.is_empty() {
                println!("из проекта добавлены блоки: {}", written.join(", "));
            }
        }
    }
    match Library::load_with_user(&dir) {
        Ok((lib, errors)) => {
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
fn find_library(project_dir: &Path) -> Option<PathBuf> {
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
        .find(|dir| dir.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_build::placement::DemoteReason;
    use crab_build::upload_plan::DemotedSheet;

    fn report() -> UploadReport {
        UploadReport {
            demoted: vec![DemotedSheet {
                sheet: 2,
                name: "sheet2".to_owned(),
                reason: DemoteReason::NoSpace,
                step_us: 250,
                fast_step_us: 70,
                exceeds_period: false,
            }],
            wamrc_problem: None,
            needed: 8192,
            free: 4096,
        }
    }

    #[test]
    fn verdict_without_report() {
        assert_eq!(report_verdict(None, false), Ok(None));
        assert_eq!(report_verdict(None, true), Ok(None));
    }

    #[test]
    fn verdict_with_report() {
        let r = report();
        assert_eq!(report_verdict(Some(&r), true), Err(r.message()));
        assert_eq!(report_verdict(Some(&r), false), Ok(Some(r.message())));
    }

    #[test]
    fn require_aot_flag_parses() {
        let cli = Cli::try_parse_from([
            "crablogic",
            "gen-wasm",
            "p.crab",
            "--upload",
            "--require-aot",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::GenWasm {
                require_aot: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["crablogic", "gen-wasm", "p.crab"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::GenWasm {
                require_aot: false,
                ..
            }
        ));
    }

    #[test]
    fn stop_verdict_with_and_without_flag() {
        use crab_build::upload_plan::StoppedSheet;
        assert_eq!(stop_verdict(None, false), Ok(None));
        let stops = StopReport {
            sheets: vec![StoppedSheet {
                sheet: 2,
                name: "sheet2".to_owned(),
                bytes: 1000,
            }],
        };
        let err = stop_verdict(Some(&stops), false).unwrap_err();
        assert!(err.starts_with(&stops.message()));
        assert!(err.contains("--allow-stop"));
        assert_eq!(stop_verdict(Some(&stops), true), Ok(Some(stops.message())));
    }

    #[test]
    fn allow_stop_flag_parses() {
        let cli = Cli::try_parse_from([
            "crablogic",
            "gen-wasm",
            "p.crab",
            "--upload",
            "--allow-stop",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::GenWasm {
                allow_stop: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["crablogic", "gen-wasm", "p.crab"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::GenWasm {
                allow_stop: false,
                ..
            }
        ));
    }
}
