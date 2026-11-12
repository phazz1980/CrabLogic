//! Uploading one module image (`.wasm`, or the `.aot` built from it) to the master firmware over a
//! serial link. The master tells the kind by the magic of the image; the protocol is the same.
//!
//! Protocol (lines end with `\n`): `put <name> <len> <crc32 hex> <allow,list|-> mem=<bytes>`, the
//! master answers `ready <chunk>`, then the bytes follow in chunks of up to `<chunk>`, each answered
//! `ack <bytes so far>`; at the end the master answers `swap_us=…` and `ok`, or `err …`. The module
//! is stored in the flash of the board and (re)started without a reset (`specs/mcu-memory`).
//! A master before that answers a bare `ready` and takes all the bytes at once.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::wamrc::{self, AotTarget, WamrcError};

/// Longest module name the master accepts.
pub const MAX_NAME: usize = 15;
/// How long to wait for any answer of the master.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a module could not be uploaded.
#[derive(Debug, thiserror::Error)]
pub enum ModuleUploadError {
    #[error("имя модуля «{0}» не подходит: до {MAX_NAME} символов, только a-z, 0-9 и «_»")]
    BadName(String),
    #[error("в списке прав недопустимое имя «{0}»")]
    BadAllow(String),
    #[error("файл не похож на модуль (нет заголовка \\0asm или \\0aot)")]
    NotWasm,
    #[error("ошибка связи с платой: {0}")]
    Io(#[from] io::Error),
    #[error("плата не ответила за {} с", ANSWER_TIMEOUT.as_secs())]
    Timeout,
    #[error("мастер-прошивка не отвечает: на плате другая прошивка, не тот порт или плата ещё загружается")]
    NoMaster,
    #[error("плата отклонила модуль: {0}")]
    Rejected(String),
    #[error("модуль собран старой версией CrabLogic (нет сведений о памяти): пересоберите его")]
    OldModule,
    #[error("мастер не удалил модуль «{0}»")]
    NotDeleted(String),
}

/// What the master reported about a stored module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleUploaded {
    /// Time the master spent storing and restarting the module, microseconds.
    pub swap_us: Option<u32>,
}

/// CRC-32 (IEEE 802.3), as the master computes it.
pub fn crc32(data: &[u8]) -> u32 {
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

/// For a master that takes a module at once (bare `ready`): bytes sent at once, and the pause after
/// each piece.
const UPLOAD_PIECE: usize = 1024;
const PIECE_PAUSE: Duration = Duration::from_millis(15);

/// Bytes of memory the module needs, from its `crab.mem` section (`specs/mcu-memory`); `None` for a
/// module built before the section existed.
pub fn module_memory(wasm: &[u8]) -> Option<u32> {
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::CustomSection(section)) = payload {
            if section.name() == crab_codegen::wasm::MEM_SECTION {
                let bytes: [u8; 4] = section.data().try_into().ok()?;
                return Some(u32::from_le_bytes(bytes));
            }
        }
    }
    None
}

