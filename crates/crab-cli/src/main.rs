//! `crablogic` — command line of CrabLogic.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod bench;
mod report_cmd;
mod report_decode;

use clap::{Parser, Subcommand};
use crab_build::upload_plan::{self, SheetModule, StopReport, UploadReport};
use crab_codegen::{GenError, Program};
use crab_core::logic::SheetSpeed;
use crab_core::{Diagnostic, Library, Project, Severity};

/// Exit code when the schema has errors.
const EXIT_SCHEMA_ERRORS: u8 = 1;
/// Exit code when the project or library cannot be loaded.
const EXIT_LOAD_FAILED: u8 = 2;
/// Exit code when module generation fails for another reason (a block the generator refuses, files).
const EXIT_GEN_FAILED: u8 = 3;
/// Exit code when an upload to the board fails.
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
    /// Показывать живые значения с платы (модули собраны с --watch). Без --port и --host связь
    /// выбирается по проекту: сохранённый порт есть в системе — по USB, иначе по адресу `[board] host`
    Monitor {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Порт платы, например COM3
        #[arg(long, short)]
        port: Option<String>,
        /// Адрес платы по сети, например 192.168.1.50 или 192.168.1.50:4040 (нужен пароль проекта и
        /// ключ на плате: crablogic netkey)
        #[arg(long)]
        host: Option<String>,
    },
    /// Напечатать хеш проекта (логика и настройки, без координат): по нему редактор сверяет файл с платой
    Hash {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
    },
    /// Прочитать проект с платы (недоступно с мастер-прошивкой: она не хранит копию проекта)
    #[command(hide = true)]
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
        #[arg(long, short, conflicts_with = "host")]
        port: Option<String>,
        /// Залить по сети: адрес платы, например 192.168.1.50 или 192.168.1.50:4040 (нужен пароль проекта
        /// и ключ на плате: crablogic netkey)
        #[arg(long)]
        host: Option<String>,
        /// Ошибка, если какой-то лист не удаётся ускорить (без флага — предупреждение)
        #[arg(long)]
        require_aot: bool,
        /// Разрешить остановить листы на время заливки, если новая версия не помещается рядом со старой (без флага — ошибка)
        #[arg(long)]
        allow_stop: bool,
    },
    /// Найти платы CrabLogic в локальной сети (ответ платы на широковещательный запрос)
    Find {
        /// Сколько ждать ответов, миллисекунд
        #[arg(long, default_value_t = 1500)]
        timeout_ms: u64,
    },
    /// Записать на плату по USB ключ для заливки по сети (из пароля проекта) или убрать его
    Netkey {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Порт платы, например COM3 (по умолчанию единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Убрать ключ с платы (заливка по сети перестанет работать)
        #[arg(long)]
        remove: bool,
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
        /// Порт платы, например COM3 (по умолчанию: порт проекта из --project, иначе единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Адрес платы по сети, например 192.168.1.50 или 192.168.1.50:4040. Пароль для входа берётся
        /// из проекта, поэтому с --host обязателен --project
        #[arg(long)]
        host: Option<String>,
        /// Файл проекта .crab (или папка с ним): пароль для --host; без --port и --host из него
        /// выбирается связь (порт, если он есть в системе, иначе адрес)
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Замеры производительности мастера на плате: заливает стенды, снимает цифры из `list`, пишет
    /// отчёт bench/<чип>-<мастер>-<дата>.{toml,md}. Модули на плате заменяются стендами.
    Bench {
        /// Порт платы, например COM10 (по умолчанию единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Только эти стенды через запятую: id (logic_100_aot), начало id (logic_10) или семейство (adc)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Папка для отчётов
        #[arg(long, default_value = "bench")]
        out: PathBuf,
        /// Файл с настройками Wi-Fi и MQTT для сетевого стенда (в git не попадает)
        #[arg(long, default_value = "bench/secrets.toml")]
        secrets: PathBuf,
        /// Чип платы: esp32 или esp32c3 (по умолчанию определяется по мастеру)
        #[arg(long)]
        chip: Option<String>,
        /// Короткий прогон для проверки: 2 с разогрева и 1 с между чтениями (цифры неточны)
        #[arg(long)]
        quick: bool,
        /// IP платы для стенда Modbus TCP (по умолчанию берётся из журнала мастера: «wifi ready, address»)
        #[arg(long)]
        modbus_host: Option<String>,
        /// Сравнить два готовых отчёта .toml вместо замера: --compare a.toml b.toml
        #[arg(long, num_args = 2, value_names = ["A", "B"])]
        compare: Option<Vec<PathBuf>>,
    },
    /// Записать на плату мастер-прошивку (Rust) из папки firmware дистрибутива
    FlashMaster {
        /// Файл проекта .crab: плата берётся из него (иначе нужен --chip)
        project: Option<PathBuf>,
        /// Чип платы: esp32 или esp32c3 (вместо проекта)
        #[arg(long)]
        chip: Option<String>,
        /// Порт платы, например COM3 (по умолчанию единственный найденный)
        #[arg(long, short)]
        port: Option<String>,
        /// Папка с образами мастера (по умолчанию firmware рядом с программой)
        #[arg(long)]
        firmware: Option<PathBuf>,
    },
    /// Свои платы (контроллеры): проверка папки платы
    Board {
        #[command(subcommand)]
        action: BoardAction,
    },
    /// Наборы пользовательских блоков: список, экспорт и импорт файлом .crabblocks
    Block {
        #[command(subcommand)]
        action: BlockAction,
    },
    /// Собрать отчёт об ошибке в ZIP: данные платы, дамп сбоя, проект без паролей
    Report {
        /// Порт платы, например COM3 (без порта отчёт без данных платы)
        #[arg(long, short)]
        port: Option<String>,
        /// Файл проекта .crab (пароли и ключи заменяются на ***)
        #[arg(long)]
        project: Option<PathBuf>,
        /// Описание проблемы
        #[arg(long)]
        describe: Option<String>,
        /// Куда сохранить (по умолчанию crablogic-report-<дата>-<время>.zip в текущей папке)
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Стереть дамп сбоя на плате после того, как он прочитан и отчёт записан
        #[arg(long)]
        erase_core: bool,
    },
    /// Расшифровать дамп сбоя платы из отчёта в стек с именами функций (нужны gdb чипа и ELF мастера)
    ReportDecode {
        /// ZIP-файл отчёта
        zip: PathBuf,
        /// ELF мастера той же версии (по умолчанию firmware/<чип>/crab-master-rs.elf или последняя сборка)
        #[arg(long)]
        elf: Option<PathBuf>,
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
    /// Проверить папку одного блока (block.toml, block.wasm, docs.md)
    Check {
        /// Папка блока, например user_library/Мои блоки/my_block
        dir: PathBuf,
    },
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

#[derive(Subcommand)]
enum BoardAction {
    /// Проверить папку платы (board.toml): формат, выводы чипа, периферию, поддержку мастером на Rust
    Check {
        /// Папка платы, например user_library/boards/my_board
        dir: PathBuf,
    },
}

fn main() -> ExitCode {
    let code = run();
    // The project's own blocks were only needed while the command ran.
    let _ = std::fs::remove_dir_all(session_blocks_dir());
    code
}

/// Session folder of the blocks a project brings (see `load_library`).
fn session_blocks_dir() -> PathBuf {
    std::env::temp_dir()
        .join("crablogic_cli_blocks")
        .join(std::process::id().to_string())
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { project } => run_check(&project, cli.library.as_deref()),
        Command::Report {
            port,
            project,
            describe,
            out,
            erase_core,
        } => report_cmd::run_report(
            report_cmd::ReportArgs {
                port,
                project,
                describe,
                out,
                erase_core,
            },
            cli.library.as_deref(),
        ),
        Command::ReportDecode { zip, elf } => match report_decode::run(&zip, elf.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ошибка: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Monitor {
            project,
            port,
            host,
        } => run_monitor(&project, cli.library.as_deref(), port, host),
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
        Command::Netkey {
            project,
            port,
            remove,
        } => run_netkey(&project, port, remove),
        Command::Find { timeout_ms } => run_find(timeout_ms),
        Command::UploadWasm {
            file,
            name,
            allow,
            port,
            host,
            project,
        } => run_upload_wasm(&file, name, allow, port, host, project.as_deref()),
        Command::GenWasm {
            project,
            sheet,
            watch,
            upload,
            port,
            host,
            require_aot,
            allow_stop,
        } => run_gen_wasm(
            &project,
            cli.library.as_deref(),
            sheet,
            watch,
            upload,
            LinkSpec {
                port,
                host,
                password: String::new(),
            },
            UploadFlags {
                require_aot,
                allow_stop,
            },
        ),
        Command::Bench {
            port,
            only,
            out,
            secrets,
            chip,
            quick,
            modbus_host,
            compare,
        } => match compare {
            Some(files) => bench::run_compare(&files[0], &files[1]),
            None => bench::run(
                bench::BenchArgs {
                    port,
                    only,
                    out,
                    secrets,
                    chip,
                    quick,
                    modbus_host,
                },
                cli.library.as_deref(),
            ),
        },
        Command::FlashMaster {
            project,
            chip,
            port,
            firmware,
        } => match run_flash_master(project.as_deref(), chip.as_deref(), port, firmware) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ошибка: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Board {
            action: BoardAction::Check { dir },
        } => run_board_check(&dir, cli.library.as_deref()),
        Command::Block { action } => run_block(action, cli.library.as_deref()),
        Command::Convert { project } => run_convert(&project, cli.library.as_deref()),
    }
}

/// `block check`: one block folder with every problem named and a hint how to fix it.
fn run_block_check(dir: &Path, library: Option<&Path>) -> ExitCode {
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

fn run_block(action: BlockAction, library: Option<&Path>) -> ExitCode {
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

    let report = crab_codegen::check_project(&project, &lib);
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

/// Uploads one `.wasm` module to the master firmware over serial (as an AOT image if the master runs AOT).
fn run_upload_wasm(
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

/// The link of `upload-wasm`: `--host` (with the password of `--project`) or `--port`; with neither,
/// the choice by `--project` (a saved port that is present goes by USB, else the saved address) or
/// the only port found.
fn explicit_link(
    port: Option<String>,
    host: Option<String>,
    project: Option<&Path>,
) -> Result<LinkSpec, String> {
    if port.is_some() && host.is_some() {
        return Err("укажите что-то одно: --port или --host".into());
    }
    let loaded = match project {
        Some(path) => Some(Project::load(path).map_err(|e| e.to_string())?),
        None => None,
    };
    let password = loaded
        .as_ref()
        .map(|p| p.config.project.password.clone())
        .unwrap_or_default();
    if host.is_some() && loaded.is_none() {
        return Err(
            "для --host нужен --project <файл проекта>: пароль для входа по сети берётся из проекта"
                .into(),
        );
    }
    if port.is_some() || host.is_some() {
        return Ok(LinkSpec {
            port,
            host,
            password,
        });
    }
    match loaded {
        Some(project) => project_link_spec(&project),
        None => Ok(LinkSpec::serial(None)),
    }
}

/// The link of the project by the serial ports present now.
fn project_link_spec(project: &Project) -> Result<LinkSpec, String> {
    let mut link = LinkSpec::serial(None);
    link.password = project.config.project.password.clone();
    match crab_build::project_link(&project.config.board)? {
        crab_build::Chosen::Serial(port) => link.port = Some(port),
        crab_build::Chosen::Net(host) => link.host = Some(host),
    }
    Ok(link)
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

/// Opens the serial port of the board (the only found one if `port` is not given) without
/// resetting it. Returns the port and its name.
/// `board check`: one board folder with every problem named (a broken board is otherwise only
/// skipped by the library, and its blocks show up as unknown types).
fn run_board_check(dir: &Path, library: Option<&Path>) -> ExitCode {
    use crab_core::boards::{Boards, UserBoard};
    let board = match UserBoard::load(dir) {
        Ok(board) => board,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut bad = 0;
    let info = &board.def.info;
    if crab_build::flash::MasterChip::of_fqbn(&info.fqbn).is_none() {
        eprintln!(
            "ошибка: мастер на Rust есть только для ESP32 и ESP32-C3, а FQBN «{}» — другой чип",
            info.fqbn
        );
        bad += 1;
    }
    if !info.options.is_empty() {
        eprintln!(
            "предупреждение: [board.options] ({}) были нужны только сборке скетча (её больше нет) — мастер на Rust их не читает, уберите",
            info.options.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    // Device blocks are made from built-in blocks: an unknown `kind` or a bad parameter shows here.
    let here = std::env::current_dir().unwrap_or_default();
    match library
        .map(Path::to_path_buf)
        .or_else(|| find_library(&here))
        .map(Library::load)
    {
        Some(Ok((mut lib, _))) => {
            let mut boards = Boards::default();
            boards.insert(board.clone());
            for e in lib.set_boards(boards) {
                eprintln!("ошибка: {e}");
                bad += 1;
            }
            for dev in &board.def.devices {
                let kind = format!("{}_{}", info.id, dev.id);
                if lib.get(&kind).is_some() {
                    println!(
                        "блок периферии: {kind} («{}: {}», каналов {})",
                        info.name,
                        dev.name,
                        dev.channels.len()
                    );
                }
            }
        }
        _ => {
            eprintln!(
                "предупреждение: библиотека блоков не найдена (--library): периферия не проверена"
            );
        }
    }
    println!(
        "Плата «{}» ({}, {}): выводов {}, выводов расширителей {}, устройств {}",
        info.name,
        info.id,
        info.chip,
        board.def.pins.len(),
        board.def.expander_pins.len(),
        board.def.devices.len()
    );
    if !board.dir.join("docs.md").is_file() {
        println!("предупреждение: нет docs.md с описанием платы");
    }
    if bad == 0 {
        println!("Ошибок нет");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `flash-master`: the prebuilt Rust master for the board of the project (or `--chip`).
fn run_flash_master(
    project: Option<&Path>,
    chip: Option<&str>,
    port: Option<String>,
    firmware: Option<PathBuf>,
) -> Result<(), String> {
    use crab_build::flash;
    let fqbn = match (project, chip) {
        (_, Some(chip)) => format!("esp32:esp32:{chip}"),
        (Some(project), None) => Project::load(project)
            .map_err(|e| e.to_string())?
            .config
            .board
            .fqbn
            .clone(),
        (None, None) => return Err("укажите проект или --chip esp32 / esp32c3".into()),
    };
    let dir = firmware
        .or_else(flash::firmware_dir)
        .ok_or("не найдена папка firmware с образами мастера (укажите --firmware)")?;
    let image = flash::find_image(&dir, &fqbn).map_err(|e| e.to_string())?;
    let port = match port {
        Some(port) => port,
        None => flash::single_usb_port().map_err(|e| e.to_string())?,
    };
    let started = std::time::Instant::now();
    flash::flash_master(&image, &port, &mut |line| println!("{line}"))
        .map_err(|e| e.to_string())?;
    println!(
        "Мастер-прошивка записана на {port} за {:.1} с",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// How to reach the board: over USB (`port`, or the only found one) or over the network (`host`).
struct LinkSpec {
    port: Option<String>,
    host: Option<String>,
    /// Project password, the source of the network key.
    password: String,
}

impl LinkSpec {
    fn serial(port: Option<String>) -> Self {
        LinkSpec {
            port,
            host: None,
            password: String::new(),
        }
    }
}

/// Opens the link to the master: TCP with the login for `host`, otherwise the serial port.
/// Returns the link and a name for messages.
fn open_link(spec: LinkSpec) -> Result<(Box<dyn crab_build::net_link::Link>, String), String> {
    match spec.host {
        Some(host) => {
            let stream =
                crab_build::net_link::connect(&host, &spec.password).map_err(|e| e.to_string())?;
            Ok((Box::new(stream), host))
        }
        None => {
            let (serial, port) = open_master_port(spec.port)?;
            Ok((Box::new(serial), port))
        }
    }
}

/// `find`: prints the CrabLogic boards that answer the search in the local network.
fn run_find(timeout_ms: u64) -> ExitCode {
    let found = crab_debug::find::find_boards(std::time::Duration::from_millis(timeout_ms));
    if found.is_empty() {
        eprintln!("платы не найдены.");
        eprintln!("  Проверьте, что плата подключена к той же сети (Wi-Fi или Ethernet).");
        eprintln!(
            "  Брандмауэр Windows может блокировать ответы: разрешите crablogic в частной сети."
        );
        eprintln!("  Широковещание не проходит через другую подсеть и VLAN: тогда введите адрес вручную (--host).");
        return ExitCode::FAILURE;
    }
    for f in &found {
        println!("{}", find_line(f));
    }
    ExitCode::SUCCESS
}

/// One line of `find`: address, id, chip, master version, hash, key state and name.
fn find_line(f: &crab_debug::find::Found) -> String {
    let b = &f.board;
    let hash = b.hash.as_deref().unwrap_or("-");
    let key = if b.has_key {
        "ключ есть"
    } else {
        "ключа нет"
    };
    let mut line = format!(
        "{}  {}  {}  мастер {}  хеш {}  {}",
        f.ip, b.id, b.chip, b.master, hash, key
    );
    if !b.name.is_empty() {
        line.push_str("  ");
        line.push_str(&b.name);
    }
    line
}

/// `netkey`: writes the network key of the project password to the board over USB, or removes it.
fn run_netkey(project_dir: &Path, port: Option<String>, remove: bool) -> ExitCode {
    let failed = |message: String| {
        eprintln!("ошибка: {message}");
        ExitCode::from(EXIT_BUILD_FAILED)
    };
    let password = if remove {
        None
    } else {
        match Project::load(project_dir) {
            Ok(project) => Some(project.config.project.password.clone()),
            Err(e) => {
                eprintln!("ошибка: {e}");
                return ExitCode::from(EXIT_LOAD_FAILED);
            }
        }
    };
    let (mut serial, _) = match open_master_port(port) {
        Ok(opened) => opened,
        Err(message) => return failed(message),
    };
    if let Err(e) =
        crab_build::mcu_module::wait_for_master(&mut serial, std::time::Duration::from_secs(8))
    {
        return failed(e.to_string());
    }
    if let Err(e) = crab_build::net_link::set_net_key(&mut serial, password.as_deref()) {
        return failed(e.to_string());
    }
    if remove {
        println!("ключ для заливки по сети удалён с платы");
    } else {
        println!(
            "ключ для заливки по сети записан на плату (порт 4040 откроется, когда модуль поднимет Wi-Fi или Ethernet)"
        );
    }
    ExitCode::SUCCESS
}

fn open_master_port(
    port: Option<String>,
) -> Result<(Box<dyn serialport::SerialPort>, String), String> {
    let port = match port {
        Some(port) => port,
        None => crab_build::serial_ports()
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

/// Prints live values every half second until Ctrl+C.
/// Reads the project out of the board's flash and saves it as `output`.
/// `pull` read the project copy from the old debug firmware (binary protocol); the Rust master
/// keeps no copy of the project, so there is nothing to read.
fn run_pull(
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

fn run_monitor(
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
        .find(|dir| {
            let own = dir
                .parent()
                .is_some_and(|p| p.join(crab_core::project::PROJECT_FILE).is_file());
            dir.is_dir() && !own && crab_core::library::holds_blocks(dir)
        })
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
    fn host_conflicts_with_port_and_netkey_parses() {
        assert!(Cli::try_parse_from([
            "crablogic",
            "gen-wasm",
            "p.crab",
            "--upload",
            "--host",
            "1.2.3.4",
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "crablogic",
            "gen-wasm",
            "p.crab",
            "--upload",
            "--host",
            "1.2.3.4",
            "-p",
            "COM3",
        ])
        .is_err());
        let cli = Cli::try_parse_from(["crablogic", "netkey", "p.crab", "--remove"]).unwrap();
        assert!(matches!(cli.command, Command::Netkey { remove: true, .. }));
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
