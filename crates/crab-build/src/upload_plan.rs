//! Plan of uploading the sheets of a project to a master: which image (AOT or plain `.wasm`)
//! every sheet gets, given the free flash of the board and the speed of the sheet.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crab_core::logic::SheetSpeed;

use crate::board_memo::BoardMemo;
use crate::mcu_module::{read_line, MasterInfo, ModuleImage, ModuleUploadError};
use crate::placement::{self, DemoteReason, Mode, PlacementError, SheetImage};
use crate::wamrc::{self, WamrcError};

/// Scan period of the master; every sheet has it for now.
pub const MASTER_PERIOD_MS: u32 = 10;

/// Size of a flash sector of the `crabmod` partition.
const SECTOR: usize = 4096;
/// Header the master writes before every image.
const AREA_HEADER: usize = 256;

/// Bytes an image of `len` bytes occupies in `crabmod` (whole sectors, with the header).
pub fn area_bytes(len: usize) -> usize {
    (AREA_HEADER + len).div_ceil(SECTOR) * SECTOR
}

/// What the master reports about its `crabmod` partition (answer to `info`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrabmodSpace {
    /// Free bytes.
    pub free: usize,
    pub capacity: usize,
    /// Stored images (every generation separately).
    pub areas: Vec<StoredArea>,
}

/// One image stored in `crabmod`, as the master lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArea {
    pub name: String,
    /// Offset of the area in the partition.
    pub at: usize,
    /// Image length (the area is [`area_bytes`] long).
    pub len: usize,
    pub gen: u32,
}

impl CrabmodSpace {
    /// Reads the lines of the `info` answer; `None` without a `crabmod=` line.
    pub fn parse<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let mut space: Option<(usize, usize)> = None;
        let mut areas = Vec::new();
        for line in lines {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("crabmod=") {
                let head = rest.split_whitespace().next()?;
                let (free, capacity) = head.split_once('/')?;
                space = Some((free.parse().ok()?, capacity.parse().ok()?));
                continue;
            }
            let mut words = line.split_whitespace();
            let Some(name) = words.next() else { continue };
            if name.contains('=') {
                continue;
            }
            let rest: Vec<&str> = words.collect();
            let field = |key: &str| {
                rest.iter()
                    .find_map(|w| w.strip_prefix(key))
                    .and_then(|v| v.parse::<usize>().ok())
            };
            if let (Some(at), Some(len)) = (field("at="), field("len=")) {
                areas.push(StoredArea {
                    name: name.to_owned(),
                    at,
                    len,
                    gen: field("gen=")
                        .and_then(|g| u32::try_from(g).ok())
                        .unwrap_or(0),
                });
            }
        }
        let (free, capacity) = space?;
        Some(CrabmodSpace {
            free,
            capacity,
            areas,
        })
    }

    /// Free bytes for uploading these modules: free sectors plus the areas of their running
    /// versions (the highest generation of each name), which the upload replaces. Stale
    /// generations stay occupied.
    pub fn free_for(&self, names: &[&str]) -> usize {
        let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
        let replaced: usize = unique
            .into_iter()
            .filter_map(|name| self.running(name))
            .map(|a| area_bytes(a.len))
            .sum();
        self.free.saturating_add(replaced)
    }

    /// The running area of a module: the one with the highest generation.
    fn running(&self, name: &str) -> Option<&StoredArea> {
        self.areas
            .iter()
            .filter(|a| a.name == name)
            .max_by_key(|a| a.gen)
    }
}

/// How a module gets replaced on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replace {
    /// The new image goes into a free gap while the old one keeps running.
    Beside,
    /// There is no gap: the old module is deleted first (it stands still during the write).
    InPlace,
}

/// No contiguous free flash for a module even after deleting its old version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("модуль «{name}»: не хватает непрерывного места во флеше (нужно {} КБ, наибольший свободный участок {} КБ) — удалите ненужные модули с платы или разделите лист", kb(*needed), kb(*largest))]
pub struct NoRoom {
    pub name: String,
    pub needed: usize,
    pub largest: usize,
}

/// An occupied span in the simulation: start, length, module name, generation.
type Span = (usize, usize, String, u32);

fn gaps(occupied: &[Span], capacity: usize) -> Vec<(usize, usize)> {
    let mut sorted: Vec<(usize, usize)> = occupied.iter().map(|o| (o.0, o.1)).collect();
    sorted.sort_unstable();
    let mut result = Vec::new();
    let mut cursor = 0usize;
    for (at, len) in sorted {
        if at > cursor {
            result.push((cursor, at - cursor));
        }
        cursor = cursor.max(at.saturating_add(len));
    }
    if capacity > cursor {
        result.push((cursor, capacity - cursor));
    }
    result
}

