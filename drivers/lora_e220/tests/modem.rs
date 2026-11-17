//! The module layer: register bytes by the datasheet, the setup automaton against a simulated
//! module (loss of an answer, no AUX pin, key change), frames out of a UART stream.

use lora_e220::e220::*;
use lora_e220::Radio;
use std::collections::VecDeque;

fn settings() -> Settings {
    Settings {
        addr: 0x0001,
        channel: 0x12,
        air: Air::R2400,
        sub: SubPacket::B200,
        power: Power::Dbm10,
        key: 0xABCD,
    }
}

#[test]
fn register_image_follows_the_datasheet() {
    // REG0 = 9600 8N1 + 2.4 kbps = 0b011_00_010; REG1 = 200 bytes, 10 dBm; REG3 = RSSI byte,
    // fixed transmission, WOR cycle 2000 ms.
    assert_eq!(
        settings().regs(),
        [0x00, 0x01, 0x62, 0x03, 0x12, 0xC3, 0xAB, 0xCD]
    );
    assert_eq!(
        write_cmd(&settings()),
        [0xC0, 0x00, 0x08, 0x00, 0x01, 0x62, 0x03, 0x12, 0xC3, 0xAB, 0xCD]
    );
    assert_eq!(READ_CMD, [0xC1, 0x00, 0x06]);
    let fast = Settings {
        air: Air::R62500,
        sub: SubPacket::B32,
        power: Power::Dbm22,
        ..settings()
    };
    assert_eq!(fast.regs()[2], 0x67);
    assert_eq!(fast.regs()[3], 0xC0);
    assert_eq!(prefix(0x0010, 0x12), [0x00, 0x10, 0x12]);
    assert!(settings().valid());
    assert!(!Settings {
        channel: 84,
        ..settings()
    }
    .valid());
}

/// A module that keeps registers, answers the config commands and lives by its own clock.
struct Fake {
    now: u32,
    config: bool,
    regs: [u8; 8],
    has_aux: bool,
    /// Busy (AUX low) until this time.
    busy_until: u32,
    to_host: VecDeque<(u32, u8)>,
    written: Vec<u8>,
    drop_next_answers: u32,
    flash_writes: u32,
    sent_to_air: Vec<Vec<u8>>,
}

impl Fake {
    fn new(has_aux: bool) -> Self {
        Self {
            now: 0,
            config: false,
            regs: [0, 0, 0x62, 0x00, 0x00, 0x03, 0, 0],
            has_aux,
            busy_until: 0,
            to_host: VecDeque::new(),
            written: Vec::new(),
            drop_next_answers: 0,
            flash_writes: 0,
            sent_to_air: Vec::new(),
        }
    }

    fn answer(&mut self, bytes: &[u8]) {
        if self.drop_next_answers > 0 {
            self.drop_next_answers -= 1;
            return;
        }
        let at = self.now + 5;
        for b in bytes {
            self.to_host.push_back((at, *b));
        }
    }
}

impl Hal for Fake {
    fn set_config_mode(&mut self, on: bool) {
        self.config = on;
        // Switching the mode takes the module a while.
        self.busy_until = self.now + if on { 40 } else { 25 };
    }

    fn aux(&mut self) -> Option<bool> {
        self.has_aux.then_some(self.now >= self.busy_until)
    }

    fn write(&mut self, data: &[u8]) -> bool {
        if !self.config {
            self.sent_to_air.push(data.to_vec());
            return true;
        }
        match data {
            [0xC1, 0x00, 0x06] => {
                let mut a = vec![0xC1, 0x00, 0x06];
                a.extend_from_slice(&self.regs[..6]);
                self.answer(&a);
            }
            [0xC0, 0x00, 0x08, rest @ ..] if rest.len() == 8 => {
                self.regs.copy_from_slice(rest);
                self.flash_writes += 1;
                let mut a = vec![0xC1, 0x00, 0x08];
                a.extend_from_slice(rest);
                self.answer(&a);
            }
            _ => {}
        }
        self.written.extend_from_slice(data);
        true
    }

    fn read(&mut self) -> Option<u8> {
        let (t, _) = self.to_host.front()?;
        if *t <= self.now {
            self.to_host.pop_front().map(|(_, b)| b)
        } else {
            None
        }
    }

    fn flush_rx(&mut self) {
        self.to_host.clear();
    }
}

fn run_setup(link: &mut Link<Fake>, ms: u32) {
    for _ in 0..ms {
        let now = link.hal.now + 1;
        link.hal.now = now;
        link.step(now);
    }
}

#[test]
fn a_blank_module_is_written_once_and_checked() {
    let mut link = Link::new(Fake::new(true), &settings(), false, false);
    run_setup(&mut link, 3000);
    assert!(link.ready());
    assert_eq!(link.hal.flash_writes, 1);
    assert_eq!(link.hal.regs, settings().regs());
    assert!(link.setup.key_written());
    assert!(!link.hal.config, "back in normal mode");
}

#[test]
fn a_module_that_already_matches_is_not_written() {
    let mut fake = Fake::new(true);
    fake.regs = settings().regs();
    let mut link = Link::new(fake, &settings(), false, false);
    run_setup(&mut link, 3000);
    assert!(link.ready());
    assert_eq!(link.hal.flash_writes, 0);
    assert!(!link.setup.key_written());
}

#[test]
fn a_changed_key_forces_the_write() {
    let mut fake = Fake::new(true);
    fake.regs = settings().regs();
    let mut link = Link::new(fake, &settings(), true, false);
    run_setup(&mut link, 3000);
    assert!(link.ready());
    assert_eq!(link.hal.flash_writes, 1);
    assert!(link.setup.key_written());
}

