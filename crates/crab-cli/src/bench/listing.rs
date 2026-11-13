//! The master's `list` answer as numbers, and the serial helpers that ask for it.
//!
//! One line per module:
//! `<name> run|stop steps=.. restarts=.. out0=.. us=last/max/gap/sum qlost=.. mem=.. code=..
//! wasm=.. mode=aot|interp crc=..`, then `caps=`, `modcaps=`, `id=`, `free=.. largest=.. min=..`
//! and `end`. In `us=` the fields are the last step, the longest step since the start (or since
//! `rmax`), the longest gap between two steps, and the sum of all step times (wraps at 2^32).

use std::io::{Read, Write};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleLine {
    pub name: String,
    pub running: bool,
    pub steps: u32,
    pub restarts: u32,
    /// `[last, max, gap, sum]`
    pub us: Option<[u32; 4]>,
    pub qlost: u32,
    /// Heap the module takes (`mem=`) and the code part of it (`code=`), bytes.
    pub mem: Option<u32>,
    pub code: Option<u32>,
    /// Size of the stored image.
    pub wasm: Option<u32>,
    /// `aot` or `interp`.
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListAnswer {
    pub modules: Vec<ModuleLine>,
    pub free: Option<u32>,
    pub largest: Option<u32>,
    /// Smallest free heap since power-on.
    pub min_free: Option<u32>,
    /// `id=` of the master (`rs-0.4.1`).
    pub id: Option<String>,
}

impl ListAnswer {
    pub fn module(&self, name: &str) -> Option<&ModuleLine> {
        self.modules.iter().find(|m| m.name == name)
    }
}

/// Parses the lines of one answer (up to, not including, `end`). Lines it does not know are
/// skipped: the master's log may be mixed in.
pub fn parse_list<'a>(lines: impl IntoIterator<Item = &'a str>) -> ListAnswer {
    let mut answer = ListAnswer::default();
    for line in lines {
        let line = line.trim();
        if let Some(id) = line.strip_prefix("id=") {
            answer.id = Some(id.trim().to_owned());
        } else if line.starts_with("free=") {
            for word in line.split_whitespace() {
                let num = |v: &str| v.parse::<u32>().ok();
                if let Some(v) = word.strip_prefix("free=") {
                    answer.free = num(v);
                } else if let Some(v) = word.strip_prefix("largest=") {
                    answer.largest = num(v);
                } else if let Some(v) = word.strip_prefix("min=") {
                    answer.min_free = num(v);
                }
            }
        } else if let Some(module) = parse_module(line) {
            answer.modules.push(module);
        }
    }
    answer
}

