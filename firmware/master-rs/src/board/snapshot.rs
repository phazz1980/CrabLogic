//! What the debugger reads, published by the scan after every step (`specs/fast-cycle`): the
//! thread `crab-io` answers `list`, `info`, `out`, `ev` and `log` from here, so a poll of the
//! editor never takes time from the scan. The scan only `try_lock`s: it never waits for the
//! debugger, a busy snapshot is published one step later.

use std::collections::VecDeque;
use std::sync::Mutex;

use crab_kernel::host::OutputEvent;
use crab_master_rs::boot_guard;
use crab_master_rs::cycle::Cycle;
use crab_master_rs::protocol::{format_events, format_outputs};
use esp_idf_svc::sys;

use super::spin::{Guard, Spin};
use super::{devices, free_heap, retain, MODCAPS};

/// Lines the journal keeps.
pub const JOURNAL_LINES: usize = 16;
/// Output events kept per module until an `ev` takes them.
pub const EVENTS: usize = 64;

/// The figures of a module that change every step.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub run: bool,
    pub steps: u32,
    pub restarts: u32,
    pub last_us: u32,
    pub max_us: u32,
    pub period_max_us: u32,
    pub sum_us: u32,
    pub qlost: u32,
    pub mem: u32,
    pub heap: u32,
}

/// A module as the debugger sees it.
pub struct ModuleView {
    pub name: String,
    /// It has an instance (`out`, `ev` answer for it).
    pub live: bool,
    pub len: usize,
    pub crc: u32,
    pub mode: &'static str,
    pub stats: Stats,
    /// The output words, copied every step.
    pub outputs: Vec<u32>,
    pub events: VecDeque<OutputEvent>,
    pub lost: u32,
}

impl ModuleView {
    pub fn new(name: String, live: bool, len: usize, crc: u32, mode: &'static str) -> Self {
        Self {
            name,
            live,
            len,
            crc,
            mode,
            stats: Stats::default(),
            outputs: Vec::new(),
            events: VecDeque::with_capacity(EVENTS),
            lost: 0,
        }
    }
}

/// A stored area as `info` prints it.
pub struct AreaView {
    pub name: String,
    pub at: usize,
    pub len: usize,
    pub gen: u32,
    pub mode: &'static str,
    pub memory: u32,
    pub allow: String,
}

pub struct Snapshot {
    /// The layout of the master this snapshot was built for (modules started or removed).
    pub layout: u32,
    pub modules: Vec<ModuleView>,
    pub areas: Vec<AreaView>,
    /// Free, capacity and largest free block of the `crabmod` storage.
    pub storage: (usize, usize, usize),
    pub bad_boots: u32,
    pub cycle: Option<Cycle>,
}

pub static SNAP: Spin<Snapshot> = Spin::new(Snapshot {
    layout: u32::MAX,
    modules: Vec::new(),
    areas: Vec::new(),
    storage: (0, 0, 0),
    bad_boots: 0,
    cycle: None,
});

/// Module errors and `log` lines of modules: (milliseconds since start, text).
static JOURNAL: Mutex<VecDeque<(u32, String)>> = Mutex::new(VecDeque::new());

/// The scan only tries this lock (a step later when busy); `crab-io` waits for it.
fn snap() -> Guard<'static, Snapshot> {
    SNAP.lock_wait()
}

pub fn log(ms: u32, line: String) {
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    if j.len() == JOURNAL_LINES {
        j.pop_front();
    }
    j.push_back((ms, line));
}

/// `log`.
pub fn answer_log() {
    let lines: Vec<(u32, String)> = JOURNAL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    for (ms, text) in lines {
        println!("{ms} {text}");
    }
    println!("end");
}

