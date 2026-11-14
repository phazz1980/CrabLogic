//! The board side of the Rust master (ESP-IDF): the kernel runs the modules stored in the
//! `crabmod` partition on WAMR, and the USB console takes the commands of the editor
//! (`crab_master_rs::protocol`).
//!
//! Two threads (`specs/fast-cycle`). The scan: a hardware timer on its core wakes it (`tick`, period
//! from the modules' `period_ms()`), it steps the modules, publishes what the debugger reads
//! (`snapshot`), then runs the commands handed to it and moves an upload one step further (a flash
//! sector erased or a chunk written). The thread `crab-io` (`io`) reads the port, answers the polls
//! of the debugger from the snapshot and writes every answer. Nothing is printed on its own:
//! every line is an answer to a command, module errors wait in the journal (`log`).
//! A module whose step failed stops (its outputs are safe) and starts again from its stored
//! image after a growing pause (`restart`).
//!
//! Commands: `put`, `list`, `info`, `tick`, `del`, `swap`, `reset`, `rmax`, `log`; live
//! data of a module: `in`, `param`, `out`, `ev`, `force`, `unforce`, `pins`.

mod analog;
mod bus;
mod coredump;
mod count;
mod devices;
mod engine;
mod eth;
mod flash;
mod i2c_send;
mod io;
mod modbus;
mod mqtt;
mod net;
mod netcmd;
mod note_queue;
mod notes;
mod pcf;
mod pulse;
mod retain;
mod snapshot;
mod sparkplug;
mod spin;
mod tick;
mod uart;
pub(crate) mod usb;
mod watchdog;
mod web;

use std::time::{Duration, Instant};

use crab_kernel::bus::TopicKind;
use crab_kernel::engine::{Instance, Limits};
use crab_kernel::handle::Handle;
use crab_kernel::host::{ModuleHost, Word};
use crab_kernel::lease::{Fault, PinUse};
use crab_kernel::Kernel;
use crab_master_rs::boot_guard::{self, Counter, Reset};
use crab_master_rs::cycle::{self, Cycle};
use crab_master_rs::protocol::{
    format_bus_module, format_pin, format_topic, parse_command, Command, Event, ParseError,
    MAX_SLOTS,
};
use crab_master_rs::restart::Restart;
use crab_master_rs::storage::{crc32_update, Area, Meta, Storage, SECTOR};
use crab_master_rs::wamr::{crab_mem, ExecutionMode};
use esp_idf_svc::sys;

use devices::BoardDevices;
use engine::{Image, Wamr, WamrInstance};
use flash::PartitionFlash;

/// How often a growing overrun count goes to the journal, milliseconds.
const OVERRUN_NOTE_MS: u64 = 10_000;
/// Priority of the scan thread: above lwIP and the network tasks of the master, below Wi-Fi
/// and `esp_timer`.
const SCAN_PRIORITY: u8 = 20;
/// WAMR needs room on the native stack of the thread that runs the modules.
const RUNTIME_STACK: usize = 32 * 1024;
/// Bytes of a `put` chunk: fits the receive buffer of the USB driver.
const CHUNK: usize = 1024;
/// How long the next chunk of an upload is waited for.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5);
/// Watched signals per module, as in the C++ master.
const OUTPUTS: usize = 128;
/// Largest linear memory of one module, bytes.
const MAX_MEMORY: usize = 128 * 1024;
/// Most steps one `tick` runs.
const MAX_TICKS: u32 = 100_000;
/// What modules may use, as `modcaps=` names it, and the CPU the AOT images must be built for.
/// `math`, `tag_text` and `on_fault` are done by `crab-kernel` itself (its host and the pin leases).
/// `retain` and `retain_text` keep values in NVS (`retain.rs`); UART blocks: `uart.rs`; PCF8574 expanders: `pcf.rs`.
#[cfg(target_arch = "riscv32")]
const MODCAPS: &str = "flashmod,abi5,aot,sparkplug,bus,math,tag_text,on_fault,retain,retain_text,uart_format,uart_console,pcf_expander,modbus,web,rv32imc";
/// Classic ESP32: AOT images from a wamrc built with Espressif's LLVM, pre-linked by the editor
/// (`crab-build/src/aot_xtensa.rs`) and run from flash through the instruction bus (`shim.c`).
#[cfg(not(target_arch = "riscv32"))]
const MODCAPS: &str = "flashmod,abi5,aot,sparkplug,bus,math,tag_text,on_fault,retain,retain_text,uart_format,uart_console,pcf_expander,modbus,web,dac,xtensa-esp32";

type Module = WamrInstance<ModuleHost<BoardDevices>>;

/// Heap a loaded module must leave free (see [`Master::load`]).
const HEAP_RESERVE: u32 = 16 * 1024;
/// Heap of a module besides its linear memory and image copy: the WAMR module and instance, the
/// execution environment with its stack, the host (estimate for the check before loading).
const MODULE_OVERHEAD: u32 = 12 * 1024;

/// The error of a module that does not fit with the reserve.
fn low_memory(need: u32, free: u32) -> String {
    format!(
        "не хватает памяти: модулю нужно около {need} Б и {HEAP_RESERVE} Б запаса мастеру, свободно {free} Б"
    )
}

fn free_heap() -> u32 {
    // SAFETY: no arguments, no state.
    unsafe { sys::esp_get_free_heap_size() }
}

