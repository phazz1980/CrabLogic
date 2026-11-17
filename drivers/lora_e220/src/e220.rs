//! The E220-400T22D module: register image, the address prefix of fixed transmission, assembling
//! frames from the UART stream, and the non-blocking automaton that sets the module up from the
//! block's settings (read the registers, compare, write only when they differ).
//!
//! Module UART stays at 9600 8N1 (the factory value) in both modes: only M0/M1 change.

use crate::frame::{MAX_DATA, MAX_FRAME, NODE_MAX};
use crate::safe::{copy, Safe};
use crate::{since, Radio};

/// Low address byte of the centre; slaves use their node number 0..15, so 16 is free. The high
/// byte (ADDH) is the hash of the network key ([`key_tag`]): the key itself cannot be read back
/// from the module, but the high byte can, so a changed key shows up as a changed register.
pub const CENTRE_NODE: u8 = 0x10;

/// One byte that stands for the key (FNV-1a folded); 0 for no key.
pub fn key_tag(key: &[u8]) -> u8 {
    if key.is_empty() {
        return 0;
    }
    let mut h: u32 = 0x811C_9DC5;
    for &b in key {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    (h ^ (h >> 8) ^ (h >> 16) ^ (h >> 24)) as u8
}

/// The 16-bit key written to the write-only registers 06h/07h.
pub fn key_word(key: &[u8]) -> u16 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in key {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    if key.is_empty() {
        0
    } else {
        (h >> 16) as u16 ^ h as u16
    }
}

/// Air data rate (register 02h, bits 2..0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Air {
    R2400 = 2,
    R4800 = 3,
    R9600 = 4,
    R19200 = 5,
    R38400 = 6,
    R62500 = 7,
}

/// Sub-packet size in bytes (register 03h, bits 7..6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubPacket {
    B200 = 0,
    B128 = 1,
    B64 = 2,
    B32 = 3,
}

/// Transmit power (register 03h, bits 1..0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Dbm22 = 0,
    Dbm17 = 1,
    Dbm13 = 2,
    Dbm10 = 3,
}

/// What the block asks the module to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Module address: high byte the network tag ([`key_tag`]), low byte the node ([`CENTRE_NODE`]
    /// for the centre, the node number for a slave).
    pub addr: u16,
    /// Channel 0..83 (410.125 MHz + channel).
    pub channel: u8,
    pub air: Air,
    pub sub: SubPacket,
    pub power: Power,
    /// Encryption key, written to the write-only registers 06h/07h.
    pub key: u16,
}

pub const CHANNEL_MAX: u8 = 83;

/// Registers 00h..07h written to the module.
pub const REGS: usize = 8;
/// Registers 00h..05h that can be read back.
pub const READABLE: usize = 6;

impl Settings {
    pub fn valid(&self) -> bool {
        self.channel <= CHANNEL_MAX
    }

    /// The register image. REG0: 9600 baud, 8N1, air rate; REG1: sub-packet, power; REG2: channel;
    /// REG3: RSSI byte on, fixed transmission, WOR cycle left at its default (2000 ms).
    pub fn regs(&self) -> [u8; REGS] {
        let [ah, al] = self.addr.to_be_bytes();
        let [kh, kl] = self.key.to_be_bytes();
        [
            ah,
            al,
            (0b011 << 5) | self.air as u8,
            ((self.sub as u8) << 6) | self.power as u8,
            self.channel,
            0x80 | 0x40 | 0x03,
            kh,
            kl,
        ]
    }
}

/// Command that reads registers 00h..05h.
pub const READ_CMD: [u8; 3] = [0xC1, 0x00, READABLE as u8];

/// Command that writes all eight registers into the module's flash.
pub fn write_cmd(settings: &Settings) -> [u8; 3 + REGS] {
    write_image(&settings.regs())
}

fn write_image(regs: &[u8; REGS]) -> [u8; 3 + REGS] {
    let mut cmd = [0u8; 3 + REGS];
    copy(&mut cmd, &[0xC0, 0x00, REGS as u8]);
    if let Some(tail) = cmd.get_mut(3..) {
        copy(tail, regs);
    }
    cmd
}

