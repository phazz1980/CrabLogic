//! Upload of the master firmware and of the sheet modules in the background, so the editor stays
//! responsive.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crab_build::BuildControl;
use crab_codegen::Program;
use crab_core::{Library, Project};

use crate::app::LogLevel;

pub enum JobKind {
    /// The prebuilt master firmware of the distribution onto the board.
    Upload { port: Option<String> },
    /// Sheets as WASM modules into the master firmware already on the board.
    UploadModules { port: Option<String> },
}

pub struct Done {
    pub message: String,
    /// Port the firmware was uploaded to.
    pub port: Option<String>,
    pub debug: bool,
}

pub enum JobEvent {
    Line(LogLevel, String),
    Done(Result<Done, String>),
    /// The upload waits for the user: some sheets cannot be accelerated. Answer with
    /// [`Job::answer`].
    Ask(UploadQuestion),
}

/// What an upload question is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// Some sheets cannot be accelerated.
    Demote,
    /// Some sheets stop while they are replaced in place.
    Stop,
}

/// One question for the whole upload (`specs/wasm-kernel`, decision 16).
pub struct UploadQuestion {
    pub kind: QuestionKind,
    pub message: String,
    /// Sheets that go without acceleration; «do not ask again» makes them background ones.
    pub sheets: Vec<u32>,
}

pub struct Job {
    pub title: &'static str,
    /// Upload: the serial port must stay free.
    pub uses_port: bool,
    control: Arc<BuildControl>,
    rx: Receiver<JobEvent>,
    /// Answers to [`JobEvent::Ask`]: `true` uploads anyway.
    answers: Sender<bool>,
}

impl Job {
    /// Starts an upload on a worker thread. `wake` is called after every event so the UI can
    /// repaint.
    /// `debug`: the modules report every block output; after the upload the editor connects.
    pub fn start(
        kind: JobKind,
        debug: bool,
        project: Project,
        lib: Library,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let title = match kind {
            JobKind::Upload { .. } => crate::i18n::tr("Заливка мастер-прошивки"),
            JobKind::UploadModules { .. } => crate::i18n::tr("Заливка модулей"),
        };
        let uses_port = true;
        let control = Arc::new(BuildControl::default());
        let worker = control.clone();
        let (tx, rx) = mpsc::channel();
        let (answers, answer_rx) = mpsc::channel();
        thread::spawn(move || {
            let send = |e: JobEvent| {
                let _ = tx.send(e);
                wake();
            };
            let cancel = worker.clone();
            let report = Report {
                out: &mut |level, line| send(JobEvent::Line(level, line.to_owned())),
                ask: &mut |question| {
                    send(JobEvent::Ask(question));
                    wait_answer(&answer_rx, &cancel)
                },
            };
            let result = run(kind, debug, &project, &lib, worker, report);
            send(JobEvent::Done(result));
        });
        Self {
            title,
            uses_port,
            control,
            rx,
            answers,
        }
    }

    /// Answers the question of the upload: `true` uploads anyway, `false` stops it.
    pub fn answer(&self, upload: bool) {
        let _ = self.answers.send(upload);
    }

    /// Stops the job; it ends with an error soon after.
    pub fn cancel(&self) {
        self.control.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    /// Events received since the last call.
    pub fn poll(&self) -> Vec<JobEvent> {
        self.rx.try_iter().collect()
    }
}

/// Where a job reports: the log, and the questions to the user.
struct Report<'a> {
    out: &'a mut dyn FnMut(LogLevel, &str),
    /// Asks the user and waits; `false` when declined or the job is cancelled.
    ask: &'a mut dyn FnMut(UploadQuestion) -> bool,
}