fn parse_module(line: &str) -> Option<ModuleLine> {
    let mut words = line.split_whitespace();
    let name = words.next()?;
    let state = words.next()?;
    if !matches!(state, "run" | "stop") || !line.contains(" steps=") {
        return None;
    }
    let mut m = ModuleLine {
        name: name.to_owned(),
        running: state == "run",
        ..ModuleLine::default()
    };
    let num = |v: &str| v.parse::<u32>().ok();
    for word in words {
        if let Some(v) = word.strip_prefix("steps=") {
            m.steps = num(v).unwrap_or(0);
        } else if let Some(v) = word.strip_prefix("restarts=") {
            m.restarts = num(v).unwrap_or(0);
        } else if let Some(v) = word.strip_prefix("qlost=") {
            m.qlost = num(v).unwrap_or(0);
        } else if let Some(v) = word.strip_prefix("mem=") {
            m.mem = num(v);
        } else if let Some(v) = word.strip_prefix("code=") {
            m.code = num(v);
        } else if let Some(v) = word.strip_prefix("wasm=") {
            m.wasm = num(v);
        } else if let Some(v) = word.strip_prefix("mode=") {
            m.mode = Some(v.to_owned());
        } else if let Some(v) = word.strip_prefix("us=") {
            let n: Vec<u32> = v.split('/').filter_map(num).collect();
            m.us = <[u32; 4]>::try_from(n).ok();
        }
    }
    Some(m)
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Reads and drops what is waiting on the port (a moment at most: a module may log all the time).
pub fn drain<P: Read>(port: &mut P) {
    let started = Instant::now();
    let mut buf = [0u8; 256];
    while started.elapsed() < Duration::from_millis(300) {
        match port.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

/// Reads one line, `None` on timeout.
fn read_line<P: Read>(port: &mut P, limit: Duration) -> Result<Option<String>, String> {
    let started = Instant::now();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match port.read(&mut byte) {
            Ok(0) => {}
            Ok(_) if byte[0] == b'\n' => {
                return Ok(Some(String::from_utf8_lossy(&line).trim().to_owned()))
            }
            Ok(_) => line.push(byte[0]),
            Err(e) if is_timeout(&e) => {}
            Err(e) => return Err(format!("ошибка связи с платой: {e}")),
        }
        if started.elapsed() > limit {
            return Ok(None);
        }
    }
}

/// Sends `list` and reads the answer.
pub fn query_list<P: Read + Write>(port: &mut P) -> Result<ListAnswer, String> {
    drain(port);
    port.write_all(b"list\n")
        .and_then(|()| port.flush())
        .map_err(|e| format!("ошибка связи с платой: {e}"))?;
    let started = Instant::now();
    let mut lines = Vec::new();
    while started.elapsed() < Duration::from_secs(4) {
        match read_line(port, Duration::from_secs(4))? {
            Some(line) if line == "end" => return Ok(parse_list(lines.iter().map(String::as_str))),
            Some(line) => lines.push(line),
            None => break,
        }
    }
    Err("плата не ответила на list".into())
}

/// Sends `log` and returns the journal lines (`<ms> [source] text`) up to `end`.
pub fn query_log<P: Read + Write>(port: &mut P) -> Result<Vec<String>, String> {
    drain(port);
    port.write_all(
        b"log
",
    )
    .and_then(|()| port.flush())
    .map_err(|e| format!("ошибка связи с платой: {e}"))?;
    let started = Instant::now();
    let mut lines = Vec::new();
    while started.elapsed() < Duration::from_secs(4) {
        match read_line(port, Duration::from_secs(4))? {
            Some(line) if line == "end" => return Ok(lines),
            Some(line) => lines.push(line),
            None => break,
        }
    }
    Err("плата не ответила на log".into())
}

/// The address from the latest `wifi ready, address <ip>` line of the journal.
pub fn wifi_address<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<std::net::Ipv4Addr> {
    const MARK: &str = "wifi ready, address ";
    lines
        .into_iter()
        .filter_map(|line| {
            let at = line.find(MARK)?;
            line[at + MARK.len()..].trim().parse().ok()
        })
        .last()
}

/// Sends one command and waits for its `ok` or `err ...` line.
pub fn command<P: Read + Write>(port: &mut P, text: &str) -> Result<(), String> {
    drain(port);
    port.write_all(format!("{text}\n").as_bytes())
        .and_then(|()| port.flush())
        .map_err(|e| format!("ошибка связи с платой: {e}"))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        match read_line(port, Duration::from_secs(5))? {
            Some(line) if line == "ok" => return Ok(()),
            Some(line) if line.starts_with("err") => return Err(line),
            Some(_) => {}
            None => break,
        }
    }
    Err(format!("плата не ответила на «{text}»"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded from an ESP32 with the Rust master (module sheet1, AOT).
    pub const ANSWER: &str = "\
sheet1 run steps=1842 restarts=0 out0=00000001 us=212/390/100004/390140 qlost=0 mem=18944 code=9120 wasm=2144 mode=aot crc=a1b2c3d4
sheet2 stop steps=0 restarts=2 out0=00000000 us=0/0/0/0 qlost=0 mem=0 code=0 wasm=900 mode=interp crc=00000001
caps=us,ev,bus
modcaps=math,retain,aot,xtensa-esp32
id=rs-0.4.1
free=84212 largest=61440 min=70116";

    #[test]
    fn parses_modules_and_memory() {
        let a = parse_list(ANSWER.lines());
        assert_eq!(a.modules.len(), 2);
        let m = a.module("sheet1").unwrap();
        assert!(m.running);
        assert_eq!(m.steps, 1842);
        assert_eq!(m.us, Some([212, 390, 100_004, 390_140]));
        assert_eq!(m.mem, Some(18944));
        assert_eq!(m.code, Some(9120));
        assert_eq!(m.mode.as_deref(), Some("aot"));
        let stopped = a.module("sheet2").unwrap();
        assert!(!stopped.running);
        assert_eq!(stopped.restarts, 2);
        assert_eq!(a.free, Some(84212));
        assert_eq!(a.largest, Some(61440));
        assert_eq!(a.min_free, Some(70116));
        assert_eq!(a.id.as_deref(), Some("rs-0.4.1"));
    }

    #[test]
    fn older_master_without_us_gives_none() {
        let a = parse_list([
            "sheet1 run steps=5 restarts=0 out0=0 crc=00000001",
            "free=100",
        ]);
        assert_eq!(a.modules[0].us, None);
        assert_eq!(a.free, Some(100));
        assert_eq!(a.largest, None);
    }

    #[test]
    fn wifi_address_is_the_latest_ready_note() {
        let log = [
            "1200 [net] wifi ready, address 192.168.1.50",
            "5000 [net] wifi link lost",
            "9000 [net] wifi ready, address 192.168.1.77",
            "9100 [mqtt] connected",
        ];
        assert_eq!(
            wifi_address(log),
            Some(std::net::Ipv4Addr::new(192, 168, 1, 77))
        );
        assert_eq!(wifi_address(["1 [net] wifi link lost"]), None);
        assert_eq!(wifi_address(["1 [net] wifi ready, address ???"]), None);
    }

    #[test]
    fn log_lines_are_skipped() {
        let a = parse_list(["I (123) wifi: connected", "ok", "free=5 largest=4 min=3"]);
        assert!(a.modules.is_empty());
        assert_eq!(a.min_free, Some(3));
    }
}
