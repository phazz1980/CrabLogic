//! Slave side: answers the requests of the centre. It never sends by itself. A request with the
//! number of the previous one is a repeat: it is answered again with the same reply and its values
//! are not applied twice.

use crate::frame::{decode, encode, fingerprint, Error, Header, MAX_FRAME, MAX_VALUES};
use crate::safe::{copy, Safe};
use crate::{since, Layout, Radio};

/// Counters of what the slave saw (wrapping, for diagnostics).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub requests: u32,
    pub repeats: u32,
    pub bad_crc: u32,
    pub bad_frame: u32,
    pub foreign: u32,
    pub send_fail: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Slave {
    node: u8,
    req: Layout,
    rep: Layout,
    fp: u16,
    offline_ms: u32,
    /// Commands received from the centre (last received, kept while the link is lost).
    pub rx: [u32; MAX_VALUES],
    /// Values for the centre; the owner writes them, the next reply carries them.
    pub tx: [u32; MAX_VALUES],
    /// Number of exchanges whose values were applied (a repeat is not counted).
    pub applied: u32,
    last_seq: Option<u8>,
    reply: [u8; MAX_FRAME],
    reply_len: usize,
    last_rx: u32,
    heard: bool,
    pub stats: Stats,
}

impl Slave {
    /// `req` — types the centre sends, `rep` — types this slave answers with.
    pub fn new(node: u8, req: Layout, rep: Layout, offline_ms: u32) -> Self {
        Self {
            node,
            req,
            rep,
            fp: fingerprint(req.types(), rep.types()),
            offline_ms,
            rx: [0; MAX_VALUES],
            tx: [0; MAX_VALUES],
            applied: 0,
            last_seq: None,
            reply: [0; MAX_FRAME],
            reply_len: 0,
            last_rx: 0,
            heard: false,
            stats: Stats::default(),
        }
    }

    /// The centre was heard within the allowed time.
    pub fn linked(&self, now: u32) -> bool {
        self.heard && since(now, self.last_rx) <= self.offline_ms
    }

    /// One pass of the scan: takes every waiting frame and answers the valid ones.
    pub fn step<R: Radio>(&mut self, now: u32, radio: &mut R) {
        let mut buf = [0u8; MAX_FRAME];
        let mut cells = [0u32; MAX_VALUES];
        while let Some(n) = radio.recv(&mut buf) {
            let Some(frame) = buf.get(..n) else {
                self.stats.bad_frame = self.stats.bad_frame.wrapping_add(1);
                continue;
            };
            match decode(frame, self.req.types(), self.fp, &mut cells) {
                Ok(d) if !d.header.reply && d.header.node == self.node => {
                    self.last_rx = now;
                    self.heard = true;
                    if self.last_seq == Some(d.header.seq) {
                        self.stats.repeats = self.stats.repeats.wrapping_add(1);
                    } else {
                        self.stats.requests = self.stats.requests.wrapping_add(1);
                        if d.values {
                            copy(&mut self.rx, cells.part(self.req.len()));
                            self.applied = self.applied.wrapping_add(1);
                        }
                        self.last_seq = Some(d.header.seq);
                        self.reply_len = encode(
                            &mut self.reply,
                            Header {
                                reply: true,
                                node: self.node,
                                seq: d.header.seq,
                            },
                            self.rep.types(),
                            self.tx.part(self.rep.len()),
                            self.fp,
                        )
                        .unwrap_or(0);
                    }
                    if self.reply_len == 0 || !radio.send(self.reply.part(self.reply_len)) {
                        self.stats.send_fail = self.stats.send_fail.wrapping_add(1);
                    }
                }
                Ok(_) => self.stats.foreign = self.stats.foreign.wrapping_add(1),
                Err(Error::Crc) => self.stats.bad_crc = self.stats.bad_crc.wrapping_add(1),
                Err(_) => self.stats.bad_frame = self.stats.bad_frame.wrapping_add(1),
            }
        }
    }
}