/// A module of the master: its image in the storage and, if it started, its instance.
struct Slot {
    name: String,
    /// Offset of its area in the storage.
    at: usize,
    /// `None` for a module that did not start.
    handle: Option<Handle>,
    /// Stepped by `tick` only.
    paused: bool,
    steps: u32,
    faults: u32,
    last_us: u32,
    max_us: u32,
    /// Longest time between two steps, microseconds.
    period_max_us: u32,
    sum_us: u32,
    last_step: Option<Instant>,
    /// Heap the instance took when it was loaded.
    heap: u32,
    /// When a failed module starts again.
    restart: Restart,
    /// Pins already reported as held by another module (once per pin).
    shared_pins: Vec<u8>,
    /// Event queue length set by `queue`; 0 for the default.
    queue: usize,
}

impl Slot {
    fn new(name: String, at: usize, handle: Option<Handle>, heap: u32) -> Self {
        Self {
            name,
            at,
            handle,
            paused: false,
            steps: 0,
            faults: 0,
            last_us: 0,
            max_us: 0,
            period_max_us: 0,
            sum_us: 0,
            last_step: None,
            heap,
            restart: Restart::new(),
            shared_pins: Vec::new(),
            queue: 0,
        }
    }
}

/// A `put` between its header and the last chunk.
struct Upload {
    /// The mode is known once the first chunk has come.
    meta: Meta,
    at: usize,
    /// The next sector to erase and the end of the area; `ready` is sent when they meet.
    erase_at: usize,
    erase_end: usize,
    received: usize,
    crc: u32,
    last_rx: Instant,
}

/// The answer to a line that did not parse, as the C++ master gives it.
fn fallback(line: &str) -> &'static str {
    match line.split(' ').next().unwrap_or("") {
        "put" if line.len() > 3 => "bad put",
        "tick" if line.len() > 4 => "bad tick",
        "in" | "param" | "force" if line.contains(' ') => "bad index or type",
        "unforce" if line.contains(' ') => "bad index",
        "out" if line.contains(' ') => "bad count",
        "topic" | "queue" | "bus" => "bad bus command",
        "spdump" => "spdump <ms>",
        "pulsetest" => "pulsetest <pin>",
        "pulsebench" => "pulsebench <pin> <n>",
        "ev" | "reset" if line.contains(' ') => "no such module",
        _ => "unknown command",
    }
}

struct Master {
    wamr: Wamr,
    kernel: Kernel<Module, BoardDevices>,
    storage: Storage<PartitionFlash>,
    slots: Vec<Slot>,
    upload: Option<Upload>,
    started: Instant,
    /// Grows whenever modules start or go: the snapshot is rebuilt then.
    layout: u32,
    /// Bad resets in a row before this start; at [`boot_guard::SAFE_AFTER`] the stored modules
    /// are not started (safe mode).
    bad_boots: u32,
    /// Timing of the scan.
    cycle: Cycle,
    /// Overruns already reported to the journal, and when.
    overruns_noted: u32,
    overruns_noted_at: u64,
}

/// The boot loop counter: in RTC memory, which keeps its contents over every reset but a power-on.
#[link_section = ".rtc_noinit.crab_boot"]
static mut BOOT_COUNTER: Counter = Counter { magic: 0, bad: 0 };

/// Counts this start in [`BOOT_COUNTER`]; returns the bad resets in a row.
fn count_boot() -> u32 {
    // SAFETY: a plain query of ESP-IDF.
    let why = match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => Reset::PowerOn,
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT
        | sys::esp_reset_reason_t_ESP_RST_INT_WDT
        | sys::esp_reset_reason_t_ESP_RST_WDT
        | sys::esp_reset_reason_t_ESP_RST_PANIC => Reset::Bad,
        _ => Reset::Other,
    };
    // SAFETY: called once, from the runtime thread, before anything else touches the counter.
    // Volatile copy in and out: the optimizer must not shrink `magic` (only ever 0 or MAGIC) to a
    // bit, or the garbage of RTC memory passes for a valid counter.
    unsafe {
        let p = core::ptr::addr_of_mut!(BOOT_COUNTER);
        let mut counter = p.read_volatile();
        let bad = counter.boot(why);
        p.write_volatile(counter);
        bad
    }
}

impl Master {
    fn new() -> Result<Self, String> {
        let wamr = Wamr::new()?;
        let storage = Storage::open(PartitionFlash::open()?);
        let mut master = Self {
            wamr,
            kernel: Kernel::new(),
            storage,
            slots: Vec::new(),
            upload: None,
            started: Instant::now(),
            layout: 0,
            bad_boots: {
                watchdog::take_trip();
                count_boot()
            },
            cycle: Cycle::new(crab_kernel::abi::DEFAULT_PERIOD_MS),
            overruns_noted: 0,
            overruns_noted_at: 0,
        };
        let safe = boot_guard::safe(master.bad_boots);
        if safe {
            println!(
                "safe mode: reset by watchdog {} times, modules are not started",
                master.bad_boots
            );
        }
        let stored: Vec<(String, usize)> = master
            .storage
            .areas()
            .iter()
            .map(|a| (a.meta.name.clone(), a.at))
            .collect();
        for (name, at) in stored {
            if safe {
                master.slots.push(Slot::new(name, at, None, 0));
                continue;
            }
            if let Err(e) = master.start(at, true) {
                master.log(&name, &e);
                master.slots.push(Slot::new(name, at, None, 0));
            }
        }
        Ok(master)
    }

