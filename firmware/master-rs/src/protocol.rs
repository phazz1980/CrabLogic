//! The line protocol of the master over USB, as the editor already speaks it to the C++ master
//! (`crab-build/src/mcu_module.rs`, `crab-debug/src/master.rs`).
//!
//! Lines end with `\n`. `put <name> <len> <crc32 hex> <allow,list|-> [mem=<bytes>]` is answered
//! `ready <chunk>`; then the image follows in chunks of up to `<chunk>` bytes, each answered
//! `ack <bytes so far>`; at the end `swap_us=<n>` and `ok`, or `err <reason>`.
//!
//! The debugger commands (`in`, `param`, `out`, `ev`, `force`, `unforce`, `reset`, `pins`) keep the
//! C++ master's wording; `format_*` build the answer lines (the caller adds `ok` / `end`).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::{self, Write as _};

use crate::storage::{MAX_ALLOW, MAX_NAME};

/// Commands the Rust master understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Put {
        name: String,
        len: usize,
        crc32: u32,
        /// Hardware rights, comma separated; empty for none.
        allow: String,
        /// Bytes of linear memory (`crab.mem`), if the sender gave them.
        memory: Option<u32>,
    },
    /// `list` and `stats` are one command, as in the C++ master.
    List,
    /// The storage: free space and the stored images.
    Info,
    /// `spdump <ms>`: the Sparkplug messages as hex, nothing is sent.
    SpDump {
        ts_ms: u64,
    },
    Tick {
        name: String,
        count: u32,
        dt_ms: u32,
    },
    Delete {
        name: String,
    },
    /// Restarts a module from its stored image.
    Swap {
        name: String,
    },
    /// Forgets the longest step times.
    ResetMax,
    /// The journal of module errors.
    Log,
    /// Sets input `index` of a module to the raw 32-bit `word`.
    SetInput {
        name: String,
        index: usize,
        word: u32,
    },
    /// Sets parameter `index` of a module to the raw 32-bit `word`.
    SetParam {
        name: String,
        index: usize,
        word: u32,
    },
    /// Reads the first `count` outputs of a module.
    Outputs {
        name: String,
        count: usize,
    },
    /// Changes of 0/1 outputs since the previous `ev`.
    Events {
        name: String,
    },
    /// Forces output `index` to the raw 32-bit `word`.
    Force {
        name: String,
        index: usize,
        word: u32,
    },
    /// Releases one forced output; `index` is `None` for `all`.
    Unforce {
        name: String,
        index: Option<usize>,
    },
    /// Restarts a module with cleared state.
    Reset {
        name: String,
    },
    /// The held pins.
    Pins,
    /// `topic <name> <state|event> <maxlen>`: declares a bus topic.
    Topic {
        name: String,
        state: bool,
        max_len: usize,
    },
    /// `queue <module> <n>`: the event queue of a module.
    Queue {
        name: String,
        cap: usize,
    },
    /// `bus`: the queues of the modules and the topics.
    Bus,
}

/// Longest topic name, exclusive (as in the C++ master).
pub const MAX_TOPIC_NAME: usize = 24;
/// Longest message of a topic.
pub const MAX_TOPIC_LEN: usize = 64;
/// Longest queue of a module.
pub const MAX_QUEUE: usize = 16;