/// Waits for the user's answer, giving up when the job is cancelled or the editor is gone.
fn wait_answer(answers: &Receiver<bool>, control: &BuildControl) -> bool {
    loop {
        if control.is_cancelled() {
            return false;
        }
        match answers.recv_timeout(Duration::from_millis(100)) {
            Ok(answer) => return answer,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn run(
    kind: JobKind,
    debug: bool,
    project: &Project,
    lib: &Library,
    control: Arc<BuildControl>,
    report: Report,
) -> Result<Done, String> {
    let Report { out, ask } = report;
    if let JobKind::UploadModules { port } = &kind {
        return upload_modules(port.as_deref(), project, lib, debug, &control, out, ask);
    }
    // The upload puts the master on the board, with debugging on or off; the schema follows
    // as modules (debugging is a flag of the module build).
    match &kind {
        JobKind::Upload { port } => upload_master(port.as_deref(), project, lib, control, out),
        JobKind::UploadModules { .. } => unreachable!("handled above"),
    }
}

/// Flashes the prebuilt Rust master (`firmware/<chip>/` of the distribution): the schema itself
/// goes to the board as WASM modules (`upload_modules`).
fn upload_master(
    port: Option<&str>,
    project: &Project,
    _lib: &Library,
    _control: Arc<BuildControl>,
    out: &mut dyn FnMut(LogLevel, &str),
) -> Result<Done, String> {
    use crab_build::flash;
    let started = std::time::Instant::now();
    let board = &project.config.board;
    let dir = flash::firmware_dir().ok_or_else(|| {
        crate::i18n::tr("Не найдена папка firmware с образами мастера рядом с программой")
            .to_owned()
    })?;
    let image = flash::find_image(&dir, &board.fqbn).map_err(|e| e.to_string())?;
    let port = match port.map(str::to_owned).or_else(|| board.port.clone()) {
        Some(port) if crab_build::net_link::looks_like_address(&port) => {
            return Err(crate::i18n::tr(
                "Мастер-прошивка заливается только по USB — выберите COM-порт",
            )
            .to_owned());
        }
        Some(port) => port,
        None => flash::single_usb_port().map_err(|e| e.to_string())?,
    };
    out(
        LogLevel::Info,
        &crate::i18n::fmt("Заливка мастер-прошивки на {0}...", &[&port]),
    );
    flash::flash_master(&image, &port, &mut |l| out(LogLevel::Info, l))
        .map_err(|e| e.to_string())?;
    Ok(Done {
        message: crate::i18n::fmt(
            "Мастер-прошивка залита на {0} за {1} с",
            &[&port, &seconds(started)],
        ),
        port: Some(port),
        debug: false,
    })
}

/// Builds the WASM module of every sheet of the prepared project; returns the prepared project
/// too, with the modules and the sheet numbers in the same order.
pub fn build_sheets(
    project: &Project,
    lib: &Library,
    watch: bool,
) -> Result<(Project, Vec<crab_codegen::WasmModule>, Vec<u32>), String> {
    let project = crab_codegen::prepare(project, lib);
    let mut program = Program::build(&project, lib).map_err(|e| e.to_string())?;
    program
        .auto_group_for_board(watch)
        .map_err(|e| e.to_string())?;
    let mut modules = Vec::new();
    let mut numbers = Vec::new();
    for sheet in program.sheets() {
        let module = program
            .wasm_module_with(sheet, watch)
            .map_err(|e| crate::i18n::fmt("лист {0}: {1}", &[&sheet, &e]))?;
        modules.push(module);
        numbers.push(sheet);
    }
    Ok((project, modules, numbers))
}

/// What the project will cost the board, from its blocks and the modules the build makes of the
/// sheets (`wasm_lens`: sizes of those modules, for the upload time). `None` when the project
/// does not build.
pub fn estimate_text(
    project: &Project,
    lib: &Library,
    wasm_lens: &[usize],
) -> Option<crate::app::EstimateText> {
    let project = crab_codegen::prepare(project, lib);
    let mut program = Program::build(&project, lib).ok()?;
    program.auto_group_for_board(false).ok()?;
    let est = program.estimate();
    let kb = |bytes: i64| (bytes as f64 / 1024.0).round() as i64;
    let used = kb(i64::from(est.heap));
    let budget = kb(i64::from(est.budget));
    let over = !est.fits();
    let summary = if over {
        crate::i18n::fmt("ОЗУ платы: не хватает, ≈ {0} из {1} КБ", &[&used, &budget])
    } else {
        crate::i18n::fmt(
            "ОЗУ платы: ≈ {0} из {1} КБ, ещё ≈ {2} блоков",
            &[&used, &budget, &est.more_blocks],
        )
    };
    let mut details = vec![crate::i18n::fmt(
        "Модулей на плате: {0}, блоков: {1}",
        &[
            &est.modules.len(),
            &est.modules.iter().map(|m| m.blocks).sum::<usize>(),
        ],
    )];
    for m in &est.modules {
        let sheets = m
            .sheets
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        details.push(crate::i18n::fmt(
            "Модуль «{0}» (листы {1}): блоков {2}, ОЗУ ≈ {3} КБ, шаг ≈ {4} мкс раз в {5} мс",
            &[
                &m.name,
                &sheets,
                &m.blocks,
                &format!("{:.1}", f64::from(m.heap) / 1024.0),
                &m.step_us.round(),
                &m.period_ms,
            ],
        ));
    }
    details.push(crate::i18n::fmt(
        "Самый занятый период: ≈ {0}% (шаги модулей идут один за другим)",
        &[&est.busiest_percent().round()],
    ));
    let seconds: f64 = wasm_lens
        .iter()
        .map(|&len| {
            crab_build::upload_plan::stop_seconds(crab_build::upload_plan::estimated_aot_len(len))
        })
        .sum();
    if seconds > 0.0 {
        details.push(crate::i18n::fmt(
            "Заливка всех модулей займёт ≈ {0} с",
            &[&seconds.ceil()],
        ));
    }
    if est.network {
        details.push(crate::i18n::fmt(
            "Сеть (Wi-Fi, MQTT и др.) отнимает у проекта ≈ {0} КБ ОЗУ",
            &[&(kb(i64::from(crab_core::cost::heap_for(est.chip).net_stack)))],
        ));
    }
    if !over {
        details.push(crate::i18n::fmt(
            "Ещё поместится ≈ {0} блоков (логика, математика, остальное в пропорции 50/30/20) в уже занятых модулях",
            &[&est.more_blocks],
        ));
    }
    details.push(
        crate::i18n::tr("Оценка по замерам на платах; память под тексты не учтена.").to_owned(),
    );
    Some(crate::app::EstimateText {
        summary,
        over,
        details,
    })
}

/// Modules with their sheet numbers and speeds for the upload planning.
pub fn sheet_modules<'a>(
    project: &Project,
    modules: &'a [crab_codegen::WasmModule],
    numbers: &[u32],
) -> Vec<crab_build::upload_plan::SheetModule<'a>> {
    modules
        .iter()
        .zip(numbers)
        .map(|(module, &sheet)| crab_build::upload_plan::SheetModule {
            sheet,
            name: &module.name,
            wasm: &module.wasm,
            wasm_aot: &module.wasm_aot,
            speed: project
                .logic
                .sheets
                .iter()
                .find(|s| s.number == sheet)
                .map_or(crab_core::SheetSpeed::Normal, |s| s.speed),
        })
        .collect()
}

/// Builds every sheet as a WASM module and sends it to the master firmware on the board.
/// With `watch` the modules report every block output, so the debugger can show them.
fn upload_modules(
    port: Option<&str>,
    project: &Project,
    lib: &Library,
    watch: bool,
    control: &BuildControl,
    out: &mut dyn FnMut(LogLevel, &str),
    ask: &mut dyn FnMut(UploadQuestion) -> bool,
) -> Result<Done, String> {
    let started = std::time::Instant::now();
    let (project, modules, numbers) = build_sheets(project, lib, watch)?;
    let project = &project;
    let board = &project.config.board;
    let explicit = port.map(str::trim).filter(|p| !p.is_empty());
    // Without an explicit choice: the project's port if it is present, else its address.
    let link = match explicit {
        Some(text) => crab_build::net_link::BoardLink::parse(text),
        None => match crab_build::project_link(board)? {
            crab_build::Chosen::Serial(port) => crab_build::net_link::BoardLink::Serial(port),
            crab_build::Chosen::Net(host) => crab_build::net_link::BoardLink::Tcp(host),
        },
    };
    let (mut serial, port, over_net): (Box<dyn crab_build::net_link::Link>, String, bool) =
        match link {
            crab_build::net_link::BoardLink::Tcp(host) => {
                let stream = crab_build::net_link::connect(&host, &project.config.project.password)
                    .map_err(|e| e.to_string())?;
                (Box::new(stream), host, true)
            }
            crab_build::net_link::BoardLink::Serial(port) => {
                let mut serial = serialport::new(&port, 115_200)
                    .timeout(std::time::Duration::from_millis(200))
                    .open()
                    .map_err(|e| crate::i18n::fmt("порт {0} не открыт: {1}", &[&port, &e]))?;
                // Release DTR/RTS so opening the port does not reset the board.
                let _ = serial.write_data_terminal_ready(false);
                let _ = serial.write_request_to_send(false);
                (Box::new(serial), port, false)
            }
        };
    let info = crab_build::mcu_module::wait_for_master_info(
        &mut serial,
        std::time::Duration::from_secs(8),
    )
    .map_err(|e| e.to_string())?;
    // Modules are built for the Rust master only (`specs/rust-only`).
    let image = crab_build::flash::image_version(&project.config.board.fqbn);
    match crab_build::flash::master_verdict(info.id.as_deref(), image.as_deref()) {
        crab_build::flash::MasterVerdict::NotRust => {
            return Err(crate::i18n::tr(
                "На плате не мастер на Rust (старый C++-мастер или другая прошивка): залейте мастер кнопкой ⬆, затем модули",
            )
            .to_owned());
        }
        crab_build::flash::MasterVerdict::Older(have, want) => out(
            LogLevel::Warning,
            &crate::i18n::fmt(
                "Мастер на плате версии {0}, в программе {1}: залейте мастер кнопкой ⬆",
                &[&have, &want],
            ),
        ),
        crab_build::flash::MasterVerdict::Current => {}
    }
    // A module that uses what only a newer master can do would only fail on the board.
    for module in &modules {
        let missing = info.missing_caps(&module.needs);
        if !missing.is_empty() {
            return Err(crab_build::mcu_module::missing_caps_message(
                &module.name,
                &missing,
            ));
        }
    }
    // An old master still runs the modules, but out of RAM: the memory left is small.
    if info.keeps_modules_in_ram() {
        out(
            LogLevel::Warning,
            crate::i18n::tr(crab_build::mcu_module::OLD_STORAGE_MESSAGE),
        );
    }
    // Safe mode after a boot loop: the stored modules of the board are stopped; the uploaded ones run.
    if info.safe_boots > 0 {
        out(LogLevel::Warning, &safe_mode_message(info.safe_boots));
    }
    // Which sheets go accelerated is decided for the whole upload at once (`specs/wasm-kernel`, 3.8–3.9).
    let space = if info.takes_aot() {
        crab_build::upload_plan::read_crabmod_space(&mut serial).map_err(|e| e.to_string())?
    } else {
        None
    };
    let sheets = sheet_modules(project, &modules, &numbers);
    let plan = crab_build::upload_plan::plan_upload(
        &info,
        space.as_ref(),
        &sheets,
        &project.work_dir().join("aot"),
        &mut |target| {
            let wamrc = crate::theme::wamrc_path();
            let manual = (!wamrc.is_empty()).then(|| std::path::PathBuf::from(wamrc));
            let mut shown = 0;
            // Both callbacks log; they never run at the same time.
            let log = std::cell::RefCell::new(&mut *out);
            crab_build::wamrc::ensure_for(
                target,
                manual.as_deref(),
                control.cancel_flag(),
                &mut |line| (log.borrow_mut())(LogLevel::Info, line),
                &mut |got, total| {
                    let percent = (got * 100).checked_div(total).unwrap_or(0);
                    if percent >= shown + 10 {
                        shown = percent / 10 * 10;
                        (log.borrow_mut())(
                            LogLevel::Info,
                            &crate::i18n::fmt("Скачано {0}%", &[&shown]),
                        );
                    }
                },
            )
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(report) = &plan.report {
        let message = report.message();
        out(LogLevel::Warning, &message);
        let question = UploadQuestion {
            kind: QuestionKind::Demote,
            message,
            sheets: report.demoted_sheets(),
        };
        if !ask(question) {
            return Err(crate::i18n::tr("Заливка отменена").to_owned());
        }
    }
    if let Some(stops) = &plan.stops {
        let message = stops.message();
        out(LogLevel::Warning, &message);
        let question = UploadQuestion {
            kind: QuestionKind::Stop,
            message,
            sheets: stops.sheets.iter().map(|s| s.sheet).collect(),
        };
        if !ask(question) {
            return Err(crate::i18n::tr("Заливка отменена").to_owned());
        }
    }
    // Modules of an earlier build that this one no longer has (a sheet joined into another
    // module, renamed or deleted) would run beside the new ones: remove them first.
    if let Some(space) = &space {
        let keep: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        for name in space.stale(&keep) {
            crab_build::mcu_module::delete_module(&mut serial, &name).map_err(|e| e.to_string())?;
            out(
                LogLevel::Warning,
                &crate::i18n::fmt(
                    "Модуль «{0}» убран с платы: его нет в новой сборке",
                    &[&name],
                ),
            );
        }
    }
    for ((module, image), &in_place) in modules.iter().zip(&plan.images).zip(&plan.in_place) {
        let allow = crab_sim::mcu::Manifest::parse(&module.manifest)
            .map(|m| m.allow)
            .unwrap_or_default();
        if image.aot {
            out(
                LogLevel::Info,
                &crate::i18n::fmt(
                    "Модуль «{0}»: AOT {1} байт (WASM {2} байт) -> {3}...",
                    &[&module.name, &image.bytes.len(), &module.wasm.len(), &port],
                ),
            );
        } else {
            out(
                LogLevel::Info,
                &crate::i18n::fmt(
                    "Модуль «{0}» ({1} байт) -> {2}...",
                    &[&module.name, &module.wasm.len(), &port],
                ),
            );
        }
        let mem = crab_build::mcu_module::module_memory(&module.wasm)
            .ok_or(crab_build::mcu_module::ModuleUploadError::OldModule)
            .map_err(|e| e.to_string())?;
        if in_place {
            crab_build::mcu_module::delete_module(&mut serial, &module.name)
                .map_err(|e| e.to_string())?;
            out(
                LogLevel::Warning,
                &crate::i18n::fmt(
                    "Модуль «{0}» остановлен для замены на месте",
                    &[&module.name],
                ),
            );
        }
        let module_started = std::time::Instant::now();
        crab_build::mcu_module::upload_image(&mut serial, &module.name, &image.bytes, mem, &allow)
            .map_err(|e| e.to_string())?;
        out(
            LogLevel::Info,
            &crate::i18n::fmt(
                "Модуль «{0}» залит за {1} с",
                &[&module.name, &seconds(module_started)],
            ),
        );
    }
    // The project name for the network search; an old master that does not know it is skipped.
    let _ = crab_build::mcu_module::set_board_name(&mut serial, &project.config.project.name);
    // Remember the board for the check panel; a failure here never fails the upload.
    if info.takes_aot() {
        match crab_build::upload_plan::read_crabmod_space(&mut serial) {
            Ok(Some(space)) => {
                let mut memo = crab_build::board_memo::BoardMemo::new(&info, &space);
                memo.modules = modules
                    .iter()
                    .zip(&plan.images)
                    .map(|(m, image)| crab_build::board_memo::MemoModule {
                        name: m.name.clone(),
                        wasm_crc: crab_build::mcu_module::crc32(&m.wasm),
                        image_crc: crab_build::mcu_module::crc32(&image.bytes),
                    })
                    .collect();
                if let Err(e) = crab_build::board_memo::save(&project.work_dir(), &memo) {
                    out(
                        LogLevel::Info,
                        &crate::i18n::fmt("Не удалось запомнить состояние платы: {0}", &[&e]),
                    );
                }
            }
            Ok(None) => {}
            Err(e) => out(
                LogLevel::Info,
                &crate::i18n::fmt("Не удалось запомнить состояние платы: {0}", &[&e]),
            ),
        }
    }
    Ok(Done {
        message: crate::i18n::fmt(
            "Модули залиты на {0} за {1} с (со сборкой)",
            &[&port, &seconds(started)],
        ),
        // The debugger connects to the same link after the upload.
        port: Some(if over_net {
            format!("tcp:{port}")
        } else {
            port
        }),
        debug: watch,
    })
}

/// Warning about a master in safe mode after `bad` resets in a row by a watchdog or a panic.
/// Seconds since `started`, one decimal: for the messages of an upload.
fn seconds(started: std::time::Instant) -> String {
    format!("{:.1}", started.elapsed().as_secs_f32())
}

pub fn safe_mode_message(bad: u32) -> String {
    crate::i18n::tr(
        "Мастер в безопасном режиме: после {0} сбоев подряд сохранённые модули не запущены. Залейте исправленные листы или сбросьте плату кнопкой",
    )
    .replace("{0}", &bad.to_string())
}

/// Flash wear line of the master for the debug status, and whether it is a warning
/// (more than `NVS_HOUR_WARN` writes in the last hour).
pub fn nvs_message(n: &crab_debug::protocol::NvsStats) -> (String, bool) {
    if n.hour > crab_debug::protocol::NVS_HOUR_WARN {
        let text = crate::i18n::fmt(
            "Частая запись во флеш: {0} за час — флеш изнашивается. Проверьте переменные с сохранением.",
            &[&n.hour],
        );
        return (text, true);
    }
    let mut text = crate::i18n::fmt("Флеш: записей {0}, за час {1}", &[&n.writes, &n.hour]);
    if let Some(free) = n.free {
        text.push_str(&crate::i18n::fmt(", свободно {0} записей", &[&free]));
    }
    (text, false)
}

#[cfg(test)]
mod nvs_message_tests {
    use super::nvs_message;
    use crab_debug::protocol::NvsStats;

    #[test]
    fn calm_and_frequent_writes() {
        let calm = NvsStats {
            writes: 12,
            hour: 3,
            used: Some(50),
            free: Some(450),
        };
        let (text, warn) = nvs_message(&calm);
        assert!(!warn);
        assert!(text.contains("12") && text.contains("450"));
        let busy = NvsStats {
            writes: 500,
            hour: 61,
            used: None,
            free: None,
        };
        let (text, warn) = nvs_message(&busy);
        assert!(warn);
        assert!(text.contains("61"));
    }
}

#[cfg(test)]
mod estimate_tests {
    use super::estimate_text;
    use crab_core::{Library, Project};

    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The panel text of a small project: the status line, a line per module, the time of the
    /// upload and the room left.
    #[test]
    fn the_estimate_of_a_small_project_is_written_out() {
        let lib = Library::load(root().join("library")).unwrap().0;
        let project = Project::load(root().join("examples/background")).unwrap();
        let text = estimate_text(&project, &lib, &[800, 800]).expect("the example builds");
        assert!(!text.over);
        assert!(text.summary.starts_with("ОЗУ платы: ≈"), "{}", text.summary);
        assert!(text.summary.contains("ещё ≈"), "{}", text.summary);
        let all = text.details.join("\n");
        assert!(all.contains("Модуль «"), "{all}");
        assert!(all.contains("Заливка всех модулей"), "{all}");
        assert!(all.contains("Самый занятый период"), "{all}");
        assert!(all.contains("Ещё поместится"), "{all}");
    }
}
