//! `crablogic bench`: uploads test stands to the board one by one, reads the numbers the master
//! reports in `list`, and writes the report (`bench/<chip>-<master>-<date>.{toml,md}`).
//! `crablogic bench --compare a.toml b.toml` shows what got faster or slower.
//!
//! Spec: `specs/mcu-bench`. The board protocol is the existing one; the master is not changed.

mod listing;
mod modbus;
mod report;
mod secrets;
mod stands;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crab_build::mcu_module::{self as mm, MasterInfo};
use crab_codegen::{Program, WasmModule};
use crab_core::{Library, Project};

use listing::{ListAnswer, ModuleLine};
use report::{BenchReport, Meta, ModbusResult, RestartTimes, SheetsFit, StandResult, StepFigures};
use secrets::Secrets;
use stands::{Chip, Family, Mode, Stand};

/// Parameters of `crablogic bench`.
pub struct BenchArgs {
    pub port: Option<String>,
    pub only: Vec<String>,
    pub out: PathBuf,
    pub secrets: PathBuf,
    pub chip: Option<String>,
    pub quick: bool,
    /// Address of the board for the Modbus stand (else read from the master's journal).
    pub modbus_host: Option<String>,
}

/// How long the stands run before and while they are read.
#[derive(Debug, Clone, Copy)]
struct Timing {
    /// The module runs this long before the figures are taken.
    warm: Duration,
    /// Pause between the three readings.
    gap: Duration,
}

impl Timing {
    fn new(quick: bool) -> Self {
        if quick {
            Self {
                warm: Duration::from_secs(2),
                gap: Duration::from_secs(1),
            }
        } else {
            Self {
                warm: Duration::from_secs(10),
                gap: Duration::from_secs(2),
            }
        }
    }
}

const SAMPLES: usize = 3;