#[test]
fn a_lost_answer_is_retried() {
    let mut fake = Fake::new(true);
    fake.drop_next_answers = 2;
    let mut link = Link::new(fake, &settings(), false, false);
    run_setup(&mut link, 8000);
    assert!(link.ready());
    assert_eq!(link.hal.regs, settings().regs());
}

#[test]
fn works_without_the_aux_pin() {
    let mut link = Link::new(Fake::new(false), &settings(), false, false);
    run_setup(&mut link, 3000);
    assert!(link.ready());
    assert_eq!(link.hal.regs, settings().regs());
}

#[test]
fn a_silent_module_fails_and_tries_again_later() {
    let mut fake = Fake::new(true);
    fake.drop_next_answers = u32::MAX;
    let mut link = Link::new(fake, &settings(), false, false);
    run_setup(&mut link, 12_000);
    assert!(!link.ready());
    assert!(link.setup.failures >= 1);
    // The module wakes up: the next try succeeds.
    link.hal.drop_next_answers = 0;
    run_setup(&mut link, 12_000);
    assert!(link.ready());
}

#[test]
fn nothing_is_sent_before_the_setup_is_done() {
    let mut link = Link::new(Fake::new(true), &settings(), false, true);
    assert!(!link.send(&[0x08, 0x00, 0x00, 0x00]));
    assert_eq!(link.refused, 1);
    run_setup(&mut link, 3000);
    assert!(link.send(&[0x08, 0x00, 0x00, 0x00]));
    // Address of node 1 and the channel come first.
    assert_eq!(
        link.hal.sent_to_air[0],
        vec![0x00, 0x01, 0x12, 0x08, 0x00, 0x00, 0x00]
    );
}

#[test]
fn a_slave_writes_to_the_centre() {
    let mut link = Link::new(Fake::new(true), &settings(), false, false);
    run_setup(&mut link, 3000);
    assert!(link.send(&[0x88, 0x00, 0x00, 0x00]));
    assert_eq!(link.hal.sent_to_air[0][..3], [0x00, 0x10, 0x12]);
}

#[test]
fn frames_come_out_of_the_stream_and_the_rssi_byte_is_cut() {
    let mut link = Link::new(Fake::new(true), &settings(), false, false);
    run_setup(&mut link, 3000);
    let now = link.hal.now;
    // Two packets back to back: frame with 2 data bytes + RSSI, then an empty frame + RSSI.
    let stream = [
        0x08u8, 0x02, 0xAA, 0xBB, 0x11, 0x22, 0xC8, // frame 1, RSSI C8
        0x09, 0x00, 0x33, 0x44, 0xC9, // frame 2, RSSI C9
    ];
    for b in stream {
        link.hal.to_host.push_back((now, b));
    }
    let mut buf = [0u8; 64];
    let n = link.recv(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[0x08, 0x02, 0xAA, 0xBB, 0x11, 0x22]);
    assert_eq!(link.rssi(), 0xC8);
    let n = link.recv(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[0x09, 0x00, 0x33, 0x44]);
    assert_eq!(link.rssi(), 0xC9);
    assert!(link.recv(&mut buf).is_none());
}

#[test]
fn a_pause_inside_a_packet_resynchronises() {
    let mut r = Reader::new();
    let mut out = [0u8; 64];
    // A packet that lost its tail, then (after a pause) a whole one.
    assert!(r.feed(0x08, 0, &mut out).is_none());
    assert!(r.feed(0x02, 1, &mut out).is_none());
    assert!(r.feed(0xAA, 2, &mut out).is_none());
    let whole = [0x09u8, 0x00, 0x33, 0x44, 0xC9];
    let mut got = None;
    for (i, b) in whole.iter().enumerate() {
        got = r.feed(*b, 100 + i as u32, &mut out);
    }
    assert_eq!(got, Some(4));
    assert_eq!(&out[..4], &[0x09, 0x00, 0x33, 0x44]);
    assert_eq!(r.dropped, 1);
}

#[test]
fn an_impossible_length_is_dropped() {
    let mut r = Reader::new();
    let mut out = [0u8; 64];
    assert!(r.feed(0x08, 0, &mut out).is_none());
    assert!(r.feed(0xFF, 1, &mut out).is_none());
    assert_eq!(r.dropped, 1);
}

#[test]
fn the_key_tag_is_read_back_from_the_address() {
    assert_eq!(key_tag(b""), 0);
    assert_ne!(key_tag(b"secret"), key_tag(b"other"));
    assert_eq!(key_word(b""), 0);
    assert_ne!(key_word(b"secret"), key_word(b"other"));
    // A network address carries the tag, so frames go to it.
    let tag = key_tag(b"secret");
    let s = Settings {
        addr: u16::from_be_bytes([tag, 3]),
        key: key_word(b"secret"),
        ..settings()
    };
    let mut link = Link::new(Fake::new(true), &s, false, true);
    run_setup(&mut link, 3000);
    assert!(link.ready());
    assert!(link.send(&[0x08, 0x00, 0x00, 0x00]));
    assert_eq!(link.hal.sent_to_air[0][..3], [tag, 0x01, 0x12]);
    // A module of another key is rewritten even though only the tag differs.
    let mut fake = Fake::new(true);
    fake.regs = settings().regs();
    let mut link = Link::new(fake, &s, false, true);
    run_setup(&mut link, 3000);
    assert_eq!(link.hal.flash_writes, 1);
}