/// The three bytes the module takes before a packet in fixed transmission: address and channel.
pub fn prefix(addr: u16, channel: u8) -> [u8; 3] {
    let [h, l] = addr.to_be_bytes();
    [h, l, channel]
}

/// What the board gives the driver: the UART of the module and its three control pins.
pub trait Hal {
    /// M0 = M1 = 1 (configuration) or 0 (normal).
    fn set_config_mode(&mut self, on: bool);
    /// AUX: `Some(true)` the module is idle, `None` the pin is not wired.
    fn aux(&mut self) -> Option<bool>;
    /// Writes to the module; `false` if the UART cannot take all of it now.
    fn write(&mut self, data: &[u8]) -> bool;
    fn read(&mut self) -> Option<u8>;
    /// Drops what is in the receive buffer.
    fn flush_rx(&mut self);
}

/// Packets arrive back to back; a longer pause inside one means bytes were lost.
const RX_GAP_MS: u32 = 30;
/// A received packet carries the RSSI byte after the frame.
const RSSI: usize = 1;

/// Assembles frames from the UART stream by the length field; resynchronises by the pause.
pub struct Reader {
    buf: [u8; MAX_FRAME + RSSI],
    len: usize,
    last: u32,
    /// RSSI byte of the last frame (−dBm = 256 − value).
    pub rssi: u8,
    /// Partial frames dropped (gap) and frames with an impossible length.
    pub dropped: u32,
}

impl Reader {
    pub const fn new() -> Self {
        Self {
            buf: [0; MAX_FRAME + RSSI],
            len: 0,
            last: 0,
            rssi: 0,
            dropped: 0,
        }
    }

    /// Takes one byte; returns the finished frame (without the RSSI byte) in `out`.
    pub fn feed(&mut self, byte: u8, now: u32, out: &mut [u8]) -> Option<usize> {
        if self.len > 0 && since(now, self.last) > RX_GAP_MS {
            self.len = 0;
            self.dropped = self.dropped.wrapping_add(1);
        }
        self.last = now;
        if self.len >= self.buf.len() {
            self.len = 0;
            self.dropped = self.dropped.wrapping_add(1);
        }
        self.buf.wr(self.len, byte);
        self.len += 1;
        let data_len = usize::from(self.buf.rd(1));
        if self.len >= 2 && data_len > MAX_DATA {
            self.len = 0;
            self.dropped = self.dropped.wrapping_add(1);
            return None;
        }
        if self.len < 2 || self.len < 4 + data_len + RSSI {
            return None;
        }
        let frame = self.len - RSSI;
        self.rssi = self.buf.rd(frame);
        self.len = 0;
        copy(out.get_mut(..frame)?, self.buf.part(frame));
        Some(frame)
    }
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

const ENTER_PAUSE_MS: u32 = 2;
const NO_AUX_PAUSE_MS: u32 = 20;
const ENTER_TIMEOUT_MS: u32 = 2000;
const EXIT_TIMEOUT_MS: u32 = 500;
const REPLY_TIMEOUT_MS: u32 = 1000;
const WRITE_QUIET_MS: u32 = 30;
const WRITE_TIMEOUT_MS: u32 = 1000;
const RETRY_PAUSE_MS: u32 = 100;
const RETRY_AFTER_FAIL_MS: u32 = 5000;
const TRIES: u8 = 3;

#[derive(Clone, Copy)]
enum St {
    Start,
    Enter { t: u32 },
    SendRead,
    ReadWait { t: u32, got: usize },
    SendWrite,
    WriteWait { t: u32, last: u32, got: usize },
    Pause { until: u32 },
    Exit { t: u32 },
    Ready,
    Failed { t: u32 },
}

/// Sets the module up. Never blocks: every wait is a time check in [`Setup::step`].
pub struct Setup {
    want: [u8; REGS],
    /// Write even if the readable registers match (the key changed; it cannot be read back).
    force: bool,
    st: St,
    tries: u8,
    writing: bool,
    wrote: bool,
    key_written: bool,
    buf: [u8; 3 + READABLE],
    /// Setups that gave up (the module did not answer).
    pub failures: u32,
}

impl Setup {
    pub fn new(settings: &Settings, force_write: bool) -> Self {
        Self {
            want: settings.regs(),
            force: force_write,
            st: St::Start,
            tries: 0,
            writing: false,
            wrote: false,
            key_written: false,
            buf: [0; 3 + READABLE],
            failures: 0,
        }
    }

