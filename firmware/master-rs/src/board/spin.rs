//! A light lock for data the scan shares with a background thread (`specs/fast-cycle`). A
//! `std::sync::Mutex` (a FreeRTOS mutex inside) costs about 5 µs per lock on the ESP32, even a
//! `try_lock`; an atomic flag costs well under 1 µs.
//!
//! Two ways to use it, never mixed on one lock:
//! - **Try** (`try_lock` from the scan, `lock_wait` from a background thread): the scan never waits;
//!   when the lock is busy it skips this step's work (the snapshot is published a step later).
//! - **Ceiling** (`lock_scan` from the scan, `lock_ceiling` from a background thread): the thread
//!   runs above the scan priority while it holds the lock (priority ceiling), so on one core the
//!   scan always finds the lock free, and on two cores it waits at most for the short section
//!   the other core is in. Sections under it must be short and never block.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use esp_idf_svc::sys;

/// Priority of a background thread while it holds a ceiling lock: just above the scan.
const CEILING: u32 = super::SCAN_PRIORITY as u32 + 1;

pub struct Spin<T> {
    busy: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: access to `data` is serialised by `busy`.
unsafe impl<T: Send> Sync for Spin<T> {}

pub struct Guard<'a, T> {
    lock: &'a Spin<T>,
    /// The priority to go back to (ceiling locks).
    restore: Option<u32>,
}

impl<T> Spin<T> {
    pub const fn new(value: T) -> Self {
        Self {
            busy: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    fn acquire(&self) -> bool {
        !self.busy.swap(true, Ordering::Acquire)
    }

    /// One attempt, never waits.
    #[inline]
    pub fn try_lock(&self) -> Option<Guard<'_, T>> {
        self.acquire().then_some(Guard {
            lock: self,
            restore: None,
        })
    }

    /// For a background thread on a "try" lock: waits, a millisecond at a time.
    pub fn lock_wait(&self) -> Guard<'_, T> {
        loop {
            if let Some(g) = self.try_lock() {
                return g;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// For the scan on a "ceiling" lock: the holder runs on the other core and is short.
    #[inline]
    pub fn lock_scan(&self) -> Guard<'_, T> {
        while !self.acquire() {
            core::hint::spin_loop();
        }
        Guard {
            lock: self,
            restore: None,
        }
    }

    /// For a background thread on a "ceiling" lock: above the scan priority while held.
    pub fn lock_ceiling(&self) -> Guard<'_, T> {
        // SAFETY: plain FreeRTOS calls on the calling task (a null handle means "this task").
        let old = unsafe {
            let old = sys::uxTaskPriorityGet(core::ptr::null_mut());
            if old < CEILING {
                sys::vTaskPrioritySet(core::ptr::null_mut(), CEILING);
            }
            old
        };
        while !self.acquire() {
            // Another background holder at the same priority: let it finish.
            std::thread::yield_now();
        }
        Guard {
            lock: self,
            restore: (old < CEILING).then_some(old),
        }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.lock.busy.store(false, Ordering::Release);
        if let Some(old) = self.restore {
            // SAFETY: back to the priority the task had before the lock.
            unsafe { sys::vTaskPrioritySet(core::ptr::null_mut(), old) };
        }
    }
}