/// `bench --compare`.
pub fn run_compare(a: &Path, b: &Path) -> ExitCode {
    let read = |path: &Path| -> Result<BenchReport, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("не прочитан {}: {e}", path.display()))?;
        BenchReport::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))
    };
    match (read(a), read(b)) {
        (Ok(a), Ok(b)) => {
            print!("{}", report::render_compare(&a, &b));
            ExitCode::SUCCESS
        }
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("ошибка: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Ctx {
    serial: Box<dyn serialport::SerialPort>,
    info: MasterInfo,
    lib: Library,
    chip: Chip,
    secrets: Option<Secrets>,
    modbus_host: Option<std::net::IpAddr>,
    timing: Timing,
    /// Cache of AOT images (`.crab/` is not in git).
    cache: PathBuf,
}

/// `bench` against a board.
pub fn run(args: BenchArgs, library: Option<&Path>) -> ExitCode {
    match run_inner(args, library) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ошибка: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: BenchArgs, library: Option<&Path>) -> Result<(), String> {
    let lib = load_library(library)?;
    let secrets = Secrets::load(&args.secrets)?;
    if secrets.is_none() {
        println!(
            "Нет {}: сетевые замеры будут пропущены (образец — bench/secrets.example.toml).",
            args.secrets.display()
        );
    }
    let modbus_host = args
        .modbus_host
        .as_deref()
        .map(|h| {
            h.trim()
                .parse::<std::net::IpAddr>()
                .map_err(|_| "--modbus-host: нужен IP-адрес, например 192.168.1.50".to_owned())
        })
        .transpose()?;
    let (mut serial, port) = crate::open_master_port(args.port.clone())?;
    let info =
        mm::wait_for_master_info(&mut serial, Duration::from_secs(8)).map_err(|e| e.to_string())?;
    let chip = match args.chip.as_deref() {
        Some(name) => Chip::from_name(name).ok_or("--chip: esp32 или esp32c3")?,
        None => match info.aot_target() {
            Some(crab_build::wamrc::AotTarget::XtensaEsp32) => Chip::Esp32,
            Some(crab_build::wamrc::AotTarget::Riscv32Imc) => Chip::Esp32c3,
            None => return Err("не удалось определить чип по мастеру: укажите --chip".into()),
        },
    };
    let timing = Timing::new(args.quick);
    let mut ctx = Ctx {
        serial,
        info,
        lib,
        chip,
        secrets,
        modbus_host,
        timing,
        cache: PathBuf::from(".crab").join("bench").join("aot"),
    };
    let first = listing::query_list(&mut ctx.serial)?;
    let master = first.id.as_deref().map_or_else(
        || "unknown".to_owned(),
        |id| id.trim_start_matches("rs-").to_owned(),
    );
    println!(
        "Плата на {port}: {}, мастер {master}. Все модули на плате будут заменены стендами.",
        chip.name()
    );

    let selected = select_stands(&args.only);
    if selected.is_empty() {
        return Err("под --only не подошёл ни один стенд".into());
    }
    let mut report = BenchReport {
        meta: Meta {
            chip: chip.name().to_owned(),
            master,
            date: utc_now(),
            crablogic: env!("CARGO_PKG_VERSION").to_owned(),
            aot_target: format!("{:?}", ctx.info.aot_target()),
            warm_secs: timing.warm.as_secs() as u32,
            gap_secs: timing.gap.as_secs() as u32,
        },
        ..BenchReport::default()
    };
    let started = Instant::now();
    for (i, stand) in selected.iter().enumerate() {
        println!("[{}/{}] {}", i + 1, selected.len(), stand.id);
        match stand.family {
            Family::Sheets => report.sheets = Some(run_sheets(&mut ctx)),
            Family::Restart => {
                let (result, times) = run_restart(&mut ctx, stand);
                println!("    {}", summary(&result));
                report.stands.push(result);
                report.restart = times;
            }
            Family::ModbusTcp => {
                let (result, modbus) = run_modbus(&mut ctx, stand);
                println!("    {}", summary(&result));
                if let Some(m) = &modbus {
                    println!("    {}", modbus_summary(m));
                }
                note_skipped(&mut report, &result);
                report.stands.push(result);
                report.modbus = modbus;
            }
            _ => {
                let result = run_stand(&mut ctx, stand);
                println!("    {}", summary(&result));
                note_skipped(&mut report, &result);
                report.stands.push(result);
            }
        }
    }
    // Leave the board without the stand modules (`CRAB_BENCH_KEEP=1` keeps the last stand for a
    // look at the board by hand).
    if std::env::var_os("CRAB_BENCH_KEEP").is_none() {
        let _ = clear_board(&mut ctx);
    }
    println!("Прогон занял {} с", started.elapsed().as_secs());
    write_report(&args.out, &report)
}

/// A stand that could not run for a reason that is not the board's goes to the notes.
fn note_skipped(report: &mut BenchReport, result: &StandResult) {
    if result.status == "skipped" {
        if let Some(reason) = &result.reason {
            report.notes.push(format!("{}: {reason}", result.id));
        }
    }
}

fn load_library(explicit: Option<&Path>) -> Result<Library, String> {
    let here = std::env::current_dir().unwrap_or_default();
    let dir = explicit
        .map(Path::to_path_buf)
        .or_else(|| crate::find_library(&here))
        .ok_or("не найдена папка библиотеки блоков `library/`; укажите её через --library")?;
    let (lib, errors) = Library::load_with_user(&dir).map_err(|e| e.to_string())?;
    for e in errors {
        eprintln!("предупреждение: {e}");
    }
    Ok(lib)
}

/// Stands for `--only`: a token is an id, an id prefix or a family. Call stands bring the empty
/// stand, which their cost is measured against.
fn select_stands(only: &[String]) -> Vec<Stand> {
    let all = stands::standard_stands();
    let mut selected: Vec<Stand> = if only.is_empty() {
        all.clone()
    } else {
        all.iter()
            .filter(|s| {
                only.iter()
                    .any(|t| s.id == *t || is_id_prefix(&s.id, t) || s.family.name() == t)
            })
            .cloned()
            .collect()
    };
    if selected.iter().any(|s| s.family.is_call())
        && !selected.iter().any(|s| s.family == Family::Empty)
    {
        if let Some(empty) = all.iter().find(|s| s.family == Family::Empty) {
            selected.insert(0, empty.clone());
        }
    }
    selected
}

/// `logic_10` is a prefix of `logic_10_aot` but not of `logic_100_aot`.
fn is_id_prefix(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('_'))
}

