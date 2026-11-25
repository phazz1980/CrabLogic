pub(crate) mod block;
pub(crate) mod board;
pub(crate) mod common;
pub(crate) mod inspect;
pub(crate) mod link;
pub(crate) mod net;
pub(crate) mod project;
pub(crate) mod wasm;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use crab_core::Project;

use crate::{bench, report_cmd, report_decode};
use common::EXIT_LOAD_FAILED;
use link::LinkSpec;
use wasm::UploadFlags;

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
    /// Собрать электрическую документацию щита (panel.toml) в PDF: схемы на листах A4
    Panel {
        /// Файл проекта .crab (или папка с ним)
        project: PathBuf,
        /// Язык подписей на листах: ru или en
        #[arg(long, default_value = "ru")]
        lang: String,
        /// Формат листов: a3 (по умолчанию; ГОСТ IEC 61082-1 рекомендует его для схем) или a4
        #[arg(long, default_value = "a3")]
        format: String,
        /// Куда записать PDF (по умолчанию `<имя проекта>_panel.pdf` в текущей папке)
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Записать номера проводов в проект, чтобы провода не меняли номера при правках щита
        #[arg(long)]
        save_numbers: bool,
        /// Папка для таблиц в CSV (перечень клемм, перечень проводов, спецификация; разделитель `;`,
        /// UTF-8 с меткой, открывается в Excel)
        #[arg(long)]
        csv: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum BlockAction {
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

pub(crate) fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { project } => project::run_check(&project, cli.library.as_deref()),
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
        } => inspect::run_monitor(&project, cli.library.as_deref(), port, host),
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
        } => inspect::run_pull(
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
        } => net::run_netkey(&project, port, remove),
        Command::Find { timeout_ms } => net::run_find(timeout_ms),
        Command::UploadWasm {
            file,
            name,
            allow,
            port,
            host,
            project,
        } => wasm::run_upload_wasm(&file, name, allow, port, host, project.as_deref()),
        Command::GenWasm {
            project,
            sheet,
            watch,
            upload,
            port,
            host,
            require_aot,
            allow_stop,
        } => wasm::run_gen_wasm(
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
        } => match board::run_flash_master(project.as_deref(), chip.as_deref(), port, firmware) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ошибка: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Board {
            action: BoardAction::Check { dir },
        } => board::run_board_check(&dir, cli.library.as_deref()),
        Command::Block { action } => block::run_block(action, cli.library.as_deref()),
        Command::Convert { project } => project::run_convert(&project, cli.library.as_deref()),
        Command::Panel {
            project,
            lang,
            format,
            out,
            save_numbers,
            csv,
        } => project::run_panel(
            &project,
            cli.library.as_deref(),
            &lang,
            &format,
            out.as_deref(),
            save_numbers,
            csv.as_deref(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_build::placement::DemoteReason;
    use crab_build::upload_plan::{DemotedSheet, StopReport, UploadReport};
    use wasm::{report_verdict, stop_verdict};

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