    /// The module is set up and in normal mode.
    pub fn ready(&self) -> bool {
        matches!(self.st, St::Ready)
    }

    /// The last setup wrote the registers (the owner then stores the key's hash).
    pub fn key_written(&self) -> bool {
        self.key_written
    }

    /// Starts over (the module stopped answering, or the settings changed).
    pub fn restart(&mut self, settings: &Settings, force_write: bool) {
        self.want = settings.regs();
        self.force = force_write;
        self.st = St::Start;
    }

    fn aux_ready<H: Hal>(hal: &mut H, now: u32, t: u32, timeout: u32) -> Option<bool> {
        let waited = since(now, t);
        if waited >= timeout {
            return Some(false);
        }
        match hal.aux() {
            Some(true) if waited >= ENTER_PAUSE_MS => Some(true),
            None if waited >= NO_AUX_PAUSE_MS => Some(true),
            _ => None,
        }
    }

    fn retry(&mut self, now: u32, hal: &mut impl Hal) {
        self.tries += 1;
        if self.tries >= TRIES {
            self.failures = self.failures.wrapping_add(1);
            hal.set_config_mode(false);
            self.st = St::Failed { t: now };
        } else {
            self.st = St::Pause {
                until: now.wrapping_add(RETRY_PAUSE_MS),
            };
        }
    }

    pub fn step<H: Hal>(&mut self, now: u32, hal: &mut H) {
        match self.st {
            St::Ready => {}
            St::Start => {
                self.tries = 0;
                self.writing = false;
                self.wrote = false;
                self.key_written = false;
                hal.set_config_mode(true);
                self.st = St::Enter { t: now };
            }
            St::Enter { t } => match Self::aux_ready(hal, now, t, ENTER_TIMEOUT_MS) {
                Some(true) => {
                    hal.flush_rx();
                    self.st = St::SendRead;
                }
                Some(false) => self.retry(now, hal),
                None => {}
            },
            St::SendRead => {
                hal.flush_rx();
                if hal.write(&READ_CMD) {
                    self.st = St::ReadWait { t: now, got: 0 };
                } else {
                    self.retry(now, hal);
                }
            }
            St::ReadWait { t, mut got } => {
                while got < self.buf.len() {
                    let Some(b) = hal.read() else { break };
                    self.buf.wr(got, b);
                    got += 1;
                }
                if got == self.buf.len() {
                    self.checked(now, hal);
                } else if since(now, t) >= REPLY_TIMEOUT_MS {
                    self.retry(now, hal);
                } else {
                    self.st = St::ReadWait { t, got };
                }
            }
            St::SendWrite => {
                hal.flush_rx();
                let cmd = write_image(&self.want);
                if hal.write(&cmd) {
                    self.st = St::WriteWait {
                        t: now,
                        last: now,
                        got: 0,
                    };
                } else {
                    self.retry(now, hal);
                }
            }
            St::WriteWait {
                t,
                mut last,
                mut got,
            } => {
                // The answer (an echo of the parameters) is only drained: the read-back decides.
                while hal.read().is_some() {
                    got += 1;
                    last = now;
                }
                let quiet = got > 0 && since(now, last) >= WRITE_QUIET_MS;
                if quiet || since(now, t) >= WRITE_TIMEOUT_MS {
                    self.wrote = true;
                    self.writing = true;
                    self.tries = 0;
                    self.st = St::SendRead;
                } else {
                    self.st = St::WriteWait { t, last, got };
                }
            }
            St::Pause { until } => {
                if since(now, until) < u32::MAX / 2 {
                    self.st = if self.writing && !self.wrote {
                        St::SendWrite
                    } else {
                        St::SendRead
                    };
                }
            }
            St::Exit { t } => {
                if Self::aux_ready(hal, now, t, EXIT_TIMEOUT_MS).is_some() {
                    hal.flush_rx();
                    self.key_written = self.wrote;
                    self.st = St::Ready;
                }
            }
            St::Failed { t } => {
                if since(now, t) >= RETRY_AFTER_FAIL_MS {
                    self.st = St::Start;
                }
            }
        }
    }