fn summary(r: &StandResult) -> String {
    match r.status.as_str() {
        "ok" => format!(
            "шаг {:.1} мкс (макс. {} мкс), режим {}",
            r.mean_us.unwrap_or(0.0),
            r.max_us.map_or("—".to_owned(), |v| v.to_string()),
            r.actual_mode.as_deref().unwrap_or("—")
        ),
        status => format!("{status}: {}", r.reason.as_deref().unwrap_or("")),
    }
}

fn write_report(out: &Path, report: &BenchReport) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("не создана папка {}: {e}", out.display()))?;
    let stem = report.file_stem();
    let toml_path = out.join(format!("{stem}.toml"));
    let md_path = out.join(format!("{stem}.md"));
    std::fs::write(&toml_path, report.to_toml()?)
        .map_err(|e| format!("не записан {}: {e}", toml_path.display()))?;
    std::fs::write(&md_path, report.to_markdown())
        .map_err(|e| format!("не записан {}: {e}", md_path.display()))?;
    println!(
        "Отчёт: {}\n       {}",
        md_path.display(),
        toml_path.display()
    );
    Ok(())
}

/// `YYYY-MM-DD HH:MM` in UTC (no time zone database at hand).
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_utc(secs)
}

fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

// ---- stands: project -> modules -------------------------------------------------------------

/// Check, then every sheet of the project as a WASM module.
fn generate_modules(project: &Project, lib: &Library) -> Result<Vec<WasmModule>, String> {
    let report = crab_codegen::check_project(project, lib);
    if let Some(error) = report.errors().next() {
        return Err(format!("схема стенда с ошибкой: {}", error.message));
    }
    let project = crab_codegen::prepare(project, lib);
    let program = Program::build(&project, lib).map_err(|e| e.to_string())?;
    program
        .sheets()
        .into_iter()
        .map(|n| {
            program
                .wasm_module_with(n, false)
                .map_err(|e| format!("лист {n}: {e}"))
        })
        .collect()
}

fn allow_of(module: &WasmModule) -> Vec<String> {
    crab_sim::mcu::Manifest::parse(&module.manifest)
        .map(|m| m.allow)
        .unwrap_or_default()
}

/// Why a stand has no figures.
enum Outcome {
    /// Cannot run here (no AOT, no secrets): not a fault of the board.
    Skipped(String),
    Failed(String),
}

struct Uploaded {
    wasm_bytes: u32,
    image_bytes: u32,
    upload_ms: u32,
}

