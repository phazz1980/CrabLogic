//! Centre side: polls up to [`MAX_PEERS`] slaves one after another. An exchange is a request and
//! the reply with the same number; a lost one is repeated with the same bytes (so the slave can
//! tell a repeat), after the retries the slave is skipped until the next round. A changed command
//! goes out at once without waiting for the round pause. Nothing blocks and nothing allocates.

use crate::frame::{decode, encode, fingerprint, Error, Header, MAX_FRAME, MAX_VALUES, SEQ_MASK};
use crate::safe::{copy, Safe};
use crate::{since, Layout, Radio};

/// Slaves of one centre.
pub const MAX_PEERS: usize = 8;

/// Timings and limits (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// How long to wait for a reply before a repeat.
    pub reply_timeout_ms: u32,
    /// Repeats after the first send.
    pub retries: u8,
    /// Pause between two exchanges.
    pub gap_ms: u32,
    /// Pause after a full round (skipped by a changed command).
    pub round_pause_ms: u32,
    /// Silence after which a slave is offline.
    pub offline_ms: u32,
    /// Every n-th visit carries the full image even when nothing changed.
    pub full_every: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reply_timeout_ms: 800,
            retries: 2,
            gap_ms: 10,
            round_pause_ms: 500,
            offline_ms: 5000,
            full_every: 4,
        }
    }
}

/// Counters of one slave (wrapping, for diagnostics).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub sent: u32,
    pub replies: u32,
    pub repeats: u32,
    pub timeouts: u32,
    pub bad_crc: u32,
    pub late: u32,
    pub send_fail: u32,
}

/// One slave as the centre sees it.
#[derive(Debug, Clone, Copy)]
pub struct Peer {
    node: u8,
    req: Layout,
    rep: Layout,
    fp: u16,
    /// Commands for the slave; the owner writes them.
    pub tx: [u32; MAX_VALUES],
    /// Values the slave answered with (the last ones, kept while it is offline).
    pub rx: [u32; MAX_VALUES],
    /// Replied within the allowed time.
    pub online: bool,
    delivered: [u32; MAX_VALUES],
    inflight: [u32; MAX_VALUES],
    never_sent: bool,
    seq: u8,
    visits: u8,
    last_ok: u32,
    heard: bool,
    pub stats: Stats,
}

impl Peer {
    /// `req` — types of the commands sent, `rep` — types of the slave's answers.
    pub fn new(node: u8, req: Layout, rep: Layout) -> Self {
        Self {
            node,
            req,
            rep,
            fp: fingerprint(req.types(), rep.types()),
            tx: [0; MAX_VALUES],
            rx: [0; MAX_VALUES],
            online: false,
            delivered: [0; MAX_VALUES],
            inflight: [0; MAX_VALUES],
            never_sent: true,
            seq: 0,
            visits: 0,
            last_ok: 0,
            heard: false,
            stats: Stats::default(),
        }
    }

    /// The commands differ from what the slave last confirmed.
    fn dirty(&self) -> bool {
        self.never_sent || self.tx.part(self.req.len()) != self.delivered.part(self.req.len())
    }
}

#[derive(Debug, Clone, Copy)]
enum State {
    Idle { since: u32 },
    Wait { idx: usize, since: u32, tries: u8 },
}

pub struct Master {
    cfg: Config,
    peers: [Option<Peer>; MAX_PEERS],
    state: State,
    cursor: usize,
    pause_until: Option<u32>,
    /// The last exchange was a priority one: the next goes by the round.
    after_priority: bool,
    last: [u8; MAX_FRAME],
    last_len: usize,
}