fn first_fit(occupied: &[Span], capacity: usize, need: usize) -> Option<usize> {
    gaps(occupied, capacity)
        .into_iter()
        .find(|g| g.1 >= need)
        .map(|g| g.0)
}

/// Removes the running (highest generation) span of `name` that is older than `below`.
fn remove_running(occupied: &mut Vec<Span>, name: &str, below: u32) -> bool {
    let idx = occupied
        .iter()
        .enumerate()
        .filter(|(_, o)| o.2 == name && o.3 < below)
        .max_by_key(|(_, o)| o.3)
        .map(|(i, _)| i);
    idx.map(|i| occupied.remove(i)).is_some()
}

/// Decides for every upload `(module name, image length)`, in upload order, whether it fits
/// beside the old version or has to replace it in place. This is an estimate: the master picks
/// the gap with its own cursor, so its real choice may differ from the first fit used here.
pub fn replacement_plan(
    space: &CrabmodSpace,
    uploads: &[(&str, usize)],
) -> Result<Vec<Replace>, NoRoom> {
    let mut occupied: Vec<Span> = space
        .areas
        .iter()
        .map(|a| (a.at, area_bytes(a.len), a.name.clone(), a.gen))
        .collect();
    let mut result = Vec::with_capacity(uploads.len());
    for &(name, len) in uploads {
        let need = area_bytes(len);
        let new_gen = occupied
            .iter()
            .filter(|o| o.2 == name)
            .map(|o| o.3)
            .max()
            .map_or(1, |g| g.saturating_add(1));
        if let Some(at) = first_fit(&occupied, space.capacity, need) {
            // The old running area is removed once the new one has started.
            remove_running(&mut occupied, name, new_gen);
            occupied.push((at, need, name.to_owned(), new_gen));
            result.push(Replace::Beside);
            continue;
        }
        if remove_running(&mut occupied, name, new_gen) {
            if let Some(at) = first_fit(&occupied, space.capacity, need) {
                occupied.push((at, need, name.to_owned(), new_gen));
                result.push(Replace::InPlace);
                continue;
            }
        }
        let largest = gaps(&occupied, space.capacity)
            .into_iter()
            .map(|g| g.1)
            .max()
            .unwrap_or(0);
        return Err(NoRoom {
            name: name.to_owned(),
            needed: need,
            largest,
        });
    }
    Ok(result)
}

/// Asks the master for `info` and reads its answer up to `end`.
pub fn read_crabmod_space<P: Read + Write>(
    port: &mut P,
) -> Result<Option<CrabmodSpace>, ModuleUploadError> {
    port.write_all(b"info\n")?;
    port.flush()?;
    let mut lines = Vec::new();
    loop {
        let line = read_line(port)?;
        if line == "end" {
            break;
        }
        lines.push(line);
    }
    Ok(CrabmodSpace::parse(lines.iter().map(String::as_str)))
}

/// Rough step estimate (microseconds) for a sheet of `wasm_len` bytes, from the measurements in
/// `specs/wasm-on-mcu/plan.md`; to be refined: (interpreter, AOT).
pub fn step_estimate(wasm_len: usize) -> (u32, u32) {
    let len = u32::try_from(wasm_len).unwrap_or(u32::MAX / 2);
    let interp = 12 + len.saturating_mul(2) / 5;
    (interp, 8 + interp / 4)
}

/// A sheet to upload.
pub struct SheetModule<'a> {
    pub sheet: u32,
    pub name: &'a str,
    pub wasm: &'a [u8],
    /// The module built for AOT, see `WasmModule::wasm_aot`; the AOT image is made from it.
    pub wasm_aot: &'a [u8],
    pub speed: SheetSpeed,
}

/// One sheet that did not get the accelerated image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemotedSheet {
    pub sheet: u32,
    pub name: String,
    pub reason: DemoteReason,
    /// Step estimate of the plain module and of the accelerated one, microseconds.
    pub step_us: u32,
    pub fast_step_us: u32,
    pub exceeds_period: bool,
}

/// What did not go as wished in an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadReport {
    pub demoted: Vec<DemotedSheet>,
    /// Text of the `wamrc` failure, if there was one.
    pub wamrc_problem: Option<String>,
    /// Bytes needed with every non-background sheet accelerated.
    pub needed: usize,
    /// Bytes free for this upload.
    pub free: usize,
}

fn kb(bytes: usize) -> usize {
    bytes.div_ceil(1024)
}

impl UploadReport {
    /// Numbers of the sheets that were not accelerated.
    pub fn demoted_sheets(&self) -> Vec<u32> {
        self.demoted.iter().map(|d| d.sheet).collect()
    }

