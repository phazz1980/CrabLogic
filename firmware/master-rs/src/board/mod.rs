//! The board side of the Rust master (ESP-IDF): the kernel runs the modules stored in the
//! `crabmod` partition on WAMR, and the USB console takes the commands of the editor
//! (`crab_master_rs::protocol`).
//!
//! One thread does everything in turn: reads the port, moves an upload one step further (a
//! flash sector erased or a chunk written), scans the modules. Nothing is printed on its own:
//! every line is an answer to a command, module errors wait in the journal (`log`).
//! A module whose step failed stops (its outputs are safe) and starts again from its stored
//! image after a growing pause (`restart`).
//!
//! Commands: `put`, `list`, `info`, `tick`, `del`, `swap`, `reset`, `rmax`, `log`; live
//! data of a module: `in`, `param`, `out`, `ev`, `force`, `unforce`, `pins`.

mod devices;
mod engine;
mod flash;
mod net;
mod usb;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crab_kernel::bus::TopicKind;
use crab_kernel::engine::{Instance, Limits};
use crab_kernel::handle::Handle;
use crab_kernel::host::{ModuleHost, Word};
use crab_kernel::lease::{Fault, PinUse};
use crab_kernel::Kernel;
use crab_master_rs::boot_guard::{self, Counter, Reset};
use crab_master_rs::protocol::{
    format_bus_module, format_events, format_outputs, format_pin, format_topic, parse_command,
    Command, Event, Input, ParseError, MAX_SLOTS,
};
use crab_master_rs::restart::Restart;
use crab_master_rs::storage::{crc32_update, Area, Meta, Storage, SECTOR};
use crab_master_rs::wamr::{crab_mem, ExecutionMode};
use esp_idf_svc::sys;

use devices::BoardDevices;
use engine::{Image, Wamr, WamrInstance};
use flash::PartitionFlash;

/// Scan period of the kernel, milliseconds.
const QUANTUM_MS: u32 = 10;
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
/// Lines the journal keeps.
const JOURNAL: usize = 16;
/// The CPU the AOT images must be built for, as `modcaps=` names it.
#[cfg(target_arch = "riscv32")]
const AOT_CPU: &str = "rv32imc";

type Module = WamrInstance<ModuleHost<BoardDevices>>;

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
        "ev" | "reset" if line.contains(' ') => "no such module",
        _ => "unknown command",
    }
}