fn check_name_and_allow(name: &str, allow: &[String]) -> Result<(), ModuleUploadError> {
    let valid_ident = |s: &str| {
        !s.is_empty()
            && s.len() <= MAX_NAME
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    if !valid_ident(name) {
        return Err(ModuleUploadError::BadName(name.to_owned()));
    }
    if let Some(bad) = allow.iter().find(|a| !valid_ident(a)) {
        return Err(ModuleUploadError::BadAllow(bad.clone()));
    }
    Ok(())
}

/// Sends `wasm` to the master as module `name` with the hardware rights in `allow`.
pub fn upload_module<P: Read + Write>(
    port: &mut P,
    name: &str,
    wasm: &[u8],
    allow: &[String],
) -> Result<ModuleUploaded, ModuleUploadError> {
    // Same order of errors as before: name, rights, header, memory section.
    check_name_and_allow(name, allow)?;
    if !wasm.starts_with(b"\0asm") {
        return Err(ModuleUploadError::NotWasm);
    }
    let mem = module_memory(wasm).ok_or(ModuleUploadError::OldModule)?;
    upload_image(port, name, wasm, mem, allow)
}

/// Sends a ready image (`.wasm` or the `.aot` built from it) to the master as module `name`.
/// `mem` is the memory the module needs (`module_memory` of its `.wasm`).
pub fn upload_image<P: Read + Write>(
    port: &mut P,
    name: &str,
    image: &[u8],
    mem: u32,
    allow: &[String],
) -> Result<ModuleUploaded, ModuleUploadError> {
    check_name_and_allow(name, allow)?;
    if !(image.starts_with(b"\0asm") || image.starts_with(b"\0aot")) {
        return Err(ModuleUploadError::NotWasm);
    }
    let rights = if allow.is_empty() {
        "-".to_owned()
    } else {
        allow.join(",")
    };
    // The board may have printed boot messages or answers to earlier commands.
    drain(port)?;
    let header = format!(
        "put {name} {} {:08x} {rights} mem={mem}\n",
        image.len(),
        crc32(image)
    );
    port.write_all(header.as_bytes())?;
    port.flush()?;
    // `None` for a master that takes the module at once.
    let chunk = loop {
        let line = read_line(port)?;
        if line == "ready" {
            break None;
        }
        if let Some(n) = line.strip_prefix("ready ") {
            match n.trim().parse::<usize>() {
                Ok(n) if n > 0 => break Some(n),
                _ => {
                    return Err(ModuleUploadError::Rejected(format!(
                        "непонятный ответ «{line}»"
                    )))
                }
            }
        }
        if let Some(reason) = line.strip_prefix("err") {
            return Err(ModuleUploadError::Rejected(reason.trim().to_owned()));
        }
    };
    match chunk {
        // Each chunk waits for the master to write the one before into the flash.
        Some(chunk) => {
            let mut sent = 0;
            for piece in image.chunks(chunk) {
                port.write_all(piece)?;
                port.flush()?;
                sent += piece.len();
                loop {
                    let line = read_line(port)?;
                    if let Some(at) = line.strip_prefix("ack ") {
                        if at.trim().parse::<usize>().ok() != Some(sent) {
                            return Err(ModuleUploadError::Rejected(format!(
                                "плата приняла не столько байт: «{line}», отправлено {sent}"
                            )));
                        }
                        break;
                    }
                    if let Some(reason) = line.strip_prefix("err") {
                        return Err(ModuleUploadError::Rejected(reason.trim().to_owned()));
                    }
                }
            }
        }
        // In pieces: the native USB of an ESP32-C3 has a receive buffer of a few KB and drops what
        // does not fit when the master has not read yet (a module of 8 KB came out with a wrong CRC).
        None => {
            for piece in image.chunks(UPLOAD_PIECE) {
                port.write_all(piece)?;
                port.flush()?;
                std::thread::sleep(PIECE_PAUSE);
            }
        }
    }
    let mut swap_us = None;
    loop {
        let line = read_line(port)?;
        if let Some(value) = line.strip_prefix("swap_us=") {
            swap_us = value.trim().parse().ok();
        } else if line == "ok" {
            return Ok(ModuleUploaded { swap_us });
        } else if let Some(reason) = line.strip_prefix("err") {
            return Err(ModuleUploadError::Rejected(reason.trim().to_owned()));
        }
    }
}

/// Deletes module `name` from the master (`del`): its outputs go to the safe state and its flash
/// area is freed.
pub fn delete_module<P: Read + Write>(port: &mut P, name: &str) -> Result<(), ModuleUploadError> {
    check_name_and_allow(name, &[])?;
    drain(port)?;
    port.write_all(format!("del {name}\n").as_bytes())?;
    port.flush()?;
    loop {
        let line = read_line(port)?;
        if line == "ok" {
            return Ok(());
        }
        if line.starts_with("err") {
            return Err(ModuleUploadError::NotDeleted(name.to_owned()));
        }
    }
}

/// What goes to the board for one module.
pub struct ModuleImage {
    pub bytes: Vec<u8>,
    /// AOT image; otherwise the `.wasm` itself.
    pub aot: bool,
    /// Why the module goes as `.wasm` although the master takes AOT (shown to the user as a warning).
    pub warning: Option<String>,
}

/// Picks the image for a master: AOT when it runs AOT and the compiler is at hand, else the `.wasm`.
/// `wamrc` is asked for the compiler only when there is no cached image (it may download it).
/// The only error is `Cancelled`; any other failure falls back to the `.wasm` with a warning, so
/// work does not stop without internet.
pub fn module_image(
    info: &MasterInfo,
    wasm: &[u8],
    cache: &Path,
    wamrc: &mut dyn FnMut(AotTarget) -> Result<PathBuf, WamrcError>,
) -> Result<ModuleImage, WamrcError> {
    let plain = |warning| ModuleImage {
        bytes: wasm.to_vec(),
        aot: false,
        warning,
    };
    let Some(target) = info.aot_target() else {
        return Ok(plain(None));
    };
    let aot = |bytes| ModuleImage {
        bytes,
        aot: true,
        warning: None,
    };
    if let Some(bytes) = wamrc::cached(wasm, target, cache) {
        return Ok(aot(bytes));
    }
    let built = wamrc(target).and_then(|exe| wamrc::compile(&exe, wasm, target, cache));
    match built {
        Ok(bytes) => Ok(aot(bytes)),
        Err(WamrcError::Cancelled) => Err(WamrcError::Cancelled),
        Err(e) => Ok(plain(Some(format!(
            "Модуль залит без AOT-компиляции и будет работать медленнее (в интерпретаторе): {e}"
        )))),
    }
}

/// What a master says about itself in the answer to `stats`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MasterInfo {
    /// `id=`: the fingerprint of the firmware; `None` for a master that does not report one.
    pub id: Option<String>,
    /// `native=`: native parts of blocks built in, as (block type, instances).
    pub natives: Vec<(String, u32)>,
    /// `modcaps=`: features of modules this master has beyond its first version; empty for a
    /// master that does not report any (an old one).
    pub caps: Vec<String>,
    /// `safe=`: resets by a watchdog or a panic in a row that put the master into safe mode (its
    /// stored modules not started); 0 when it runs normally.
    pub safe_boots: u32,
}