/// Removes every module from the board.
fn clear_board(ctx: &mut Ctx) -> Result<(), String> {
    let list = listing::query_list(&mut ctx.serial)?;
    for module in &list.modules {
        mm::delete_module(&mut ctx.serial, &module.name).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Sends the module as asked: `.wasm` for the interpreter, the AOT image for AOT. The time is the
/// transfer and the master storing it, not the AOT compiler.
fn upload(ctx: &mut Ctx, module: &WasmModule, mode: Mode) -> Result<Uploaded, Outcome> {
    let missing = ctx.info.missing_caps(&module.needs);
    if !missing.is_empty() {
        return Err(Outcome::Skipped(format!(
            "мастеру не хватает: {}",
            missing.join(", ")
        )));
    }
    let allow = allow_of(module);
    let name = module.name.as_str();
    let rejected = |e: mm::ModuleUploadError| match e {
        mm::ModuleUploadError::Rejected(why) => Outcome::Failed(format!("не влезло: {why}")),
        other => Outcome::Failed(other.to_string()),
    };
    match mode {
        Mode::Interp => {
            let started = Instant::now();
            mm::upload_module(&mut ctx.serial, name, &module.wasm, &allow).map_err(rejected)?;
            Ok(Uploaded {
                wasm_bytes: module.wasm.len() as u32,
                image_bytes: module.wasm.len() as u32,
                upload_ms: started.elapsed().as_millis() as u32,
            })
        }
        Mode::Aot => {
            if !ctx.info.takes_aot() {
                return Err(Outcome::Skipped("мастер на плате не принимает AOT".into()));
            }
            let never_cancelled = std::sync::atomic::AtomicBool::new(false);
            let image = mm::module_image(&ctx.info, &module.wasm_aot, &ctx.cache, &mut |target| {
                crab_build::wamrc::ensure_for(
                    target,
                    None,
                    &never_cancelled,
                    &mut |line| println!("    {line}"),
                    &mut |_, _| {},
                )
            })
            .map_err(|e| Outcome::Failed(e.to_string()))?;
            if !image.aot {
                return Err(Outcome::Skipped(
                    image
                        .warning
                        .unwrap_or_else(|| "AOT-образ не собран".to_owned()),
                ));
            }
            let mem = mm::module_memory(&module.wasm)
                .ok_or_else(|| Outcome::Failed(mm::ModuleUploadError::OldModule.to_string()))?;
            let started = Instant::now();
            mm::upload_image(&mut ctx.serial, name, &image.bytes, mem, &allow).map_err(rejected)?;
            Ok(Uploaded {
                wasm_bytes: module.wasm.len() as u32,
                image_bytes: image.bytes.len() as u32,
                upload_ms: started.elapsed().as_millis() as u32,
            })
        }
    }
}

fn run_stand(ctx: &mut Ctx, stand: &Stand) -> StandResult {
    let (id, family, mode) = (stand.id.as_str(), stand.family.name(), stand.mode.name());
    let fail = |reason: String| StandResult::failed(id, family, stand.n, mode, reason);
    let built = match stands::build_project(stand, ctx.chip, ctx.secrets.as_ref()) {
        Ok(built) => built,
        Err(reason) => return StandResult::skipped(id, family, stand.n, mode, reason),
    };
    let modules = match generate_modules(&built.project, &ctx.lib) {
        Ok(modules) => modules,
        Err(reason) => return fail(reason),
    };
    let Some(module) = modules.first() else {
        return fail("в стенде нет листов".into());
    };
    if let Err(e) = clear_board(ctx) {
        return fail(e);
    }
    let uploaded = match upload(ctx, module, stand.mode) {
        Ok(uploaded) => uploaded,
        Err(Outcome::Skipped(reason)) => {
            return StandResult::skipped(id, family, stand.n, mode, reason)
        }
        Err(Outcome::Failed(reason)) => return fail(reason),
    };
    let warm = if matches!(stand.family, Family::Net | Family::ModbusTcp) {
        // Wi-Fi joins (and MQTT connects) first.
        ctx.timing.warm * 2
    } else {
        ctx.timing.warm
    };
    let mut result = StandResult {
        id: id.to_owned(),
        family: family.to_owned(),
        n: stand.n,
        blocks: built.blocks,
        calls: built.calls,
        mode: mode.to_owned(),
        status: "ok".into(),
        wasm_bytes: Some(uploaded.wasm_bytes),
        image_bytes: Some(uploaded.image_bytes),
        upload_ms: Some(uploaded.upload_ms),
        ..StandResult::default()
    };
    match measure(ctx, &module.name, warm) {
        Ok(figures) => figures.fill(&mut result),
        Err(reason) => {
            result.status = "failed".into();
            result.reason = Some(reason);
        }
    }
    result
}

// ---- reading the numbers --------------------------------------------------------------------

/// What three readings of `list` say about one module.
#[derive(Debug, Clone, PartialEq)]
struct Figures {
    steps_per_s: f64,
    mean_us: Option<f64>,
    last_us: Option<f64>,
    max_us: Option<u32>,
    gap_us: Option<u32>,
    spread_us: Option<u32>,
    mode: Option<String>,
    free: Option<u32>,
    largest: Option<u32>,
    min_free: Option<u32>,
    module_mem: Option<u32>,
    module_code: Option<u32>,
}

impl Figures {
    fn fill(self, r: &mut StandResult) {
        r.steps_per_s = Some(self.steps_per_s);
        r.mean_us = self.mean_us;
        r.last_us = self.last_us;
        r.max_us = self.max_us;
        r.gap_us = self.gap_us;
        r.spread_us = self.spread_us;
        r.actual_mode = self.mode;
        r.free = self.free;
        r.largest = self.largest;
        r.min_free = self.min_free;
        r.module_mem = self.module_mem;
        r.module_code = self.module_code;
    }
}

/// Figures from readings taken at the given times. The first reading is the start of the window:
/// the mean step is the growth of the board's sum of step times over the steps made since.
fn figures_of(readings: &[(Instant, ListAnswer)], name: &str) -> Result<Figures, String> {
    let lines: Vec<(Instant, &ModuleLine)> = readings
        .iter()
        .filter_map(|(at, a)| Some((*at, a.module(name)?)))
        .collect();
    if lines.len() != readings.len() || lines.len() < 2 {
        return Err("модуль пропал с платы во время замера".into());
    }
    let (t0, first) = lines[0];
    let (t1, last) = lines[lines.len() - 1];
    if lines.iter().any(|(_, m)| !m.running) {
        return Err(
            "модуль остановился во время замера (подробнее — журнал мастера, команда log)".into(),
        );
    }
    if lines.iter().any(|(_, m)| m.restarts != first.restarts) {
        return Err("модуль перезапускался во время замера".into());
    }
    let steps = last.steps.wrapping_sub(first.steps);
    if steps == 0 || last.steps < first.steps {
        return Err("модуль не делает шагов".into());
    }
    let secs = t1.duration_since(t0).as_secs_f64().max(0.001);
    let us: Vec<[u32; 4]> = lines.iter().filter_map(|(_, m)| m.us).collect();
    let (mean_us, last_us, max_us, gap_us, spread_us) = if us.len() == lines.len() {
        let sum = us[us.len() - 1][3].wrapping_sub(us[0][3]);
        let lasts: Vec<u32> = us.iter().map(|u| u[0]).collect();
        let tail = us[us.len() - 1];
        (
            Some(f64::from(sum) / f64::from(steps)),
            Some(lasts.iter().map(|&v| f64::from(v)).sum::<f64>() / lasts.len() as f64),
            Some(tail[1]),
            Some(tail[2]),
            Some(
                lasts.iter().max().copied().unwrap_or(0) - lasts.iter().min().copied().unwrap_or(0),
            ),
        )
    } else {
        (None, None, None, None, None)
    };
    let end = &readings[readings.len() - 1].1;
    Ok(Figures {
        steps_per_s: f64::from(steps) / secs,
        mean_us,
        last_us,
        max_us,
        gap_us,
        spread_us,
        mode: last.mode.clone(),
        free: end.free,
        largest: end.largest,
        min_free: end.min_free,
        module_mem: last.mem,
        module_code: last.code,
    })
}

/// Waits `warm`, resets the maxima of the board, takes [`SAMPLES`] readings `gap` apart.
fn measure(ctx: &mut Ctx, name: &str, warm: Duration) -> Result<Figures, String> {
    std::thread::sleep(warm);
    // The longest step and gap since the start include the upload; the window starts here.
    listing::command(&mut ctx.serial, "rmax")?;
    let mut readings = Vec::new();
    for i in 0..SAMPLES {
        if i > 0 {
            std::thread::sleep(ctx.timing.gap);
        }
        let answer = listing::query_list(&mut ctx.serial)?;
        readings.push((Instant::now(), answer));
    }
    figures_of(&readings, name)
}

// ---- Modbus TCP -----------------------------------------------------------------------------

/// How long the board may take to join Wi-Fi and tell its address.
const WIFI_WAIT: Duration = Duration::from_secs(30);

fn modbus_summary(m: &ModbusResult) -> String {
    match m.mean_ms {
        Some(mean) => format!(
            "Modbus: {} запросов, ошибок {}, ответ {mean:.2} мс (95 % {:.2} мс), {:.1} запр./с",
            m.requests,
            m.errors,
            m.p95_ms.unwrap_or(0.0),
            m.req_per_s.unwrap_or(0.0)
        ),
        None => format!(
            "Modbus: нет цифр: {}",
            m.note.as_deref().unwrap_or("нет ответов")
        ),
    }
}

/// The address from the master's journal (`wifi ready, address <ip>`), waiting for Wi-Fi.
fn wait_wifi_address(
    serial: &mut Box<dyn serialport::SerialPort>,
    within: Duration,
) -> Option<std::net::Ipv4Addr> {
    let started = Instant::now();
    loop {
        if let Ok(lines) = listing::query_log(serial) {
            if let Some(ip) = listing::wifi_address(lines.iter().map(String::as_str)) {
                return Some(ip);
            }
        }
        if started.elapsed() >= within {
            return None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// The step figures of the stand, then the same measured while the PC polls the board.
fn run_modbus(ctx: &mut Ctx, stand: &Stand) -> (StandResult, Option<ModbusResult>) {
    let mut result = run_stand(ctx, stand);
    if !result.is_ok() {
        return (result, None);
    }
    let (id, family, mode) = (stand.id.as_str(), stand.family.name(), stand.mode.name());
    let skip = |reason: &str| StandResult::skipped(id, family, stand.n, mode, reason.to_owned());
    let name = match listing::query_list(&mut ctx.serial) {
        Ok(list) => list.modules.first().map(|m| m.name.clone()),
        Err(_) => None,
    };
    let Some(name) = name else {
        return (skip("модуль пропал после замера"), None);
    };
    let host = match ctx.modbus_host {
        Some(host) => host,
        None => match wait_wifi_address(&mut ctx.serial, WIFI_WAIT) {
            Some(ip) => std::net::IpAddr::V4(ip),
            None => {
                return (
                    skip("плата не подключилась к Wi-Fi за 30 с (нет адреса в журнале мастера); можно указать --modbus-host"),
                    None,
                )
            }
        },
    };
    let addr = std::net::SocketAddr::new(host, 502);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let poller = {
        let stop = std::sync::Arc::clone(&stop);
        std::thread::spawn(move || modbus::poll(addr, modbus::REQUESTS, &stop))
    };
    // The polling goes on while the sheet is measured; `measure` resets the maxima itself.
    let polled = measure(ctx, &name, Duration::from_millis(500));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Ok(poll) = poller.join() else {
        result.status = "failed".into();
        result.reason = Some("опрос Modbus завершился аварийно".into());
        return (result, None);
    };
    let stats = modbus::latency_stats(&poll.samples_ms);
    let secs = poll.elapsed.as_secs_f64();
    let mut m = ModbusResult {
        requests: poll.requests,
        errors: poll.errors,
        mean_ms: stats.map(|s| s.mean_ms),
        p95_ms: stats.map(|s| s.p95_ms),
        max_ms: stats.map(|s| s.max_ms),
        req_per_s: (secs > 0.0 && stats.is_some()).then(|| poll.requests as f64 / secs),
        note: poll.first_error.as_ref().map(|e| {
            if poll.gave_up {
                format!("Опрос остановлен: плата не отвечает ({e}).")
            } else {
                format!("Первая ошибка: {e}.")
            }
        }),
        polled: None,
    };
    match polled {
        Ok(f) => {
            m.polled = Some(StepFigures {
                mean_us: f.mean_us,
                max_us: f.max_us,
                gap_us: f.gap_us,
                spread_us: f.spread_us,
            });
        }
        Err(reason) => {
            let why = format!("Шаг во время опроса не замерен: {reason}.");
            m.note = Some(match m.note.take() {
                Some(old) => format!("{old} {why}"),
                None => why,
            });
        }
    }
    (result, Some(m))
}

// ---- memory and restarts --------------------------------------------------------------------

/// Adds copies of a small sheet one by one until the board refuses or stops one.
fn run_sheets(ctx: &mut Ctx) -> SheetsFit {
    let mut fit = SheetsFit {
        tried: stands::MAX_SHEETS,
        ..SheetsFit::default()
    };
    let project = stands::sheets_project(ctx.chip, stands::MAX_SHEETS);
    let modules = match generate_modules(&project, &ctx.lib) {
        Ok(modules) => modules,
        Err(e) => {
            fit.reason = Some(e);
            return fit;
        }
    };
    if let Err(e) = clear_board(ctx) {
        fit.reason = Some(e);
        return fit;
    }
    let mode = if ctx.info.takes_aot() {
        Mode::Aot
    } else {
        Mode::Interp
    };
    for (i, module) in modules.iter().enumerate() {
        fit.tried = i + 1;
        match upload(ctx, module, mode) {
            Ok(_) => {}
            Err(Outcome::Skipped(e) | Outcome::Failed(e)) => {
                fit.reason = Some(e);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        let list = match listing::query_list(&mut ctx.serial) {
            Ok(list) => list,
            Err(e) => {
                fit.reason = Some(e);
                break;
            }
        };
        let running = list.modules.iter().filter(|m| m.running).count();
        fit.free_after.push(list.free.unwrap_or(0));
        if running < i + 1 {
            fit.reason = Some(format!("лист {} не запустился", i + 1));
            break;
        }
        fit.fit = running;
    }
    fit
}

/// Time from the restart of the module, and from the reset of the board, to the module's first
/// step. The stand is the logic sheet of 100 blocks.
fn run_restart(ctx: &mut Ctx, stand: &Stand) -> (StandResult, Option<RestartTimes>) {
    let mut result = run_stand(ctx, stand);
    if !result.is_ok() {
        return (result, None);
    }
    // The module keeps its own steps: after a restart its counter starts from zero.
    let name = match listing::query_list(&mut ctx.serial) {
        Ok(list) => list.modules.first().map(|m| m.name.clone()),
        Err(_) => None,
    };
    let Some(name) = name else {
        result.status = "failed".into();
        result.reason = Some("модуль пропал после замера".into());
        return (result, None);
    };
    let mut times = RestartTimes::default();
    let started = Instant::now();
    match listing::command(&mut ctx.serial, &format!("swap {name}")) {
        Ok(()) => {
            times.module_ms =
                wait_first_step(&mut ctx.serial, &name, started, Duration::from_secs(10));
        }
        Err(e) => times.note = Some(format!("swap: {e}.")),
    }
    // Reset by the RTS line (the EN pin of a dev board); the DTR line stays released so the
    // board boots the program.
    let before = listing::query_list(&mut ctx.serial)
        .ok()
        .and_then(|l| l.module(&name).map(|m| m.steps))
        .unwrap_or(u32::MAX);
    let _ = ctx.serial.write_data_terminal_ready(false);
    let _ = ctx.serial.write_request_to_send(true);
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    let _ = ctx.serial.write_request_to_send(false);
    times.boot_ms = wait_boot(
        &mut ctx.serial,
        &name,
        before,
        started,
        Duration::from_secs(20),
    );
    if times.boot_ms.is_none() {
        let note = "Сброс по линии RTS не сработал на этой плате (нет ответа мастера за 20 с).";
        times.note = Some(match times.note.take() {
            Some(old) => format!("{old} {note}"),
            None => note.to_owned(),
        });
    }
    (result, Some(times))
}

/// Polls `list` until the module has made a step.
fn wait_first_step<P: std::io::Read + std::io::Write>(
    port: &mut P,
    name: &str,
    started: Instant,
    within: Duration,
) -> Option<u32> {
    while started.elapsed() < within {
        if let Ok(list) = listing::query_list(port) {
            if list.module(name).is_some_and(|m| m.running && m.steps >= 1) {
                return Some(started.elapsed().as_millis() as u32);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Like [`wait_first_step`] after a reset: the board is silent while it boots, and a module whose
/// counter is not below `before` is the old run.
fn wait_boot<P: std::io::Read + std::io::Write>(
    port: &mut P,
    name: &str,
    before: u32,
    started: Instant,
    within: Duration,
) -> Option<u32> {
    while started.elapsed() < within {
        if let Ok(list) = listing::query_list(port) {
            if list
                .module(name)
                .is_some_and(|m| m.running && m.steps >= 1 && m.steps < before)
            {
                return Some(started.elapsed().as_millis() as u32);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[cfg(test)]
mod tests;