    /// One text for the whole upload.
    pub fn message(&self) -> String {
        let mut text = String::from("Не все листы будут ускорены:");
        for d in &self.demoted {
            let why = match d.reason {
                DemoteReason::NoSpace => format!(
                    "не хватило места во флеше (нужно {} КБ, свободно {} КБ); шаг ≈ {} мкс вместо ≈ {} мкс",
                    kb(self.needed),
                    kb(self.free),
                    d.step_us,
                    d.fast_step_us
                ),
                DemoteReason::NoAotImage => format!(
                    "ускоренная сборка недоступна: {}; шаг ≈ {} мкс",
                    self.wamrc_problem.as_deref().unwrap_or("нет компилятора"),
                    d.step_us
                ),
            };
            text.push_str(&format!("\n  лист {} («{}»): {why}", d.sheet, d.name));
            if d.exceeds_period {
                text.push_str(&format!(
                    " — длиннее периода {MASTER_PERIOD_MS} мс, таймеры и импульсы будут неточными"
                ));
            }
        }
        let mut fixes = Vec::new();
        if self
            .demoted
            .iter()
            .any(|d| d.reason == DemoteReason::NoSpace)
        {
            fixes.push("удалите ненужные модули с платы");
            fixes.push("разделите лист");
        }
        if self
            .demoted
            .iter()
            .any(|d| d.reason == DemoteReason::NoAotImage)
        {
            fixes.push("скачайте wamrc или укажите путь к нему в настройках");
        }
        fixes.push("поставьте листу скорость «Фоновая»");
        let last = fixes.pop().unwrap_or_default();
        text.push_str("\nКак исправить: ");
        if !fixes.is_empty() {
            text.push_str(&fixes.join(", "));
            text.push_str(" или ");
        }
        text.push_str(last);
        text
    }
}

/// The images to send, in the order of the input sheets.
pub struct UploadPlan {
    pub images: Vec<ModuleImage>,
    /// Per image: `true` if the old module must be deleted before the upload (no room beside).
    pub in_place: Vec<bool>,
    /// `Some` iff at least one sheet is not accelerated although it could have been.
    pub report: Option<UploadReport>,
    /// `Some` iff at least one sheet has to be stopped for the upload.
    pub stops: Option<StopReport>,
}

/// One sheet that stands still while it is replaced in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedSheet {
    pub sheet: u32,
    pub name: String,
    /// Image length.
    pub bytes: usize,
}

/// Sheets that stop for the upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopReport {
    pub sheets: Vec<StoppedSheet>,
}

/// Rough time of writing an image of `bytes` bytes, seconds (constants, not measured).
fn stop_seconds(bytes: usize) -> f64 {
    0.5 + bytes as f64 / 40_000.0
}

impl StopReport {
    /// Rough total time the sheets stand still, seconds.
    pub fn total_seconds(&self) -> f64 {
        self.sheets.iter().map(|s| stop_seconds(s.bytes)).sum()
    }