impl MasterInfo {
    /// A master before `specs/mcu-memory` keeps the modules in RAM (no `flashmod` in `modcaps=`): it
    /// still runs them, but has much less memory left; flashing the master again fixes it.
    pub fn keeps_modules_in_ram(&self) -> bool {
        !self.caps.iter().any(|c| c == "flashmod")
    }

    /// The master runs AOT images (`aot` in `modcaps=`).
    pub fn takes_aot(&self) -> bool {
        self.caps.iter().any(|c| c == "aot")
    }

    /// CPU to build AOT images for: only when the master runs AOT and names its CPU, so an image
    /// for another architecture is never built.
    pub fn aot_target(&self) -> Option<AotTarget> {
        let has = |cap: &str| self.caps.iter().any(|c| c == cap);
        if !has("aot") {
            None
        } else if has("rv32imc") {
            Some(AotTarget::Riscv32Imc)
        } else if has("xtensa-esp32") {
            Some(AotTarget::XtensaEsp32)
        } else {
            None
        }
    }

    /// What of `needs` this master does not have, in the order of `needs`.
    pub fn missing_caps(&self, needs: &[String]) -> Vec<String> {
        needs
            .iter()
            .filter(|n| !self.caps.contains(n))
            .cloned()
            .collect()
    }

    /// Takes `id=...` and `native=type:n,...` lines; other lines are not about this.
    fn take_line(&mut self, line: &str) {
        if let Some(id) = line.strip_prefix("id=") {
            self.id = Some(id.trim().to_owned());
        } else if let Some(n) = line.strip_prefix("safe=") {
            self.safe_boots = n.trim().parse().unwrap_or(0);
        } else if let Some(list) = line.strip_prefix("modcaps=") {
            self.caps = list
                .split(',')
                .map(|c| c.trim().to_owned())
                .filter(|c| !c.is_empty())
                .collect();
        } else if let Some(list) = line.strip_prefix("native=") {
            self.natives = list
                .split(',')
                .filter_map(|item| {
                    let (kind, count) = item.trim().split_once(':')?;
                    Some((kind.to_owned(), count.parse().ok()?))
                })
                .collect();
        }
    }
}