impl Master {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            peers: [None; MAX_PEERS],
            state: State::Idle { since: 0 },
            cursor: 0,
            pause_until: None,
            after_priority: false,
            last: [0; MAX_FRAME],
            last_len: 0,
        }
    }

    /// Adds a slave; returns its index, or `None` if the table is full or the address is taken.
    pub fn add(&mut self, peer: Peer) -> Option<usize> {
        if self.peers.iter().flatten().any(|p| p.node == peer.node) {
            return None;
        }
        let (idx, slot) = self
            .peers
            .iter_mut()
            .enumerate()
            .find(|(_, p)| p.is_none())?;
        *slot = Some(peer);
        Some(idx)
    }

    pub fn peer(&self, idx: usize) -> Option<&Peer> {
        self.peers.get(idx)?.as_ref()
    }

    pub fn peer_mut(&mut self, idx: usize) -> Option<&mut Peer> {
        self.peers.get_mut(idx)?.as_mut()
    }

    fn count(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    /// One pass of the scan.
    pub fn step<R: Radio>(&mut self, now: u32, radio: &mut R) {
        let offline_ms = self.cfg.offline_ms;
        for p in self.peers.iter_mut().flatten() {
            p.online = p.heard && since(now, p.last_ok) <= offline_ms;
        }
        match self.state {
            State::Wait {
                idx,
                since: t,
                tries,
            } => self.wait(now, radio, idx, t, tries),
            State::Idle { since: t } => {
                if since(now, t) >= self.cfg.gap_ms {
                    self.start(now, radio);
                }
            }
        }
    }

    fn wait<R: Radio>(&mut self, now: u32, radio: &mut R, idx: usize, t: u32, tries: u8) {
        let mut buf = [0u8; MAX_FRAME];
        while let Some(n) = radio.recv(&mut buf) {
            if buf.get(..n).is_some_and(|f| self.accept(idx, f, now)) {
                self.state = State::Idle { since: now };
                return;
            }
        }
        if since(now, t) < self.cfg.reply_timeout_ms {
            return;
        }
        let Some(p) = self.peers.get_mut(idx).and_then(|p| p.as_mut()) else {
            self.state = State::Idle { since: now };
            return;
        };
        if tries < self.cfg.retries {
            p.stats.repeats = p.stats.repeats.wrapping_add(1);
            if !radio.send(self.last.part(self.last_len)) {
                p.stats.send_fail = p.stats.send_fail.wrapping_add(1);
            }
            self.state = State::Wait {
                idx,
                since: now,
                tries: tries + 1,
            };
        } else {
            p.stats.timeouts = p.stats.timeouts.wrapping_add(1);
            self.state = State::Idle { since: now };
        }
    }

    /// Checks a frame against the exchange in progress; `true` — it is the reply.
    fn accept(&mut self, idx: usize, frame: &[u8], now: u32) -> bool {
        let Some(p) = self.peers.get_mut(idx).and_then(|p| p.as_mut()) else {
            return false;
        };
        let mut cells = [0u32; MAX_VALUES];
        match decode(frame, p.rep.types(), p.fp, &mut cells) {
            Ok(d) if d.header.reply && d.header.node == p.node && d.header.seq == p.seq => {
                if d.values {
                    copy(&mut p.rx, cells.part(p.rep.len()));
                }
                p.delivered = p.inflight;
                p.never_sent = false;
                p.heard = true;
                p.online = true;
                p.last_ok = now;
                p.stats.replies = p.stats.replies.wrapping_add(1);
                true
            }
            Ok(_) => {
                p.stats.late = p.stats.late.wrapping_add(1);
                false
            }
            Err(Error::Crc) => {
                p.stats.bad_crc = p.stats.bad_crc.wrapping_add(1);
                false
            }
            Err(_) => {
                p.stats.late = p.stats.late.wrapping_add(1);
                false
            }
        }
    }

    /// The slave that goes next: an online one with a changed command (not twice in a row),
    /// otherwise the next one of the round after its pause.
    fn pick(&mut self, now: u32) -> Option<(usize, bool)> {
        if !self.after_priority {
            let dirty = self
                .peers
                .iter()
                .position(|p| p.as_ref().is_some_and(|p| p.online && p.dirty()));
            if let Some(i) = dirty {
                return Some((i, true));
            }
        }
        if self
            .pause_until
            .is_some_and(|t| since(now, t) > u32::MAX / 2)
        {
            return None;
        }
        let total = self.count();
        if total == 0 {
            return None;
        }
        for _ in 0..MAX_PEERS {
            let i = self.cursor % MAX_PEERS;
            self.cursor += 1;
            if self.peers.get(i).is_some_and(Option::is_some) {
                if self
                    .peers
                    .get(i + 1..)
                    .unwrap_or(&[])
                    .iter()
                    .all(Option::is_none)
                {
                    self.pause_until = Some(now.wrapping_add(self.cfg.round_pause_ms));
                    self.cursor = 0;
                }
                return Some((i, false));
            }
        }
        None
    }

    fn start<R: Radio>(&mut self, now: u32, radio: &mut R) {
        let Some((idx, priority)) = self.pick(now) else {
            return;
        };
        self.after_priority = priority;
        let full_every = self.cfg.full_every.max(1);
        let Some(p) = self.peers.get_mut(idx).and_then(|p| p.as_mut()) else {
            return;
        };
        p.seq = (p.seq + 1) & SEQ_MASK;
        p.visits = p.visits.wrapping_add(1);
        let with_values = p.dirty() || p.visits % full_every == 0;
        p.inflight = p.tx;
        let cells: &[u32] = if with_values {
            p.tx.part(p.req.len())
        } else {
            &[]
        };
        let header = Header {
            reply: false,
            node: p.node,
            seq: p.seq,
        };
        let Some(len) = encode(&mut self.last, header, p.req.types(), cells, p.fp) else {
            self.state = State::Idle { since: now };
            return;
        };
        self.last_len = len;
        p.stats.sent = p.stats.sent.wrapping_add(1);
        if !radio.send(self.last.part(len)) {
            p.stats.send_fail = p.stats.send_fail.wrapping_add(1);
            self.state = State::Idle { since: now };
            return;
        }
        self.state = State::Wait {
            idx,
            since: now,
            tries: 0,
        };
    }
}