    /// One text for the user.
    pub fn message(&self) -> String {
        let mut text = String::from(
            "Не хватает места, чтобы залить новую версию рядом со старой. На время заливки остановятся (выходы сбросятся в безопасное состояние, отката при сбое нет):",
        );
        for s in &self.sheets {
            text.push_str(&format!(
                "\n  лист {} («{}»): ≈ {:.0} с",
                s.sheet,
                s.name,
                stop_seconds(s.bytes).ceil()
            ));
        }
        text.push_str(
            "\nЧтобы не останавливать: удалите ненужные модули с платы или разделите лист.",
        );
        text
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UploadPlanError {
    #[error("отменено")]
    Cancelled,
    #[error(transparent)]
    Placement(#[from] PlacementError),
    #[error(transparent)]
    NoRoom(#[from] NoRoom),
}

fn image(bytes: Vec<u8>, aot: bool) -> ModuleImage {
    ModuleImage {
        bytes,
        aot,
        warning: None,
    }
}

/// Adds the replacement mode of every image (beside or in place) to a plan.
fn finish(
    images: Vec<ModuleImage>,
    report: Option<UploadReport>,
    space: Option<&CrabmodSpace>,
    modules: &[SheetModule],
) -> Result<UploadPlan, UploadPlanError> {
    let (in_place, stops) = match space {
        Some(space) => {
            let uploads: Vec<(&str, usize)> = modules
                .iter()
                .zip(&images)
                .map(|(m, img)| (m.name, img.bytes.len()))
                .collect();
            let modes = replacement_plan(space, &uploads)?;
            let stopped: Vec<StoppedSheet> = modules
                .iter()
                .zip(&images)
                .zip(&modes)
                .filter(|(_, mode)| **mode == Replace::InPlace)
                .map(|((m, img), _)| StoppedSheet {
                    sheet: m.sheet,
                    name: m.name.to_owned(),
                    bytes: img.bytes.len(),
                })
                .collect();
            let flags = modes.iter().map(|m| *m == Replace::InPlace).collect();
            let stops = (!stopped.is_empty()).then_some(StopReport { sheets: stopped });
            (flags, stops)
        }
        None => (vec![false; modules.len()], None),
    };
    Ok(UploadPlan {
        images,
        in_place,
        report,
        stops,
    })
}

/// The input of the placement for one sheet; `aot_bytes` is the area of its accelerated image.
fn sheet_image(m: &SheetModule, aot_bytes: Option<usize>) -> SheetImage {
    let (step_interp_us, step_aot_us) = step_estimate(m.wasm.len());
    SheetImage {
        number: m.sheet,
        speed: m.speed,
        wasm_bytes: area_bytes(m.wasm.len()),
        aot_bytes,
        period_ms: MASTER_PERIOD_MS,
        step_interp_us,
        step_aot_us,
    }
}

/// The report entry of a sheet that was moved to the interpreter.
fn demoted_sheet(m: &SheetModule, placed: &placement::Placed) -> Option<DemotedSheet> {
    let reason = placed.demoted?;
    let (_, fast_step_us) = step_estimate(m.wasm.len());
    Some(DemotedSheet {
        sheet: m.sheet,
        name: m.name.to_owned(),
        reason,
        step_us: placed.step_us,
        fast_step_us,
        exceeds_period: placed.exceeds_period,
    })
}

/// Size of an AOT image estimated from the size of the `.wasm` it is built from (`wasm_aot`, with
/// import wrappers) when it is not built yet. Measured on C3: blink 569 -> 2628 bytes, a 94-block
/// self test 7642 -> 18096 (without wrappers 46064); kept a little above both.
pub fn estimated_aot_len(wasm_len: usize) -> usize {
    wasm_len * 9 / 4 + 1536
}

/// Placement of the sheets on the board as remembered in `memo`, for the check panel: no wamrc
/// run; an AOT image not in `cache` is estimated from the `.wasm` size.
pub fn preview(
    memo: &BoardMemo,
    modules: &[SheetModule],
    cache: &Path,
) -> Result<Vec<DemotedSheet>, PlacementError> {
    let info = memo.info();
    let space = memo.space();
    let target = info.aot_target();
    if target.is_none() {
        if let Some(m) = modules.iter().find(|m| m.speed == SheetSpeed::Fast) {
            return Err(PlacementError::FastWithoutAot { number: m.sheet });
        }
    }
    let sheets: Vec<SheetImage> = modules
        .iter()
        .map(|m| {
            let aot = match target {
                Some(target) if m.speed != SheetSpeed::Background => {
                    let len = wamrc::cached(m.wasm_aot, target, cache)
                        .map_or_else(|| estimated_aot_len(m.wasm_aot.len()), |b| b.len());
                    Some(area_bytes(len))
                }
                _ => None,
            };
            sheet_image(m, aot)
        })
        .collect();
    let names: Vec<&str> = modules.iter().map(|m| m.name).collect();
    let placement = placement::place(&sheets, space.free_for(&names))?;
    if target.is_none() {
        // Everything runs in the interpreter: nothing is "demoted", only the fit is checked.
        return Ok(Vec::new());
    }
    Ok(modules
        .iter()
        .zip(&placement.sheets)
        .filter_map(|(m, placed)| demoted_sheet(m, placed))
        .collect())
}

/// Decides the image of every sheet for `info`. `space` is what the master reports about its
/// flash (`None`: unknown, no limit). `wamrc` is asked for the compiler at most once, and only
/// when some image is not in `cache` yet.
pub fn plan_upload(
    info: &MasterInfo,
    space: Option<&CrabmodSpace>,
    modules: &[SheetModule],
    cache: &Path,
    wamrc: &mut dyn FnMut() -> Result<PathBuf, WamrcError>,
) -> Result<UploadPlan, UploadPlanError> {
    let Some(target) = info.aot_target() else {
        if let Some(m) = modules.iter().find(|m| m.speed == SheetSpeed::Fast) {
            return Err(PlacementError::FastWithoutAot { number: m.sheet }.into());
        }
        let images = modules
            .iter()
            .map(|m| image(m.wasm.to_vec(), false))
            .collect();
        return finish(images, None, space, modules);
    };

    // The compiler is looked for once; `Err` keeps why it is not at hand.
    let mut compiler: Option<Result<PathBuf, String>> = None;
    let mut problem: Option<String> = None;
    let mut aot_images: Vec<Option<Vec<u8>>> = Vec::with_capacity(modules.len());
    for m in modules {
        if m.speed == SheetSpeed::Background {
            aot_images.push(None);
            continue;
        }
        if let Some(bytes) = wamrc::cached(m.wasm_aot, target, cache) {
            aot_images.push(Some(bytes));
            continue;
        }
        if compiler.is_none() {
            compiler = Some(match wamrc() {
                Ok(exe) => Ok(exe),
                Err(WamrcError::Cancelled) => return Err(UploadPlanError::Cancelled),
                Err(e) => Err(e.to_string()),
            });
        }
        let built = match compiler.as_ref() {
            Some(Ok(exe)) => wamrc::compile(exe, m.wasm_aot, target, cache),
            Some(Err(text)) => Err(WamrcError::Failed(text.clone())),
            None => unreachable!("the compiler was looked for above"),
        };
        match built {
            Ok(bytes) => aot_images.push(Some(bytes)),
            Err(WamrcError::Cancelled) => return Err(UploadPlanError::Cancelled),
            Err(e) => {
                problem.get_or_insert_with(|| match e {
                    WamrcError::Failed(text) => text,
                    other => other.to_string(),
                });
                aot_images.push(None);
            }
        }
    }

    let sheets: Vec<SheetImage> = modules
        .iter()
        .zip(&aot_images)
        .map(|(m, aot)| sheet_image(m, aot.as_ref().map(|b| area_bytes(b.len()))))
        .collect();
    let names: Vec<&str> = modules.iter().map(|m| m.name).collect();
    let free = space.map_or(usize::MAX, |s| s.free_for(&names));
    let placement = placement::place(&sheets, free)?;

    let mut images = Vec::with_capacity(modules.len());
    let mut demoted = Vec::new();
    for ((m, aot), placed) in modules.iter().zip(aot_images).zip(&placement.sheets) {
        match (placed.mode, aot) {
            (Mode::Aot, Some(bytes)) => images.push(image(bytes, true)),
            _ => images.push(image(m.wasm.to_vec(), false)),
        }
        demoted.extend(demoted_sheet(m, placed));
    }
    let report = (!demoted.is_empty()).then(|| UploadReport {
        demoted,
        wamrc_problem: problem,
        needed: sheets
            .iter()
            .map(|s| match s.speed {
                SheetSpeed::Background => s.wasm_bytes,
                _ => s.aot_bytes.unwrap_or(s.wasm_bytes),
            })
            .sum(),
        free,
    });
    finish(images, report, space, modules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const INFO_ANSWER: &str = "master=crab-master-rs 0.1.0\n\
        crabmod=40960/131072 largest=24576\n\
        sheet1 at=0 len=5000 gen=1 mode=aot mem=1024 allow=-\n\
        sheet1 at=8192 len=3000 gen=2 mode=interp mem=1024 allow=-\n\
        other at=16384 len=100 gen=1 mode=interp mem=64 allow=-\n\
        end\n";

    struct FakePort {
        written: Vec<u8>,
        replies: VecDeque<u8>,
    }

    impl Read for FakePort {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.replies.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Ok(0),
            }
        }
    }

    impl Write for FakePort {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir()
                .join(format!("crab-upload-plan-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn info(caps: &str) -> MasterInfo {
        MasterInfo {
            caps: caps.split(',').map(str::to_owned).collect(),
            ..MasterInfo::default()
        }
    }

    fn aot_info() -> MasterInfo {
        info("flashmod,abi4,aot,rv32imc")
    }

    fn wasm(fill: u8, len: usize) -> Vec<u8> {
        let mut v = b"\0asm\x01\0\0\0".to_vec();
        v.resize(len, fill);
        v
    }

    /// Puts an "AOT image" of `len` bytes for `wasm` into the cache.
    fn prefill(cache: &Path, wasm: &[u8], len: usize) {
        let key = wamrc::cache_key(wasm, crate::wamrc::AotTarget::Riscv32Imc);
        let mut image = b"\0aot".to_vec();
        image.resize(len, 7);
        std::fs::write(cache.join(format!("{key}.aot")), image).unwrap();
    }

    fn module<'a>(sheet: u32, wasm: &'a [u8], speed: SheetSpeed) -> SheetModule<'a> {
        SheetModule {
            sheet,
            name: if sheet == 2 { "sheet2" } else { "sheet3" },
            wasm,
            wasm_aot: wasm,
            speed,
        }
    }

    fn memo(caps: &str, free: usize) -> BoardMemo {
        BoardMemo::new(
            &info(caps),
            &CrabmodSpace {
                free,
                capacity: 131072,
                areas: Vec::new(),
            },
        )
    }

    #[test]
    fn preview_fits_reports_nothing() {
        let dir = TempDir::new("pv-fits");
        let w = wasm(1, 300);
        let m = [module(2, &w, SheetSpeed::Normal)];
        let r = preview(&memo("flashmod,abi4,aot,rv32imc", 65536), &m, &dir.0).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn preview_short_flash_demotes_normal_sheet() {
        let dir = TempDir::new("pv-short");
        let w = wasm(1, 2000);
        let m = [module(2, &w, SheetSpeed::Normal)];
        // Room for the .wasm area only; the estimated AOT image takes two sectors.
        let r = preview(&memo("aot,rv32imc", 4096), &m, &dir.0);
        let r = r.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sheet, 2);
        assert_eq!(r[0].reason, DemoteReason::NoSpace);
    }

    #[test]
    fn preview_fast_too_big() {
        let dir = TempDir::new("pv-fast");
        let w = wasm(1, 2000);
        let m = [module(2, &w, SheetSpeed::Fast)];
        let err = preview(&memo("aot,rv32imc", 4096), &m, &dir.0).unwrap_err();
        assert!(matches!(err, PlacementError::FastDoesNotFit { .. }));
    }

    #[test]
    fn preview_fast_without_aot() {
        let dir = TempDir::new("pv-noaot");
        let w = wasm(1, 300);
        let m = [module(2, &w, SheetSpeed::Fast)];
        let err = preview(&memo("flashmod", 65536), &m, &dir.0).unwrap_err();
        assert_eq!(err, PlacementError::FastWithoutAot { number: 2 });
        let normal = [module(2, &w, SheetSpeed::Normal)];
        assert!(preview(&memo("flashmod", 65536), &normal, &dir.0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn preview_prefers_cached_image_size() {
        let dir = TempDir::new("pv-cache");
        let w = wasm(1, 300);
        let m = [module(2, &w, SheetSpeed::Normal)];
        // The estimate (about 2200 bytes) takes one sector; both images fit in two sectors.
        let free = 8192;
        assert!(preview(&memo("aot,rv32imc", free), &m, &dir.0)
            .unwrap()
            .is_empty());
        // A cached image of 9000 bytes takes three sectors: the sheet is demoted now.
        prefill(&dir.0, &w, 9000);
        let r = preview(&memo("aot,rv32imc", free), &m, &dir.0).unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn area_bytes_edges() {
        assert_eq!(area_bytes(0), 4096);
        assert_eq!(area_bytes(3840), 4096);
        assert_eq!(area_bytes(3841), 8192);
    }

    #[test]
    fn parse_sample_answer() {
        let space = CrabmodSpace::parse(INFO_ANSWER.lines()).unwrap();
        assert_eq!(space.free, 40960);
        assert_eq!(space.capacity, 131072);
        assert_eq!(
            space.areas,
            vec![
                area("sheet1", 0, 5000, 1),
                area("sheet1", 8192, 3000, 2),
                area("other", 16384, 100, 1)
            ]
        );
    }

    #[test]
    fn parse_without_crabmod_line() {
        let lines = ["master=old 0.1", "sheet1 at=0 len=10 gen=1"];
        assert_eq!(CrabmodSpace::parse(lines), None);
    }

    fn area(name: &str, at: usize, len: usize, gen: u32) -> StoredArea {
        StoredArea {
            name: name.to_owned(),
            at,
            len,
            gen,
        }
    }

    fn space_of(capacity: usize, areas: Vec<StoredArea>) -> CrabmodSpace {
        let used: usize = areas.iter().map(|a| area_bytes(a.len)).sum();
        CrabmodSpace {
            free: capacity - used,
            capacity,
            areas,
        }
    }

    #[test]
    fn free_for_counts_only_running_generation() {
        let space = CrabmodSpace::parse(INFO_ANSWER.lines()).unwrap();
        // Running sheet1 is gen 2 (3000 -> 4096); gen 1 stays occupied; "other" 4096.
        assert_eq!(space.free_for(&["sheet1"]), 40960 + 4096);
        assert_eq!(space.free_for(&["nothing"]), 40960);
        assert_eq!(space.free_for(&["sheet1", "other"]), 40960 + 4096 + 4096);
        assert_eq!(space.free_for(&["sheet1", "sheet1"]), 40960 + 4096);
    }

    #[test]
    fn parse_skips_lines_without_at_and_defaults_gen() {
        let lines = ["crabmod=0/8192", "a len=10", "b at=0 len=10"];
        let space = CrabmodSpace::parse(lines).unwrap();
        assert_eq!(space.areas, vec![area("b", 0, 10, 0)]);
    }

    #[test]
    fn replacement_beside_when_gap_exists() {
        let space = space_of(5 * 4096, vec![area("a", 0, 100, 1)]);
        let plan = replacement_plan(&space, &[("a", 100)]).unwrap();
        assert_eq!(plan, vec![Replace::Beside]);
    }

    #[test]
    fn replacement_in_place_when_no_gap_beside() {
        let space = space_of(
            2 * 4096,
            vec![area("a", 0, 100, 1), area("b", 4096, 100, 1)],
        );
        let plan = replacement_plan(&space, &[("a", 100)]).unwrap();
        assert_eq!(plan, vec![Replace::InPlace]);
    }

    #[test]
    fn fragmentation_total_free_is_not_enough() {
        // 8 sectors; free sectors 1, 3, 5 (total 3), but 2 in a row are needed.
        let space = space_of(
            8 * 4096,
            vec![
                area("a", 0, 100, 1),
                area("b", 2 * 4096, 100, 1),
                area("c", 4 * 4096, 100, 1),
                area("d", 6 * 4096, 100, 1),
                area("e", 7 * 4096, 100, 1),
            ],
        );
        assert!(space.free >= 2 * 4096);
        // Deleting "a" gives sectors 0 and 1 in a row.
        let plan = replacement_plan(&space, &[("a", 5000)]).unwrap();
        assert_eq!(plan, vec![Replace::InPlace]);
        // A module that is not on the board has nothing to delete.
        let err = replacement_plan(&space, &[("new", 5000)]).unwrap_err();
        assert_eq!(err.needed, 8192);
        assert_eq!(err.largest, 4096);
        assert!(err.to_string().contains("непрерывного места"));
    }

    #[test]
    fn stale_generation_stays_occupied() {
        let space = space_of(
            2 * 4096,
            vec![area("a", 0, 100, 1), area("a", 4096, 100, 2)],
        );
        assert_eq!(space.free_for(&["a"]), 4096);
        let plan = replacement_plan(&space, &[("a", 100)]).unwrap();
        assert_eq!(plan, vec![Replace::InPlace]);
        // Two sectors needed: the stale one cannot be reclaimed.
        let err = replacement_plan(&space, &[("a", 5000)]).unwrap_err();
        assert_eq!(err.largest, 4096);
    }

    #[test]
    fn sequential_uploads_reuse_freed_areas() {
        let space = space_of(
            3 * 4096,
            vec![area("a", 0, 100, 1), area("b", 4096, 100, 1)],
        );
        // "a" goes beside (sector 2), freeing sector 0; "b" then goes beside into sector 0.
        let plan = replacement_plan(&space, &[("a", 100), ("b", 100)]).unwrap();
        assert_eq!(plan, vec![Replace::Beside, Replace::Beside]);
    }

    #[test]
    fn plan_upload_sets_stops_when_no_room_beside() {
        let w = wasm(1, 100);
        let t = TempDir::new("stops");
        let space = space_of(
            2 * 4096,
            vec![area("sheet2", 0, 100, 1), area("other", 4096, 100, 1)],
        );
        let plan = plan_upload(
            &info("flashmod,abi4"),
            Some(&space),
            &[module(2, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || panic!("no compiler needed"),
        )
        .ok()
        .unwrap();
        assert_eq!(plan.in_place, vec![true]);
        let stops = plan.stops.unwrap();
        assert_eq!(stops.sheets[0].sheet, 2);
        let text = stops.message();
        assert!(text.contains("лист 2 («sheet2»)"), "{text}");
        assert!(text.contains("Чтобы не останавливать"), "{text}");
        assert!(stops.total_seconds() > 0.5);
    }

    #[test]
    fn plan_upload_without_space_is_beside() {
        let w = wasm(1, 100);
        let t = TempDir::new("beside");
        let plan = plan_upload(
            &info("flashmod,abi4"),
            None,
            &[module(2, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || panic!("no compiler needed"),
        )
        .ok()
        .unwrap();
        assert_eq!(plan.in_place, vec![false]);
        assert!(plan.stops.is_none());
    }

    #[test]
    fn reads_space_from_port() {
        let mut port = FakePort {
            written: Vec::new(),
            replies: INFO_ANSWER.bytes().collect(),
        };
        let space = read_crabmod_space(&mut port).unwrap().unwrap();
        assert_eq!(port.written, b"info\n");
        assert_eq!(space.free, 40960);
        assert_eq!(space.areas.len(), 3);
    }

    #[test]
    fn step_estimate_grows_with_size() {
        let (i, a) = step_estimate(1000);
        assert_eq!(i, 412);
        assert_eq!(a, 8 + 103);
    }

    #[test]
    fn master_without_aot_gives_plain_images() {
        let w = wasm(1, 100);
        let t = TempDir::new("noaot");
        let plan = plan_upload(
            &info("flashmod,abi4"),
            None,
            &[module(2, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || panic!("no compiler needed"),
        )
        .ok()
        .unwrap();
        assert!(plan.report.is_none());
        assert!(!plan.images[0].aot);
        assert_eq!(plan.images[0].bytes, w);
        assert!(plan.images[0].warning.is_none());
    }

    #[test]
    fn fast_without_aot_is_error() {
        let w = wasm(1, 100);
        let t = TempDir::new("fast");
        let err = plan_upload(
            &info("flashmod"),
            None,
            &[module(2, &w, SheetSpeed::Fast)],
            &t.0,
            &mut || panic!("no compiler needed"),
        )
        .err()
        .unwrap();
        assert!(matches!(
            err,
            UploadPlanError::Placement(PlacementError::FastWithoutAot { number: 2 })
        ));
    }

    #[test]
    fn background_is_never_compiled() {
        let w = wasm(1, 100);
        let t = TempDir::new("bg");
        let plan = plan_upload(
            &aot_info(),
            None,
            &[module(2, &w, SheetSpeed::Background)],
            &t.0,
            &mut || panic!("background sheets are not compiled"),
        )
        .ok()
        .unwrap();
        assert!(plan.report.is_none());
        assert!(!plan.images[0].aot);
    }

    #[test]
    fn cached_images_are_used_without_compiler() {
        let w = wasm(1, 100);
        let t = TempDir::new("cached");
        prefill(&t.0, &w, 500);
        let plan = plan_upload(
            &aot_info(),
            None,
            &[module(2, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || panic!("cached"),
        )
        .ok()
        .unwrap();
        assert!(plan.report.is_none());
        assert!(plan.images[0].aot);
        assert_eq!(plan.images[0].bytes.len(), 500);
    }

    #[test]
    fn short_space_demotes_and_reports() {
        let a = wasm(1, 100);
        let b = wasm(2, 100);
        let t = TempDir::new("space");
        prefill(&t.0, &a, 5000);
        prefill(&t.0, &b, 5000);
        // Both AOT: 16384; one demoted: 12288.
        let space = CrabmodSpace {
            free: 12288,
            capacity: 131072,
            areas: Vec::new(),
        };
        let plan = plan_upload(
            &aot_info(),
            Some(&space),
            &[
                module(2, &a, SheetSpeed::Normal),
                module(3, &b, SheetSpeed::Normal),
            ],
            &t.0,
            &mut || panic!("cached"),
        )
        .ok()
        .unwrap();
        assert!(!plan.images[0].aot);
        assert!(plan.images[1].aot);
        let report = plan.report.unwrap();
        assert_eq!(report.demoted_sheets(), vec![2]);
        let text = report.message();
        assert!(text.contains("лист 2 («sheet2»)"), "{text}");
        assert!(text.contains("не хватило места во флеше"), "{text}");
        assert!(text.contains("16 КБ, свободно 12 КБ"), "{text}");
        assert!(!text.contains("таймеры"), "{text}");
        assert!(!text.contains("AOT"), "{text}");
    }

    #[test]
    fn no_compiler_demotes_with_period_warning() {
        let w = wasm(1, 30_000);
        let t = TempDir::new("nocomp");
        let mut asked = 0;
        let plan = plan_upload(
            &aot_info(),
            None,
            &[module(3, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || {
                asked += 1;
                Err(WamrcError::Download("нет сети".to_owned()))
            },
        )
        .ok()
        .unwrap();
        assert_eq!(asked, 1);
        assert!(!plan.images[0].aot);
        let text = plan.report.unwrap().message();
        assert!(text.contains("лист 3 («sheet3»)"), "{text}");
        assert!(text.contains("ускоренная сборка недоступна"), "{text}");
        assert!(text.contains("нет сети"), "{text}");
        assert!(text.contains("таймеры и импульсы"), "{text}");
        assert!(text.contains("скачайте wamrc"), "{text}");
    }

    #[test]
    fn cancelled_compiler_cancels_plan() {
        let w = wasm(1, 100);
        let t = TempDir::new("cancel");
        let err = plan_upload(
            &aot_info(),
            None,
            &[module(2, &w, SheetSpeed::Normal)],
            &t.0,
            &mut || Err(WamrcError::Cancelled),
        )
        .err()
        .unwrap();
        assert!(matches!(err, UploadPlanError::Cancelled));
    }

    #[test]
    fn fast_with_failed_compiler_is_error() {
        let w = wasm(1, 100);
        let t = TempDir::new("fastfail");
        let err = plan_upload(
            &aot_info(),
            None,
            &[module(2, &w, SheetSpeed::Fast)],
            &t.0,
            &mut || Err(WamrcError::Failed("x".to_owned())),
        )
        .err()
        .unwrap();
        assert!(matches!(
            err,
            UploadPlanError::Placement(PlacementError::FastWithoutAot { number: 2 })
        ));
    }
}