/// The error for a module the master on the board cannot run: it lacks features the module uses.
pub fn missing_caps_message(module: &str, missing: &[String]) -> String {
    let what = missing
        .iter()
        .map(|c| cap_title(c))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "модулю «{module}» нужно от мастера: {what}. Мастер на плате старее — перезалейте мастер («Залить мастер-прошивку»)"
    )
}

/// Shown when the master on the board keeps the modules in RAM ([`MasterInfo::keeps_modules_in_ram`]).
pub const OLD_STORAGE_MESSAGE: &str = "Мастер на плате старой версии: держит модули в ОЗУ, и памяти \
     на Wi-Fi и MQTT остаётся мало. Перезалейте мастер («Залить мастер-прошивку»); после этого модули \
     заливаются заново";

/// What a master feature of `modcaps=` is for, in words.
fn cap_title(cap: &str) -> &str {
    match cap {
        "math" => "математика (sin/cos/tan/pow)",
        "retain" => "сохранение значений во флеш",
        "tag_text" => "текстовые теги между листами",
        "retain_text" => "сохранение текста во флеш",
        "uart_format" => "смена формата UART",
        "pcf_expander" => "расширители PCF8574/PCF8575",
        "uart_console" => "вывод в консоль мастера (UART0/USB)",
        "sparkplug" => "Sparkplug B (MQTT-узел)",
        "modbus" => "Modbus RTU/TCP",
        "abi4" => "модули нового формата (ABI 4)",
        "bus" => {
            "шины для драйверов (GPIO, I²C, SPI, UART, 1-Wire из block.wasm; только мастер на Rust)"
        }
        other => other,
    }
}

/// Waits until the master answers: opening the port can reset a board with native USB
/// (ESP32-C3), and a command sent while it boots is lost. Pings with `stats` until the
/// answer ends with `end`.
pub fn wait_for_master<P: Read + Write>(
    port: &mut P,
    within: Duration,
) -> Result<(), ModuleUploadError> {
    wait_for_master_info(port, within).map(|_| ())
}

/// Like [`wait_for_master`], and returns what the master reported about itself.
pub fn wait_for_master_info<P: Read + Write>(
    port: &mut P,
    within: Duration,
) -> Result<MasterInfo, ModuleUploadError> {
    const PING_ANSWER: Duration = Duration::from_millis(700);
    let started = Instant::now();
    while started.elapsed() < within {
        port.write_all(b"stats\n")?;
        port.flush()?;
        let ping = Instant::now();
        let mut info = MasterInfo::default();
        while ping.elapsed() < PING_ANSWER {
            match read_line_within(port, PING_ANSWER) {
                Ok(line) if line == "end" => return Ok(info),
                Ok(line) => info.take_line(&line),
                Err(ModuleUploadError::Timeout) => {}
                Err(e) => return Err(e),
            }
        }
    }
    Err(ModuleUploadError::NoMaster)
}