/// Inputs and parameters per module.
pub const MAX_SLOTS: usize = 8;
/// Outputs per module (force index `< 128`, `out` count `<= 128`).
pub const MAX_OUTPUTS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    UnknownCommand,
    MissingArgument,
    InvalidNumber,
    InvalidCrc,
    InvalidName,
    InvalidAllow,
    TooManyArguments,
    /// Index out of range, or the type is not `i`/`f`.
    BadIndexOrType,
    BadIndex,
    BadCount,
    /// A malformed `topic`, `queue` or `bus` line.
    BadBus,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Empty => "пустая команда",
            Self::UnknownCommand => "неизвестная команда",
            Self::MissingArgument => "не хватает аргумента",
            Self::InvalidNumber => "неверное число",
            Self::InvalidCrc => "неверный CRC32",
            Self::InvalidName => "неверное имя модуля",
            Self::InvalidAllow => "неверный список прав",
            Self::TooManyArguments => "лишние аргументы",
            // The editor matches these words of the C++ master.
            Self::BadIndexOrType => "bad index or type",
            Self::BadIndex => "bad index",
            Self::BadCount => "bad count",
            Self::BadBus => "bad bus command",
        };
        f.write_str(text)
    }
}

fn ident(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn name(value: &str) -> Result<String, ParseError> {
    if value.len() > MAX_NAME || !ident(value) {
        return Err(ParseError::InvalidName);
    }
    Ok(String::from(value))
}

fn one<'a>(args: &mut impl Iterator<Item = &'a str>) -> Result<&'a str, ParseError> {
    args.next().ok_or(ParseError::MissingArgument)
}

fn number<'a, T: core::str::FromStr>(
    args: &mut impl Iterator<Item = &'a str>,
) -> Result<T, ParseError> {
    one(args)?.parse().map_err(|_| ParseError::InvalidNumber)
}

fn no_more<'a>(args: &mut impl Iterator<Item = &'a str>) -> Result<(), ParseError> {
    if args.next().is_some() {
        Err(ParseError::TooManyArguments)
    } else {
        Ok(())
    }
}

/// `<idx> <i|f> <value>` of `in`, `param` and `force`: the slot and the raw word.
fn typed_word<'a>(
    args: &mut impl Iterator<Item = &'a str>,
    limit: usize,
) -> Result<(usize, u32), ParseError> {
    let index: usize = one(args)?.parse().map_err(|_| ParseError::BadIndexOrType)?;
    let ty = one(args)?;
    let value = one(args)?;
    if index >= limit {
        return Err(ParseError::BadIndexOrType);
    }
    let word = match ty {
        "i" => {
            let n = match value.parse::<i64>() {
                Ok(n) => n,
                // Like strtoll, a number too long for i64 saturates.
                Err(_) if is_integer(value) => {
                    if value.starts_with('-') {
                        i64::MIN
                    } else {
                        i64::MAX
                    }
                }
                Err(_) => return Err(ParseError::InvalidNumber),
            };
            n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32 as u32
        }
        "f" => value
            .parse::<f32>()
            .map_err(|_| ParseError::InvalidNumber)?
            .to_bits(),
        _ => return Err(ParseError::BadIndexOrType),
    };
    Ok((index, word))
}

fn is_integer(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// `bus`, `topic <name> <state|event> <maxlen>` and `queue <module> <n>`; any slip is `BadBus`.
fn parse_bus<'a>(
    command: &str,
    mut args: impl Iterator<Item = &'a str>,
) -> Result<Command, ParseError> {
    let bad = ParseError::BadBus;
    let parsed = match command {
        "bus" => Command::Bus,
        "topic" => {
            let topic = args.next().filter(|n| ident(n)).ok_or(bad.clone())?;
            let state = match args.next() {
                Some("state") => true,
                Some("event") => false,
                _ => return Err(bad),
            };
            let max_len: usize = args
                .next()
                .and_then(|v| v.parse().ok())
                .filter(|n| (1..=MAX_TOPIC_LEN).contains(n))
                .ok_or(bad.clone())?;
            if topic.len() >= MAX_TOPIC_NAME {
                return Err(bad);
            }
            Command::Topic {
                name: String::from(topic),
                state,
                max_len,
            }
        }
        _ => {
            let module = args.next().filter(|n| ident(n)).ok_or(bad.clone())?;
            let cap: usize = args
                .next()
                .and_then(|v| v.parse().ok())
                .filter(|n| (1..=MAX_QUEUE).contains(n))
                .ok_or(bad.clone())?;
            Command::Queue {
                name: String::from(module),
                cap,
            }
        }
    };
    if args.next().is_some() {
        return Err(bad);
    }
    Ok(parsed)
}

