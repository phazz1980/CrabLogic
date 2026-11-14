//! The queue of background I²C writes (`I2C_SEND`, `specs/fast-cycle`): a fixed number of slots,
//! one per module that sends, each holding at most one write. The step copies its bytes in and
//! goes on; the I²C thread of the board takes the oldest write, sends it and leaves the result
//! for the next send of that module. Nothing is allocated; the board side is `board/i2c_send.rs`.

#[cfg(test)]
use crab_kernel::abi::bus_op::ERR_TIMEOUT;
use crab_kernel::abi::bus_op::{ERR_ARGS, ERR_BUS, MAX_LEN};

/// Modules that may send at the same time.
pub const SLOTS: usize = 4;
/// Largest write, bytes.
pub const MAX: usize = MAX_LEN as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not owned by a module.
    Free,
    /// Owned, nothing on its way.
    Idle,
    /// Waiting for the I²C thread.
    Queued,
    /// Being sent just now.
    Busy,
}

struct Job {
    state: State,
    /// The module went while its write was being sent: the slot is freed when it is done.
    gone: bool,
    ticket: u32,
    addr: u8,
    len: u16,
    /// The error of the last write (a `crab.bus` code), 0 when it went out.
    result: i32,
    data: [u8; MAX],
}

const FREE: Job = Job {
    state: State::Free,
    gone: false,
    ticket: 0,
    addr: 0,
    len: 0,
    result: 0,
    data: [0; MAX],
};

pub struct SendQueue {
    jobs: [Job; SLOTS],
    ticket: u32,
    /// Writes sent and failed since start.
    pub sent: u32,
    pub failed: u32,
}

impl Default for SendQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl SendQueue {
    pub const fn new() -> Self {
        Self {
            jobs: [FREE; SLOTS],
            ticket: 0,
            sent: 0,
            failed: 0,
        }
    }

    /// A slot for a module; `None` when all are owned.
    pub fn claim(&mut self) -> Option<usize> {
        let slot = self.jobs.iter().position(|j| j.state == State::Free)?;
        self.jobs[slot] = Job {
            state: State::Idle,
            ..FREE
        };
        Some(slot)
    }

    /// The module of `slot` went: a queued write is dropped, one being sent finishes first.
    pub fn release(&mut self, slot: usize) {
        let Some(j) = self.jobs.get_mut(slot) else {
            return;
        };
        if j.state == State::Busy {
            j.gone = true;
        } else {
            j.state = State::Free;
        }
    }

    /// Queues `data` for `addr`: its length when queued, 0 when the previous write of the slot is
    /// still on its way, the error of the previous write (once; nothing queued then) or `ERR_ARGS`.
    /// Empty `data` only asks: 1 when the previous write went out, 0 or the error as above.
    pub fn send(&mut self, slot: usize, addr: u8, data: &[u8]) -> i32 {
        let Some(j) = self.jobs.get_mut(slot) else {
            return ERR_ARGS;
        };
        match j.state {
            State::Free => return ERR_ARGS,
            State::Queued | State::Busy => return 0,
            State::Idle => {}
        }
        if j.result < 0 {
            return core::mem::take(&mut j.result);
        }
        if data.is_empty() {
            return 1;
        }
        if data.len() > MAX {
            return ERR_ARGS;
        }
        j.data[..data.len()].copy_from_slice(data);
        j.len = data.len() as u16;
        j.addr = addr;
        j.ticket = self.ticket;
        self.ticket = self.ticket.wrapping_add(1);
        j.state = State::Queued;
        data.len() as i32
    }

    pub fn has_work(&self) -> bool {
        self.jobs.iter().any(|j| j.state == State::Queued)
    }