/// Reads and drops whatever is waiting on the port. Stops after a moment even if the
/// board keeps talking (a module that logs errors in a loop never lets the port go quiet).
fn drain<P: Read>(port: &mut P) -> Result<(), ModuleUploadError> {
    const LIMIT: Duration = Duration::from_millis(300);
    let started = Instant::now();
    let mut buf = [0u8; 256];
    while started.elapsed() < LIMIT {
        match port.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if is_timeout(&e) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// Reads one trimmed line; read timeouts of the port are retried until `ANSWER_TIMEOUT`.
pub(crate) fn read_line<P: Read>(port: &mut P) -> Result<String, ModuleUploadError> {
    read_line_within(port, ANSWER_TIMEOUT)
}

fn read_line_within<P: Read>(port: &mut P, limit: Duration) -> Result<String, ModuleUploadError> {
    let started = Instant::now();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match port.read(&mut byte) {
            Ok(0) => return Err(ModuleUploadError::Timeout),
            Ok(_) if byte[0] == b'\n' => {
                return Ok(String::from_utf8_lossy(&line).trim().to_owned())
            }
            Ok(_) => line.push(byte[0]),
            Err(e) if is_timeout(&e) => {}
            Err(e) => return Err(e.into()),
        }
        if started.elapsed() > limit {
            return Err(ModuleUploadError::Timeout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A fake master: records what was written, answers from a script once the header is in.
    struct Fake {
        written: Vec<u8>,
        replies: VecDeque<u8>,
        ready: Vec<u8>,
        done: Vec<u8>,
        sent_ready: bool,
    }

    impl Fake {
        fn new(ready: &str, done: &str) -> Self {
            Fake {
                written: Vec::new(),
                replies: VecDeque::new(),
                ready: ready.as_bytes().to_vec(),
                done: done.as_bytes().to_vec(),
                sent_ready: false,
            }
        }
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.replies.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Err(io::ErrorKind::TimedOut.into()),
            }
        }
    }

    impl Write for Fake {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(data);
            if !self.sent_ready && self.written.ends_with(b"\n") {
                self.sent_ready = true;
                self.replies.extend(self.ready.clone());
            } else if self.sent_ready && (data.starts_with(b"\0asm") || data.starts_with(b"\0aot"))
            {
                self.replies.extend(self.done.clone());
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A master that takes the module in chunks: answers `ack` to each one, or what `script` says
    /// for the chunk with that number (0 = the answer to the header, instead of `ready`).
    struct ChunkMaster {
        chunk: usize,
        len: usize,
        written: Vec<u8>,
        header_len: Option<usize>,
        chunks: usize,
        out: VecDeque<u8>,
        script: Vec<(usize, &'static str)>,
        done: &'static str,
    }

    impl ChunkMaster {
        fn new(chunk: usize, len: usize, done: &'static str) -> Self {
            ChunkMaster {
                chunk,
                len,
                written: Vec::new(),
                header_len: None,
                chunks: 0,
                out: VecDeque::new(),
                script: Vec::new(),
                done,
            }
        }

        fn data(&self) -> &[u8] {
            &self.written[self.header_len.unwrap_or(self.written.len())..]
        }
    }

    impl Read for ChunkMaster {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.out.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Err(io::ErrorKind::TimedOut.into()),
            }
        }
    }

    impl Write for ChunkMaster {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(data);
            let scripted =
                |m: &Self, n: usize| m.script.iter().find(|(i, _)| *i == n).map(|(_, t)| *t);
            match self.header_len {
                None if self.written.ends_with(b"\n") => {
                    self.header_len = Some(self.written.len());
                    let answer = scripted(self, 0)
                        .map(str::to_owned)
                        .unwrap_or(format!("ready {}\n", self.chunk));
                    self.out.extend(answer.into_bytes());
                }
                None => {}
                Some(_) => {
                    assert!(
                        data.len() <= self.chunk,
                        "a chunk longer than the master takes"
                    );
                    self.chunks += 1;
                    let got = self.data().len();
                    let answer = match scripted(self, self.chunks) {
                        Some(t) => t.to_owned(),
                        None if got == self.len => format!("ack {got}\n{}", self.done),
                        None => format!("ack {got}\n"),
                    };
                    self.out.extend(answer.into_bytes());
                }
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// An empty module with a `crab.mem` section of 4096 bytes.
    const WASM: &[u8] = b"\0asm\x01\0\0\0\0\x0d\x08crab.mem\0\x10\0\0";

    /// A master that answers `stats` with the given text.
    struct StatsMaster {
        answer: Vec<u8>,
        out: std::collections::VecDeque<u8>,
    }

    impl Read for StatsMaster {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.out.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Err(io::ErrorKind::TimedOut.into()),
            }
        }
    }

    impl Write for StatsMaster {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if data
                == b"stats
"
            {
                self.out.extend(self.answer.clone());
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn master_info_comes_from_the_stats_answer() {
        let mut port = StatsMaster {
            answer: b"blink run steps=5 restarts=0 out0=00000000 crc=00000000
id=00ab12cd34ef5678
native=bmp:2,twice:1
safe=3
free=1 largest=1
end
"
            .to_vec(),
            out: Default::default(),
        };
        let info = wait_for_master_info(&mut port, Duration::from_secs(2)).unwrap();
        assert_eq!(info.id.as_deref(), Some("00ab12cd34ef5678"));
        assert_eq!(info.safe_boots, 3);
        assert_eq!(
            info.natives,
            [("bmp".to_owned(), 2), ("twice".to_owned(), 1)]
        );
        // An older master says nothing about itself.
        let mut old = StatsMaster {
            answer: b"free=1 largest=1
end
"
            .to_vec(),
            out: Default::default(),
        };
        assert_eq!(
            wait_for_master_info(&mut old, Duration::from_secs(2)).unwrap(),
            MasterInfo::default()
        );
    }

    #[test]
    fn caps_of_the_master_decide_which_modules_it_can_run() {
        let mut port = StatsMaster {
            answer: b"free=1 largest=1
caps=ev,us
modcaps=math,retain
end
"
            .to_vec(),
            out: Default::default(),
        };
        let info = wait_for_master_info(&mut port, Duration::from_secs(2)).unwrap();
        assert_eq!(info.caps, ["math", "retain"]);
        let needs = ["math".to_owned(), "uart_format".to_owned()];
        assert_eq!(info.missing_caps(&needs), ["uart_format"]);
        assert!(info.missing_caps(&[]).is_empty());
        // An old master reports nothing: everything new is missing.
        assert_eq!(MasterInfo::default().missing_caps(&needs), needs);
        let text = missing_caps_message("main", &["uart_format".to_owned()]);
        assert!(
            text.contains("«main»") && text.contains("перезалейте мастер"),
            "{text}"
        );
    }

    /// A module of `n` bytes with a `crab.mem` section.
    fn big_module(n: usize) -> Vec<u8> {
        let mut wasm = WASM.to_vec();
        let pad = n - wasm.len() - 6;
        // A custom section "x" with padding (section id 0, LEB128 size).
        wasm.push(0);
        let size = (pad + 2) as u32;
        wasm.extend([
            (size & 0x7f) as u8 | 0x80,
            ((size >> 7) & 0x7f) as u8 | 0x80,
            (size >> 14) as u8,
        ]);
        wasm.extend([1, b'x']);
        wasm.resize(n, 0xAB);
        wasm
    }

    #[test]
    fn uploads_in_chunks_answered_by_ack() {
        let wasm = big_module(1300);
        let mut port = ChunkMaster::new(512, wasm.len(), "swap_us=120\nok\n");
        let done = upload_module(&mut port, "big", &wasm, &[]).unwrap();
        assert_eq!(done.swap_us, Some(120));
        assert_eq!(port.chunks, 3);
        assert_eq!(port.data(), wasm.as_slice());
    }

    #[test]
    fn chunk_upload_stops_on_board_error() {
        let wasm = big_module(1300);
        // The cable is pulled: the master gives up waiting for the second chunk.
        let mut port = ChunkMaster::new(512, wasm.len(), "ok\n");
        port.script.push((2, "err timeout\n"));
        let e = upload_module(&mut port, "big", &wasm, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r == "timeout"));
        assert_eq!(port.chunks, 2, "nothing is sent after the error");
        // A wrong CRC at the end.
        let mut port = ChunkMaster::new(512, wasm.len(), "err crc/len\n");
        let e = upload_module(&mut port, "big", &wasm, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r == "crc/len"));
        // No free area.
        let mut port = ChunkMaster::new(512, wasm.len(), "ok\n");
        port.script.push((0, "err no space\n"));
        let e = upload_module(&mut port, "big", &wasm, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r == "no space"));
        assert!(port.data().is_empty());
    }

    #[test]
    fn chunk_upload_checks_the_ack_offset() {
        let wasm = big_module(1300);
        let mut port = ChunkMaster::new(512, wasm.len(), "ok\n");
        port.script.push((1, "ack 100\n"));
        let e = upload_module(&mut port, "big", &wasm, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r.contains("ack 100")));
    }

    #[test]
    fn master_without_flashmod_keeps_modules_in_ram() {
        let info = |caps: &str| {
            let mut port = StatsMaster {
                answer: format!("modcaps={caps}\nend\n").into_bytes(),
                out: Default::default(),
            };
            wait_for_master_info(&mut port, Duration::from_secs(2)).unwrap()
        };
        assert!(info("math,retain").keeps_modules_in_ram());
        assert!(!info("math,flashmod,retain").keeps_modules_in_ram());
        assert!(MasterInfo::default().keeps_modules_in_ram());
        assert!(OLD_STORAGE_MESSAGE.contains("Перезалейте мастер"));
    }

    #[test]
    fn uploads_an_aot_image_with_its_own_length_and_crc() {
        let aot = b"\0aot\x05\0\0\0some code bytes";
        let mut port = Fake::new("ready\n", "ok\n");
        let allow = vec!["gpio_write".to_owned()];
        upload_image(&mut port, "blink", aot, 4096, &allow).unwrap();
        let header = format!(
            "put blink {} {:08x} gpio_write mem=4096\n",
            aot.len(),
            crc32(aot)
        );
        assert!(port.written.starts_with(header.as_bytes()));
        assert!(port.written.ends_with(aot));
    }

    #[test]
    fn upload_image_refuses_garbage_and_module_entry_refuses_aot() {
        let mut port = Fake::new("ready\n", "ok\n");
        assert!(matches!(
            upload_image(&mut port, "m", b"hello world", 4096, &[]),
            Err(ModuleUploadError::NotWasm)
        ));
        assert!(matches!(
            upload_module(&mut port, "m", b"\0aot\x05\0\0\0", &[]),
            Err(ModuleUploadError::NotWasm)
        ));
        assert!(port.written.is_empty());
    }

    #[test]
    fn takes_aot_comes_from_modcaps() {
        let info = |answer: &str| {
            let mut port = StatsMaster {
                answer: answer.as_bytes().to_vec(),
                out: Default::default(),
            };
            wait_for_master_info(&mut port, Duration::from_secs(2)).unwrap()
        };
        assert!(info("modcaps=flashmod,abi4,aot\nend\n").takes_aot());
        assert!(!info("modcaps=math,retain\nend\n").takes_aot());
        assert!(!info("free=1 largest=1\nend\n").takes_aot());
    }

    fn info_with(caps: &[&str]) -> MasterInfo {
        MasterInfo {
            caps: caps.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        }
    }

    fn temp_cache(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("crab-image-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    const TINY_WASM: &[u8] = b"\0asm\x01\0\0\0";

    #[test]
    fn aot_target_needs_aot_and_cpu() {
        assert_eq!(
            info_with(&["aot", "rv32imc"]).aot_target(),
            Some(AotTarget::Riscv32Imc)
        );
        assert_eq!(
            info_with(&["aot", "xtensa-esp32"]).aot_target(),
            Some(AotTarget::XtensaEsp32)
        );
        assert_eq!(info_with(&["aot"]).aot_target(), None);
        assert_eq!(info_with(&["rv32imc"]).aot_target(), None);
        assert_eq!(info_with(&["xtensa-esp32"]).aot_target(), None);
        assert_eq!(info_with(&[]).aot_target(), None);
    }

    #[test]
    fn image_is_wasm_when_master_cannot_aot() {
        let cache = temp_cache("noaot");
        for caps in [&["flashmod"][..], &["aot"][..]] {
            let mut asked = false;
            let image = module_image(&info_with(caps), TINY_WASM, &cache, &mut |_| {
                asked = true;
                Err(WamrcError::Cancelled)
            })
            .unwrap();
            assert!(!asked);
            assert!(!image.aot);
            assert_eq!(image.bytes, TINY_WASM);
            assert!(image.warning.is_none());
        }
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn image_cache_hit_skips_compiler() {
        let cache = temp_cache("hit");
        let key = wamrc::cache_key(TINY_WASM, AotTarget::Riscv32Imc);
        std::fs::write(cache.join(format!("{key}.aot")), b"\0aot cached").unwrap();
        let mut asked = false;
        let image = module_image(
            &info_with(&["aot", "rv32imc"]),
            TINY_WASM,
            &cache,
            &mut |_| {
                asked = true;
                Err(WamrcError::Cancelled)
            },
        )
        .unwrap();
        assert!(!asked);
        assert!(image.aot);
        assert_eq!(image.bytes, b"\0aot cached");
        assert!(image.warning.is_none());
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn image_falls_back_to_wasm_with_warning() {
        let cache = temp_cache("fallback");
        let info = info_with(&["aot", "rv32imc"]);
        let image = module_image(&info, TINY_WASM, &cache, &mut |_| {
            Err(WamrcError::Download("нет сети".to_owned()))
        })
        .unwrap();
        assert!(!image.aot);
        assert_eq!(image.bytes, TINY_WASM);
        let warning = image.warning.unwrap();
        assert!(warning.contains("нет сети"), "{warning}");
        assert!(warning.contains("медленнее"), "{warning}");

        let missing = cache.join("no-such-wamrc.exe");
        let image = module_image(&info, TINY_WASM, &cache, &mut |_| Ok(missing.clone())).unwrap();
        assert!(!image.aot);
        assert!(image.warning.is_some());
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn image_cancel_is_an_error() {
        let cache = temp_cache("cancel");
        let result = module_image(
            &info_with(&["aot", "rv32imc"]),
            TINY_WASM,
            &cache,
            &mut |_| Err(WamrcError::Cancelled),
        );
        assert!(matches!(result, Err(WamrcError::Cancelled)));
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn uploads_and_reports_swap_time() {
        let mut port = Fake::new("ready\n", "swap_us=9274\nok\n");
        let allow = vec!["gpio_write".to_owned(), "millis".to_owned()];
        let done = upload_module(&mut port, "blink", WASM, &allow).unwrap();
        assert_eq!(done.swap_us, Some(9274));
        let header = format!(
            "put blink {} {:08x} gpio_write,millis mem=4096\n",
            WASM.len(),
            crc32(WASM)
        );
        assert!(port.written.starts_with(header.as_bytes()));
        assert!(port.written.ends_with(WASM));
    }

    #[test]
    fn no_rights_are_sent_as_dash() {
        let mut port = Fake::new("ready\n", "ok\n");
        upload_module(&mut port, "hang", WASM, &[]).unwrap();
        assert!(String::from_utf8_lossy(&port.written).contains(" - mem=4096\n"));
    }

    #[test]
    fn board_error_after_data_is_reported() {
        let mut port = Fake::new("ready\n", "err crc/len\n");
        let e = upload_module(&mut port, "blink", WASM, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r == "crc/len"));
    }

    #[test]
    fn board_error_before_data_is_reported() {
        let mut port = Fake::new("err no memory\n", "");
        let e = upload_module(&mut port, "blink", WASM, &[]).unwrap_err();
        assert!(matches!(e, ModuleUploadError::Rejected(r) if r == "no memory"));
        assert!(!port.written.ends_with(WASM));
    }

    #[test]
    fn rejects_bad_input_without_touching_the_port() {
        let mut port = Fake::new("ready\n", "ok\n");
        assert!(matches!(
            upload_module(&mut port, "Bad Name", WASM, &[]),
            Err(ModuleUploadError::BadName(_))
        ));
        assert!(matches!(
            upload_module(&mut port, "m", b"hello", &[]),
            Err(ModuleUploadError::NotWasm)
        ));
        assert!(matches!(
            upload_module(&mut port, "m", WASM, &["a b".to_owned()]),
            Err(ModuleUploadError::BadAllow(_))
        ));
        assert!(matches!(
            upload_module(&mut port, "m", b"\0asm\x01\0\0\0", &[]),
            Err(ModuleUploadError::OldModule)
        ));
        assert!(port.written.is_empty());
    }

    #[test]
    fn delete_module_sends_del_and_waits_for_ok() {
        let mut port = Fake::new(
            "noise
ok
",
            "",
        );
        delete_module(&mut port, "sheet1").unwrap();
        assert_eq!(
            port.written,
            b"del sheet1
"
        );
    }

    #[test]
    fn delete_module_reports_err() {
        let mut port = Fake::new(
            "err no such module
",
            "",
        );
        let err = delete_module(&mut port, "sheet1").unwrap_err();
        assert!(matches!(err, ModuleUploadError::NotDeleted(_)));
        assert!(err.to_string().contains("«sheet1»"));
    }
}