/// Parses one command line (without its newline). The image of a `put` is not a line: see
/// [`Input`].
pub fn parse_command(line: &str) -> Result<Command, ParseError> {
    let mut args = line.split_ascii_whitespace();
    let command = args.next().ok_or(ParseError::Empty)?;
    let parsed = match command {
        "put" => {
            let module = name(one(&mut args)?)?;
            let len = number(&mut args)?;
            let crc32 =
                u32::from_str_radix(one(&mut args)?, 16).map_err(|_| ParseError::InvalidCrc)?;
            let allow = match one(&mut args)? {
                "-" => String::new(),
                list if list.len() <= MAX_ALLOW && list.split(',').all(ident) => String::from(list),
                _ => return Err(ParseError::InvalidAllow),
            };
            // `key=value` words after the rights; unknown ones are for newer masters.
            let mut memory = None;
            for word in args.by_ref() {
                match word.split_once('=') {
                    Some(("mem", v)) => {
                        memory = Some(v.parse().map_err(|_| ParseError::InvalidNumber)?)
                    }
                    Some(_) => {}
                    None => return Err(ParseError::TooManyArguments),
                }
            }
            Command::Put {
                name: module,
                len,
                crc32,
                allow,
                memory,
            }
        }
        "list" | "stats" => Command::List,
        "info" => Command::Info,
        "rmax" => Command::ResetMax,
        "log" => Command::Log,
        "tick" => Command::Tick {
            name: name(one(&mut args)?)?,
            count: number(&mut args)?,
            dt_ms: number(&mut args)?,
        },
        "del" => Command::Delete {
            name: name(one(&mut args)?)?,
        },
        "swap" => Command::Swap {
            name: name(one(&mut args)?)?,
        },
        "in" | "param" | "force" => {
            let module = name(one(&mut args)?)?;
            let limit = if command == "force" {
                MAX_OUTPUTS
            } else {
                MAX_SLOTS
            };
            let (index, word) = typed_word(&mut args, limit)?;
            match command {
                "in" => Command::SetInput {
                    name: module,
                    index,
                    word,
                },
                "param" => Command::SetParam {
                    name: module,
                    index,
                    word,
                },
                _ => Command::Force {
                    name: module,
                    index,
                    word,
                },
            }
        }
        "unforce" => {
            let module = name(one(&mut args)?)?;
            let index = match one(&mut args)? {
                "all" => None,
                word => match word.parse::<usize>() {
                    Ok(i) if i < MAX_OUTPUTS => Some(i),
                    _ => return Err(ParseError::BadIndex),
                },
            };
            Command::Unforce {
                name: module,
                index,
            }
        }
        "out" => {
            let module = name(one(&mut args)?)?;
            let count = match one(&mut args)?.parse::<usize>() {
                Ok(n) if n <= MAX_OUTPUTS => n,
                _ => return Err(ParseError::BadCount),
            };
            Command::Outputs {
                name: module,
                count,
            }
        }
        "ev" => Command::Events {
            name: name(one(&mut args)?)?,
        },
        "reset" => Command::Reset {
            name: name(one(&mut args)?)?,
        },
        "pins" => Command::Pins,
        "spdump" => Command::SpDump {
            ts_ms: one(&mut args)?
                .parse()
                .map_err(|_| ParseError::InvalidNumber)?,
        },
        "bus" | "topic" | "queue" => parse_bus(command, &mut args)?,
        _ => return Err(ParseError::UnknownCommand),
    };
    no_more(&mut args)?;
    Ok(parsed)
}