struct Master {
    wamr: Wamr,
    kernel: Kernel<Module, BoardDevices>,
    storage: Storage<PartitionFlash>,
    slots: Vec<Slot>,
    input: Input,
    upload: Option<Upload>,
    /// Module errors and `log` lines of modules: (milliseconds since start, text).
    journal: VecDeque<(u32, String)>,
    started: Instant,
    /// Bad resets in a row before this start; at [`boot_guard::SAFE_AFTER`] the stored modules
    /// are not started (safe mode).
    bad_boots: u32,
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
    unsafe { (*core::ptr::addr_of_mut!(BOOT_COUNTER)).boot(why) }
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
            input: Input::new(),
            upload: None,
            journal: VecDeque::new(),
            started: Instant::now(),
            bad_boots: count_boot(),
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
        if self.journal.len() == JOURNAL {
            self.journal.pop_front();
        }
        let ms = self.started.elapsed().as_millis() as u32;
        self.journal.push_back((ms, format!("[{name}] {text}")));
    }

    fn slot(&self, name: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.name == name)
    }

    /// Loads the image of an area; returns the instance and the heap it took.
    fn load(&self, area: &Area) -> Result<(Module, u32), String> {
        let before = free_heap();
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
                Image::Wasm(bytes.to_vec())
            }
            ExecutionMode::AotXip => Image::AotXip(bytes),
        };
        let limits = Limits {
            max_memory: MAX_MEMORY,
            ..Limits::default()
        };
        let host = ModuleHost::new(BoardDevices, OUTPUTS);
        let module = self
            .wamr
            .load_image(image, memory, &area.allow_list(), limits, host)?;
        Ok((module, before.saturating_sub(free_heap())))
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
        Ok(())
    }

    /// One step of a module with its statistics. An error stops the module until its restart,
    /// so every error goes to the journal (at most one per pause).
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
        let name = slot.name.clone();
        for line in lines.into_iter().chain(news) {
            self.log(&name, &line);
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
                println!("ready {CHUNK}");
                self.input.expect_body(u.meta.len, CHUNK);
                u.last_rx = Instant::now();
            }
        } else if u.last_rx.elapsed() > UPLOAD_TIMEOUT {
            // No header was written: the sectors stay free.
            println!("err timeout");
            self.input.cancel_body();
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
                    self.input.cancel_body();
                    return;
                }
            };
        }
        if self.storage.write(u.at, u.received, data).is_err() {
            println!("err flash");
            self.input.cancel_body();
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

    fn outputs(&mut self, name: &str, count: usize) {
        let Some(host) = self.host_of(name) else {
            println!("err no such module");
            return;
        };
        let raw: Vec<u32> = host.outputs.iter().map(|w| w.0).collect();
        println!("{}", format_outputs(&raw, count));
        println!("ok");
    }

    fn events(&mut self, name: &str) {
        let Some(host) = self.host_of(name) else {
            println!("err no such module");
            return;
        };
        let (events, lost) = host.take_events();
        let events: Vec<(u8, u8, u32)> = events.iter().map(|e| (e.slot, e.value, e.ms)).collect();
        println!(
            "{}",
            format_events(name, devices::millis() as u32, lost, &events)
        );
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
                };
                let fault = match lease.fault() {
                    Fault::Low => "0",
                    Fault::High => "1",
                    Fault::Hold => "hold",
                };
                let level = (use_ == PinUse::Out && pin < 32).then(|| out >> pin & 1 != 0);
                println!("{}", format_pin(pin, &s.name, word, fault, level));
            }
        }
        println!("out={out:08x}");
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

    fn list(&self) {
        for s in &self.slots {
            let module = s.handle.and_then(|h| self.kernel.get(h));
            let memory = module.map_or(0, |m| m.memory_len());
            let out0 = module
                .and_then(|m| m.host().outputs.first())
                .map_or(0, |w| w.0);
            let lost = s.handle.map_or(0, |h| self.kernel.bus().lost(h));
            let (len, crc, stored) = self.storage.area(s.at).map_or((0, 0, "-"), |a| {
                (a.meta.len, a.meta.crc, a.meta.mode.as_str())
            });
            let mode = module.map_or(stored, |m| m.mode().as_str());
            println!(
                "{} {} steps={} restarts={} out0={:08x} us={}/{}/{}/{} qlost={} mem={} code={} wasm={} mode={} crc={:08x}",
                s.name,
                if s.handle.is_some() && !s.restart.waiting() {
                    "run"
                } else {
                    "stop"
                },
                s.steps,
                s.restart.restarts(),
                out0,
                s.last_us,
                s.max_us,
                s.period_max_us,
                s.sum_us,
                lost,
                memory,
                s.heap.saturating_sub(memory),
                len,
                mode,
                crc
            );
        }
        // What this master has of the features of the C++ one: step times, modules in flash,
        // ABI 4, output events (`ev`); `aot` is its own: it takes XIP AOT images, built for the CPU
        // named next to it
        // (the editor does not compile for a CPU it does not see here).
        println!("caps=us,ev,bus");
        println!("modcaps=flashmod,abi4,aot,{AOT_CPU}");
        // The Rust master is not built per project: the editor reads this prefix as "current".
        println!("id=rs-{}", env!("CARGO_PKG_VERSION"));
        // Safe mode after a boot loop: the editor warns that the stored modules do not run.
        if boot_guard::safe(self.bad_boots) {
            println!("safe={}", self.bad_boots);
        }
        // SAFETY: plain queries of the heap.
        let (largest, min) = unsafe {
            (
                sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_8BIT),
                sys::esp_get_minimum_free_heap_size(),
            )
        };
        println!("free={} largest={largest} min={min}", free_heap());
        println!("end");
    }

    fn info(&self) {
        println!("master=crab-master-rs {}", env!("CARGO_PKG_VERSION"));
        println!(
            "crabmod={}/{} largest={}",
            self.storage.free(),
            self.storage.capacity(),
            self.storage.largest()
        );
        for a in self.storage.areas() {
            let allow = if a.meta.allow.is_empty() {
                "-"
            } else {
                &a.meta.allow
            };
            println!(
                "{} at={} len={} gen={} mode={} mem={} allow={allow}",
                a.meta.name,
                a.at,
                a.meta.len,
                a.gen,
                a.meta.mode.as_str(),
                a.meta.memory
            );
        }
        println!("end");
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
            Ok(Command::List) => self.list(),
            Ok(Command::Info) => self.info(),
            Ok(Command::Tick { name, count, dt_ms }) => self.tick(&name, count, dt_ms),
            Ok(Command::Delete { name }) => self.delete(&name),
            Ok(Command::Swap { name }) => self.restart(&name),
            Ok(Command::ResetMax) => {
                for s in &mut self.slots {
                    s.max_us = 0;
                    s.period_max_us = 0;
                }
                println!("ok");
            }
            Ok(Command::SetInput { name, index, word }) => self.set_word(&name, index, word, false),
            Ok(Command::SetParam { name, index, word }) => self.set_word(&name, index, word, true),
            Ok(Command::Outputs { name, count }) => self.outputs(&name, count),
            Ok(Command::Events { name }) => self.events(&name),
            Ok(Command::Force { name, index, word }) => self.force(&name, index, word),
            Ok(Command::Unforce { name, index }) => self.unforce(&name, index),
            Ok(Command::Reset { name }) => self.reset(&name),
            Ok(Command::Pins) => self.pins(),
            Ok(Command::Topic {
                name,
                state,
                max_len,
            }) => self.topic(&name, state, max_len),
            Ok(Command::Queue { name, cap }) => self.queue(&name, cap),
            Ok(Command::Bus) => self.bus(),
            Ok(Command::Log) => {
                for (ms, text) in &self.journal {
                    println!("{ms} {text}");
                }
                println!("end");
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

    fn run(&mut self) -> ! {
        let mut buf = [0u8; CHUNK];
        let mut last_scan = Instant::now();
        loop {
            // Waiting for input is also the pause between scans.
            let since = last_scan.elapsed().as_millis() as u32;
            let n = usb::read(&mut buf, QUANTUM_MS.saturating_sub(since).max(1));
            if n > 0 {
                self.input.push(&buf[..n]);
                while let Some(event) = self.input.next_event() {
                    match event {
                        Event::Line(line) => self.command(&line),
                        Event::Chunk(data) => self.chunk(&data),
                    }
                }
            }
            self.upload_step();
            for text in net::take_notes() {
                self.log("net", &text);
            }
            let dt = last_scan.elapsed().as_millis() as u32;
            if dt >= QUANTUM_MS {
                last_scan += Duration::from_millis(dt as u64);
                self.scan(dt);
            }
        }
    }
}

pub fn main() {
    sys::link_patches();
    let runtime = std::thread::Builder::new()
        .stack_size(RUNTIME_STACK)
        .spawn(|| {
            let started = usb::init().and_then(|()| Master::new());
            match started {
                Ok(mut master) => {
                    println!("master ready");
                    master.run()
                }
                Err(e) => println!("err {e}"),
            }
        })
        .expect("runtime thread");
    let _ = runtime.join();
}