    fn log(&mut self, name: &str, text: &str) {
        let ms = self.started.elapsed().as_millis() as u32;
        snapshot::log(ms, format!("[{name}] {text}"));
    }

    fn slot(&self, name: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.name == name)
    }

    /// Loads the image of an area; returns the instance and the heap it took.
    ///
    /// A module never takes the last of the heap: the rest of the firmware (std's thread locals,
    /// Wi-Fi, lwIP) aborts on a failed allocation, so a load that would leave less than
    /// [`HEAP_RESERVE`] is refused before it starts, and one that did is undone
    /// (`specs/safety-audit`, item 9).
    fn load(&self, area: &Area) -> Result<(Module, u32), String> {
        let before = free_heap();
        let copy = match area.meta.mode {
            ExecutionMode::Interpreter => area.meta.len as u32,
            ExecutionMode::AotXip => 0,
        };
        let need = area
            .meta
            .memory
            .saturating_add(copy)
            .saturating_add(MODULE_OVERHEAD);
        if before < need.saturating_add(HEAP_RESERVE) {
            return Err(low_memory(need, before));
        }
        // SAFETY: an area is reused only after the module that runs from it is gone
        // (`put` and `del` drop the instance first).
        let bytes = unsafe { self.storage.flash().mapped(area.data_at(), area.meta.len) }
            .ok_or_else(|| "образ модуля выходит за раздел crabmod".to_owned())?;
        let mut memory = area.meta.memory;
        let image = match area.meta.mode {
            ExecutionMode::Interpreter => {
                if memory == 0 {
                    memory = crab_mem(bytes).unwrap_or(0);
                }
                // A failed Rust allocation aborts the firmware: the copy is reserved first, so
                // a module that does not fit is an error, not a restart (`specs/safety-audit`).
                let mut copy = Vec::new();
                copy.try_reserve_exact(bytes.len()).map_err(|_| {
                    format!(
                        "не хватает памяти для образа модуля ({} байт, свободно {})",
                        bytes.len(),
                        free_heap()
                    )
                })?;
                copy.extend_from_slice(bytes);
                Image::Wasm(copy)
            }
            ExecutionMode::AotXip => Image::AotXip(bytes),
        };
        let limits = Limits {
            max_memory: MAX_MEMORY,
            ..Limits::default()
        };
        let host = ModuleHost::new(BoardDevices::default(), OUTPUTS);
        let module = self
            .wamr
            .load_image(image, memory, &area.allow_list(), limits, host)?;
        let after = free_heap();
        if after < HEAP_RESERVE {
            drop(module);
            return Err(low_memory(before.saturating_sub(after), before));
        }
        Ok((module, before.saturating_sub(after)))
    }

    /// The new version did not load next to the running old one (not enough memory): the old
    /// instance is freed but keeps its outputs, the new one is loaded again, and if that fails
    /// the old image is put back. Returns the new handle and heap, or the original error `err`
    /// (the old version then runs again, or the module is stopped if even that failed).
    fn load_without_old(
        &mut self,
        old: Handle,
        index: usize,
        area: &Area,
        err: String,
    ) -> Result<(Handle, u32), String> {
        let name = area.meta.name.clone();
        let Ok(evicted) = self.kernel.evict(old) else {
            return Err(err);
        };
        self.log(
            &name,
            "не хватило памяти рядом со старой версией: старая выгружена на время загрузки",
        );
        let evicted = match self.load(area) {
            Ok((module, heap)) => match self.kernel.restore(evicted, module) {
                Ok(h) => return Ok((h, heap)),
                Err(e) => {
                    self.slots[index].handle = None;
                    return Err(e);
                }
            },
            Err(_) => evicted,
        };
        let previous = self.storage.area(self.slots[index].at).cloned();
        match previous.map(|a| self.load(&a)) {
            Some(Ok((module, heap))) => match self.kernel.restore(evicted, module) {
                Ok(h) => {
                    self.slots[index].handle = Some(h);
                    self.slots[index].heap = heap;
                }
                Err(_) => self.slots[index].handle = None,
            },
            _ => {
                self.kernel.abandon(evicted);
                self.slots[index].handle = None;
            }
        }
        Err(err)
    }

    /// Starts the module of the area at `at`. A running module with the same name is replaced
    /// without touching its outputs; if the new one does not start, the old one keeps running.
    /// `by_hand` (upload, `swap`, `reset`, boot) starts the restart policy afresh; a restart by
    /// the policy keeps its pause growth, fault count, pause flag and reported pins.
    fn start(&mut self, at: usize, by_hand: bool) -> Result<(), String> {
        // Loading and starting a module is long and known: the scan watchdog waits longer.
        watchdog::hold(watchdog::HOLD_MS);
        let area = self
            .storage
            .area(at)
            .cloned()
            .ok_or_else(|| "образ модуля не найден".to_owned())?;
        let index = self.slot(&area.meta.name);
        let running = index.and_then(|i| self.slots[i].handle);
        let queue = index.map_or(0, |i| self.slots[i].queue);
        let (handle, heap) = match (self.load(&area), running, index) {
            (Ok((module, heap)), Some(old), _) => {
                (self.kernel.swap(old, module, &[], queue)?, heap)
            }
            (Ok((module, heap)), None, _) => (self.kernel.add(module, &[], queue)?, heap),
            (Err(e), Some(old), Some(i)) => self.load_without_old(old, i, &area, e)?,
            (Err(e), _, _) => return Err(e),
        };
        let mut slot = Slot::new(area.meta.name, at, Some(handle), heap);
        slot.queue = queue;
        let now = devices::uptime_ms();
        if let Some(old) = index.map(|i| &mut self.slots[i]) {
            slot.restart = core::mem::take(&mut old.restart);
            if !by_hand {
                slot.faults = old.faults;
                slot.paused = old.paused;
                slot.shared_pins = core::mem::take(&mut old.shared_pins);
            }
        }
        if by_hand {
            slot.restart.started_by_hand(now);
        } else {
            slot.restart.started(now);
        }
        match index {
            Some(i) => self.slots[i] = slot,
            None => self.slots.push(slot),
        }
        self.layout = self.layout.wrapping_add(1);
        Ok(())
    }

    /// One step of a module with its statistics. An error stops the module until its restart,
    /// so every error goes to the journal (at most one per pause).
    /// In IRAM with the rest of the hot path (see `engine::native`).
    #[link_section = ".iram1.crab_step"]
    fn step(&mut self, index: usize, dt_ms: u32) -> bool {
        let Some(handle) = self.slots[index].handle else {
            return false;
        };
        let begin = Instant::now();
        let leases_before = self.kernel.get(handle).map_or(0, |m| m.host().leases.len());
        if let Some(m) = self.kernel.get_mut(handle) {
            m.host_mut().now_ms = devices::millis() as u32;
        }
        let result = self.kernel.step(handle, dt_ms);
        let us = begin.elapsed().as_micros() as u32;
        let lines = self
            .kernel
            .get_mut(handle)
            .map(|m| core::mem::take(&mut m.host_mut().log))
            .unwrap_or_default();
        // Pins this step took: reported once when another module holds them too.
        let mut taken = Vec::new();
        if let Some(m) = self.kernel.get(handle) {
            let leases = &m.host().leases;
            if leases.len() > leases_before {
                taken = leases.iter().map(|l| l.pin()).collect();
            }
        }
        let shared: Vec<(u8, Handle)> = taken
            .into_iter()
            .filter(|p| !self.slots[index].shared_pins.contains(p))
            .filter_map(|p| Some((p, self.kernel.holder_of(p, handle)?)))
            .collect();
        let mut news = Vec::new();
        for (pin, other) in shared {
            let owner = self
                .slots
                .iter()
                .find(|s| s.handle == Some(other))
                .map_or("?", |s| s.name.as_str());
            news.push(format!("пин {pin} уже занят модулем «{owner}»"));
            self.slots[index].shared_pins.push(pin);
        }
        let slot = &mut self.slots[index];
        if let Some(last) = slot.last_step.replace(begin) {
            let period = begin.duration_since(last).as_micros() as u32;
            slot.period_max_us = slot.period_max_us.max(period);
        }
        slot.steps = slot.steps.wrapping_add(1);
        slot.last_us = us;
        slot.max_us = slot.max_us.max(us);
        slot.sum_us = slot.sum_us.wrapping_add(us);
        if let Err(e) = &result {
            slot.faults = slot.faults.wrapping_add(1);
            let pause = slot.restart.failed(devices::uptime_ms());
            news.push(format!(
                "ошибка шага: {e}; перезапуск через {} с",
                pause.div_ceil(1000)
            ));
        }
        // The name is copied only when there is something to log: no heap work on a quiet step.
        if !lines.is_empty() || !news.is_empty() {
            let name = slot.name.clone();
            for line in lines.into_iter().chain(news) {
                self.log(&name, &line);
            }
        }
        result.is_ok()
    }

    /// A fresh instance of a module whose pause is over.
    fn restart_failed(&mut self, index: usize, now: u64) {
        let name = self.slots[index].name.clone();
        match self.start(self.slots[index].at, false) {
            Ok(()) => self.log(&name, "перезапущен после ошибки"),
            Err(e) => {
                // `started` ends the wait, `failed` begins the next one with a longer pause:
                // a module that does not start is not retried every scan.
                let restart = &mut self.slots[index].restart;
                restart.started(now);
                let pause = restart.failed(now);
                let text = format!(
                    "не удалось перезапустить: {e}; повтор через {} с",
                    pause.div_ceil(1000)
                );
                self.log(&name, &text);
            }
        }
    }

    fn scan(&mut self, dt_ms: u32) {
        for i in 0..self.slots.len() {
            let now = devices::uptime_ms();
            let restart = &self.slots[i].restart;
            if restart.waiting() {
                if restart.due(now) {
                    self.restart_failed(i, now);
                }
            } else if !self.slots[i].paused {
                self.step(i, dt_ms);
            }
        }
    }

    fn put(&mut self, name: String, len: usize, crc: u32, allow: String, memory: Option<u32>) {
        if self.upload.is_some() {
            println!("err busy");
            return;
        }
        if len == 0 {
            println!("err bad put");
            return;
        }
        match self.storage.reserve(len) {
            Ok(at) => {
                self.upload = Some(Upload {
                    meta: Meta {
                        name,
                        len,
                        crc,
                        mode: ExecutionMode::Interpreter,
                        memory: memory.unwrap_or(0),
                        allow,
                    },
                    at,
                    erase_at: at,
                    erase_end: at + Storage::<PartitionFlash>::area_size(len),
                    received: 0,
                    crc: 0,
                    last_rx: Instant::now(),
                })
            }
            Err(e) => println!("err {e}"),
        }
    }

    /// Moves an upload on between scans: one sector erased (the flash stops the CPU for the
    /// time of an erase, so not all at once), then `ready`; later the timeout of a chunk.
    fn upload_step(&mut self) {
        let Some(mut u) = self.upload.take() else {
            return;
        };
        if u.erase_at < u.erase_end {
            if self.storage.erase_sector(u.erase_at).is_err() {
                println!("err flash");
                return;
            }
            u.erase_at += SECTOR;
            if u.erase_at == u.erase_end {
                // The parser switches before `ready` goes out: the image may follow at once.
                io::with_input(|i| i.expect_body(u.meta.len, CHUNK));
                println!("ready {CHUNK}");
                u.last_rx = Instant::now();
            }
        } else if u.last_rx.elapsed() > UPLOAD_TIMEOUT {
            // No header was written: the sectors stay free.
            println!("err timeout");
            io::with_input(|i| i.cancel_body());
            return;
        }
        self.upload = Some(u);
    }

    fn chunk(&mut self, data: &[u8]) {
        let Some(mut u) = self.upload.take() else {
            return;
        };
        if u.received == 0 {
            u.meta.mode = match data.get(..4) {
                Some(b"\0asm") => ExecutionMode::Interpreter,
                Some(b"\0aot") => ExecutionMode::AotXip,
                _ => {
                    println!("err not a module");
                    io::with_input(|i| i.cancel_body());
                    return;
                }
            };
        }
        if self.storage.write(u.at, u.received, data).is_err() {
            println!("err flash");
            io::with_input(|i| i.cancel_body());
            return;
        }
        u.crc = crc32_update(u.crc, data);
        u.received += data.len();
        u.last_rx = Instant::now();
        println!("ack {}", u.received);
        if u.received < u.meta.len {
            self.upload = Some(u);
            return;
        }
        if u.crc != u.meta.crc {
            println!("err crc/len");
            return;
        }
        let name = u.meta.name.clone();
        let old_at = self.slot(&name).map(|i| self.slots[i].at);
        if let Err(e) = self.storage.commit(u.at, u.meta) {
            println!("err {e}");
            return;
        }
        self.layout = self.layout.wrapping_add(1);
        let begin = Instant::now();
        let result = self.start(u.at, true);
        println!("swap_us={}", begin.elapsed().as_micros());
        match result {
            Ok(()) => {
                // The old instance is gone, so its image may go too.
                if let Some(old) = old_at.filter(|&old| old != u.at) {
                    self.storage.remove(old);
                }
                println!("ok");
            }
            Err(e) => {
                self.log(&name, &e);
                self.storage.remove(u.at);
                let old_runs = self
                    .slot(&name)
                    .is_some_and(|i| self.slots[i].handle.is_some());
                let note = if old_runs { ", old version runs" } else { "" };
                println!("err not started{note}: {e}");
            }
        }
    }

    fn delete(&mut self, name: &str) {
        let Some(index) = self.slot(name) else {
            println!("err");
            return;
        };
        let slot = self.slots.remove(index);
        if let Some(handle) = slot.handle {
            // Its outputs go to their safe state; the instance is dropped before its image.
            drop(self.kernel.unload(handle));
        }
        self.storage.remove(slot.at);
        self.layout = self.layout.wrapping_add(1);
        println!("ok");
    }

    fn restart(&mut self, name: &str) {
        let Some(index) = self.slot(name) else {
            println!("err");
            return;
        };
        match self.start(self.slots[index].at, true) {
            Ok(()) => println!("ok"),
            Err(e) => {
                self.log(name, &e);
                println!("err");
            }
        }
    }

    /// The host of a running module.
    fn host_of(&mut self, name: &str) -> Option<&mut ModuleHost<BoardDevices>> {
        let handle = self.slots[self.slot(name)?].handle?;
        Some(self.kernel.get_mut(handle)?.host_mut())
    }

    /// `in` and `param`: the editor sets a value of a module by its index.
    fn set_word(&mut self, name: &str, index: usize, word: u32, param: bool) {
        let Some(host) = self.host_of(name) else {
            println!("err no such module");
            return;
        };
        let words = if param {
            &mut host.params
        } else {
            &mut host.inputs
        };
        // Empty by default; a module reads 0 beyond the end.
        if words.len() < MAX_SLOTS {
            words.resize(MAX_SLOTS, Word::default());
        }
        words[index] = Word(word);
        println!("ok");
    }

    fn force(&mut self, name: &str, index: usize, word: u32) {
        let Some(host) = self.host_of(name) else {
            println!("err no such module");
            return;
        };
        if host.force(index, Word(word)) {
            println!("ok");
        } else {
            println!("err bad index or type");
        }
    }

    /// `index` is `None` for all of them.
    fn unforce(&mut self, name: &str, index: Option<usize>) {
        let Some(host) = self.host_of(name) else {
            println!("err no such module");
            return;
        };
        match index {
            Some(i) if !host.unforce(i) => println!("err bad index"),
            Some(_) => println!("ok"),
            None => {
                host.unforce_all();
                println!("ok");
            }
        }
    }

    /// A fresh instance from the stored image: state, forced values and the pause are gone.
    fn reset(&mut self, name: &str) {
        let Some(index) = self.slot(name).filter(|&i| self.slots[i].handle.is_some()) else {
            println!("err no such module");
            return;
        };
        match self.start(self.slots[index].at, true) {
            Ok(()) => println!("ok"),
            Err(e) => {
                self.log(name, &e);
                println!("err not started (see log)");
            }
        }
    }

    /// Outputs every running module holds, then the levels the pins are driven to.
    fn pins(&self) {
        let out = devices::out_register();
        for s in &self.slots {
            let Some(host) = s.handle.and_then(|h| self.kernel.get(h)).map(|m| m.host()) else {
                continue;
            };
            for lease in host.leases.iter() {
                let (pin, use_) = (lease.pin(), lease.use_());
                let word = match use_ {
                    PinUse::Out => "out",
                    PinUse::Pwm => "pwm",
                    PinUse::Dac => "dac",
                };
                let value;
                let fault = match lease.fault() {
                    Fault::Low => "0",
                    Fault::High => "1",
                    Fault::Hold => "hold",
                    Fault::Value(code) => {
                        value = code.to_string();
                        value.as_str()
                    }
                };
                let level = (use_ == PinUse::Out && pin < 32).then(|| out >> pin & 1 != 0);
                println!("{}", format_pin(pin, &s.name, word, fault, level));
            }
        }
        println!("out={out:08x}");
        // The codes the DAC outputs (`-` for a channel not set up).
        #[cfg(not(target_arch = "riscv32"))]
        {
            let code = |pin| analog::dac_code(pin).map_or("-".to_string(), |c| c.to_string());
            println!("dac={},{}", code(25), code(26));
        }
        println!("end");
    }

    fn tick(&mut self, name: &str, count: u32, dt_ms: u32) {
        let Some(index) = self.slot(name) else {
            println!("err no such module");
            return;
        };
        if count > MAX_TICKS {
            println!("err bad tick");
            return;
        }
        self.slots[index].paused = true;
        // A module waiting for its restart has a broken instance: not stepped.
        let ok = self.slots[index].handle.is_some()
            && !self.slots[index].restart.waiting()
            && (0..count).all(|_| self.step(index, dt_ms));
        println!("{}", if ok { "ok" } else { "err step failed" });
    }

    /// `bus`: the queues of the modules, then the topics.
    fn bus(&self) {
        let bus = self.kernel.bus();
        for s in &self.slots {
            let Some(h) = s.handle else { continue };
            let cap = bus.queue_cap(h).unwrap_or(0);
            println!("{}", format_bus_module(&s.name, cap, bus.lost(h)));
        }
        for t in bus.topics() {
            println!(
                "{}",
                format_topic(
                    t.name,
                    t.kind == TopicKind::State,
                    t.max_len,
                    t.refused,
                    t.last
                )
            );
        }
        println!("ok");
    }

    fn topic(&mut self, name: &str, state: bool, max_len: usize) {
        let kind = if state {
            TopicKind::State
        } else {
            TopicKind::Event
        };
        match self.kernel.bus_mut().configure(name, kind, max_len) {
            Ok(()) => println!("ok"),
            Err(_) => println!("err too many topics"),
        }
    }

    fn queue(&mut self, name: &str, cap: usize) {
        let Some(index) = self.slot(name) else {
            println!("err no such module");
            return;
        };
        self.slots[index].queue = cap;
        if let Some(h) = self.slots[index].handle {
            self.kernel.bus_mut().set_queue(h, cap);
        }
        println!("ok");
    }

    fn command(&mut self, line: &str) {
        match parse_command(line) {
            Ok(Command::Put {
                name,
                len,
                crc32,
                allow,
                memory,
            }) => self.put(name, len, crc32, allow, memory),
            // Answered by `crab-io`; here only if handed over anyway.
            Ok(Command::List) => snapshot::answer_list(),
            Ok(Command::Info) => snapshot::answer_info(),
            Ok(Command::Tick { name, count, dt_ms }) => self.tick(&name, count, dt_ms),
            Ok(Command::Delete { name }) => self.delete(&name),
            Ok(Command::Swap { name }) => self.restart(&name),
            Ok(Command::ResetMax) => {
                for s in &mut self.slots {
                    s.max_us = 0;
                    s.period_max_us = 0;
                }
                self.cycle.reset_stats();
                println!("ok");
            }
            Ok(Command::SetInput { name, index, word }) => self.set_word(&name, index, word, false),
            Ok(Command::SetParam { name, index, word }) => self.set_word(&name, index, word, true),
            Ok(Command::Outputs { name, count }) => snapshot::answer_outputs(&name, count),
            Ok(Command::Events { name }) => snapshot::answer_events(&name),
            Ok(Command::Force { name, index, word }) => self.force(&name, index, word),
            Ok(Command::Unforce { name, index }) => self.unforce(&name, index),
            Ok(Command::Reset { name }) => self.reset(&name),
            Ok(Command::Pins) => self.pins(),
            Ok(Command::SpDump { ts_ms }) => sparkplug::dump(ts_ms),
            Ok(Command::PulseTest { pin }) => pulse::self_test(pin),
            Ok(Command::PulseBench { pin, runs }) => pulse::bench(pin, runs),
            Ok(Command::Topic {
                name,
                state,
                max_len,
            }) => self.topic(&name, state, max_len),
            Ok(Command::Queue { name, cap }) => self.queue(&name, cap),
            Ok(Command::Bus) => self.bus(),
            Ok(Command::Log) => snapshot::answer_log(),
            // Answered by `crab-io`; here only if handed over anyway.
            Ok(Command::CoreInfo) => coredump::answer_info(),
            Ok(Command::CoreRead { offset }) => coredump::answer_read(offset),
            Ok(Command::CoreErase) => coredump::answer_erase(),
            // Never a panic in the scan: the test crash is made by `crab-io` only.
            // Only test builds crash on purpose (in `crab-io`); a released master refuses it.
            Ok(Command::SelfTestPanic) => println!("err selftest is not in this build"),
            // Handled by `crab-io` (USB only); here only if handed over anyway.
            Ok(Command::NetKey(_)) => println!("err netkey only over USB"),
            // A hang of the scan on purpose: the scan watchdog must restart the board.
            #[cfg(feature = "selftest")]
            Ok(Command::SelfTestHang) => {
                println!("ok");
                // The normal deadline, not the longer one commands get.
                watchdog::feed();
                loop {
                    core::hint::spin_loop();
                }
            }
            #[cfg(not(feature = "selftest"))]
            Ok(
                Command::SelfTestHang | Command::SelfTestPwm { .. } | Command::SelfTestDac { .. },
            ) => {
                println!("err selftest is not in this build")
            }
            #[cfg(feature = "selftest")]
            Ok(Command::SelfTestPwm { pin, hz, pin2 }) => {
                if count::selftest_pwm(pin, hz, pin2) {
                    println!("ok");
                } else {
                    println!("err pwm not started");
                }
            }
            #[cfg(feature = "selftest")]
            Ok(Command::SelfTestDac { pin, code }) => {
                #[cfg(not(target_arch = "riscv32"))]
                match analog::selftest_dac(pin.clamp(0, 255) as u8, code.clamp(0, 255) as u8) {
                    Some((raw, mv)) => println!("ok raw={raw} mv={mv}"),
                    None => println!("err no dac on this pin"),
                }
                #[cfg(target_arch = "riscv32")]
                {
                    let _ = (pin, code);
                    println!("err no dac on this chip");
                }
            }
            // The answers the editor knows from the C++ master.
            Err(
                e @ (ParseError::BadIndexOrType
                | ParseError::BadIndex
                | ParseError::BadCount
                | ParseError::BadBus),
            ) => {
                println!("err {e}")
            }
            Err(_) => println!("err {}", fallback(line)),
        }
    }

    /// The period the running modules ask for (the shortest).
    fn wanted_period(&self) -> u32 {
        cycle::period_of(
            self.slots
                .iter()
                .filter(|s| !s.paused && !s.restart.waiting())
                .filter_map(|s| s.handle.and_then(|h| self.kernel.get(h)))
                .map(|m| m.period_ms()),
        )
    }

    /// Overruns go to the journal at most every [`OVERRUN_NOTE_MS`] while they grow.
    fn note_overruns(&mut self) {
        let over = self.cycle.overruns;
        if over < self.overruns_noted {
            // `rmax` cleared the count.
            self.overruns_noted = over;
            return;
        }
        let now = devices::uptime_ms();
        if over == self.overruns_noted || now < self.overruns_noted_at + OVERRUN_NOTE_MS {
            return;
        }
        let text = format!(
            "шаг дольше периода {} мс: переполнений {} (+{})",
            self.cycle.period_ms(),
            over,
            over.wrapping_sub(self.overruns_noted)
        );
        self.overruns_noted = over;
        self.overruns_noted_at = now;
        self.log("cycle", &text);
    }

    /// Everything between two steps: the commands handed over by `crab-io`, the upload, notes of
    /// the network.
    fn between_steps(&mut self) {
        let events = io::take_for_scan();
        if !events.is_empty() || self.upload.is_some() {
            // Commands and image chunks may write flash: longer than a step may take.
            watchdog::hold(watchdog::HOLD_MS);
        }
        for event in events {
            match event {
                Event::Line(line) => self.command(&line),
                Event::Chunk(data) => self.chunk(&data),
            }
        }
        self.upload_step();
        for text in net::take_notes() {
            self.log("net", &text);
        }
        for text in eth::take_notes() {
            self.log("eth", &text);
        }
        for text in mqtt::take_notes() {
            self.log("mqtt", &text);
        }
        for (source, text) in notes::take() {
            self.log(source, &text);
        }
        self.note_overruns();
    }

    /// Publishes what the debugger reads; skipped (one step later) when `crab-io` holds the
    /// snapshot just now: the scan never waits for it. Nothing is allocated unless modules started
    /// or went since the last time.
    #[link_section = ".iram1.crab_publish"]
    fn publish(&mut self) {
        let Some(mut snap) = snapshot::SNAP.try_lock() else {
            return;
        };
        if snap.layout != self.layout {
            self.rebuild(&mut snap);
        }
        for (view, s) in snap.modules.iter_mut().zip(&self.slots) {
            let module = s.handle.and_then(|h| self.kernel.get_mut(h));
            let mut st = snapshot::Stats {
                run: s.handle.is_some() && !s.restart.waiting(),
                steps: s.steps,
                restarts: s.restart.restarts(),
                last_us: s.last_us,
                max_us: s.max_us,
                period_max_us: s.period_max_us,
                sum_us: s.sum_us,
                heap: s.heap,
                ..Default::default()
            };
            if let Some(m) = module {
                st.mem = m.memory_len();
                let host = m.host_mut();
                view.outputs.clear();
                view.outputs.extend(host.outputs.iter().map(|w| w.0));
                let lost = host.move_events(&mut view.events, snapshot::EVENTS);
                view.lost = view.lost.saturating_add(lost);
            }
            st.qlost = s.handle.map_or(0, |h| self.kernel.bus().lost(h));
            view.stats = st;
        }
        match &mut snap.cycle {
            Some(c) => c.clone_from(&self.cycle),
            None => snap.cycle = Some(self.cycle.clone()),
        }
    }

    /// The parts of the snapshot that change only when modules start or go.
    fn rebuild(&self, snap: &mut snapshot::Snapshot) {
        snap.modules = self
            .slots
            .iter()
            .map(|s| {
                let module = s.handle.and_then(|h| self.kernel.get(h));
                let (len, crc, stored) = self.storage.area(s.at).map_or((0, 0, "-"), |a| {
                    (a.meta.len, a.meta.crc, a.meta.mode.as_str())
                });
                let mode = module.map_or(stored, |m| m.mode().as_str());
                snapshot::ModuleView::new(s.name.clone(), module.is_some(), len, crc, mode)
            })
            .collect();
        snap.areas = self
            .storage
            .areas()
            .iter()
            .map(|a| snapshot::AreaView {
                name: a.meta.name.clone(),
                at: a.at,
                len: a.meta.len,
                gen: a.gen,
                mode: a.meta.mode.as_str(),
                memory: a.meta.memory,
                allow: a.meta.allow.clone(),
            })
            .collect();
        snap.storage = (
            self.storage.free(),
            self.storage.capacity(),
            self.storage.largest(),
        );
        snap.bad_boots = self.bad_boots;
        snap.layout = self.layout;
    }

    fn run(&mut self) -> ! {
        let (mut ticker, error) = tick::Ticker::new(self.cycle.period_ms());
        if let Some(e) = error {
            self.log(
                "cycle",
                &format!("{e}; период по тикам FreeRTOS, сторож шага не работает"),
            );
        }
        watchdog::set_period(self.cycle.period_ms());
        loop {
            ticker.wait();
            watchdog::feed();
            let dt = self.cycle.begin(tick::now_us());
            self.scan(dt);
            let overran = self.cycle.end(tick::now_us());
            self.publish();
            if overran {
                // As a PLC: a step that ran over waits for the next mark of the grid, the marks
                // passed during it are dropped. The wait also lets the idle task of the core run.
                ticker.skip();
            }
            let busy = tick::now_us();
            self.between_steps();
            self.cycle
                .between(tick::now_us().saturating_sub(busy).min(u32::MAX as u64) as u32);
            let want = self.wanted_period();
            if want != self.cycle.period_ms() {
                self.cycle.set_period(want);
                ticker.set_period(want);
                watchdog::set_period(want);
            }
        }
    }
}

