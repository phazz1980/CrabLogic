//! The board side of the Rust master (ESP-IDF): the kernel runs the modules stored in the
//! `crabmod` partition on WAMR, and the USB console takes the commands of the editor
//! (`crab_master_rs::protocol`).
//!
//! One thread does everything in turn: reads the port, moves an upload one step further (a
//! flash sector erased or a chunk written), scans the modules. Nothing is printed on its own:
//! every line is an answer to a command, module errors wait in the journal (`log`).

mod devices;
mod engine;
mod flash;
mod usb;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crab_kernel::engine::{Instance, Limits};
use crab_kernel::handle::Handle;
use crab_kernel::host::ModuleHost;
use crab_kernel::Kernel;
use crab_master_rs::protocol::{parse_command, Command, Event, Input};
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
    last_error: String,
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
            last_error: String::new(),
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
        };
        let stored: Vec<(String, usize)> = master
            .storage
            .areas()
            .iter()
            .map(|a| (a.meta.name.clone(), a.at))
            .collect();
        for (name, at) in stored {
            if let Err(e) = master.start(at) {
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

    /// Starts the module of the area at `at`. A running module with the same name is replaced
    /// without touching its outputs; if the new one does not start, the old one keeps running.
    fn start(&mut self, at: usize) -> Result<(), String> {
        let area = self
            .storage
            .area(at)
            .cloned()
            .ok_or_else(|| "образ модуля не найден".to_owned())?;
        let (module, heap) = self.load(&area)?;
        let index = self.slot(&area.meta.name);
        let handle = match index.and_then(|i| self.slots[i].handle) {
            Some(old) => self.kernel.swap(old, module, &[], 0)?,
            None => self.kernel.add(module, &[], 0)?,
        };
        let slot = Slot::new(area.meta.name, at, Some(handle), heap);
        match index {
            Some(i) => self.slots[i] = slot,
            None => self.slots.push(slot),
        }
        Ok(())
    }

    /// One step of a module with its statistics; an error goes to the journal once.
    fn step(&mut self, index: usize, dt_ms: u32) -> bool {
        let Some(handle) = self.slots[index].handle else {
            return false;
        };
        let begin = Instant::now();
        let result = self.kernel.step(handle, dt_ms);
        let us = begin.elapsed().as_micros() as u32;
        let lines = self
            .kernel
            .get_mut(handle)
            .map(|m| core::mem::take(&mut m.host_mut().log))
            .unwrap_or_default();
        let slot = &mut self.slots[index];
        if let Some(last) = slot.last_step.replace(begin) {
            let period = begin.duration_since(last).as_micros() as u32;
            slot.period_max_us = slot.period_max_us.max(period);
        }
        slot.steps = slot.steps.wrapping_add(1);
        slot.last_us = us;
        slot.max_us = slot.max_us.max(us);
        slot.sum_us = slot.sum_us.wrapping_add(us);
        let name = slot.name.clone();
        let mut news = None;
        if let Err(e) = &result {
            slot.faults = slot.faults.wrapping_add(1);
            if slot.last_error != *e {
                slot.last_error = e.clone();
                news = Some(e.clone());
            }
        }
        for line in lines {
            self.log(&name, &line);
        }
        if let Some(e) = news {
            self.log(&name, &e);
        }
        result.is_ok()
    }

    fn scan(&mut self, dt_ms: u32) {
        for i in 0..self.slots.len() {
            if !self.slots[i].paused {
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
        let result = self.start(u.at);
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
        match self.start(self.slots[index].at) {
            Ok(()) => println!("ok"),
            Err(e) => {
                self.log(name, &e);
                println!("err");
            }
        }
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
        let ok = self.slots[index].handle.is_some() && (0..count).all(|_| self.step(index, dt_ms));
        println!("{}", if ok { "ok" } else { "err step failed" });
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
                if s.handle.is_some() { "run" } else { "stop" },
                s.steps,
                s.faults,
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
        // ABI 4; `aot` is its own: it takes XIP AOT images, built for the CPU named next to it
        // (the editor does not compile for a CPU it does not see here).
        println!("caps=us");
        println!("modcaps=flashmod,abi4,aot,{AOT_CPU}");
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
            Ok(Command::Log) => {
                for (ms, text) in &self.journal {
                    println!("{ms} {text}");
                }
                println!("end");
            }
            // The answers the editor knows from the C++ master.
            Err(_) if line.starts_with("put ") => println!("err bad put"),
            Err(_) if line.starts_with("tick ") => println!("err bad tick"),
            Err(_) => println!("err unknown command"),
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
