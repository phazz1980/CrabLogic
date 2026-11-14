//! The thread `crab-i2c`: sends the queued I²C writes of modules (`I2C_SEND`) so the step does
//! not wait for the bus (`specs/fast-cycle`). The queue (`crab_master_rs::send_queue`) and this
//! thread appear with the first send; on the ESP32 the thread runs on core 0, away from the scan.
//! The step never waits here: a busy queue answers "not yet" (0) and the module tries again.

use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Condvar, Mutex};

use crab_kernel::abi::bus_op::ERR_BUS;
use crab_master_rs::send_queue::{SendQueue, MAX};
use esp_idf_svc::sys;

const STACK: usize = 3072;

/// The queue; `None` until the first module sends.
static QUEUE: Mutex<Option<Box<SendQueue>>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
/// The FreeRTOS task of the thread (for its stack figure in `list`).
static TASK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

/// A slot for a module: sets the queue and the thread up on the first call. `Err` is a
/// `crab.bus` code: no thread or every slot owned.
pub fn claim() -> Result<usize, i32> {
    let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    if q.is_none() {
        if !start() {
            return Err(ERR_BUS);
        }
        *q = Some(Box::default());
    }
    q.as_mut().and_then(|q| q.claim()).ok_or(ERR_BUS)
}

/// The module of `slot` went (called when its bus goes).
pub fn release(slot: usize) {
    if let Some(q) = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        q.release(slot);
    }
}

/// Queues a write; see `SendQueue::send`. Never waits: 0 while the thread holds the queue.
#[link_section = ".iram1.crab_i2c_send"]
pub fn send(slot: usize, addr: u8, data: &[u8]) -> i32 {
    let code = match QUEUE.try_lock() {
        Ok(mut q) => match q.as_mut() {
            Some(q) => q.send(slot, addr, data),
            None => ERR_BUS,
        },
        Err(_) => return 0,
    };
    if code > 0 {
        WAKE.notify_one();
    }
    code
}

/// Stack never used by the thread, bytes; `None` before it started.
pub fn stack_left() -> Option<u32> {
    let task = TASK.load(Ordering::Relaxed);
    // SAFETY: the task never ends once started; a null handle is not passed.
    (!task.is_null()).then(|| unsafe { sys::uxTaskGetStackHighWaterMark(task.cast()) })
}

fn start() -> bool {
    super::spawn_background(b"crab-i2c\0", STACK, run)
}

fn run() {
    // SAFETY: the handle of the calling task.
    TASK.store(
        unsafe { sys::xTaskGetCurrentTaskHandle() }.cast(),
        Ordering::Relaxed,
    );
    let mut buf = [0u8; MAX];
    loop {
        let job = {
            let mut q = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = q.as_mut().and_then(|q| q.take(&mut buf)) {
                    break job;
                }
                q = WAKE.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (slot, addr, len) = job;
        let err = super::bus::queued_i2c_write(addr, &buf[..len]);
        if let Some(q) = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            q.done(slot, err);
        }
    }
}
