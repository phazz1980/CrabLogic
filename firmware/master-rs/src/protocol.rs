//! The line protocol of the master over USB, as the editor already speaks it to the C++ master
//! (`crab-build/src/mcu_module.rs`, `crab-debug/src/master.rs`).
//!
//! Lines end with `\n`. `put <name> <len> <crc32 hex> <allow,list|-> [mem=<bytes>]` is answered
//! `ready <chunk>`; then the image follows in chunks of up to `<chunk>` bytes, each answered
//! `ack <bytes so far>`; at the end `swap_us=<n>` and `ok`, or `err <reason>`.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

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
}

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
        _ => return Err(ParseError::UnknownCommand),
    };
    no_more(&mut args)?;
    Ok(parsed)
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
}