/// `out 0000002a 3f800000`: the first `count` outputs, 8 hex digits each; missing ones are zero.
pub fn format_outputs(outputs: &[u32], count: usize) -> String {
    let mut line = String::from("out");
    for i in 0..count {
        let _ = write!(line, " {:08x}", outputs.get(i).copied().unwrap_or(0));
    }
    line
}

/// `ev <name> <now ms> <lost> <slot>:<value>:<ms> ...`
pub fn format_events(name: &str, now_ms: u32, lost: u32, events: &[(u8, u8, u32)]) -> String {
    let mut line = String::new();
    let _ = write!(line, "ev {name} {now_ms} {lost}");
    for (slot, value, ms) in events {
        let _ = write!(line, " {slot}:{value}:{ms}");
    }
    line
}

/// One line of `pins`: `<pin> <module> out|pwm|pullup fault=0|1|hold[ level=0|1]`.
pub fn format_pin(pin: u8, module: &str, use_: &str, fault: &str, level: Option<bool>) -> String {
    let mut line = String::new();
    let _ = write!(line, "{pin} {module} {use_} fault={fault}");
    if let Some(level) = level {
        let _ = write!(line, " level={}", u8::from(level));
    }
    line
}

/// `bus <module> queue=<cap> lost=<n>`
pub fn format_bus_module(name: &str, cap: usize, lost: u32) -> String {
    let mut line = String::new();
    let _ = write!(line, "bus {name} queue={cap} lost={lost}");
    line
}

/// `topic <name> <state|event> max=<n> refused=<n> last=<hex|->`; the value is shown for a
/// state topic only.
pub fn format_topic(
    name: &str,
    state: bool,
    max_len: usize,
    refused: u32,
    last: Option<&[u8]>,
) -> String {
    let mut line = String::new();
    let kind = if state { "state" } else { "event" };
    let _ = write!(
        line,
        "topic {name} {kind} max={max_len} refused={refused} last="
    );
    match last.filter(|_| state) {
        Some(bytes) => {
            for b in bytes {
                let _ = write!(line, "{b:02x}");
            }
        }
        None => line.push('-'),
    }
    line
}

/// Longest command line; a longer run of bytes with no newline is noise and is dropped.
const MAX_LINE: usize = 512;

/// What [`Input`] made of the bytes received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A command line, trimmed.
    Line(String),
    /// The next chunk of the image of a `put`.
    Chunk(Vec<u8>),
}

/// The bytes of the port as a stream of lines and, after an accepted `put`, of image chunks.
#[derive(Debug, Default)]
pub struct Input {
    buf: Vec<u8>,
    /// Image bytes still expected and the chunk size announced with `ready`.
    body: Option<(usize, usize)>,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next `len` bytes are an image, taken in chunks of `chunk` (the last one is shorter).
    pub fn expect_body(&mut self, len: usize, chunk: usize) {
        self.body = (len > 0).then_some((len, chunk.max(1)));
    }

    /// Back to lines; what was received of the image so far is dropped.
    pub fn cancel_body(&mut self) {
        self.body = None;
        self.buf.clear();
    }

    pub fn in_body(&self) -> bool {
        self.body.is_some()
    }

