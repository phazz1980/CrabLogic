//! Debugging through the master firmware (`specs/wasm-only`): the host polls the values
//! of every watched signal of each module with `out <module> <n>` and shows them like
//! the binary session does. The board keeps running its modules; nothing is flashed.

use std::collections::BTreeMap;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crab_core::SignalType;

use crate::protocol::{MasterMemory, ModuleMemory, NvsStats, SheetStats, Stats};
use crate::session::{Command, Live};
use crate::table::{DebugTable, Value};
use crate::transport::{Connector, Transport};

/// Points of one chart; a fast blink makes two points per change.
const HISTORY: usize = 6000;
const CONSOLE: usize = 500;
/// How often the board is asked which modules it runs.
const LIST_EVERY: Duration = Duration::from_secs(1);
/// How often the board is asked for `info` (flash wear counters).
const INFO_EVERY: Duration = Duration::from_secs(10);
/// Pause between two polls of the board.
const POLL: Duration = Duration::from_millis(150);
/// The board is considered silent after this long without an answer.
const SILENT: Duration = Duration::from_secs(3);

/// A module on the board and where its watched signals go in the debug table.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleMap {
    pub name: String,
    /// Table entry of each watched slot of the module, in the order of `output_i/f(slot, v)`;
    /// `None` for a slot the table does not show (strings).
    pub slots: Vec<Option<usize>>,
    /// CRC-32 of the module bytes; the board reports the same for what it runs.
    pub crc: u32,
    /// CRC-32 of the image actually uploaded when it differs from the WASM (AOT);
    /// the board reports it instead of `crc`.
    pub image_crc: Option<u32>,
    /// Number of the sheet the module runs, for the diagnostics table.
    pub sheet: u32,
}

/// One module of the `list` answer.
#[derive(Debug, Clone, Default)]
struct ModStat {
    name: String,
    steps: u32,
    /// `us=<last>/<max>/<longest gap between steps>/<sum>`; `None` for an older master.
    us: Option<[u32; 4]>,
    /// `qlost=<n>`: event messages dropped from the module queue; 0 for an older master.
    qlost: u32,
    /// `mem=`, `code=`, `wasm=`: heap the module takes; `None` for an older master.
    memory: Option<ModuleMemory>,
}

/// What the polling thread remembers between lines of the board.
#[derive(Default)]
struct Aux {
    /// The master sends changes of 0/1 outputs with their time (`caps=ev`).
    events: bool,
    /// Modules of the running `list` answer.
    mods: Vec<ModStat>,
    free_heap: u32,
    /// `largest=` and `min=` of the `free=` line, `stack=` line; zero / empty for an older master.
    largest: u32,
    min_free: u32,
    stacks: Vec<(String, u32)>,
    /// Per module: steps and summed step time at the previous report.
    prev: BTreeMap<String, (u32, u32)>,
    prev_at: Option<Instant>,
    /// Inside the `info` answer (between its `master=` line and `end`).
    in_info: bool,
}

