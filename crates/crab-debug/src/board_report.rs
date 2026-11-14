//! Collecting the board's state for a bug report over the master's text protocol:
//! answers of `list`, `info`, `log` and the crash dump read piece by piece (`core ...`).
//!
//! A failing step adds a line to `problems` and the next one still runs, so a report is
//! made even from a half-dead board.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// Size of one `core read` piece, bytes.
const PIECE: usize = 512;
/// Attempts to read one piece with a good CRC.
const TRIES: usize = 3;

#[derive(Debug, Default)]
pub struct BoardDump {
    /// Commands echoed as `> list`, each followed by its answer lines.
    pub text: String,
    /// The whole crash dump (verified), if the board has one.
    pub coredump: Option<Vec<u8>>,
    /// What could not be collected (Russian text).
    pub problems: Vec<String>,
}

/// CRC-32 (IEEE 802.3), as the master computes it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Drops what is waiting on the port (a moment at most: the board may log all the time).
fn drain<P: Read>(port: &mut P) {
    let started = Instant::now();
    let mut buf = [0u8; 256];
    while started.elapsed() < Duration::from_millis(300) {
        match port.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// Reads one line until `deadline`; `Ok(None)` on timeout.
fn read_line<P: Read>(port: &mut P, deadline: Instant) -> Result<Option<String>, String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match port.read(&mut byte) {
            Ok(0) => std::thread::sleep(Duration::from_millis(1)),
            Ok(_) if byte[0] == b'\n' => {
                return Ok(Some(String::from_utf8_lossy(&line).trim().to_owned()))
            }
            Ok(_) => line.push(byte[0]),
            Err(e) if is_timeout(&e) => {}
            Err(e) => return Err(format!("ошибка связи с платой: {e}")),
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
    }
}

fn send<P: Write>(port: &mut P, command: &str) -> Result<(), String> {
    port.write_all(format!("{command}\n").as_bytes())
        .and_then(|()| port.flush())
        .map_err(|e| format!("ошибка связи с платой: {e}"))
}

/// Sends `command` and returns its answer lines up to the terminator (`end` or `ok`),
/// without it. Lines of the board's own log (`warn ...`) are skipped.
fn ask<P: Read + Write>(
    port: &mut P,
    command: &str,
    terminator: &str,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    drain(port);
    send(port, command)?;
    let deadline = Instant::now() + timeout;
    let mut lines = Vec::new();
    loop {
        match read_line(port, deadline)? {
            Some(line) if line == terminator => return Ok(lines),
            Some(line) if line.starts_with("err") => return Err(format!("{command}: {line}")),
            Some(line) if line.starts_with("warn") || line.is_empty() => {}
            Some(line) => lines.push(line),
            None => return Err(format!("плата не ответила на «{command}»")),
        }
    }
}

/// Reads one piece at `offset` with the expected length; retries on a bad answer.
fn read_piece<P: Read + Write>(
    port: &mut P,
    offset: usize,
    want: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for _ in 0..TRIES {
        match try_piece(port, offset, want, timeout) {
            Ok(data) => return Ok(data),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn try_piece<P: Read + Write>(
    port: &mut P,
    offset: usize,
    want: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    drain(port);
    send(port, &format!("core read {offset}"))?;
    let deadline = Instant::now() + timeout;
    let prefix = format!("core {offset} ");
    let mut piece = None;
    loop {
        match read_line(port, deadline)? {
            Some(line) if line.starts_with(&prefix) && piece.is_none() => {
                piece = Some(parse_piece(&line[prefix.len()..], want)?);
            }
            Some(line) if line == "ok" => {
                return piece.ok_or_else(|| "плата ответила ok без данных".to_owned());
            }
            Some(line) if line.starts_with("err") => return Err(line),
            Some(_) => {}
            None => return Err(format!("нет ответа на «core read {offset}»")),
        }
    }
}

/// `<hex bytes> <crc32 8 hex>` of a piece answer.
fn parse_piece(text: &str, want: usize) -> Result<Vec<u8>, String> {
    let mut words = text.split_whitespace();
    let (Some(hex), Some(crc), None) = (words.next(), words.next(), words.next()) else {
        return Err("неверный формат порции дампа".into());
    };
    let crc = u32::from_str_radix(crc, 16).map_err(|_| "неверная CRC порции")?;
    if hex.len() != want * 2 || !hex.is_ascii() {
        return Err("неверная длина порции дампа".into());
    }
    let data = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| "неверные данные порции дампа")?;
    if crc32(&data) != crc {
        return Err("CRC порции дампа не совпала".into());
    }
    Ok(data)
}

/// `core len=<n> crc=<hex>` gives `Some((n, crc))`.
fn parse_core_info(lines: &[String]) -> Option<(usize, u32)> {
    let line = lines.iter().find(|l| l.starts_with("core len="))?;
    let mut len = None;
    let mut crc = None;
    for word in line.split_whitespace() {
        if let Some(v) = word.strip_prefix("len=") {
            len = v.parse().ok();
        } else if let Some(v) = word.strip_prefix("crc=") {
            crc = u32::from_str_radix(v, 16).ok();
        }
    }
    Some((len?, crc?))
}

fn read_core<P: Read + Write>(
    port: &mut P,
    len: usize,
    crc: u32,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut image = Vec::with_capacity(len);
    while image.len() < len {
        let offset = image.len();
        let want = PIECE.min(len - offset);
        let data = read_piece(port, offset, want, timeout)
            .map_err(|e| format!("дамп сбоя не прочитан (смещение {offset}): {e}"))?;
        image.extend_from_slice(&data);
    }
    if crc32(&image) != crc {
        return Err("дамп сбоя прочитан, но общая CRC не совпала".into());
    }
    Ok(image)
}

/// Asks the board for `list`, `info`, `log` and the crash dump. `timeout` is the limit of
/// one command (one piece for the dump).
pub fn collect<P: Read + Write>(port: &mut P, timeout: Duration) -> BoardDump {
    let mut dump = BoardDump::default();
    drain(port);
    for (command, terminator) in [("list", "end"), ("info", "end"), ("log", "end")] {
        match ask(port, command, terminator, timeout) {
            Ok(lines) => push_answer(&mut dump.text, command, &lines),
            Err(e) => dump.problems.push(e),
        }
    }
    match ask(port, "core info", "ok", timeout) {
        Ok(lines) => {
            push_answer(&mut dump.text, "core info", &lines);
            if let Some((len, crc)) = parse_core_info(&lines) {
                match read_core(port, len, crc, timeout) {
                    Ok(image) => dump.coredump = Some(image),
                    Err(e) => dump.problems.push(e),
                }
            }
        }
        Err(e) => dump
            .problems
            .push(format!("не удалось узнать о дампе сбоя: {e}")),
    }
    dump
}

fn push_answer(text: &mut String, command: &str, lines: &[String]) {
    text.push_str("> ");
    text.push_str(command);
    text.push('\n');
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
}

/// Opens the port (115200 baud) and collects like [`collect`].
#[cfg(feature = "serial")]
pub fn collect_from_port(name: &str) -> BoardDump {
    match serialport::new(name, 115_200)
        .timeout(Duration::from_millis(50))
        .open()
    {
        Ok(mut port) => collect(&mut port, Duration::from_secs(4)),
        Err(e) => BoardDump {
            problems: vec![format!("не удалось открыть порт {name}: {e}")],
            ..BoardDump::default()
        },
    }
}

/// Erases the crash dump on the board (`core erase`).
pub fn erase_core<P: Read + Write>(port: &mut P) -> Result<(), String> {
    ask(port, "core erase", "ok", Duration::from_secs(5)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    /// Answers scripted per command; a command without an answer stays silent.
    #[derive(Default)]
    struct Fake {
        answers: HashMap<String, VecDeque<String>>,
        rx: VecDeque<u8>,
        sent: Vec<String>,
    }

    impl Fake {
        fn on(&mut self, command: &str, answer: &str) {
            self.answers
                .entry(command.to_owned())
                .or_default()
                .push_back(answer.to_owned());
        }
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.rx.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => {
                    std::thread::sleep(Duration::from_millis(1));
                    Err(std::io::ErrorKind::TimedOut.into())
                }
            }
        }
    }

    impl Write for Fake {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let command = String::from_utf8_lossy(data).trim().to_owned();
            if let Some(answer) = self.answers.get_mut(&command).and_then(VecDeque::pop_front) {
                self.rx.extend(answer.bytes());
            }
            self.sent.push(command);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn hex(data: &[u8]) -> String {
        data.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn piece_answer(offset: usize, data: &[u8], crc: u32) -> String {
        format!("core {offset} {} {crc:08x}\nok\n", hex(data))
    }

    #[test]
    fn collects_text_and_dump_with_retry_and_stray_line() {
        let image: Vec<u8> = (0..1200u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut port = Fake::default();
        port.rx.extend(b"junk before\n");
        port.on("list", "warn heap low\nm run steps=1\nend\n");
        port.on("info", "master=crab-master-rs 2.0.0\nreset=panic\nend\n");
        port.on("log", "1 [x] hello\nend\n");
        port.on(
            "core info",
            &format!("core len={} crc={:08x}\nok\n", image.len(), crc32(&image)),
        );
        for (i, chunk) in image.chunks(PIECE).enumerate() {
            let offset = i * PIECE;
            if i == 1 {
                // First answer of the second piece is corrupted.
                port.on(
                    &format!("core read {offset}"),
                    &piece_answer(offset, chunk, crc32(chunk) ^ 1),
                );
            }
            port.on(
                &format!("core read {offset}"),
                &piece_answer(offset, chunk, crc32(chunk)),
            );
        }
        let dump = collect(&mut port, Duration::from_millis(200));
        assert!(dump.problems.is_empty(), "{:?}", dump.problems);
        assert_eq!(dump.coredump.as_deref(), Some(image.as_slice()));
        assert!(dump.text.contains("> list\nm run steps=1\n"));
        assert!(!dump.text.contains("warn"));
        assert!(dump.text.contains("reset=panic"));
        assert!(dump.text.contains("> core info\ncore len=1200"));
        assert_eq!(
            port.sent.iter().filter(|c| *c == "core read 512").count(),
            2
        );
    }

    #[test]
    fn missing_answer_is_a_problem_but_the_rest_is_collected() {
        let mut port = Fake::default();
        port.on("info", "master=crab-master-rs 2.0.0\nend\n");
        port.on("log", "end\n");
        port.on("core info", "core none\nok\n");
        let dump = collect(&mut port, Duration::from_millis(60));
        assert_eq!(dump.problems.len(), 1);
        assert!(dump.problems[0].contains("list"));
        assert!(dump.text.contains("> info"));
        assert!(dump.text.contains("> core info\ncore none"));
        assert!(dump.coredump.is_none());
    }

    #[test]
    fn dump_that_stays_broken_is_not_returned() {
        let image = vec![5u8; 600];
        let mut port = Fake::default();
        port.on("list", "end\n");
        port.on("info", "end\n");
        port.on("log", "end\n");
        port.on(
            "core info",
            &format!("core len=600 crc={:08x}\nok\n", crc32(&image)),
        );
        for _ in 0..TRIES {
            port.on(
                "core read 0",
                &piece_answer(0, &image[..PIECE], crc32(&image[..PIECE]) ^ 1),
            );
        }
        let dump = collect(&mut port, Duration::from_millis(100));
        assert!(dump.coredump.is_none());
        assert_eq!(dump.problems.len(), 1);
        assert!(dump.problems[0].contains("дамп"));
    }

    #[test]
    fn erase_waits_for_ok() {
        let mut port = Fake::default();
        port.on("core erase", "ok\n");
        assert!(erase_core(&mut port).is_ok());
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