    /// The next whole line or chunk, `None` until more bytes come.
    pub fn next_event(&mut self) -> Option<Event> {
        if let Some((left, chunk)) = self.body {
            let n = left.min(chunk);
            if self.buf.len() < n {
                return None;
            }
            let data: Vec<u8> = self.buf.drain(..n).collect();
            self.body = (left > n).then_some((left - n, chunk));
            return Some(Event::Chunk(data));
        }
        loop {
            let Some(end) = self.buf.iter().position(|&b| b == b'\n') else {
                if self.buf.len() > MAX_LINE {
                    self.buf.clear();
                }
                return None;
            };
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if !text.is_empty() {
                return Some(Event::Line(String::from(text)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_put_as_the_editor_sends_it() {
        assert_eq!(
            parse_command("put pump 1024 00a1b2c3 gpio_write,gpio_read mem=4096"),
            Ok(Command::Put {
                name: String::from("pump"),
                len: 1024,
                crc32: 0x00a1_b2c3,
                allow: String::from("gpio_write,gpio_read"),
                memory: Some(4096),
            })
        );
        // No rights, no memory, a word of a newer editor.
        assert_eq!(
            parse_command("put pump 8 0 - later=1"),
            Ok(Command::Put {
                name: String::from("pump"),
                len: 8,
                crc32: 0,
                allow: String::new(),
                memory: None,
            })
        );
    }

    #[test]
    fn parses_the_other_commands() {
        assert_eq!(parse_command("list"), Ok(Command::List));
        assert_eq!(parse_command("stats"), Ok(Command::List));
        assert_eq!(parse_command("info"), Ok(Command::Info));
        assert_eq!(parse_command("rmax"), Ok(Command::ResetMax));
        assert_eq!(parse_command("log"), Ok(Command::Log));
        assert_eq!(
            parse_command("tick sheet_1 3 20"),
            Ok(Command::Tick {
                name: String::from("sheet_1"),
                count: 3,
                dt_ms: 20
            })
        );
        assert_eq!(
            parse_command("del sheet_1"),
            Ok(Command::Delete {
                name: String::from("sheet_1")
            })
        );
        assert_eq!(
            parse_command("swap sheet_1"),
            Ok(Command::Swap {
                name: String::from("sheet_1")
            })
        );
    }

    #[test]
    fn rejects_bad_input_without_panicking() {
        assert_eq!(parse_command(""), Err(ParseError::Empty));
        assert_eq!(parse_command("fly"), Err(ParseError::UnknownCommand));
        assert_eq!(
            parse_command("put x nope 00 -"),
            Err(ParseError::InvalidNumber)
        );
        assert_eq!(parse_command("put x 1 zzzz -"), Err(ParseError::InvalidCrc));
        assert_eq!(
            parse_command("put x 1 00 gpio write"),
            Err(ParseError::TooManyArguments)
        );
        assert_eq!(
            parse_command("put x 1 00 gpio/write"),
            Err(ParseError::InvalidAllow)
        );
        assert_eq!(
            parse_command("put x 1 00 - mem=lots"),
            Err(ParseError::InvalidNumber)
        );
        assert_eq!(parse_command("del bad/name"), Err(ParseError::InvalidName));
        assert_eq!(
            parse_command("del a_name_longer_than_15"),
            Err(ParseError::InvalidName)
        );
        assert_eq!(parse_command("tick x 1"), Err(ParseError::MissingArgument));
        assert_eq!(
            parse_command("list extra"),
            Err(ParseError::TooManyArguments)
        );
    }

    fn typed(cmd: &str) -> Command {
        parse_command(cmd).unwrap()
    }

    #[test]
    fn parses_the_debugger_commands() {
        let n = || String::from("s");
        assert_eq!(
            typed("force s 3 i 1"),
            Command::Force {
                name: n(),
                index: 3,
                word: 1
            }
        );
        assert_eq!(
            typed("force s 4 f 2.5"),
            Command::Force {
                name: n(),
                index: 4,
                word: 2.5f32.to_bits()
            }
        );
        assert_eq!(
            typed("in s 0 i -5"),
            Command::SetInput {
                name: n(),
                index: 0,
                word: (-5i32) as u32
            }
        );
        assert_eq!(
            typed("in s 0 i 99999999999"),
            Command::SetInput {
                name: n(),
                index: 0,
                word: i32::MAX as u32
            }
        );
        assert_eq!(
            typed("param s 7 i -99999999999999999999999"),
            Command::SetParam {
                name: n(),
                index: 7,
                word: i32::MIN as u32
            }
        );
        assert_eq!(
            typed("unforce s all"),
            Command::Unforce {
                name: n(),
                index: None
            }
        );
        assert_eq!(
            typed("unforce s 7"),
            Command::Unforce {
                name: n(),
                index: Some(7)
            }
        );
        assert_eq!(
            typed("out s 12"),
            Command::Outputs {
                name: n(),
                count: 12
            }
        );
        assert_eq!(
            typed("out s 128"),
            Command::Outputs {
                name: n(),
                count: 128
            }
        );
        assert_eq!(typed("ev s"), Command::Events { name: n() });
        assert_eq!(typed("reset s"), Command::Reset { name: n() });
        assert_eq!(typed("pins"), Command::Pins);
        assert_eq!(
            typed("spdump 1700000000000"),
            Command::SpDump {
                ts_ms: 1_700_000_000_000
            }
        );
        assert_eq!(parse_command("spdump"), Err(ParseError::MissingArgument));
        assert_eq!(parse_command("spdump x"), Err(ParseError::InvalidNumber));
    }

    #[test]
    fn parses_floats_as_rust_prints_them() {
        let bits = |text: &str| match typed(&format!("in s 1 f {text}")) {
            Command::SetInput { word, .. } => word,
            other => panic!("{other:?}"),
        };
        assert_eq!(bits("1e20"), 1e20f32.to_bits());
        assert_eq!(bits("inf"), f32::INFINITY.to_bits());
        assert_eq!(bits("-inf"), f32::NEG_INFINITY.to_bits());
        assert!(f32::from_bits(bits("NaN")).is_nan());
        assert_eq!(bits("-0"), (-0.0f32).to_bits());
    }

    #[test]
    fn rejects_bad_debugger_commands() {
        let err = |cmd: &str| parse_command(cmd).unwrap_err();
        assert_eq!(err("in s 8 i 1"), ParseError::BadIndexOrType);
        assert_eq!(err("in s -1 i 1"), ParseError::BadIndexOrType);
        assert_eq!(err("force s 128 i 1"), ParseError::BadIndexOrType);
        assert_eq!(err("in s 0 x 1"), ParseError::BadIndexOrType);
        assert_eq!(err("in s 0 i abc"), ParseError::InvalidNumber);
        assert_eq!(err("in s 0 i 1.5"), ParseError::InvalidNumber);
        assert_eq!(err("in s 0 f abc"), ParseError::InvalidNumber);
        assert_eq!(err("out s 129"), ParseError::BadCount);
        assert_eq!(err("out s many"), ParseError::BadCount);
        assert_eq!(err("unforce s 128"), ParseError::BadIndex);
        assert_eq!(err("unforce s none"), ParseError::BadIndex);
        assert_eq!(err("unforce s"), ParseError::MissingArgument);
        assert_eq!(err("in s 0 i"), ParseError::MissingArgument);
        assert_eq!(err("in s"), ParseError::MissingArgument);
        assert_eq!(err("out s"), ParseError::MissingArgument);
        assert_eq!(err("ev"), ParseError::MissingArgument);
        assert_eq!(err("reset bad/name"), ParseError::InvalidName);
        assert_eq!(err("force bad/name 0 i 1"), ParseError::InvalidName);
        assert_eq!(err("pins x"), ParseError::TooManyArguments);
        assert_eq!(err("ev s x"), ParseError::TooManyArguments);
    }

    #[test]
    fn debugger_errors_use_the_cpp_words() {
        assert_eq!(ParseError::BadIndexOrType.to_string(), "bad index or type");
        assert_eq!(ParseError::BadIndex.to_string(), "bad index");
        assert_eq!(ParseError::BadCount.to_string(), "bad count");
    }

    #[test]
    fn formats_the_debugger_answers() {
        assert_eq!(
            format_outputs(&[42, 0x3f80_0000], 3),
            "out 0000002a 3f800000 00000000"
        );
        assert_eq!(format_outputs(&[1], 0), "out");
        assert_eq!(
            format_events("s", 1000, 2, &[(3, 1, 990), (3, 0, 995)]),
            "ev s 1000 2 3:1:990 3:0:995"
        );
        assert_eq!(format_events("s", 1000, 0, &[]), "ev s 1000 0");
        assert_eq!(
            format_pin(2, "blink", "out", "0", Some(true)),
            "2 blink out fault=0 level=1"
        );
        assert_eq!(
            format_pin(4, "blink", "pwm", "hold", None),
            "4 blink pwm fault=hold"
        );
        assert_eq!(
            format_pin(5, "m", "out", "1", Some(false)),
            "5 m out fault=1 level=0"
        );
    }

    #[test]
    fn input_splits_lines_and_image_chunks() {
        let mut input = Input::new();
        input.push(b"\r\n  list\r\nput a 5 0 -");
        assert_eq!(input.next_event(), Some(Event::Line(String::from("list"))));
        assert_eq!(input.next_event(), None);
        input.push(b"\n");
        assert_eq!(
            input.next_event(),
            Some(Event::Line(String::from("put a 5 0 -")))
        );
        // The image may hold newlines and comes in pieces of any size.
        input.expect_body(5, 2);
        input.push(b"\n");
        assert_eq!(input.next_event(), None);
        input.push(b"\n\n\n\nlist\n");
        assert_eq!(input.next_event(), Some(Event::Chunk(b"\n\n".to_vec())));
        assert_eq!(input.next_event(), Some(Event::Chunk(b"\n\n".to_vec())));
        assert!(input.in_body());
        assert_eq!(input.next_event(), Some(Event::Chunk(b"\n".to_vec())));
        assert!(!input.in_body());
        assert_eq!(input.next_event(), Some(Event::Line(String::from("list"))));
    }

    #[test]
    fn input_drops_a_cancelled_image_and_noise() {
        let mut input = Input::new();
        input.expect_body(100, 10);
        input.push(b"12345");
        input.cancel_body();
        input.push(b"list\n");
        assert_eq!(input.next_event(), Some(Event::Line(String::from("list"))));
        input.push(&[b'x'; MAX_LINE + 1]);
        assert_eq!(input.next_event(), None);
        input.push(b"info\n");
        assert_eq!(input.next_event(), Some(Event::Line(String::from("info"))));
    }

    #[test]
    fn bus_commands_parse() {
        assert_eq!(parse_command("bus"), Ok(Command::Bus));
        assert_eq!(
            parse_command("topic temp state 4"),
            Ok(Command::Topic {
                name: "temp".into(),
                state: true,
                max_len: 4
            })
        );
        assert_eq!(
            parse_command("queue pump 16"),
            Ok(Command::Queue {
                name: "pump".into(),
                cap: 16
            })
        );
        for bad in [
            "bus x",
            "topic",
            "topic t",
            "topic t flag 4",
            "topic t state 0",
            "topic t state 65",
            "topic t state 4 5",
            "topic aaaaaaaaaaaaaaaaaaaaaaaa event 4",
            "topic bad-name event 4",
            "queue",
            "queue m",
            "queue m 0",
            "queue m 17",
            "queue m 2 3",
        ] {
            assert_eq!(parse_command(bad), Err(ParseError::BadBus), "{bad}");
        }
        assert_eq!(ParseError::BadBus.to_string(), "bad bus command");
    }

    #[test]
    fn bus_lines_format() {
        assert_eq!(format_bus_module("pump", 8, 3), "bus pump queue=8 lost=3");
        assert_eq!(
            format_topic("t", true, 4, 1, Some(&[0x0a, 0xff])),
            "topic t state max=4 refused=1 last=0aff"
        );
        assert_eq!(
            format_topic("t", true, 4, 0, None),
            "topic t state max=4 refused=0 last=-"
        );
        assert_eq!(
            format_topic("e", false, 8, 0, Some(&[1])),
            "topic e event max=8 refused=0 last=-"
        );
    }
}