    /// A full answer to the read command arrived: compare with what is wanted.
    fn checked(&mut self, now: u32, hal: &mut impl Hal) {
        let ok = self.buf[..3] == READ_CMD && self.buf[3..] == self.want[..READABLE];
        if ok && (!self.force || self.wrote) {
            hal.set_config_mode(false);
            self.st = St::Exit { t: now };
        } else if self.wrote {
            // Written, yet different when read back.
            self.retry(now, hal);
        } else {
            self.writing = true;
            self.st = St::SendWrite;
        }
    }
}

/// Module as a radio for the automata: the prefix is put in front of a frame, the frames are
/// taken from the byte stream. Does nothing until the setup is done.
pub struct Link<H: Hal> {
    pub hal: H,
    pub setup: Setup,
    reader: Reader,
    channel: u8,
    /// High byte of every address of this network.
    net: u8,
    /// Frames from the centre go to the node in their header; a slave always writes to the centre.
    centre: bool,
    now: u32,
    /// Frames refused because the module was busy (AUX low) or not set up.
    pub refused: u32,
}

impl<H: Hal> Link<H> {
    pub fn new(hal: H, settings: &Settings, force_write: bool, centre: bool) -> Self {
        Self {
            hal,
            setup: Setup::new(settings, force_write),
            reader: Reader::new(),
            channel: settings.channel,
            net: settings.addr.to_be_bytes()[0],
            centre,
            now: 0,
            refused: 0,
        }
    }

    pub fn ready(&self) -> bool {
        self.setup.ready()
    }

    /// Partial or impossible packets dropped by the reader.
    pub fn dropped(&self) -> u32 {
        self.reader.dropped
    }

    pub fn rssi(&self) -> u8 {
        self.reader.rssi
    }

    /// One pass of the scan: the setup while it runs, afterwards the receive stream is kept moving.
    pub fn step(&mut self, now: u32) {
        self.now = now;
        // Bytes of the stream go to the reader inside `recv`; only the setup runs here.
        if !self.setup.ready() {
            self.setup.step(now, &mut self.hal);
        }
    }
}

impl<H: Hal> Radio for Link<H> {
    fn send(&mut self, frame: &[u8]) -> bool {
        if !self.setup.ready() || self.hal.aux() == Some(false) || frame.is_empty() {
            self.refused = self.refused.wrapping_add(1);
            return false;
        }
        let node = if self.centre {
            (frame.rd(0) >> 3) & NODE_MAX
        } else {
            CENTRE_NODE
        };
        let node = u16::from_be_bytes([self.net, node]);
        let mut out = [0u8; 3 + MAX_FRAME];
        copy(&mut out, &prefix(node, self.channel));
        let Some(dst) = out.get_mut(3..3 + frame.len()) else {
            return false;
        };
        copy(dst, frame);
        let sent = self.hal.write(out.part(3 + frame.len()));
        if !sent {
            self.refused = self.refused.wrapping_add(1);
        }
        sent
    }

    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        if !self.setup.ready() {
            return None;
        }
        let mut frame = [0u8; MAX_FRAME];
        while let Some(b) = self.hal.read() {
            if let Some(n) = self.reader.feed(b, self.now, &mut frame) {
                copy(buf.get_mut(..n)?, frame.part(n));
                return Some(n);
            }
        }
        None
    }
}