/// Debugging session with the master firmware: same shape as [`crate::Session`].
pub struct MasterSession {
    live: Arc<Mutex<Live>>,
    tx: Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MasterSession {
    pub fn start(
        table: DebugTable,
        modules: Vec<ModuleMap>,
        connect: Connector,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let n = table.entries.len();
        let live = Arc::new(Mutex::new(Live {
            values: vec![None; n],
            texts: vec![None; table.texts.len()],
            forced: vec![None; n],
            history: vec![Vec::new(); n],
            status: "Подключение...".into(),
            ..Default::default()
        }));
        let (tx, rx) = mpsc::channel();
        let shared = live.clone();
        let thread = thread::spawn(move || run(table, modules, connect, rx, shared, wake));
        Self {
            live,
            tx,
            thread: Some(thread),
        }
    }

    pub fn snapshot(&self) -> Live {
        self.live.lock().expect("live state").clone()
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MasterSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    table: DebugTable,
    modules: Vec<ModuleMap>,
    connect: Connector,
    rx: Receiver<Command>,
    live: Arc<Mutex<Live>>,
    wake: impl Fn(),
) {
    let started = Instant::now();
    live.lock().expect("live state").clock = Some(started);
    loop {
        let mut port = match connect() {
            Ok(port) => port,
            Err(e) => {
                set_status(&live, false, format!("Нет связи с платой: {e}"));
                wake();
                if wait_or_stop(&rx, Duration::from_secs(1)) {
                    return;
                }
                continue;
            }
        };
        set_status(&live, true, "Подключено, ждём мастер-прошивку...".into());
        wake();
        match serve(&table, &modules, started, port.as_mut(), &rx, &live, &wake) {
            Ok(true) => return,
            Ok(false) => set_status(
                &live,
                false,
                "Мастер не отвечает: на плате другая прошивка? Переподключение...".into(),
            ),
            Err(e) => set_status(&live, false, format!("Связь потеряна: {e}")),
        }
        wake();
        if wait_or_stop(&rx, Duration::from_millis(500)) {
            return;
        }
    }
}

/// Polls the board until asked to stop (`true`) or until it stops answering (`false`).
fn serve(
    table: &DebugTable,
    modules: &[ModuleMap],
    started: Instant,
    port: &mut dyn Transport,
    rx: &Receiver<Command>,
    live: &Arc<Mutex<Live>>,
    wake: &impl Fn(),
) -> io::Result<bool> {
    let mut buf = [0u8; 1024];
    let mut line = Vec::new();
    // Modules asked and not yet answered, oldest first.
    let mut asked: Vec<usize> = Vec::new();
    let mut last_rx = Instant::now();
    let mut last_poll = Instant::now() - POLL;
    let mut last_list = Instant::now() - LIST_EVERY;
    let mut last_info = Instant::now() - INFO_EVERY;
    let mut running: Vec<(String, u32)> = Vec::new();
    let mut aux = Aux::default();
    loop {
        loop {
            match rx.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(true),
                Ok(command) => send_command(command, table, modules, port, live)?,
                Err(TryRecvError::Empty) => break,
            }
        }
        if last_list.elapsed() >= LIST_EVERY {
            port.write_all(
                b"list
",
            )?;
            last_list = Instant::now();
        } else if last_info.elapsed() >= INFO_EVERY {
            port.write_all(
                b"info
",
            )?;
            last_info = Instant::now();
        }
        if asked.is_empty() && last_poll.elapsed() >= POLL {
            for (i, m) in modules.iter().enumerate() {
                // Events first: the answer to `out` then never comes before older changes.
                if aux.events {
                    port.write_all(format!("ev {}\n", m.name).as_bytes())?;
                }
                port.write_all(format!("out {} {}\n", m.name, m.slots.len()).as_bytes())?;
                asked.push(i);
            }
            last_poll = Instant::now();
        }
        let n = port.read(&mut buf)?;
        if n == 0 {
            if last_rx.elapsed() > SILENT {
                return Ok(false);
            }
            // An unanswered request is asked again after a while.
            if !asked.is_empty() && last_poll.elapsed() > Duration::from_secs(1) {
                asked.clear();
            }
            continue;
        }
        last_rx = Instant::now();
        let mut changed = false;
        for &b in &buf[..n] {
            if b != b'\n' {
                line.push(b);
                continue;
            }
            let text = String::from_utf8_lossy(&line).trim().to_owned();
            line.clear();
            changed |= handle_line(
                &text,
                table,
                modules,
                &mut asked,
                &mut running,
                &mut aux,
                started,
                live,
            );
        }
        if changed {
            wake();
        }
    }
}

/// Handles one line of the board; `true` if the shown data changed.
#[allow(clippy::too_many_arguments)]
fn handle_line(
    text: &str,
    table: &DebugTable,
    modules: &[ModuleMap],
    asked: &mut Vec<usize>,
    running: &mut Vec<(String, u32)>,
    aux: &mut Aux,
    started: Instant,
    live: &Arc<Mutex<Live>>,
) -> bool {
    let mut l = live.lock().expect("live state");
    // `info` answer (asked now and then for the flash wear): only its `nvs` line is used, the
    // rest, and the closing `end`, must not reach the `list` handling or the console.
    if text.starts_with("master=") {
        aux.in_info = true;
        return false;
    }
    if aux.in_info {
        if text == "end" {
            aux.in_info = false;
        } else if let Some(nvs) = NvsStats::parse(text) {
            let changed = l.nvs.as_ref() != Some(&nvs);
            l.nvs = Some(nvs);
            return changed;
        }
        return false;
    }
    // `list` answer: `<name> run|stop steps=.. restarts=.. out0=.. crc=<hex>`, then `end`.
    if let Some((head, crc)) = text.rsplit_once(" crc=") {
        let name = head.split_whitespace().next().unwrap_or_default();
        let crc = u32::from_str_radix(crc.trim(), 16).unwrap_or(0);
        running.push((name.to_owned(), crc));
        aux.mods.push(parse_module_stat(name, head));
        return false;
    }
    if let Some(caps) = text.strip_prefix("caps=") {
        aux.events = caps.split(',').any(|c| c.trim() == "ev");
        l.events = aux.events;
        return false;
    }
    // `list` answer: `free=<bytes> largest=<bytes> min=<bytes>`.
    if let Some(rest) = text.strip_prefix("free=") {
        let mut words = rest.split_whitespace();
        aux.free_heap = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
        for word in words {
            if let Some(v) = word.strip_prefix("largest=") {
                aux.largest = v.parse().unwrap_or(0);
            } else if let Some(v) = word.strip_prefix("min=") {
                aux.min_free = v.parse().unwrap_or(0);
            }
        }
        return false;
    }
    // `list` answer: `stack=<task>:<bytes never used>,...`.
    if let Some(rest) = text.strip_prefix("stack=") {
        aux.stacks = rest
            .split(',')
            .filter_map(|item| {
                let (task, left) = item.trim().split_once(':')?;
                Some((task.to_owned(), left.parse().ok()?))
            })
            .collect();
        return false;
    }
    if text == "end" {
        l.stats = make_stats(aux, modules);
        let missing = modules.iter().find(|m| {
            !running
                .iter()
                .any(|(n, c)| *n == m.name && (*c == m.crc || Some(*c) == m.image_crc))
        });
        running.clear();
        l.connected = true;
        l.schema_matches = Some(missing.is_none());
        l.status = missing.map_or(String::new(), |m| {
            format!(
                "Модуль «{}» на плате не совпадает со схемой: залейте модули (📦)",
                m.name
            )
        });
        return true;
    }
    // Changes of 0/1 outputs since the previous `ev`:
    // `ev <name> <board ms now> <lost> <slot>:<value>:<board ms> ...`.
    if let Some(rest) = text.strip_prefix("ev ") {
        let t_rx = started.elapsed().as_secs_f64();
        return apply_events(rest, t_rx, table, modules, &mut l);
    }
    if let Some(words) = text.strip_prefix("out") {
        let Some(&m) = asked.first() else {
            return false;
        };
        asked.remove(0);
        let t = started.elapsed().as_secs_f64();
        for (slot, word) in words.split_whitespace().enumerate() {
            let Some(Some(i)) = modules[m].slots.get(slot) else {
                continue;
            };
            let Ok(raw) = u32::from_str_radix(word, 16) else {
                continue;
            };
            let v = Value::from_raw(table.entries[*i].ty, raw);
            l.values[*i] = Some(v);
            let y = match v {
                Value::Bool(b) => f64::from(u8::from(b)),
                Value::Int(x) => f64::from(x),
                Value::Float(x) => f64::from(x),
            };
            let h = &mut l.history[*i];
            // With events the chart of a 0/1 signal is drawn from them; the sample only
            // corrects a state the events did not tell (a lost event, a forced value).
            if aux.events && matches!(v, Value::Bool(_)) && h.last().is_some_and(|p| p.1 == y) {
                continue;
            }
            push_point(h, t, y);
        }
        l.frames += 1;
        return true;
    }
    // `list` answer: the address of the board, absent while it has none.
    if let Some(ip) = text.strip_prefix("ip=") {
        let ip = ip.trim().to_owned();
        let changed = l.address.as_deref() != Some(ip.as_str());
        l.address = Some(ip);
        return changed;
    }
    // `list` answer: what this master is (`id=<hex>`) and which native parts of blocks it has
    // (`native=<type>:<instances>,...`).
    if let Some(id) = text.strip_prefix("id=") {
        let id = id.trim().to_owned();
        let changed = l.master_id.as_deref() != Some(id.as_str());
        l.master_id = Some(id);
        return changed;
    }
    // `list` answer of a master in safe mode: how many bad resets in a row stopped its modules.
    if let Some(n) = text.strip_prefix("safe=") {
        let n = n.trim().parse().ok();
        let changed = l.master_safe != n;
        l.master_safe = n;
        return changed;
    }
    if let Some(list) = text.strip_prefix("native=") {
        l.master_natives = list
            .split(',')
            .filter_map(|item| {
                let (kind, count) = item.trim().split_once(':')?;
                Some((kind.to_owned(), count.parse().ok()?))
            })
            .collect();
        return true;
    }
    if text == "ok" || text.is_empty() {
        return false;
    }
    if text.starts_with("err no such module") {
        // The answer to the request for the oldest unanswered module.
        let name = asked
            .first()
            .map_or(String::new(), |&m| modules[m].name.clone());
        if !asked.is_empty() {
            asked.remove(0);
        }
        l.status = format!("На плате нет модуля «{name}»: залейте модули (📦)");
        return true;
    }
    l.console.push(text.to_owned());
    let excess = l.console.len().saturating_sub(CONSOLE);
    l.console.drain(..excess);
    l.console_total += 1;
    true
}

/// A point of a chart; the oldest ones go when there are too many.
fn push_point(h: &mut Vec<(f64, f64)>, t: f64, y: f64) {
    h.push((t, y));
    let excess = h.len().saturating_sub(HISTORY);
    h.drain(..excess);
}

/// `<name> run|stop steps=.. restarts=.. out0=.. us=a/b/c/d` (the part before ` crc=`).
fn parse_module_stat(name: &str, head: &str) -> ModStat {
    let mut m = ModStat {
        name: name.to_owned(),
        ..ModStat::default()
    };
    let num = |v: &str| v.parse::<u32>().ok();
    let (mut mem, mut code, mut wasm, mut accelerated) = (None, None, None, None);
    for word in head.split_whitespace().skip(1) {
        if let Some(v) = word.strip_prefix("mem=") {
            mem = num(v);
        } else if let Some(v) = word.strip_prefix("code=") {
            code = num(v);
        } else if let Some(v) = word.strip_prefix("wasm=") {
            wasm = num(v);
        } else if let Some(v) = word.strip_prefix("mode=") {
            accelerated = match v {
                "aot" => Some(true),
                "interp" => Some(false),
                _ => None,
            };
        } else if let Some(v) = word.strip_prefix("steps=") {
            m.steps = v.parse().unwrap_or(0);
        } else if let Some(v) = word.strip_prefix("qlost=") {
            m.qlost = v.parse().unwrap_or(0);
        } else if let Some(v) = word.strip_prefix("us=") {
            let n: Vec<u32> = v.split('/').filter_map(|x| x.parse().ok()).collect();
            if let Ok(us) = <[u32; 4]>::try_from(n) {
                m.us = Some(us);
            }
        }
    }
    if let (Some(mem), Some(code), Some(wasm)) = (mem, code, wasm) {
        m.memory = Some(ModuleMemory {
            name: name.to_owned(),
            mem,
            code,
            wasm,
            accelerated,
        });
    }
    m
}

/// Cycle figures from the `list` answer that has just ended; cycles are counted per 0.5 s
/// as in the USB debugging, and the master's modules run side by side, so the longest
/// figure of a module is the figure of the board.
fn make_stats(aux: &mut Aux, modules: &[ModuleMap]) -> Option<Stats> {
    let mods = std::mem::take(&mut aux.mods);
    if mods.is_empty() {
        return None;
    }
    let now = Instant::now();
    let dt = aux
        .prev_at
        .map_or(1.0, |t| now.duration_since(t).as_secs_f64())
        .max(0.1);
    aux.prev_at = Some(now);
    let (mut d_max, mut d_all, mut d_sum) = (0u32, 0u64, 0u64);
    let modules_memory: Vec<ModuleMemory> = mods.iter().filter_map(|m| m.memory.clone()).collect();
    let memory =
        (aux.min_free > 0 || !aux.stacks.is_empty() || !modules_memory.is_empty()).then(|| {
            MasterMemory {
                largest: aux.largest,
                min_free: aux.min_free,
                stacks: std::mem::take(&mut aux.stacks),
                modules: modules_memory,
            }
        });
    let mut stats = Stats {
        free_heap: aux.free_heap,
        memory,
        ..Stats::default()
    };
    for m in &mods {
        stats.bus_lost = stats.bus_lost.saturating_add(m.qlost);
        let sum = m.us.map_or(0, |u| u[3]);
        if let Some(&(steps, total)) = aux.prev.get(&m.name) {
            // Fewer steps than before: the module was restarted.
            let (ds, dsum) = if m.steps < steps {
                (m.steps, sum)
            } else {
                (m.steps - steps, sum.wrapping_sub(total))
            };
            d_max = d_max.max(ds);
            d_all += u64::from(ds);
            d_sum += u64::from(dsum);
        }
        aux.prev.insert(m.name.clone(), (m.steps, sum));
        if let Some([last, max, period, _]) = m.us {
            stats.last_us = stats.last_us.max(last);
            stats.max_us = stats.max_us.max(max);
            stats.period_max_us = stats.period_max_us.max(period);
            if let Some(map) = modules.iter().find(|k| k.name == m.name) {
                stats.sheets.push(SheetStats {
                    sheet: map.sheet,
                    last_us: last,
                    max_us: max,
                });
            }
        }
    }
    stats.cycles = (f64::from(d_max) * 0.5 / dt).round() as u32;
    stats.avg_us = d_sum.checked_div(d_all).map_or(stats.last_us, |v| v as u32);
    stats.sheets.sort_by_key(|s| s.sheet);
    Some(stats)
}

/// Applies the answer to `ev` (without the word `ev`): every change of a 0/1 output gets its
/// own place on the chart, at the time the board says it happened. Returns `true` if
/// anything was shown.
fn apply_events(
    rest: &str,
    t_rx: f64,
    table: &DebugTable,
    modules: &[ModuleMap],
    l: &mut Live,
) -> bool {
    let mut words = rest.split_whitespace();
    let Some(map) = words
        .next()
        .and_then(|name| modules.iter().find(|m| m.name == name))
    else {
        return false;
    };
    let now: u32 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
    let _lost = words.next();
    let mut changed = false;
    for word in words {
        let mut f = word.split(':');
        let (Some(slot), Some(val), Some(ms)) = (
            f.next().and_then(|w| w.parse::<usize>().ok()),
            f.next().and_then(|w| w.parse::<u32>().ok()),
            f.next().and_then(|w| w.parse::<u32>().ok()),
        ) else {
            continue;
        };
        let Some(Some(i)) = map.slots.get(slot) else {
            continue;
        };
        if table.entries[*i].ty != SignalType::Bool {
            continue;
        }
        let age = f64::from(now.wrapping_sub(ms)) / 1000.0;
        let h = &mut l.history[*i];
        let last = h.last().copied();
        // Never before the previous point, whatever the delay of the line was.
        let t = (t_rx - age).max(last.map_or(0.0, |p| p.0));
        let y = f64::from(u8::from(val != 0));
        if let Some((_, prev)) = last.filter(|p| p.1 != y) {
            push_point(h, t, prev);
        }
        push_point(h, t, y);
        l.values[*i] = Some(Value::Bool(val != 0));
        changed = true;
    }
    // The line goes on to now, so the chart does not stand still while nothing changes.
    for i in map.slots.iter().flatten() {
        if table.entries[*i].ty != SignalType::Bool {
            continue;
        }
        let h = &mut l.history[*i];
        if let Some((_, y)) = h.last().copied().filter(|p| p.0 < t_rx) {
            // A flat stretch is one segment: only its end moves.
            if h.len() >= 2 && h[h.len() - 2].1 == y {
                h.last_mut().expect("not empty").0 = t_rx;
            } else {
                push_point(h, t_rx, y);
            }
            changed = true;
        }
    }
    changed
}

/// Forcing and releasing: the master keeps the forced value of a module output.
fn send_command(
    command: Command,
    table: &DebugTable,
    modules: &[ModuleMap],
    port: &mut dyn Transport,
    live: &Arc<Mutex<Live>>,
) -> io::Result<()> {
    // Module and slot that show table entry `index`.
    let place = |index: usize| {
        modules.iter().find_map(|m| {
            let slot = m.slots.iter().position(|s| *s == Some(index))?;
            Some((m.name.as_str(), slot))
        })
    };
    match command {
        Command::Force { index, value } => {
            let (Some(entry), Some((name, slot))) = (table.entries.get(index), place(index)) else {
                return Ok(());
            };
            let value = value.map(|v| v.cast(entry.ty));
            live.lock().expect("live state").forced[index] = value;
            let text = match value {
                None => format!(
                    "unforce {name} {slot}
"
                ),
                Some(Value::Float(x)) => format!(
                    "force {name} {slot} f {x}
"
                ),
                Some(Value::Int(x)) => format!(
                    "force {name} {slot} i {x}
"
                ),
                Some(Value::Bool(b)) => format!(
                    "force {name} {slot} i {}
",
                    u8::from(b)
                ),
            };
            port.write_all(text.as_bytes())
        }
        Command::ReleaseAll => {
            live.lock().expect("live state").forced.fill(None);
            for m in modules {
                port.write_all(
                    format!(
                        "unforce {} all
",
                        m.name
                    )
                    .as_bytes(),
                )?;
            }
            Ok(())
        }
        Command::ResetStats => port.write_all(
            b"rmax
",
        ),
        // The master has no setpoints or console input yet.
        _ => Ok(()),
    }
}

fn set_status(live: &Arc<Mutex<Live>>, connected: bool, status: String) {
    let mut l = live.lock().expect("live state");
    l.connected = connected;
    l.status = status;
    if !connected {
        l.schema_matches = None;
        l.stats = None;
        l.nvs = None;
    }
}

/// Sleeps up to `d`; returns true if the session was asked to stop meanwhile.
fn wait_or_stop(rx: &Receiver<Command>, d: Duration) -> bool {
    match rx.recv_timeout(d) {
        Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::{PortRef, SignalType};

    use crate::table::{Entry, Source};

    fn entry(name: &str, ty: SignalType) -> Entry {
        Entry {
            source: Source::Output(PortRef::new(name, "out")),
            ty,
            storage: name.into(),
            cpp_type: "",
        }
    }

    fn table() -> DebugTable {
        DebugTable {
            entries: vec![entry("a", SignalType::Int), entry("b", SignalType::Float)],
            texts: Vec::new(),
        }
    }

    #[test]
    fn answer_goes_to_table_entries() {
        let table = table();
        let modules = vec![ModuleMap {
            name: "sheet1".into(),
            slots: vec![Some(1), None, Some(0)],
            crc: 5,
            image_crc: None,
            sheet: 1,
        }];
        let live = Arc::new(Mutex::new(Live {
            values: vec![None; 2],
            forced: vec![None; 2],
            history: vec![Vec::new(); 2],
            ..Default::default()
        }));
        let mut asked = vec![0];
        let two = format!("out {:08x} 00000000 {:08x}", 1.5f32.to_bits(), 7);
        assert!(handle_line(
            &two,
            &table,
            &modules,
            &mut asked,
            &mut Vec::new(),
            &mut Aux::default(),
            Instant::now(),
            &live
        ));
        let l = live.lock().unwrap();
        assert_eq!(l.values[1], Some(Value::Float(1.5)));
        assert_eq!(l.values[0], Some(Value::Int(7)));
        assert!(asked.is_empty());
    }

    #[test]
    fn master_id_and_native_parts_are_kept_not_printed() {
        let table = table();
        let live = Arc::new(Mutex::new(Live::default()));
        let line = |text: &str| {
            handle_line(
                text,
                &table,
                &[],
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Aux::default(),
                Instant::now(),
                &live,
            )
        };
        assert!(line("id=00ab12cd34ef5678"));
        assert!(!line("id=00ab12cd34ef5678"), "the same id is not a change");
        assert!(line("native=bmp:2,twice:1"));
        assert!(line("safe=3"));
        let l = live.lock().unwrap();
        assert_eq!(l.master_safe, Some(3));
        assert_eq!(l.master_id.as_deref(), Some("00ab12cd34ef5678"));
        assert_eq!(
            l.master_natives,
            [("bmp".to_owned(), 2), ("twice".to_owned(), 1)]
        );
        assert!(l.console.is_empty(), "{:?}", l.console);
    }

    #[test]
    fn info_answer_gives_nvs_and_stays_out_of_the_console() {
        let table = table();
        let live = Arc::new(Mutex::new(Live::default()));
        let mut aux = Aux::default();
        for text in [
            "master=crab-master-rs 0.1.0",
            "crabmod=1/2 largest=1",
            "a at=0 len=10 gen=1",
            "nvs writes=12 hour=3 used=50 free=450",
            "end",
        ] {
            handle_line(
                text,
                &table,
                &[],
                &mut Vec::new(),
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        let l = live.lock().unwrap();
        assert_eq!(l.nvs.as_ref().map(|n| (n.writes, n.hour)), Some((12, 3)));
        assert!(l.console.is_empty(), "{:?}", l.console);
        assert!(!aux.in_info);
    }

    #[test]
    fn missing_module_is_reported() {
        let table = table();
        let modules = vec![ModuleMap {
            name: "sheet1".into(),
            slots: vec![Some(0)],
            crc: 5,
            image_crc: None,
            sheet: 1,
        }];
        let live = Arc::new(Mutex::new(Live::default()));
        let mut asked = vec![0];
        handle_line(
            "err no such module",
            &table,
            &modules,
            &mut asked,
            &mut Vec::new(),
            &mut Aux::default(),
            Instant::now(),
            &live,
        );
        let l = live.lock().unwrap();
        assert!(l.status.contains("sheet1"));
    }

    #[test]
    fn list_compares_module_crc() {
        let table = table();
        let modules = vec![ModuleMap {
            name: "sheet1".into(),
            slots: vec![Some(0)],
            crc: 0xabc,
            image_crc: None,
            sheet: 1,
        }];
        let live = Arc::new(Mutex::new(Live::default()));
        let run = |crc: &str| {
            let mut running = Vec::new();
            let mut asked = Vec::new();
            for line in [
                format!("sheet1 run steps=1 restarts=0 out0=0 crc={crc}"),
                "end".into(),
            ] {
                handle_line(
                    &line,
                    &table,
                    &modules,
                    &mut asked,
                    &mut running,
                    &mut Aux::default(),
                    Instant::now(),
                    &live,
                );
            }
            live.lock().unwrap().schema_matches
        };
        assert_eq!(run("00000abc"), Some(true));
        assert_eq!(run("00000001"), Some(false));
    }

    #[test]
    fn list_accepts_image_crc() {
        let table = table();
        let modules = vec![ModuleMap {
            name: "sheet1".into(),
            slots: vec![Some(0)],
            crc: 0xabc,
            image_crc: Some(0xdef),
            sheet: 1,
        }];
        let live = Arc::new(Mutex::new(Live::default()));
        let run = |crc: &str| {
            let mut running = Vec::new();
            let mut asked = Vec::new();
            for line in [
                format!("sheet1 run steps=1 restarts=0 out0=0 crc={crc}"),
                "end".into(),
            ] {
                handle_line(
                    &line,
                    &table,
                    &modules,
                    &mut asked,
                    &mut running,
                    &mut Aux::default(),
                    Instant::now(),
                    &live,
                );
            }
            live.lock().unwrap().schema_matches
        };
        assert_eq!(run("00000def"), Some(true));
        assert_eq!(run("00000abc"), Some(true));
        assert_eq!(run("00000001"), Some(false));
    }

    fn bool_table() -> DebugTable {
        DebugTable {
            entries: vec![entry("led", SignalType::Bool), entry("n", SignalType::Int)],
            texts: Vec::new(),
        }
    }

    fn bool_module() -> Vec<ModuleMap> {
        vec![ModuleMap {
            name: "sheet1".into(),
            slots: vec![Some(0), Some(1)],
            crc: 5,
            image_crc: None,
            sheet: 1,
        }]
    }

    fn bool_live() -> Arc<Mutex<Live>> {
        Arc::new(Mutex::new(Live {
            values: vec![None; 2],
            forced: vec![None; 2],
            history: vec![Vec::new(); 2],
            ..Default::default()
        }))
    }

    #[test]
    fn events_draw_a_square_wave_at_board_time() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        let mut line = |text: &str| {
            handle_line(
                text,
                &table,
                &modules,
                &mut vec![0],
                &mut Vec::new(),
                &mut aux,
                Instant::now() - Duration::from_secs(10),
                &live,
            )
        };
        assert!(!line("caps=ev,us"));
        // Board clock 1000 ms now; changes 900 (on), 800 (off earlier), 700 ... in order.
        assert!(line("ev sheet1 1000 0 0:1:700 0:0:800 0:1:900 1:1:950"));
        let l = live.lock().unwrap();
        let h = &l.history[0];
        // Every change is an upright edge: two points at the same time.
        let edges: Vec<(f64, f64)> = h
            .iter()
            .copied()
            .filter(|p| p.0 < h.last().unwrap().0)
            .collect();
        let levels: Vec<f64> = edges.iter().map(|p| p.1).collect();
        assert_eq!(levels, [1.0, 1.0, 0.0, 0.0, 1.0]);
        let times: Vec<f64> = edges.iter().map(|p| p.0).collect();
        assert!(
            (times[2] - times[1]).abs() < 1e-9,
            "edge is vertical: {times:?}"
        );
        assert!(
            ((times[3] - times[0]) - 0.2).abs() < 1e-6,
            "700 -> 900 ms: {times:?}"
        );
        // The chart goes on to the time of the line.
        assert_eq!(h.last().unwrap().1, 1.0);
        assert_eq!(l.values[0], Some(Value::Bool(true)));
        // Int slots are not events.
        assert!(l.history[1].is_empty());
    }

    #[test]
    fn sample_does_not_repeat_what_events_told() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux {
            events: true,
            ..Aux::default()
        };
        let mut asked = Vec::new();
        for text in [
            "out 00000001 00000005",
            "out 00000001 00000005",
            "out 00000000 00000005",
        ] {
            asked.push(0);
            handle_line(
                text,
                &table,
                &modules,
                &mut asked,
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        let l = live.lock().unwrap();
        // First sample starts the line, the repeat is skipped, the mismatch (lost event) is kept.
        assert_eq!(
            l.history[0].iter().map(|p| p.1).collect::<Vec<_>>(),
            [1.0, 0.0]
        );
        // Numbers are always sampled.
        assert_eq!(l.history[1].len(), 3);
    }

    #[test]
    fn list_gives_cycle_statistics() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        let mut feed = |lines: &[&str]| {
            for text in lines {
                handle_line(
                    text,
                    &table,
                    &modules,
                    &mut Vec::new(),
                    &mut Vec::new(),
                    &mut aux,
                    Instant::now(),
                    &live,
                );
            }
        };
        let crc = format!("{:08x}", 5);
        feed(&[
            &format!("sheet1 run steps=100 restarts=0 out0=0 us=300/900/12000/30000 crc={crc}"),
            "free=123456 largest=100000",
            "end",
        ]);
        let first = live.lock().unwrap().stats.clone().unwrap();
        assert_eq!(first.last_us, 300);
        assert_eq!(first.max_us, 900);
        assert_eq!(first.period_max_us, 12000);
        assert_eq!(first.free_heap, 123_456);
        assert_eq!(first.sheets.len(), 1);
        assert_eq!(first.sheets[0].sheet, 1);
        feed(&[
            &format!("sheet1 run steps=150 restarts=0 out0=0 us=200/900/12000/40000 crc={crc}"),
            "end",
        ]);
        assert_eq!(first.memory, None);
        let second = live.lock().unwrap().stats.clone().unwrap();
        // 50 steps took 10000 us: 200 us each.
        assert_eq!(second.avg_us, 200);
        assert!(second.cycles > 0);
    }

    #[test]
    fn list_gives_heap_stacks_and_module_memory() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        let crc = format!("{:08x}", 5);
        for text in [
            format!("sheet1 run steps=1 restarts=0 out0=0 qlost=0 mem=4096 code=9216 wasm=5120 mode=aot crc={crc}"),
            "free=50000 largest=31000 min=42000".to_owned(),
            "stack=loop:5000,hardware:1200,sheet1:2300".to_owned(),
            "end".to_owned(),
        ] {
            handle_line(
                &text,
                &table,
                &modules,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        let stats = live.lock().unwrap().stats.clone().unwrap();
        assert_eq!(stats.free_heap, 50_000);
        let m = stats.memory.unwrap();
        assert_eq!((m.largest, m.min_free), (31_000, 42_000));
        assert_eq!(m.stacks[1], ("hardware".to_owned(), 1200));
        assert_eq!(m.stacks.len(), 3);
        assert_eq!(
            m.modules,
            vec![ModuleMemory {
                name: "sheet1".into(),
                mem: 4096,
                code: 9216,
                wasm: 5120,
                accelerated: Some(true),
            }]
        );
    }

    #[test]
    fn module_mode_comes_from_the_list_line() {
        let mode = |head: &str| {
            parse_module_stat("s", head)
                .memory
                .and_then(|m| m.accelerated)
        };
        assert_eq!(mode("s run mem=1 code=2 wasm=3 mode=aot"), Some(true));
        assert_eq!(mode("s run mem=1 code=2 wasm=3 mode=interp"), Some(false));
        // The C++ master does not tell the mode.
        assert_eq!(mode("s run mem=1 code=2 wasm=3"), None);
    }

    #[test]
    fn older_master_without_times_still_reports_cycles() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        for text in ["sheet1 run steps=10 restarts=0 out0=0 crc=00000005", "end"] {
            handle_line(
                text,
                &table,
                &modules,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        let stats = live.lock().unwrap().stats.clone().unwrap();
        assert_eq!(stats.max_us, 0);
        assert!(!aux.events, "no events without caps");
    }

    #[test]
    fn lost_bus_messages_of_all_modules_are_summed() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        for text in [
            "sheet1 run steps=10 restarts=0 out0=0 us=1/2/3/4 qlost=3 crc=00000005",
            "bot run steps=10 restarts=0 out0=0 qlost=4 crc=00000006",
            "end",
        ] {
            handle_line(
                text,
                &table,
                &modules,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        assert_eq!(live.lock().unwrap().stats.clone().unwrap().bus_lost, 7);
    }

    #[test]
    fn a_flat_signal_stays_two_points() {
        let (table, modules, live) = (bool_table(), bool_module(), bool_live());
        let mut aux = Aux::default();
        for _ in 0..50 {
            handle_line(
                "ev sheet1 1000 0 0:1:900",
                &table,
                &modules,
                &mut vec![0],
                &mut Vec::new(),
                &mut aux,
                Instant::now() - Duration::from_secs(5),
                &live,
            );
            handle_line(
                "ev sheet1 1100 0",
                &table,
                &modules,
                &mut vec![0],
                &mut Vec::new(),
                &mut aux,
                Instant::now(),
                &live,
            );
        }
        // Repeated polls without changes only move the end of the last segment.
        assert!(live.lock().unwrap().history[0].len() < 210);
    }
}