/// Starts a background thread of the master (network, Modbus, expanders, the port, I²C writes).
/// On the dual-core ESP32 it runs on core 0: between steps a thread on the scan core would run its
/// code from flash there and push the code of the sheets out of that core's cache (a step with
/// Wi-Fi and MQTT on was 2.5× longer). `name` ends with a NUL.
pub(crate) fn spawn_background<F>(name: &'static [u8], stack: usize, f: F) -> bool
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(target_arch = "xtensa")]
    let saved = {
        use esp_idf_svc::hal::{cpu::Core, task::thread::ThreadSpawnConfiguration};
        let saved = ThreadSpawnConfiguration::get();
        let conf = ThreadSpawnConfiguration {
            name: Some(name),
            pin_to_core: Some(Core::Core0),
            ..Default::default()
        };
        let _ = conf.set();
        saved
    };
    let label = core::str::from_utf8(name.strip_suffix(b"\0").unwrap_or(name)).unwrap_or("crab");
    let spawned = std::thread::Builder::new()
        .name(label.into())
        .stack_size(stack)
        .spawn(f);
    #[cfg(target_arch = "xtensa")]
    {
        // Back to what the calling thread had (none: the defaults).
        let _ = saved.unwrap_or_default().set();
    }
    spawned.is_ok()
}

