//! The thread `crab-io` (`specs/fast-cycle`): reads the USB console, answers the polls of the
//! debugger from the snapshot (`list`, `info`, `out`, `ev`, `log`), hands every other command and
//! the chunks of an upload to the scan, and writes all answers to the port. On the ESP32 it runs
//! on core 0, away from the scan.

use std::collections::VecDeque;
use std::sync::Mutex;

use crab_master_rs::protocol::{parse_command, Command, Event, Input};

use super::spin::Spin;
use super::{snapshot, usb, CHUNK};

const IO_STACK: usize = 4096;
/// How long a read of the port waits for the first byte, milliseconds: also how late an answer
/// of the scan may go out.
const READ_WAIT_MS: u32 = 2;

/// The parser of the port: the scan switches it to image chunks once an upload is ready.
static INPUT: Mutex<Option<Input>> = Mutex::new(None);
/// Events waiting for the scan, at most (`specs/safety-audit`): `crab-io` waits for room, so the
/// port is slowed down instead of the memory growing or an image chunk being lost.
const TO_SCAN_MAX: usize = 64;
/// Commands and chunks for the scan, oldest first.
static TO_SCAN: Spin<VecDeque<Event>> = Spin::new(VecDeque::new());

/// Runs `f` on the parser of the port.
pub fn with_input<R>(f: impl FnOnce(&mut Input) -> R) -> R {
    let mut input = INPUT.lock().unwrap_or_else(|e| e.into_inner());
    f(input.get_or_insert_with(Input::new))
}

/// What came for the scan since the last call; never waits (an empty queue when the thread
/// `crab-io` holds it just now).
pub fn take_for_scan() -> VecDeque<Event> {
    match TO_SCAN.try_lock() {
        // Taken only when something came: an empty queue is not swapped (no heap work).
        Some(mut q) if !q.is_empty() => core::mem::take(&mut *q),
        _ => VecDeque::new(),
    }
}

/// Starts the thread; false if it could not (the caller then has no port: a fatal error).
pub fn start() -> bool {
    let spawned = super::spawn_background(b"crab-io\0", IO_STACK, run);
    if spawned {
        usb::queue_answers(true);
    }
    spawned
}

fn run() {
    let mut buf = [0u8; CHUNK];
    let mut events = Vec::new();
    let mut out = Vec::new();
    let mut held = Vec::with_capacity(HELD_MAX);
    loop {
        let n = usb::read(&mut buf, READ_WAIT_MS);
        if n > 0 {
            usb_input(&buf[..n], &mut held, &mut events);
        }
        usb::take_answers(&mut out);
        if !out.is_empty() {
            usb::write_raw(&out);
            super::netcmd::mirror(&out);
            out.clear();
        }
    }
}

/// The longest USB line held back while a network client is logged in.
const HELD_MAX: usize = 160;

/// USB has priority over the network (`specs/ota`, phase 1b): while a network client is logged
/// in, USB bytes are held until a whole line arrives. A printable, non-empty line drops the
/// network client and goes to the parser; anything else (line noise when a cable is plugged in,
/// bare line ends) is thrown away, so it neither drops the client nor mixes into its stream.
fn usb_input(bytes: &[u8], held: &mut Vec<u8>, events: &mut Vec<Event>) {
    if !super::netcmd::active() {
        if held.is_empty() {
            return feed_events(bytes, false, events);
        }
        held.extend_from_slice(bytes);
        feed_events(held, false, events);
        return held.clear();
    }
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\n' {
            if held.len() < HELD_MAX {
                held.push(b);
            } else {
                held.clear(); // too long for a command: noise
            }
            continue;
        }
        let line = held.trim_ascii();
        if !line.is_empty() && line.iter().all(|c| (0x20..0x7f).contains(c) || *c == b'\t') {
            super::netcmd::kick_for_usb();
            held.push(b'\n');
            held.extend_from_slice(&bytes[i + 1..]);
            feed_events(held, false, events);
            return held.clear();
        }
        held.clear();
    }
}

/// Bytes of a network client (`board/netcmd.rs`) into the same parser as USB.
pub fn feed(bytes: &[u8], from_net: bool) {
    let mut events = Vec::new();
    feed_events(bytes, from_net, &mut events);
}

fn feed_events(bytes: &[u8], from_net: bool, events: &mut Vec<Event>) {
    with_input(|input| {
        // A client dropped for USB: its last bytes must not reach the parser USB now owns.
        if from_net && !super::netcmd::active() {
            return;
        }
        input.push(bytes);
        while let Some(event) = input.next_event() {
            events.push(event);
        }
    });
    if !from_net && !events.is_empty() {
        super::netcmd::usb_heard();
    }
    for event in events.drain(..) {
        handle(event, from_net);
    }
}

fn handle(event: Event, from_net: bool) {
    if let Event::Line(line) = &event {
        match parse_command(line) {
            // The key of the network server: over USB only (physical access).
            Ok(Command::NetKey(_)) if from_net => return println!("err netkey only over USB"),
            Ok(Command::NetKey(key)) => {
                return if super::netcmd::set_key(key) {
                    println!("ok")
                } else {
                    println!("err nvs")
                };
            }
            Ok(Command::Name(name)) => {
                return if super::netfind::set_name(name.as_deref()) {
                    println!("ok")
                } else {
                    println!("err nvs")
                };
            }
            Ok(Command::List) => return snapshot::answer_list(),
            Ok(Command::Info) => return snapshot::answer_info(),
            Ok(Command::Outputs { name, count }) => return snapshot::answer_outputs(&name, count),
            Ok(Command::Events { name }) => return snapshot::answer_events(&name),
            Ok(Command::Log) => return snapshot::answer_log(),
            Ok(Command::CoreInfo) => return super::coredump::answer_info(),
            Ok(Command::CoreRead { offset }) => return super::coredump::answer_read(offset),
            Ok(Command::CoreErase) => return super::coredump::answer_erase(),
            // A crash on purpose (`specs/bug-report`): ESP-IDF writes the dump and restarts.
            #[cfg(feature = "selftest")]
            Ok(Command::SelfTestPanic) => panic!("selftest panic"),
            _ => {}
        }
    }
    let mut event = Some(event);
    while let Some(e) = event.take() {
        let mut q = TO_SCAN.lock_wait();
        if q.len() < TO_SCAN_MAX {
            q.push_back(e);
        } else {
            drop(q);
            event = Some(e);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