/// `list`: the figures are copied under the lock and printed after it.
pub fn answer_list() {
    let (modules, bad_boots, cycle) = {
        let s = snap();
        let modules: Vec<(String, Stats, u32, usize, &'static str, u32)> = s
            .modules
            .iter()
            .map(|m| {
                let out0 = m.outputs.first().copied().unwrap_or(0);
                (m.name.clone(), m.stats, out0, m.len, m.mode, m.crc)
            })
            .collect();
        (modules, s.bad_boots, s.cycle.clone())
    };
    for (name, st, out0, len, mode, crc) in modules {
        println!(
            "{} {} steps={} restarts={} out0={:08x} us={}/{}/{}/{} qlost={} mem={} code={} wasm={} mode={} crc={:08x}",
            name,
            if st.run { "run" } else { "stop" },
            st.steps,
            st.restarts,
            out0,
            st.last_us,
            st.max_us,
            st.period_max_us,
            st.sum_us,
            st.qlost,
            st.mem,
            st.heap.saturating_sub(st.mem),
            len,
            mode,
            crc
        );
    }
    // What this master has of the features of the C++ one: step times, modules in flash,
    // output events (`ev`); `aot` is its own: it takes XIP AOT images, built for the CPU named
    // next to it (the editor does not compile for a CPU it does not see here).
    println!("caps=us,ev,bus");
    println!("modcaps={MODCAPS}");
    // The Rust master is not built per project: the editor reads this prefix as "current".
    println!("id=rs-{}", env!("CARGO_PKG_VERSION"));
    // Safe mode after a boot loop: the editor warns that the stored modules do not run.
    if boot_guard::safe(bad_boots) {
        println!("safe={bad_boots}");
    }
    // SAFETY: plain queries of the heap.
    let (largest, min) = unsafe {
        (
            sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_8BIT),
            sys::esp_get_minimum_free_heap_size(),
        )
    };
    println!("free={} largest={largest} min={min}", free_heap());
    // Stack never used by this thread (`crab-io`) since its start, bytes.
    // SAFETY: a null handle means the calling task.
    let left = unsafe { sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
    // The thread `crab-i2c` (queued I²C writes) is there once a module sent.
    match super::i2c_send::stack_left() {
        Some(i2c) => println!("stack=crab-io:{left},crab-i2c:{i2c}"),
        None => println!("stack=crab-io:{left}"),
    }
    if let Some(c) = cycle {
        println!(
            "cycle period={} steps={} gap={} over={} late50={} late100={} between={}",
            c.period_us(),
            c.steps,
            c.gap_max_us,
            c.overruns,
            c.late50,
            c.late100,
            c.between_max_us
        );
    }
    println!("end");
}

/// `info`.
pub fn answer_info() {
    let (storage, areas) = {
        let s = snap();
        let areas: Vec<String> = s
            .areas
            .iter()
            .map(|a| {
                let allow = if a.allow.is_empty() { "-" } else { &a.allow };
                format!(
                    "{} at={} len={} gen={} mode={} mem={} allow={allow}",
                    a.name, a.at, a.len, a.gen, a.mode, a.memory
                )
            })
            .collect();
        (s.storage, areas)
    };
    println!("master=crab-master-rs {}", env!("CARGO_PKG_VERSION"));
    // Why the board last started (a crash shows as `panic` or a watchdog), and the bad starts in a
    // row: for bug reports (`specs/bug-report`).
    // SAFETY: a plain query of ESP-IDF.
    let reason = match unsafe { sys::esp_reset_reason() } {
        sys::esp_reset_reason_t_ESP_RST_POWERON => "poweron",
        sys::esp_reset_reason_t_ESP_RST_EXT => "pin",
        sys::esp_reset_reason_t_ESP_RST_SW => "software",
        sys::esp_reset_reason_t_ESP_RST_PANIC => "panic",
        sys::esp_reset_reason_t_ESP_RST_INT_WDT => "int_wdt",
        sys::esp_reset_reason_t_ESP_RST_TASK_WDT => "task_wdt",
        sys::esp_reset_reason_t_ESP_RST_WDT => "wdt",
        sys::esp_reset_reason_t_ESP_RST_BROWNOUT => "brownout",
        sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => "deepsleep",
        _ => "other",
    };
    let bad = snap().bad_boots;
    println!("reset={reason} bad={bad}");
    println!("crabmod={}/{} largest={}", storage.0, storage.1, storage.2);
    for line in areas {
        println!("{line}");
    }
    let (total, hour) = retain::write_counts();
    match retain::nvs_entries() {
        Some((used, free)) => println!("nvs writes={total} hour={hour} used={used} free={free}"),
        None => println!("nvs writes={total} hour={hour}"),
    }
    println!("end");
}

/// `out <name> <count>`.
pub fn answer_outputs(name: &str, count: usize) {
    let words = {
        let s = snap();
        s.modules
            .iter()
            .find(|m| m.name == name && m.live)
            .map(|m| m.outputs.clone())
    };
    match words {
        Some(words) => {
            println!("{}", format_outputs(&words, count));
            println!("ok");
        }
        None => println!("err no such module"),
    }
}

/// `ev <name>`: takes the events published so far.
pub fn answer_events(name: &str) {
    let taken = {
        let mut s = snap();
        s.modules
            .iter_mut()
            .find(|m| m.name == name && m.live)
            .map(|m| {
                let events: Vec<(u8, u8, u32)> = m
                    .events
                    .drain(..)
                    .map(|e| (e.slot, e.value, e.ms))
                    .collect();
                (events, core::mem::take(&mut m.lost))
            })
    };
    match taken {
        Some((events, lost)) => {
            println!(
                "{}",
                format_events(name, devices::millis() as u32, lost, &events)
            );
            println!("ok");
        }
        None => println!("err no such module"),
    }
}