    /// The oldest queued write, copied into `buf` and marked as on its way: (slot, address, length).
    pub fn take(&mut self, buf: &mut [u8; MAX]) -> Option<(usize, u8, usize)> {
        let now = self.ticket;
        let slot = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| j.state == State::Queued)
            .max_by_key(|(_, j)| now.wrapping_sub(j.ticket))
            .map(|(i, _)| i)?;
        let j = &mut self.jobs[slot];
        let len = j.len as usize;
        buf[..len].copy_from_slice(&j.data[..len]);
        j.state = State::Busy;
        Some((slot, j.addr, len))
    }

    /// The write of `slot` is done: `err` 0 or a `crab.bus` code (negative).
    pub fn done(&mut self, slot: usize, err: i32) {
        let Some(j) = self.jobs.get_mut(slot) else {
            return;
        };
        if err == 0 {
            self.sent = self.sent.wrapping_add(1);
        } else {
            self.failed = self.failed.wrapping_add(1);
        }
        if j.gone {
            *j = FREE;
            return;
        }
        j.result = if err < 0 {
            err
        } else if err > 0 {
            ERR_BUS
        } else {
            0
        };
        j.state = State::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_write_per_slot_in_order() {
        let mut q = SendQueue::new();
        let a = q.claim().unwrap();
        let b = q.claim().unwrap();
        assert_eq!(q.send(b, 0x3C, &[1, 2, 3]), 3);
        assert_eq!(q.send(a, 0x27, &[9]), 1);
        // The slot is busy until its write is done.
        assert_eq!(q.send(b, 0x3C, &[4]), 0);
        let mut buf = [0u8; MAX];
        assert_eq!(q.take(&mut buf), Some((b, 0x3C, 3)));
        assert_eq!(&buf[..3], &[1, 2, 3]);
        assert_eq!(q.send(b, 0x3C, &[4]), 0);
        q.done(b, 0);
        assert_eq!(q.take(&mut buf), Some((a, 0x27, 1)));
        assert_eq!(q.take(&mut buf), None);
        assert_eq!(q.send(b, 0x3C, &[4]), 1);
        assert_eq!(q.sent, 1);
    }

    #[test]
    fn a_failure_is_reported_once() {
        let mut q = SendQueue::new();
        let a = q.claim().unwrap();
        let mut buf = [0u8; MAX];
        assert_eq!(q.send(a, 0x3C, &[1]), 1);
        q.take(&mut buf);
        q.done(a, ERR_BUS);
        assert_eq!(q.send(a, 0x3C, &[2]), ERR_BUS);
        assert!(!q.has_work());
        assert_eq!(q.send(a, 0x3C, &[2]), 1);
        assert_eq!(q.failed, 1);
    }

    #[test]
    fn slots_are_bounded_and_freed() {
        let mut q = SendQueue::new();
        let slots: Vec<usize> = (0..SLOTS).map(|_| q.claim().unwrap()).collect();
        assert_eq!(q.claim(), None);
        // A queued write of a removed module is dropped.
        assert_eq!(q.send(slots[0], 1, &[1]), 1);
        q.release(slots[0]);
        assert!(!q.has_work());
        // One being sent finishes first.
        assert_eq!(q.send(slots[1], 1, &[1]), 1);
        let mut buf = [0u8; MAX];
        q.take(&mut buf);
        q.release(slots[1]);
        assert_eq!(q.claim(), Some(slots[0]));
        assert_eq!(q.claim(), None);
        q.done(slots[1], 0);
        assert_eq!(q.claim(), Some(slots[1]));
    }

    #[test]
    fn bad_arguments() {
        let mut q = SendQueue::new();
        assert_eq!(q.send(0, 1, &[1]), ERR_ARGS);
        assert_eq!(q.send(SLOTS, 1, &[1]), ERR_ARGS);
        let a = q.claim().unwrap();
        assert_eq!(q.send(a, 1, &[0; MAX + 1]), ERR_ARGS);
    }

    #[test]
    fn an_empty_send_asks_about_the_previous_one() {
        let mut q = SendQueue::new();
        let a = q.claim().unwrap();
        assert_eq!(q.send(a, 1, &[]), 1);
        assert_eq!(q.send(a, 1, &[7]), 1);
        assert_eq!(q.send(a, 1, &[]), 0);
        let mut buf = [0u8; MAX];
        q.take(&mut buf);
        assert_eq!(q.send(a, 1, &[]), 0);
        q.done(a, ERR_TIMEOUT);
        assert_eq!(q.send(a, 1, &[]), ERR_TIMEOUT);
        assert_eq!(q.send(a, 1, &[]), 1);
        assert!(!q.has_work());
        assert_eq!(q.send(a, 1, &[0; MAX + 1]), ERR_ARGS);
    }
}
