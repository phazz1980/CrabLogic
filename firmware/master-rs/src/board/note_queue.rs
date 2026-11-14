//! Journal queues: lines (or tagged lines) that a service queues and the runtime moves into the
//! journal between two steps. A "try" lock (`board/spin.rs`): the scan never waits for the queue.
//! A line the scan could not queue is dropped and counted; the next batch the scan takes reports
//! the count, so a loss is never silent. The queue keeps the newest `MAX` lines.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};

use esp_idf_svc::sys;

use super::spin::Spin;

/// Lines kept while nobody takes them.
const MAX: usize = 16;

pub struct NoteQueue<T> {
    queue: Spin<VecDeque<T>>,
    /// Lines the scan side dropped since the last `take`.
    dropped: AtomicU32,
}

impl<T> NoteQueue<T> {
    pub const fn new() -> Self {
        Self {
            queue: Spin::new(VecDeque::new()),
            dropped: AtomicU32::new(0),
        }
    }

    fn push_into(mut q: super::spin::Guard<'_, VecDeque<T>>, item: T) {
        // The oldest line is freed after the lock is released.
        let old = if q.len() == MAX { q.pop_front() } else { None };
        q.push_back(item);
        drop(q);
        drop(old);
    }

    /// From a background thread: waits for the queue (a millisecond at a time).
    pub fn push_wait(&self, item: T) {
        Self::push_into(self.queue.lock_wait(), item);
    }

    /// From the scan: never waits; a busy queue drops the line and counts it.
    pub fn push_try(&self, item: T) {
        match self.queue.try_lock() {
            Some(q) => Self::push_into(q, item),
            None => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// From a place that may run on the scan or on another task: as `push_try` on the scan (a task
    /// at the scan priority or above), as `push_wait` elsewhere.
    pub fn push_any(&self, item: T) {
        // SAFETY: a plain query of the calling task (a null handle means "this task").
        let prio = unsafe { sys::uxTaskPriorityGet(core::ptr::null_mut()) };
        if prio >= super::SCAN_PRIORITY as u32 {
            self.push_try(item);
        } else {
            self.push_wait(item);
        }
    }

    /// From the scan: the queued lines and the count of dropped ones. A busy queue gives nothing
    /// now (the lines stay for the next step); the queue is moved out, nothing is allocated for
    /// an empty one.
    pub fn take_try(&self) -> (Vec<T>, u32) {
        let lines = match self.queue.try_lock() {
            Some(mut q) => core::mem::take(&mut *q),
            None => VecDeque::new(),
        };
        (Vec::from(lines), self.dropped.swap(0, Ordering::Relaxed))
    }
}

/// The journal line for lines the scan side could not queue.
pub fn dropped_text(n: u32) -> String {
    format!("пропущено записей журнала: {n}")
}