pub fn main() {
    sys::link_patches();
    // On the dual-core ESP32 the sheets run on the APP core, away from Wi-Fi, lwIP and the
    // Ethernet driver on core 0 (a bench showed a 3.6× longer step with Wi-Fi and MQTT on).
    // The scan runs at a high priority on both chips (`specs/fast-cycle`).
    {
        use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;
        let conf = ThreadSpawnConfiguration {
            name: Some(b"crab-runtime\0"),
            #[cfg(target_arch = "xtensa")]
            pin_to_core: Some(esp_idf_svc::hal::cpu::Core::Core1),
            priority: SCAN_PRIORITY,
            ..Default::default()
        };
        if let Err(e) = conf.set() {
            println!("warn runtime thread: {e}");
        }
    }
    let runtime = std::thread::Builder::new()
        .stack_size(RUNTIME_STACK)
        .spawn(|| {
            let started = usb::init().and_then(|()| {
                coredump::init();
                Master::new()
            });
            match started {
                Ok(mut master) => {
                    println!("master ready");
                    if !io::start() {
                        println!("err io thread");
                    }
                    if !netcmd::start() {
                        println!("err netcmd thread");
                    }
                    master.run()
                }
                Err(e) => println!("err {e}"),
            }
        })
        .expect("runtime thread");
    let _ = runtime.join();
}
